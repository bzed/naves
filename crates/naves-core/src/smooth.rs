// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A smooth curve fitted to one line inside a corridor.
//!
//! A line is resampled at equal steps ([`resample`]) and every sample gets one unknown, its offset `d_i`
//! along the line's normal at that sample. [`smooth_line`] minimises
//!
//! ```text
//! Σ c_i · |q_{i-1} − 2·q_i + q_{i+1}|²  +  stay · Σ d_i²        with q_i = p_i + d_i · n_i
//! ```
//!
//! subject to `|d_i| ≤ corridor` and `d_i = 0` at fixed samples. The cost is a convex quadratic and the
//! constraints are bounds, so the answer is unique; it is solved by [`crate::qp`], which is deterministic
//! bit for bit.
//!
//! [`smooth_graph`] applies the fit to a whole [`RoadGraph`], chain by chain: a chain is a run of stretches
//! that merely continue each other at nodes where exactly two stretches meet. Every node of a chain is a
//! fixed sample in the fit, and junctions, dead ends and bridge ends stay where they are; a node where a road
//! merely continues (two stretches) may move inside a widened zone (see [`WidenedZone`]). At a junction the
//! next sample is fixed as well, so the direction in which a road leaves it does not change either.
//!
//! The fit shrinks curves. [`smooth_graph`] therefore follows it with a second pass per chain (module
//! `widen`) that widens curves tighter than the road pieces can build, outward, to the family's minimum
//! radius; see [`WidenedZone`].

use std::collections::{BTreeMap, BTreeSet};

use rayon::prelude::*;

use crate::catalog::{Family, PieceCatalog};
use crate::qp::{Problem, Row, solve};
use crate::roadnet::{RoadGraph, Stretch, crossings_of, polyline_length};

mod widen;

/// The step of a resampled line, in metres.
pub const SAMPLE_M: f64 = 2.0;
/// How far a sample may move sideways, in metres.
pub const CORRIDOR_M: f64 = 2.0;
/// The weight of the pull of a sample towards its original position.
pub const W_STAY: f64 = 0.02;
/// The stiffness (`c_i`) callers give to samples inside a bridge stretch.
pub const BRIDGE_STRAIGHT_WEIGHT: f64 = 10.0;

/// How much road before and after a tight run belongs to its zone, in minimum radii.
pub const WIDEN_LEAD: f64 = 4.0;
/// How far a sample of a zone may move outward from its original position, in minimum radii.
pub const WIDEN_REACH: f64 = 2.0;
/// The number of directions in which the second difference of a zone sample is bounded.
pub const WIDEN_DIRECTIONS: usize = 8;
/// The cost of exceeding the radius bound, per metre of second difference.
pub const WIDEN_SLACK_COST: f64 = 1e4;
/// The most widening passes per chain.
pub const WIDEN_PASSES: usize = 3;
/// Switches the widening pass; off returns exactly what the fit alone gives.
pub const WIDEN_ENABLED: bool = true;
/// Switches the smoothing stage of the build; off, the road graph is what it was before the stage existed.
pub const SMOOTH_ENABLED: bool = true;

/// Down to this many degrees of summed turn a tight run is an open corner: its zone may cut inward as well
/// (up to what a fillet of the minimum radius needs); a bend that turns more is widened outward only.
pub const WIDEN_INWARD_MAX_DEG: f64 = 120.0;
/// A zone that still has a sample tighter than this fraction of the wanted radius after the last pass goes
/// back to the original line (the resampled vertices OpenStreetMap drew).
pub const WIDEN_KEEP_FRACTION: f64 = 0.9;
/// After the stage, a point of a stretch is dropped when taking it out moves the line by less than this, in
/// metres (a straight 100 m road is two points). Ends of stretches are never dropped.
///
/// Measured on the Berchtesgaden example at terrain size 2048: the smoothed graph has 59 453 points at 0.05 m
/// and 31 961 at 0.20 m (27 894 without the stage), and road placement takes 877 s at 0.05 m and 791 s at
/// 0.20 m. The findings are not worse (road in water 8 to 3, earthworks capped 81 to 69, terrain misses 3678 to
/// 3627, gaps 369 to 371, conflicts 828 = 828). Road pieces are placed with tolerances of tens of centimetres,
/// so 0.20 m of line simplification does not change what can be built.
pub const SIMPLIFY_TOL_M: f64 = 0.20;
/// A zone that goes back to the original line is blended into the fit over this length at each end, in metres
/// (linear, from the fit's offset there to zero), so the line has no step.
pub const WIDEN_REVERT_BLEND_M: f64 = 6.0;
/// Samples of a zone within this distance, along the chain, of a bridge end are held in the widening pass, in
/// metres: the approach of a bridge is not widened.
pub const WIDEN_BRIDGE_HOLD_M: f64 = 25.0;
/// A chain with more samples than this (counted from the lengths of its stretches, before resampling) is
/// neither fitted nor widened.
pub const MAX_CHAIN_SAMPLES: usize = 200_000;

/// The most segments [`resample`] makes of one line (a guard against absurd lengths).
const MAX_SEGMENTS: f64 = 1.0e6;

/// One sample of a line to smooth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// The position.
    pub at: (f64, f64),
    /// A fixed sample does not move.
    pub fixed: bool,
    /// The weight `c_i` of the bend at this sample (1 on plain road).
    pub stiffness: f64,
}

/// The settings of [`smooth_line`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SmoothParams {
    /// The largest sideways move of a sample, in metres.
    pub corridor: f64,
    /// The weight of the pull towards the original position.
    pub stay: f64,
    /// The tolerance of the drop of collinear points after the stage ([`SIMPLIFY_TOL_M`]); `0.0` keeps every
    /// sample.
    pub simplify: f64,
}

impl Default for SmoothParams {
    fn default() -> Self {
        SmoothParams {
            corridor: CORRIDOR_M,
            stay: W_STAY,
            simplify: SIMPLIFY_TOL_M,
        }
    }
}

/// Why a line is not smoothed.
#[derive(Debug, thiserror::Error)]
pub enum SmoothError {
    /// Fewer than three samples.
    #[error("a line needs at least three samples, it has {0}")]
    TooShort(usize),
    /// A coordinate, stiffness or setting is NaN, infinite or (for a weight or the corridor) negative.
    #[error("a coordinate, stiffness or setting is not a finite number")]
    NotFinite,
    /// All samples lie on one point.
    #[error("all samples lie on one point")]
    Degenerate,
    /// The solver did not give a solution.
    #[error("the solver failed: {0}")]
    Solver(String),
    /// The chain has more than [`MAX_CHAIN_SAMPLES`] samples.
    #[error("too long to smooth: about {0} samples")]
    TooLong(f64),
}

/// Points at equal steps of about `step` along `line`; first and last are kept exactly.
///
/// The step is adjusted to divide the line's length evenly (at least one step). Zero-length segments are
/// skipped. A line with a non-finite coordinate, or a `step` that is not positive and finite, comes back
/// unchanged.
#[must_use]
pub fn resample(line: &[(f64, f64)], step: f64) -> Vec<(f64, f64)> {
    let finite = line.iter().all(|p| p.0.is_finite() && p.1.is_finite());
    if !finite || !step.is_finite() || step <= 0.0 {
        return line.to_vec();
    }
    let mut points: Vec<(f64, f64)> = Vec::with_capacity(line.len());
    for &p in line {
        if points.last() != Some(&p) {
            points.push(p);
        }
    }
    let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
        return points;
    };
    if points.len() < 2 {
        return points;
    }
    // cumulative length at each point
    let mut cumulative = Vec::with_capacity(points.len());
    let mut total = 0.0;
    cumulative.push(0.0);
    for w in points.windows(2) {
        if let [a, b] = w {
            total += (b.0 - a.0).hypot(b.1 - a.1);
        }
        cumulative.push(total);
    }
    if !total.is_finite() {
        return line.to_vec();
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is rounded, at least 1 and at most MAX_SEGMENTS"
    )]
    let segments = (total / step).round().clamp(1.0, MAX_SEGMENTS) as usize;
    let mut out = Vec::with_capacity(segments + 1);
    out.push(first);
    let mut seg = 1;
    for k in 1..segments {
        #[expect(
            clippy::cast_precision_loss,
            reason = "k is below MAX_SEGMENTS, exact in f64"
        )]
        let target = total * (k as f64) / (segments as f64);
        while cumulative.get(seg).is_some_and(|&c| c < target) {
            seg += 1;
        }
        let (Some(&a), Some(&b), Some(&c0), Some(&c1)) = (
            points.get(seg - 1),
            points.get(seg),
            cumulative.get(seg - 1),
            cumulative.get(seg),
        ) else {
            break;
        };
        // c1 > c0: the loop above stops at the first point whose length reaches the target, and no
        // segment has length zero
        let t = (target - c0) / (c1 - c0);
        out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
    }
    out.push(last);
    out
}

/// The radius of the circle through three consecutive points (infinite on a straight line).
#[must_use]
pub fn radius_at(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
    let cross = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    if cross.abs() <= f64::MIN_POSITIVE {
        return f64::INFINITY;
    }
    let side = |p: (f64, f64), q: (f64, f64)| (q.0 - p.0).hypot(q.1 - p.1);
    side(a, b) * side(b, c) * side(c, a) / (2.0 * cross.abs())
}

/// The left unit normal of the direction from the previous to the next sample (one-sided at the ends);
/// zero where the two coincide.
fn normals(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let last = points.len().saturating_sub(1);
    (0..points.len())
        .map(|i| {
            let prev = points.get(i.saturating_sub(1)).copied().unwrap_or_default();
            let next = points.get((i + 1).min(last)).copied().unwrap_or_default();
            let (dx, dy) = (next.0 - prev.0, next.1 - prev.1);
            let len = dx.hypot(dy);
            if len > 0.0 {
                (-dy / len, dx / len)
            } else {
                (0.0, 0.0)
            }
        })
        .collect()
}

/// The smoothed positions of `samples`, same length and order. Fixed samples do not move; no sample moves
/// more than `params.corridor` along its normal.
///
/// # Errors
///
/// [`SmoothError::TooShort`] for fewer than three samples, [`SmoothError::NotFinite`] for a coordinate,
/// stiffness or setting that is not finite (or a negative weight or corridor),
/// [`SmoothError::Degenerate`] when all samples lie on one point and [`SmoothError::Solver`] when the
/// solver gives no solution.
pub fn smooth_line(
    samples: &[Sample],
    params: &SmoothParams,
) -> Result<Vec<(f64, f64)>, SmoothError> {
    let count = samples.len();
    if count < 3 {
        return Err(SmoothError::TooShort(count));
    }
    let settings_ok = params.corridor.is_finite()
        && params.corridor >= 0.0
        && params.stay.is_finite()
        && params.stay >= 0.0;
    let samples_ok = samples.iter().all(|s| {
        s.at.0.is_finite() && s.at.1.is_finite() && s.stiffness.is_finite() && s.stiffness >= 0.0
    });
    if !settings_ok || !samples_ok {
        return Err(SmoothError::NotFinite);
    }
    let points: Vec<(f64, f64)> = samples.iter().map(|s| s.at).collect();
    if points.windows(2).all(|w| w.first() == w.get(1)) {
        return Err(SmoothError::Degenerate);
    }
    let normal = normals(&points);

    let mut problem = Problem::new(count);
    for (i, (w, nw)) in samples.windows(3).zip(normal.windows(3)).enumerate() {
        // the second difference at sample i + 1, per coordinate
        if let ([before, mid, after], [na, nb, nc]) = (w, nw) {
            let centre = i + 1;
            let along_x = [(i, na.0), (centre, -2.0 * nb.0), (i + 2, nc.0)];
            let along_y = [(i, na.1), (centre, -2.0 * nb.1), (i + 2, nc.1)];
            problem.add_square(
                mid.stiffness,
                &along_x,
                before.at.0 - 2.0 * mid.at.0 + after.at.0,
            );
            problem.add_square(
                mid.stiffness,
                &along_y,
                before.at.1 - 2.0 * mid.at.1 + after.at.1,
            );
        }
    }
    for (i, s) in samples.iter().enumerate() {
        problem.add_square(params.stay, &[(i, 1.0)], 0.0);
        let bound = if s.fixed { 0.0 } else { params.corridor };
        problem.rows.push(Row {
            terms: vec![(i, 1.0)],
            lo: -bound,
            hi: bound,
        });
    }
    let d = solve(&problem).map_err(|e| SmoothError::Solver(e.to_string()))?;
    if d.len() != count || d.iter().any(|v| !v.is_finite()) {
        return Err(SmoothError::Solver(
            "the solution is not finite".to_string(),
        ));
    }
    Ok(samples
        .iter()
        .zip(&normal)
        .zip(&d)
        .map(|((s, nm), &off)| {
            if s.fixed {
                s.at
            } else {
                let off = off.clamp(-params.corridor, params.corridor);
                (s.at.0 + off * nm.0, s.at.1 + off * nm.1)
            }
        })
        .collect())
}

/// A two-stretch node turns at most this many degrees for the road to go on through it; a sharper turn
/// ends the chain (the limit the chain pairing of [`crate::roadnet::chains`] uses).
pub const CHAIN_TURN_MAX_DEG: f64 = 150.0;

/// The name of a stretch, as [`Stretch::id`].
pub type StretchId = String;

/// What smoothing did to one stretch.
#[derive(Debug, Clone, PartialEq)]
pub struct StretchSmooth {
    /// The stretch.
    pub id: StretchId,
    /// The furthest any of its samples moved, in metres. It counts samples only: the vertices of a curve that
    /// went back to the line as drawn carry no origin sample and are not counted.
    pub max_move_m: f64,
    /// Its length tighter than its family's minimum radius, before.
    pub tight_before_m: f64,
    /// The same after.
    pub tight_after_m: f64,
    /// The furthest any of its samples that lie in a tight zone moved (outward), in metres.
    pub max_outward_m: f64,
}

/// One tight zone of a chain and what widening did to it.
#[derive(Debug, Clone, PartialEq)]
pub struct WidenedZone {
    /// The tightest sample before widening.
    pub at: (f64, f64),
    /// The stretches the zone covers, in chain order.
    pub stretches: Vec<StretchId>,
    /// The smallest radius of the zone on the fitted line, in metres.
    pub radius_before_m: f64,
    /// The smallest radius of the zone after widening, in metres.
    pub radius_after_m: f64,
    /// The minimum radius the family asks for, in metres.
    pub radius_wanted_m: f64,
    /// The furthest any sample of the zone lies outward of its original line, in metres.
    pub max_outward_m: f64,
    /// The furthest any sample of the zone lies inward of its original line, in metres (an open corner is
    /// cut; see [`WIDEN_INWARD_MAX_DEG`]).
    pub max_inward_m: f64,
    /// No sample of the zone, nor the held sample on each side of it, is tighter than wanted afterwards
    /// (within 1 %).
    pub reached: bool,
    /// The bend cannot be built as a curve: after the last pass the zone was still tighter than
    /// [`WIDEN_KEEP_FRACTION`] of the wanted radius, so it went back to the original line (the vertices OpenStreetMap drew, and it moved nothing).
    /// `radius_after_m` is what the attempt got. The second allowed form is a concrete plate that joins the two
    /// legs; it is a later task, designed from the zones with this flag.
    pub plate_candidate: bool,
    /// The widened line had a sample in water that the original line did not have, so the zone went back to
    /// the original line like a plate candidate (and is not one unless [`Self::plate_candidate`] says so).
    pub blocked_by_water: bool,
}

/// What smoothing did, for the build report and the measurement.
#[derive(Debug, Clone, PartialEq)]
pub struct SmoothReport {
    /// The number of chains.
    pub chains: usize,
    /// Stretches whose chain could not be fitted (input that is not finite, degenerate input, a failed
    /// solve), with the reason, in stretch order. A sound stretch too short to smooth is counted in
    /// [`Self::skipped_short`] instead.
    pub kept: Vec<(StretchId, String)>,
    /// Sound stretches too short to smooth (fewer than three samples, a line of less than one step): they
    /// keep their line and are counted here, not listed among the failures in [`Self::kept`]. A stretch
    /// that is too short because its line is not finite or collapses to one point is a failure and is
    /// listed there.
    pub skipped_short: usize,
    /// The furthest any sample moved, in metres.
    pub max_move_m: f64,
    /// Road length tighter than its family's minimum radius, before. Measured on the resampled line
    /// ([`SAMPLE_M`] steps), not on the original polyline, so it already has the corners that resampling
    /// shaved. A zone that went back to the original line is measured at its vertex, on the resample of the
    /// written line too, which can read up to about 2 m less per reverted sharp corner.
    pub tight_before_m: f64,
    /// The same after, on the same resampled line.
    pub tight_after_m: f64,
    /// One entry per stretch, in stretch order.
    pub per_stretch: Vec<StretchSmooth>,
    /// The tight zones, in chain order and then along the chain.
    pub zones: Vec<WidenedZone>,
    /// The furthest outward move of any zone sample, in metres. [`Self::max_move_m`] covers only the samples
    /// outside zones.
    pub max_outward_m: f64,
    /// Stretches whose chain was fitted but whose widening failed in the solver (the chain keeps the fit),
    /// with the reason, in stretch order.
    pub widen_failed: Vec<(StretchId, String)>,
    /// The points of all stretches before the stage.
    pub points_before: usize,
    /// The points of all stretches after the fit and the widening, before the collinear ones are dropped.
    pub points_fitted: usize,
    /// The points of all stretches after the drop ([`SIMPLIFY_TOL_M`]).
    pub points_after: usize,
}

impl SmoothReport {
    /// One line for the build summary: the chains, the largest move, the tight length before and after, the
    /// zones reached and not reached and the plate candidates.
    #[must_use]
    pub fn summary(&self) -> String {
        let reached = self.zones.iter().filter(|z| z.reached).count();
        let plates = self.zones.iter().filter(|z| z.plate_candidate).count();
        let blocked = self.zones.iter().filter(|z| z.blocked_by_water).count();
        format!(
            "Road smoothing: {} chains, largest move {:.1} m (outward {:.1} m), tight length {:.0} m -> {:.0} m, \
             curve zones {reached} reached / {} not reached, {plates} plate candidate(s), {blocked} blocked by water",
            self.chains,
            self.max_move_m,
            self.max_outward_m,
            self.tight_before_m,
            self.tight_after_m,
            self.zones.len() - reached,
        )
    }
}

/// The distance of `p` from the segment `a`-`b`.
fn segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let along = if len2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - a.0 - along * dx).hypot(p.1 - a.1 - along * dy)
}

/// The line without the points that do not matter: every dropped point lies within `tolerance` metres of the
/// result (Ramer, Douglas and Peucker; the first and the last point stay). A line that is not finite, with
/// fewer than three points, or a tolerance that is not positive give the line back unchanged. Deterministic;
/// iterative, so a long line cannot overflow the stack.
///
/// The crates considered for this, none of them a dependency of `naves-core`:
///
/// - `geo`: its `Simplify` is this algorithm, but it brings `geo-types`, `robust`, `spade`, `rstar` and more for
///   thirty lines.
/// - `kurbo`: simplifies Bézier paths, not polylines.
/// - `simplify-polyline` 0.5.0 (a port of simplify-js, MIT): one maintainer and no release since October 2023;
///   `simplify_dp_step` recurses once per kept point (a chain of 200 000 samples can overflow the stack) and
///   indexes `points[0]` and `points[i]` unchecked; it does not look at non-finite points and a tolerance of
///   zero or NaN is not refused. Its optional radial pre-filter drops points by a different rule than "every
///   dropped point lies within the tolerance of the result".
/// - `rdp` 0.12.8 (MIT): an FFI wrapper around `geo`'s Ramer-Douglas-Peucker and Visvalingam-Whyatt for C callers
///   (raw pointers, `unsafe`), so it would bring `geo` as well; about a hundred recent downloads.
#[must_use]
pub fn simplify_line(line: &[(f64, f64)], tolerance: f64) -> Vec<(f64, f64)> {
    let n = line.len();
    if n < 3
        || tolerance.is_nan()
        || tolerance <= 0.0
        || line.iter().any(|p| !p.0.is_finite() || !p.1.is_finite())
    {
        return line.to_vec();
    }
    let mut keep = vec![false; n];
    for index in [0, n - 1] {
        if let Some(k) = keep.get_mut(index) {
            *k = true;
        }
    }
    let mut stack = vec![(0, n - 1)];
    while let Some((a, b)) = stack.pop() {
        let (Some(&from), Some(&to)) = (line.get(a), line.get(b)) else {
            continue;
        };
        let mut far = (0.0_f64, None);
        for k in a + 1..b {
            if let Some(&p) = line.get(k) {
                let d = segment_distance(p, from, to);
                if d > far.0 {
                    far = (d, Some(k));
                }
            }
        }
        if let (d, Some(k)) = far
            && d > tolerance
        {
            if let Some(slot) = keep.get_mut(k) {
                *slot = true;
            }
            stack.push((a, k));
            stack.push((k, b));
        }
    }
    line.iter()
        .zip(&keep)
        .filter_map(|(p, &k)| k.then_some(*p))
        .collect()
}

/// The tightest curve piece's radius of a road family in the catalog (`None` if it has no curve piece).
#[must_use]
pub fn min_radius(catalog: &PieceCatalog, family: Family) -> Option<f64> {
    catalog.tightest_arc(family)
}

/// An end of a stretch: the stretch's index and whether it is its start.
type Arm = (usize, bool);

/// The unit direction in which the stretch leaves the node at one of its ends, if it has one.
fn leaving(s: &Stretch, at_start: bool) -> Option<(f64, f64)> {
    let mut pts: Box<dyn Iterator<Item = &(f64, f64)>> = if at_start {
        Box::new(s.points.iter())
    } else {
        Box::new(s.points.iter().rev())
    };
    let origin = *pts.next()?;
    let to = pts.find(|p| **p != origin)?;
    let (dx, dy) = (to.0 - origin.0, to.1 - origin.1);
    let len = dx.hypot(dy);
    (len > 0.0 && len.is_finite()).then(|| (dx / len, dy / len))
}

/// All coordinates finite and at least two distinct points.
fn sound(s: &Stretch) -> bool {
    s.points.iter().all(|p| p.0.is_finite() && p.1.is_finite())
        && s.points.windows(2).any(|w| w.first() != w.get(1))
}

/// Why a stretch the fit calls [`SmoothError::TooShort`] is a failure rather than merely short: its line
/// is not finite, or it collapses to one point.
fn unsound_reason(s: Option<&Stretch>) -> String {
    let not_finite = s.is_some_and(|s| {
        s.points
            .iter()
            .any(|p| !p.0.is_finite() || !p.1.is_finite())
    });
    if not_finite {
        SmoothError::NotFinite.to_string()
    } else {
        SmoothError::Degenerate.to_string()
    }
}

/// The end point of a stretch at `at_start`.
fn end_point(s: &Stretch, at_start: bool) -> Option<(f64, f64)> {
    if at_start {
        s.points.first().copied()
    } else {
        s.points.last().copied()
    }
}

/// One stretch of a chain, in the chain's direction.
#[derive(Clone, Copy)]
struct Link {
    stretch: usize,
    reversed: bool,
}

struct ChainPlan {
    links: Vec<Link>,
    /// A junction (three or more stretches) at the first and at the last node.
    junction: (bool, bool),
}

/// The chains of the graph, in the order of their first stretch.
fn plan_chains(graph: &RoadGraph) -> Vec<ChainPlan> {
    let mut arms_at: BTreeMap<i64, Vec<Arm>> = BTreeMap::new();
    for (i, s) in graph.stretches.iter().enumerate() {
        arms_at.entry(s.start).or_default().push((i, true));
        arms_at.entry(s.end).or_default().push((i, false));
    }
    // at a node where the road goes on, its two arms, both ways round
    let mut partner: BTreeMap<Arm, Arm> = BTreeMap::new();
    for arms in arms_at.values() {
        let [a, b] = arms.as_slice() else { continue };
        let (Some(sa), Some(sb)) = (graph.stretches.get(a.0), graph.stretches.get(b.0)) else {
            continue;
        };
        if !sound(sa) || !sound(sb) || sa.tunnel != sb.tunnel {
            continue;
        }
        let (Some(pa), Some(pb)) = (end_point(sa, a.1), end_point(sb, b.1)) else {
            continue;
        };
        if pa != pb {
            continue;
        }
        let (Some(da), Some(db)) = (leaving(sa, a.1), leaving(sb, b.1)) else {
            continue;
        };
        let angle = (da.0 * db.1 - da.1 * db.0)
            .atan2(da.0 * db.0 + da.1 * db.1)
            .abs();
        if 180.0 - angle.to_degrees() <= CHAIN_TURN_MAX_DEG {
            partner.insert(*a, *b);
            partner.insert(*b, *a);
        }
    }
    let junction_at = |node: i64| arms_at.get(&node).is_some_and(|a| a.len() >= 3);
    let mut visited = vec![false; graph.stretches.len()];
    let mut plans = Vec::new();
    for s in 0..graph.stretches.len() {
        if visited.get(s).copied().unwrap_or(true) {
            continue;
        }
        // back to the head of the chain: enter stretch `head.0` at its start (`head.1`) or at its end
        let mut head: Arm = (s, true);
        for _ in 0..=graph.stretches.len() {
            let Some(&(t, t_arm)) = partner.get(&head) else {
                break;
            };
            let before = (t, !t_arm);
            if before == (s, true) {
                break;
            }
            head = before;
        }
        let mut links = Vec::new();
        let mut enter = head;
        let mut ring = false;
        loop {
            if visited.get(enter.0).copied().unwrap_or(true) {
                ring = !links.is_empty();
                break;
            }
            if let Some(v) = visited.get_mut(enter.0) {
                *v = true;
            }
            links.push(Link {
                stretch: enter.0,
                reversed: !enter.1,
            });
            match partner.get(&(enter.0, !enter.1)) {
                Some(&next) => enter = next,
                None => break,
            }
        }
        let entry = |l: &Link| {
            graph
                .stretches
                .get(l.stretch)
                .map_or(0, |st| if l.reversed { st.end } else { st.start })
        };
        if ring {
            // cut at the node with the lowest id
            let cut = links
                .iter()
                .enumerate()
                .min_by_key(|(_, l)| entry(l))
                .map_or(0, |(i, _)| i);
            links.rotate_left(cut);
        }
        let exit = |l: &Link| {
            graph
                .stretches
                .get(l.stretch)
                .map_or(0, |st| if l.reversed { st.start } else { st.end })
        };
        let junction = (
            !ring && links.first().is_some_and(|l| junction_at(entry(l))),
            !ring && links.last().is_some_and(|l| junction_at(exit(l))),
        );
        plans.push(ChainPlan { links, junction });
    }
    plans
}

/// What the solve of one chain gives back.
struct Fitted {
    /// The smoothed samples.
    after: Vec<(f64, f64)>,
}

/// A chain's samples, the index of the last sample of every link (the first link starts at 0), and the
/// stiffness and fixed flags.
struct Prepared {
    samples: Vec<Sample>,
    bounds: Vec<usize>,
    /// The chain's original line as the stretches have it (not resampled), the reference for "not inward".
    raw: Vec<(f64, f64)>,
}

fn prepare(graph: &RoadGraph, plan: &ChainPlan) -> Prepared {
    let mut points: Vec<(f64, f64)> = Vec::new();
    let mut raw: Vec<(f64, f64)> = Vec::new();
    let mut bounds: Vec<usize> = Vec::new();
    for link in &plan.links {
        let Some(s) = graph.stretches.get(link.stretch) else {
            continue;
        };
        let mut line = s.points.clone();
        if link.reversed {
            line.reverse();
        }
        raw.extend(line.iter().copied().skip(usize::from(!raw.is_empty())));
        let rs = resample(&line, SAMPLE_M);
        points.extend(rs.into_iter().skip(usize::from(!points.is_empty())));
        bounds.push(points.len().saturating_sub(1));
    }
    let count = points.len();
    let mut fixed = vec![false; count];
    let mut stiffness = vec![1.0; count];
    let mut first = 0;
    for (link, &last) in plan.links.iter().zip(&bounds) {
        let bridge = graph.stretches.get(link.stretch).is_some_and(|s| s.bridge);
        if bridge {
            for k in first + 1..last {
                if let Some(c) = stiffness.get_mut(k) {
                    *c = BRIDGE_STRAIGHT_WEIGHT;
                }
            }
        }
        for k in [first, last] {
            if let Some(f) = fixed.get_mut(k) {
                *f = true;
            }
        }
        first = last;
    }
    if plan.junction.0
        && let Some(f) = fixed.get_mut(1)
    {
        *f = true;
    }
    if plan.junction.1
        && let Some(f) = fixed.get_mut(count.saturating_sub(2))
    {
        *f = true;
    }
    Prepared {
        samples: points
            .iter()
            .zip(&fixed)
            .zip(&stiffness)
            .map(|((&at, &fixed), &stiffness)| Sample {
                at,
                fixed,
                stiffness,
            })
            .collect(),
        bounds,
        raw,
    }
}

/// The road length tighter than the family's minimum radius, per link: every sample with two neighbours
/// counts half of each segment next to it towards the link the segment belongs to.
fn tight_lengths(points: &[(f64, f64)], bounds: &[usize], minimum: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; bounds.len()];
    let mut first = 0;
    for ((&last, &min_r), total) in bounds.iter().zip(minimum).zip(out.iter_mut()) {
        for k in first..last {
            let (Some(&a), Some(&b)) = (points.get(k), points.get(k + 1)) else {
                continue;
            };
            let half = (b.0 - a.0).hypot(b.1 - a.1) / 2.0;
            for e in [k, k + 1] {
                let tight = e
                    .checked_sub(1)
                    .and_then(|p| Some((points.get(p)?, points.get(e)?, points.get(e + 1)?)))
                    .is_some_and(|(p, q, r)| radius_at(*p, *q, *r) < min_r);
                if tight {
                    *total += half;
                }
            }
        }
        first = last;
    }
    out
}

/// The line with every segment longer than 1.5 [`SAMPLE_M`] divided into equal parts of about that length (the
/// vertices stay, and a line that is sampled already is unchanged), and where its links end: the line the tight
/// length is measured on, so that a stretch of widely spaced original vertices is measured as the samples it is
/// made of, as before the stage.
fn resampled_by_link(points: &[(f64, f64)], bounds: &[usize]) -> (Vec<(f64, f64)>, Vec<usize>) {
    let mut line: Vec<(f64, f64)> = Vec::new();
    let mut out_bounds = Vec::with_capacity(bounds.len());
    let mut first = 0;
    for &last in bounds {
        let span = points.get(first..=last).unwrap_or_default();
        if line.is_empty()
            && let Some(&p) = span.first()
        {
            line.push(p);
        }
        for w in span.windows(2) {
            let [a, b] = w else { continue };
            let length = (b.0 - a.0).hypot(b.1 - a.1);
            let parts = if length > 1.5 * SAMPLE_M && length.is_finite() {
                // at most MAX_SEGMENTS parts, rounded from a positive length
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a rounded positive length over the step, clamped"
                )]
                let n = (length / SAMPLE_M).round().clamp(1.0, 100_000.0) as u32;
                n
            } else {
                1
            };
            for k in 1..parts {
                let t = f64::from(k) / f64::from(parts);
                line.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
            }
            line.push(*b);
        }
        out_bounds.push(line.len().saturating_sub(1));
        first = last;
    }
    (line, out_bounds)
}

/// The minimum radius at every sample of a chain: that of the link the sample belongs to (a node belongs to
/// the link before it).
fn sample_minimum(count: usize, bounds: &[usize], minimum: &[f64]) -> Vec<f64> {
    let mut out = vec![f64::NAN; count];
    let mut first = 0;
    for (&last, &m) in bounds.iter().zip(minimum) {
        for k in first..=last {
            if let Some(slot) = out.get_mut(k) {
                *slot = m;
            }
        }
        first = last + 1;
    }
    out
}

/// Which samples of a chain may move inside a zone: those the fit may move, and the nodes where the road
/// merely continues (not at the ends of the chain and not next to a bridge).
fn movable_samples(
    graph: &RoadGraph,
    plan: &ChainPlan,
    prepared: &Prepared,
    bridge_nodes: &BTreeSet<i64>,
) -> Vec<bool> {
    let mut movable: Vec<bool> = prepared.samples.iter().map(|s| !s.fixed).collect();
    // the zone widens the approaches of a bridge, the deck does not bend
    let mut first = 0;
    for (link, &last) in plan.links.iter().zip(&prepared.bounds) {
        if graph.stretches.get(link.stretch).is_none_or(|s| s.bridge) {
            for k in first..=last {
                if let Some(m) = movable.get_mut(k) {
                    *m = false;
                }
            }
        }
        first = last;
    }
    for (j, pair) in plan.links.windows(2).enumerate() {
        let bridge = pair
            .iter()
            .any(|l| graph.stretches.get(l.stretch).is_none_or(|s| s.bridge));
        if bridge {
            continue;
        }
        if let Some(m) = prepared.bounds.get(j).and_then(|&k| movable.get_mut(k)) {
            *m = true;
        }
    }
    hold_bridge_approaches(graph, plan, prepared, bridge_nodes, &mut movable);
    movable
}

/// Holds the samples within [`WIDEN_BRIDGE_HOLD_M`] along the chain of the end of a bridge: the bridge links of
/// the chain, and the ends of the chain that lie at the end node of a bridge stretch.
fn hold_bridge_approaches(
    graph: &RoadGraph,
    plan: &ChainPlan,
    prepared: &Prepared,
    bridge_nodes: &BTreeSet<i64>,
    movable: &mut [bool],
) {
    let count = prepared.samples.len();
    let mut arc = Vec::with_capacity(count);
    let mut total = 0.0;
    let mut previous: Option<(f64, f64)> = None;
    for s in &prepared.samples {
        if let Some(p) = previous {
            total += (s.at.0 - p.0).hypot(s.at.1 - p.1);
        }
        arc.push(total);
        previous = Some(s.at);
    }
    let at = |k: usize| arc.get(k).copied().unwrap_or(0.0);
    // the stretches of arc length that count as a bridge end: from the first to the last sample of a bridge link,
    // and the two ends of the chain where a bridge stretch ends
    let mut spans: Vec<(f64, f64)> = Vec::new();
    let mut first = 0;
    for (link, &last) in plan.links.iter().zip(&prepared.bounds) {
        if graph.stretches.get(link.stretch).is_some_and(|s| s.bridge) {
            spans.push((at(first), at(last)));
        }
        first = last;
    }
    let node_of = |link: Option<&Link>, entering: bool| {
        link.and_then(|l| graph.stretches.get(l.stretch).map(|s| (s, l.reversed)))
            .map(|(s, reversed)| if entering == reversed { s.end } else { s.start })
    };
    if node_of(plan.links.first(), true).is_some_and(|n| bridge_nodes.contains(&n)) {
        spans.push((0.0, 0.0));
    }
    if node_of(plan.links.last(), false).is_some_and(|n| bridge_nodes.contains(&n)) {
        spans.push((total, total));
    }
    for (k, m) in movable.iter_mut().enumerate() {
        let here = at(k);
        if spans.iter().any(|&(from, to)| {
            here >= from - WIDEN_BRIDGE_HOLD_M && here <= to + WIDEN_BRIDGE_HOLD_M
        }) {
            *m = false;
        }
    }
}

/// Whether a point (map metres) lies in water.
pub type WaterTest<'a> = &'a (dyn Fn((f64, f64)) -> bool + Sync);

/// What every chain is solved against.
struct Context<'a> {
    /// The nodes at an end of a bridge stretch.
    bridge_nodes: &'a BTreeSet<i64>,
    in_water: WaterTest<'a>,
}

/// What is solved for one chain.
struct Solved {
    prepared: Prepared,
    fitted: Result<Fitted, SmoothError>,
    /// The minimum radius of every link.
    link_minimum: Vec<f64>,
    widened: Option<widen::Outcome>,
}

/// How far the samples `range` of a written-back chain moved: sideways for those outside a zone (against
/// the sample they came from) and outward for those inside one (`signed` is how far outward of the original
/// line a sample lies, negative inward).
fn link_moves(
    range: std::ops::RangeInclusive<usize>,
    after: &[(f64, f64)],
    mask: &[bool],
    origin: &[Option<usize>],
    before: &[(f64, f64)],
    signed: &[f64],
) -> (f64, f64) {
    let mut moved = 0.0_f64;
    let mut outward = 0.0_f64;
    for k in range {
        let Some(now) = after.get(k) else { continue };
        if mask.get(k).copied().unwrap_or(false) {
            outward = outward.max(signed.get(k).copied().unwrap_or(0.0));
        } else if let Some(was) = origin.get(k).copied().flatten().and_then(|o| before.get(o)) {
            moved = moved.max((was.0 - now.0).hypot(was.1 - now.1));
        }
    }
    (moved, outward)
}

/// The fit and, when asked, the widening of one chain.
fn solve_chain(
    graph: &RoadGraph,
    plan: &ChainPlan,
    family_minimum: &[f64],
    params: &SmoothParams,
    widen_on: bool,
    context: &Context<'_>,
) -> Solved {
    let length: f64 = plan
        .links
        .iter()
        .filter_map(|l| graph.stretches.get(l.stretch))
        .map(|s| polyline_length(&s.points))
        .filter(|l| l.is_finite())
        .sum();
    let samples = length / SAMPLE_M;
    let limit = f64::from(u32::try_from(MAX_CHAIN_SAMPLES).unwrap_or(u32::MAX));
    let too_long = samples > limit;
    let prepared = if too_long {
        // nothing is resampled: a chain this long is left alone
        Prepared {
            samples: Vec::new(),
            bounds: vec![0; plan.links.len()],
            raw: Vec::new(),
        }
    } else {
        prepare(graph, plan)
    };
    let fitted = if too_long {
        Err(SmoothError::TooLong(samples.round()))
    } else {
        smooth_line(&prepared.samples, params).map(|after| Fitted { after })
    };
    let link_minimum: Vec<f64> = plan
        .links
        .iter()
        .map(|l| family_minimum.get(l.stretch).copied().unwrap_or(f64::NAN))
        .collect();
    let link_bridge: Vec<bool> = plan
        .links
        .iter()
        .map(|l| graph.stretches.get(l.stretch).is_some_and(|s| s.bridge))
        .collect();
    let movable = movable_samples(graph, plan, &prepared, context.bridge_nodes);
    let resampled: Vec<(f64, f64)> = prepared.samples.iter().map(|s| s.at).collect();
    let widened = match (&fitted, widen_on) {
        (Ok(f), true) => widen::widen_chain(&widen::ChainLine {
            original: &prepared.raw,
            fitted: &f.after,
            bounds: &prepared.bounds,
            link_minimum: &link_minimum,
            link_bridge: &link_bridge,
            movable: &movable,
            resampled: &resampled,
            in_water: context.in_water,
        }),
        _ => None,
    };
    Solved {
        prepared,
        fitted,
        link_minimum,
        widened,
    }
}

/// The zones of a widened chain, for the report.
fn chain_zones(graph: &RoadGraph, plan: &ChainPlan, widened: &widen::Outcome) -> Vec<WidenedZone> {
    widened
        .zones
        .iter()
        .map(|z| WidenedZone {
            at: z.at,
            stretches: z
                .links
                .iter()
                .filter_map(|&j| plan.links.get(j))
                .filter_map(|l| graph.stretches.get(l.stretch))
                .map(|s| s.id.clone())
                .collect(),
            radius_before_m: z.radius_before,
            radius_after_m: z.radius_after,
            radius_wanted_m: z.wanted,
            max_outward_m: z.max_outward,
            max_inward_m: z.max_inward,
            reached: z.reached,
            plate_candidate: z.plate_candidate,
            blocked_by_water: z.blocked_by_water,
        })
        .collect()
}

/// The line that is written back for a chain, where its links end, which samples lie in a zone, where each
/// sample came from in the fitted line and how far outward of the original line the zone samples lie.
struct Written<'a> {
    after: Vec<(f64, f64)>,
    bounds: Vec<usize>,
    mask: Vec<bool>,
    origin: Vec<Option<usize>>,
    signed: Vec<f64>,
    widen_error: Option<&'a SmoothError>,
}

fn written<'a>(
    widened: Option<&'a widen::Outcome>,
    fit_line: &[(f64, f64)],
    fit_bounds: &[usize],
) -> Written<'a> {
    match widened {
        Some(w) => Written {
            after: w.line.clone(),
            bounds: w.bounds.clone(),
            // when the widening failed the chain is the fit: nothing is reported as an outward move
            mask: if w.error.is_some() {
                vec![false; w.line.len()]
            } else {
                w.mask.clone()
            },
            origin: w.origin.clone(),
            signed: w.signed.clone(),
            widen_error: w.error.as_ref(),
        },
        None => Written {
            after: fit_line.to_vec(),
            bounds: fit_bounds.to_vec(),
            mask: vec![false; fit_line.len()],
            origin: (0..fit_line.len()).map(Some).collect(),
            signed: Vec::new(),
            widen_error: None,
        },
    }
}

/// What the chains add up to.
struct Tally {
    kept: Vec<Option<String>>,
    widen_failed: Vec<Option<String>>,
    skipped_short: usize,
    per_stretch: Vec<StretchSmooth>,
    zones: Vec<WidenedZone>,
}

impl Tally {
    fn new(graph: &RoadGraph) -> Tally {
        Tally {
            kept: vec![None; graph.stretches.len()],
            widen_failed: vec![None; graph.stretches.len()],
            skipped_short: 0,
            per_stretch: graph
                .stretches
                .iter()
                .map(|s| StretchSmooth {
                    id: s.id.clone(),
                    max_move_m: 0.0,
                    tight_before_m: 0.0,
                    tight_after_m: 0.0,
                    max_outward_m: 0.0,
                })
                .collect(),
            zones: Vec::new(),
        }
    }

    /// The stretches with a reason, in stretch order.
    fn reasons(graph: &RoadGraph, reasons: &[Option<String>]) -> Vec<(StretchId, String)> {
        graph
            .stretches
            .iter()
            .zip(reasons)
            .filter_map(|(s, k)| k.as_ref().map(|why| (s.id.clone(), why.clone())))
            .collect()
    }

    fn report(self, graph: &RoadGraph, chains: usize) -> SmoothReport {
        let per = &self.per_stretch;
        SmoothReport {
            chains,
            kept: Tally::reasons(graph, &self.kept),
            skipped_short: self.skipped_short,
            max_move_m: per.iter().map(|p| p.max_move_m).fold(0.0, f64::max),
            tight_before_m: per.iter().map(|p| p.tight_before_m).sum(),
            tight_after_m: per.iter().map(|p| p.tight_after_m).sum(),
            max_outward_m: per.iter().map(|p| p.max_outward_m).fold(0.0, f64::max),
            widen_failed: Tally::reasons(graph, &self.widen_failed),
            zones: self.zones,
            per_stretch: self.per_stretch,
            points_before: 0,
            points_fitted: 0,
            points_after: 0,
        }
    }
}

/// The graph with every stretch's line smoothed, chain by chain, and the curves that are tighter than the road
/// pieces can build widened outward (see [`WidenedZone`]). Node ids and stretch ids are unchanged; junction
/// nodes, dead ends and bridge end nodes do not move. Outside a zone no sample moves more than [`CORRIDOR_M`]
/// sideways; a node where a road merely continues moves with the road inside a zone, and the graph's
/// crossings are then rebuilt from the stretches. A stretch whose chain cannot be fitted keeps its line and
/// is listed in [`SmoothReport::kept`]; a sound stretch with fewer than three samples has nothing to smooth,
/// keeps its line and is counted in [`SmoothReport::skipped_short`] instead. A chain whose widening fails keeps
/// the fit and is listed in [`SmoothReport::widen_failed`].
///
/// `min_radius` gives the tightest radius of a stretch's family (`NaN`, `0.0` or infinity for none: no zone).
/// With [`WIDEN_ENABLED`] off this is [`smooth_graph_fit_only`].
#[must_use]
pub fn smooth_graph(
    graph: &RoadGraph,
    min_radius: &dyn Fn(&Stretch) -> f64,
    params: &SmoothParams,
) -> (RoadGraph, SmoothReport) {
    smooth_graph_with(graph, min_radius, &|_| false, params, WIDEN_ENABLED)
}

/// [`smooth_graph`] that knows where the water is: a zone whose widened line has a sample in water that the
/// original line did not have goes back to the original line and is flagged
/// [`WidenedZone::blocked_by_water`].
#[must_use]
pub fn smooth_graph_in_water(
    graph: &RoadGraph,
    min_radius: &dyn Fn(&Stretch) -> f64,
    in_water: WaterTest<'_>,
    params: &SmoothParams,
) -> (RoadGraph, SmoothReport) {
    smooth_graph_with(graph, min_radius, in_water, params, WIDEN_ENABLED)
}

/// [`smooth_graph`] without the widening pass: what the fit alone gives.
#[must_use]
pub fn smooth_graph_fit_only(
    graph: &RoadGraph,
    min_radius: &dyn Fn(&Stretch) -> f64,
    params: &SmoothParams,
) -> (RoadGraph, SmoothReport) {
    smooth_graph_with(graph, min_radius, &|_| false, params, false)
}

/// Whether a node inside the chain (between two links) lies elsewhere in the written line than in the original one.
fn inner_node_moved(
    plan: &ChainPlan,
    old_bounds: &[usize],
    new_bounds: &[usize],
    before: &[(f64, f64)],
    after: &[(f64, f64)],
) -> bool {
    old_bounds
        .iter()
        .zip(new_bounds)
        .take(plan.links.len().saturating_sub(1))
        .any(|(&k, &m)| before.get(k) != after.get(m))
}

/// The points of all stretches.
fn point_count(graph: &RoadGraph) -> usize {
    graph.stretches.iter().map(|s| s.points.len()).sum()
}

/// The nodes at an end of a bridge stretch.
fn bridge_end_nodes(graph: &RoadGraph) -> BTreeSet<i64> {
    graph
        .stretches
        .iter()
        .filter(|s| s.bridge)
        .flat_map(|s| [s.start, s.end])
        .collect()
}

fn smooth_graph_with(
    graph: &RoadGraph,
    min_radius: &dyn Fn(&Stretch) -> f64,
    in_water: WaterTest<'_>,
    params: &SmoothParams,
    widen_on: bool,
) -> (RoadGraph, SmoothReport) {
    let plans = plan_chains(graph);
    let family_minimum: Vec<f64> = graph.stretches.iter().map(min_radius).collect();
    let bridge_nodes = bridge_end_nodes(graph);
    let context = Context {
        bridge_nodes: &bridge_nodes,
        in_water,
    };
    let solved: Vec<Solved> = plans
        .par_iter()
        .map(|plan| solve_chain(graph, plan, &family_minimum, params, widen_on, &context))
        .collect();
    let mut result = graph.clone();
    let mut tally = Tally::new(graph);
    let mut fitted_points: Vec<usize> = graph.stretches.iter().map(|s| s.points.len()).collect();
    let mut node_moved = false;
    for (plan, solved) in plans.iter().zip(&solved) {
        let Solved {
            prepared,
            fitted,
            link_minimum,
            widened,
            ..
        } = solved;
        let before: Vec<(f64, f64)> = prepared.samples.iter().map(|s| s.at).collect();
        let tight_before = tight_lengths(&before, &prepared.bounds, link_minimum);
        let (fit_line, failure) = match fitted {
            Ok(f) => (f.after.clone(), None),
            Err(e) => (before.clone(), Some(e)),
        };
        // the line that is written back, where its links end, which samples lie in a zone and where each
        // sample came from in the fitted line
        let Written {
            after,
            bounds,
            mask,
            origin,
            signed,
            widen_error,
        } = written(widened.as_ref(), &fit_line, &prepared.bounds);
        if let Some(w) = widened {
            tally.zones.extend(chain_zones(graph, plan, w));
        }
        let (measured, measured_bounds) = resampled_by_link(&after, &bounds);
        let tight_after = tight_lengths(&measured, &measured_bounds, link_minimum);
        // A stretch that is not sound is never joined to a neighbour, so its chain has one link. The
        // condition still covers every link, so a chain of sound stretches is counted as short as a whole.
        let all_sound = plan
            .links
            .iter()
            .all(|l| graph.stretches.get(l.stretch).is_some_and(sound));
        let mut first = 0;
        for (j, (link, &last)) in plan.links.iter().zip(&bounds).enumerate() {
            if let Some(error) = failure {
                let short = matches!(error, SmoothError::TooShort(_));
                if short && all_sound {
                    tally.skipped_short += 1;
                } else if let Some(k) = tally.kept.get_mut(link.stretch) {
                    *k = Some(if short {
                        unsound_reason(graph.stretches.get(link.stretch))
                    } else {
                        error.to_string()
                    });
                }
            } else if let Some(s) = result.stretches.get_mut(link.stretch) {
                let mut line: Vec<(f64, f64)> = after
                    .get(first..=last)
                    .map(<[(f64, f64)]>::to_vec)
                    .unwrap_or_default();
                if link.reversed {
                    line.reverse();
                }
                if let Some(n) = fitted_points.get_mut(link.stretch) {
                    *n = line.len();
                }
                s.points = simplify_line(&line, params.simplify);
            }
            if let Some(e) = widen_error
                && let Some(w) = tally.widen_failed.get_mut(link.stretch)
            {
                *w = Some(e.to_string());
            }
            if let Some(entry) = tally.per_stretch.get_mut(link.stretch) {
                let (moved, outward) =
                    link_moves(first..=last, &after, &mask, &origin, &before, &signed);
                entry.max_move_m = moved;
                entry.max_outward_m = outward;
                entry.tight_before_m = tight_before.get(j).copied().unwrap_or(0.0);
                entry.tight_after_m = tight_after.get(j).copied().unwrap_or(0.0);
            }
            first = last;
        }
        // a node inside the chain that moved: the crossings must follow
        node_moved |= inner_node_moved(plan, &prepared.bounds, &bounds, &before, &after);
    }
    if node_moved {
        result.crossings = crossings_of(&result.stretches);
    }
    let mut report = tally.report(graph, plans.len());
    report.points_before = point_count(graph);
    report.points_fitted = fitted_points.iter().sum();
    report.points_after = point_count(&result);
    (result, report)
}

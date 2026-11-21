// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The second pass of the smoothing: tight curves are widened outward.
//!
//! The fit ([`super::smooth_line`]) only moves a sample sideways and pulls a curve inward, so a bend that the
//! road pieces cannot build stays unbuildable. This pass takes the fitted line of one chain, finds the *tight
//! runs* (samples whose radius is below the family's minimum), puts a *zone* around each (the run plus
//! `WIDEN_LEAD · minimum` of road on both sides; zones that touch are one) and solves a second convex
//! problem in which the samples of a zone move only outward.
//!
//! # The problem of one pass
//!
//! * **Unknowns.** One offset `e_i` per sample, along the *outward* normal `o_i = σ_i · n_i` of the line at the
//!   start of the pass (`σ_i = −1` where the bend turns left, `+1` where it turns right; at a sample of a
//!   merged zone, that of the nearest tight run), so `q_i = p_i + e_i · o_i`. Then one slack `s_i ≥ 0` for every
//!   sample that has a radius bound.
//! * **Rows.** Only a sample of a zone that may move has a free `e_i`: its bounds are `max(0, −t_i) ≤ e_i ≤
//!   max(·, REACH · minimum − t_i)` with `t_i = (p_i − orig_i) · o_i`, the outward distance the fit already
//!   put it at from its *original* position, so the result is never inward of the original line and never
//!   further out than `WIDEN_REACH · minimum`. Every other sample is held (`e_i = 0`): so are the first and
//!   last sample of the chain, the sample next to a junction, and the nodes of a bridge. A node where a
//!   road merely continues is free inside a zone. For every sample of a zone and the one on each side of
//!   it, the second difference `v = q_{i−1} − 2·q_i + q_{i+1}` is bounded by `WIDEN_DIRECTIONS` rows
//!   `u_k · v ≤ h² / (1.08 · minimum) + s_i`, `u_k` evenly spaced unit vectors, `h` the mean of the two
//!   segment lengths at the start of the pass.
//! * **Open corners.** A zone around a tight run that turns by at most `WIDEN_INWARD_MAX_DEG` degrees in total may
//!   also cut inward, up to `R · (sec(θ / 2) − 1)` (see [`inward_allowance`]); in the first pass such a zone moves
//!   inward only (measured: when both directions are free the cheapest way to meet the chord-length bound of the
//!   first pass is to push the legs 9 m outward), the later passes allow both.
//! * **Objective.** The fit's `Σ c_i |v_i|²` over the whole chain, `W_STAY · Σ e_i²`, and
//!   `WIDEN_SLACK_COST · Σ s_i`.
//!
//! Moving outward only makes the segments longer, so the bound taken from the start of the pass is on the safe
//! side, but a wider curve needs more samples. Before every pass after the first, the samples of the zones are
//! therefore resampled at `SAMPLE_M` between the samples that stay (the ends of a zone, held samples, nodes), so
//! the held samples and the nodes keep their identity. Passes repeat up to `WIDEN_PASSES` times, or until no
//! sample of the chain is tighter than its minimum. A zone found in a later pass is added to the earlier ones.
//! The original line is the reference for "not inward": the outward distance of a sample is measured to the
//! nearest point of it.

use std::collections::BTreeMap;
use std::f64::consts::TAU;

use super::{
    BRIDGE_STRAIGHT_WEIGHT, SAMPLE_M, SmoothError, W_STAY, WIDEN_DIRECTIONS, WIDEN_INWARD_MAX_DEG,
    WIDEN_KEEP_FRACTION, WIDEN_LEAD, WIDEN_PASSES, WIDEN_REACH, WIDEN_SLACK_COST, normals,
    radius_at, resample, sample_minimum,
};
use crate::qp::{Problem, Row, solve};

type P = (f64, f64);

/// A zone counts as reached when no sample is tighter than this fraction of its minimum. The polygon that
/// bounds the second difference lets the length exceed the bound by at most 8.3 %, and the bound is taken
/// 8 % lower, so a solved zone is within 0.3 % of the minimum.
pub(super) const REACHED_FRACTION: f64 = 0.99;

/// The spec takes the bound on the second difference this much lower than the polygon's circle, because the
/// polygon's corners are `1 / cos(π / WIDEN_DIRECTIONS)` further out.
const POLYGON_MARGIN: f64 = 1.08;

/// What the widening of one chain needs.
pub(super) struct ChainLine<'a> {
    /// The original line as the stretches have it.
    pub original: &'a [P],
    /// The fitted line.
    pub fitted: &'a [P],
    /// The index in the fitted line of the last sample of every link.
    pub bounds: &'a [usize],
    /// The minimum radius of every link.
    pub link_minimum: &'a [f64],
    /// Whether every link is a bridge.
    pub link_bridge: &'a [bool],
    /// Whether a sample of the fitted line may move when it lies in a zone.
    pub movable: &'a [bool],
}

/// What widening a chain gives.
pub(super) struct Outcome {
    /// The widened line (the fitted line if the solver failed).
    pub line: Vec<P>,
    /// The index in `line` of the last sample of every link.
    pub bounds: Vec<usize>,
    /// The samples that lie in a zone.
    pub mask: Vec<bool>,
    /// For every sample, its index in the fitted line, or `None` for a sample that was added.
    pub origin: Vec<Option<usize>>,
    /// For every sample of a zone, how far outward of the original line it lies (negative: inward).
    pub signed: Vec<f64>,
    /// The zones, as they were judged; a zone that went back to the fit is among them.
    pub zones: Vec<ZoneFigures>,
    /// Why the solver failed, if it did.
    pub error: Option<SmoothError>,
}

/// A minimum radius that can be used.
pub(super) fn usable(minimum: f64) -> bool {
    minimum.is_finite() && minimum > 0.0
}

/// The state of a chain between passes.
struct State {
    line: Vec<P>,
    bounds: Vec<usize>,
    mask: Vec<bool>,
    /// `-1` or `1`: the outward side in terms of the left normal.
    side: Vec<f64>,
    /// How far inward of the original line a sample of a zone may go, in metres.
    inward: Vec<f64>,
    movable: Vec<bool>,
    origin: Vec<Option<usize>>,
}

impl State {
    fn minimum(&self, c: &ChainLine<'_>) -> Vec<f64> {
        sample_minimum(self.line.len(), &self.bounds, c.link_minimum)
    }
}

/// The inclusive index ranges of consecutive samples whose radius is below their minimum.
fn tight_runs(points: &[P], minimum: &[f64]) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (k, w) in points.windows(3).enumerate() {
        let index = k + 1;
        let tight = matches!((w, minimum.get(index)), ([prev, mid, next], Some(&min))
            if usable(min) && radius_at(*prev, *mid, *next) < min);
        if !tight {
            continue;
        }
        match runs.last_mut() {
            Some(run) if run.1 + 1 == index => run.1 = index,
            _ => runs.push((index, index)),
        }
    }
    runs
}

/// The signed turn at sample `i`, in radians (left positive).
fn turn_at(points: &[P], index: usize) -> f64 {
    let (Some(prev), Some(mid), Some(next)) = (
        index.checked_sub(1).and_then(|k| points.get(k)),
        points.get(index),
        points.get(index + 1),
    ) else {
        return 0.0;
    };
    let incoming = (mid.0 - prev.0, mid.1 - prev.1);
    let outgoing = (next.0 - mid.0, next.1 - mid.1);
    (incoming.0 * outgoing.1 - incoming.1 * outgoing.0)
        .atan2(incoming.0 * outgoing.0 + incoming.1 * outgoing.1)
}

/// Adds the zones of the tight runs of the line to the mask and gives every sample of them the side that is
/// outward (`-1` for a left-turning bend, `1` otherwise).
fn mark_zones(st: &mut State, minimum: &[f64], fitted: bool) {
    let old_mask = st.mask.clone();
    let points = &st.line;
    let runs = tight_runs(points, minimum);
    let mut arc = Vec::with_capacity(points.len());
    let mut total = 0.0;
    arc.push(0.0);
    for w in points.windows(2) {
        if let [a, b] = w {
            total += (b.0 - a.0).hypot(b.1 - a.1);
        }
        arc.push(total);
    }
    let at = |i: usize| arc.get(i).copied().unwrap_or(0.0);
    // for every sample of this pass: the distance to its nearest tight run and that run's side
    let mut best: Vec<Option<(usize, f64, f64)>> = vec![None; points.len()];
    for &(a, b) in &runs {
        let m = minimum
            .get(a..=b)
            .into_iter()
            .flatten()
            .copied()
            .fold(f64::INFINITY, f64::min);
        let reach = WIDEN_LEAD * m;
        let turn: f64 = (a..=b).map(|i| turn_at(points, i)).sum();
        let sigma = if turn > 0.0 { -1.0 } else { 1.0 };
        let mut lo = a;
        while lo > 0 && at(a) - at(lo - 1) <= reach {
            lo -= 1;
        }
        let mut hi = b;
        while hi + 1 < points.len() && at(hi + 1) - at(b) <= reach {
            hi += 1;
        }
        // only the fitted line tells what the bend is: a zone found later has seen part of it widened (its new
        // samples take the allowance of the zone they grow from, below)
        let inward = if fitted {
            inward_allowance(points, lo, hi, m)
        } else {
            0.0
        };
        for i in lo..=hi {
            let d = if i < a { a - i } else { i.saturating_sub(b) };
            if let Some(slot) = best.get_mut(i)
                && slot.is_none_or(|(old, _, _)| d < old)
            {
                *slot = Some((d, sigma, inward));
            }
        }
    }
    for (((mask, side), inward), found) in st
        .mask
        .iter_mut()
        .zip(st.side.iter_mut())
        .zip(st.inward.iter_mut())
        .zip(&best)
    {
        // a sample that is in a zone already keeps its side: a reverse bend that an earlier pass made to
        // leave the zone must not turn the zone around
        if let Some((_, sigma, allowance)) = found
            && !*mask
        {
            *mask = true;
            *side = *sigma;
            *inward = *allowance;
        }
    }
    if !fitted {
        // samples that joined a zone take the allowance of the nearest sample that was in one already
        let mut carried = 0.0;
        let mut from_left = vec![0.0; st.mask.len()];
        for (k, slot) in from_left.iter_mut().enumerate() {
            if old_mask.get(k).copied().unwrap_or(false) {
                carried = st.inward.get(k).copied().unwrap_or(0.0);
            }
            *slot = carried;
        }
        carried = 0.0;
        for k in (0..st.mask.len()).rev() {
            if old_mask.get(k).copied().unwrap_or(false) {
                carried = st.inward.get(k).copied().unwrap_or(0.0);
            } else if st.mask.get(k).copied().unwrap_or(false)
                && let Some(slot) = st.inward.get_mut(k)
            {
                *slot = carried.max(from_left.get(k).copied().unwrap_or(0.0));
            }
        }
    }
}

/// How far inward of the original line a zone may go. Let `theta` be the net turn of the fitted line over the
/// zone (`first..=last`: the absolute value of the summed signed turns, so the kinks of resampling and the two
/// bends of an S-bend do not add up). If it is at most `WIDEN_INWARD_MAX_DEG` degrees the zone is an open corner,
/// and a fillet of the minimum radius `minimum` comes as close as `minimum * (1 - cos(theta / 2))` to the legs
/// (the distance from the legs, which is what the rows measure; not from the corner's vertex). A bend that turns
/// more is widened outward only.
fn inward_allowance(points: &[P], first: usize, last: usize, minimum: f64) -> f64 {
    let theta: f64 = (first..=last)
        .map(|i| turn_at(points, i))
        .sum::<f64>()
        .abs();
    if theta.to_degrees() <= WIDEN_INWARD_MAX_DEG {
        minimum * (1.0 - (theta / 2.0).cos())
    } else {
        0.0
    }
}

/// The nearest point of `line` to `p`.
fn nearest(line: &[P], point: P) -> P {
    let mut best = (f64::INFINITY, point);
    for w in line.windows(2) {
        let [from, to] = w else { continue };
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let len2 = dx * dx + dy * dy;
        let along = (((point.0 - from.0) * dx + (point.1 - from.1) * dy)
            / len2.max(f64::MIN_POSITIVE))
        .clamp(0.0, 1.0);
        let foot = (from.0 + dx * along, from.1 + dy * along);
        let gap = (point.0 - foot.0).hypot(point.1 - foot.1);
        if gap < best.0 {
            best = (gap, foot);
        }
    }
    best.1
}

fn angle_of(k: usize) -> f64 {
    let k = f64::from(u32::try_from(k).unwrap_or(u32::MAX));
    let of = f64::from(u32::try_from(WIDEN_DIRECTIONS).unwrap_or(u32::MAX));
    TAU * k / of
}

/// The weights `c_i` of the bends: bridge links are stiff inside.
fn stiffness_of(count: usize, bounds: &[usize], bridge: &[bool]) -> Vec<f64> {
    let mut out = vec![1.0; count];
    let mut first = 0;
    for (&last, &is_bridge) in bounds.iter().zip(bridge) {
        if is_bridge {
            for k in first + 1..last {
                if let Some(c) = out.get_mut(k) {
                    *c = BRIDGE_STRAIGHT_WEIGHT;
                }
            }
        }
        first = last;
    }
    out
}

/// The unit directions of the zone samples (outward) and what the pass knows about the line.
struct Pass<'a> {
    points: &'a [P],
    dir: Vec<P>,
    minimum: Vec<f64>,
    /// Whether a sample is free to move in this pass.
    active: Vec<bool>,
    /// How far inward of the original line a sample may go.
    inward: &'a [f64],
    opening: bool,
    /// The samples that have a radius bound: a zone and one sample on each side of it.
    bounded: Vec<usize>,
}

impl Pass<'_> {
    fn dir_at(&self, index: usize) -> P {
        self.dir.get(index).copied().unwrap_or_default()
    }

    fn point_at(&self, index: usize) -> P {
        self.points.get(index).copied().unwrap_or_default()
    }

    fn minimum_at(&self, index: usize) -> f64 {
        self.minimum.get(index).copied().unwrap_or(f64::NAN)
    }

    /// The second difference of the line at `index` (not the first or last sample).
    fn second(&self, index: usize) -> P {
        let (prev, mid, next) = (
            self.point_at(index - 1),
            self.point_at(index),
            self.point_at(index + 1),
        );
        (prev.0 - 2.0 * mid.0 + next.0, prev.1 - 2.0 * mid.1 + next.1)
    }

    /// The bounds of the offset of a sample: held, or between "not inward of the original line" and
    /// [`WIDEN_REACH`] minimum radii outward of it.
    fn limits(&self, chain: &ChainLine<'_>, index: usize) -> (f64, f64) {
        if !self.active.get(index).copied().unwrap_or(false) {
            return (0.0, 0.0);
        }
        let point = self.point_at(index);
        let foot = nearest(chain.original, point);
        let dir = self.dir_at(index);
        // how far outward of the original line the sample already is
        let out = (point.0 - foot.0) * dir.0 + (point.1 - foot.1) * dir.1;
        let lowest = -out - self.inward.get(index).copied().unwrap_or(0.0);
        let reach = if self.opening && self.inward.get(index).copied().unwrap_or(0.0) > 0.0 {
            0.0
        } else {
            WIDEN_REACH * self.minimum_at(index) - out
        };
        (lowest, lowest.max(reach))
    }

    /// The rows that bound the second difference at `index` by `WIDEN_DIRECTIONS` projections, with the
    /// slack unknown `slack`.
    fn radius_rows(&self, index: usize, slack: usize) -> Vec<Row> {
        let (prev, mid, next) = (
            self.point_at(index - 1),
            self.point_at(index),
            self.point_at(index + 1),
        );
        let step = f64::midpoint(
            (mid.0 - prev.0).hypot(mid.1 - prev.1),
            (next.0 - mid.0).hypot(next.1 - mid.1),
        );
        let bound = step * step / (self.minimum_at(index) * POLYGON_MARGIN);
        let second = self.second(index);
        let (dir_prev, dir_mid, dir_next) = (
            self.dir_at(index - 1),
            self.dir_at(index),
            self.dir_at(index + 1),
        );
        (0..WIDEN_DIRECTIONS)
            .map(|k| {
                let u = (angle_of(k).cos(), angle_of(k).sin());
                let along = |d: P| u.0 * d.0 + u.1 * d.1;
                Row {
                    terms: vec![
                        (index - 1, along(dir_prev)),
                        (index, -2.0 * along(dir_mid)),
                        (index + 1, along(dir_next)),
                        (slack, -1.0),
                    ],
                    lo: f64::NEG_INFINITY,
                    hi: bound - (u.0 * second.0 + u.1 * second.1),
                }
            })
            .collect()
    }
}

/// One pass: the line after the zone samples have moved outward as far as the problem asks.
fn solve_pass(chain: &ChainLine<'_>, st: &State, first_pass: bool) -> Result<Vec<P>, SmoothError> {
    let points = &st.line;
    let count = points.len();
    let dir: Vec<P> = normals(points)
        .iter()
        .zip(&st.side)
        .map(|(normal, side)| (normal.0 * side, normal.1 * side))
        .collect();
    let minimum = st.minimum(chain);
    let in_zone = |index: usize| st.mask.get(index).copied().unwrap_or(false);
    let usable_at = |index: usize| minimum.get(index).copied().is_some_and(usable);
    let active: Vec<bool> = (0..count)
        .map(|index| {
            in_zone(index) && st.movable.get(index).copied().unwrap_or(false) && usable_at(index)
        })
        .collect();
    let bounded: Vec<usize> = (1..count.saturating_sub(1))
        .filter(|&index| {
            usable_at(index) && (in_zone(index) || in_zone(index - 1) || in_zone(index + 1))
        })
        .collect();
    let pass = Pass {
        points,
        dir,
        minimum,
        active,
        inward: &st.inward,
        opening: first_pass,
        bounded,
    };
    let stiffness = stiffness_of(count, &st.bounds, chain.link_bridge);
    let mut problem = Problem::new(count + pass.bounded.len());
    for index in 1..count.saturating_sub(1) {
        let (prev, mid, next) = (
            pass.dir_at(index - 1),
            pass.dir_at(index),
            pass.dir_at(index + 1),
        );
        let second = pass.second(index);
        let weight = stiffness.get(index).copied().unwrap_or(1.0);
        problem.add_square(
            weight,
            &[
                (index - 1, prev.0),
                (index, -2.0 * mid.0),
                (index + 1, next.0),
            ],
            second.0,
        );
        problem.add_square(
            weight,
            &[
                (index - 1, prev.1),
                (index, -2.0 * mid.1),
                (index + 1, next.1),
            ],
            second.1,
        );
    }
    let mut limits = Vec::with_capacity(count);
    for index in 0..count {
        problem.add_square(W_STAY, &[(index, 1.0)], 0.0);
        let (lo, hi) = pass.limits(chain, index);
        limits.push((lo, hi));
        problem.rows.push(Row {
            terms: vec![(index, 1.0)],
            lo,
            hi,
        });
    }
    for (k, &index) in pass.bounded.iter().enumerate() {
        let slack = count + k;
        problem.rows.push(Row {
            terms: vec![(slack, 1.0)],
            lo: 0.0,
            hi: f64::INFINITY,
        });
        problem.add_linear(WIDEN_SLACK_COST, slack);
        problem.rows.extend(pass.radius_rows(index, slack));
    }
    let x = solve(&problem).map_err(|e| SmoothError::Solver(e.to_string()))?;
    if x.len() != problem.unknowns || x.iter().any(|v| !v.is_finite()) {
        return Err(SmoothError::Solver(
            "the solution is not finite".to_string(),
        ));
    }
    Ok((0..count)
        .map(|index| {
            let point = pass.point_at(index);
            if !pass.active.get(index).copied().unwrap_or(false) {
                return point;
            }
            let (lo, hi) = limits.get(index).copied().unwrap_or((0.0, 0.0));
            let offset = x.get(index).copied().unwrap_or(0.0).clamp(lo, hi);
            let dir = pass.dir_at(index);
            (point.0 + offset * dir.0, point.1 + offset * dir.1)
        })
        .collect())
}

/// The state with the samples inside zones resampled at [`SAMPLE_M`] between the samples that stay: the ends of
/// a zone, samples that may not move, and the nodes between links.
fn resampled(st: &State) -> State {
    let n = st.line.len();
    let is_bound = |i: usize| st.bounds.contains(&i);
    let mask_at = |i: usize| st.mask.get(i).copied().unwrap_or(false);
    let keep = |i: usize| {
        i == 0
            || i + 1 >= n
            || !(mask_at(i - 1) && mask_at(i) && mask_at(i + 1))
            || !st.movable.get(i).copied().unwrap_or(false)
            || is_bound(i)
    };
    let mut out = State {
        line: Vec::with_capacity(n),
        bounds: Vec::new(),
        mask: Vec::with_capacity(n),
        side: Vec::with_capacity(n),
        inward: Vec::with_capacity(n),
        movable: Vec::with_capacity(n),
        origin: Vec::with_capacity(n),
    };
    let mut new_index: Vec<Option<usize>> = vec![None; n];
    let push_kept = |out: &mut State, new_index: &mut Vec<Option<usize>>, i: usize| {
        if let Some(slot) = new_index.get_mut(i) {
            *slot = Some(out.line.len());
        }
        out.line.push(st.line.get(i).copied().unwrap_or_default());
        out.mask.push(mask_at(i));
        out.side.push(st.side.get(i).copied().unwrap_or(1.0));
        out.inward.push(st.inward.get(i).copied().unwrap_or(0.0));
        out.movable
            .push(st.movable.get(i).copied().unwrap_or(false));
        out.origin.push(st.origin.get(i).copied().flatten());
    };
    push_kept(&mut out, &mut new_index, 0);
    let mut a = 0;
    while a + 1 < n {
        let mut b = a + 1;
        while b + 1 < n && !keep(b) {
            b += 1;
        }
        if b > a + 1 {
            let segment = st.line.get(a..=b).unwrap_or_default();
            let rs = resample(segment, SAMPLE_M);
            let side = st.side.get(a).copied().unwrap_or(1.0);
            let inward = st.inward.get(a).copied().unwrap_or(0.0);
            if let Some(inner) = rs.get(1..rs.len().saturating_sub(1)) {
                for &p in inner {
                    out.line.push(p);
                    out.mask.push(true);
                    out.side.push(side);
                    out.inward.push(inward);
                    out.movable.push(true);
                    out.origin.push(None);
                }
            }
        }
        push_kept(&mut out, &mut new_index, b);
        a = b;
    }
    out.bounds = st
        .bounds
        .iter()
        .filter_map(|&k| new_index.get(k).copied().flatten())
        .collect();
    out
}

/// Widens the zones of one chain; `None` when the fitted line has no tight run (nothing to do).
pub(super) fn widen_chain(c: &ChainLine<'_>) -> Option<Outcome> {
    let n = c.fitted.len();
    let mut st = State {
        line: c.fitted.to_vec(),
        bounds: c.bounds.to_vec(),
        mask: vec![false; n],
        side: vec![1.0; n],
        inward: vec![0.0; n],
        movable: c.movable.to_vec(),
        origin: (0..n).map(Some).collect(),
    };
    let mut first: Option<Vec<bool>> = None;
    for pass in 0..WIDEN_PASSES {
        if tight_runs(&st.line, &st.minimum(c)).is_empty() {
            break;
        }
        if pass > 0 {
            st = resampled(&st);
        }
        let minimum = st.minimum(c);
        mark_zones(&mut st, &minimum, pass == 0);
        if first.is_none() {
            first = Some(st.mask.clone());
        }
        match solve_pass(c, &st, pass == 0) {
            Ok(next) => st.line = next,
            Err(e) => {
                // the chain keeps the fit; the zones are those of the first pass, judged on the fit
                let failed = State {
                    mask: first.unwrap_or_else(|| vec![false; n]),
                    ..whole_fit(c)
                };
                return Some(finish(c, failed, Some(e)));
            }
        }
    }
    first.map(|_| finish(c, st, None))
}

/// What the solver's answer comes to: a zone that stays far from its minimum goes back to the fit.
fn finish(c: &ChainLine<'_>, st: State, error: Option<SmoothError>) -> Outcome {
    let judged = error.is_none();
    let mut zones = zone_figures(c, &st, judged);
    if !judged {
        // the chain is the fit: its zone samples are not a move of the widening (and their sides are not known)
        for z in &mut zones {
            z.max_outward = 0.0;
            z.max_inward = 0.0;
        }
    }
    let mut st = st;
    let reverted: Vec<(usize, usize)> = zones
        .iter()
        .filter(|z| z.plate_candidate)
        .map(|z| z.range)
        .collect();
    if !reverted.is_empty() {
        st = revert(c, &st, &reverted);
        for z in zones.iter_mut().filter(|z| z.plate_candidate) {
            z.max_outward = 0.0;
            z.max_inward = 0.0;
        }
    }
    let signed = signed_outward(c, &st);
    Outcome {
        line: st.line,
        bounds: st.bounds,
        mask: st.mask,
        origin: st.origin,
        signed,
        zones,
        error,
    }
}

/// The state with the zones `ranges` (inclusive sample ranges) put back to the fitted samples they grew from:
/// the samples on both sides of a zone are held, so they are the fit's and the result is seamless.
fn revert(c: &ChainLine<'_>, st: &State, ranges: &[(usize, usize)]) -> State {
    let n = st.line.len();
    let mut out = State {
        line: Vec::with_capacity(n),
        bounds: Vec::new(),
        mask: Vec::with_capacity(n),
        side: Vec::with_capacity(n),
        inward: Vec::with_capacity(n),
        movable: Vec::with_capacity(n),
        origin: Vec::with_capacity(n),
    };
    let origin_at = |i: usize| st.origin.get(i).copied().flatten();
    // The samples on both sides of a zone lie outside every mask, and a sample without an origin only exists
    // inside a mask (masks only grow), so both have an origin. Should that ever fail, the whole chain goes back
    // to the fit rather than leaving a zone that is flagged as reverted half widened.
    let Some(spans) = ranges
        .iter()
        .map(|&(lo, hi)| {
            let start = lo
                .checked_sub(1)
                .map_or(Some(0), |b| origin_at(b).map(|o| o + 1))?;
            let end = if hi + 1 >= n {
                Some(c.fitted.len())
            } else {
                origin_at(hi + 1)
            }?;
            Some((lo, hi, start, end))
        })
        .collect::<Option<Vec<_>>>()
    else {
        return whole_fit(c);
    };
    let mut i = 0;
    while i < n {
        if let Some(&(_, hi, start, end)) = spans.iter().find(|s| s.0 == i) {
            for k in start..end {
                out.line.push(c.fitted.get(k).copied().unwrap_or_default());
                out.mask.push(false);
                out.side.push(1.0);
                out.inward.push(0.0);
                out.movable.push(c.movable.get(k).copied().unwrap_or(false));
                out.origin.push(Some(k));
            }
            i = hi + 1;
        } else {
            out.line.push(st.line.get(i).copied().unwrap_or_default());
            out.mask.push(st.mask.get(i).copied().unwrap_or(false));
            out.side.push(st.side.get(i).copied().unwrap_or(1.0));
            out.inward.push(st.inward.get(i).copied().unwrap_or(0.0));
            out.movable
                .push(st.movable.get(i).copied().unwrap_or(false));
            out.origin.push(st.origin.get(i).copied().flatten());
            i += 1;
        }
    }
    let position: BTreeMap<usize, usize> = out
        .origin
        .iter()
        .enumerate()
        .filter_map(|(at, o)| o.map(|fit| (fit, at)))
        .collect();
    out.bounds = c
        .bounds
        .iter()
        .filter_map(|b| position.get(b).copied())
        .collect();
    out
}

/// The fitted line as a state without zones.
fn whole_fit(c: &ChainLine<'_>) -> State {
    let n = c.fitted.len();
    State {
        line: c.fitted.to_vec(),
        bounds: c.bounds.to_vec(),
        mask: vec![false; n],
        side: vec![1.0; n],
        inward: vec![0.0; n],
        movable: c.movable.to_vec(),
        origin: (0..n).map(Some).collect(),
    }
}

/// How far outward of the original line every sample of a zone lies (negative: inward); 0 outside zones.
fn signed_outward(c: &ChainLine<'_>, st: &State) -> Vec<f64> {
    let normal = normals(&st.line);
    st.line
        .iter()
        .enumerate()
        .map(|(k, &p)| {
            if !st.mask.get(k).copied().unwrap_or(false) {
                return 0.0;
            }
            let foot = nearest(c.original, p);
            let (nm, side) = (
                normal.get(k).copied().unwrap_or_default(),
                st.side.get(k).copied().unwrap_or(1.0),
            );
            (p.0 - foot.0) * nm.0 * side + (p.1 - foot.1) * nm.1 * side
        })
        .collect()
}

/// What to do with a zone after the last pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verdict {
    /// No sample is tighter than the minimum (within `REACHED_FRACTION`).
    Reached,
    /// Tighter than wanted, but not by much: the widened line is kept.
    Kept,
    /// Below `WIDEN_KEEP_FRACTION` of the minimum: it goes back to the fit.
    Revert,
}

/// Judges a zone from its smallest radius and the one it wanted.
pub(super) fn verdict(radius_after: f64, wanted: f64, reached: bool) -> Verdict {
    if reached {
        Verdict::Reached
    } else if radius_after < wanted * WIDEN_KEEP_FRACTION {
        Verdict::Revert
    } else {
        Verdict::Kept
    }
}

/// One zone, as indices into the widened line.
pub(super) struct ZoneFigures {
    /// The first and the last sample of the zone.
    pub range: (usize, usize),
    /// The links the zone covers, as indices in chain order.
    pub links: Vec<usize>,
    /// The tightest sample of the fitted line.
    pub at: P,
    pub radius_before: f64,
    pub radius_after: f64,
    pub wanted: f64,
    pub max_outward: f64,
    pub max_inward: f64,
    pub reached: bool,
    pub plate_candidate: bool,
}

fn radius_of(points: &[P], index: usize) -> f64 {
    match (
        index.checked_sub(1).and_then(|k| points.get(k)),
        points.get(index),
        points.get(index + 1),
    ) {
        (Some(prev), Some(mid), Some(next)) => radius_at(*prev, *mid, *next),
        _ => f64::INFINITY,
    }
}

/// The links of a chain whose samples overlap `range` (by at least a segment, if the range has one).
fn links_of(bounds: &[usize], range: (usize, usize)) -> Vec<usize> {
    let (lo, hi) = range;
    let mut first = 0;
    let mut out = Vec::new();
    for (link, &last) in bounds.iter().enumerate() {
        let overlap = hi.min(last) > lo.max(first) || (hi == lo && lo >= first && lo <= last);
        if overlap {
            out.push(link);
        }
        first = last;
    }
    out
}

/// The figures of every maximal run of the mask of `st`. `judge` sets `plate_candidate` for the zones that are
/// to go back to the fit.
fn zone_figures(chain: &ChainLine<'_>, st: &State, judge: bool) -> Vec<ZoneFigures> {
    let fit_minimum = sample_minimum(chain.fitted.len(), chain.bounds, chain.link_minimum);
    let minimum = sample_minimum(st.line.len(), &st.bounds, chain.link_minimum);
    let signed = signed_outward(chain, st);
    let mut out = Vec::new();
    let mut at = 0;
    while at < st.mask.len() {
        if !st.mask.get(at).copied().unwrap_or(false) {
            at += 1;
            continue;
        }
        let lo = at;
        while st.mask.get(at).copied().unwrap_or(false) {
            at += 1;
        }
        let hi = at - 1;
        // the part of the fitted line this zone grew from
        let from: Vec<usize> = st
            .origin
            .get(lo..=hi)
            .into_iter()
            .flatten()
            .filter_map(|o| *o)
            .collect();
        let (first, last) = (
            from.first().copied().unwrap_or(0),
            from.last().copied().unwrap_or(0),
        );
        let mut tightest: Option<(usize, f64)> = None;
        for k in first..=last {
            let m = fit_minimum.get(k).copied().unwrap_or(f64::NAN);
            let r = radius_of(chain.fitted, k);
            if usable(m) && tightest.is_none_or(|(_, old)| r.total_cmp(&old).is_lt()) {
                tightest = Some((k, r));
            }
        }
        // the samples the radius rows bound: the zone and the held sample on each side of it
        let mut radius_after = f64::INFINITY;
        let mut reached = true;
        for k in lo.saturating_sub(1)..=(hi + 1).min(st.line.len().saturating_sub(1)) {
            let m = minimum.get(k).copied().unwrap_or(f64::NAN);
            let r = radius_of(&st.line, k);
            radius_after = radius_after.min(r);
            if usable(m) && r < m * REACHED_FRACTION {
                reached = false;
            }
        }
        let moves = signed.get(lo..=hi).unwrap_or_default();
        let (k, radius_before) = tightest.unwrap_or((first, f64::INFINITY));
        let wanted = fit_minimum.get(k).copied().unwrap_or(f64::NAN);
        out.push(ZoneFigures {
            range: (lo, hi),
            links: links_of(&st.bounds, (lo, hi)),
            at: chain.fitted.get(k).copied().unwrap_or_default(),
            radius_before,
            radius_after,
            wanted,
            max_outward: moves.iter().copied().fold(0.0, f64::max),
            max_inward: moves.iter().map(|m| -m).fold(0.0, f64::max),
            reached,
            plate_candidate: judge && verdict(radius_after, wanted, reached) == Verdict::Revert,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Verdict, verdict};

    #[test]
    fn a_zone_is_judged_by_its_radius() {
        assert_eq!(verdict(10.0, 10.0, true), Verdict::Reached);
        // between 0.9 and 0.99 of the wanted radius the widened line is kept
        assert_eq!(verdict(9.5, 10.0, false), Verdict::Kept);
        assert_eq!(verdict(9.0, 10.0, false), Verdict::Kept);
        // below 0.9 it goes back to the fit
        assert_eq!(verdict(8.99, 10.0, false), Verdict::Revert);
        assert_eq!(verdict(1.0, 10.0, false), Verdict::Revert);
        // a wanted radius that is not a number never reverts
        assert_eq!(verdict(1.0, f64::NAN, false), Verdict::Kept);
    }
}

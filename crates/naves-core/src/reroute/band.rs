// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The feasibility band of road planning: along a candidate route, which road heights are reachable within a
//! grade limit and the allowed cut and fill, how much earthwork a real profile needs, and the grade limit of a leg.
//!
//! The spec is "The feasibility band and the profile" and "The grade a leg may have" in
//! `docs/superpowers/specs/2026-11-21-road-planning-design.md`. The road is pinned to the terrain at both anchors
//! (within [`Band::anchor_tol_m`]), so a route cannot put the climb of a hillside into embankments at its ends.

use crate::catalog::Family;
use crate::heightmap::Heightmap;
use crate::limits::{grade_aim, grade_ceiling};
use crate::reroute::RerouteParams;
use crate::reroute::raster::{Blocked, Clearance, Obstacles};

/// Tolerance of the interval arithmetic, in metres.
const SLACK: f64 = 1e-9;
/// Smallest and largest sample spacing, in metres.
const SPACING_RANGE: (f64, f64) = (0.1, 5.0);
/// Spacing used when the terrain cell is not a number, in metres.
const SPACING_DEFAULT: f64 = 2.0;
/// Most samples of one line.
const MAX_STEPS: f64 = 1.0e6;
/// Spacing of the samples of the drawn-grade windows, in metres.
const DRAWN_SPACING_M: f64 = 2.0;

/// The limits a road height is held to along a route.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    /// Largest cut (road below the terrain), in metres.
    pub cut_m: f64,
    /// Largest fill (road above the terrain), in metres.
    pub fill_m: f64,
    /// Largest grade (rise over run).
    pub grade: f64,
    /// Spacing of the samples along the route, in metres.
    pub spacing_m: f64,
    /// How far the road height at an anchor may be from the terrain, in metres.
    pub anchor_tol_m: f64,
}

impl Band {
    /// The band of the parameters for a grade limit and a terrain cell: the samples are half a cell apart,
    /// 0.1 to 5 m.
    #[must_use]
    pub fn from_params(params: &RerouteParams, grade: f64, cell_m: f64) -> Band {
        let spacing_m = if cell_m.is_finite() {
            (cell_m / 2.0).clamp(SPACING_RANGE.0, SPACING_RANGE.1)
        } else {
            SPACING_DEFAULT
        };
        Band {
            cut_m: params.cut_m,
            fill_m: params.fill_m,
            grade,
            spacing_m,
            anchor_tol_m: params.anchor_tol_m,
        }
    }
}

/// The interval of road heights reachable at a point, in metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reach {
    /// Lowest reachable height.
    pub lo: f64,
    /// Highest reachable height.
    pub hi: f64,
}

impl Reach {
    /// The road is at the height `z` within `tol`.
    #[must_use]
    pub fn pinned(z: f64, tol: f64) -> Reach {
        Reach {
            lo: z - tol,
            hi: z + tol,
        }
    }
}

/// What the band says of a step: the reach at its end, the climb and the earthwork it needs at least.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandStep {
    /// The reachable heights at the end of the step.
    pub reach: Reach,
    /// The rise and fall of the nominal height (the terrain clamped into the reach), in metres.
    pub climb_m: f64,
    /// The earthwork of the nominal height, in m². A lower bound of the earthwork of a real profile.
    pub earthwork_low_m2: f64,
}

/// Why a route fails the band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BandFail {
    /// A segment is blocked.
    #[error("blocked: {0:?}")]
    Blocked(Blocked),
    /// A point has no terrain height (outside the map, no data, not a number).
    #[error("no terrain height")]
    NoGround,
    /// The grade, cut and fill leave no reachable height.
    #[error("the grade limit leaves no reachable height")]
    Grade,
    /// The road does not reach the end anchor's height.
    #[error("the road does not reach the end anchor's height")]
    Arrival,
}

/// The terrain height at a map position, or `None` outside the map or where the height is not a number.
#[must_use]
pub fn ground_at(map: &Heightmap, p: (f64, f64)) -> Option<f64> {
    let extent = f64::from(map.size.saturating_sub(1)) * f64::from(map.cell_m);
    if !(p.0 >= 0.0 && p.0 <= extent && p.1 >= 0.0 && p.1 <= extent) {
        return None;
    }
    let h = f64::from(map.sample(p.0, p.1).height);
    h.is_finite().then_some(h)
}

/// A sample of a route: a position and the distance from the previous sample.
#[derive(Debug, Clone, Copy)]
struct At {
    p: (f64, f64),
    ds: f64,
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to 1..=MAX_STEPS first"
)]
fn step_count(v: f64) -> usize {
    v.clamp(1.0, MAX_STEPS) as usize
}

#[expect(
    clippy::cast_precision_loss,
    reason = "a count of at most MAX_STEPS is exact in f64"
)]
fn to_f64(n: usize) -> f64 {
    n as f64
}

/// The samples of a polyline: equally spaced, at most `spacing` apart, the end point last.
fn sample_line(line: &[(f64, f64)], spacing: f64) -> Result<Vec<At>, BandFail> {
    if line.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return Err(BandFail::NoGround);
    }
    let Some(&first) = line.first() else {
        return Ok(Vec::new());
    };
    let lengths: Vec<f64> = line
        .iter()
        .zip(line.iter().skip(1))
        .map(|(a, b)| (b.0 - a.0).hypot(b.1 - a.1))
        .collect();
    let total: f64 = lengths.iter().sum();
    let mut out = vec![At { p: first, ds: 0.0 }];
    if total <= 0.0 || !total.is_finite() {
        return Ok(out);
    }
    let spacing = if spacing.is_finite() && spacing > 0.0 {
        spacing
    } else {
        SPACING_DEFAULT
    };
    let n = step_count((total / spacing - SLACK).ceil());
    let ds = total / to_f64(n);
    let (mut seg, mut before) = (0_usize, 0.0_f64);
    for i in 1..=n {
        let target = to_f64(i) * ds;
        while let Some(&len) = lengths.get(seg)
            && before + len < target
            && seg + 1 < lengths.len()
        {
            before += len;
            seg += 1;
        }
        let end = line.get(seg + 1).copied();
        let p = match (line.get(seg).copied(), end, lengths.get(seg)) {
            (Some(a), Some(b), Some(&len)) if i < n && len > 0.0 => {
                let t = ((target - before) / len).clamp(0.0, 1.0);
                (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
            }
            _ => line.last().copied().unwrap_or(first),
        };
        out.push(At { p, ds });
    }
    Ok(out)
}

/// The terrain height at every sample.
fn heights_at(
    samples: &[At],
    ground: &dyn Fn((f64, f64)) -> Option<f64>,
) -> Result<Vec<f64>, BandFail> {
    samples
        .iter()
        .map(|s| {
            ground(s.p)
                .filter(|z| z.is_finite())
                .ok_or(BandFail::NoGround)
        })
        .collect()
}

/// An interval; one that is empty by no more than the tolerance becomes a point.
fn settle(lo: f64, hi: f64) -> Option<Reach> {
    if lo > hi + SLACK {
        None
    } else if lo > hi {
        let m = f64::midpoint(lo, hi);
        Some(Reach { lo: m, hi: m })
    } else {
        Some(Reach { lo, hi })
    }
}

fn clamp_in(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

/// The forward pass: the reach at each sample, from the reach at the first.
fn forward(z: &[f64], samples: &[At], band: &Band, start: Reach) -> Result<Vec<Reach>, BandFail> {
    let mut out = Vec::with_capacity(z.len());
    let mut prev: Option<Reach> = None;
    for (&z, s) in z.iter().zip(samples) {
        let (lo, hi) = match prev {
            None => (start.lo, start.hi),
            Some(r) => {
                let d = band.grade * s.ds + SLACK;
                (r.lo - d, r.hi + d)
            }
        };
        let r = settle(lo.max(z - band.cut_m), hi.min(z + band.fill_m)).ok_or(BandFail::Grade)?;
        out.push(r);
        prev = Some(r);
    }
    Ok(out)
}

/// The band test of one step of a route: every segment must be free of obstacles, the road height reachable at
/// the end of the polyline is carried on from `start` (the reach at its first point), and the climb and the
/// earthwork of the nominal height (the terrain clamped into the reach) are reported.
///
/// The earthwork is a lower bound: [`profile`] never needs less over the same route.
///
/// # Errors
///
/// [`BandFail::Blocked`] for an obstacle, [`BandFail::NoGround`] where there is no terrain height,
/// [`BandFail::Grade`] where no road height is reachable.
pub fn band_step(
    line: &[(f64, f64)],
    ground: &dyn Fn((f64, f64)) -> Option<f64>,
    band: &Band,
    start: Reach,
    obstacles: &Obstacles,
    clear: Clearance,
) -> Result<BandStep, BandFail> {
    let samples = sample_line(line, band.spacing_m)?;
    for (a, b) in line.iter().zip(line.iter().skip(1)) {
        obstacles
            .segment(*a, *b, clear)
            .map_err(BandFail::Blocked)?;
    }
    let z = heights_at(&samples, ground)?;
    let reaches = forward(&z, &samples, band, start)?;
    let (mut climb, mut earth) = (0.0, 0.0);
    let mut prev: Option<f64> = None;
    for ((&z, r), s) in z.iter().zip(&reaches).zip(&samples) {
        let h = clamp_in(z, r.lo, r.hi);
        if let Some(p) = prev {
            climb += (h - p).abs();
            earth += (h - z).abs() * s.ds;
        }
        prev = Some(h);
    }
    Ok(BandStep {
        reach: reaches.last().copied().unwrap_or(start),
        climb_m: climb,
        earthwork_low_m2: earth,
    })
}

/// The reach arrives at an anchor at the terrain height `z_end` if it meets `z_end` within `tol`.
#[must_use]
pub fn arrives(reach: Reach, z_end: f64, tol: f64) -> bool {
    reach.lo <= z_end + tol + SLACK && reach.hi >= z_end - tol - SLACK
}

/// A road height profile along a route.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    /// The road height at every sample, from the start anchor to the end anchor.
    pub heights: Vec<f64>,
    /// The earthwork, `Σ|h − z| · ds`, in m².
    pub earthwork_m2: f64,
    /// The deepest cut, in metres.
    pub cut_max_m: f64,
    /// The highest fill, in metres.
    pub fill_max_m: f64,
    /// The steepest grade of the profile.
    pub max_grade: f64,
}

/// The profile closest to the terrain that keeps the grade, the cut and the fill, with both ends pinned to the
/// terrain within [`Band::anchor_tol_m`]: the forward pass, the backward pass, then the profile picked closest to
/// the terrain (notes 5.1, steps 1 to 4).
///
/// # Errors
///
/// [`BandFail::NoGround`] where there is no terrain height, [`BandFail::Grade`] when the forward pass empties,
/// [`BandFail::Arrival`] when the road cannot reach the end anchor.
pub fn profile(
    line: &[(f64, f64)],
    ground: &dyn Fn((f64, f64)) -> Option<f64>,
    band: &Band,
) -> Result<Profile, BandFail> {
    let samples = sample_line(line, band.spacing_m)?;
    let z = heights_at(&samples, ground)?;
    let (Some(&z_first), Some(&z_last)) = (z.first(), z.last()) else {
        return Ok(Profile {
            heights: Vec::new(),
            earthwork_m2: 0.0,
            cut_max_m: 0.0,
            fill_max_m: 0.0,
            max_grade: 0.0,
        });
    };
    let mut reach = forward(
        &z,
        &samples,
        band,
        Reach::pinned(z_first, band.anchor_tol_m),
    )?;
    if let Some(last) = reach.last_mut() {
        let end = Reach::pinned(z_last, band.anchor_tol_m);
        *last = settle(last.lo.max(end.lo), last.hi.min(end.hi)).ok_or(BandFail::Arrival)?;
    }
    let mut after: Option<(Reach, f64)> = None;
    for (here, s) in reach.iter_mut().zip(&samples).rev() {
        if let Some((next, ds)) = after {
            let d = band.grade * ds;
            *here = settle(here.lo.max(next.lo - d), here.hi.min(next.hi + d))
                .ok_or(BandFail::Arrival)?;
        }
        after = Some((*here, s.ds));
    }
    let mut heights: Vec<f64> = Vec::with_capacity(z.len());
    for ((&z, r), s) in z.iter().zip(&reach).zip(&samples) {
        let (lo, hi) = match heights.last() {
            None => (r.lo, r.hi),
            Some(&h) => {
                let d = band.grade * s.ds;
                (r.lo.max(h - d), r.hi.min(h + d))
            }
        };
        heights.push(clamp_in(z, lo, hi));
    }
    let (mut earth, mut cut, mut fill, mut steepest) = (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
    let mut prev: Option<f64> = None;
    for ((&h, &z), s) in heights.iter().zip(&z).zip(&samples) {
        cut = cut.max(z - h);
        fill = fill.max(h - z);
        if let Some(p) = prev {
            earth += (h - z).abs() * s.ds;
            if s.ds > 0.0 {
                steepest = steepest.max((h - p).abs() / s.ds);
            }
        }
        prev = Some(h);
    }
    Ok(Profile {
        heights,
        earthwork_m2: earth,
        cut_max_m: cut,
        fill_max_m: fill,
        max_grade: steepest,
    })
}

/// The grade of the drawn line between the anchors of a leg.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawnGrade {
    /// Terrain height at the end minus that at the start, in metres.
    pub delta_z: f64,
    /// `|delta_z|` over the length of the line.
    pub mean: f64,
    /// The steepest grade over any window of the requested length (reported, not used for the limit).
    pub window: f64,
}

/// The drawn grade of a line: the mean between its ends and the steepest window of `window_m` metres
/// (the whole line when it is shorter). `None` for a line without length or without terrain.
#[must_use]
pub fn drawn_grade(
    line: &[(f64, f64)],
    ground: &dyn Fn((f64, f64)) -> Option<f64>,
    window_m: f64,
) -> Option<DrawnGrade> {
    let samples = sample_line(line, DRAWN_SPACING_M).ok()?;
    let z = heights_at(&samples, ground).ok()?;
    let length: f64 = samples.iter().map(|s| s.ds).sum();
    if samples.len() < 2 || length <= 0.0 {
        return None;
    }
    let delta_z = z.last()? - z.first()?;
    let ds = length / to_f64(samples.len() - 1);
    let k = step_count((window_m / ds).round()).min(z.len() - 1);
    let span = to_f64(k) * ds;
    let window = z
        .iter()
        .zip(z.iter().skip(k))
        .map(|(a, b)| (b - a).abs() / span)
        .fold(0.0, f64::max);
    Some(DrawnGrade {
        delta_z,
        mean: delta_z.abs() / length,
        window,
    })
}

/// The grade limit of a leg: `min(ceiling, max(aim, drawn mean))`, with the smallest aim and ceiling of the
/// families of the zone. No families: 0.
#[must_use]
pub fn leg_limit(families: &[Family], drawn: &DrawnGrade) -> f64 {
    if families.is_empty() {
        return 0.0;
    }
    let aim = families
        .iter()
        .map(|&f| grade_aim(f))
        .fold(f64::INFINITY, f64::min);
    let ceiling = families
        .iter()
        .map(|&f| grade_ceiling(f))
        .fold(f64::INFINITY, f64::min);
    ceiling.min(aim.max(drawn.mean))
}

/// The limits tried in turn: the limit, then thirds (`steps` of them) of the gap to the ceiling, ending with
/// the ceiling. Only the limit when it is at the ceiling already.
#[must_use]
pub fn relaxed_limits(limit: f64, ceiling: f64, steps: usize) -> Vec<f64> {
    if steps == 0 || limit.partial_cmp(&ceiling) != Some(std::cmp::Ordering::Less) {
        return vec![limit];
    }
    let gap = ceiling - limit;
    let n = to_f64(steps);
    let mut out: Vec<f64> = (0..steps).map(|k| limit + gap * to_f64(k) / n).collect();
    out.push(ceiling);
    out
}

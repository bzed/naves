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
///
/// A start can only be made with [`Reach::at_anchor`], whose tolerance is the band's own; every other reach comes
/// out of the propagation of [`band_step`]. A start that floats in the band (the spike's `[z - 6, z + 6]`) cannot be
/// written from outside this module, because it would let the road put the climb of a hillside into embankments at
/// the anchor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reach {
    lo: f64,
    hi: f64,
}

impl Reach {
    /// The road is at the terrain height `z` of an anchor within the band's [`Band::anchor_tol_m`]: the start of a
    /// route. [`band_step`] refuses a start that is not finite or is empty (a NaN or negative tolerance).
    #[must_use]
    pub fn at_anchor(z: f64, band: &Band) -> Reach {
        Reach {
            lo: z - band.anchor_tol_m,
            hi: z + band.anchor_tol_m,
        }
    }

    /// The same, from a bare tolerance, for the ends pinned inside this module.
    fn pinned(z: f64, tol: f64) -> Reach {
        Reach {
            lo: z - tol,
            hi: z + tol,
        }
    }

    /// Lowest reachable height.
    #[must_use]
    pub fn lo(&self) -> f64 {
        self.lo
    }

    /// Highest reachable height.
    #[must_use]
    pub fn hi(&self) -> f64 {
        self.hi
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
    /// A segment is blocked. [`Blocked::Outside`] (an end outside the raster, or not a number) is a step not taken
    /// for the search, a failed check for the final check of a route, and never a reason for keeping a zone.
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
    /// A parameter is not usable: a grade, cut, fill or anchor tolerance that is NaN or negative, or a start reach
    /// that is not finite or empty. The band refuses it, because `f64::max` and `f64::min` drop a NaN and would
    /// remove the bound.
    #[error("a band parameter is not usable")]
    Invalid,
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
pub(crate) struct At {
    /// The position.
    pub(crate) p: (f64, f64),
    /// The distance from the previous sample (0 for the first).
    pub(crate) ds: f64,
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

/// The samples of a polyline, without allocating: the start, then `n` points equally spaced along the line, at
/// most `spacing` apart (a spacing that is not a positive finite number counts as 2 m, `n` is at least 1 and at most
/// 10^6), the end point last. A line of no length, or of fewer than two points, has its first point only, and an
/// empty line has none. The one placement of samples that the band and the cost field both use. The points of the
/// line must be finite; [`band_step`] and the field's edges guarantee it.
pub(crate) fn line_samples(line: &[(f64, f64)], spacing: f64) -> LineSamples<'_> {
    let total: f64 = line
        .iter()
        .zip(line.iter().skip(1))
        .map(|(a, b)| (b.0 - a.0).hypot(b.1 - a.1))
        .sum();
    let (n, ds) = if total <= 0.0 || !total.is_finite() {
        (0, 0.0)
    } else {
        let spacing = if spacing.is_finite() && spacing > 0.0 {
            spacing
        } else {
            SPACING_DEFAULT
        };
        let n = step_count((total / spacing - SLACK).ceil());
        (n, total / to_f64(n))
    };
    LineSamples {
        line,
        n,
        ds,
        k: 0,
        seg: 0,
        before: 0.0,
    }
}

/// The iterator of [`line_samples`].
#[derive(Debug, Clone)]
pub(crate) struct LineSamples<'a> {
    line: &'a [(f64, f64)],
    n: usize,
    ds: f64,
    /// How many samples were given.
    k: usize,
    seg: usize,
    before: f64,
}

impl LineSamples<'_> {
    /// The length of segment `i` of the line.
    fn len(&self, i: usize) -> Option<f64> {
        let a = self.line.get(i)?;
        let b = self.line.get(i + 1)?;
        Some((b.0 - a.0).hypot(b.1 - a.1))
    }
}

impl Iterator for LineSamples<'_> {
    type Item = At;

    fn next(&mut self) -> Option<At> {
        let first = self.line.first().copied()?;
        let k = self.k;
        if k > self.n {
            return None;
        }
        self.k += 1;
        if k == 0 {
            return Some(At { p: first, ds: 0.0 });
        }
        let target = to_f64(k) * self.ds;
        while let Some(len) = self.len(self.seg)
            && self.before + len < target
            && self.seg + 1 < self.line.len().saturating_sub(1)
        {
            self.before += len;
            self.seg += 1;
        }
        let p = match (
            self.line.get(self.seg).copied(),
            self.line.get(self.seg + 1).copied(),
            self.len(self.seg),
        ) {
            (Some(a), Some(b), Some(len)) if k < self.n && len > 0.0 => {
                let t = ((target - self.before) / len).clamp(0.0, 1.0);
                (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
            }
            _ => self.line.last().copied().unwrap_or(first),
        };
        Some(At { p, ds: self.ds })
    }
}

/// The samples of a polyline: equally spaced, at most `spacing` apart, the end point last.
fn sample_line(line: &[(f64, f64)], spacing: f64) -> Result<Vec<At>, BandFail> {
    if line.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return Err(BandFail::NoGround);
    }
    Ok(line_samples(line, spacing).collect())
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

/// A grade that is not negative and not NaN (infinite allowed: the cost field runs the band with the grade limit
/// removed), and a cut, fill and anchor tolerance that are finite and not negative. NaN would be dropped silently by
/// `f64::max` and `f64::min`; an infinite tolerance would float the anchors, an infinite cut or fill would remove
/// the band, and the field removes only the grade.
fn check_band(band: &Band) -> Result<(), BandFail> {
    let finite = |v: f64| v.is_finite() && v >= 0.0;
    if band.grade >= 0.0 && finite(band.cut_m) && finite(band.fill_m) && finite(band.anchor_tol_m) {
        Ok(())
    } else {
        Err(BandFail::Invalid)
    }
}

/// A start reach that is finite and not empty.
fn check_start(start: Reach) -> Result<(), BandFail> {
    if start.lo.is_finite() && start.hi.is_finite() && start.lo <= start.hi + SLACK {
        Ok(())
    } else {
        Err(BandFail::Invalid)
    }
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
/// This is for pruning inside the search. It never accepts a route: only [`arrive`] does. The forward band's
/// earthwork is 0 at the ceiling for the switchback whose real profile needs 67.6 m², and `band_step` with
/// [`arrives`] may accept, by a nanometre per sample, a route that [`arrive`] refuses (this function widens the
/// reach by `10^-9` per sample); [`arrive`] decides.
///
/// The earthwork is a lower bound of the earthwork of [`arrive`], pointwise, only when both sample the same line
/// at the same points. A search that adds the lower bounds of its steps (each step is sampled by itself, [`arrive`]
/// samples the whole route) must charge `max(Σ low, real)` for an arrived state, not the sum alone.
///
/// # Errors
///
/// [`BandFail::Blocked`] for an obstacle, [`BandFail::NoGround`] where there is no terrain height,
/// [`BandFail::Grade`] where no road height is reachable, [`BandFail::Invalid`] for a parameter that is not
/// usable (see [`BandFail::Invalid`]; a grade of `f64::INFINITY` is allowed, the cost field runs the band with
/// the grade limit removed).
pub fn band_step(
    line: &[(f64, f64)],
    ground: &dyn Fn((f64, f64)) -> Option<f64>,
    band: &Band,
    start: Reach,
    obstacles: &Obstacles,
    clear: Clearance,
) -> Result<BandStep, BandFail> {
    check_band(band)?;
    check_start(start)?;
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

/// The reach arrives at an anchor at the terrain height `z_end` if it meets `z_end` within `tol`. For pruning
/// inside the search only (see [`band_step`]); a NaN anywhere gives `false`.
#[must_use]
pub fn arrives(reach: Reach, z_end: f64, tol: f64) -> bool {
    reach.lo <= z_end + tol + SLACK && reach.hi >= z_end - tol - SLACK
}

/// A road height profile along a route. Only [`arrive`] makes one, so a `Profile` is the proof that a route was
/// accepted.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    heights: Vec<f64>,
    ground: Vec<f64>,
    earthwork_m2: f64,
    cut_max_m: f64,
    fill_max_m: f64,
    max_grade: f64,
}

impl Profile {
    /// The road height at every sample, from the start anchor to the end anchor.
    #[must_use]
    pub fn heights(&self) -> &[f64] {
        &self.heights
    }

    /// The terrain height at every sample.
    #[must_use]
    pub fn ground(&self) -> &[f64] {
        &self.ground
    }

    /// The earthwork, `Σ|h − z| · ds`, in m²: per sample, times the spacing before it (the first sample, at the
    /// start anchor, adds nothing).
    #[must_use]
    pub fn earthwork_m2(&self) -> f64 {
        self.earthwork_m2
    }

    /// The deepest cut, in metres.
    #[must_use]
    pub fn cut_max_m(&self) -> f64 {
        self.cut_max_m
    }

    /// The highest fill, in metres.
    #[must_use]
    pub fn fill_max_m(&self) -> f64 {
        self.fill_max_m
    }

    /// The steepest grade of the profile.
    #[must_use]
    pub fn max_grade(&self) -> f64 {
        self.max_grade
    }
}

/// The only way to turn a candidate route into an accepted route. Both ends are pinned to the terrain of the
/// route's own first and last point (within [`Band::anchor_tol_m`]); the result is the profile closest to the
/// terrain that keeps the grade, the cut and the fill: the forward pass, the backward pass, then the pick (notes
/// 5.1, steps 1 to 4). Its earthwork, cut and fill are the real ones. [`band_step`] and [`arrives`] are for
/// pruning inside the search and never for acceptance.
///
/// # Errors
///
/// [`BandFail::NoGround`] where there is no terrain height, [`BandFail::Grade`] when the forward pass empties,
/// [`BandFail::Arrival`] when the road cannot reach the end anchor, [`BandFail::Invalid`] for a grade, cut, fill or
/// tolerance that is NaN or negative (a grade of `f64::INFINITY` is allowed).
pub fn arrive(
    line: &[(f64, f64)],
    ground: &dyn Fn((f64, f64)) -> Option<f64>,
    band: &Band,
) -> Result<Profile, BandFail> {
    check_band(band)?;
    let samples = sample_line(line, band.spacing_m)?;
    let z = heights_at(&samples, ground)?;
    let (Some(&z_first), Some(&z_last)) = (z.first(), z.last()) else {
        return Ok(Profile {
            heights: Vec::new(),
            ground: Vec::new(),
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
        ground: z,
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

/// The grade limit of a leg and the ceiling it may be relaxed to: `min(ceiling, max(aim, drawn mean))` and
/// `ceiling`, with the smallest aim and ceiling of the families of the zone. `None` without families: a zone
/// always has one, so that is a bug of the caller and must not become a grade of 0.
#[must_use]
pub fn leg_limit(families: &[Family], drawn: &DrawnGrade) -> Option<(f64, f64)> {
    if families.is_empty() {
        return None;
    }
    let aim = families
        .iter()
        .map(|&f| grade_aim(f))
        .fold(f64::INFINITY, f64::min);
    let ceiling = families
        .iter()
        .map(|&f| grade_ceiling(f))
        .fold(f64::INFINITY, f64::min);
    Some((ceiling.min(aim.max(drawn.mean)), ceiling))
}

/// The limits tried in turn: the limit, then thirds (`steps` of them) of the gap to the ceiling, ending with
/// the ceiling. Only the limit when it is at the ceiling already. `None` when either is NaN: that is a bug of the
/// caller (a leg with a limit from [`leg_limit`] always has numbers), not a leg without a route.
#[must_use]
pub fn relaxed_limits(limit: f64, ceiling: f64, steps: usize) -> Option<Vec<f64>> {
    if limit.is_nan() || ceiling.is_nan() {
        return None;
    }
    if steps == 0 || limit >= ceiling {
        return Some(vec![limit]);
    }
    let gap = ceiling - limit;
    let n = to_f64(steps);
    let mut out: Vec<f64> = (0..steps).map(|k| limit + gap * to_f64(k) / n).collect();
    out.push(ceiling);
    Some(out)
}

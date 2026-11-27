// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The arc search of one leg: a hybrid A* over poses from the start anchor to the end anchor (spec: "The search for
//! one leg", clean-room notes 3.5).
//!
//! A state is a pose with the cost so far and the interval of road heights reachable there. Five arcs of one step
//! leave every state; from every state within [`LegInput::arrive_reach_m`] of the end anchor the Dubins paths of
//! the planning radius to the end pose are tried, and the first that passes the cheap tests becomes an *arrival
//! candidate* in the open set with its lower-bound cost. An arrival is never merged with another (each is its own
//! candidate). When a candidate is popped the whole route is built and checked from scratch: [`arrive`], the
//! two-pass profile of the whole route, whose real earthwork replaces the lower bound in the cost, the radius, every
//! segment, the stray cap, the crossings of every wall (each wall's count must equal what the original line made)
//! and the self-clearance. A route that fails is dropped and the next Dubins path of the same state is tried, so a
//! failed check never keeps a later arrival out. A route whose real cost rose above the next entry of the open set is
//! put back with its real cost.
//!
//! The search is deterministic: its open set is ordered by priority, cost, state key and insertion order with
//! `total_cmp`, and the table of state keys is only looked up, never iterated. A state is dropped when a stored state of
//! its cell and heading bin costs no more and reaches as far towards the end anchor's height; the comparison is of
//! the end of the reachable interval that matters, within [`DOMINANCE_TOL_M`] (below the end's heights the top, above
//! them the bottom, where they meet both ends). Otherwise both are kept, up to [`PER_KEY`] of them.
//!
//! The estimate of the cost to go is the larger of the cost-to-go [`Field`] and an analytic bound from the distance
//! and from the gap between the state's reachable heights and the end anchor's. The field is not strictly
//! admissible (see its module), so the search at weight 1 is not exact either, and an empty open set is not a proof
//! that there is no route. Budgets are counts of expanded states, never time.
//!
//! The tests inside the search sample an arc by length only (chords of 2 m, which are 5 cm off the arc at a radius of
//! 10 m); the route that is returned has a point about every 2° of an arc, and is checked on those points.
//!
//! All input is untrusted: nothing here panics, and every allocation is bounded by a budget.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::f64::consts::TAU;
use std::hash::{BuildHasherDefault, Hasher};

use crate::smooth::{radius_at, resample, simplify_line};

use super::band::{Band, Reach, arrive, arrives, band_step, check_band, line_samples};
use super::dubins::{DubinsPath, arc_points, dubins_paths, follow, segments};
use super::field::Field;
use super::raster::{Clearance, Obstacles, Wall};
use super::{
    Anchor, PLAN_MAX_TURN_DEG, PLAN_RADIUS_MARGIN, PLAN_REACHED_FRACTION, Pose, RerouteError,
    RerouteParams, SAMPLE_M,
};

/// The most states one try may expand, whatever its budget says.
const MAX_STATES: usize = 200_000;
/// The largest radius, step and arrival reach a leg may have, in metres (a bigger one is a mistake of the caller and
/// would give polylines of millions of points).
const MAX_RADIUS_M: f64 = 1_000.0;
const MAX_STEP_M: f64 = 1_000.0;
const MAX_REACH_M: f64 = 10_000.0;
/// States kept per cell and heading bin: those that no cheaper state dominates (see [`DOMINANCE_TOL_M`]); the dearest
/// goes beyond this number.
pub const PER_KEY: usize = 4;
/// The estimate of the cost to go is never above this (a leg whose heights cannot be met has no finite bound).
const HEURISTIC_CAP: f64 = 1.0e12;
/// Fewest and most heading bins of the state key.
const BINS_RANGE: (f64, f64) = (32.0, 180.0);
/// Offset of the cell coordinates in a state key, and their limit.
const CELL_OFFSET: i64 = 1 << 26;
const CELL_LIMIT: i64 = (1 << 27) - 1;
/// Two anchors this close (metres) are one point.
const SAME_ANCHOR_M: f64 = 1e-9;
/// Slack of the grade of a profile, in metres of rise per metre.
const GRADE_SLACK: f64 = 1e-9;
/// Slack of the end of a route, in metres.
const END_SLACK_M: f64 = 1e-6;
/// How much wider than a cheaper state's reachable interval a dearer state's must be, at an end, to be kept, in metres.
pub const DOMINANCE_TOL_M: f64 = 2.0;
/// Slack of the comparison of two reachable intervals, and of two costs, in their own units.
const SLACK: f64 = 1e-9;
/// Samples this near (metres) to each other count as nearer than the self-clearance only if they are nearer by
/// more than this: the two ends of a hairpin whose legs are exactly the clearance apart are not a violation.
const SELF_SLACK_M: f64 = 1e-6;
/// The cap of the stray is tested by the true distance to the original line when it is below this many raster cells.
const TRUE_CAP_CELLS: f64 = 2.0;
/// The most samples of an original line that [`self_clearance`] compares.
const SELF_MAX_SAMPLES: f64 = 2_000.0;

/// The weights of the cost of a step, in metres of road.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    /// Metres of road per metre of climb.
    pub slope: f64,
    /// Weight of the heading change, times `|δ| · R`.
    pub curve: f64,
    /// Metres of road per m² of earthwork.
    pub earth: f64,
    /// Metres of road per m² between the route and the original line.
    pub stray: f64,
}

impl Weights {
    /// The weights of the parameters.
    #[must_use]
    pub fn from_params(params: &RerouteParams) -> Weights {
        Weights {
            slope: params.w_slope,
            curve: params.w_curve,
            earth: params.w_earth,
            stray: params.w_stray,
        }
    }
}

/// Everything a leg is searched with. Built once per leg by the caller; the raster, the walls and the field are
/// shared by every grade limit and every budget try.
pub struct LegInput<'a> {
    /// Where the leg starts.
    pub start: Anchor,
    /// Where the leg ends.
    pub end: Anchor,
    /// The planning radius: no arc is tighter, in metres (at most 1000).
    pub radius: f64,
    /// The length of a search step, in metres (at most 1000).
    pub step_m: f64,
    /// The band, with the grade limit of [`search_leg`]. [`search_leg_relaxed`] changes its grade only.
    pub band: &'a Band,
    /// The terrain height at a point, or `None` where there is none.
    pub ground: &'a dyn Fn((f64, f64)) -> Option<f64>,
    /// The obstacle raster of the leg, with its original line set and its maps finished.
    pub obstacles: &'a Obstacles,
    /// The lines the route may cross only as often as the original line did: the same values for blocking a step
    /// and for counting a route, each built once from the original line.
    pub walls: &'a [Wall],
    /// The clearances of the segment test (other roads and water).
    pub clear: Clearance,
    /// The cost-to-go field of the leg (built for the end anchor).
    pub field: &'a Field,
    /// The weights of the cost.
    pub weights: Weights,
    /// The furthest a route may be from the original line, in metres. In the raster's stored measure (which is 0
    /// inside the cells the original line touches), or by the true distance to [`LegInput::original`] when it is
    /// below two raster cells.
    pub stray_cap_m: f64,
    /// Dubins arrivals are tried from states this near the end anchor, in metres (at most 10 000).
    pub arrive_reach_m: f64,
    /// Two samples of the route more than this far apart along it must keep [`LegInput::self_clear_m`] from each
    /// other, in metres.
    pub self_along_m: f64,
    /// The distance that such samples must keep, in metres: given by the caller, who sets it to the smallest
    /// distance that the original line itself kept between two such samples and between its two ends, but no more
    /// than the road clearance (the true distance, about 13 m, not the raster's threshold) and no less than the
    /// road's width (see [`self_clearance`]). A hairpin whose legs are 8 m apart clears itself by
    /// 8 m, not by the road clearance. 0 turns the rule off.
    pub self_clear_m: f64,
    /// The original line of the leg, for the true distance of a route from it ([`Route::max_move_m`], and the stray
    /// cap when it is below two raster cells).
    pub original: &'a [(f64, f64)],
    /// The heuristic weight and the number of states to expand of each try; the next try runs only when the
    /// budget ran out.
    pub budgets: &'a [(f64, usize)],
}

/// A route found for a leg.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// The points from the start anchor to the end anchor: arcs about every 2°, straights at most 2 m apart.
    pub points: Vec<(f64, f64)>,
    /// The length, in metres.
    pub length_m: f64,
    /// The rise and fall of the profile of [`arrive`], in metres.
    pub climb_m: f64,
    /// The real earthwork of the profile, in m².
    pub earthwork_m2: f64,
    /// The deepest cut of the profile, in metres.
    pub cut_max_m: f64,
    /// The highest fill of the profile, in metres.
    pub fill_max_m: f64,
    /// The largest distance of a route point from the original line in the raster's stored measure (0 inside the
    /// cells the line touches), in metres: the measure of the stray cost and of the cap.
    pub max_stray_m: f64,
    /// The largest true distance of a route point from the original line ([`LegInput::original`]), in metres.
    pub max_move_m: f64,
    /// The net rise of the terrain between the two anchors over the length of the route: `|Δz| / length`, never the
    /// summed climb. The road is pinned to the terrain at the anchors only within the band's tolerance, so the grade
    /// of the road can be up to `2 · tolerance / length` less or more than this.
    pub planned_grade: f64,
    /// The grade limit the route was found at.
    pub grade_limit: f64,
    /// The index in the limits of [`search_leg_relaxed`] of the limit that worked (0 for [`search_leg`]).
    pub relax_steps: usize,
    /// The cost of the route.
    pub cost: f64,
    /// The heuristic weight of the try that found it.
    pub weight: f64,
    /// States expanded by the whole call, with every try and every limit.
    pub states: usize,
}

/// What a search of a leg found.
#[derive(Debug, Clone, PartialEq)]
pub enum LegOutcome {
    /// A route.
    Found(Route),
    /// No route found. `budget_exhausted` is true when the last try ran out of states. It is false when its open set
    /// ran empty: that is **not** a proof that there is no route, because the estimate is not strictly admissible
    /// and a cell and heading keep a bounded number of states. `closest_m` is the nearest an expanded state came
    /// to the end anchor.
    NoRoute {
        /// The last budget ran out before the open set did.
        budget_exhausted: bool,
        /// States expanded by the whole call.
        states: usize,
        /// The smallest distance of an expanded state to the end anchor, in metres.
        closest_m: f64,
    },
}

/// How a state came about.
#[derive(Debug, Clone, Copy)]
enum Via {
    /// A first state at an anchor.
    Start,
    /// An arc (a straight for a heading change of 0) of the given heading change and length.
    Arc { delta: f64, length: f64 },
    /// A Dubins path to the end pose (the `rank` of the path among the paths between the two poses, shortest first),
    /// then the end anchor's straight (if it has one).
    Dubins { path: DubinsPath, rank: u8 },
}

/// A state of the search, in an arena.
#[derive(Debug, Clone, Copy)]
struct Node {
    at: (f64, f64),
    heading: f64,
    /// The cost so far (for an arrival candidate the lower bound; for a verified arrival the real cost).
    g: f64,
    /// The lower bound of the earthwork so far, in m² (inside `g`, weighted).
    earth_low: f64,
    reach: Reach,
    parent: u32,
    via: Via,
    /// An arrival candidate: its route is not built or checked until it is popped.
    arrived: bool,
    /// An arrival whose whole route was checked and whose `g` is the real cost.
    verified: bool,
    key: u64,
    /// The route length at this state, in metres.
    along: f64,
    /// The crossings of each wall that allows some, so far: the first in the counts arena.
    counts: usize,
    /// The samples of the step that made it: the first and how many, in the sample arena.
    samples: (usize, usize),
}

const NONE: u32 = u32::MAX;

/// An entry of the open set.
#[derive(Debug, Clone, Copy)]
struct Item {
    prio: f64,
    g: f64,
    key: u64,
    seq: u64,
    idx: u32,
}

impl PartialEq for Item {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Item {}

impl PartialOrd for Item {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Item {
    /// The `BinaryHeap` pops the greatest: the lowest priority, then cost, then key, then insertion order.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .prio
            .total_cmp(&self.prio)
            .then_with(|| other.g.total_cmp(&self.g))
            .then_with(|| other.key.cmp(&self.key))
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

/// A hasher for the table of state keys: a multiplicative mix of the 64-bit key. The table is only looked up, never
/// iterated, so the order of its entries cannot reach a result.
#[derive(Default)]
struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.write_u64(u64::from(*b));
        }
    }

    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

/// The states of a cell and heading bin that are alive: at most [`PER_KEY`].
#[derive(Debug, Clone, Copy, Default)]
struct Slot {
    ids: [u32; PER_KEY],
    n: usize,
}

impl Slot {
    fn ids(&self) -> &[u32] {
        self.ids.get(..self.n).unwrap_or(&[])
    }
}

/// The bounding box of a wall, to skip the walls a step is far from.
#[derive(Debug, Clone, Copy)]
struct WallBox {
    lo: (f64, f64),
    hi: (f64, f64),
}

/// The lowest and the highest coordinate of a polyline; a point that is not a number makes the box overlap nothing.
fn bounds(line: &[(f64, f64)]) -> ((f64, f64), (f64, f64)) {
    let fold = |f: fn(f64, f64) -> f64, init: f64, pick: fn(&(f64, f64)) -> f64| {
        line.iter().map(pick).fold(init, f)
    };
    (
        (
            fold(f64::min, f64::INFINITY, |p| p.0),
            fold(f64::min, f64::INFINITY, |p| p.1),
        ),
        (
            fold(f64::max, f64::NEG_INFINITY, |p| p.0),
            fold(f64::max, f64::NEG_INFINITY, |p| p.1),
        ),
    )
}

impl WallBox {
    fn of(line: &[(f64, f64)]) -> WallBox {
        let (lo, hi) = bounds(line);
        WallBox { lo, hi }
    }

    /// Whether a polyline's box overlaps this one. A wall with a coordinate that is not a number overlaps
    /// nothing: `crossings` finds no crossing with it either.
    fn near(&self, lo: (f64, f64), hi: (f64, f64)) -> bool {
        lo.0 <= self.hi.0 && hi.0 >= self.lo.0 && lo.1 <= self.hi.1 && hi.1 >= self.lo.1
    }
}

/// The distance between two points.
fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// The distance of a point from a polyline (infinite for no segment).
fn poly_dist(p: (f64, f64), line: &[(f64, f64)]) -> f64 {
    let one = |a: (f64, f64), b: (f64, f64)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let l2 = dx * dx + dy * dy;
        let t = if l2 > 0.0 {
            (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
    };
    match line {
        [] => f64::INFINITY,
        [only] => dist(p, *only),
        _ => line
            .windows(2)
            .map(|w| match w {
                [a, b] => one(*a, *b),
                _ => f64::INFINITY,
            })
            .fold(f64::INFINITY, f64::min),
    }
}

fn unit(heading: f64) -> (f64, f64) {
    let (s, c) = heading.sin_cos();
    (c, s)
}

fn norm(heading: f64) -> f64 {
    let h = heading.rem_euclid(TAU);
    if h >= TAU { 0.0 } else { h }
}

fn finite2(p: (f64, f64)) -> bool {
    p.0.is_finite() && p.1.is_finite()
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to a few hundred bins first"
)]
fn bin_count(v: f64) -> usize {
    v.clamp(BINS_RANGE.0, BINS_RANGE.1) as usize
}

#[expect(
    clippy::cast_precision_loss,
    reason = "a count of at most a few hundred is exact"
)]
fn to_f64(n: usize) -> f64 {
    n as f64
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "floored and clamped to the cell limit first"
)]
fn cell_of(v: f64) -> u64 {
    let c = (v.floor().clamp(-1.0e8, 1.0e8)) as i64;
    u64::try_from(c.saturating_add(CELL_OFFSET).clamp(0, CELL_LIMIT)).unwrap_or(0)
}

/// A wall that the route may cross a number of times, with its box.
struct Counted {
    wall: usize,
    wbox: WallBox,
    allowed: usize,
}

/// The inputs of a leg after they were checked, with what the search derives from them once.
struct Leg<'a> {
    input: &'a LegInput<'a>,
    band: Band,
    /// The terrain height at the end anchor.
    z_end: f64,
    /// The pose the Dubins paths arrive at: the end heading, the end anchor's straight before the anchor.
    arrive_at: Option<Pose>,
    /// Heading changes of the steps.
    deltas: [f64; 5],
    bins: usize,
    /// The origin and the size of the cells of the state key.
    origin: (f64, f64),
    cell: f64,
    /// The walls that allow no crossing, with their boxes.
    blockers: Vec<(usize, WallBox)>,
    /// The walls that allow some.
    counted: Vec<Counted>,
    /// The tightest three-point radius a route may have.
    min_radius: f64,
    /// The cap is tested by the true distance to the original line.
    true_cap: bool,
    /// The least ratio of a chord of `SAMPLE_M` to its arc at the planning radius.
    chord_ratio: f64,
}

/// The inputs must be numbers.
fn validate(input: &LegInput<'_>) -> Result<(), RerouteError> {
    let anchor_ok = |a: &Anchor| {
        finite2(a.at) && a.heading.is_none_or(f64::is_finite) && a.straight_m.is_finite()
    };
    let band = input.band;
    let w = input.weights;
    let numbers = [
        input.radius,
        input.step_m,
        input.stray_cap_m,
        input.arrive_reach_m,
        input.self_along_m,
        input.self_clear_m,
        w.slope,
        w.curve,
        w.earth,
        w.stray,
        band.cut_m,
        band.fill_m,
        band.spacing_m,
        band.anchor_tol_m,
    ];
    if !anchor_ok(&input.start)
        || !anchor_ok(&input.end)
        || numbers.iter().any(|v| !v.is_finite())
        || band.grade.is_nan()
        || input.budgets.iter().any(|b| !b.0.is_finite())
    {
        return Err(RerouteError::NotFinite);
    }
    let negative = numbers.iter().take(10).any(|v| *v < 0.0)
        || input.start.straight_m < 0.0
        || input.end.straight_m < 0.0
        || input.budgets.iter().any(|b| b.0 < 0.0);
    if negative
        || input.radius <= 0.0
        || input.step_m <= 0.0
        || input.radius > MAX_RADIUS_M
        || input.step_m > MAX_STEP_M
        || input.arrive_reach_m > MAX_REACH_M
        || band.spacing_m <= 0.0
        || input.budgets.is_empty()
        || check_band(band).is_err()
        || dist(input.start.at, input.end.at) <= SAME_ANCHOR_M
    {
        return Err(RerouteError::Degenerate);
    }
    Ok(())
}

impl<'a> Leg<'a> {
    /// The leg, or `None` where the terrain at the end anchor is missing (nothing to search for).
    fn new(input: &'a LegInput<'a>, band: Band) -> Option<Leg<'a>> {
        let z_end = (input.ground)(input.end.at).filter(|z| z.is_finite())?;
        let theta = (input.step_m / input.radius).min(PLAN_MAX_TURN_DEG.to_radians());
        let arrive_at = input.end.heading.map(|h| {
            let back = unit(h);
            Pose {
                at: (
                    input.end.at.0 - input.end.straight_m * back.0,
                    input.end.at.1 - input.end.straight_m * back.1,
                ),
                heading: h,
            }
        });
        let mut blockers = Vec::new();
        let mut counted = Vec::new();
        for (i, w) in input.walls.iter().enumerate() {
            if w.allowed == 0 {
                blockers.push((i, WallBox::of(&w.line)));
            } else {
                counted.push(Counted {
                    wall: i,
                    wbox: WallBox::of(&w.line),
                    allowed: w.allowed,
                });
            }
        }
        Some(Leg {
            input,
            band,
            z_end,
            arrive_at,
            deltas: [0.0, theta / 2.0, -theta / 2.0, theta, -theta],
            bins: bin_count((TAU / (theta / 4.0)).ceil()),
            origin: input.start.at,
            cell: input.step_m / 2.0,
            blockers,
            counted,
            min_radius: PLAN_REACHED_FRACTION * input.radius / PLAN_RADIUS_MARGIN,
            true_cap: input.stray_cap_m < TRUE_CAP_CELLS * input.obstacles.resolution_m(),
            chord_ratio: {
                let half = (SAMPLE_M / (2.0 * input.radius)).min(1.5);
                half.sin() / half
            },
        })
    }

    /// The key of a state: its cell and its heading bin.
    fn key(&self, at: (f64, f64), heading: f64) -> u64 {
        let bin = (norm(heading) / TAU * to_f64(self.bins)).round();
        let bin = u64::try_from(bin_index(bin, self.bins)).unwrap_or(0);
        let cx = cell_of((at.0 - self.origin.0) / self.cell);
        let cy = cell_of((at.1 - self.origin.1) / self.cell);
        (bin << 54) | (cx << 27) | cy
    }

    /// Whether a point is within the stray cap: by the stored distance of the raster (`stored`), or, when the cap is
    /// below two raster cells, by the true distance to the original line.
    fn within_cap(&self, p: (f64, f64), stored: f64) -> bool {
        if self.true_cap {
            poly_dist(p, self.input.original) <= self.input.stray_cap_m
        } else {
            stored <= self.input.stray_cap_m
        }
    }

    /// The gap between the reachable heights and the end anchor's heights (within the tolerance), in metres: the
    /// road at the state is somewhere in `reach`, the road at the end within `tolerance` of the terrain there.
    fn height_gap(&self, reach: Reach) -> f64 {
        let tol = self.band.anchor_tol_m;
        (reach.lo() - (self.z_end + tol))
            .max((self.z_end - tol) - reach.hi())
            .max(0.0)
    }

    /// The estimate of the cost to go from a state: the larger of the field and the analytic bound. The bound is the
    /// larger of the distance and `gap / grade` in length, plus `W_SLOPE · gap` in climb, `gap` being the distance
    /// between the state's reachable heights and the end anchor's. It is a lower bound: the road at the state is
    /// within `reach`, the road at the end is within the tolerance of the terrain there, the grade limit bounds the
    /// length, and the nominal height of the band (the terrain clamped into the reach) at the end is within the
    /// tolerance of the terrain too, and at the state inside the reach, so its climb is at least the gap.
    fn heuristic(&self, at: (f64, f64), reach: Reach) -> f64 {
        let d = dist(at, self.input.end.at);
        let gap = self.height_gap(reach);
        let climb = if gap <= 0.0 {
            0.0
        } else if self.band.grade > 0.0 {
            gap / self.band.grade
        } else {
            f64::INFINITY
        };
        let analytic = d.max(climb) + self.input.weights.slope * gap;
        let h = analytic.max(self.input.field.estimate(at));
        if h.is_nan() {
            0.0
        } else {
            h.min(HEURISTIC_CAP)
        }
    }

    /// Whether a state with the reachable heights `a` is at least as good for the climb still to make as one with `b`:
    /// towards the end anchor's height it reaches as far (within [`DOMINANCE_TOL_M`]). Where `b` is below the end
    /// anchor's heights only the top of the interval matters, where above only the bottom, and where `b` already
    /// meets them the whole interval does.
    fn covers(&self, a: Reach, b: Reach) -> bool {
        let tol = self.band.anchor_tol_m;
        let up = a.hi() >= b.hi() - DOMINANCE_TOL_M;
        let down = a.lo() <= b.lo() + DOMINANCE_TOL_M;
        if b.hi() < self.z_end - tol {
            up
        } else if b.lo() > self.z_end + tol {
            down
        } else {
            up && down
        }
    }

    /// Whether a path of `length` metres from a state with these reachable heights can end at the end anchor's
    /// height: the grade limit moves the interval by at most `grade · length`.
    fn height_ok(&self, reach: Reach, length: f64) -> bool {
        let grade = self.band.grade;
        if !grade.is_finite() {
            return true;
        }
        let tol = self.band.anchor_tol_m;
        let rise = grade * length + SLACK;
        reach.lo() - rise <= self.z_end + tol && reach.hi() + rise >= self.z_end - tol
    }
}

/// `bin` (a rounded float) as an index below `bins`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "rounded and clamped to the bins first"
)]
fn bin_index(bin: f64, bins: usize) -> usize {
    (bin.max(0.0) as usize) % bins.max(1)
}

/// What a step or an arrival costs and leaves, once it passed every test.
struct Eval {
    reach: Reach,
    cost: f64,
    earth_low: f64,
    length: f64,
    /// The crossings of each wall that allows some, from the start of the route.
    counts: Vec<u32>,
    samples: Vec<(f64, f64, f64)>,
}

/// How one try ended.
enum Tried {
    Found(Box<Route>),
    /// The open set ran empty.
    Emptied,
    Exhausted,
}

/// How a popped arrival candidate ended.
enum Settled {
    Found(Box<Route>),
    Dropped,
}

struct Search<'a, 'b> {
    leg: &'b Leg<'a>,
    weight: f64,
    nodes: Vec<Node>,
    samples: Vec<(f64, f64, f64)>,
    counts: Vec<u32>,
    best: HashMap<u64, Slot, BuildHasherDefault<KeyHasher>>,
    heap: BinaryHeap<Item>,
    seq: u64,
    expanded: usize,
    closest: f64,
    /// The lowest real cost of an arrival whose whole route was checked and that waits in the open set.
    verified_g: f64,
    /// The routes of verified arrivals that wait in the open set, by the index of their node.
    pending: Vec<(u32, Route)>,
}

/// The samples of a step: every `SAMPLE_M` along it and its end, with their distance along the route, not the
/// start.
fn step_samples(along: f64, poly: &[(f64, f64)]) -> Vec<(f64, f64, f64)> {
    let mut s = along;
    line_samples(poly, SAMPLE_M)
        .skip(1)
        .map(|at| {
            s += at.ds;
            (at.p.0, at.p.1, s)
        })
        .collect()
}

/// The polyline of an arc (straight for a heading change of 0) that starts at `from`: chords of at most `SAMPLE_M`,
/// and when `fine`, a point about every 2° as well.
fn arc_polyline(from: Pose, delta: f64, length: f64, fine: bool) -> Vec<(f64, f64)> {
    let curvature = if length > 0.0 { delta / length } else { 0.0 };
    let angle = if fine { delta } else { 0.0 };
    arc_points(from, curvature, length, segments(angle, length, SAMPLE_M))
}

/// The polyline of a Dubins arrival from `from`: the path, then the end anchor's straight, or the path's end
/// moved onto the anchor.
fn dubins_polyline(
    path: &DubinsPath,
    from: Pose,
    radius: f64,
    end: &Anchor,
    fine: bool,
) -> Vec<(f64, f64)> {
    let mut poly = if fine {
        path.points(from, radius, SAMPLE_M)
    } else {
        path.points_coarse(from, radius, SAMPLE_M)
    };
    if end.straight_m > 0.0 {
        poly.push(end.at);
    } else if let Some(last) = poly.last_mut() {
        *last = end.at;
    }
    poly.dedup_by(|a, b| dist(*a, *b) < 1e-12);
    poly
}

impl<'a, 'b> Search<'a, 'b> {
    fn new(leg: &'b Leg<'a>, weight: f64) -> Search<'a, 'b> {
        Search {
            leg,
            weight,
            nodes: Vec::new(),
            samples: Vec::new(),
            counts: Vec::new(),
            best: HashMap::default(),
            heap: BinaryHeap::new(),
            seq: 0,
            expanded: 0,
            closest: dist(leg.input.start.at, leg.input.end.at),
            verified_g: f64::INFINITY,
            pending: Vec::new(),
        }
    }

    fn node(&self, i: u32) -> Option<&Node> {
        self.nodes.get(usize::try_from(i).ok()?)
    }

    fn pose_of(n: &Node) -> Pose {
        Pose {
            at: n.at,
            heading: n.heading,
        }
    }

    fn put(&mut self, node: Node, prio: f64) -> Option<u32> {
        let idx = u32::try_from(self.nodes.len()).ok()?;
        self.heap.push(Item {
            prio,
            g: node.g,
            key: node.key,
            seq: self.seq,
            idx,
        });
        self.seq = self.seq.saturating_add(1);
        self.nodes.push(node);
        Some(idx)
    }

    /// Adds a state to the arena and to the open set unless the stored states of its cell and heading dominate it:
    /// one with no higher cost whose reachable heights contain this one's. An arrival candidate is never merged.
    /// A new state removes the stored ones it dominates; at most [`PER_KEY`] stay, the dearest goes.
    fn push(&mut self, node: Node) -> Option<u32> {
        if node.arrived {
            let prio = node.g;
            return self.put(node, prio);
        }
        let slot = self.best.get(&node.key).copied().unwrap_or_default();
        let stored = |i: u32| self.node(i).map(|s| (s.g, s.reach));
        if slot
            .ids()
            .iter()
            .filter_map(|&i| stored(i))
            .any(|(g, reach)| g <= node.g && self.leg.covers(reach, node.reach))
        {
            return None;
        }
        let mut kept: Vec<(u32, f64)> = slot
            .ids()
            .iter()
            .filter_map(|&i| stored(i).map(|(g, reach)| (i, g, reach)))
            .filter(|&(_, g, reach)| !(node.g <= g && self.leg.covers(node.reach, reach)))
            .map(|(i, g, _)| (i, g))
            .collect();
        if kept.len() >= PER_KEY {
            // the dearest of the stored goes, if this one is cheaper; else this one is dropped
            let worst = kept
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.1.total_cmp(&b.1.1).then(a.1.0.cmp(&b.1.0)))
                .map(|(k, &(_, g))| (k, g));
            match worst {
                Some((k, g)) if node.g < g => {
                    kept.remove(k);
                }
                _ => return None,
            }
        }
        let h = self.leg.heuristic(node.at, node.reach);
        let prio = node.g + self.weight * h;
        let key = node.key;
        let idx = self.put(node, prio)?;
        let mut next = Slot::default();
        for (i, _) in kept.iter().chain(std::iter::once(&(idx, 0.0))) {
            if let Some(cell) = next.ids.get_mut(next.n) {
                *cell = *i;
                next.n += 1;
            }
        }
        self.best.insert(key, next);
        Some(idx)
    }

    /// Whether a state is still stored for its cell and heading (a dominated one is stale).
    fn alive(&self, node: &Node, idx: u32) -> bool {
        self.best
            .get(&node.key)
            .is_some_and(|slot| slot.ids().contains(&idx))
    }

    /// The root states: one at the start anchor, or one per heading at a dead end.
    fn start(&mut self) -> bool {
        let leg = self.leg;
        let input = leg.input;
        let Some(z) = (input.ground)(input.start.at).filter(|z| z.is_finite()) else {
            return false;
        };
        let reach = Reach::at_anchor(z, &leg.band);
        self.samples.push((input.start.at.0, input.start.at.1, 0.0));
        self.counts
            .extend(std::iter::repeat_n(0, leg.counted.len()));
        let root = |heading: f64| Node {
            at: input.start.at,
            heading: norm(heading),
            g: 0.0,
            earth_low: 0.0,
            reach,
            parent: NONE,
            via: Via::Start,
            arrived: false,
            verified: false,
            key: leg.key(input.start.at, heading),
            along: 0.0,
            counts: 0,
            samples: (0, 1),
        };
        let Some(h) = input.start.heading else {
            // a dead end: every heading is a first state
            return (0..leg.bins).all(|j| {
                let heading = TAU * to_f64(j) / to_f64(leg.bins);
                self.push(root(heading)).is_some()
            });
        };
        let node = root(h);
        if input.start.straight_m <= 0.0 {
            return self.push(node).is_some();
        }
        // the arm of the junction first, as a fixed straight
        let Some(parent) = u32::try_from(self.nodes.len()).ok() else {
            return false;
        };
        self.nodes.push(node);
        let from = Pose {
            at: input.start.at,
            heading: norm(h),
        };
        let length = input.start.straight_m;
        let poly = arc_polyline(from, 0.0, length, false);
        let Some(ev) = self.evaluate(parent, &poly, 0.0) else {
            return false;
        };
        self.add_arc(parent, from, 0.0, length, ev)
    }

    /// The new child of `parent` that follows an arc of the given heading change and length.
    fn add_arc(&mut self, parent: u32, from: Pose, delta: f64, length: f64, ev: Eval) -> bool {
        let curvature = if length > 0.0 { delta / length } else { 0.0 };
        let end = follow(from, curvature, length);
        self.add(parent, end, Via::Arc { delta, length }, ev, false)
            .is_some()
    }

    /// A state after a step, or an arrival candidate.
    fn add(&mut self, parent: u32, end: Pose, via: Via, ev: Eval, arrived: bool) -> Option<u32> {
        let p = *self.node(parent)?;
        let (first, count) = (self.samples.len(), ev.samples.len());
        let counts = self.counts.len();
        let heading = norm(end.heading);
        let node = Node {
            at: end.at,
            heading,
            g: p.g + ev.cost,
            earth_low: p.earth_low + ev.earth_low,
            reach: ev.reach,
            parent,
            via,
            arrived,
            verified: false,
            key: if arrived {
                0
            } else {
                self.leg.key(end.at, heading)
            },
            along: p.along + ev.length,
            counts,
            samples: (first, count),
        };
        let idx = self.push(node);
        if idx.is_some() {
            self.samples.extend(ev.samples);
            self.counts.extend(ev.counts);
        }
        idx
    }

    /// The crossings of every wall by a polyline, added to those of the parent's route: `None` where a wall that
    /// allows no crossing is crossed or a wall that allows some is crossed more often than it allows.
    fn wall_counts(&self, parent: &Node, poly: &[(f64, f64)]) -> Option<Vec<u32>> {
        let leg = self.leg;
        let (lo, hi) = bounds(poly);
        let walls = leg.input.walls;
        for (i, wbox) in &leg.blockers {
            if wbox.near(lo, hi) && walls.get(*i).is_some_and(|w| w.crossings(poly) > 0) {
                return None;
            }
        }
        let before = self
            .counts
            .get(parent.counts..parent.counts.saturating_add(leg.counted.len()))
            .unwrap_or(&[]);
        let mut out = Vec::with_capacity(leg.counted.len());
        for (k, c) in leg.counted.iter().enumerate() {
            let n = if c.wbox.near(lo, hi) {
                walls.get(c.wall).map_or(0, |w| w.crossings(poly))
            } else {
                0
            };
            let total = before
                .get(k)
                .copied()
                .unwrap_or(0)
                .saturating_add(u32::try_from(n).unwrap_or(u32::MAX));
            if usize::try_from(total).unwrap_or(usize::MAX) > c.allowed {
                return None;
            }
            out.push(total);
        }
        Some(out)
    }

    /// Every test of a step or an arrival, and its cost: the stray cap, the walls, the self-clearance, the band
    /// (which also tests every segment against the obstacles). `turn` is the heading change, in radians. The polyline
    /// is sampled by length only.
    fn evaluate(&self, parent: u32, poly: &[(f64, f64)], turn: f64) -> Option<Eval> {
        let leg = self.leg;
        let input = leg.input;
        let p = self.node(parent)?;
        let obstacles = input.obstacles;
        // the stray cap at every point and every middle, and the stray of the cost
        let (mut length, mut stray) = (0.0, 0.0);
        let mut prev: Option<((f64, f64), f64)> = None;
        for &q in poly {
            let here = obstacles.stray_m(q);
            if !finite2(q) || here.is_nan() || !leg.within_cap(q, here) {
                return None;
            }
            if let Some((a, before)) = prev {
                let mid = (f64::midpoint(a.0, q.0), f64::midpoint(a.1, q.1));
                if !leg.within_cap(mid, obstacles.stray_m(mid)) {
                    return None;
                }
                let l = dist(a, q);
                length += l;
                stray += l * f64::midpoint(before, here);
            }
            prev = Some((q, here));
        }
        let counts = self.wall_counts(p, poly)?;
        let samples = step_samples(p.along, poly);
        if !self.self_clear(parent, &samples) {
            return None;
        }
        let step = band_step(
            poly,
            input.ground,
            &leg.band,
            p.reach,
            obstacles,
            input.clear,
        )
        .ok()?;
        let wt = input.weights;
        let cost = length
            + wt.slope * step.climb_m
            + wt.curve * turn.abs() * input.radius
            + wt.earth * step.earthwork_low_m2
            + wt.stray * stray;
        Some(Eval {
            reach: step.reach,
            cost,
            earth_low: step.earthwork_low_m2,
            length,
            counts,
            samples,
        })
    }

    /// The squared distance below which two samples that far apart along the route are too near.
    fn self_limit2(&self) -> f64 {
        let c = (self.leg.input.self_clear_m - SELF_SLACK_M).max(0.0);
        c * c
    }

    /// Whether the new samples keep the self-clearance from the earlier samples of the route (and from each
    /// other) that are more than `self_along_m` back along it.
    fn self_clear(&self, parent: u32, new: &[(f64, f64, f64)]) -> bool {
        let c2 = self.self_limit2();
        if c2 <= 0.0 {
            return true;
        }
        let along = self.leg.input.self_along_m;
        let near = |a: &(f64, f64, f64), b: &(f64, f64, f64)| {
            (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) < c2
        };
        for (i, a) in new.iter().enumerate() {
            if new
                .iter()
                .skip(i + 1)
                .any(|b| b.2 - a.2 > along && near(a, b))
            {
                return false;
            }
        }
        let Some(last) = new.last().map(|l| l.2) else {
            return true;
        };
        let mut cur = parent;
        while let Some(n) = self.node(cur) {
            let (first, count) = n.samples;
            for h in self.samples.iter().skip(first).take(count) {
                if last - h.2 > along && new.iter().any(|a| a.2 - h.2 > along && near(a, h)) {
                    return false;
                }
            }
            cur = n.parent;
        }
        true
    }

    /// The expansion of a state: the arrival, then the five steps.
    fn expand(&mut self, idx: u32) {
        let Some(node) = self.node(idx).copied() else {
            return;
        };
        let d = dist(node.at, self.leg.input.end.at);
        self.closest = self.closest.min(d);
        if d <= self.leg.input.arrive_reach_m {
            self.try_arrivals(idx, 0);
        }
        let from = Self::pose_of(&node);
        for delta in self.leg.deltas {
            let length = self.leg.input.step_m;
            let poly = arc_polyline(from, delta, length, false);
            if let Some(ev) = self.evaluate(idx, &poly, delta) {
                self.add_arc(idx, from, delta, length, ev);
            }
        }
    }

    /// The first Dubins path of a state, from the path of rank `first_rank` on (shortest first), that passes the
    /// cheap tests becomes an arrival candidate. A path whose length alone, added to the cost of the state, reaches the
    /// cost of an arrival that was already checked is not tried (the cost of a path is at least its length), nor is
    /// one whose length cannot change the reachable heights as far as the end anchor needs.
    fn try_arrivals(&mut self, idx: u32, first_rank: usize) {
        let leg = self.leg;
        let Some(node) = self.node(idx).copied() else {
            return;
        };
        let from = Self::pose_of(&node);
        let end = leg.input.end;
        let Some(target) = leg.arrive_at else {
            if first_rank == 0 {
                self.arrive_at_dead_end(idx, &node, from, &end);
            }
            return;
        };
        for (rank, path) in dubins_paths(from, target, leg.input.radius)
            .into_iter()
            .enumerate()
            .skip(first_rank)
        {
            let length = path.length() + end.straight_m;
            // the cost of a candidate is at least the length of its chords, which is the arc length times
            // `sin x / x` (x the half angle of a chord); the curve term is not counted
            if node.g + length * leg.chord_ratio >= self.verified_g {
                // the paths are sorted by length: none of the next is cheaper
                break;
            }
            if !leg.height_ok(node.reach, length) {
                continue;
            }
            let poly = dubins_polyline(&path, from, leg.input.radius, &end, false);
            let turn = path.turning(leg.input.radius);
            let end_pose = Pose {
                at: end.at,
                heading: target.heading,
            };
            let via = Via::Dubins {
                path,
                rank: u8::try_from(rank).unwrap_or(u8::MAX),
            };
            if self.candidate(idx, &poly, turn, via, end_pose) {
                return;
            }
        }
    }

    /// The single arc through a dead end whose radius is not under the planning radius.
    fn arrive_at_dead_end(&mut self, idx: u32, node: &Node, from: Pose, end: &Anchor) {
        let radius = self.leg.input.radius;
        let dir = (end.at.0 - from.at.0, end.at.1 - from.at.1);
        let c = dir.0.hypot(dir.1);
        let alpha = wrap(dir.1.atan2(dir.0) - from.heading);
        if alpha.abs() >= std::f64::consts::FRAC_PI_2 || c <= 0.0 {
            return;
        }
        let (delta, length) = if alpha.abs() < 1e-9 {
            (0.0, c)
        } else {
            let r = c / (2.0 * alpha.abs().sin());
            if r < radius {
                return;
            }
            (2.0 * alpha, r * 2.0 * alpha.abs())
        };
        if node.g + length * self.leg.chord_ratio >= self.verified_g
            || !self.leg.height_ok(node.reach, length)
        {
            return;
        }
        let mut poly = arc_polyline(from, delta, length, false);
        if let Some(last) = poly.last_mut() {
            *last = end.at;
        }
        let end_pose = Pose {
            at: end.at,
            heading: from.heading + delta,
        };
        self.candidate(idx, &poly, delta, Via::Arc { delta, length }, end_pose);
    }

    /// An arrival candidate: it passes the tests of a step and the band's end tolerance. It goes into the open set with
    /// its lower-bound cost; its whole route is built and checked when it is popped.
    fn candidate(&mut self, idx: u32, poly: &[(f64, f64)], turn: f64, via: Via, end: Pose) -> bool {
        let leg = self.leg;
        if poly.len() < 2 {
            return false;
        }
        let Some(ev) = self.evaluate(idx, poly, turn) else {
            return false;
        };
        if !arrives(ev.reach, leg.z_end, leg.band.anchor_tol_m) {
            return false;
        }
        self.add(idx, end, via, ev, true).is_some()
    }

    /// The points of the route to a state, the start anchor first; `fine` samples an arc about every 2°.
    fn route_to(&self, idx: u32, fine: bool) -> Vec<(f64, f64)> {
        let mut chain = Vec::new();
        let mut cur = idx;
        while let Some(n) = self.node(cur) {
            chain.push(cur);
            if n.parent == NONE {
                break;
            }
            cur = n.parent;
        }
        chain.reverse();
        let mut points: Vec<(f64, f64)> = Vec::new();
        for &i in &chain {
            let Some(n) = self.node(i) else {
                continue;
            };
            let Some(parent) = self.node(n.parent) else {
                points.push(n.at);
                continue;
            };
            let from = Self::pose_of(parent);
            let poly = match n.via {
                Via::Start => Vec::new(),
                Via::Arc { delta, length } => {
                    let mut poly = arc_polyline(from, delta, length, fine);
                    if n.arrived
                        && let Some(last) = poly.last_mut()
                    {
                        *last = self.leg.input.end.at;
                    }
                    poly
                }
                Via::Dubins { path, .. } => dubins_polyline(
                    &path,
                    from,
                    self.leg.input.radius,
                    &self.leg.input.end,
                    fine,
                ),
            };
            points.extend(poly.into_iter().skip(1));
        }
        points
    }

    /// The route of a popped arrival, built on the fine points and checked from scratch: the points, the end, the
    /// radius, every segment, the stray cap, the crossings of every wall (each must equal what the original line
    /// made), the self-clearance over all pairs and the profile at the limit the route was found at. Returns the route
    /// with its real cost.
    fn finish(&self, idx: u32) -> Option<Route> {
        let leg = self.leg;
        let input = leg.input;
        let node = self.node(idx)?;
        let points = self.route_to(idx, true);
        let (first, last) = (points.first()?, points.last()?);
        if dist(*first, input.start.at) > END_SLACK_M
            || dist(*last, input.end.at) > END_SLACK_M
            || points.iter().any(|p| !finite2(*p))
        {
            return None;
        }
        let resampled: Vec<(f64, f64)> = line_samples(&points, SAMPLE_M).map(|a| a.p).collect();
        if resampled
            .windows(3)
            .any(|w| matches!(w, [a, b, c] if radius_at(*a, *b, *c) < leg.min_radius))
        {
            return None;
        }
        if points
            .windows(2)
            .any(|w| !matches!(w, [a, b] if input.obstacles.segment(*a, *b, input.clear).is_ok()))
            || points
                .iter()
                .any(|p| !leg.within_cap(*p, input.obstacles.stray_m(*p)))
            || input
                .walls
                .iter()
                .any(|w| w.crossings(&points) != w.allowed)
            || !self.route_self_clear(idx)
        {
            return None;
        }
        let profile = arrive(&points, input.ground, &leg.band).ok()?;
        if profile.max_grade() > leg.band.grade + GRADE_SLACK {
            return None;
        }
        let length: f64 = points
            .windows(2)
            .map(|w| match w {
                [a, b] => dist(*a, *b),
                _ => 0.0,
            })
            .sum();
        let (z0, z1) = (profile.ground().first()?, profile.ground().last()?);
        let climb = profile
            .heights()
            .windows(2)
            .map(|w| match w {
                [a, b] => (b - a).abs(),
                _ => 0.0,
            })
            .sum();
        let max_stray = points
            .iter()
            .map(|p| input.obstacles.stray_m(*p))
            .fold(0.0, f64::max);
        let max_move = points
            .iter()
            .map(|p| poly_dist(*p, input.original))
            .filter(|d| d.is_finite())
            .fold(0.0, f64::max);
        // the lower bound of the whole route never exceeds the real earthwork; where the steps' own sampling made it
        // higher, the higher counts
        let cost = if node.verified {
            node.g
        } else {
            node.g + input.weights.earth * (profile.earthwork_m2() - node.earth_low).max(0.0)
        };
        Some(Route {
            length_m: length,
            climb_m: climb,
            earthwork_m2: profile.earthwork_m2(),
            cut_max_m: profile.cut_max_m(),
            fill_max_m: profile.fill_max_m(),
            max_stray_m: max_stray,
            max_move_m: max_move,
            planned_grade: (z1 - z0).abs() / length,
            grade_limit: leg.band.grade,
            relax_steps: 0,
            cost,
            weight: self.weight,
            states: 0,
            points,
        })
    }

    /// All pairs of the route's samples more than `self_along_m` apart along it keep the self-clearance.
    fn route_self_clear(&self, idx: u32) -> bool {
        let c2 = self.self_limit2();
        let along = self.leg.input.self_along_m;
        if c2 <= 0.0 {
            return true;
        }
        let mut chain = Vec::new();
        let mut cur = idx;
        while let Some(n) = self.node(cur) {
            chain.push(n.samples);
            if n.parent == NONE {
                break;
            }
            cur = n.parent;
        }
        let all: Vec<&(f64, f64, f64)> = chain
            .iter()
            .rev()
            .flat_map(|&(first, count)| self.samples.iter().skip(first).take(count))
            .collect();
        for (i, a) in all.iter().enumerate() {
            for b in all.iter().skip(i + 1) {
                if b.2 - a.2 > along && (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) < c2 {
                    return false;
                }
            }
        }
        true
    }

    /// An arrival candidate that was popped: its whole route is checked. A route that fails is dropped, and the
    /// next Dubins path of the state it came from is tried. A route whose real cost is above the next entry of the
    /// open set is put back with its real cost, to be popped in its turn.
    fn settle(&mut self, idx: u32, node: &Node) -> Settled {
        if let Some(at) = self.pending.iter().position(|(i, _)| *i == idx) {
            // a verified arrival popped again: its route was checked and kept
            return Settled::Found(Box::new(self.pending.swap_remove(at).1));
        }
        let Some(route) = self.finish(idx) else {
            if let Via::Dubins { rank, .. } = node.via {
                self.try_arrivals(node.parent, usize::from(rank) + 1);
            }
            return Settled::Dropped;
        };
        let next = self.heap.peek().map(|top| top.prio);
        if node.verified || next.is_none_or(|n| route.cost <= n + SLACK) {
            return Settled::Found(Box::new(route));
        }
        let mut again = *node;
        again.g = route.cost;
        again.verified = true;
        self.verified_g = self.verified_g.min(route.cost);
        if let Some(again_idx) = self.push(again) {
            self.pending.push((again_idx, route));
        }
        Settled::Dropped
    }

    /// One try of the search within a budget of expanded states.
    fn run(&mut self, budget: usize) -> Tried {
        if !self.start() {
            return Tried::Emptied;
        }
        while let Some(item) = self.heap.pop() {
            let Some(node) = self.node(item.idx).copied() else {
                continue;
            };
            if node.arrived {
                match self.settle(item.idx, &node) {
                    Settled::Found(route) => return Tried::Found(route),
                    Settled::Dropped => continue,
                }
            }
            // a state that another dominated is stale
            if !self.alive(&node, item.idx) {
                continue;
            }
            if self.expanded >= budget {
                return Tried::Exhausted;
            }
            self.expanded += 1;
            self.expand(item.idx);
        }
        Tried::Emptied
    }
}

/// `a` as an angle in `-PI..=PI`.
fn wrap(a: f64) -> f64 {
    let r = a.rem_euclid(TAU);
    if r > std::f64::consts::PI { r - TAU } else { r }
}

/// The search of one leg at one grade limit, with the tries of the budgets.
fn search_at(
    input: &LegInput<'_>,
    band: Band,
    relax_steps: usize,
) -> Result<LegOutcome, RerouteError> {
    let none = |exhausted: bool, states: usize, closest: f64| {
        Ok(LegOutcome::NoRoute {
            budget_exhausted: exhausted,
            states,
            closest_m: closest,
        })
    };
    let closest0 = dist(input.start.at, input.end.at);
    let Some(leg) = Leg::new(input, band) else {
        return none(false, 0, closest0);
    };
    let (mut states, mut closest) = (0_usize, closest0);
    for &(weight, budget) in input.budgets {
        let mut search = Search::new(&leg, weight);
        let tried = search.run(budget.min(MAX_STATES));
        states = states.saturating_add(search.expanded);
        closest = closest.min(search.closest);
        match tried {
            Tried::Found(mut route) => {
                route.relax_steps = relax_steps;
                route.states = states;
                return Ok(LegOutcome::Found(*route));
            }
            Tried::Emptied => return none(false, states, closest),
            Tried::Exhausted => {}
        }
    }
    none(true, states, closest)
}

/// The checks from scratch of a route that a search returned, for the caller that accepts it: the points finite and
/// at the two anchors (within 10⁻⁶ m), the tightest three-point radius on a 2 m resample (no smaller than
/// `min_radius_m`, the caller's `PLAN_REACHED_FRACTION` of the family's tightest arc), every segment against the
/// obstacles, the stray cap (the raster's stored measure, or the true one below two cells, as
/// [`LegInput::stray_cap_m`] says), every wall ([`Wall::is_kept`]: a road or a stream exactly as often as the
/// drawn line, a deck or a tunnel no more often), the self-clearance over all pairs of its samples more than
/// [`LegInput::self_along_m`] apart, and the two-pass profile at the grade of `band` (the band the route was
/// found at). A route that fails is a bug: the leg keeps its line with detail `check_failed`.
#[must_use]
pub fn check_route(
    input: &LegInput<'_>,
    band: &Band,
    min_radius_m: f64,
    points: &[(f64, f64)],
) -> bool {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return false;
    };
    if dist(*first, input.start.at) > END_SLACK_M
        || dist(*last, input.end.at) > END_SLACK_M
        || points.iter().any(|p| !finite2(*p))
    {
        return false;
    }
    let Some(leg) = Leg::new(input, *band) else {
        return false;
    };
    let samples: Vec<(f64, f64, f64)> = line_samples(points, SAMPLE_M)
        .scan(0.0_f64, |along, at| {
            *along += at.ds;
            Some((at.p.0, at.p.1, *along))
        })
        .collect();
    let resampled: Vec<(f64, f64)> = samples.iter().map(|&(x, y, _)| (x, y)).collect();
    if resampled
        .windows(3)
        .any(|w| matches!(w, [a, b, c] if radius_at(*a, *b, *c) < min_radius_m))
    {
        return false;
    }
    if points
        .windows(2)
        .any(|w| !matches!(w, [a, b] if input.obstacles.segment(*a, *b, input.clear).is_ok()))
        || points
            .iter()
            .any(|p| !leg.within_cap(*p, input.obstacles.stray_m(*p)))
        || input.walls.iter().any(|w| !w.is_kept(points))
    {
        return false;
    }
    // the self-clearance, over every pair of the route's samples that is more than `self_along_m` apart along it
    let c = (input.self_clear_m - SELF_SLACK_M).max(0.0);
    if c > 0.0 {
        for (i, a) in samples.iter().enumerate() {
            for b in samples.iter().skip(i + 1) {
                if b.2 - a.2 > input.self_along_m
                    && (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) < c * c
                {
                    return false;
                }
            }
        }
    }
    match arrive(points, input.ground, band) {
        Ok(profile) => profile.max_grade() <= band.grade + GRADE_SLACK,
        Err(_) => false,
    }
}

/// A route's write-back: the points the stretch is written with, and whether the write-back fell back to the
/// samples (see [`written_line`]).
#[derive(Debug, Clone, PartialEq)]
pub struct WrittenLine {
    /// The points of the written line. The two anchors stay bit for bit, whatever was dropped.
    pub points: Vec<(f64, f64)>,
    /// The write-back fell back: the line failed the check after its drop, so the samples of the search are
    /// written instead. `false` also covers a line with nothing to drop and the drop that is switched off
    /// altogether (a tolerance that is not positive keeps every sample by [`simplify_line`]'s rule, too).
    pub fell_back: bool,
}

/// The write-back of a route the search returned and [`check_route`] accepted: the sampled line at the stage's own
/// sampling — [`resample`] at [`SAMPLE_M`] — with the points dropped that a straight between two of its samples does
/// not need ([`simplify_line`] at `tolerance`, see [`crate::reroute::PLAN_WRITE_TOL_M`]).
///
/// The resampling is the drop that matters. The search samples an arc about every 2° of its curvature — 0.37 m at the
/// planning radius of 10.5 m — and the write-back of that line would write a route more than ten times as dense as
/// the network around it. On the stage's `SAMPLE_M` sampling the same arc turns 10.9° per sample, and the measures the
/// stage reads a written line with — the radius of [`check_route`], the tight length of the report — sample at
/// `SAMPLE_M` again, so the written arc still reads its own radius: 10.4 m against the 9.9 m the pieces ask for,
/// whatever the phase of the samples. An arc cannot be dropped further: [`simplify_line`] at a tolerance that is
/// worth having (the smoothing stage's 0.20 m) leaves chords of about 4 m whose joints turn 22°, and the resample
/// reads the joint, not the arc the chords stand for: 5 m. So the tolerance only takes what is collinear, and
/// straight parts lose every interior point while arcs keep the sampling.
///
/// The written line still passes [`check_route`] from scratch: a longer way over an obstacle, a wall crossed one
/// time too many and the rest of the checks run on it as on the line the search returned. A line that fails keeps
/// that line, sample by sample, and [`WrittenLine::fell_back`] says so; a tolerance that is not positive keeps every
/// sample of the search. No point of the searched line is further from the written line than the sagitta of a
/// `SAMPLE_M` chord of an arc at the planning radius: 0.05 m at 10.5 m.
#[must_use]
pub fn written_line(
    input: &LegInput<'_>,
    band: &Band,
    min_radius_m: f64,
    route: &Route,
    tolerance: f64,
) -> WrittenLine {
    let sampled = resample(&route.points, SAMPLE_M);
    let simplified = if tolerance > 0.0 {
        simplify_line(&sampled, tolerance)
    } else {
        route.points.clone()
    };
    let droppable = simplified.len() < route.points.len();
    let (written, fell_back) = if droppable && check_route(input, band, min_radius_m, &simplified) {
        (simplified, false)
    } else {
        (route.points.clone(), droppable)
    };
    WrittenLine {
        points: written,
        fell_back,
    }
}

/// The distance that the own road's samples that are more than `self_along_m` apart along it must keep from each other
/// (the `self_clear_m` of [`LegInput`]), set from the original line of the leg: the smallest distance between two
/// samples of the original line (every 2 m) that are more than `self_along_m` apart along it, and the distance between
/// its two ends whatever their distance along it, but no more than `road_threshold_m` and no less than `width_m` (the
/// road's width). A hairpin whose legs are 8 m apart gets 8 m, not the road clearance: a bend and the spread of its
/// legs are one road. The ends count because a route between close anchors is longer than `self_along_m` even where
/// the drawing is shorter. A line of one point, or with a coordinate that is not a number, gets the threshold (but not
/// less than the width).
///
/// `road_threshold_m` is the road clearance itself (about 13 m: the reach of the road plus a terrain cell), because the
/// samples are compared by their true distance; not the raster's `thr(c) = (c + res) · 1.0824 + res · √2`, which is
/// the measure of the chamfer map.
#[must_use]
pub fn self_clearance(
    original: &[(f64, f64)],
    self_along_m: f64,
    road_threshold_m: f64,
    width_m: f64,
) -> f64 {
    let positive = |v: f64| if v.is_finite() { v.max(0.0) } else { 0.0 };
    let (threshold, width) = (positive(road_threshold_m), positive(width_m));
    if !self_along_m.is_finite() || original.iter().any(|p| !finite2(*p)) {
        return threshold.max(width);
    }
    let total: f64 = original
        .windows(2)
        .map(|w| match w {
            [a, b] => dist(*a, *b),
            _ => 0.0,
        })
        .sum();
    let spacing = SAMPLE_M.max(total / SELF_MAX_SAMPLES);
    let mut s = 0.0;
    let samples: Vec<(f64, f64, f64)> = line_samples(original, spacing)
        .map(|at| {
            s += at.ds;
            (at.p.0, at.p.1, s)
        })
        .collect();
    // The two ends of the original line count whatever their distance along it: the route between them is at least
    // as long as the Dubins path between the poses, often longer than `self_along_m` where the drawing was shorter.
    let mut least = match (original.first(), original.last()) {
        (Some(first), Some(last)) if original.len() > 1 => threshold.min(dist(*first, *last)),
        _ => threshold,
    };
    for (i, a) in samples.iter().enumerate() {
        for b in samples.iter().skip(i + 1) {
            if b.2 - a.2 > self_along_m {
                least = least.min(dist((a.0, a.1), (b.0, b.1)));
            }
        }
    }
    least.max(width)
}

/// Searches one leg at the grade limit of `input.band`: a hybrid A* over poses between the two anchors, in the
/// tries of `input.budgets`. A route is accepted only if its whole profile keeps the grade limit.
///
/// # Errors
///
/// [`RerouteError::NotFinite`] for an anchor, a radius, a step, a weight, a cap, a clearance or a band value that is
/// not a finite number (the grade may be infinite); [`RerouteError::Degenerate`] for anchors at one point, a radius
/// or step that is not positive or above 1000 m, an arrival reach above 10 000 m, a negative weight, cap, clearance or
/// straight, or no budget.
pub fn search_leg(input: &LegInput<'_>) -> Result<LegOutcome, RerouteError> {
    validate(input)?;
    search_at(input, *input.band, 0)
}

/// Searches one leg at each of the grade `limits` in turn until one finds a route; `relax_steps` of the route is the
/// index of that limit. The band's grade is replaced by each limit; the field and the raster are shared. The
/// states of the failed limits count in `states`. Without a route at any limit, the outcome is that of the last.
///
/// # Errors
///
/// As [`search_leg`], and [`RerouteError::NotFinite`] for a limit that is NaN, [`RerouteError::Degenerate`] for no
/// limit or one that is negative.
pub fn search_leg_relaxed(
    input: &LegInput<'_>,
    limits: &[f64],
) -> Result<LegOutcome, RerouteError> {
    validate(input)?;
    if limits.iter().any(|l| l.is_nan()) {
        return Err(RerouteError::NotFinite);
    }
    if limits.is_empty() || limits.iter().any(|l| *l < 0.0) {
        return Err(RerouteError::Degenerate);
    }
    let (mut states, mut closest) = (0_usize, f64::INFINITY);
    let mut last = None;
    for (k, &limit) in limits.iter().enumerate() {
        let band = Band {
            grade: limit,
            ..*input.band
        };
        match search_at(input, band, k)? {
            LegOutcome::Found(mut route) => {
                route.states = route.states.saturating_add(states);
                return Ok(LegOutcome::Found(route));
            }
            LegOutcome::NoRoute {
                budget_exhausted,
                states: n,
                closest_m,
            } => {
                states = states.saturating_add(n);
                closest = closest.min(closest_m);
                last = Some(budget_exhausted);
            }
        }
    }
    Ok(LegOutcome::NoRoute {
        budget_exhausted: last.unwrap_or(false),
        states,
        closest_m: closest,
    })
}

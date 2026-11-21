// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The arc search of one leg: a hybrid A* over poses from the start anchor to the end anchor (spec: "The search for
//! one leg", clean-room notes 3.5).
//!
//! A state is a pose with the cost so far and the interval of road heights reachable there. Five arcs of one step
//! leave every state; from every state within [`LegInput::arrive_reach_m`] of the end anchor the Dubins paths of
//! the planning radius to the end pose are tried, and the first that passes every test is an *arrived* state. The
//! forward band only prunes: a route is accepted only through [`arrive`], the two-pass profile of the whole
//! route, whose earthwork replaces the lower bound in the cost of the arrived state. The search is deterministic:
//! its open set is ordered by priority, cost, state key and insertion order with `total_cmp`, and the table of
//! state keys is only looked up, never iterated.
//!
//! The estimate of the cost to go is the larger of the cost-to-go [`Field`] and an analytic bound. The field is
//! not strictly admissible (see its module), so the search at weight 1 is not exact either; it is as good as the
//! field. Budgets are counts of expanded states, never time.
//!
//! All input is untrusted: nothing here panics, and every allocation is bounded by a budget.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::f64::consts::TAU;

use crate::smooth::radius_at;

use super::band::{Band, Reach, arrive, arrives, band_step, check_band, line_samples};
use super::dubins::{DubinsPath, arc_points, dubins_paths, follow, segments};
use super::field::Field;
use super::raster::{Clearance, Obstacles, Wall};
use super::{
    Anchor, PLAN_MAX_TURN_DEG, PLAN_RADIUS_MARGIN, PLAN_REACHED_FRACTION, Pose, RerouteError,
    RerouteParams, SAMPLE_M,
};

/// The most states one try may expand, whatever its budget says.
const MAX_STATES: usize = 1_000_000;
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
    /// The planning radius: no arc is tighter, in metres.
    pub radius: f64,
    /// The length of a search step, in metres.
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
    /// The clearances of the segment test. `clear.road_m()` is also the self-clearance.
    pub clear: Clearance,
    /// The cost-to-go field of the leg (built for the end anchor).
    pub field: &'a Field,
    /// The weights of the cost.
    pub weights: Weights,
    /// The furthest a route may be from the original line, in the raster's stored measure, in metres.
    pub stray_cap_m: f64,
    /// Dubins arrivals are tried from states this near the end anchor, in metres.
    pub arrive_reach_m: f64,
    /// Two samples of the route more than this far apart along it must keep the road clearance from each other,
    /// in metres.
    pub self_along_m: f64,
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
    /// The largest distance of a route point from the original line, in the raster's stored measure, in metres.
    pub max_stray_m: f64,
    /// The net rise between the anchors over the length: `|Δz| / length`, never the summed climb.
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
    /// No route. `budget_exhausted` is true when the last try ran out of states, false when its open set ran
    /// empty (there is none, up to the discretisation). `closest_m` is the nearest an expanded state came to the
    /// end anchor.
    NoRoute {
        /// The last budget ran out before the search ended.
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
    /// A Dubins path to the end pose, then the end anchor's straight (if it has one).
    Dubins(DubinsPath),
}

/// A state of the search, in an arena.
#[derive(Debug, Clone, Copy)]
struct Node {
    at: (f64, f64),
    heading: f64,
    /// The cost so far.
    g: f64,
    /// The lower bound of the earthwork so far, in m² (inside `g`, weighted).
    earth_low: f64,
    reach: Reach,
    parent: u32,
    via: Via,
    arrived: bool,
    key: u64,
    /// The route length at this state, in metres.
    along: f64,
    /// Crossings of the walls that allow some, so far.
    crossed: usize,
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

/// The bounding box of a wall, to skip the walls a step is far from.
#[derive(Debug, Clone, Copy)]
struct WallBox {
    lo: (f64, f64),
    hi: (f64, f64),
}

impl WallBox {
    fn of(line: &[(f64, f64)]) -> WallBox {
        let fold = |f: fn(f64, f64) -> f64, init: f64, pick: fn(&(f64, f64)) -> f64| {
            line.iter().map(pick).fold(init, f)
        };
        WallBox {
            lo: (
                fold(f64::min, f64::INFINITY, |p| p.0),
                fold(f64::min, f64::INFINITY, |p| p.1),
            ),
            hi: (
                fold(f64::max, f64::NEG_INFINITY, |p| p.0),
                fold(f64::max, f64::NEG_INFINITY, |p| p.1),
            ),
        }
    }

    /// Whether a polyline's box overlaps this one. A box with a NaN overlaps nothing, and so does a wall
    /// with a coordinate that is not a number: `crossings` finds no crossing with it either.
    fn near(&self, lo: (f64, f64), hi: (f64, f64)) -> bool {
        lo.0 <= self.hi.0 && hi.0 >= self.lo.0 && lo.1 <= self.hi.1 && hi.1 >= self.lo.1
    }
}

/// The distance between two points.
fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
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
    walls: Vec<WallBox>,
    /// How many crossings of the walls that allow some a route may have in all.
    allowed: usize,
    /// The tightest three-point radius a route may have.
    min_radius: f64,
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
    let negative = numbers.iter().take(9).any(|v| *v < 0.0)
        || input.start.straight_m < 0.0
        || input.end.straight_m < 0.0
        || input.budgets.iter().any(|b| b.0 < 0.0);
    if negative
        || input.radius <= 0.0
        || input.step_m <= 0.0
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
        let allowed = input
            .walls
            .iter()
            .filter(|w| w.allowed > 0)
            .map(|w| w.allowed)
            .fold(0_usize, usize::saturating_add);
        Some(Leg {
            input,
            band,
            z_end,
            arrive_at,
            deltas: [0.0, theta / 2.0, -theta / 2.0, theta, -theta],
            bins: bin_count((TAU / (theta / 4.0)).ceil()),
            origin: input.start.at,
            cell: input.step_m / 2.0,
            walls: input.walls.iter().map(|w| WallBox::of(&w.line)).collect(),
            allowed,
            min_radius: PLAN_REACHED_FRACTION * input.radius / PLAN_RADIUS_MARGIN,
        })
    }

    /// The key of a state: arrived, its cell and its heading bin.
    fn key(&self, at: (f64, f64), heading: f64, arrived: bool) -> u64 {
        let bin = (norm(heading) / TAU * to_f64(self.bins)).round();
        let bin = u64::try_from(bin_index(bin, self.bins)).unwrap_or(0);
        let cx = cell_of((at.0 - self.origin.0) / self.cell);
        let cy = cell_of((at.1 - self.origin.1) / self.cell);
        (u64::from(arrived) << 62) | (bin << 54) | (cx << 27) | cy
    }

    /// The estimate of the cost to go from a point: the larger of the field and the analytic bound.
    fn heuristic(&self, at: (f64, f64)) -> f64 {
        let d = dist(at, self.input.end.at);
        let dz = (self.input.ground)(at)
            .filter(|z| z.is_finite())
            .map_or(0.0, |z| {
                ((self.z_end - z).abs() - self.band.cut_m - self.band.fill_m).max(0.0)
            });
        let climb = if dz <= 0.0 {
            0.0
        } else if self.band.grade > 0.0 {
            dz / self.band.grade
        } else {
            f64::INFINITY
        };
        let analytic = d.max(climb) + self.input.weights.slope * dz;
        let h = analytic.max(self.input.field.estimate(at));
        if h.is_nan() {
            0.0
        } else {
            h.min(HEURISTIC_CAP)
        }
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
    crossed: usize,
    samples: Vec<(f64, f64, f64)>,
}

/// How one try ended.
enum Tried {
    Found(Route),
    Proven,
    Exhausted,
}

struct Search<'a, 'b> {
    leg: &'b Leg<'a>,
    weight: f64,
    nodes: Vec<Node>,
    samples: Vec<(f64, f64, f64)>,
    best: HashMap<u64, u32>,
    heap: BinaryHeap<Item>,
    seq: u64,
    expanded: usize,
    closest: f64,
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

/// The polyline of an arc (straight for a heading change of 0) that starts at `from`.
fn arc_polyline(from: Pose, delta: f64, length: f64) -> Vec<(f64, f64)> {
    let curvature = if length > 0.0 { delta / length } else { 0.0 };
    arc_points(from, curvature, length, segments(delta, length, SAMPLE_M))
}

/// The polyline of a Dubins arrival from `from`: the path, then the end anchor's straight, or the path's end
/// moved onto the anchor.
fn dubins_polyline(path: &DubinsPath, from: Pose, radius: f64, end: &Anchor) -> Vec<(f64, f64)> {
    let mut poly = path.points(from, radius, SAMPLE_M);
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
            best: HashMap::new(),
            heap: BinaryHeap::new(),
            seq: 0,
            expanded: 0,
            closest: dist(leg.input.start.at, leg.input.end.at),
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

    /// Adds a state to the arena and, if no cheaper state has its key, to the open set.
    fn push(&mut self, node: Node) -> Option<u32> {
        if let Some(&b) = self.best.get(&node.key)
            && self.node(b).is_some_and(|o| o.g <= node.g)
        {
            return None;
        }
        let idx = u32::try_from(self.nodes.len()).ok()?;
        let h = if node.arrived {
            0.0
        } else {
            self.leg.heuristic(node.at)
        };
        self.best.insert(node.key, idx);
        self.heap.push(Item {
            prio: node.g + self.weight * h,
            g: node.g,
            key: node.key,
            seq: self.seq,
            idx,
        });
        self.seq = self.seq.saturating_add(1);
        self.nodes.push(node);
        Some(idx)
    }

    /// The root states: one at the start anchor, or one per heading at a dead end.
    fn start(&mut self) -> bool {
        let leg = self.leg;
        let input = leg.input;
        let Some(z) = (input.ground)(input.start.at).filter(|z| z.is_finite()) else {
            return false;
        };
        let reach = Reach::at_anchor(z, &leg.band);
        let root = |heading: f64| Node {
            at: input.start.at,
            heading: norm(heading),
            g: 0.0,
            earth_low: 0.0,
            reach,
            parent: NONE,
            via: Via::Start,
            arrived: false,
            key: leg.key(input.start.at, heading, false),
            along: 0.0,
            crossed: 0,
            samples: (0, 1),
        };
        self.samples.push((input.start.at.0, input.start.at.1, 0.0));
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
        let Some(parent) = self.push_root_only(node) else {
            return false;
        };
        let from = Pose {
            at: input.start.at,
            heading: norm(h),
        };
        let length = input.start.straight_m;
        let poly = arc_polyline(from, 0.0, length);
        let Some(ev) = self.evaluate(parent, &poly, 0.0) else {
            return false;
        };
        self.add_arc(parent, from, 0.0, length, ev)
    }

    /// A root that only anchors a chain: in the arena, not in the open set.
    fn push_root_only(&mut self, node: Node) -> Option<u32> {
        let idx = u32::try_from(self.nodes.len()).ok()?;
        self.nodes.push(node);
        Some(idx)
    }

    /// The new child of `parent` that follows an arc of the given heading change and length.
    fn add_arc(&mut self, parent: u32, from: Pose, delta: f64, length: f64, ev: Eval) -> bool {
        let curvature = if length > 0.0 { delta / length } else { 0.0 };
        let end = follow(from, curvature, length);
        self.add(parent, end, Via::Arc { delta, length }, ev, false)
            .is_some()
    }

    /// A state after a step or an arrival.
    fn add(&mut self, parent: u32, end: Pose, via: Via, ev: Eval, arrived: bool) -> Option<u32> {
        let p = *self.node(parent)?;
        let first = self.samples.len();
        let count = ev.samples.len();
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
            key: self.leg.key(end.at, heading, arrived),
            along: p.along + ev.length,
            crossed: p.crossed.saturating_add(ev.crossed),
            samples: (first, count),
        };
        let idx = self.push(node);
        if idx.is_some() {
            self.samples.extend(ev.samples);
        }
        idx
    }

    /// The crossings of the walls by a polyline: `None` where a wall that allows none is crossed.
    fn wall_crossings(&self, poly: &[(f64, f64)]) -> Option<usize> {
        let leg = self.leg;
        let fold = |f: fn(f64, f64) -> f64, init: f64, pick: fn(&(f64, f64)) -> f64| {
            poly.iter().map(pick).fold(init, f)
        };
        let lo = (
            fold(f64::min, f64::INFINITY, |p| p.0),
            fold(f64::min, f64::INFINITY, |p| p.1),
        );
        let hi = (
            fold(f64::max, f64::NEG_INFINITY, |p| p.0),
            fold(f64::max, f64::NEG_INFINITY, |p| p.1),
        );
        let mut crossed = 0_usize;
        for (wall, wbox) in leg.input.walls.iter().zip(&leg.walls) {
            if !wbox.near(lo, hi) {
                continue;
            }
            let n = wall.crossings(poly);
            if n > 0 && wall.allowed == 0 {
                return None;
            }
            if wall.allowed > 0 {
                crossed = crossed.saturating_add(n);
            }
        }
        Some(crossed)
    }

    /// Every test of a step or an arrival, and its cost: the stray cap, the walls, the band (which also tests every
    /// segment against the obstacles), the self-clearance. `turn` is the heading change, in radians.
    fn evaluate(&self, parent: u32, poly: &[(f64, f64)], turn: f64) -> Option<Eval> {
        let leg = self.leg;
        let input = leg.input;
        let p = self.node(parent)?;
        let cap = input.stray_cap_m;
        if poly.iter().any(|q| {
            !finite2(*q)
                || input.obstacles.stray_m(*q).is_nan()
                || input.obstacles.stray_m(*q) > cap
        }) {
            return None;
        }
        let crossed = self.wall_crossings(poly)?;
        if p.crossed.saturating_add(crossed) > leg.allowed {
            return None;
        }
        let step = band_step(
            poly,
            input.ground,
            &leg.band,
            p.reach,
            input.obstacles,
            input.clear,
        )
        .ok()?;
        let mut length = 0.0;
        let mut stray = 0.0;
        for w in poly.windows(2) {
            if let [a, b] = *w {
                let l = dist(a, b);
                length += l;
                stray += l * f64::midpoint(input.obstacles.stray_m(a), input.obstacles.stray_m(b));
            }
        }
        let samples = step_samples(p.along, poly);
        if !self.self_clear(parent, &samples) {
            return None;
        }
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
            crossed,
            samples,
        })
    }

    /// Whether the new samples keep the road clearance from the earlier samples of the route (and from each
    /// other) that are more than `self_along_m` back along it.
    fn self_clear(&self, parent: u32, new: &[(f64, f64, f64)]) -> bool {
        let c = self.leg.input.clear.road_m();
        let along = self.leg.input.self_along_m;
        if c <= 0.0 {
            return true;
        }
        let c2 = c * c;
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
            self.arrival(idx, &node);
        }
        let from = Self::pose_of(&node);
        for delta in self.leg.deltas {
            let length = self.leg.input.step_m;
            let poly = arc_polyline(from, delta, length);
            if let Some(ev) = self.evaluate(idx, &poly, delta) {
                self.add_arc(idx, from, delta, length, ev);
            }
        }
    }

    /// The first Dubins path (or the arc through a dead end) from a state to the end pose that passes every test
    /// and the profile of the whole route becomes an arrived state.
    fn arrival(&mut self, idx: u32, node: &Node) {
        let leg = self.leg;
        let from = Self::pose_of(node);
        let end = leg.input.end;
        let Some(target) = leg.arrive_at else {
            self.arrive_at_dead_end(idx, from, &end);
            return;
        };
        for path in dubins_paths(from, target, leg.input.radius) {
            let poly = dubins_polyline(&path, from, leg.input.radius, &end);
            let turn = path.turning(leg.input.radius);
            let end_pose = Pose {
                at: end.at,
                heading: target.heading,
            };
            if self.arrived(idx, &poly, turn, Via::Dubins(path), end_pose) {
                return;
            }
        }
    }

    /// The single arc through a dead end whose radius is not under the planning radius.
    fn arrive_at_dead_end(&mut self, idx: u32, from: Pose, end: &Anchor) {
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
        let mut poly = arc_polyline(from, delta, length);
        if let Some(last) = poly.last_mut() {
            *last = end.at;
        }
        let heading = from.heading + delta;
        let end_pose = Pose {
            at: end.at,
            heading,
        };
        self.arrived(idx, &poly, delta, Via::Arc { delta, length }, end_pose);
    }

    /// An arrival candidate: it passes the tests of a step, the band's end tolerance, and the profile of the whole
    /// route from the first anchor to the last, at the same grade limit. The cost of the state is the step's cost
    /// with the real earthwork instead of the lower bound, where that is higher.
    fn arrived(&mut self, idx: u32, poly: &[(f64, f64)], turn: f64, via: Via, end: Pose) -> bool {
        let leg = self.leg;
        if poly.len() < 2 {
            return false;
        }
        let Some(mut ev) = self.evaluate(idx, poly, turn) else {
            return false;
        };
        if !arrives(ev.reach, leg.z_end, leg.band.anchor_tol_m) {
            return false;
        }
        let mut points = self.route_to(idx);
        points.extend(poly.iter().skip(1).copied());
        let Ok(profile) = arrive(&points, leg.input.ground, &leg.band) else {
            return false;
        };
        let Some(p) = self.node(idx) else {
            return false;
        };
        // the lower bound of the whole route never exceeds the real earthwork; where the steps' own sampling made
        // it higher, the higher counts
        let low = p.earth_low + ev.earth_low;
        let extra = (profile.earthwork_m2() - low).max(0.0);
        ev.cost += leg.input.weights.earth * extra;
        self.add(idx, end, via, ev, true).is_some()
    }

    /// The points of the route to a state, the start anchor first.
    fn route_to(&self, idx: u32) -> Vec<(f64, f64)> {
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
                    let mut poly = arc_polyline(from, delta, length);
                    if n.arrived
                        && let Some(last) = poly.last_mut()
                    {
                        *last = self.leg.input.end.at;
                    }
                    poly
                }
                Via::Dubins(path) => {
                    dubins_polyline(&path, from, self.leg.input.radius, &self.leg.input.end)
                }
            };
            points.extend(poly.into_iter().skip(1));
        }
        points
    }

    /// The route of a popped arrived state, checked once more from scratch: the points, the end, the radius, every
    /// segment, the stray cap, the walls, the self-clearance over all pairs and the profile at the limit the route
    /// was found at.
    fn finish(&self, idx: u32) -> Option<Route> {
        let leg = self.leg;
        let input = leg.input;
        let node = self.node(idx)?;
        let points = self.route_to(idx);
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
                .any(|p| input.obstacles.stray_m(*p) > input.stray_cap_m)
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
        let route = Route {
            length_m: length,
            climb_m: climb,
            earthwork_m2: profile.earthwork_m2(),
            cut_max_m: profile.cut_max_m(),
            fill_max_m: profile.fill_max_m(),
            max_stray_m: max_stray,
            planned_grade: (z1 - z0).abs() / length,
            grade_limit: leg.band.grade,
            relax_steps: 0,
            cost: node.g,
            weight: self.weight,
            states: 0,
            points,
        };
        Some(route)
    }

    /// All pairs of the route's samples more than `self_along_m` apart along it keep the road clearance.
    fn route_self_clear(&self, idx: u32) -> bool {
        let c = self.leg.input.clear.road_m();
        let along = self.leg.input.self_along_m;
        if c <= 0.0 {
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
                if b.2 - a.2 > along && (a.0 - b.0).hypot(a.1 - b.1) < c {
                    return false;
                }
            }
        }
        true
    }

    /// One try of the search within a budget of expanded states.
    fn run(&mut self, budget: usize) -> Tried {
        if !self.start() {
            return Tried::Proven;
        }
        while let Some(item) = self.heap.pop() {
            let Some(node) = self.node(item.idx).copied() else {
                continue;
            };
            // a state that a cheaper one with its key replaced is stale
            if self.best.get(&node.key) != Some(&item.idx) {
                continue;
            }
            if node.arrived {
                if let Some(route) = self.finish(item.idx) {
                    return Tried::Found(route);
                }
                // a route that fails its check is dropped, the search goes on
                continue;
            }
            if self.expanded >= budget {
                return Tried::Exhausted;
            }
            self.expanded += 1;
            self.expand(item.idx);
        }
        Tried::Proven
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
                return Ok(LegOutcome::Found(route));
            }
            Tried::Proven => return none(false, states, closest),
            Tried::Exhausted => {}
        }
    }
    none(true, states, closest)
}

/// Searches one leg at the grade limit of `input.band`: a hybrid A* over poses between the two anchors, in the
/// tries of `input.budgets`. A route is accepted only if its whole profile keeps the grade limit.
///
/// # Errors
///
/// [`RerouteError::NotFinite`] for an anchor, a radius, a step, a weight, a cap or a band value that is not a
/// finite number (the grade may be infinite); [`RerouteError::Degenerate`] for anchors at one point, a radius or step
/// that is not positive, a negative weight, cap or straight, or no budget.
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

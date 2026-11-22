// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Road planning: a bend tighter than the road pieces can build is re-routed across the terrain between two
//! anchors on its own road.
//!
//! The design is in `docs/superpowers/specs/2026-11-21-road-planning-design.md`. This module holds the named
//! constants of its constants table and [`RerouteParams`], which collects them so that tests can change them.
//! The parts are sub-modules: [`raster`] is the obstacle raster with its distance maps and wall lines, [`band`] the
//! feasibility band, the profile and the grade limit of a leg, [`field`] the cost-to-go field of a leg, [`dubins`]
//! the Dubins paths the search arrives with and [`search`] the arc search of one leg.

use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;

use rayon::prelude::*;

use crate::catalog::Family;
use crate::conform::core_reach_m;
use crate::heightmap::Heightmap;
use crate::reroute::band::{Band, arrive, drawn_grade, ground_at, leg_limit, relaxed_limits};
use crate::reroute::field::Field;
use crate::reroute::raster::{
    Clearance, Obstacles, Wall, WallKind, crossings_away, exempt_samples, samples_with_arc,
};
use crate::reroute::search::{
    LegInput, LegOutcome, Route, Weights, search_leg_relaxed, self_clearance,
};
use crate::roadnet::{RoadGraph, Stretch, crossings_of};
use crate::smooth::{ChainPlan, Link, WaterTest, plan_chains, radius_at, resample, tight_runs};

pub mod band;
pub mod dubins;
pub mod field;
pub mod raster;
pub mod search;

/// A position and the direction a road runs in there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// The position, in map metres.
    pub at: (f64, f64),
    /// The heading, in radians, counter-clockwise from +x.
    pub heading: f64,
}

/// One end of a leg: where the planned road starts or ends, and how.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    /// The position, in map metres. It never moves.
    pub at: (f64, f64),
    /// The heading of the road through the anchor, in radians, counter-clockwise from +x; the direction the road
    /// travels in, at the start and at the end of the leg alike. `None` at a dead end: the direction is free.
    pub heading: Option<f64>,
    /// A straight of this length along the heading, at the start after the anchor, at the end before it (a
    /// junction's arm; 0 elsewhere). Ignored without a heading.
    pub straight_m: f64,
}

/// Why a leg is not searched at all: the input is wrong, which is a bug of the caller and not a leg without a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RerouteError {
    /// An input that has to be a finite number is not (an anchor, the radius, a weight, a cap, a band value).
    #[error("an input of the leg is not a finite number")]
    NotFinite,
    /// The two anchors are the same point, or a radius, a step, a weight or a budget makes no search.
    #[error("the leg is degenerate")]
    Degenerate,
}

/// The switch: road planning runs only when this is on. Off until the result is judged.
pub const REROUTE_ENABLED: bool = false;
/// Octile and chamfer excess: a chamfer distance is up to this much longer than the straight one.
pub const CHAMFER_ERROR: f64 = 1.0824;
/// A two-stretch node that turns more than this (degrees) ends a chain.
pub const CHAIN_TURN_MAX_DEG: f64 = crate::smooth::CHAIN_TURN_MAX_DEG;
/// Spacing of the samples of a road for the exemption and the stamping, in metres.
pub const SAMPLE_M: f64 = crate::smooth::SAMPLE_M;
/// Minimum radii of road beside a tight run that belong to its zone.
pub const PLAN_LEAD: f64 = 2.0;
/// The planning radius is this much over the tightest arc.
pub const PLAN_RADIUS_MARGIN: f64 = 1.05;
/// A written route's tightest sample, as a fraction of the tightest arc, that counts as reached.
pub const PLAN_REACHED_FRACTION: f64 = 0.99;
/// Search step, in metres.
pub const PLAN_STEP_M: f64 = 6.0;
/// Largest heading change of a step, in degrees.
pub const PLAN_MAX_TURN_DEG: f64 = 45.0;
/// Straight at a junction anchor, in metres.
pub const PLAN_JUNCTION_STRAIGHT_M: f64 = 6.25;
/// Road held beside a bridge or tunnel end, in metres.
pub const PLAN_BRIDGE_HOLD_M: f64 = crate::smooth::WIDEN_BRIDGE_HOLD_M;
/// A deck or tunnel this near a fixed anchor belongs to it and is exempt there, in metres.
pub const PLAN_DECK_ANCHOR_M: f64 = 12.0;
/// Furthest a route strays from the original line, in minimum radii.
pub const PLAN_STRAY_MAX: f64 = 3.0;
/// Largest cut of the profile band, in metres.
pub const PLAN_CUT_M: f64 = 6.0;
/// Largest fill of the profile band, in metres.
pub const PLAN_FILL_M: f64 = 6.0;
/// Road height at an anchor must be within this of the terrain, in metres.
pub const PLAN_ANCHOR_TOL_M: f64 = 0.5;
/// Thirds of the gap to the grade ceiling that are tried in turn.
pub const PLAN_GRADE_RELAX_STEPS: usize = 3;
/// A planned leg this much steeper than drawn is counted.
pub const PLAN_STEEPER_MARGIN: f64 = 0.02;
/// Window of the reported steepest drawn grade, in metres.
pub const PLAN_DRAWN_WINDOW_M: f64 = 20.0;
/// Metres of road per metre of climb.
pub const W_SLOPE: f64 = 4.0;
/// Weight of the turn, times `|δ| · R`.
pub const W_CURVE: f64 = 0.1;
/// Metres of road per m² of earthwork.
pub const W_EARTH: f64 = 1.0;
/// Metres of road per m² between route and original line.
pub const W_STRAY: f64 = 0.25;
/// Weight and expanded states of each try of the search.
pub const PLAN_BUDGETS: [(f64, usize); 3] = [(1.0, 10_000), (1.5, 20_000), (3.0, 40_000)];
/// A longer zone is not planned, in metres.
pub const PLAN_MAX_ZONE_M: f64 = 400.0;
/// Exempt part of a road beyond where the original line comes near, in metres.
pub const PLAN_EXEMPT_EXTEND_M: f64 = 10.0;
/// Wall crossings this near an anchor are not counted, in metres.
pub const PLAN_CROSSING_END_M: f64 = 10.0;
/// Most cells of the obstacle raster of a leg; the resolution grows by 1.25 until it fits.
pub const PLAN_RASTER_MAX_CELLS: usize = 4_000_000;
/// Most cells of the cost-to-go field of a leg; the resolution grows by 1.25 until it fits.
pub const PLAN_FIELD_MAX_CELLS: usize = 1_000_000;

/// The constants of road planning as values, so that tests can change them. `Default` is the table.
#[derive(Debug, Clone, PartialEq)]
pub struct RerouteParams {
    /// See [`REROUTE_ENABLED`].
    pub enabled: bool,
    /// See [`CHAMFER_ERROR`].
    pub chamfer_error: f64,
    /// See [`PLAN_LEAD`].
    pub lead: f64,
    /// See [`PLAN_RADIUS_MARGIN`].
    pub radius_margin: f64,
    /// See [`PLAN_REACHED_FRACTION`].
    pub reached_fraction: f64,
    /// See [`PLAN_STEP_M`].
    pub step_m: f64,
    /// See [`PLAN_MAX_TURN_DEG`].
    pub max_turn_deg: f64,
    /// See [`PLAN_JUNCTION_STRAIGHT_M`].
    pub junction_straight_m: f64,
    /// See [`PLAN_BRIDGE_HOLD_M`].
    pub bridge_hold_m: f64,
    /// See [`PLAN_DECK_ANCHOR_M`].
    pub deck_anchor_m: f64,
    /// See [`PLAN_STRAY_MAX`].
    pub stray_max: f64,
    /// See [`PLAN_CUT_M`].
    pub cut_m: f64,
    /// See [`PLAN_FILL_M`].
    pub fill_m: f64,
    /// See [`PLAN_ANCHOR_TOL_M`].
    pub anchor_tol_m: f64,
    /// See [`PLAN_GRADE_RELAX_STEPS`].
    pub grade_relax_steps: usize,
    /// See [`PLAN_STEEPER_MARGIN`].
    pub steeper_margin: f64,
    /// See [`PLAN_DRAWN_WINDOW_M`].
    pub drawn_window_m: f64,
    /// See [`W_SLOPE`].
    pub w_slope: f64,
    /// See [`W_CURVE`].
    pub w_curve: f64,
    /// See [`W_EARTH`].
    pub w_earth: f64,
    /// See [`W_STRAY`].
    pub w_stray: f64,
    /// See [`PLAN_BUDGETS`].
    pub budgets: [(f64, usize); 3],
    /// See [`PLAN_MAX_ZONE_M`].
    pub max_zone_m: f64,
    /// See [`PLAN_EXEMPT_EXTEND_M`].
    pub exempt_extend_m: f64,
    /// See [`PLAN_CROSSING_END_M`].
    pub crossing_end_m: f64,
    /// See [`PLAN_RASTER_MAX_CELLS`].
    pub raster_max_cells: usize,
    /// See [`PLAN_FIELD_MAX_CELLS`].
    pub field_max_cells: usize,
}

impl Default for RerouteParams {
    fn default() -> Self {
        Self {
            enabled: REROUTE_ENABLED,
            chamfer_error: CHAMFER_ERROR,
            lead: PLAN_LEAD,
            radius_margin: PLAN_RADIUS_MARGIN,
            reached_fraction: PLAN_REACHED_FRACTION,
            step_m: PLAN_STEP_M,
            max_turn_deg: PLAN_MAX_TURN_DEG,
            junction_straight_m: PLAN_JUNCTION_STRAIGHT_M,
            bridge_hold_m: PLAN_BRIDGE_HOLD_M,
            deck_anchor_m: PLAN_DECK_ANCHOR_M,
            stray_max: PLAN_STRAY_MAX,
            cut_m: PLAN_CUT_M,
            fill_m: PLAN_FILL_M,
            anchor_tol_m: PLAN_ANCHOR_TOL_M,
            grade_relax_steps: PLAN_GRADE_RELAX_STEPS,
            steeper_margin: PLAN_STEEPER_MARGIN,
            drawn_window_m: PLAN_DRAWN_WINDOW_M,
            w_slope: W_SLOPE,
            w_curve: W_CURVE,
            w_earth: W_EARTH,
            w_stray: W_STRAY,
            budgets: PLAN_BUDGETS,
            max_zone_m: PLAN_MAX_ZONE_M,
            exempt_extend_m: PLAN_EXEMPT_EXTEND_M,
            crossing_end_m: PLAN_CROSSING_END_M,
            raster_max_cells: PLAN_RASTER_MAX_CELLS,
            field_max_cells: PLAN_FIELD_MAX_CELLS,
        }
    }
}

/// What fixes an end of a zone: a free point on the line, or one of the places the road may not move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorKind {
    /// A point on the drawn line, with the line's heading there.
    Line,
    /// A junction, by node id: its position and the direction of the arm are kept.
    Junction(i64),
    /// A dead end, by node id: its position is kept, the direction is free.
    DeadEnd(i64),
    /// The end of a bridge's held approach.
    BridgeHold,
    /// The end of a tunnel's held approach.
    TunnelHold,
    /// The node where a ring chain is cut, by node id: its position and the heading are kept.
    RingCut(i64),
}

/// Why a zone kept its drawn line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeptReason {
    /// No route was found and the estimate allows no proof that there is none.
    NoRoute,
    /// No route was found while the water was in the way.
    BlockedByWater,
    /// No route was found while another road was in the way.
    BlockedByRoad,
    /// A fixed pose (a junction, a dead end, a bridge or tunnel end, a ring cut) left no room.
    Held,
}

impl KeptReason {
    /// The name of the reason in the report (`no_route`, `blocked_by_water`, `blocked_by_road`, `held`).
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            KeptReason::NoRoute => "no_route",
            KeptReason::BlockedByWater => "blocked_by_water",
            KeptReason::BlockedByRoad => "blocked_by_road",
            KeptReason::Held => "held",
        }
    }
}

/// What became of a zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoneOutcome {
    /// Every leg of the zone was planned and written into the graph.
    Planned,
    /// At least one leg of the zone kept its drawn line, with the reason and a detail for the report
    /// (`anchors_close`, `crosses_deck`, `tight_run_held`, ...). The zone's planned legs are still written into
    /// the graph; the kept legs keep their drawn line.
    Kept {
        /// Why it kept its line.
        reason: KeptReason,
        /// What exactly stopped it, for the report.
        detail: String,
    },
}

/// The later task a kept bend is a candidate for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Candidate {
    /// A bend that cannot be built as a curve: a concrete plate joins its legs.
    Plate,
    /// A bend blocked by water: a curved bridge carries the road over it.
    CurvedBridge,
}

impl Candidate {
    /// The name of the candidate in the report (`plate`, `curved_bridge`).
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Candidate::Plate => "plate",
            Candidate::CurvedBridge => "curved_bridge",
        }
    }
}

/// One zone of tight road and what road planning did with it.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedZone {
    /// `<first stretch id of the chain>#<n>`, with `n` counting the zones of the chain.
    pub id: String,
    /// Where the zone is tightest, in map metres.
    pub at: (f64, f64),
    /// The stretches the zone covers, in chain order, without repeats.
    pub stretches: Vec<String>,
    /// The number of legs the zone was split into by held parts.
    pub legs: usize,
    /// What fixes the zone's two ends.
    pub anchors: (AnchorKind, AnchorKind),
    /// The junction the bend passes, if an end is a junction.
    pub junction: Option<i64>,
    /// The other zone of the same bend across a junction, if it has one.
    pub partner: Option<String>,
    /// What became of the zone.
    pub outcome: ZoneOutcome,
    /// The smallest radius of the drawn line in the tight run, in metres.
    pub radius_before_m: f64,
    /// The minimum radius the road pieces ask for there, in metres.
    pub radius_wanted_m: f64,
    /// The smallest radius of the planned route on its 2 m samples, in metres (the drawn radius if the zone kept
    /// its line).
    pub radius_after_m: f64,
    /// The length of the drawn line from the zone's first to its last sample, in metres.
    pub length_before_m: f64,
    /// The same after planning: the routes and the kept legs, in metres.
    pub length_after_m: f64,
    /// The largest true distance of a planned route from the drawn line, in metres: the larger measure of the
    /// search's `max_move_m`, not the raster's stored stray distance (which is 0 across the cells the line
    /// touches). 0 for a zone that kept its line.
    pub max_stray_m: f64,
    /// The drawn grade between the zone's ends, mean over the line, in metres of rise per metre.
    pub drawn_grade: f64,
    /// The steepest drawn grade over a window of [`PLAN_DRAWN_WINDOW_M`], reported only.
    pub drawn_grade_window: f64,
    /// The steepest grade of the planned road profile (`Profile::max_grade`), in metres of rise per metre;
    /// 0 if the zone kept its line.
    pub planned_grade: f64,
    /// The grade limit the route was planned at (the drawn limit if the zone kept its line).
    pub grade_limit: f64,
    /// The relaxation steps that were needed (or tried, for a kept zone).
    pub relax_steps: usize,
    /// The climb of the planned profiles, in metres.
    pub climb_m: f64,
    /// The real earthwork of the planned profiles, in m².
    pub earthwork_m2: f64,
    /// The deepest cut of the planned profiles, in metres.
    pub cut_max_m: f64,
    /// The highest fill of the planned profiles, in metres.
    pub fill_max_m: f64,
    /// The states the searches of the zone expanded.
    pub states: usize,
    /// The heuristic weight of the try that found the route (`None` for a kept zone).
    pub weight: Option<f64>,
    /// Whether a search ran out of its budget. `false` is not a proof that there is no route: it is also what a
    /// search whose open set ran empty reports.
    pub budget_exhausted: bool,
    /// Whether the zone's drawn line itself passes the segment test, its clearances included.
    pub original_feasible: bool,
    /// The candidate the bend is for, where it is kept.
    pub candidate: Option<Candidate>,
}

/// What road planning did to the whole graph.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RerouteReport {
    /// The zones, in chain order and then along the chain.
    pub zones: Vec<PlannedZone>,
    /// The stretches and zones that were not planned at all, with the reason: `(subject, reason)`.
    pub skipped: Vec<(String, String)>,
    /// Road length tighter than its family's minimum radius, before, in metres.
    pub tight_before_m: f64,
    /// The same after, in metres.
    pub tight_after_m: f64,
    /// The largest true distance of a planned route from the drawn line, in metres.
    pub max_stray_m: f64,
    /// The number of planned legs steeper than drawn by more than [`PLAN_STEEPER_MARGIN`].
    pub steeper_than_drawn: usize,
    /// The routes reverted because they came too near an earlier accepted route (the cross-check is a later
    /// task; it is 0 here).
    pub reverted: usize,
    /// The states every search of every leg expanded.
    pub states: usize,
}

/// Re-routes the tight bends of `graph`: finds the zones of the chains whose drawn line is tighter than the road
/// pieces, splits them into legs between anchors, searches a route for each leg and writes the routes back as the
/// stretches' points. Stretch ids, node ids and every attribute stay; nodes inside a planned leg move to the route.
///
/// `min_radius` gives the tightest arc a stretch's family can build (a value that is not finite or not positive
/// means the family is never planned), `ground` the terrain, `in_water` the water test and `streams` the stream
/// lines. The parameters are [`RerouteParams`].
///
/// A leg that cannot be planned keeps its drawn line; a zone whose legs are not all planned keeps the drawn line
/// of its kept legs. Nothing outside a zone changes. The result is deterministic: the zones are found in chain
/// order, the legs are searched in parallel with `rayon` and collected in that order, and every comparison of
/// floats is made with `total_cmp`.
///
/// In this task a failed leg is [`KeptReason::NoRoute`] or [`KeptReason::Held`]; the diagnostic searches for
/// `blocked_by_water` and `blocked_by_road`, and the check of routes against each other, are a later task.
#[must_use]
pub fn reroute_graph(
    graph: &RoadGraph,
    ground: &Heightmap,
    min_radius: &dyn Fn(&Stretch) -> f64,
    in_water: WaterTest<'_>,
    streams: &[Vec<(f64, f64)>],
    params: &RerouteParams,
) -> (RoadGraph, RerouteReport) {
    plan_graph_impl(graph, ground, min_radius, in_water, streams, params)
}

// ---------------------------------------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------------------------------------

/// Tolerance of the positions and arcs, in metres.
const ARC_EPS: f64 = 1e-9;

/// A count as `f64` (exact for the counts the stage uses).
fn fl(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// Whether a minimum radius can be planned against.
fn usable(minimum: f64) -> bool {
    minimum.is_finite() && minimum > 0.0
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn finite(p: (f64, f64)) -> bool {
    p.0.is_finite() && p.1.is_finite()
}

/// A stretch's line is sound when it is finite and has at least two distinct points.
fn sound(s: &Stretch) -> bool {
    s.points.iter().all(|p| finite(*p)) && s.points.windows(2).any(|w| w.first() != w.get(1))
}

/// The length of a polyline.
fn poly_length(points: &[(f64, f64)]) -> f64 {
    points
        .windows(2)
        .filter_map(|w| match w {
            [a, b] => Some(dist(*a, *b)),
            _ => None,
        })
        .sum()
}

/// The distance of a point from a polyline (infinite for an empty line).
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
            .filter_map(|w| match w {
                [a, b] => Some(one(*a, *b)),
                _ => None,
            })
            .fold(f64::INFINITY, f64::min),
    }
}

/// Whether any segment of `line` has a bounding box that overlaps `lo`..`hi`.
fn line_near(line: &[(f64, f64)], lo: (f64, f64), hi: (f64, f64)) -> bool {
    if line
        .iter()
        .any(|p| p.0 >= lo.0 && p.1 >= lo.1 && p.0 <= hi.0 && p.1 <= hi.1)
    {
        return true;
    }
    line.windows(2).any(|w| match w {
        [a, b] => {
            a.0.min(b.0) <= hi.0
                && a.0.max(b.0) >= lo.0
                && a.1.min(b.1) <= hi.1
                && a.1.max(b.1) >= lo.1
        }
        _ => false,
    })
}

/// The cumulative length at every point of a polyline.
fn cumulative(points: &[(f64, f64)]) -> Vec<f64> {
    let mut out = Vec::with_capacity(points.len());
    out.push(0.0);
    let mut at = 0.0;
    for w in points.windows(2) {
        if let [a, b] = w {
            at += dist(*a, *b);
        }
        out.push(at);
    }
    out
}

/// The heading of a line at sample `i`, over one step on each side (one-sided at the ends).
fn heading_at(points: &[(f64, f64)], i: usize) -> f64 {
    let last = points.len().saturating_sub(1);
    let a = points.get(i.saturating_sub(1)).copied().unwrap_or_default();
    let b = points.get((i + 1).min(last)).copied().unwrap_or_default();
    (b.1 - a.1).atan2(b.0 - a.0)
}

/// The three-point radius at sample `i` of a chain line; at the cut of a ring the line is closed, so its
/// neighbours are the last sample before the cut and the first one after it.
fn radius_at_index(chain: &ChainLine, i: usize) -> f64 {
    if chain.ring && (i == 0 || i + 1 == chain.pts.len()) {
        let n = chain.pts.len();
        if let (Some(&a), Some(&b), Some(&c)) = (
            n.checked_sub(2).and_then(|k| chain.pts.get(k)),
            chain.pts.first(),
            chain.pts.get(1),
        ) {
            return radius_at(a, b, c);
        }
    }
    match (
        i.checked_sub(1).and_then(|k| chain.pts.get(k)),
        chain.pts.get(i),
        chain.pts.get(i + 1),
    ) {
        (Some(&a), Some(&b), Some(&c)) => radius_at(a, b, c),
        _ => f64::INFINITY,
    }
}

/// Whether the drawn line is tighter than its minimum at the cut of a ring (the closed line's bend).
fn cut_tight(chain: &ChainLine) -> bool {
    let n = chain.pts.len();
    if n < 3 {
        return false;
    }
    let minimum = chain.minr.first().copied().unwrap_or(f64::INFINITY);
    match (
        n.checked_sub(2).and_then(|k| chain.pts.get(k)),
        chain.pts.first(),
        chain.pts.get(1),
    ) {
        (Some(&a), Some(&b), Some(&c)) => usable(minimum) && radius_at(a, b, c) < minimum,
        _ => false,
    }
}

/// The last sample whose arc is at or before `a`.
fn index_of_arc(arc: &[f64], a: f64) -> usize {
    arc.partition_point(|&x| x <= a).saturating_sub(1)
}

/// What fixes a chain end or a leg end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndKind {
    /// A junction, by node id.
    Junction(i64),
    /// A dead end, by node id.
    DeadEnd(i64),
    /// The cut of a ring, by node id.
    RingCut(i64),
    /// The end of a tunnel's held approach.
    TunnelHold,
    /// The end of a bridge's held approach.
    BridgeHold,
    /// A sharper two-stretch node: a free point with the line's heading.
    Sharp,
    /// A free point on the line.
    Line,
}

impl EndKind {
    fn anchor_kind(self) -> AnchorKind {
        match self {
            EndKind::Junction(node) => AnchorKind::Junction(node),
            EndKind::DeadEnd(node) => AnchorKind::DeadEnd(node),
            EndKind::RingCut(node) => AnchorKind::RingCut(node),
            EndKind::TunnelHold => AnchorKind::TunnelHold,
            EndKind::BridgeHold => AnchorKind::BridgeHold,
            EndKind::Sharp | EndKind::Line => AnchorKind::Line,
        }
    }

    fn fixed(self) -> bool {
        !matches!(self, EndKind::Sharp | EndKind::Line)
    }
}

/// A part of a chain where the road is held: a bridge with its approaches, or a tunnel's approach.
#[derive(Debug, Clone, Copy)]
struct HeldPart {
    from: f64,
    to: f64,
    tunnel: bool,
}

/// One chain of the graph, its samples and what the stage needs of every sample.
struct ChainLine {
    /// The id of the chain: the id of its first stretch.
    id: String,
    /// The stretches, in the chain's direction.
    links: Vec<Link>,
    /// The samples, every [`SAMPLE_M`] along the chain.
    pts: Vec<(f64, f64)>,
    /// The distance along the chain of every sample.
    arc: Vec<f64>,
    /// The last sample of every link.
    bounds: Vec<usize>,
    /// The stretch every sample belongs to.
    stretch_of: Vec<usize>,
    /// The minimum radius every sample is held to (infinite: never planned).
    minr: Vec<f64>,
    /// Half the width of the road at every sample.
    half: Vec<f64>,
    /// Whether the sample lies on a roundabout.
    roundabout: Vec<bool>,
    /// The parts of the chain the road may not move in.
    held: Vec<HeldPart>,
    /// What fixes the two ends of the chain.
    start_kind: EndKind,
    end_kind: EndKind,
    /// Whether the chain is a ring, cut at its first and last sample.
    ring: bool,
    /// The length of the chain, in metres.
    total: f64,
}

/// What every chain of one graph needs of the graph: the arms at each node and the tunnel flags, built once.
struct GraphContext {
    arms_at: BTreeMap<i64, Vec<usize>>,
    tunnel: Vec<bool>,
}

impl GraphContext {
    fn new(graph: &RoadGraph) -> GraphContext {
        let mut arms_at: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
        let mut tunnel = Vec::with_capacity(graph.stretches.len());
        for (i, s) in graph.stretches.iter().enumerate() {
            arms_at.entry(s.start).or_default().push(i);
            arms_at.entry(s.end).or_default().push(i);
            tunnel.push(s.tunnel);
        }
        GraphContext { arms_at, tunnel }
    }

    /// The number of stretch arms at a node (a loop counts twice).
    fn degree(&self, node: i64) -> usize {
        self.arms_at.get(&node).map_or(1, Vec::len)
    }

    /// Whether a stretch at the node other than `stretch` has a different tunnel flag.
    fn other_tunnel(&self, node: i64, stretch: usize) -> bool {
        let current = self.tunnel.get(stretch).copied().unwrap_or(false);
        self.arms_at.get(&node).is_some_and(|arms| {
            arms.iter()
                .any(|&i| i != stretch && self.tunnel.get(i).copied().unwrap_or(false) != current)
        })
    }
}

/// The node a link enters and the node it leaves.
fn entry_node(graph: &RoadGraph, link: &Link) -> i64 {
    graph
        .stretches
        .get(link.stretch)
        .map_or(0, |s| if link.reversed { s.end } else { s.start })
}

fn exit_node(graph: &RoadGraph, link: &Link) -> i64 {
    graph
        .stretches
        .get(link.stretch)
        .map_or(0, |s| if link.reversed { s.start } else { s.end })
}

/// The line of a chain, or `None` with the reason in `skipped`. Every sample carries the **true** arc of the
/// drawn line at that point, not the chord length of the resampled points: the write-back places the original
/// vertices by the same measure, so a node inside a leg is one point and no vertex is written after the route.
#[expect(
    clippy::too_many_lines,
    reason = "the chain and its per-sample tables are built in one pass over the same state"
)]
fn build_chain_line(
    graph: &RoadGraph,
    ctx: &GraphContext,
    plan: &ChainPlan,
    min_of: &[f64],
    half_of: &[f64],
    params: &RerouteParams,
    skipped: &mut Vec<(String, String)>,
) -> Option<ChainLine> {
    let first = *plan.links.first()?;
    let id = graph.stretches.get(first.stretch)?.id.clone();
    for link in &plan.links {
        let s = graph.stretches.get(link.stretch)?;
        if s.tunnel {
            skipped.push((s.id.clone(), "tunnel".to_string()));
            return None;
        }
        if !sound(s) {
            skipped.push((s.id.clone(), "not finite".to_string()));
            return None;
        }
    }
    let estimate: f64 = plan
        .links
        .iter()
        .filter_map(|l| graph.stretches.get(l.stretch))
        .map(|s| poly_length(&s.points))
        .sum();
    if !estimate.is_finite() || estimate / SAMPLE_M > fl(crate::smooth::MAX_CHAIN_SAMPLES) {
        skipped.push((id.clone(), "too long".to_string()));
        return None;
    }
    let mut pts: Vec<(f64, f64)> = Vec::new();
    let mut arc: Vec<f64> = Vec::new();
    let mut bounds: Vec<usize> = Vec::new();
    let mut total = 0.0_f64;
    for link in &plan.links {
        let s = graph.stretches.get(link.stretch)?;
        let mut line = s.points.clone();
        if link.reversed {
            line.reverse();
        }
        let sampled = resample(&line, SAMPLE_M);
        let segments = sampled.len().saturating_sub(1);
        let link_len = poly_length(&line);
        if pts.is_empty()
            && let Some(&p) = sampled.first()
        {
            pts.push(p);
            arc.push(0.0);
        }
        for k in 1..=segments {
            if let Some(&p) = sampled.get(k) {
                pts.push(p);
            }
            let at = if segments > 0 {
                total + link_len * fl(k) / fl(segments)
            } else {
                total
            };
            arc.push(at);
        }
        total += link_len;
        bounds.push(pts.len().saturating_sub(1));
    }
    let count = pts.len();
    let mut stretch_of = vec![0_usize; count];
    let mut minr = vec![f64::INFINITY; count];
    let mut half = vec![0.0_f64; count];
    let mut roundabout = vec![false; count];
    let mut held: Vec<HeldPart> = Vec::new();
    let mut first_sample = 0_usize;
    for (link, &last) in plan.links.iter().zip(&bounds) {
        let s = graph.stretches.get(link.stretch)?;
        let minimum = min_of
            .get(link.stretch)
            .copied()
            .filter(|v| usable(*v))
            .unwrap_or(f64::INFINITY);
        let half_width = half_of.get(link.stretch).copied().unwrap_or(0.0);
        for k in first_sample..=last {
            if let Some(v) = stretch_of.get_mut(k) {
                *v = link.stretch;
            }
            if let Some(v) = minr.get_mut(k) {
                *v = minimum;
            }
            if let Some(v) = half.get_mut(k) {
                *v = half_width;
            }
            if let Some(v) = roundabout.get_mut(k) {
                *v = s.roundabout;
            }
        }
        if s.bridge {
            let from = arc.get(first_sample).copied().unwrap_or(0.0) - params.bridge_hold_m;
            let to = arc.get(last).copied().unwrap_or(0.0) + params.bridge_hold_m;
            held.push(HeldPart {
                from,
                to,
                tunnel: false,
            });
        }
        first_sample = last;
    }
    let start_node = entry_node(graph, &first);
    let end_node = exit_node(graph, plan.links.last()?);
    let ring = start_node == end_node;
    let kind_at_node = |node: i64, link: &Link| -> EndKind {
        if ring {
            return EndKind::RingCut(node);
        }
        match ctx.degree(node) {
            1 => EndKind::DeadEnd(node),
            n if n >= 3 => EndKind::Junction(node),
            _ => {
                if ctx.other_tunnel(node, link.stretch) {
                    EndKind::TunnelHold
                } else {
                    EndKind::Sharp
                }
            }
        }
    };
    let start_kind = kind_at_node(start_node, &first);
    let end_kind = kind_at_node(end_node, plan.links.last()?);
    let approach_m = params.bridge_hold_m.max(0.0);
    if start_kind == EndKind::TunnelHold {
        held.push(HeldPart {
            from: 0.0,
            to: approach_m.min(total),
            tunnel: true,
        });
    }
    if end_kind == EndKind::TunnelHold {
        held.push(HeldPart {
            from: (total - approach_m).max(0.0),
            to: total,
            tunnel: true,
        });
    }
    Some(ChainLine {
        id,
        links: plan.links.clone(),
        pts,
        arc,
        bounds,
        stretch_of,
        minr,
        half,
        roundabout,
        held,
        start_kind,
        end_kind,
        ring,
        total,
    })
}

/// Whether a sample of the chain is inside a held part.
fn in_held(chain: &ChainLine, k: usize) -> bool {
    let a = chain.arc.get(k).copied().unwrap_or(f64::NAN);
    chain.held.iter().any(|h| a > h.from && a < h.to)
}

/// The hold a sample just inside a held part belongs to.
fn held_kind(chain: &ChainLine, k: usize) -> EndKind {
    let a = chain.arc.get(k).copied().unwrap_or(f64::NAN);
    if chain
        .held
        .iter()
        .any(|h| h.tunnel && a > h.from && a < h.to)
    {
        EndKind::TunnelHold
    } else {
        EndKind::BridgeHold
    }
}

/// What fixes a leg end at sample `idx`.
fn leg_end_kind(chain: &ChainLine, idx: usize, at_start: bool) -> EndKind {
    let last = chain.pts.len().saturating_sub(1);
    if idx == 0 {
        return chain.start_kind;
    }
    if idx >= last {
        return chain.end_kind;
    }
    if in_held(chain, idx) {
        return held_kind(chain, idx);
    }
    if at_start {
        if idx > 0 && in_held(chain, idx - 1) {
            return held_kind(chain, idx - 1);
        }
    } else if in_held(chain, idx + 1) {
        return held_kind(chain, idx + 1);
    }
    EndKind::Line
}

/// A zone before its legs are searched.
struct ZoneWork {
    chain: usize,
    id: String,
    s: usize,
    e: usize,
    tightest: usize,
    /// The tight samples of the zone.
    tight: Vec<usize>,
    /// The zone cannot be searched at all, for this reason.
    fixed: Option<(KeptReason, String)>,
    legs: Vec<LegSeed>,
}

/// One leg of a zone: its samples and the kinds of its two anchors.
struct LegSeed {
    a: usize,
    b: usize,
    start: EndKind,
    end: EndKind,
}

/// A zone under construction: its sample range and the tight runs that were merged into it.
type MergedZone = (usize, usize, Vec<(usize, usize)>);

/// The tight runs of a chain, each grown by the lead and merged in sample space, so that two grown zones that
/// would share a sample are one zone.
fn find_zones(chain: &ChainLine, lead: f64) -> Vec<(usize, usize, Vec<usize>, usize)> {
    let mut runs = tight_runs(&chain.pts, &chain.minr);
    // a ring is closed: the bend at its cut is between the last segment and the first
    if chain.ring && cut_tight(chain) {
        runs.insert(0, (0, 0));
    }
    let total = chain.total;
    let last_index = chain.pts.len().saturating_sub(1);
    let mut merged: Vec<MergedZone> = Vec::new();
    for (a, b) in runs {
        let (Some(&ma), Some(&mb)) = (chain.minr.get(a), chain.minr.get(b)) else {
            continue;
        };
        if !usable(ma) || !usable(mb) {
            continue;
        }
        let za = (chain.arc.get(a).copied().unwrap_or(0.0) - lead * ma).max(0.0);
        let zb = (chain.arc.get(b).copied().unwrap_or(0.0) + lead * mb).min(total);
        let s = index_of_arc(&chain.arc, za);
        let mut e = index_of_arc(&chain.arc, zb);
        if chain.arc.get(e).copied().unwrap_or(0.0) < zb - ARC_EPS {
            e += 1;
        }
        let e = e.min(last_index);
        match merged.last_mut() {
            // the rounding can make two grown zones share a sample: then they are one zone
            Some(last) if s <= last.1 => {
                last.1 = last.1.max(e);
                last.2.push((a, b));
            }
            _ => merged.push((s, e, vec![(a, b)])),
        }
    }
    merged
        .into_iter()
        .map(|(s, e, runs)| {
            let tight: Vec<usize> = runs
                .into_iter()
                .flat_map(|(a, b)| a..=b)
                .filter(|&k| k <= last_index)
                .collect();
            let tightest = tight
                .iter()
                .copied()
                .min_by(|&i, &j| {
                    radius_at_index(chain, i)
                        .total_cmp(&radius_at_index(chain, j))
                        .then_with(|| i.cmp(&j))
                })
                .unwrap_or(s);
            (s, e, tight, tightest)
        })
        .collect()
}

#[expect(
    clippy::many_single_char_names,
    reason = "the sample indices are `s`, `e` and `k`, as the design names them"
)]
fn make_zone_work(
    chain_index: usize,
    chain: &ChainLine,
    (s, e, tight, tightest): (usize, usize, Vec<usize>, usize),
    ordinal: usize,
    params: &RerouteParams,
    skipped: &mut Vec<(String, String)>,
) -> ZoneWork {
    let id = format!("{}#{ordinal}", chain.id);
    let wanted = chain.minr.get(tightest).copied().unwrap_or(f64::INFINITY);
    let mut fixed = None;
    if !usable(wanted) {
        fixed = Some((KeptReason::Held, "no_arc".to_string()));
    }
    if fixed.is_none()
        && tight
            .iter()
            .any(|&k| chain.roundabout.get(k).copied().unwrap_or(false))
    {
        fixed = Some((KeptReason::Held, "roundabout".to_string()));
    }
    let length =
        chain.arc.get(e).copied().unwrap_or(0.0) - chain.arc.get(s).copied().unwrap_or(0.0);
    if fixed.is_none() && chain.ring && tight.contains(&0) {
        // the bend is at the fixed ring cut: the route cannot change the angle there
        fixed = Some((KeptReason::Held, "ring_cut".to_string()));
    }
    if fixed.is_none() && length > params.max_zone_m {
        fixed = Some((KeptReason::NoRoute, "too_long".to_string()));
    }
    let mut legs = Vec::new();
    if fixed.is_none() {
        let mut k = s;
        while k <= e {
            if in_held(chain, k) {
                k += 1;
                continue;
            }
            let a = k;
            while k < e && !in_held(chain, k + 1) {
                k += 1;
            }
            let b = k;
            if tight.iter().any(|&t| t >= a && t <= b) {
                legs.push(LegSeed {
                    a,
                    b,
                    start: leg_end_kind(chain, a, true),
                    end: leg_end_kind(chain, b, false),
                });
            }
            k += 1;
        }
        if legs.is_empty() {
            fixed = Some((KeptReason::Held, "tight_run_held".to_string()));
        }
    }
    if let Some((_, detail)) = &fixed {
        skipped.push((id.clone(), detail.clone()));
    }
    ZoneWork {
        chain: chain_index,
        id,
        s,
        e,
        tightest,
        tight,
        fixed,
        legs,
    }
}

/// One leg to search.
struct LegJob {
    zone: usize,
    chain: usize,
    a: usize,
    b: usize,
    start: EndKind,
    end: EndKind,
    tight: Vec<usize>,
}

/// What searching one leg gave.
struct LegPlan {
    a: usize,
    b: usize,
    outcome: LegOutcomeKind,
    start: AnchorKind,
    end: AnchorKind,
    states: usize,
    budget_exhausted: bool,
    weight: Option<f64>,
    relax_steps: usize,
    grade_limit: f64,
    drawn_grade: f64,
    planned_grade: f64,
    original_feasible: bool,
    route: Option<Route>,
}

#[derive(Debug, Clone)]
enum LegOutcomeKind {
    Planned,
    Kept { reason: KeptReason, detail: String },
}

/// The anchor of a leg at sample `idx`.
fn leg_anchor(
    chain: &ChainLine,
    idx: usize,
    kind: EndKind,
    at_start: bool,
    params: &RerouteParams,
) -> Anchor {
    let at = chain.pts.get(idx).copied().unwrap_or_default();
    let heading = match kind {
        EndKind::DeadEnd(_) => None,
        EndKind::Junction(_) => {
            let (a, b) = if at_start {
                (idx, idx + 1)
            } else {
                (idx.saturating_sub(1), idx)
            };
            match (chain.pts.get(a), chain.pts.get(b)) {
                (Some(&a), Some(&b)) => Some((b.1 - a.1).atan2(b.0 - a.0)),
                _ => Some(heading_at(&chain.pts, idx)),
            }
        }
        _ => Some(heading_at(&chain.pts, idx)),
    };
    let straight_m = if matches!(kind, EndKind::Junction(_)) {
        params.junction_straight_m
    } else {
        0.0
    };
    Anchor {
        at,
        heading,
        straight_m,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the fields of the result are passed one by one, each with its own meaning"
)]
fn kept_leg(
    job: &LegJob,
    reason: KeptReason,
    detail: &str,
    states: usize,
    budget_exhausted: bool,
    grade_limit: f64,
    drawn_grade: f64,
    original_feasible: bool,
) -> LegPlan {
    LegPlan {
        a: job.a,
        b: job.b,
        outcome: LegOutcomeKind::Kept {
            reason,
            detail: detail.to_string(),
        },
        start: job.start.anchor_kind(),
        end: job.end.anchor_kind(),
        states,
        budget_exhausted,
        weight: None,
        relax_steps: 0,
        grade_limit,
        drawn_grade,
        planned_grade: 0.0,
        original_feasible,
        route: None,
    }
}

/// A leg the stage did not search: its drawn grade and the limit it would have used.
fn held_leg(
    job: &LegJob,
    reason: KeptReason,
    detail: &str,
    grade_limit: f64,
    drawn_grade: f64,
) -> LegPlan {
    kept_leg(
        job,
        reason,
        detail,
        0,
        false,
        grade_limit,
        drawn_grade,
        true,
    )
}

/// The distinct families of the samples of a leg.
fn leg_families(graph: &RoadGraph, chain: &ChainLine, a: usize, b: usize) -> Vec<Family> {
    let mut set: BTreeSet<Family> = BTreeSet::new();
    for k in a..=b {
        if let Some(&stretch) = chain.stretch_of.get(k)
            && let Some(s) = graph.stretches.get(stretch)
        {
            set.insert(s.family);
        }
    }
    set.into_iter().collect()
}

/// Searches one leg: its obstacles, walls, field, band and limits, then the arc search.
#[expect(
    clippy::too_many_lines,
    reason = "the scenario of one leg is built in one place, so that no rule is applied twice"
)]
fn plan_leg(
    job: &LegJob,
    chain: &ChainLine,
    graph: &RoadGraph,
    ground: &Heightmap,
    in_water: WaterTest<'_>,
    streams: &[Vec<(f64, f64)>],
    params: &RerouteParams,
) -> LegPlan {
    let Some(orig) = chain.pts.get(job.a..=job.b) else {
        return held_leg(job, KeptReason::Held, "no_line", 0.0, 0.0);
    };
    let cell = f64::from(ground.cell_m);
    let ground_fn = |p: (f64, f64)| ground_at(ground, p);
    let start_anchor = leg_anchor(chain, job.a, job.start, true, params);
    let end_anchor = leg_anchor(chain, job.b, job.end, false, params);
    let anchors = [start_anchor.at, end_anchor.at];
    let end_m = params.crossing_end_m;
    let drawn = drawn_grade(orig, &ground_fn, params.drawn_window_m);
    let (drawn_mean, _) = drawn.map_or((0.0, 0.0), |d| (d.mean, d.window));
    let half_width = (job.a..=job.b)
        .filter_map(|k| chain.half.get(k).copied())
        .fold(0.0_f64, f64::max);
    let reach_own = core_reach_m(half_width, cell);
    let clearance = reach_own + cell;
    let clear = Clearance::new(reach_own, clearance).unwrap_or(Clearance::NONE);
    let wanted = (job.a..=job.b)
        .filter_map(|k| chain.minr.get(k).copied())
        .filter(|v| usable(*v))
        .fold(f64::NEG_INFINITY, f64::max);
    if !usable(wanted) {
        return held_leg(job, KeptReason::Held, "no_arc", 0.0, drawn_mean);
    }
    let nearest = dist(start_anchor.at, end_anchor.at);
    if orig.len() < 2
        || nearest < 2.0 * SAMPLE_M
        || (matches!(job.start, EndKind::Junction(_))
            && matches!(job.end, EndKind::Junction(_))
            && nearest < 2.0 * params.junction_straight_m)
    {
        return held_leg(job, KeptReason::Held, "anchors_close", 0.0, drawn_mean);
    }
    let families = leg_families(graph, chain, job.a, job.b);
    let Some(drawn) = drawn else {
        return held_leg(job, KeptReason::NoRoute, "no_ground", 0.0, drawn_mean);
    };
    let Some((limit, ceiling)) = leg_limit(&families, &drawn) else {
        return held_leg(job, KeptReason::Held, "no_family", 0.0, drawn_mean);
    };
    let Some(limits) = relaxed_limits(limit, ceiling, params.grade_relax_steps) else {
        return held_leg(job, KeptReason::Held, "no_grade_limit", limit, drawn_mean);
    };
    let radius = wanted * params.radius_margin;
    if !radius.is_finite() || radius <= 0.0 || radius > 1000.0 {
        return held_leg(job, KeptReason::Held, "bad_radius", limit, drawn_mean);
    }
    let stray_cap = params.stray_max * wanted;
    let self_along = PI * radius + 2.0 * clearance;
    let self_clear = self_clearance(orig, self_along, clearance, 2.0 * half_width);

    let (mut low, mut high) = (
        (f64::INFINITY, f64::INFINITY),
        (f64::NEG_INFINITY, f64::NEG_INFINITY),
    );
    for p in orig {
        low = (low.0.min(p.0), low.1.min(p.1));
        high = (high.0.max(p.0), high.1.max(p.1));
    }
    let pad = stray_cap + clearance + 2.0 * cell;
    let box_min = (low.0 - pad, low.1 - pad);
    let box_max = (high.0 + pad, high.1 + pad);
    let extent = f64::from(ground.size.saturating_sub(1)) * cell;
    let near_edge = orig.iter().any(|p| {
        p.0 < reach_own || p.1 < reach_own || p.0 > extent - reach_own || p.1 > extent - reach_own
    });
    let edge_margin = if near_edge { 0.0 } else { reach_own };
    let mut obstacles = Obstacles::new(
        box_min,
        box_max,
        cell,
        extent,
        edge_margin,
        params.raster_max_cells,
    );
    obstacles.stamp_water(in_water);
    let own: BTreeSet<usize> = chain.links.iter().map(|l| l.stretch).collect();
    let mut fixed_anchors: Vec<(f64, f64)> = [job.start, job.end]
        .iter()
        .zip([start_anchor.at, end_anchor.at])
        .filter_map(|(&kind, at)| kind.fixed().then_some(at))
        .collect();
    // A bridge or tunnel hold ends at the chain's end node; a deck or tunnel there is part of the anchor, not an
    // obstacle beside it (the tunnel of a TunnelHold is a chain of its own and would otherwise block the start).
    if matches!(job.start, EndKind::TunnelHold | EndKind::BridgeHold)
        && let Some(&p) = chain.pts.first()
    {
        fixed_anchors.push(p);
    }
    if matches!(job.end, EndKind::TunnelHold | EndKind::BridgeHold)
        && let Some(&p) = chain.pts.last()
    {
        fixed_anchors.push(p);
    }
    let mut walls: Vec<Wall> = Vec::new();
    let mut deck_crossed = false;
    let filter = (box_min.0 - clearance, box_min.1 - clearance);
    let filter_hi = (box_max.0 + clearance, box_max.1 + clearance);
    for (i, s) in graph.stretches.iter().enumerate() {
        if own.contains(&i) || !line_near(&s.points, filter, filter_hi) {
            continue;
        }
        let reach_other = core_reach_m(s.family.width_m() / 2.0, cell);
        let deck = s.bridge || s.tunnel;
        let near_fixed = fixed_anchors
            .iter()
            .any(|p| poly_dist(*p, &s.points) <= params.deck_anchor_m);
        let cross = crossings_away(orig, &s.points, anchors, end_m);
        if deck && !near_fixed {
            obstacles.stamp_road(&s.points, reach_other, &[]);
            if cross > 0 {
                deck_crossed = true;
            }
        } else {
            let near = reach_other + obstacles.threshold(clearance);
            let flags = exempt_samples(&s.points, orig, near, params.exempt_extend_m);
            obstacles.stamp_road(&s.points, reach_other, &flags);
            if flags.iter().any(|&f| f) {
                walls.push(Wall {
                    line: s.points.clone(),
                    allowed: cross,
                    kind: WallKind::Road,
                    anchors,
                    end_m,
                });
            }
        }
    }
    // the road the leg continues into: the first `PLAN_SELF_FREE` along the chain from each anchor is not an
    // obstacle. The distance is measured along the chain (the arc of its 2 m samples), not as the crow flies.
    let self_free = obstacles.threshold(clearance) + reach_own;
    if let Some(before) = chain.pts.get(..=job.a)
        && before.len() >= 2
    {
        let flags = self_free_flags(before, true, self_free);
        obstacles.stamp_road(before, reach_own, &flags);
    }
    if let Some(after) = chain.pts.get(job.b..)
        && after.len() >= 2
    {
        let flags = self_free_flags(after, false, self_free);
        obstacles.stamp_road(after, reach_own, &flags);
    }
    for stream in streams {
        if !line_near(stream, filter, filter_hi) {
            continue;
        }
        let allowed = crossings_away(orig, stream, anchors, end_m);
        walls.push(Wall {
            line: stream.clone(),
            allowed,
            kind: WallKind::Stream,
            anchors,
            end_m,
        });
    }
    obstacles.set_original(orig);
    obstacles.finish();
    let original_feasible = orig
        .windows(2)
        .all(|w| matches!(w, [a, b] if obstacles.segment(*a, *b, clear).is_ok()));
    if deck_crossed {
        return kept_leg(
            job,
            KeptReason::Held,
            "crosses_deck",
            0,
            false,
            limit,
            drawn_mean,
            original_feasible,
        );
    }
    let band = Band::from_params(params, limit, cell);
    let field = Field::build(
        box_min,
        box_max,
        end_anchor.at,
        params.step_m,
        cell,
        &band,
        &ground_fn,
        &obstacles,
        &walls,
        clear,
        stray_cap,
        params.field_max_cells,
    );
    let input = LegInput {
        start: start_anchor,
        end: end_anchor,
        radius,
        step_m: params.step_m,
        band: &band,
        ground: &ground_fn,
        obstacles: &obstacles,
        walls: &walls,
        clear,
        field: &field,
        weights: Weights::from_params(params),
        stray_cap_m: stray_cap,
        arrive_reach_m: (3.0 * params.step_m).max(4.0 * radius),
        self_along_m: self_along,
        self_clear_m: self_clear,
        original: orig,
        budgets: params.budgets.as_slice(),
    };
    match search_leg_relaxed(&input, &limits) {
        Ok(LegOutcome::Found(route)) => {
            let rband = Band::from_params(params, route.grade_limit, cell);
            let planned_grade = arrive(&route.points, &ground_fn, &rband)
                .map_or(route.planned_grade, |p| p.max_grade());
            LegPlan {
                a: job.a,
                b: job.b,
                outcome: LegOutcomeKind::Planned,
                start: job.start.anchor_kind(),
                end: job.end.anchor_kind(),
                states: route.states,
                budget_exhausted: false,
                weight: Some(route.weight),
                relax_steps: route.relax_steps,
                grade_limit: route.grade_limit,
                drawn_grade: drawn.mean,
                planned_grade,
                original_feasible,
                route: Some(route),
            }
        }
        Ok(LegOutcome::NoRoute {
            budget_exhausted,
            states,
            ..
        }) => {
            let lead = params.lead * wanted;
            let near_fixed = |kind: EndKind, at: (f64, f64)| {
                kind.fixed()
                    && job
                        .tight
                        .iter()
                        .any(|&k| chain.pts.get(k).is_some_and(|p| dist(*p, at) <= lead))
            };
            let (reason, detail) =
                if near_fixed(job.start, start_anchor.at) || near_fixed(job.end, end_anchor.at) {
                    (KeptReason::Held, "fixed_anchor")
                } else {
                    (KeptReason::NoRoute, "no_route")
                };
            kept_leg(
                job,
                reason,
                detail,
                states,
                budget_exhausted,
                limit,
                drawn.mean,
                original_feasible,
            )
        }
        Err(_) => kept_leg(
            job,
            KeptReason::Held,
            "invalid_input",
            0,
            false,
            limit,
            drawn.mean,
            original_feasible,
        ),
    }
}

/// Which samples of the own chain outside a leg are exempt: the first `self_free` metres **along the line** from
/// its end (`from_end`, the part before a start anchor) or from its start (the part after an end anchor), on the
/// line's 2 m samples. The road a leg continues into is free for that length along the chain, not by straight-line
/// distance from the anchor.
fn self_free_flags(line: &[(f64, f64)], from_end: bool, self_free: f64) -> Vec<bool> {
    let samples = samples_with_arc(line);
    let total = samples.last().map_or(0.0, |&(_, arc)| arc);
    samples
        .iter()
        .map(|&(_, arc)| {
            if from_end {
                total - arc <= self_free
            } else {
                arc <= self_free
            }
        })
        .collect()
}

/// The tight length of a chain, in the smoothing's measure: half of every segment next to a tight sample.
fn chain_tight_length(chain: &ChainLine) -> f64 {
    let n = chain.pts.len();
    let mut total = 0.0;
    for k in 0..n.saturating_sub(1) {
        let (Some(&from), Some(&to)) = (chain.arc.get(k), chain.arc.get(k + 1)) else {
            continue;
        };
        let half = (to - from) / 2.0;
        for e in [k, k + 1] {
            if !chain.ring && (e == 0 || e + 1 >= n) {
                continue;
            }
            let minimum = chain.minr.get(e).copied().unwrap_or(f64::INFINITY);
            if usable(minimum) && radius_at_index(chain, e) < minimum {
                total += half;
            }
        }
    }
    total
}

/// A planned leg's route, ready to be written into a stretch.
struct LegRoute {
    a_arc: f64,
    b_arc: f64,
    route: Vec<(f64, f64)>,
    cum: Vec<f64>,
    total: f64,
}

/// Pushes the part of a route between the fractions `f0` and `f1` of its length.
fn push_route(
    out: &mut Vec<(f64, f64)>,
    route: &[(f64, f64)],
    cum: &[f64],
    total: f64,
    f0: f64,
    f1: f64,
) {
    if route.is_empty() || !total.is_finite() || total <= 0.0 {
        return;
    }
    let point_at = |t: f64| -> Option<(f64, f64)> {
        if t <= 0.0 {
            return route.first().copied();
        }
        if t >= 1.0 {
            return route.last().copied();
        }
        let target = t * total;
        let k = cum
            .partition_point(|&c| c < target)
            .saturating_sub(1)
            .min(route.len().saturating_sub(2));
        let a = route.get(k).copied()?;
        let b = route.get(k + 1).copied()?;
        let c0 = cum.get(k).copied().unwrap_or(0.0);
        let c1 = cum.get(k + 1).copied().unwrap_or(c0);
        let span = c1 - c0;
        let u = if span > 0.0 {
            ((target - c0) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Some((a.0 + (b.0 - a.0) * u, a.1 + (b.1 - a.1) * u))
    };
    let mut push = |p: (f64, f64)| {
        if out.last() != Some(&p) {
            out.push(p);
        }
    };
    if let Some(p) = point_at(f0) {
        push(p);
    }
    for k in 1..route.len() {
        let g = cum.get(k).copied().unwrap_or(0.0) / total;
        if g > f0 + ARC_EPS
            && g < f1 - ARC_EPS
            && let Some(&p) = route.get(k)
        {
            push(p);
        }
    }
    if let Some(p) = point_at(f1) {
        push(p);
    }
}

/// A stretch's points with the planned parts of the legs written in, in the chain's direction.
fn rewrite_stretch(points: &[(f64, f64)], s0: f64, legs: &[&LegRoute]) -> Vec<(f64, f64)> {
    let arcs = cumulative(points);
    let s1 = arcs.last().copied().unwrap_or(s0) + s0;
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(points.len());
    let mut j = 0_usize;
    for leg in legs {
        let lo = leg.a_arc.max(s0);
        let hi = leg.b_arc.min(s1);
        if hi <= s0 + ARC_EPS || lo >= s1 - ARC_EPS {
            continue;
        }
        while j < points.len() {
            let at = s0 + arcs.get(j).copied().unwrap_or(0.0);
            if at < lo - ARC_EPS {
                if let Some(&p) = points.get(j) {
                    out.push(p);
                }
                j += 1;
            } else {
                break;
            }
        }
        while j < points.len() {
            let at = s0 + arcs.get(j).copied().unwrap_or(0.0);
            if at <= hi + ARC_EPS {
                j += 1;
            } else {
                break;
            }
        }
        let span = leg.b_arc - leg.a_arc;
        let frac = |x: f64| {
            if span > 0.0 {
                ((x - leg.a_arc) / span).clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        push_route(
            &mut out,
            &leg.route,
            &leg.cum,
            leg.total,
            frac(lo),
            frac(hi),
        );
    }
    while j < points.len() {
        if let Some(&p) = points.get(j) {
            out.push(p);
        }
        j += 1;
    }
    out
}

/// The main stage: finds the zones, searches the legs and writes the routes back.
#[expect(
    clippy::too_many_lines,
    reason = "the stages of the design run in one sequence, in their order"
)]
fn plan_graph_impl(
    graph: &RoadGraph,
    ground: &Heightmap,
    min_radius: &dyn Fn(&Stretch) -> f64,
    in_water: WaterTest<'_>,
    streams: &[Vec<(f64, f64)>],
    params: &RerouteParams,
) -> (RoadGraph, RerouteReport) {
    let min_of: Vec<f64> = graph.stretches.iter().map(min_radius).collect();
    let half_of: Vec<f64> = graph
        .stretches
        .iter()
        .map(|s| s.family.width_m() / 2.0)
        .collect();
    let context = GraphContext::new(graph);
    let plans = plan_chains(graph);
    let mut skipped: Vec<(String, String)> = Vec::new();
    let mut chains: Vec<ChainLine> = Vec::new();
    for plan in &plans {
        if let Some(chain) = build_chain_line(
            graph,
            &context,
            plan,
            &min_of,
            &half_of,
            params,
            &mut skipped,
        ) {
            chains.push(chain);
        }
    }
    let tight_before: f64 = chains.iter().map(chain_tight_length).sum();
    let mut works: Vec<ZoneWork> = Vec::new();
    for (ci, chain) in chains.iter().enumerate() {
        for (ordinal, spec) in find_zones(chain, params.lead).into_iter().enumerate() {
            works.push(make_zone_work(
                ci,
                chain,
                spec,
                ordinal,
                params,
                &mut skipped,
            ));
        }
    }
    let mut jobs: Vec<LegJob> = Vec::new();
    for (zi, work) in works.iter().enumerate() {
        for leg in &work.legs {
            let tight: Vec<usize> = work
                .tight
                .iter()
                .copied()
                .filter(|&t| t >= leg.a && t <= leg.b)
                .collect();
            jobs.push(LegJob {
                zone: zi,
                chain: work.chain,
                a: leg.a,
                b: leg.b,
                start: leg.start,
                end: leg.end,
                tight,
            });
        }
    }
    let results: Vec<LegPlan> = jobs
        .par_iter()
        .map(|job| match chains.get(job.chain) {
            Some(chain) => plan_leg(job, chain, graph, ground, in_water, streams, params),
            None => kept_leg(job, KeptReason::Held, "no_chain", 0, false, 0.0, 0.0, true),
        })
        .collect();
    let mut zone_legs: Vec<Vec<&LegPlan>> = vec![Vec::new(); works.len()];
    for (job, result) in jobs.iter().zip(&results) {
        if let Some(list) = zone_legs.get_mut(job.zone) {
            list.push(result);
        }
    }
    let mut zones: Vec<PlannedZone> = Vec::new();
    let ground_fn = |p: (f64, f64)| ground_at(ground, p);
    for (zi, work) in works.iter().enumerate() {
        let Some(chain) = chains.get(work.chain) else {
            continue;
        };
        let legs: &[&LegPlan] = zone_legs.get(zi).map_or(&[], Vec::as_slice);
        zones.push(build_zone(work, chain, legs, graph, &ground_fn, params));
    }
    pair_partners(&mut zones);
    for zone in &zones {
        if matches!(zone.outcome, ZoneOutcome::Kept { .. })
            && !skipped.iter().any(|(id, _)| id == &zone.id)
            && let ZoneOutcome::Kept { detail, .. } = &zone.outcome
        {
            skipped.push((zone.id.clone(), detail.clone()));
        }
    }

    let mut leg_routes: BTreeMap<usize, Vec<LegRoute>> = BTreeMap::new();
    for (work, results) in works.iter().zip(&zone_legs) {
        let Some(chain) = chains.get(work.chain) else {
            continue;
        };
        for (leg, result) in work.legs.iter().zip(results.iter()) {
            if matches!(result.outcome, LegOutcomeKind::Planned)
                && let Some(route) = &result.route
            {
                let cum = cumulative(&route.points);
                let total = cum.last().copied().unwrap_or(0.0);
                leg_routes.entry(work.chain).or_default().push(LegRoute {
                    a_arc: chain.arc.get(leg.a).copied().unwrap_or(0.0),
                    b_arc: chain.arc.get(leg.b).copied().unwrap_or(0.0),
                    route: route.points.clone(),
                    cum,
                    total,
                });
            }
        }
    }
    let mut result = graph.clone();
    for (ci, legs) in &leg_routes {
        let Some(chain) = chains.get(*ci) else {
            continue;
        };
        let mut sorted: Vec<&LegRoute> = legs.iter().collect();
        sorted.sort_by(|a, b| a.a_arc.total_cmp(&b.a_arc));
        let mut first = 0_usize;
        for (lj, link) in chain.links.iter().enumerate() {
            let last = chain.bounds.get(lj).copied().unwrap_or(first);
            let s0 = chain.arc.get(first).copied().unwrap_or(0.0);
            let s1 = chain.arc.get(last).copied().unwrap_or(s0);
            let here: Vec<&LegRoute> = sorted
                .iter()
                .copied()
                .filter(|l| l.a_arc < s1 - ARC_EPS && l.b_arc > s0 + ARC_EPS)
                .collect();
            if !here.is_empty()
                && let Some(stretch) = graph.stretches.get(link.stretch)
            {
                let mut line = stretch.points.clone();
                if link.reversed {
                    line.reverse();
                }
                let mut written = rewrite_stretch(&line, s0, &here);
                if link.reversed {
                    written.reverse();
                }
                if let Some(target) = result.stretches.get_mut(link.stretch) {
                    target.points = written;
                }
            }
            first = last;
        }
    }
    result.crossings = crossings_of(&result.stretches);
    let after_plans = plan_chains(&result);
    let mut after_skipped: Vec<(String, String)> = Vec::new();
    let mut tight_after = 0.0;
    for plan in &after_plans {
        if let Some(chain) = build_chain_line(
            &result,
            &context,
            plan,
            &min_of,
            &half_of,
            params,
            &mut after_skipped,
        ) {
            tight_after += chain_tight_length(&chain);
        }
    }
    let states = zones.iter().map(|z| z.states).sum();
    let steeper_than_drawn = results
        .iter()
        .filter(|r| {
            matches!(r.outcome, LegOutcomeKind::Planned)
                && r.planned_grade > r.drawn_grade + params.steeper_margin
        })
        .count();
    let max_stray = zones.iter().map(|z| z.max_stray_m).fold(0.0, f64::max);
    (
        result,
        RerouteReport {
            zones,
            skipped,
            tight_before_m: tight_before,
            tight_after_m: tight_after,
            max_stray_m: max_stray,
            steeper_than_drawn,
            reverted: 0,
            states,
        },
    )
}

/// Builds the report entry of one zone from the results of its legs.
#[expect(
    clippy::too_many_lines,
    clippy::many_single_char_names,
    reason = "the report entry has many fields and the zone's indices are `s` and `e`"
)]
fn build_zone(
    work: &ZoneWork,
    chain: &ChainLine,
    legs: &[&LegPlan],
    graph: &RoadGraph,
    ground: &dyn Fn((f64, f64)) -> Option<f64>,
    params: &RerouteParams,
) -> PlannedZone {
    let (s, e) = (work.s, work.e);
    let at = chain.pts.get(work.tightest).copied().unwrap_or_default();
    let radius_before = radius_at_index(chain, work.tightest);
    let wanted = chain
        .minr
        .get(work.tightest)
        .copied()
        .unwrap_or(f64::INFINITY);
    let zone_line = chain.pts.get(s..=e).unwrap_or(&[]);
    let drawn = drawn_grade(zone_line, ground, params.drawn_window_m);
    let (drawn_mean, drawn_window) = drawn.map_or((0.0, 0.0), |d| (d.mean, d.window));
    let outcome = match &work.fixed {
        Some((reason, detail)) => ZoneOutcome::Kept {
            reason: *reason,
            detail: detail.clone(),
        },
        None => match legs
            .iter()
            .find(|r| matches!(r.outcome, LegOutcomeKind::Kept { .. }))
        {
            Some(failed) => match &failed.outcome {
                LegOutcomeKind::Kept { reason, detail } => ZoneOutcome::Kept {
                    reason: *reason,
                    detail: detail.clone(),
                },
                LegOutcomeKind::Planned => ZoneOutcome::Planned,
            },
            None => ZoneOutcome::Planned,
        },
    };
    let planned: Vec<&&LegPlan> = legs
        .iter()
        .filter(|r| matches!(r.outcome, LegOutcomeKind::Planned) && r.route.is_some())
        .collect();
    let mut radius_after = radius_before;
    if !planned.is_empty() {
        radius_after = planned
            .iter()
            .filter_map(|r| r.route.as_ref())
            .map(|route| {
                let resampled = resample(&route.points, SAMPLE_M);
                resampled
                    .windows(3)
                    .filter_map(|w| match w {
                        [a, b, c] => Some(radius_at(*a, *b, *c)),
                        _ => None,
                    })
                    .fold(f64::INFINITY, f64::min)
            })
            .fold(f64::INFINITY, f64::min);
    }
    let mut length_after =
        chain.arc.get(e).copied().unwrap_or(0.0) - chain.arc.get(s).copied().unwrap_or(0.0);
    let mut max_stray = 0.0_f64;
    let mut planned_grade = 0.0_f64;
    let fallback_limit = zone_grade_limit(chain, work, graph, ground, params);
    let mut grade_limit: Option<f64> = None;
    let mut relax_steps = 0;
    let (mut climb, mut earth, mut cut, mut fill) = (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
    for r in &planned {
        if let Some(route) = &r.route {
            let original = chain.arc.get(r.b).copied().unwrap_or(0.0)
                - chain.arc.get(r.a).copied().unwrap_or(0.0);
            length_after += route.length_m - original;
            max_stray = max_stray.max(route.max_move_m);
            planned_grade = planned_grade.max(r.planned_grade);
            // the limit of the first planned leg, as `weight` is its weight
            grade_limit = grade_limit.or(Some(r.grade_limit));
            relax_steps = relax_steps.max(r.relax_steps);
            climb += route.climb_m;
            earth += route.earthwork_m2;
            cut = cut.max(route.cut_max_m);
            fill = fill.max(route.fill_max_m);
        }
    }
    length_after = length_after.max(0.0);
    let anchors = match (legs.first(), legs.last()) {
        (Some(first), Some(last)) => (first.start, last.end),
        _ => (
            zone_end_kind(chain, s, true).anchor_kind(),
            zone_end_kind(chain, e, false).anchor_kind(),
        ),
    };
    let junction = [anchors.0, anchors.1].iter().find_map(|k| match k {
        AnchorKind::Junction(node) => Some(*node),
        _ => None,
    });
    let mut stretches: Vec<String> = Vec::new();
    for k in s..=e {
        if let Some(&idx) = chain.stretch_of.get(k)
            && let Some(stretch) = graph.stretches.get(idx)
            && stretches.last() != Some(&stretch.id)
        {
            stretches.push(stretch.id.clone());
        }
    }
    let candidate = match &outcome {
        ZoneOutcome::Planned => None,
        ZoneOutcome::Kept { reason, .. } => match reason {
            KeptReason::Held | KeptReason::NoRoute => Some(Candidate::Plate),
            KeptReason::BlockedByWater => Some(Candidate::CurvedBridge),
            KeptReason::BlockedByRoad => None,
        },
    };
    PlannedZone {
        id: work.id.clone(),
        at,
        stretches,
        legs: legs.len(),
        anchors,
        junction,
        partner: None,
        outcome,
        radius_before_m: radius_before,
        radius_wanted_m: wanted,
        radius_after_m: radius_after,
        length_before_m: chain.arc.get(e).copied().unwrap_or(0.0)
            - chain.arc.get(s).copied().unwrap_or(0.0),
        length_after_m: length_after,
        max_stray_m: max_stray,
        drawn_grade: drawn_mean,
        drawn_grade_window: drawn_window,
        planned_grade,
        grade_limit: grade_limit.unwrap_or(fallback_limit),
        relax_steps,
        climb_m: climb,
        earthwork_m2: earth,
        cut_max_m: cut,
        fill_max_m: fill,
        states: legs.iter().map(|r| r.states).sum(),
        weight: planned.iter().find_map(|r| r.weight),
        budget_exhausted: legs.iter().any(|r| r.budget_exhausted),
        original_feasible: legs.iter().all(|r| r.original_feasible),
        candidate,
    }
}

/// The grade limit a kept zone would have been planned at.
fn zone_grade_limit(
    chain: &ChainLine,
    work: &ZoneWork,
    graph: &RoadGraph,
    ground: &dyn Fn((f64, f64)) -> Option<f64>,
    params: &RerouteParams,
) -> f64 {
    let line = chain.pts.get(work.s..=work.e).unwrap_or(&[]);
    let Some(drawn) = drawn_grade(line, ground, params.drawn_window_m) else {
        return 0.0;
    };
    let families = leg_families(graph, chain, work.s, work.e);
    leg_limit(&families, &drawn).map_or(0.0, |(limit, _)| limit)
}

/// What fixes the end of a zone at sample `idx`, for a zone without legs.
fn zone_end_kind(chain: &ChainLine, idx: usize, at_start: bool) -> EndKind {
    let last = chain.pts.len().saturating_sub(1);
    if idx == 0 {
        return chain.start_kind;
    }
    if idx >= last {
        return chain.end_kind;
    }
    if in_held(chain, idx) {
        return held_kind(chain, idx);
    }
    if at_start {
        if idx > 0 && in_held(chain, idx - 1) {
            return held_kind(chain, idx - 1);
        }
    } else if in_held(chain, idx + 1) {
        return held_kind(chain, idx + 1);
    }
    EndKind::Line
}

/// Names the partner zone of every bend that crosses a junction, when exactly two zones meet there.
fn pair_partners(zones: &mut [PlannedZone]) {
    let mut by_junction: BTreeMap<i64, BTreeSet<usize>> = BTreeMap::new();
    for (i, zone) in zones.iter().enumerate() {
        for kind in [zone.anchors.0, zone.anchors.1] {
            if let AnchorKind::Junction(node) = kind {
                by_junction.entry(node).or_default().insert(i);
            }
        }
    }
    for ids in by_junction.values() {
        if ids.len() != 2 {
            continue;
        }
        let mut it = ids.iter().copied();
        let (Some(a), Some(b)) = (it.next(), it.next()) else {
            continue;
        };
        let (Some(ida), Some(idb)) = (
            zones.get(a).map(|z| z.id.clone()),
            zones.get(b).map(|z| z.id.clone()),
        ) else {
            continue;
        };
        if let Some(zone) = zones.get_mut(a) {
            zone.partner = Some(idb);
        }
        if let Some(zone) = zones.get_mut(b) {
            zone.partner = Some(ida);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_self_free_part_of_the_own_chain_is_measured_along_it() {
        // 100 m out, then back to a point 10 m from the start: the end is 10 m away as the crow flies but 120 m
        // along the line, so it is not in the first 32 m of the road the leg continues into
        let line = [(0.0, 0.0), (100.0, 0.0), (100.0, 10.0), (0.0, 10.0)];
        let flags = self_free_flags(&line, false, 32.0);
        assert_eq!(flags.first(), Some(&true));
        assert_eq!(
            flags.last(),
            Some(&false),
            "the far end was exempt by its straight-line distance"
        );
        let from_end = self_free_flags(&line, true, 32.0);
        assert_eq!(from_end.last(), Some(&true));
        assert_eq!(from_end.first(), Some(&false));
    }
}

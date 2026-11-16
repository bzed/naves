// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Height profiles of the road chains.
//!
//! A road's height is not its terrain's: a profile follows the terrain but keeps to a grade limit, a limit on
//! how much the grade may change from one piece to the next (the number that makes a road feel bumpy), and
//! the height of whatever it meets. The terrain is then shaped to the profile, see [`crate::conform`].
//!
//! The heights of the whole network come out of one quadratic program, [`solve_clearing`] (the design is
//! `docs/superpowers/specs/2026-11-16-height-solve-design.md`, the model is in `profile/model.rs`): every joint
//! height and every junction's height and tilt is an unknown, every rule is a row, and what cannot be kept is
//! a slack whose value is the finding ([`GradeExcess`], [`Clearance`], the junctions in
//! [`Junctions::unresolved`]). The program always has a solution, and the same input gives the same result bit
//! for bit.
//!
//! This module holds what the model is built from: the chains and their variables (joints less than
//! [`MERGE_JOINT_M`] apart are one variable), the places where a chain end lies on another road, the grade limit
//! of each segment, the spots at which roads that share terrain cells are compared, and the measuring of the
//! clearance under a bridge over a road.

use std::collections::BTreeMap;

use crate::catalog::Family;
use crate::heightmap::Heightmap;
use crate::junctions::{Junctions, RingPlane};
use crate::qp::{Problem, QpError};
use crate::roadnet::{OverCrossing, RoadGraph, Stretch, over_crossings};
use crate::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad};

mod holds;
mod model;

#[doc(hidden)]
pub use holds::{Hold, HoldRule, chain_holds, chain_holds_unchecked};

/// The steepest grade a road may carry where another road meets it (rise over run).
pub const MEETING_GRADE: f64 = 0.06;

/// The spacing at which the second difference of the smoothing is (1, -2, 1).
const REFERENCE_M: f64 = 12.5;
/// The smallest spacing the second difference is built on.
const MIN_STENCIL_M: f64 = 1.0;

/// Joints of a chain closer together than this (along the chain) are one variable: the overlapped straights of
/// the placement put two joints a few centimetres apart, which the pieces (draped on the terrain, not pitched)
/// cannot tell apart, and which the checks would read as a grade and a bend of hundreds of times the limit.
pub const MERGE_JOINT_M: f64 = 1.0;
/// Weight of the second difference against the terrain heights.
const SMOOTHNESS: f64 = 30.0;

/// The importance of a road when two want the same ground: higher wins.
pub(crate) fn rank(family: Family) -> u8 {
    match family {
        Family::Asf1 => 6,
        Family::Asf2 => 5,
        Family::City => 4,
        Family::Asf3 => 3,
        Family::Grav => 2,
        Family::Mud => 1,
    }
}

/// The height of one chain at the joints of its pieces; linear between them.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadStrip {
    /// The roads of the strip, as indices into [`PlacedNetwork::roads`].
    pub slots: Vec<usize>,
    /// Position and height at the start of the first piece and the end of each piece.
    pub joints: Vec<(f64, f64, f64)>,
    /// For each segment between two joints, whether it is a bridge deck; the ground below a span is left
    /// alone. May be shorter than the segments, which then are not bridges.
    pub bridge: Vec<bool>,
    /// For each segment between two joints, whether it is a junction piece: its side ports, where
    /// side roads end, lie beyond the through road's edge. May be shorter than the segments.
    pub junction: Vec<bool>,
    /// For each segment between two joints, the side slope of the original ground there (rise over run, not
    /// negative). May be shorter than the segments; a segment past its end has none (0).
    pub side_slope: Vec<f64>,
    /// Half the width of the road.
    pub half_width: f64,
    /// How important the road is; higher wins where two roads clash.
    pub rank: u8,
}

/// What a [`GradeExcess`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradeExcessKind {
    /// The road is steeper than its limit: `grade` and `limit` are rise over run.
    Grade,
    /// The grade changes faster than the minimum vertical radius allows (a crest or a sag that is too
    /// sharp): `grade` is the change of grade per metre and `limit` is `1 / minimum radius`, both per metre.
    VerticalBend,
}

/// A place where the limits could not be kept.
#[derive(Debug, Clone, PartialEq)]
pub struct GradeExcess {
    /// Where.
    pub at: (f64, f64),
    /// Whether it is a grade or a bend of the profile.
    pub kind: GradeExcessKind,
    /// The grade, or the change of grade per metre, found there.
    pub grade: f64,
    /// The limit it exceeds.
    pub limit: f64,
}

/// The distance between the surface of a road and the underside of the deck of a bridge over it.
pub const BRIDGE_CLEARANCE_M: f64 = 4.5;

/// How much less than [`BRIDGE_CLEARANCE_M`] still counts as reaching it.
pub const CLEARANCE_TOLERANCE_M: f64 = 0.05;

/// The clearance under a bridge over a road, as solved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clearance {
    /// Where the lines of the two roads cross.
    pub at: (f64, f64),
    /// The distance from the surface of the road below to the underside of the deck, in metres.
    pub reached: f64,
    /// What it should be: [`BRIDGE_CLEARANCE_M`].
    pub wanted: f64,
}

/// All profiles.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Profiles {
    /// The clearance under every bridge over a road that was measured (see [`solve_clearing`]).
    pub clearances: Vec<Clearance>,
    /// One strip per chain.
    pub strips: Vec<RoadStrip>,
    /// Where the limits are exceeded.
    pub excesses: Vec<GradeExcess>,
}

/// A place where a chain end meets a road solved earlier: on the strip `owner`, between its
/// joints `segment` and `segment + 1`, `t` of the way from the first to the second.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Meeting {
    owner: usize,
    segment: usize,
    t: f64,
}

/// The two ends of a bridge deck.
type DeckEnds = ((f64, f64), (f64, f64));

/// A chain (by index) and the position of one of its ends.
type ChainEnd = (usize, (f64, f64));

/// The variables of a chain: one per joint, except that joints less than a metre apart share one variable
/// and the two joints of a junction piece are held to its plane.
struct Chain {
    /// (x, y, variable) of each joint.
    joints: Vec<(f64, f64, usize)>,
    /// Position of each variable.
    at: Vec<(f64, f64)>,
    /// The length of the piece that joins variable `i` and `i + 1`.
    spacing: Vec<f64>,
    /// For a variable that is a bridge deck, the two ends of the deck: the ground there, not the ground under
    /// the span, is what the deck is held to.
    deck: Vec<Option<DeckEnds>>,
}

/// The chain of the roads `roads`, with joints less than [`MERGE_JOINT_M`] apart merged into one variable
/// (never into a bridge deck's variable, nor across joints held to two different junctions).
fn chain_of(
    net: &PlacedNetwork,
    roads: &[usize],
    cell: f64,
    junctions: &Junctions,
) -> Option<Chain> {
    let plain = build_chain(net, roads, None)?;
    let close = plain
        .joints
        .windows(2)
        .any(|w| matches!(w, [a, b] if (b.0 - a.0).hypot(b.1 - a.1) < MERGE_JOINT_M));
    if !close {
        return Some(plain);
    }
    let held = holds::joint_junctions(net, &plain, roads, cell, junctions);
    build_chain(net, roads, Some(&held))
}

/// [`chain_of`] with `merge` (the junction each joint is held to) deciding what is merged; `None` merges nothing.
fn build_chain(
    net: &PlacedNetwork,
    roads: &[usize],
    merge: Option<&[Option<usize>]>,
) -> Option<Chain> {
    let first = net.roads.get(*roads.first()?)?;
    let mut joints = vec![(first.start.x, first.start.y, 0)];
    let mut at = vec![(first.start.x, first.start.y)];
    let mut spacing = Vec::new();
    let mut deck: Vec<Option<DeckEnds>> = vec![None];
    // the joint that opened the last variable, and the way along the chain from it
    let (mut opener, mut since) = (0_usize, 0.0_f64);
    for &i in roads {
        let r = net.roads.get(i)?;
        // the run between the joints, not the length of the piece: a piece that starts a little before or after
        // the end of the one before it (the pieces of a chain do not all meet end to start) is a run of the
        // profile from one joint to the next, and the grades are measured on that
        let length = joints
            .last()
            .map_or(0.0, |(x, y, _)| (r.end.x - x).hypot(r.end.y - y));
        if r.kind == PlacedKind::Bridge {
            let v = at.len() - 1;
            joints.push((r.end.x, r.end.y, v));
            if let (Some(first_joint), Some(slot), Some(ends)) = (
                joints.iter().find(|j| j.2 == v),
                at.get_mut(v),
                deck.get_mut(v),
            ) {
                *slot = (
                    f64::midpoint(first_joint.0, r.end.x),
                    f64::midpoint(first_joint.1, r.end.y),
                );
                let start = ends.map_or((first_joint.0, first_joint.1), |(start, _)| start);
                *ends = Some((start, (r.end.x, r.end.y)));
            }
        } else {
            let var = at.len() - 1;
            let joint = joints.len();
            let mergeable = merge.is_some_and(|held| {
                let opener_hold = held.get(opener).copied().flatten();
                let joint_hold = held.get(joint).copied().flatten();
                since + length < MERGE_JOINT_M
                    && deck.get(var).is_some_and(Option::is_none)
                    && !matches!((opener_hold, joint_hold), (Some(first), Some(second)) if first != second)
            });
            if mergeable {
                since += length;
                joints.push((r.end.x, r.end.y, var));
            } else {
                joints.push((r.end.x, r.end.y, at.len()));
                at.push((r.end.x, r.end.y));
                deck.push(None);
                spacing.push(length);
                (opener, since) = (joint, 0.0);
            }
        }
    }
    Some(Chain {
        joints,
        at,
        spacing,
        deck,
    })
}

/// The signed angle from one heading to another, in `(-π, π]`.
fn wrap_angle(angle: f64) -> f64 {
    (angle + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

/// The distance from `p` to the segment `a`-`b`.
fn point_segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length2 = dx * dx + dy * dy;
    let t = if length2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
}

/// The first and last point of every chain, in a coarse grid, to find the chains that end near a road without
/// asking every chain.
struct ChainEnds {
    /// (chain, point) of the ends in each cell of [`ChainEnds::CELL_M`] metres.
    cells: BTreeMap<(i64, i64), Vec<ChainEnd>>,
}

impl ChainEnds {
    const CELL_M: f64 = 64.0;
    /// More cells than this in a query: ask every end instead.
    const MAX_QUERY_CELLS: i64 = 4096;

    fn cell(v: f64) -> i64 {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a cell number of a map position; saturates"
        )]
        let n = (v / Self::CELL_M).floor() as i64;
        n
    }

    fn new(net: &PlacedNetwork) -> ChainEnds {
        let mut cells: BTreeMap<(i64, i64), Vec<ChainEnd>> = BTreeMap::new();
        for (index, chain) in net.chains.iter().enumerate() {
            let first = chain.first().and_then(|i| net.roads.get(*i));
            let last = chain.last().and_then(|i| net.roads.get(*i));
            for point in [
                first.map(|r| (r.start.x, r.start.y)),
                last.map(|r| (r.end.x, r.end.y)),
            ]
            .into_iter()
            .flatten()
            {
                cells
                    .entry((Self::cell(point.0), Self::cell(point.1)))
                    .or_default()
                    .push((index, point));
            }
        }
        ChainEnds { cells }
    }

    /// Whether an end of a chain other than `roads` lies within `reach` of the segment `a`-`b` and `meets` it.
    fn any_near(
        &self,
        net: &PlacedNetwork,
        roads: &[usize],
        (a, b): ((f64, f64), (f64, f64)),
        reach: f64,
        meets: &dyn Fn((f64, f64)) -> bool,
    ) -> bool {
        let others = |(index, point): &ChainEnd| {
            net.chains
                .get(*index)
                .is_some_and(|c| c.as_slice() != roads)
                && point_segment_distance(*point, a, b) <= reach
                && meets(*point)
        };
        let (x0, x1) = (
            Self::cell(a.0.min(b.0) - reach),
            Self::cell(a.0.max(b.0) + reach),
        );
        let (y0, y1) = (
            Self::cell(a.1.min(b.1) - reach),
            Self::cell(a.1.max(b.1) + reach),
        );
        let span = x1
            .saturating_sub(x0)
            .saturating_add(1)
            .saturating_mul(y1.saturating_sub(y0).saturating_add(1));
        if !(0..=Self::MAX_QUERY_CELLS).contains(&span) {
            return self.cells.values().flatten().any(others);
        }
        self.cells
            .range((x0, i64::MIN)..=(x1, i64::MAX))
            .filter(|((_, y), _)| (y0..=y1).contains(y))
            .flat_map(|(_, ends)| ends)
            .any(others)
    }
}

/// Whether a road of the chain `roads` is where another chain ends on it: an end of another chain lies within the
/// road's half width plus 12.5 m of it, and that end is on the chain's surface (`strip`), as a chain end that meets
/// a road is found for the model ([`meeting_on`], within [`ON_ROAD_M`] of its edge). An end beside the road that does
/// not reach it (a dead end up the slope beside it) is no meeting.
fn meets_another_chain(
    net: &PlacedNetwork,
    ends: &ChainEnds,
    roads: &[usize],
    road: &PlacedRoad,
    strip: &RoadStrip,
) -> bool {
    let reach = road.width / 2.0 + 12.5;
    ends.any_near(
        net,
        roads,
        ((road.start.x, road.start.y), (road.end.x, road.end.y)),
        reach,
        &|end| meeting_on(strip, 0, end, ON_ROAD_M, ON_ROAD_M).is_some(),
    )
}

/// The grade limit of each segment of a chain: the family's aim, lowered at a meeting (not between two junction
/// heights) and in a tight bend.
fn segment_grades(
    net: &PlacedNetwork,
    ends: &ChainEnds,
    chain: &Chain,
    roads: &[usize],
    base: f64,
    held: &[Option<f64>],
) -> Vec<f64> {
    let mut limits = Vec::new();
    // the chain's surface, for the ends that meet it
    let strip = roads
        .first()
        .and_then(|i| net.roads.get(*i))
        .map(|first| strip_of(net, roads, joint_heights(chain, &[]), &[], first));
    let first_held = held.iter().position(Option::is_some);
    let last_held = held.iter().rposition(Option::is_some);
    for (k, &index) in roads.iter().enumerate() {
        let Some(road) = net.roads.get(index) else {
            continue;
        };
        if road.kind == PlacedKind::Bridge {
            continue;
        }
        let mut limit = base;
        // a track may meet another road at its own grade: the meeting grade would lift the lower end of a
        // steep track out of the ground
        let track = matches!(road.family, Family::Grav | Family::Mud);
        // and a piece between the first and the last junction height of the chain has the grades those heights
        // make (a free joint between two held ones is part of the span): the meeting grade is for the approach
        // from a free end to a junction. `held` is per variable, and joints less than a metre apart are one
        // variable, so the piece is found by the variable it ends at: the segment before that one
        let between_junctions = first_held.zip(last_held).is_some_and(|(first, last)| {
            chain
                .joints
                .get(k + 1)
                .and_then(|joint| joint.2.checked_sub(1))
                .is_some_and(|segment| first <= segment && segment < last)
        });
        if !track
            && !between_junctions
            && (road.kind == PlacedKind::Junction
                || strip
                    .as_ref()
                    .is_some_and(|strip| meets_another_chain(net, ends, roads, road, strip)))
        {
            limit = limit.min(MEETING_GRADE);
        }
        let turn = wrap_angle(road.end.heading - road.start.heading);
        if turn.abs() > 0.001 {
            let chord = (road.end.x - road.start.x).hypot(road.end.y - road.start.y);
            let radius = chord / (2.0 * (turn.abs() / 2.0).sin());
            if radius > 0.0 {
                let factor = ((radius - road.width / 2.0) / radius).max(0.5);
                limit = limit.min(base * factor);
            }
        }
        limits.push(limit);
    }
    limits
}

/// The height of a strip at a [`Meeting`], from its joints.
fn meeting_height(strip: &RoadStrip, meet: &Meeting) -> Option<f64> {
    let from = strip.joints.get(meet.segment)?;
    let to = strip.joints.get(meet.segment + 1)?;
    Some(from.2 + meet.t * (to.2 - from.2))
}

/// A chain end this far beyond the edge of another road still counts as meeting it.
pub(crate) const ON_ROAD_M: f64 = 1.0;

/// Where a point lies on one road's surface: the nearest place on `strip` whose surface it is on.
/// `reach` is how far beyond the edge it may be; on a junction piece `junction_reach` is how far
/// beyond the piece's own reach its side ports lie. The ground under a bridge does not count.
pub(crate) fn meeting_on(
    strip: &RoadStrip,
    owner: usize,
    at: (f64, f64),
    reach: f64,
    junction_reach: f64,
) -> Option<(f64, Meeting)> {
    let mut best: Option<(f64, Meeting)> = None;
    for segment in 0..strip.joints.len().saturating_sub(1) {
        if strip.bridge.get(segment).copied().unwrap_or(false) {
            continue;
        }
        let (Some(from), Some(to)) = (strip.joints.get(segment), strip.joints.get(segment + 1))
        else {
            continue;
        };
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let length2 = dx * dx + dy * dy;
        let t = if length2 > 0.0 {
            (((at.0 - from.0) * dx + (at.1 - from.1) * dy) / length2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let junction = strip.junction.get(segment).copied().unwrap_or(false);
        let distance = if junction {
            // a junction piece carries the side arms of the roads that end on it: its ports
            // are as far out as the piece is long
            let (mx, my) = (
                f64::midpoint(from.0, to.0) - at.0,
                f64::midpoint(from.1, to.1) - at.1,
            );
            (mx.hypot(my) - length2.sqrt() / 2.0).max(0.0)
        } else {
            (at.0 - from.0 - t * dx).hypot(at.1 - from.1 - t * dy)
        };
        let allowed = if junction {
            junction_reach
        } else {
            strip.half_width + reach
        };
        if distance <= allowed && best.as_ref().is_none_or(|(found, _)| distance < *found) {
            best = Some((distance, Meeting { owner, segment, t }));
        }
    }
    best
}

/// Where a chain end meets a road solved earlier: the nearest place whose surface the end lies
/// on - within the road's half width (and [`ON_ROAD_M`]) of its centre line, or, on a junction
/// piece, within the piece's own length of its centre, where its side ports are.
fn meeting(strips: &[RoadStrip], bounds: &[[f64; 4]], at: (f64, f64)) -> Option<Meeting> {
    let mut best: Option<(f64, Meeting)> = None;
    for (owner, strip) in strips.iter().enumerate() {
        // a strip whose box is not near cannot hold a meeting
        if bounds
            .get(owner)
            .is_some_and(|b| !((b[0]..=b[2]).contains(&at.0) && (b[1]..=b[3]).contains(&at.1)))
        {
            continue;
        }
        if let Some((distance, meet)) = meeting_on(strip, owner, at, ON_ROAD_M, ON_ROAD_M)
            && best.as_ref().is_none_or(|(found, _)| distance < *found)
        {
            best = Some((distance, meet));
        }
    }
    best.map(|(_, meet)| meet)
}

/// The box in which [`meeting`] can find a place on `strip` for a point: `(min x, min y, max x, max y)` of
/// its joints, widened by the strip's half width, [`ON_ROAD_M`] and the longest of its segments (a junction
/// piece reaches that far from its centre).
pub(crate) fn meeting_bounds(strip: &RoadStrip) -> [f64; 4] {
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut longest = 0.0_f64;
    for joint in &strip.joints {
        bounds = [
            bounds[0].min(joint.0),
            bounds[1].min(joint.1),
            bounds[2].max(joint.0),
            bounds[3].max(joint.1),
        ];
    }
    for pair in strip.joints.windows(2) {
        if let [from, to] = pair {
            longest = longest.max((to.0 - from.0).hypot(to.1 - from.1));
        }
    }
    let margin = strip.half_width + ON_ROAD_M + longest;
    [
        bounds[0] - margin,
        bounds[1] - margin,
        bounds[2] + margin,
        bounds[3] + margin,
    ]
}

/// The fixed heights of a chain's variables and where they lie on an earlier road: an end that
/// reaches one takes its height there and stays fixed, and so do up to eight variables inward
/// while they lie on that road's surface (within its half width and 1.5 cells of its centre line,
/// or on a junction piece within that distance beyond the piece's reach).
fn end_joins(
    chain: &Chain,
    strips: &[RoadStrip],
    bounds: &[[f64; 4]],
    cell: f64,
    held: &[Option<f64>],
) -> (Vec<Option<f64>>, Vec<(usize, Meeting)>) {
    let count = chain.at.len();
    let mut fixed = vec![None; count];
    let mut joins = Vec::new();
    for (side, end) in [0, count.saturating_sub(1)].into_iter().enumerate() {
        let Some(point) = chain.at.get(end) else {
            continue;
        };
        // a junction holds this end
        if held.get(end).is_some_and(Option::is_some) {
            continue;
        }
        let Some(meet) = meeting(strips, bounds, *point) else {
            continue;
        };
        let owner = meet.owner;
        let Some(owner_strip) = strips.get(owner) else {
            continue;
        };
        let reach = owner_strip.half_width + 1.5 * cell;
        let mut var = end;
        for _ in 0..8 {
            let Some(point) = chain.at.get(var) else {
                break;
            };
            let Some((_, meet_on)) = meeting_on(owner_strip, owner, *point, reach, reach) else {
                break;
            };
            let height = meeting_height(owner_strip, &meet_on);
            if let (Some(slot), Some(height)) = (fixed.get_mut(var), height) {
                *slot = Some(height);
                joins.push((var, meet_on));
            }
            if side == 0 {
                if var + 1 >= count {
                    break;
                }
                var += 1;
            } else {
                if var == 0 {
                    break;
                }
                var -= 1;
            }
        }
    }
    (fixed, joins)
}

/// The roundabout whose road the chain `roads` is, if it is one: it lies on its plane, not on the ground that
/// the plane was fitted to.
fn ring_of<'a>(
    net: &PlacedNetwork,
    roads: &[usize],
    junctions: &'a Junctions,
) -> Option<&'a RingPlane> {
    junctions.rings.iter().find(|ring| {
        roads
            .iter()
            .filter_map(|i| net.roads.get(*i))
            .any(|r| ring.stretches.contains(&r.slot.stretch))
    })
}

/// The strip of a solved chain: its joints at their heights and what the roads under them are.
fn strip_of(
    net: &PlacedNetwork,
    roads: &[usize],
    joints: Vec<(f64, f64, f64)>,
    side: &[f64],
    first: &PlacedRoad,
) -> RoadStrip {
    let is = |kind: PlacedKind| {
        roads
            .iter()
            .map(|i| net.roads.get(*i).is_some_and(|r| r.kind == kind))
            .collect()
    };
    RoadStrip {
        slots: roads.to_vec(),
        joints,
        bridge: is(PlacedKind::Bridge),
        junction: is(PlacedKind::Junction),
        side_slope: roads
            .iter()
            .map(|i| side.get(*i).copied().unwrap_or(0.0))
            .collect(),
        half_width: first.width / 2.0,
        rank: rank(first.family),
    }
}

/// Whether the road is part of the stretch `id` (a junction piece carries the ids of both its roads).
fn road_on_stretch(road: &PlacedRoad, id: &str) -> bool {
    road.slot.stretch.split('|').any(|part| part == id)
}

/// The position and height of each joint of `chain`.
fn joint_heights(chain: &Chain, heights: &[f64]) -> Vec<(f64, f64, f64)> {
    chain
        .joints
        .iter()
        .map(|joint| {
            (
                joint.0,
                joint.1,
                heights.get(joint.2).copied().unwrap_or(0.0),
            )
        })
        .collect()
}

/// Why the heights cannot be solved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    /// The solver did not report a solution.
    #[error("the road heights are not solved: {status}")]
    NotSolved {
        /// The size of the problem and the solver's status.
        status: String,
    },
    /// The input cannot be solved (no ground data under a road, a malformed problem).
    #[error("the road heights cannot be solved: {what}")]
    Invalid {
        /// What is wrong, and where.
        what: String,
    },
}

/// The profile of every chain of `net` as one quadratic program, without the clearance under bridges over roads
/// (there is no road graph; see [`solve_clearing`] for that). `side` is the side slope of the original ground at
/// each road of `net` (see [`crate::sideslope::side_slopes`]); it is kept in the strips for the terrain shaping.
/// How the problem is built and what is reported is described at [`solve_clearing`].
///
/// # Errors
///
/// As [`solve_clearing`].
pub fn solve_profiles(
    net: &PlacedNetwork,
    hm: &Heightmap,
    junctions: &Junctions,
    side: &[f64],
) -> Result<Profiles, ProfileError> {
    solve_clearing(net, hm, junctions, side, &RoadGraph::default(), &|_| None)
        .map(|(profiles, _)| profiles)
}

/// The heights of every chain of `net` and of its junctions, as one quadratic program
/// (`docs/superpowers/specs/2026-11-16-height-solve-design.md`), and the clearance under bridges over roads.
///
/// Every free joint height, and the height and tilt of every junction that holds a joint and is not on a ring,
/// is an unknown; `junctions` come in at their ground fit ([`crate::junctions::find_junctions`]: the median of
/// the ground at the junction's point, and its slope along the through road), and the solve pulls them to it
/// at a cost. A junction that holds no joint, or lies on a ring, keeps its input height and tilt. A road is
/// pulled to the ground and kept smooth; it is steeper than its grade aim only at a cost, steeper than its grade
/// ceiling or bent sharper than its vertical radius only where nothing else is possible, and that is reported in
/// [`Profiles::excesses`]. Roads that share terrain cells are tied to one height as far as the ground lets them.
/// The returned junctions have the solved heights and tilts. `unresolved` lists the links between the nearest
/// junction-held joints on both sides of a segment over its ceiling that has a held end; a segment over its
/// ceiling whose other end is a ring's constant or a meeting with another road, or whose two ends one junction
/// holds, is a [`GradeExcessKind::Grade`] excess without an entry.
///
/// A bridge over a road (see [`over_crossings`]) whose deck thickness `thickness` knows keeps
/// [`BRIDGE_CLEARANCE_M`] between the surface of the road below and the underside of its deck, as far as the
/// rest of the problem allows. It is a row of the same problem, solved once: the height of the deck less the
/// height of the road below at the crossing (between its two joints there) is at least the clearance and the
/// thickness, less a slack that is kept only where nothing else is possible. Both roads are free: the bridge's
/// road and the junctions on it rise and the road below may be lowered, each within its own grade limits and
/// vertical radius, and the cost shares the lift between them. A deck and the road below that one junction
/// holds cannot be parted, and what is short is in [`Profiles::clearances`] (measured on the result with
/// [`measure_clearances`]). A bridge whose thickness is `None`, or whose crossing is not placed, is not checked.
/// Roads that share terrain cells are not tied to each other within 50 m of a crossing, whether or not the
/// thickness is known (that needs the graph, which is why the build calls this function and not
/// [`solve_profiles`]).
///
/// # Errors
///
/// [`ProfileError::Invalid`] where the ground under a road, a junction's ground or tilt or a ring's plane is not
/// a number, and [`ProfileError::NotSolved`] when the solver reports no solution.
pub fn solve_clearing(
    net: &PlacedNetwork,
    hm: &Heightmap,
    junctions: &Junctions,
    side: &[f64],
    graph: &RoadGraph,
    thickness: &dyn Fn(&Stretch) -> Option<f64>,
) -> Result<(Profiles, Junctions), ProfileError> {
    solve_clearing_with(
        net,
        hm,
        junctions,
        side,
        graph,
        thickness,
        &crate::qp::solve,
    )
}

/// [`solve_clearing`] with another solver (for tests and for measuring), called once.
///
/// # Errors
///
/// As [`solve_clearing`], and whatever `solver` returns.
#[doc(hidden)]
pub fn solve_clearing_with(
    net: &PlacedNetwork,
    hm: &Heightmap,
    junctions: &Junctions,
    side: &[f64],
    graph: &RoadGraph,
    thickness: &dyn Fn(&Stretch) -> Option<f64>,
    solver: &dyn Fn(&Problem) -> Result<Vec<f64>, QpError>,
) -> Result<(Profiles, Junctions), ProfileError> {
    let crossings: Vec<model::Crossing> = over_crossings(graph)
        .iter()
        .filter_map(|c| {
            let upper = graph.stretches.get(c.upper)?;
            Some(model::Crossing {
                at: c.at,
                upper: upper.id.clone(),
                lower: graph.stretches.get(c.lower)?.id.clone(),
                thickness: thickness(upper)?,
            })
        })
        .collect();
    let mut model = model::build(net, hm, junctions, &apart_of(graph))?;
    model::add_clearances(&mut model, net, &crossings);
    let x = model::solve_x(&model, solver)?;
    let (mut profiles, out) = model::read(&model, net, junctions, side, &x)?;
    profiles.clearances = measure_clearances(net, &profiles.strips, graph, thickness);
    Ok((profiles, out))
}

/// The crossings of `graph` that roads are not tied near.
fn apart_of(graph: &RoadGraph) -> Vec<model::Apart> {
    over_crossings(graph)
        .iter()
        .filter_map(|c| {
            Some(model::Apart {
                at: c.at,
                upper: graph.stretches.get(c.upper)?.id.clone(),
                lower: graph.stretches.get(c.lower)?.id.clone(),
            })
        })
        .collect()
}

/// How many pairs of spots the rows of roads that share terrain cells are made from (for measuring).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NearbyPairs {
    /// The pairs of spots of roads that are close enough to share terrain cells.
    pub close: usize,
    /// Of those, the pairs that are the nearest of both spots.
    pub mutual: usize,
    /// The rows that were made (a pair is tied only in a run the ground lets it be).
    pub rows: usize,
}

/// The [`NearbyPairs`] of the problem [`solve_clearing`] builds for `net`.
///
/// For measuring (the `road-analysis` tool); not part of the library's interface.
///
/// # Errors
///
/// As [`solve_clearing`], without the solver.
#[doc(hidden)]
pub fn nearby_pairs(
    net: &PlacedNetwork,
    hm: &Heightmap,
    junctions: &Junctions,
    graph: &RoadGraph,
) -> Result<NearbyPairs, ProfileError> {
    Ok(model::build(net, hm, junctions, &apart_of(graph))?.pairs)
}

/// The grade ceilings of one chain of the problem [`solve_clearing`] builds (for measuring).
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct ChainCeilings {
    /// The roads of the chain, as indices into [`PlacedNetwork::roads`]: the `slots` of its [`RoadStrip`].
    pub roads: Vec<usize>,
    /// One entry per segment between two variables of the chain that the rows (G) hold: the joint of the strip
    /// where it starts and the one where it ends, its run (the grade is the difference of their heights over
    /// it) and its grade ceiling.
    pub segments: Vec<SegmentCeiling>,
}

/// The grade ceiling of one segment of a chain (see [`ChainCeilings`]).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentCeiling {
    /// The joint of the strip at the start of the segment.
    pub from: usize,
    /// The joint of the strip at its end.
    pub to: usize,
    /// The length the grade is measured over, in metres.
    pub run: f64,
    /// The grade ceiling.
    pub ceiling: f64,
}

/// The [`ChainCeilings`] of the problem [`solve_clearing`] builds for `net`: what a measured grade is held to.
///
/// For measuring (the `road-analysis` tool); not part of the library's interface. The model is built without the
/// crossings that roads are not tied near (no road graph): those change only the rows (N), never a ceiling.
///
/// # Errors
///
/// As [`solve_clearing`], without the solver.
#[doc(hidden)]
pub fn segment_ceilings(
    net: &PlacedNetwork,
    hm: &Heightmap,
    junctions: &Junctions,
) -> Result<Vec<ChainCeilings>, ProfileError> {
    Ok(model::build(net, hm, junctions, &[])?.ceilings())
}

/// How far from its line the placed road of a crossing may lie beyond its half width and still be the road
/// that the line crosses, in metres.
const CROSSING_REACH_M: f64 = 5.0;

/// The segment nearest `at` among `segments` (a key, its two ends and the half width of its road), within the half
/// width and [`CROSSING_REACH_M`] of it: its key and how far along it `at` projects, from 0 (its first end) to 1.
/// The first of two equally near segments wins. The clearance is measured and the rows that make it are built on
/// what this finds, so that the two cannot drift apart.
pub(crate) fn nearest_segment<K>(
    at: (f64, f64),
    segments: impl IntoIterator<Item = (K, (f64, f64), (f64, f64), f64)>,
) -> Option<(K, f64)> {
    let mut best: Option<(f64, K, f64)> = None;
    for (key, from, to, half_width) in segments {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let length2 = dx * dx + dy * dy;
        let t = if length2 > 1e-12 {
            (((at.0 - from.0) * dx + (at.1 - from.1) * dy) / length2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let off = (at.0 - from.0 - t * dx).hypot(at.1 - from.1 - t * dy);
        if off <= half_width + CROSSING_REACH_M && best.as_ref().is_none_or(|b| off < b.0) {
            best = Some((off, key, t));
        }
    }
    best.map(|(_, key, t)| (key, t))
}

/// The height of the surface of the segment of a strip nearest `at` among those that `wanted` takes (given the
/// strip and the segment): see [`nearest_segment`].
fn surface_near(
    strips: &[RoadStrip],
    at: (f64, f64),
    wanted: &dyn Fn(usize, usize) -> bool,
) -> Option<f64> {
    let segments = strips.iter().enumerate().flat_map(|(si, strip)| {
        strip
            .joints
            .windows(2)
            .enumerate()
            .filter_map(move |(segment, pair)| {
                let [from, to] = pair else { return None };
                wanted(si, segment).then_some((
                    (from.2, to.2),
                    (from.0, from.1),
                    (to.0, to.1),
                    strip.half_width,
                ))
            })
    });
    nearest_segment(at, segments).map(|((low, high), t)| low + t * (high - low))
}

/// Whether the segment of a strip is part of the stretch `id`: junction pieces carry the ids of both roads.
fn on_stretch(
    net: &PlacedNetwork,
    strips: &[RoadStrip],
    strip: usize,
    segment: usize,
    id: &str,
) -> bool {
    strips
        .get(strip)
        .and_then(|s| s.slots.get(segment))
        .and_then(|r| net.roads.get(*r))
        .is_some_and(|r| r.slot.stretch.split('|').any(|part| part == id))
}

/// The height of the surface of the road below `crossing`, as solved in `strips`; found by the lower stretch
/// of the crossing, not by what lies nearest.
fn lower_surface(
    net: &PlacedNetwork,
    strips: &[RoadStrip],
    graph: &RoadGraph,
    crossing: &OverCrossing,
) -> Option<f64> {
    let lower = graph.stretches.get(crossing.lower)?.id.as_str();
    surface_near(strips, crossing.at, &|si, k| {
        on_stretch(net, strips, si, k, lower)
    })
}

/// The clearance of the bridge of `crossing` over the road below, as solved in `strips`: from the surface of
/// the road to the underside of the deck, which hangs `thickness` below the deck's surface. The two are found by
/// the stretches of the crossing, not by what lies nearest (the approach of the bridge may end beside the
/// crossing). `None` if the bridge or the road below was not placed there.
fn clearance_at(
    net: &PlacedNetwork,
    strips: &[RoadStrip],
    graph: &RoadGraph,
    crossing: &OverCrossing,
    thickness: f64,
) -> Option<f64> {
    let upper = graph.stretches.get(crossing.upper)?.id.as_str();
    let deck = surface_near(strips, crossing.at, &|si, k| {
        on_stretch(net, strips, si, k, upper)
    })?;
    Some(deck - thickness - lower_surface(net, strips, graph, crossing)?)
}

/// The clearance under every bridge over a road of `graph` that is placed in `strips` (see [`over_crossings`]).
#[must_use]
pub fn measure_clearances(
    net: &PlacedNetwork,
    strips: &[RoadStrip],
    graph: &RoadGraph,
    thickness: &dyn Fn(&Stretch) -> Option<f64>,
) -> Vec<Clearance> {
    over_crossings(graph)
        .iter()
        .filter_map(|c| {
            let deck = graph.stretches.get(c.upper).and_then(thickness)?;
            clearance_at(net, strips, graph, c, deck).map(|reached| Clearance {
                at: c.at,
                reached,
                wanted: BRIDGE_CLEARANCE_M,
            })
        })
        .collect()
}

/// Two roads whose footprints share terrain vertices cannot differ in height by more than this.
const MATCH_M: f64 = 0.1;
/// Spacing of the points at which close roads are compared, in metres.
const SAMPLE_M: f64 = 3.0;
/// Roads that share terrain cells are not tied this close to a crossing of a bridge over a road, in metres: the
/// approaches rise to the deck right beside the road below.
const KEEP_APART_M: f64 = 50.0;

/// A point on a strip: between joint `segment` and the next, `t` of the way.
struct Spot {
    strip: usize,
    segment: usize,
    t: f64,
    x: f64,
    y: f64,
    along: f64,
}

fn spots(strips: &[RoadStrip]) -> Vec<Spot> {
    let mut spots = Vec::new();
    for (si, strip) in strips.iter().enumerate() {
        let mut along = 0.0;
        for (segment, pair) in strip.joints.windows(2).enumerate() {
            let [from, to] = pair else { continue };
            let length = (to.0 - from.0).hypot(to.1 - from.1);
            let is_bridge = strip.bridge.get(segment).copied().unwrap_or(false);
            if !is_bridge {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a sample count from a length"
                )]
                let count = ((length / SAMPLE_M).ceil() as usize).max(1);
                for k in 0..=count {
                    #[expect(clippy::cast_precision_loss, reason = "small counts")]
                    let t = k as f64 / count as f64;
                    spots.push(Spot {
                        strip: si,
                        segment,
                        t,
                        x: from.0 + t * (to.0 - from.0),
                        y: from.1 + t * (to.1 - from.1),
                        along: along + t * length,
                    });
                }
            }
            along += length;
        }
    }
    spots
}

/// Two spots of roads that are close enough to share terrain cells.
struct Pair {
    first: usize,
    second: usize,
    /// How far apart they are.
    distance: f64,
    /// The distance below which they cannot differ in height.
    gap: f64,
}

/// The pairs of spots of different roads, or of parts of one road that double back, that are close enough
/// to be compared.
fn close_pairs(spots: &[Spot], strips: &[RoadStrip], cell: f64) -> Vec<Pair> {
    let widest = strips.iter().map(|s| s.half_width).fold(0.0, f64::max);
    let reach = widest * 2.0 + 3.0 * cell + 12.0;
    if reach <= 0.0 {
        return Vec::new();
    }
    let bucket = |v: f64| {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a grid bucket of a map position"
        )]
        let b = (v / reach).floor() as i64;
        b
    };
    let mut grid: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
    for (i, spot) in spots.iter().enumerate() {
        grid.entry((bucket(spot.x), bucket(spot.y)))
            .or_default()
            .push(i);
    }
    let half_width = |strip: usize| strips.get(strip).map_or(0.0, |s| s.half_width);
    let mut pairs = Vec::new();
    for (first, a) in spots.iter().enumerate() {
        let (cx, cy) = (bucket(a.x), bucket(a.y));
        for gx in cx - 1..=cx + 1 {
            for gy in cy - 1..=cy + 1 {
                for &second in grid.get(&(gx, gy)).map_or(&[][..], Vec::as_slice) {
                    let Some(b) = spots.get(second).filter(|_| second > first) else {
                        continue;
                    };
                    let distance = (a.x - b.x).hypot(a.y - b.y);
                    let gap = half_width(a.strip) + half_width(b.strip) + 3.0 * cell;
                    // parts of one road that merely follow each other are not close
                    let follows =
                        a.strip == b.strip && (a.along - b.along).abs() <= 1.4 * distance + 3.0;
                    if distance <= gap + 12.0 && !follows {
                        pairs.push(Pair {
                            first,
                            second,
                            distance,
                            gap,
                        });
                    }
                }
            }
        }
    }
    pairs
}

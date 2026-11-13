// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Height profiles of the road chains.
//!
//! A road's height is not its terrain's: a profile follows the terrain but keeps to a grade limit, a limit on
//! how much the grade may change from one piece to the next (the number that makes a road feel bumpy), and
//! the height of whatever it meets. The terrain is then shaped to the profile, see [`crate::conform`].
//!
//! Where roads meet, the height is not solved but given: every junction has one height, taken from the original
//! ground (see [`crate::junctions`]), and a chain's variables that lie on a junction are fixed to it, the
//! centre of a junction piece in a through road as much as the end of a side road. A chain is then a row of
//! spans between fixed heights.
//!
//! Per chain in placement order: a smoothing solve (a banded system, solved directly) and clipping passes
//! alternate for a fixed 30 rounds, so that the result is deterministic. A chain end that lies on a road
//! placed earlier (within its half width, however far from that road's joints) takes that road's height
//! where they meet and keeps it, unless a junction holds it (above); an end that does not lie on another road
//! is free. Where that makes the
//! limits impossible, the ends win and a [`GradeExcess`] is reported, on the settled heights.
//!
//! Then roads that run closer together than the terrain grid can tell apart are brought to one height (the step
//! called settling): without that, the terrain under one of them is wrong however it is shaped. Settling
//! moves heights and solves again with the grade limits, so it cannot break them.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::Family;
use crate::conform::FILL_SLOPE_MAX;
use crate::heightmap::Heightmap;
use crate::junctions::{ARM_REACH_M, HOLD_M, JunctionKind, Junctions};
use crate::limits::{drive_grade, grade_aim, grade_ceiling, min_vertical_radius};
use crate::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad};

/// The steepest grade a road may carry where another road meets it (rise over run).
pub const MEETING_GRADE: f64 = 0.06;

/// How far a road may be from the terrain before it gives up its grade aim and becomes steeper.
pub const RELAX_OFF_M: f64 = 4.0;

/// Rounds of smoothing and clipping.
const ROUNDS: usize = 30;
/// The spacing at which the second difference of the smoothing is (1, -2, 1).
const REFERENCE_M: f64 = 12.5;
/// The smallest spacing the second difference is built on.
const MIN_STENCIL_M: f64 = 1.0;
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

/// All profiles.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Profiles {
    /// One strip per chain.
    pub strips: Vec<RoadStrip>,
    /// Where the limits are exceeded.
    pub excesses: Vec<GradeExcess>,
}

/// What a chain solved to, kept for [`settle`].
struct Solved {
    /// The variable of each joint of the strip.
    joint_variable: Vec<usize>,
    /// The height of each variable.
    heights: Vec<f64>,
    /// The position of each variable.
    at: Vec<(f64, f64)>,
    /// The distance from each variable to the next.
    spacing: Vec<f64>,
    limits: Limits,
    /// The meeting each fixed variable lies on.
    joins: Vec<(usize, Meeting)>,
    /// The height a junction gives each variable; [`settle`] never moves these.
    held: Vec<Option<f64>>,
    /// The heights as first solved: [`settle`] moves a variable only so far from them.
    origin: Vec<f64>,
    /// The steepest grade a vehicle manages on this road, which no piece between two junction heights may exceed.
    hard: f64,
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

/// The variables of a chain: one per joint, except that a junction has one height along its length.
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

fn chain_of(net: &PlacedNetwork, roads: &[usize]) -> Option<Chain> {
    let first = net.roads.get(*roads.first()?)?;
    let mut joints = vec![(first.start.x, first.start.y, 0)];
    let mut at = vec![(first.start.x, first.start.y)];
    let mut spacing = Vec::new();
    let mut deck: Vec<Option<DeckEnds>> = vec![None];
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
            joints.push((r.end.x, r.end.y, at.len()));
            at.push((r.end.x, r.end.y));
            deck.push(None);
            spacing.push(length);
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

    /// Whether an end of a chain other than `roads` lies within `reach` of the segment `a`-`b`.
    fn any_near(
        &self,
        net: &PlacedNetwork,
        roads: &[usize],
        a: (f64, f64),
        b: (f64, f64),
        reach: f64,
    ) -> bool {
        let others = |(index, point): &ChainEnd| {
            net.chains
                .get(*index)
                .is_some_and(|c| c.as_slice() != roads)
                && point_segment_distance(*point, a, b) <= reach
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

/// Whether the first or last point of another chain lies within the road's half width plus 12.5 m of it.
fn meets_another_chain(
    net: &PlacedNetwork,
    ends: &ChainEnds,
    roads: &[usize],
    road: &PlacedRoad,
) -> bool {
    let reach = road.width / 2.0 + 12.5;
    ends.any_near(
        net,
        roads,
        (road.start.x, road.start.y),
        (road.end.x, road.end.y),
        reach,
    )
}

/// The grade limit of each segment of a chain: the family's aim, lowered at a meeting (not between two junction
/// heights) and in a tight bend.
fn segment_grades(
    net: &PlacedNetwork,
    ends: &ChainEnds,
    roads: &[usize],
    base: f64,
    held: &[Option<f64>],
) -> Vec<f64> {
    let mut limits = Vec::new();
    let first_held = held.iter().position(Option::is_some);
    let last_held = held.iter().rposition(Option::is_some);
    for &index in roads {
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
        // make (`repair` kept the links under the ceiling, and a free joint between two held ones is part of
        // the span): the meeting grade is for the approach from a free end to a junction
        let between_junctions = first_held
            .zip(last_held)
            .is_some_and(|(first, last)| first <= limits.len() && limits.len() < last);
        if !track
            && !between_junctions
            && (road.kind == PlacedKind::Junction || meets_another_chain(net, ends, roads, road))
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

/// The bands of `W + λ D₂ᵀD₂` (the main diagonal, the first and the second off-diagonal) and the
/// right-hand side `W t`.
struct Bands {
    main: Vec<f64>,
    near: Vec<f64>,
    far: Vec<f64>,
    rhs: Vec<f64>,
}

fn bands(target: &[f64], fixed: &[bool], spacing: &[f64]) -> Bands {
    let count = target.len();
    let mut bands = Bands {
        main: vec![0.0; count],
        near: vec![0.0; count],
        far: vec![0.0; count],
        rhs: vec![0.0; count],
    };
    for (i, (&goal, &is_fixed)) in target.iter().zip(fixed).enumerate() {
        let weight = if is_fixed { 1e6 } else { 1.0 };
        if let (Some(main), Some(rhs)) = (bands.main.get_mut(i), bands.rhs.get_mut(i)) {
            *main += weight;
            *rhs += weight * goal;
        }
    }
    // each row (1, -2, 1) of the second difference adds its outer product
    for centre in 1..count.saturating_sub(1) {
        // the second difference of three joints that are not equally far apart, scaled so that it is
        // (1, -2, 1) for the usual spacing of [`REFERENCE_M`]: a pair of joints a metre apart is nearly one
        // joint, not a free point that may be a few metres off the line through its neighbours
        let before = spacing
            .get(centre - 1)
            .copied()
            .unwrap_or(REFERENCE_M)
            .max(MIN_STENCIL_M);
        let after = spacing
            .get(centre)
            .copied()
            .unwrap_or(REFERENCE_M)
            .max(MIN_STENCIL_M);
        let scale = 2.0 * REFERENCE_M * REFERENCE_M;
        let row = [
            scale / (before * (before + after)),
            -scale / (before * after),
            scale / (after * (before + after)),
        ];
        for (first, &first_weight) in row.iter().enumerate() {
            for (second, &second_weight) in row.iter().enumerate().skip(first) {
                let at = centre - 1 + first;
                let value = SMOOTHNESS * first_weight * second_weight;
                let band = match second - first {
                    0 => bands.main.get_mut(at),
                    1 => bands.near.get_mut(at),
                    _ => bands.far.get_mut(at),
                };
                if let Some(slot) = band {
                    *slot += value;
                }
            }
        }
    }
    bands
}

fn at(values: &[f64], index: usize) -> f64 {
    values.get(index).copied().unwrap_or(0.0)
}

fn put(values: &mut [f64], index: usize, value: f64) {
    if let Some(slot) = values.get_mut(index) {
        *slot = value;
    }
}

/// Solves the banded system by an `LDLᵀ` factorisation.
fn solve_bands(bands: &Bands) -> Vec<f64> {
    let count = bands.main.len();
    // L[i][i-1] and L[i][i-2], and the diagonal D
    let mut low1 = vec![0.0; count];
    let mut low2 = vec![0.0; count];
    let mut diag = vec![0.0; count];
    for i in 0..count {
        if i >= 2 {
            put(&mut low2, i, at(&bands.far, i - 2) / at(&diag, i - 2));
        }
        if i >= 1 {
            let mut value = at(&bands.near, i - 1);
            if i >= 2 {
                value -= at(&low2, i) * at(&diag, i - 2) * at(&low1, i - 1);
            }
            put(&mut low1, i, value / at(&diag, i - 1));
        }
        let mut value = at(&bands.main, i);
        if i >= 1 {
            value -= at(&low1, i).powi(2) * at(&diag, i - 1);
        }
        if i >= 2 {
            value -= at(&low2, i).powi(2) * at(&diag, i - 2);
        }
        put(&mut diag, i, value);
    }
    let mut forward = vec![0.0; count];
    for i in 0..count {
        let mut value = at(&bands.rhs, i);
        if i >= 1 {
            value -= at(&low1, i) * at(&forward, i - 1);
        }
        if i >= 2 {
            value -= at(&low2, i) * at(&forward, i - 2);
        }
        put(&mut forward, i, value);
    }
    let mut solution = vec![0.0; count];
    for i in (0..count).rev() {
        let mut value = at(&forward, i) / at(&diag, i);
        if i + 1 < count {
            value -= at(&low1, i + 1) * at(&solution, i + 1);
        }
        if i + 2 < count {
            value -= at(&low2, i + 2) * at(&solution, i + 2);
        }
        put(&mut solution, i, value);
    }
    solution
}

fn smooth(target: &[f64], fixed: &[bool], spacing: &[f64]) -> Vec<f64> {
    solve_bands(&bands(target, fixed, spacing))
}

/// The grade limits of a chain: one grade per segment between two variables, and the smallest vertical
/// radius its joints may have.
#[derive(Clone, Debug, PartialEq)]
struct Limits {
    grade: Vec<f64>,
    radius: f64,
}

/// One clipping pass over the grades, forward (from the start) or backward; fixed heights stay.
fn clip(heights: &mut [f64], spacing: &[f64], fixed: &[bool], limits: &Limits, forward: bool) {
    let count = heights.len();
    if count < 2 {
        return;
    }
    let segments: Vec<usize> = if forward {
        (0..count - 1).collect()
    } else {
        (0..count - 1).rev().collect()
    };
    let mut previous: Option<(f64, f64)> = None;
    for segment in segments {
        let length = spacing.get(segment).copied().unwrap_or(1.0).max(0.5);
        let (from, to) = (
            heights.get(segment).copied().unwrap_or(0.0),
            heights.get(segment + 1).copied().unwrap_or(0.0),
        );
        let grade_limit = limits.grade.get(segment).copied().unwrap_or(f64::INFINITY);
        let (mut low, mut high) = (-grade_limit, grade_limit);
        if let Some((before, before_length)) = previous {
            let change = f64::midpoint(before_length, length) / limits.radius;
            low = low.max(before - change);
            high = high.min(before + change);
            if low > high {
                let middle = f64::midpoint(low, high);
                (low, high) = (middle, middle);
            }
        }
        let slope = ((to - from) / length).clamp(low, high);
        let (moved, anchor) = if forward {
            (segment + 1, from)
        } else {
            (segment, to)
        };
        if fixed.get(moved).copied().unwrap_or(true) {
            // a fixed height stays, whatever grade that makes. The grade that leads into it is not kept (the
            // free joints before it are what the other pass adjusts), but the change of grade across it is
            // limited like anywhere else, from the grade the limit allows: an unclamped break would carry on as
            // a long crest or dip
            previous = Some((
                ((to - from) / length).clamp(-grade_limit, grade_limit),
                length,
            ));
            continue;
        }
        let new = if forward {
            anchor + slope * length
        } else {
            anchor - slope * length
        };
        if let Some(height) = heights.get_mut(moved) {
            *height = new;
        }
        previous = Some((slope, length));
    }
}

fn solve_chain(
    targets: &[f64],
    spacing: &[f64],
    fixed_height: &[Option<f64>],
    limits: &Limits,
) -> Vec<f64> {
    let fixed: Vec<bool> = fixed_height.iter().map(Option::is_some).collect();
    let mut goal = targets.to_vec();
    for (value, pinned) in goal.iter_mut().zip(fixed_height) {
        if let Some(height) = pinned {
            *value = *height;
        }
    }
    let mut heights = goal.clone();
    for _ in 0..ROUNDS {
        let blended: Vec<f64> = goal
            .iter()
            .zip(&heights)
            .zip(fixed_height)
            .map(|((wanted, current), pinned)| {
                pinned.unwrap_or_else(|| 0.5 * wanted + 0.5 * current)
            })
            .collect();
        heights = smooth(&blended, &fixed, spacing);
        clip(&mut heights, spacing, &fixed, limits, true);
        clip(&mut heights, spacing, &fixed, limits, false);
    }
    clip(&mut heights, spacing, &fixed, limits, true);
    // the smoothing holds a fixed height by weight, not exactly
    for (height, pinned) in heights.iter_mut().zip(fixed_height) {
        if let Some(pinned) = pinned {
            *height = *pinned;
        }
    }
    heights
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

/// What [`profile_chain`] looks things up in instead of walking every chain and strip.
struct Index {
    ends: ChainEnds,
    /// [`meeting_bounds`] of each strip solved so far.
    strips: Vec<[f64; 4]>,
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

/// Which rule holds a variable of a chain to a junction (for measuring, see [`chain_holds`]).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldRule {
    /// The variable lies in the junction's patch.
    Patch,
    /// A variable of a junction piece.
    Piece,
    /// An end of the chain.
    End,
    /// Up to eight variables inward from a held end, on the other road's level strip.
    Inward,
    /// A variable of the piece that passes the junction within [`ARM_REACH_M`].
    Arm,
}

/// The junction that holds a variable, and why (for measuring, see [`chain_holds`]).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hold {
    /// The rule.
    pub rule: HoldRule,
    /// Index into [`Junctions::points`].
    pub junction: usize,
    /// The height it gives.
    pub height: f64,
}

/// Sets the junction that holds variable `var`.
fn hold(
    held: &mut [Option<Hold>],
    var: usize,
    junction: usize,
    junctions: &Junctions,
    rule: HoldRule,
) {
    if let (Some(slot), Some(j)) = (held.get_mut(var), junctions.points.get(junction)) {
        *slot = Some(Hold {
            rule,
            junction,
            height: j.height,
        });
    }
}

/// The holds of the joints of the chain `roads`, one per joint of the strip as `solve_profiles` builds it (the
/// joints of a bridge deck share one variable), with the position of each joint.
///
/// For measuring (the `road-analysis` tool and the tests); not part of the library's interface.
#[doc(hidden)]
#[must_use]
pub fn chain_holds(
    net: &PlacedNetwork,
    roads: &[usize],
    cell: f64,
    junctions: &Junctions,
) -> Vec<((f64, f64), Option<Hold>)> {
    holds_of(net, roads, cell, junctions, true)
}

/// [`chain_holds`] without the check that the chain belongs to the junction, to count what the check removes.
#[doc(hidden)]
#[must_use]
pub fn chain_holds_unchecked(
    net: &PlacedNetwork,
    roads: &[usize],
    cell: f64,
    junctions: &Junctions,
) -> Vec<((f64, f64), Option<Hold>)> {
    holds_of(net, roads, cell, junctions, false)
}

fn holds_of(
    net: &PlacedNetwork,
    roads: &[usize],
    cell: f64,
    junctions: &Junctions,
    check: bool,
) -> Vec<((f64, f64), Option<Hold>)> {
    let Some(chain) = chain_of(net, roads) else {
        return Vec::new();
    };
    let half_width = roads
        .first()
        .and_then(|i| net.roads.get(*i))
        .map_or(0.0, |r| r.width / 2.0);
    let held = junction_heights(net, &chain, roads, half_width, cell, junctions, check);
    chain
        .joints
        .iter()
        .map(|j| ((j.0, j.1), held.get(j.2).copied().flatten()))
        .collect()
}

/// The stretch ids of the roads of a chain, junction pieces' `a|b` ids split.
fn stretch_ids(net: &PlacedNetwork, roads: &[usize]) -> BTreeSet<String> {
    roads
        .iter()
        .filter_map(|i| net.roads.get(*i))
        .flat_map(|r| r.slot.stretch.split('|').map(str::to_string))
        .collect()
}

/// The runs of consecutive variables held to one junction by `Patch`, cut down to the one variable of each run
/// that lies nearest the junction: a road that passes a junction is held at one joint, not along its length.
fn nearest_of_runs(candidates: &[(usize, usize, f64)]) -> Vec<(usize, usize)> {
    let mut runs: Vec<Vec<(usize, usize, f64)>> = Vec::new();
    for &candidate in candidates {
        match runs.last_mut() {
            Some(run)
                if run
                    .last()
                    .is_some_and(|l| l.1 == candidate.1 && l.0 + 1 == candidate.0) =>
            {
                run.push(candidate);
            }
            _ => runs.push(vec![candidate]),
        }
    }
    runs.iter()
        .filter_map(|run| run.iter().min_by(|a, b| a.2.total_cmp(&b.2)))
        .map(|&(var, junction, _)| (var, junction))
        .collect()
}

/// The height a junction gives each variable of a chain, by the least that holds the junction's height where
/// the road has it:
/// - a junction piece holds its two variables (it is level along its length);
/// - a chain that passes a junction is held at the one joint nearest the junction point (`Patch`), or, where
///   no joint lies in the patch, at the one joint of the nearest piece that lies nearest the junction (`Arm`,
///   within [`ARM_REACH_M`]);
/// - a chain that ends at a junction is held at its end (a side road ends at the port of a piece, beyond the
///   through road's edge), and inward only as far as its joints lie on the other road's level strip, up to eight.
///
/// A chain is held only to the junctions it belongs to (`Junction::stretches`), unless `check` is false (for
/// measuring what the check removes).
fn junction_heights(
    net: &PlacedNetwork,
    chain: &Chain,
    roads: &[usize],
    half_width: f64,
    cell: f64,
    junctions: &Junctions,
    check: bool,
) -> Vec<Option<Hold>> {
    let count = chain.at.len();
    let ids = stretch_ids(net, roads);
    let member = |index: usize| {
        !check
            || junctions
                .points
                .get(index)
                .is_some_and(|j| j.stretches.is_empty() || !j.stretches.is_disjoint(&ids))
    };
    // the nearest junction to a point that this chain belongs to, within the reach beyond its patch
    let nearest = |point: (f64, f64), reach: f64| {
        junctions
            .points
            .iter()
            .enumerate()
            .filter(|(i, _)| member(*i))
            .map(|(i, j)| (i, (j.at.0 - point.0).hypot(j.at.1 - point.1), j.patch_m))
            .filter(|(_, d, patch)| *d <= patch + reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, d, _)| (i, d))
    };
    let mut held: Vec<Option<Hold>> = vec![None; count];
    for (k, &road) in roads.iter().enumerate() {
        let Some(road) = net
            .roads
            .get(road)
            .filter(|r| r.kind == PlacedKind::Junction)
        else {
            continue;
        };
        let centre = (
            f64::midpoint(road.start.x, road.end.x),
            f64::midpoint(road.start.y, road.end.y),
        );
        if let (Some((junction, _)), Some(from), Some(to)) = (
            nearest(centre, HOLD_M),
            chain.joints.get(k),
            chain.joints.get(k + 1),
        ) {
            hold(&mut held, from.2, junction, junctions, HoldRule::Piece);
            hold(&mut held, to.2, junction, junctions, HoldRule::Piece);
        }
    }
    // a junction that a variable lies in the patch of: the variable nearest the junction of each run
    let candidates: Vec<(usize, usize, f64)> = chain
        .at
        .iter()
        .enumerate()
        .filter(|(var, _)| held.get(*var).is_some_and(Option::is_none))
        .filter_map(|(var, point)| nearest(*point, 0.0).map(|(j, d)| (var, j, d)))
        .collect();
    for (var, junction) in nearest_of_runs(&candidates) {
        hold(&mut held, var, junction, junctions, HoldRule::Patch);
    }
    // the ends, and inward from them while the joints lie on the surface of the other road
    for (side, end) in [0, count.saturating_sub(1)].into_iter().enumerate() {
        let Some((junction_index, _)) = chain.at.get(end).and_then(|point| nearest(*point, HOLD_M))
        else {
            continue;
        };
        let Some(junction) = junctions.points.get(junction_index) else {
            continue;
        };
        // the ground the other road's level strip takes: its half width and 1.5 cells beyond (`conform`), on a
        // junction piece counted from the end of the piece
        let reach = if junction.kind == JunctionKind::Piece {
            junction.patch_m + half_width + 1.5 * cell
        } else {
            junction.patch_m + 1.5 * cell
        };
        let mut var = end;
        for _ in 0..8 {
            let on_surface = chain
                .at
                .get(var)
                .is_some_and(|p| (p.0 - junction.at.0).hypot(p.1 - junction.at.1) <= reach);
            if var != end && !on_surface {
                break;
            }
            if held.get(var).is_some_and(Option::is_none) {
                let rule = if var == end {
                    HoldRule::End
                } else {
                    HoldRule::Inward
                };
                hold(&mut held, var, junction_index, junctions, rule);
            }
            let following = if side == 0 {
                var.checked_add(1)
            } else {
                var.checked_sub(1)
            };
            match following {
                Some(step) if step < count => var = step,
                _ => break,
            }
        }
    }
    let members: Vec<usize> = (0..junctions.points.len()).filter(|i| member(*i)).collect();
    hold_beside(chain, junctions, &members, &mut held);
    held
}

/// A junction beside a piece, where no variable lies in its patch (a road that ends on this one without a
/// crossing, or an arm that passes the crossing at up to [`ARM_REACH_M`]): the nearer variable of the nearest
/// piece holds its height. `members` are the junctions the chain belongs to; a junction that already holds a
/// variable of the chain is done.
fn hold_beside(chain: &Chain, junctions: &Junctions, members: &[usize], held: &mut [Option<Hold>]) {
    let (mut low, mut high) = (
        (f64::INFINITY, f64::INFINITY),
        (f64::NEG_INFINITY, f64::NEG_INFINITY),
    );
    for p in &chain.at {
        low = (low.0.min(p.0), low.1.min(p.1));
        high = (high.0.max(p.0), high.1.max(p.1));
    }
    let is_deck = |v: usize| chain.deck.get(v).is_some_and(Option::is_some);
    for &junction in members {
        let Some(point) = junctions.points.get(junction).map(|j| j.at) else {
            continue;
        };
        // a box test first: most junctions are nowhere near the chain
        if point.0 < low.0 - ARM_REACH_M
            || point.0 > high.0 + ARM_REACH_M
            || point.1 < low.1 - ARM_REACH_M
            || point.1 > high.1 + ARM_REACH_M
            || held.iter().flatten().any(|h| h.junction == junction)
        {
            continue;
        }
        let free = |v: usize| held.get(v).is_some_and(Option::is_none) && !is_deck(v);
        let beside = chain
            .at
            .windows(2)
            .enumerate()
            .filter_map(|(var, pair)| {
                let [from, to] = pair else { return None };
                (free(var) && free(var + 1))
                    .then(|| (point_segment_distance(point, *from, *to), var, *from, *to))
            })
            .filter(|(distance, ..)| *distance <= ARM_REACH_M)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, var, from, to)) = beside {
            let nearer = if (from.0 - point.0).hypot(from.1 - point.1)
                <= (to.0 - point.0).hypot(to.1 - point.1)
            {
                var
            } else {
                var + 1
            };
            hold(held, nearer, junction, junctions, HoldRule::Arm);
        }
    }
}

/// A span between two fixed heights that needs this share of the grade limit or more has no room to follow the
/// ground.
const TIGHT_SPAN: f64 = 0.8;

/// For the spans between two consecutive fixed variables that need most of their grade limit, the targets of the
/// variables between them become the straight line between the two heights. `repair` brings a link to its
/// ceiling and no further, so such a span can only be solved by a profile of nearly that grade everywhere; with
/// the ground as target the smoothing lags behind it and the clipping ends in one short piece with a grade of
/// several hundred percent. Returns whether a span needs more than its limit, so that the limit is to be relaxed.
fn ramp_targets(
    targets: &mut [f64],
    fixed: &[Option<f64>],
    spacing: &[f64],
    limits: &[f64],
) -> bool {
    let mut infeasible = false;
    let mut previous: Option<(usize, f64)> = None;
    for (var, height) in fixed.iter().enumerate() {
        let Some(height) = height else { continue };
        if let Some((from, low)) = previous {
            let lengths: Vec<f64> = spacing
                .get(from..var)
                .map_or_else(Vec::new, |s| s.iter().map(|l| l.max(0.5)).collect());
            let run: f64 = lengths.iter().sum();
            let limit = limits
                .get(from..var)
                .and_then(|l| l.iter().copied().reduce(f64::min))
                .unwrap_or(f64::INFINITY);
            let required = (height - low).abs() / run.max(0.5);
            infeasible |= required > limit;
            if var > from + 1 && required >= TIGHT_SPAN * limit {
                let mut along = 0.0;
                for (offset, length) in lengths
                    .iter()
                    .enumerate()
                    .take(lengths.len().saturating_sub(1))
                {
                    along += length;
                    if let Some(target) = targets.get_mut(from + offset + 1) {
                        *target = low + (height - low) * along / run;
                    }
                }
            }
        }
        previous = Some((var, *height));
    }
    infeasible
}

/// What a chain's variables are held to: the heights of the junctions, the heights of the roads its ends meet
/// (a junction beats a meeting), and the meetings.
fn fixed_heights(
    net: &PlacedNetwork,
    chain: &Chain,
    roads: &[usize],
    hm: &Heightmap,
    strips: &[RoadStrip],
    index: &Index,
    junctions: &Junctions,
) -> Option<FixedHeights> {
    let cell = f64::from(hm.cell_m);
    let half_width = net.roads.get(*roads.first()?)?.width / 2.0;
    let holds = junction_heights(net, chain, roads, half_width, cell, junctions, true);
    let held: Vec<Option<f64>> = holds.iter().map(|h| h.map(|h| h.height)).collect();
    let (mut fixed, meetings) = end_joins(chain, strips, &index.strips, cell, &held);
    for (slot, height) in fixed.iter_mut().zip(&held) {
        if height.is_some() {
            *slot = *height;
        }
    }
    Some((held, fixed, meetings))
}

/// The junction heights, all fixed heights, and the meetings of a chain's variables.
type FixedHeights = (Vec<Option<f64>>, Vec<Option<f64>>, Vec<(usize, Meeting)>);

/// The profile of one chain.
fn profile_chain(
    net: &PlacedNetwork,
    index: &Index,
    roads: &[usize],
    hm: &Heightmap,
    strips: &[RoadStrip],
    junctions: &Junctions,
) -> Option<(RoadStrip, Solved)> {
    let chain = chain_of(net, roads)?;
    let first = net.roads.get(*roads.first()?)?;
    let family = first.family;
    let spacing = chain.spacing.clone();
    let radius = min_vertical_radius(family);
    let count = chain.at.len();
    let ground = |point: (f64, f64)| f64::from(hm.sample(point.0, point.1).height);
    let targets: Vec<f64> = chain
        .at
        .iter()
        .zip(&chain.deck)
        .map(|(point, deck)| match deck {
            // a deck is held to the ground at its abutments, not to the valley floor under it
            Some((from, to)) => f64::midpoint(ground(*from), ground(*to)),
            None => ground(*point),
        })
        .collect();
    let (held, fixed, meetings) = fixed_heights(net, &chain, roads, hm, strips, index, junctions)?;
    let aim = grade_aim(family);
    let ceiling = grade_ceiling(family);
    let mut solved: Option<(Vec<f64>, Limits)> = None;
    for base in [aim, f64::midpoint(aim, ceiling), ceiling] {
        let limits = Limits {
            grade: segment_grades(net, &index.ends, roads, base, &held),
            radius,
        };
        let mut targets = targets.clone();
        let tight = ramp_targets(&mut targets, &fixed, &spacing, &limits.grade);
        let heights = if count == 1 || fixed.iter().all(Option::is_some) {
            targets
                .iter()
                .zip(&fixed)
                .map(|(target, pinned)| pinned.unwrap_or(*target))
                .collect()
        } else {
            solve_chain(&targets, &spacing, &fixed, &limits)
        };
        let off = tight
            || heights
                .iter()
                .zip(&targets)
                .zip(&fixed)
                .zip(&chain.deck)
                .any(|(((height, target), pinned), deck)| {
                    pinned.is_none() && deck.is_none() && (height - target).abs() > RELAX_OFF_M
                });
        solved = Some((heights, limits));
        if !off {
            break;
        }
    }
    let (heights, limits) = solved?;
    let joints: Vec<(f64, f64, f64)> = chain
        .joints
        .iter()
        .map(|joint| {
            (
                joint.0,
                joint.1,
                heights.get(joint.2).copied().unwrap_or(0.0),
            )
        })
        .collect();
    let strip = RoadStrip {
        slots: roads.to_vec(),
        joints,
        bridge: roads
            .iter()
            .map(|i| {
                net.roads
                    .get(*i)
                    .is_some_and(|r| r.kind == PlacedKind::Bridge)
            })
            .collect(),
        junction: roads
            .iter()
            .map(|i| {
                net.roads
                    .get(*i)
                    .is_some_and(|r| r.kind == PlacedKind::Junction)
            })
            .collect(),
        half_width: first.width / 2.0,
        rank: rank(family),
    };
    let variables = Solved {
        joint_variable: chain.joints.iter().map(|j| j.2).collect(),
        at: chain.at,
        joins: meetings,
        held,
        origin: heights.clone(),
        heights,
        spacing,
        limits,
        hard: drive_grade(family),
    };
    Some((strip, variables))
}

/// Where a solved chain breaks its limits: the grades and grade changes over the limit, on the
/// heights it is holding now.
fn excesses_of(chain: &Solved) -> Vec<GradeExcess> {
    let grades: Vec<(f64, f64)> = chain
        .heights
        .windows(2)
        .zip(&chain.spacing)
        .map(|(pair, length)| match pair {
            [from, to] => ((to - from) / length.max(0.5), *length),
            _ => (0.0, 0.5),
        })
        .collect();
    let mut excesses = Vec::new();
    for (index, (grade, length)) in grades.iter().enumerate() {
        let at = chain.at.get(index).copied().unwrap_or_default();
        let limit = chain
            .limits
            .grade
            .get(index)
            .copied()
            .unwrap_or(f64::INFINITY);
        if grade.abs() > limit + 1e-6 {
            excesses.push(GradeExcess {
                at,
                kind: GradeExcessKind::Grade,
                grade: *grade,
                limit,
            });
        }
        if let Some((before, before_length)) = index.checked_sub(1).and_then(|b| grades.get(b)) {
            let run = f64::midpoint(*before_length, *length);
            let change = (grade - before).abs() / run.max(0.5);
            let change_limit = 1.0 / chain.limits.radius;
            if change > change_limit + 1e-6 {
                excesses.push(GradeExcess {
                    at,
                    kind: GradeExcessKind::VerticalBend,
                    grade: change,
                    limit: change_limit,
                });
            }
        }
    }
    excesses
}

/// Computes the profile of every chain of `net`, in placement order, on the terrain `hm`, with the heights
/// of `junctions` where roads meet.
#[must_use]
pub fn solve_profiles(net: &PlacedNetwork, hm: &Heightmap, junctions: &Junctions) -> Profiles {
    let mut out = Profiles::default();
    let mut variables: Vec<Solved> = Vec::new();
    let mut index = Index {
        ends: ChainEnds::new(net),
        strips: Vec::new(),
    };
    for roads in &net.chains {
        let Some((strip, vars)) = profile_chain(net, &index, roads, hm, &out.strips, junctions)
        else {
            continue;
        };
        index.strips.push(meeting_bounds(&strip));
        out.strips.push(strip);
        variables.push(vars);
    }
    settle(&mut out.strips, &mut variables, hm.cell_m);
    // what is over the limits is measured on the heights that are actually used, after settling
    out.excesses = variables.iter().flat_map(excesses_of).collect();
    out
}

/// Two roads whose footprints share terrain vertices cannot differ in height by more than this.
const MATCH_M: f64 = 0.1;
/// Spacing of the points at which close roads are compared, in metres.
const SAMPLE_M: f64 = 3.0;
/// Rounds of [`settle`].
const SETTLE_ROUNDS: usize = 10;
/// How far [`settle`] may move a road from where it was first solved; roads that differ by more cross at
/// different levels or are a conflict.
const MAX_SETTLE_M: f64 = 5.0;

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

/// The two variables that carry a spot, with their weights; one variable that carries both ends takes all.
fn carriers(chains: &[Solved], spot: &Spot) -> [(usize, f64); 2] {
    let var = |joint: usize| {
        chains
            .get(spot.strip)
            .and_then(|c| c.joint_variable.get(joint))
            .copied()
            .unwrap_or(0)
    };
    let parts = [
        (var(spot.segment), 1.0 - spot.t),
        (var(spot.segment + 1), spot.t),
    ];
    if parts[0].0 == parts[1].0 {
        [(parts[0].0, 1.0), (parts[0].0, 0.0)]
    } else {
        parts
    }
}

fn spot_height(chains: &[Solved], spot: &Spot) -> f64 {
    carriers(chains, spot)
        .iter()
        .map(|&(v, w)| {
            w * chains
                .get(spot.strip)
                .and_then(|c| c.heights.get(v))
                .copied()
                .unwrap_or(0.0)
        })
        .sum()
}

/// What each variable is asked to do by the pairs that are too far apart in height: the weighted sum of the
/// shifts and the sum of the weights, per (chain, variable).
fn wanted_shifts(
    pairs: &[Pair],
    spots: &[Spot],
    strips: &[RoadStrip],
    chains: &[Solved],
    cell: f64,
) -> BTreeMap<(usize, usize), (f64, f64)> {
    let mut wanted: BTreeMap<(usize, usize), (f64, f64)> = BTreeMap::new();
    let importance = |strip: usize| f64::from(strips.get(strip).map_or(0, |s| s.rank)) + 1.0;
    for pair in pairs {
        let (Some(a), Some(b)) = (spots.get(pair.first), spots.get(pair.second)) else {
            continue;
        };
        let (ha, hb) = (spot_height(chains, a), spot_height(chains, b));
        let allowed = if pair.distance < pair.gap {
            MATCH_M
        } else {
            MATCH_M + (pair.distance - pair.gap + cell) * FILL_SLOPE_MAX
        };
        let excess = (ha - hb).abs() - allowed;
        if excess <= 0.0 {
            continue;
        }
        // the less important road moves more
        let (ra, rb) = (importance(a.strip), importance(b.strip));
        let sign = (ha - hb).signum();
        for (spot, shift) in [
            (a, -sign * excess * rb / (ra + rb)),
            (b, sign * excess * ra / (ra + rb)),
        ] {
            let parts = carriers(chains, spot);
            let norm: f64 = parts.iter().map(|p| p.1 * p.1).sum();
            for (var, w) in parts {
                if w > 1e-9 {
                    let slot = wanted.entry((spot.strip, var)).or_insert((0.0, 0.0));
                    slot.0 += w * (w * shift / norm);
                    slot.1 += w;
                }
            }
        }
    }
    wanted
}

/// The height a meeting points at, from the heights the chain that owns it holds now.
fn joined_height(chains: &[Solved], meet: &Meeting) -> Option<f64> {
    let owner = chains.get(meet.owner)?;
    let from = owner
        .joint_variable
        .get(meet.segment)
        .and_then(|var| owner.heights.get(*var).copied());
    let to = owner
        .joint_variable
        .get(meet.segment + 1)
        .and_then(|var| owner.heights.get(*var).copied());
    Some(from? + meet.t * (to? - from?))
}

/// Moves what was asked for into the targets of the chains and solves every chain that was touched again,
/// so that the grade limits and the smoothness are kept and the road does not wander off the terrain. Only
/// an end that meets another road is fixed to it; every other height, ends included, is free to be solved.
fn resolve(wanted: BTreeMap<(usize, usize), (f64, f64)>, chains: &mut [Solved]) {
    let mut touched: BTreeSet<usize> = BTreeSet::new();
    for ((strip, var), (sum, weight)) in wanted {
        if let Some(chain) = chains.get_mut(strip)
            && chain.held.get(var).is_none_or(Option::is_none)
            && let (Some(h), Some(&base)) = (chain.heights.get_mut(var), chain.origin.get(var))
        {
            *h = (*h + sum / weight.max(1e-9)).clamp(base - MAX_SETTLE_M, base + MAX_SETTLE_M);
            touched.insert(strip);
        }
    }
    for strip in touched {
        let pins: Vec<(usize, Option<f64>)> = chains.get(strip).map_or_else(Vec::new, |chain| {
            chain
                .joins
                .iter()
                .map(|(var, meet)| (*var, joined_height(chains, meet)))
                .collect()
        });
        let Some(chain) = chains.get_mut(strip) else {
            continue;
        };
        let last = chain.heights.len().saturating_sub(1);
        let mut pinned = vec![None; chain.heights.len()];
        for (var, height) in pins {
            if let (Some(slot), Some(height)) = (pinned.get_mut(var), height) {
                *slot = Some(height);
            }
        }
        // a junction's height beats a meeting's
        for (slot, height) in pinned.iter_mut().zip(&chain.held) {
            if height.is_some() {
                *slot = *height;
            }
        }
        // An end that does not meet another road is pinned at what settling moved it to, but only
        // as far as the grade limit allows from its neighbour: the move sticks (without that, the
        // solve's smoothing undoes it from one round to the next) without breaking the limit.
        for (end, other) in [(0, 1), (last, last.saturating_sub(1))] {
            if end == other || pinned.get(end).is_some_and(Option::is_some) {
                continue;
            }
            let (Some(moved), Some(neighbour)) = (
                chain.heights.get(end).copied(),
                chain.heights.get(other).copied(),
            ) else {
                continue;
            };
            let span = chain
                .spacing
                .get(end.min(other))
                .copied()
                .unwrap_or(1.0)
                .max(0.5);
            let bound = chain
                .limits
                .grade
                .get(end.min(other))
                .copied()
                .unwrap_or(f64::INFINITY)
                * span;
            let held = moved.clamp(neighbour - bound, neighbour + bound);
            if let Some(slot) = chain.heights.get_mut(end) {
                *slot = held;
            }
            if let Some(slot) = pinned.get_mut(end) {
                *slot = Some(held);
            }
        }
        let heights = solve_chain(&chain.heights, &chain.spacing, &pinned, &chain.limits);
        chain.heights = heights;
        // the solve may overshoot at a free end (the smoothing has none there): hold the move
        // near the first solve. What that breaks the limits of - ends fixed to a road they meet -
        // is reported as an excess.
        for (h, &base) in chain.heights.iter_mut().zip(&chain.origin) {
            *h = h.clamp(base - MAX_SETTLE_M, base + MAX_SETTLE_M);
        }
    }
}

/// Where the solve of a chain could not keep the grade between two junction heights, the last resort: the free
/// variables between two consecutive held ones are blended toward the straight line between the two held
/// heights by the smallest share, 0 to 1, that brings every piece of the span within `hard`. A piece's grade is
/// linear in the share, so each piece has a closed form and the span takes the largest. Every resolved link is
/// within its ceiling, below `hard`, so the straight line always qualifies. A variable that is not `movable`
/// (held, or fixed to a road it meets) stays.
fn bound_spans(
    heights: &mut [f64],
    spacing: &[f64],
    held: &[Option<f64>],
    movable: &[bool],
    hard: f64,
) {
    let anchors: Vec<usize> = held
        .iter()
        .enumerate()
        .filter_map(|(i, h)| h.map(|_| i))
        .collect();
    for pair in anchors.windows(2) {
        let [from, to] = pair else { continue };
        let (from, to) = (*from, *to);
        let length = |i: usize| spacing.get(i).copied().unwrap_or(1.0).max(0.5);
        let total: f64 = (from..to).map(length).sum();
        let (low, high) = (
            heights.get(from).copied().unwrap_or(0.0),
            heights.get(to).copied().unwrap_or(0.0),
        );
        let mut along = 0.0;
        let mut shift = vec![0.0; to - from + 1];
        for i in from + 1..to {
            along += length(i - 1);
            if movable.get(i).copied().unwrap_or(false)
                && let (Some(slot), Some(h)) = (shift.get_mut(i - from), heights.get(i))
            {
                *slot = low + (high - low) * along / total - h;
            }
        }
        // grade of piece i at share s: a + b s
        let grade = |i: usize, s: f64| {
            let (h0, h1) = (heights.get(i).copied()?, heights.get(i + 1).copied()?);
            let (d0, d1) = (
                shift.get(i - from).copied()?,
                shift.get(i + 1 - from).copied()?,
            );
            Some((h1 + s * d1 - h0 - s * d0) / length(i))
        };
        let mut share = 0.0_f64;
        for i in from..to {
            let (Some(a), Some(at_line)) = (grade(i, 0.0), grade(i, 1.0)) else {
                continue;
            };
            if a.abs() > hard && (a - at_line).abs() > 1e-12 {
                let wanted = hard * a.signum();
                share = share.max(((a - wanted) / (a - at_line)).clamp(0.0, 1.0));
            }
        }
        // a piece that was fine may not be at that share: then the whole line
        if (from..to).any(|i| grade(i, share).is_some_and(|g| g.abs() > hard + 1e-9)) {
            share = 1.0;
        }
        if share > 0.0 {
            for (i, d) in shift.iter().enumerate() {
                if let Some(h) = heights.get_mut(from + i) {
                    *h += share * d;
                }
            }
        }
    }
}

/// Brings roads that run close together to heights the terrain can follow.
///
/// The terrain cells under a road are all given the road's height, `1.5` cells beyond its edge. Where two
/// roads are closer than their footprints plus those margins, they share cells and cannot differ in height:
/// the legs of a hairpin, streets side by side, the arms of a junction. Beyond that distance the ground
/// between them may differ by the step a cell can hold, at [`FILL_SLOPE_MAX`]. Heights are compared every
/// [`SAMPLE_M`], the less important road
/// moving more; a road moves at most [`MAX_SETTLE_M`] from where it was first solved, and the grade limits
/// are kept: the moves are targets of a new solve, not commands, and only an end that meets another road is
/// fixed to it. Measured on the Berchtesgaden example this takes the pieces that miss the terrain from 5001
/// to about 3800 and the road not drivable from 37 km to 28 km. A stiffer pull towards the shared height made
/// both worse: with the ends pinned at what the pull moved them to, a strong pull left neighbouring
/// variables at opposite sides of the ± [`MAX_SETTLE_M`] range, and the grade between them broke.
fn settle(strips: &mut [RoadStrip], chains: &mut [Solved], cell: f32) {
    let spots = spots(strips);
    let pairs = close_pairs(&spots, strips, f64::from(cell));
    for _ in 0..SETTLE_ROUNDS {
        let wanted = wanted_shifts(&pairs, &spots, strips, chains, f64::from(cell));
        if wanted.is_empty() {
            break;
        }
        resolve(wanted, chains);
    }
    for chain in chains.iter_mut() {
        let movable: Vec<bool> = (0..chain.heights.len())
            .map(|v| {
                chain.held.get(v).is_some_and(Option::is_none)
                    && !chain.joins.iter().any(|(var, _)| *var == v)
            })
            .collect();
        bound_spans(
            &mut chain.heights,
            &chain.spacing,
            &chain.held,
            &movable,
            chain.hard,
        );
    }
    for (strip, chain) in strips.iter_mut().zip(chains.iter()) {
        for (joint, &var) in strip.joints.iter_mut().zip(&chain.joint_variable) {
            joint.2 = chain.heights.get(var).copied().unwrap_or(joint.2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Limits, bound_spans, smooth, solve_chain};

    #[test]
    fn a_span_with_a_steep_short_piece_is_blended_toward_the_line_just_enough() {
        // held 100 m, free joints sunk 7 m under the ground (107 m) and a short piece of 4.4 m that climbs 7 m
        // to the held 107 m: the shape the solve left on the Berchtesgaden example
        let mut heights = vec![100.0, 100.0, 100.0, 100.0, 100.0, 107.0];
        let spacing = [25.0, 25.0, 25.0, 25.0, 4.4];
        let held = [Some(100.0), None, None, None, None, Some(107.0)];
        let movable = [false, true, true, true, true, false];
        bound_spans(&mut heights, &spacing, &held, &movable, 0.3);
        for (i, w) in heights.windows(2).enumerate() {
            let grade = (w[1] - w[0]) / spacing[i];
            assert!(
                grade.abs() <= 0.3 + 1e-9,
                "piece {i}: {grade} in {heights:?}"
            );
        }
        assert!((heights[0] - 100.0).abs() < 1e-12 && (heights[5] - 107.0).abs() < 1e-12);
        // just enough: the piece sits on the limit and the others are short of the straight line
        assert!(
            ((heights[5] - heights[4]) / 4.4 - 0.3).abs() < 1e-9,
            "{heights:?}"
        );
        assert!(
            heights[1] > 100.0 && heights[1] < 100.0 + 7.0 * 25.0 / 104.4 - 0.1,
            "{heights:?}"
        );
    }

    #[test]
    fn a_span_between_two_held_heights_does_not_sag_under_the_ground() {
        // a span of the Berchtesgaden example (strip 2): the ground at about 412 m, held 414.3 m and 410.9 m, pieces
        // from 0.4 to 25 m. With the second difference of equal spacing it sagged to 380 m; with the radius of
        // the crest dropped at the held joint it dipped again
        let targets = [
            416.2, 413.7, 413.7, 413.1, 412.4, 411.8, 411.8, 411.7, 411.8, 412.0, 412.1, 412.6,
            414.1, 416.5, 414.1, 412.1, 412.3,
        ];
        let fixed = [
            Some(414.261),
            Some(414.261),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(410.916),
        ];
        let spacing = [
            24.999, 1.005, 13.080, 25.000, 12.500, 0.357, 19.578, 6.249, 5.304, 13.073, 19.509,
            13.073, 12.940, 4.357, 9.999, 4.357,
        ];
        let grade = [
            0.27, 0.27, 0.253, 0.27, 0.27, 0.27, 0.248, 0.27, 0.27, 0.248, 0.237, 0.248, 0.205,
            0.205, 0.135, 0.205,
        ];
        let limits = Limits {
            grade: grade.to_vec(),
            radius: 250.0,
        };
        let mut heights = solve_chain(&targets, &spacing, &fixed, &limits);
        // the solve may leave a bump before the last held joint (the radius is kept across held joints now);
        // what `settle` does after it bounds the span
        let movable: Vec<bool> = fixed.iter().map(Option::is_none).collect();
        bound_spans(&mut heights, &spacing, &fixed, &movable, 0.3);
        for (h, t) in heights.iter().zip(&targets) {
            assert!((h - t).abs() < 5.0, "{heights:?}");
        }
        for (i, w) in heights.windows(2).enumerate() {
            assert!(
                ((w[1] - w[0]) / spacing[i]).abs() <= 0.3 + 1e-9,
                "piece {i}: {heights:?}"
            );
        }
    }

    #[test]
    fn a_span_within_the_limit_is_left_alone() {
        let mut heights = vec![100.0, 101.0, 102.0, 103.0];
        let before = heights.clone();
        bound_spans(
            &mut heights,
            &[25.0, 25.0, 25.0],
            &[Some(100.0), None, None, Some(103.0)],
            &[false, true, true, false],
            0.3,
        );
        assert_eq!(heights, before);
    }

    #[test]
    fn a_joint_a_metre_from_a_fixed_one_is_smoothed_as_if_it_were_a_metre_away() {
        // 100 m fixed, a free joint 1 m on whose ground is 0.6 m lower, then free joints 12.5 m apart down to a
        // fixed 99.4 m: the free joint cannot be 0.6 m off the fixed one while the line through them is flat
        let target = [100.0, 99.4, 99.4, 99.4, 99.4, 99.4];
        let fixed = [true, false, false, false, false, true];
        let spacing = [1.0, 12.5, 12.5, 12.5, 12.5];
        let heights = smooth(&target, &fixed, &spacing);
        let slope_at_the_start = (heights[1] - heights[0]).abs() / 1.0;
        assert!(slope_at_the_start < 0.1, "{heights:?}");
        // equally spaced joints are smoothed as before
        let uniform = smooth(&target, &fixed, &[12.5; 5]);
        assert!(uniform[1] < 100.0 && uniform[1] > 99.3, "{uniform:?}");
    }
}

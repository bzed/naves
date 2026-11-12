// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Height profiles of the road chains.
//!
//! A road's height is not its terrain's: a profile follows the terrain but keeps to a grade limit, a limit on
//! how much the grade may change from one piece to the next (the number that makes a road feel bumpy), and
//! the height of whatever it meets. The terrain is then shaped to the profile, see [`crate::conform`].
//!
//! Per chain in placement order: a smoothing solve (a banded system, solved directly) and clipping passes
//! alternate for a fixed 30 rounds, so that the result is deterministic. A chain end that lies on a road
//! placed earlier (within its half width, however far from that road's joints) takes that road's height
//! where they meet and keeps it; an end that does not lie on another road is free. Where that makes the
//! limits impossible, the ends win and a [`GradeExcess`] is reported, on the settled heights.
//!
//! Then roads that run closer together than the terrain grid can tell apart are brought to one height (the step
//! called settling): without that, the terrain under one of them is wrong however it is shaped. Settling
//! moves heights and solves again with the grade limits, so it cannot break them.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::Family;
use crate::conform::FILL_SLOPE_MAX;
use crate::heightmap::Heightmap;
use crate::limits::{grade_aim, grade_ceiling, min_vertical_radius};
use crate::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad};

/// The steepest grade a road may carry where another road meets it (rise over run).
pub const MEETING_GRADE: f64 = 0.06;

/// How far a road may be from the terrain before it gives up its grade aim and becomes steeper.
pub const RELAX_OFF_M: f64 = 4.0;

/// Rounds of smoothing and clipping.
const ROUNDS: usize = 30;
/// Weight of the second difference against the terrain heights.
const SMOOTHNESS: f64 = 30.0;

/// The importance of a road when two want the same ground: higher wins.
fn rank(family: Family) -> u8 {
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
    /// The heights as first solved: [`settle`] moves a variable only so far from them.
    origin: Vec<f64>,
}

/// A place where a chain end meets a road solved earlier: on the strip `owner`, between its
/// joints `segment` and `segment + 1`, `t` of the way from the first to the second.
#[derive(Clone, Copy, Debug)]
struct Meeting {
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
        let length = (r.end.x - r.start.x).hypot(r.end.y - r.start.y);
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

/// The grade limit of each segment of a chain: the family's aim, lowered at a meeting and in a tight bend.
fn segment_grades(net: &PlacedNetwork, ends: &ChainEnds, roads: &[usize], base: f64) -> Vec<f64> {
    let mut limits = Vec::new();
    for &index in roads {
        let Some(road) = net.roads.get(index) else {
            continue;
        };
        if road.kind == PlacedKind::Bridge {
            continue;
        }
        let mut limit = base;
        if road.kind == PlacedKind::Junction || meets_another_chain(net, ends, roads, road) {
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

fn bands(target: &[f64], fixed: &[bool]) -> Bands {
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
    let row = [1.0, -2.0, 1.0];
    for centre in 1..count.saturating_sub(1) {
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

fn smooth(target: &[f64], fixed: &[bool]) -> Vec<f64> {
    solve_bands(&bands(target, fixed))
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
        let mut slope = ((to - from) / length).clamp(low, high);
        let (moved, anchor) = if forward {
            (segment + 1, from)
        } else {
            (segment, to)
        };
        if fixed.get(moved).copied().unwrap_or(true) {
            // a fixed height stays, whatever grade that makes
            slope = (to - from) / length;
        } else {
            let new = if forward {
                anchor + slope * length
            } else {
                anchor - slope * length
            };
            if let Some(height) = heights.get_mut(moved) {
                *height = new;
            }
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
        heights = smooth(&blended, &fixed);
        clip(&mut heights, spacing, &fixed, limits, true);
        clip(&mut heights, spacing, &fixed, limits, false);
    }
    clip(&mut heights, spacing, &fixed, limits, true);
    heights
}

/// The height of a strip at a [`Meeting`], from its joints.
fn meeting_height(strip: &RoadStrip, meet: &Meeting) -> Option<f64> {
    let from = strip.joints.get(meet.segment)?;
    let to = strip.joints.get(meet.segment + 1)?;
    Some(from.2 + meet.t * (to.2 - from.2))
}

/// A chain end this far beyond the edge of another road still counts as meeting it.
const ON_ROAD_M: f64 = 1.0;

/// Where a point lies on one road's surface: the nearest place on `strip` whose surface it is on.
/// `reach` is how far beyond the edge it may be; on a junction piece `junction_reach` is how far
/// beyond the piece's own reach its side ports lie. The ground under a bridge does not count.
fn meeting_on(
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
fn meeting_bounds(strip: &RoadStrip) -> [f64; 4] {
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
) -> (Vec<Option<f64>>, Vec<(usize, Meeting)>) {
    let count = chain.at.len();
    let mut fixed = vec![None; count];
    let mut joins = Vec::new();
    for (side, end) in [0, count.saturating_sub(1)].into_iter().enumerate() {
        let Some(point) = chain.at.get(end) else {
            continue;
        };
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

/// The profile of one chain.
fn profile_chain(
    net: &PlacedNetwork,
    index: &Index,
    roads: &[usize],
    hm: &Heightmap,
    strips: &[RoadStrip],
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
    let (fixed, meetings) = end_joins(&chain, strips, &index.strips, f64::from(hm.cell_m));
    let aim = grade_aim(family);
    let ceiling = grade_ceiling(family);
    let mut solved: Option<(Vec<f64>, Limits)> = None;
    for base in [aim, f64::midpoint(aim, ceiling), ceiling] {
        let limits = Limits {
            grade: segment_grades(net, &index.ends, roads, base),
            radius,
        };
        let heights = if count == 1 || fixed.iter().all(Option::is_some) {
            targets
                .iter()
                .zip(&fixed)
                .map(|(target, pinned)| pinned.unwrap_or(*target))
                .collect()
        } else {
            solve_chain(&targets, &spacing, &fixed, &limits)
        };
        let off = heights
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
        origin: heights.clone(),
        heights,
        spacing,
        limits,
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

/// Computes the profile of every chain of `net`, in placement order, on the terrain `hm`.
#[must_use]
pub fn solve_profiles(net: &PlacedNetwork, hm: &Heightmap) -> Profiles {
    let mut out = Profiles::default();
    let mut variables: Vec<Solved> = Vec::new();
    let mut index = Index {
        ends: ChainEnds::new(net),
        strips: Vec::new(),
    };
    for roads in &net.chains {
        let Some((strip, vars)) = profile_chain(net, &index, roads, hm, &out.strips) else {
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
    for (strip, chain) in strips.iter_mut().zip(chains.iter()) {
        for (joint, &var) in strip.joints.iter_mut().zip(&chain.joint_variable) {
            joint.2 = chain.heights.get(var).copied().unwrap_or(joint.2);
        }
    }
}

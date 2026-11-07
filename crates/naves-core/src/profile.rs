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
use crate::conform::FILL_SLOPE;
use crate::heightmap::Heightmap;
use crate::roadplace::{PlacedKind, PlacedNetwork};

/// Rounds of smoothing and clipping.
const ROUNDS: usize = 30;
/// Weight of the second difference against the terrain heights.
const SMOOTHNESS: f64 = 30.0;

/// The steepest grade a road of this family is aimed at (rise over run).
#[must_use]
pub fn max_grade(family: Family) -> f64 {
    match family {
        Family::Asf1 => 0.08,
        Family::Asf2 => 0.10,
        Family::Asf3 => 0.12,
        Family::City => 0.14,
        Family::Grav => 0.18,
        Family::Mud => 0.25,
    }
}

/// The largest change of grade from one piece to the next.
#[must_use]
pub fn max_grade_change(family: Family) -> f64 {
    match family {
        Family::Asf1 => 0.02,
        Family::Asf2 => 0.025,
        Family::Asf3 => 0.03,
        Family::City => 0.04,
        Family::Grav => 0.05,
        Family::Mud => 0.08,
    }
}

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

/// A place where the limits could not be kept.
#[derive(Debug, Clone, PartialEq)]
pub struct GradeExcess {
    /// Where.
    pub at: (f64, f64),
    /// The grade, or the change of grade, found there.
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
    /// The meeting each end lies on, if any.
    joins: [Option<Meeting>; 2],
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

/// The variables of a chain: one per joint, except that a junction has one height along its length.
struct Chain {
    /// (x, y, variable) of each joint.
    joints: Vec<(f64, f64, usize)>,
    /// Position of each variable.
    at: Vec<(f64, f64)>,
}

fn chain_of(net: &PlacedNetwork, roads: &[usize]) -> Option<Chain> {
    let first = net.roads.get(*roads.first()?)?;
    let mut joints = vec![(first.start.x, first.start.y, 0)];
    let mut at = vec![(first.start.x, first.start.y)];
    for &i in roads {
        let r = net.roads.get(i)?;
        if matches!(r.kind, PlacedKind::Junction | PlacedKind::Bridge) {
            let v = at.len() - 1;
            joints.push((r.end.x, r.end.y, v));
            if let (Some(first_joint), Some(slot)) =
                (joints.iter().find(|j| j.2 == v), at.get_mut(v))
            {
                *slot = (
                    f64::midpoint(first_joint.0, r.end.x),
                    f64::midpoint(first_joint.1, r.end.y),
                );
            }
        } else {
            joints.push((r.end.x, r.end.y, at.len()));
            at.push((r.end.x, r.end.y));
        }
    }
    Some(Chain { joints, at })
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

/// The grade limits of a family.
#[derive(Clone, Copy)]
struct Limits {
    grade: f64,
    change: f64,
}

/// One clipping pass over the grades, forward (from the start) or backward; fixed heights stay.
fn clip(heights: &mut [f64], spacing: &[f64], fixed: &[bool], limits: Limits, forward: bool) {
    let count = heights.len();
    if count < 2 {
        return;
    }
    let segments: Vec<usize> = if forward {
        (0..count - 1).collect()
    } else {
        (0..count - 1).rev().collect()
    };
    let mut previous: Option<f64> = None;
    for segment in segments {
        let length = spacing.get(segment).copied().unwrap_or(1.0).max(0.5);
        let (from, to) = (
            heights.get(segment).copied().unwrap_or(0.0),
            heights.get(segment + 1).copied().unwrap_or(0.0),
        );
        let (mut low, mut high) = (-limits.grade, limits.grade);
        if let Some(before) = previous {
            low = low.max(before - limits.change);
            high = high.min(before + limits.change);
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
        previous = Some(slope);
    }
}

fn solve_chain(
    targets: &[f64],
    spacing: &[f64],
    fixed_height: &[Option<f64>],
    limits: Limits,
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

/// Where a chain end meets a road solved earlier: the nearest place whose surface the end lies
/// on - within the road's half width (and [`ON_ROAD_M`]) of its centre line, or, on a junction
/// piece, within the piece's own length of its centre, where its side ports are. The ground under
/// a bridge does not count.
fn meeting(strips: &[RoadStrip], at: (f64, f64)) -> Option<Meeting> {
    let mut best: Option<(f64, Meeting)> = None;
    for (owner, strip) in strips.iter().enumerate() {
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
            let distance = if strip.junction.get(segment).copied().unwrap_or(false) {
                // a junction piece carries the side arms of the roads that end on it: its ports
                // are as far out as the piece is long
                let reach = length2.sqrt() / 2.0 + ON_ROAD_M;
                let (mx, my) = (
                    f64::midpoint(from.0, to.0) - at.0,
                    f64::midpoint(from.1, to.1) - at.1,
                );
                (mx.hypot(my) - reach).max(0.0)
            } else {
                (at.0 - from.0 - t * dx).hypot(at.1 - from.1 - t * dy)
            };
            if distance <= strip.half_width + ON_ROAD_M
                && best.as_ref().is_none_or(|(found, _)| distance < *found)
            {
                best = Some((distance, Meeting { owner, segment, t }));
            }
        }
    }
    best.map(|(_, meet)| meet)
}

/// The fixed heights of a chain's variables and where its ends meet an earlier road: an end that
/// reaches one takes its height there and stays fixed; any other end is free.
fn end_joins(chain: &Chain, strips: &[RoadStrip]) -> (Vec<Option<f64>>, [Option<Meeting>; 2]) {
    let count = chain.at.len();
    let mut fixed = vec![None; count];
    let mut ends_on = [None, None];
    for (side, end) in [0, count.saturating_sub(1)].into_iter().enumerate() {
        let Some(point) = chain.at.get(end) else {
            continue;
        };
        let Some(meet) = meeting(strips, *point) else {
            continue;
        };
        let height = strips
            .get(meet.owner)
            .and_then(|strip| meeting_height(strip, &meet));
        if let (Some(slot), Some(height)) = (fixed.get_mut(end), height) {
            *slot = Some(height);
        }
        if let Some(slot) = ends_on.get_mut(side) {
            *slot = Some(meet);
        }
    }
    (fixed, ends_on)
}

/// The profile of one chain.
fn profile_chain(
    net: &PlacedNetwork,
    roads: &[usize],
    hm: &Heightmap,
    strips: &[RoadStrip],
) -> Option<(RoadStrip, Solved)> {
    let chain = chain_of(net, roads)?;
    let first = net.roads.get(*roads.first()?)?;
    let family = first.family;
    let limits = Limits {
        grade: max_grade(family),
        change: max_grade_change(family),
    };
    let count = chain.at.len();
    let targets: Vec<f64> = chain
        .at
        .iter()
        .map(|point| f64::from(hm.sample(point.0, point.1).height))
        .collect();
    let spacing: Vec<f64> = chain
        .at
        .windows(2)
        .map(|pair| match pair {
            [from, to] => (to.0 - from.0).hypot(to.1 - from.1),
            _ => 1.0,
        })
        .collect();
    let (fixed, ends_on) = end_joins(&chain, strips);
    let heights = if count == 1 {
        targets
    } else {
        solve_chain(&targets, &spacing, &fixed, limits)
    };
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
        joins: ends_on,
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
    let grades: Vec<f64> = chain
        .heights
        .windows(2)
        .zip(&chain.spacing)
        .map(|(pair, length)| match pair {
            [from, to] => (to - from) / length.max(0.5),
            _ => 0.0,
        })
        .collect();
    let mut excesses = Vec::new();
    for (index, grade) in grades.iter().enumerate() {
        let at = chain.at.get(index).copied().unwrap_or_default();
        if grade.abs() > chain.limits.grade + 1e-6 {
            excesses.push(GradeExcess {
                at,
                grade: *grade,
                limit: chain.limits.grade,
            });
        }
        let before = index.checked_sub(1).and_then(|b| grades.get(b)).copied();
        if let Some(before) = before
            && (grade - before).abs() > chain.limits.change + 1e-6
        {
            excesses.push(GradeExcess {
                at,
                grade: grade - before,
                limit: chain.limits.change,
            });
        }
    }
    excesses
}

/// Computes the profile of every chain of `net`, in placement order, on the terrain `hm`.
#[must_use]
pub fn solve_profiles(net: &PlacedNetwork, hm: &Heightmap) -> Profiles {
    let mut out = Profiles::default();
    let mut variables: Vec<Solved> = Vec::new();
    for roads in &net.chains {
        let Some((strip, vars)) = profile_chain(net, roads, hm, &out.strips) else {
            continue;
        };
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
) -> BTreeMap<(usize, usize), (f64, f64)> {
    let mut wanted: BTreeMap<(usize, usize), (f64, f64)> = BTreeMap::new();
    let importance = |strip: usize| f64::from(strips.get(strip).map_or(0, |s| s.rank)) + 1.0;
    for pair in pairs {
        let (Some(a), Some(b)) = (spots.get(pair.first), spots.get(pair.second)) else {
            continue;
        };
        let (ha, hb) = (spot_height(chains, a), spot_height(chains, b));
        let allowed = MATCH_M + (pair.distance - pair.gap).max(0.0) * FILL_SLOPE;
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
        let joins = [
            chains
                .get(strip)
                .and_then(|c| c.joins.first().copied().flatten()),
            chains
                .get(strip)
                .and_then(|c| c.joins.get(1).copied().flatten()),
        ];
        let pins: [Option<f64>; 2] = [
            joins[0].and_then(|meet| joined_height(chains, &meet)),
            joins[1].and_then(|meet| joined_height(chains, &meet)),
        ];
        let Some(chain) = chains.get_mut(strip) else {
            continue;
        };
        let last = chain.heights.len().saturating_sub(1);
        let mut pinned = vec![None; chain.heights.len()];
        for (end, height) in [(0, pins[0]), (last, pins[1])] {
            if let (Some(slot), Some(height)) = (pinned.get_mut(end), height) {
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
            let bound = chain.limits.grade * span;
            let held = moved.clamp(neighbour - bound, neighbour + bound);
            if let Some(slot) = chain.heights.get_mut(end) {
                *slot = held;
            }
            if let Some(slot) = pinned.get_mut(end) {
                *slot = Some(held);
            }
        }
        chain.heights = solve_chain(&chain.heights, &chain.spacing, &pinned, chain.limits);
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
/// between them can slope at [`FILL_SLOPE`]. Heights are compared every [`SAMPLE_M`], the less important road
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
        let wanted = wanted_shifts(&pairs, &spots, strips, chains);
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

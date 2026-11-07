// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Height profiles of the road chains.
//!
//! A road's height is not its terrain's: a profile follows the terrain but keeps to a grade limit, a limit on
//! how much the grade may change from one piece to the next (the number that makes a road feel bumpy), and
//! the height of whatever it joins. The terrain is then shaped to the profile, see [`crate::conform`].
//!
//! Per chain in placement order: a smoothing solve (a banded system, solved directly) and clipping passes
//! alternate for a fixed 30 rounds, so that the result is deterministic. A chain that ends on a road placed
//! earlier takes its height there and keeps it; where that makes the limits impossible, the ends win and a
//! [`GradeExcess`] is reported.
//!
//! Then roads that run closer together than the terrain grid can tell apart are brought to one height (the step
//! called settling): without that, the terrain under one of them is wrong however it is shaped.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::Family;
use crate::conform::FILL_SLOPE;
use crate::heightmap::Heightmap;
use crate::roadplace::{PlacedKind, PlacedNetwork};

/// Rounds of smoothing and clipping.
const ROUNDS: usize = 30;
/// Weight of the second difference against the terrain heights.
const SMOOTHNESS: f64 = 30.0;
/// A chain end this close to a joint of an earlier chain takes its height.
const JOIN_RADIUS_M: f64 = 10.0;

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
    /// The distance from each variable to the next.
    spacing: Vec<f64>,
    limits: Limits,
    /// The joint, as an index into the joints solved before this chain, that each end lies on.
    joins: [Option<usize>; 2],
    /// The heights as first solved: [`settle`] moves a variable only so far from them.
    origin: Vec<f64>,
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

/// The joint of a chain solved earlier that a chain end lies on: the nearest one, as an index into `solved`.
fn joined_joint(solved: &[(f64, f64, f64)], at: (f64, f64)) -> Option<usize> {
    solved
        .iter()
        .enumerate()
        .map(|(index, joint)| ((joint.0 - at.0).hypot(joint.1 - at.1), index))
        .filter(|(distance, _)| *distance <= JOIN_RADIUS_M)
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .map(|(_, index)| index)
}

/// The fixed heights of a chain's variables and the joints its ends lie on: an end on a chain solved earlier
/// takes that chain's height.
fn end_joins(chain: &Chain, solved: &[(f64, f64, f64)]) -> (Vec<Option<f64>>, [Option<usize>; 2]) {
    let count = chain.at.len();
    let mut fixed = vec![None; count];
    let mut ends_on = [None, None];
    for (side, end) in [0, count.saturating_sub(1)].into_iter().enumerate() {
        let landing = chain
            .at
            .get(end)
            .and_then(|point| joined_joint(solved, *point));
        if let (Some(slot), Some(index)) = (fixed.get_mut(end), landing) {
            *slot = solved.get(index).map(|j| j.2);
        }
        if let Some(slot) = ends_on.get_mut(side) {
            *slot = landing;
        }
    }
    (fixed, ends_on)
}

/// The profile of one chain and where its limits are exceeded.
fn profile_chain(
    net: &PlacedNetwork,
    roads: &[usize],
    hm: &Heightmap,
    solved: &[(f64, f64, f64)],
) -> Option<(RoadStrip, Vec<GradeExcess>, Solved)> {
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
    let (fixed, ends_on) = end_joins(&chain, solved);
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
    // what the limits allow, checked on the result
    let grades: Vec<f64> = heights
        .windows(2)
        .zip(&spacing)
        .map(|(pair, length)| match pair {
            [from, to] => (to - from) / length.max(0.5),
            _ => 0.0,
        })
        .collect();
    let mut excesses = Vec::new();
    for (index, grade) in grades.iter().enumerate() {
        let at = chain.at.get(index).copied().unwrap_or_default();
        if grade.abs() > limits.grade + 1e-6 {
            excesses.push(GradeExcess {
                at,
                grade: *grade,
                limit: limits.grade,
            });
        }
        let before = index.checked_sub(1).and_then(|b| grades.get(b)).copied();
        if let Some(before) = before
            && (grade - before).abs() > limits.change + 1e-6
        {
            excesses.push(GradeExcess {
                at,
                grade: grade - before,
                limit: limits.change,
            });
        }
    }
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
        half_width: first.width / 2.0,
        rank: rank(family),
    };
    let variables = Solved {
        joint_variable: chain.joints.iter().map(|j| j.2).collect(),
        joins: ends_on,
        origin: heights.clone(),
        heights,
        spacing,
        limits,
    };
    Some((strip, excesses, variables))
}

/// Computes the profile of every chain of `net`, in placement order, on the terrain `hm`.
#[must_use]
pub fn solve_profiles(net: &PlacedNetwork, hm: &Heightmap) -> Profiles {
    let mut out = Profiles::default();
    // every joint solved so far, with its height, to find what a chain end joins
    let mut solved: Vec<(f64, f64, f64)> = Vec::new();
    let mut variables: Vec<Solved> = Vec::new();
    // which chain and joint each entry of `solved` is
    let mut owners: Vec<(usize, usize)> = Vec::new();
    for roads in &net.chains {
        let Some((strip, excesses, vars)) = profile_chain(net, roads, hm, &solved) else {
            continue;
        };
        solved.extend(strip.joints.iter().copied());
        owners.extend((0..strip.joints.len()).map(|joint| (out.strips.len(), joint)));
        out.excesses.extend(excesses);
        out.strips.push(strip);
        variables.push(vars);
    }
    settle(&mut out.strips, &mut variables, &owners, hm.cell_m);
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

/// Moves what was asked for into the targets of the chains and solves every chain that was touched again,
/// so that the grade limits and the smoothness are kept and the road does not wander off the terrain.
fn resolve(
    wanted: BTreeMap<(usize, usize), (f64, f64)>,
    chains: &mut [Solved],
    owners: &[(usize, usize)],
) {
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
        // the ends are where roads meet: an end on another road takes that road's height, any other keeps
        // what it was asked for
        let ends = [
            chains
                .get(strip)
                .and_then(|c| c.joins.first().copied().flatten()),
            chains
                .get(strip)
                .and_then(|c| c.joins.get(1).copied().flatten()),
        ]
        .map(|join| {
            let (owner, joint) = owners.get(join?).copied()?;
            let other = chains.get(owner)?;
            other
                .heights
                .get(*other.joint_variable.get(joint)?)
                .copied()
        });
        let Some(chain) = chains.get_mut(strip) else {
            continue;
        };
        let mut pinned = vec![None; chain.heights.len()];
        if let (Some(first), Some(&own)) = (pinned.first_mut(), chain.heights.first()) {
            *first = Some(ends[0].unwrap_or(own));
        }
        if let (Some(last), Some(&own)) = (pinned.last_mut(), chain.heights.last()) {
            *last = Some(ends[1].unwrap_or(own));
        }
        chain.heights = solve_chain(&chain.heights, &chain.spacing, &pinned, chain.limits);
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
/// are kept. Measured on the Berchtesgaden example this takes the pieces that miss the terrain from 5001 to
/// about 3800 and the road not drivable from 37 km to 28 km; a stiffer pull towards the shared height made
/// both worse, because it breaks the grade limits.
fn settle(strips: &mut [RoadStrip], chains: &mut [Solved], owners: &[(usize, usize)], cell: f32) {
    let spots = spots(strips);
    let pairs = close_pairs(&spots, strips, f64::from(cell));
    for _ in 0..SETTLE_ROUNDS {
        let wanted = wanted_shifts(&pairs, &spots, strips, chains);
        if wanted.is_empty() {
            break;
        }
        resolve(wanted, chains, owners);
    }
    for (strip, chain) in strips.iter_mut().zip(chains.iter()) {
        for (joint, &var) in strip.joints.iter_mut().zip(&chain.joint_variable) {
            joint.2 = chain.heights.get(var).copied().unwrap_or(joint.2);
        }
    }
}

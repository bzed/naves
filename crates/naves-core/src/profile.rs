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

use std::collections::BTreeMap;

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
/// Joints of one road must be this far apart along it to count as different parts for the switchback pass.
const SWITCHBACK_ALONG_M: f64 = 50.0;
/// Rounds of the switchback pass.
const SWITCHBACK_ROUNDS: usize = 3;

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

/// The height of a chain end that lies on a chain solved earlier: that of its nearest joint.
fn joined_height(solved: &[(f64, f64, f64)], at: (f64, f64)) -> Option<f64> {
    solved
        .iter()
        .map(|joint| ((joint.0 - at.0).hypot(joint.1 - at.1), joint.2))
        .filter(|(distance, _)| *distance <= JOIN_RADIUS_M)
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .map(|(_, height)| height)
}

/// The profile of one chain and where its limits are exceeded.
fn profile_chain(
    net: &PlacedNetwork,
    roads: &[usize],
    hm: &Heightmap,
    solved: &[(f64, f64, f64)],
) -> Option<(RoadStrip, Vec<GradeExcess>)> {
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
    let mut fixed = vec![None; count];
    for end in [0, count.saturating_sub(1)] {
        if let (Some(point), Some(slot)) = (chain.at.get(end), fixed.get_mut(end)) {
            *slot = joined_height(solved, *point);
        }
    }
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
    Some((strip, excesses))
}

/// Computes the profile of every chain of `net`, in placement order, on the terrain `hm`.
#[must_use]
pub fn solve_profiles(net: &PlacedNetwork, hm: &Heightmap) -> Profiles {
    let mut out = Profiles::default();
    // every joint solved so far, with its height, to find what a chain end joins
    let mut solved: Vec<(f64, f64, f64)> = Vec::new();
    for roads in &net.chains {
        let Some((strip, excesses)) = profile_chain(net, roads, hm, &solved) else {
            continue;
        };
        solved.extend(strip.joints.iter().copied());
        out.excesses.extend(excesses);
        out.strips.push(strip);
    }
    switchbacks(&mut out.strips, hm.cell_m);
    out
}

/// Holds joints of different parts of the network (and of one road that doubles back) to
/// `|Δh| ≤ distance × FILL_SLOPE` when they are close: two legs squeezed together would otherwise
/// ask for a wall between them.
fn switchbacks(strips: &mut [RoadStrip], cell: f32) {
    struct Joint {
        strip: usize,
        index: usize,
        x: f64,
        y: f64,
        along: f64,
        half_width: f64,
    }
    let mut joints: Vec<Joint> = Vec::new();
    for (si, strip) in strips.iter().enumerate() {
        let mut along = 0.0;
        let mut prev: Option<(f64, f64)> = None;
        for (ji, j) in strip.joints.iter().enumerate() {
            if let Some(p) = prev {
                along += (j.0 - p.0).hypot(j.1 - p.1);
            }
            prev = Some((j.0, j.1));
            joints.push(Joint {
                strip: si,
                index: ji,
                x: j.0,
                y: j.1,
                along,
                half_width: strip.half_width,
            });
        }
    }
    let widest = joints.iter().map(|j| j.half_width).fold(0.0, f64::max);
    let reach = widest * 2.0 + 3.0 * f64::from(cell);
    if reach <= 0.0 || joints.len() < 2 {
        return;
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
    for (i, j) in joints.iter().enumerate() {
        grid.entry((bucket(j.x), bucket(j.y))).or_default().push(i);
    }
    for _ in 0..SWITCHBACK_ROUNDS {
        for a in 0..joints.len() {
            let Some(ja) = joints.get(a) else { continue };
            let (cx, cy) = (bucket(ja.x), bucket(ja.y));
            for gx in cx - 1..=cx + 1 {
                for gy in cy - 1..=cy + 1 {
                    let Some(cell_joints) = grid.get(&(gx, gy)) else {
                        continue;
                    };
                    for &b in cell_joints {
                        if b <= a {
                            continue;
                        }
                        let (Some(ja), Some(jb)) = (joints.get(a), joints.get(b)) else {
                            continue;
                        };
                        let d = (ja.x - jb.x).hypot(ja.y - jb.y);
                        let local = ja.half_width.max(jb.half_width) * 2.0 + 3.0 * f64::from(cell);
                        if d > local || d < 1e-9 {
                            continue;
                        }
                        if ja.strip == jb.strip && (ja.along - jb.along).abs() <= SWITCHBACK_ALONG_M
                        {
                            continue;
                        }
                        let ha = strips
                            .get(ja.strip)
                            .and_then(|s| s.joints.get(ja.index))
                            .map_or(0.0, |j| j.2);
                        let hb = strips
                            .get(jb.strip)
                            .and_then(|s| s.joints.get(jb.index))
                            .map_or(0.0, |j| j.2);
                        let allowed = d * FILL_SLOPE;
                        let excess = (ha - hb).abs() - allowed;
                        if excess > 0.0 {
                            let shift = excess / 2.0 * (ha - hb).signum();
                            let (sa, ia, sb, ib) = (ja.strip, ja.index, jb.strip, jb.index);
                            if let Some(j) = strips.get_mut(sa).and_then(|s| s.joints.get_mut(ia)) {
                                j.2 -= shift;
                            }
                            if let Some(j) = strips.get_mut(sb).and_then(|s| s.joints.get_mut(ib)) {
                                j.2 += shift;
                            }
                        }
                    }
                }
            }
        }
    }
}

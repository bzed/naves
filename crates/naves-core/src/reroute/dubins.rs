// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dubins paths: the shortest ways between two poses for a car that cannot turn tighter than a radius, as two arcs
//! and a straight or three arcs (spec: "The search for one leg", arrival at a pose).
//!
//! The six words `LSL`, `RSR`, `LSR`, `RSL`, `RLR` and `LRL` are the classical closed forms (Dubins 1957; the
//! normalised form of Shkel and Lumelsky 2001). The crate `dubins_paths` was considered and not used, see the
//! commit that added this module. All input is untrusted: poses and radii that are not numbers give no path, and
//! nothing here panics.

use std::f64::consts::TAU;

use super::Pose;

/// An arc is sampled about every this many degrees by [`DubinsPath::points`] and [`arc_points`].
pub const ARC_SAMPLE_DEG: f64 = 2.0;
/// Spacing of the samples of a segment when the caller gives none, in metres.
const SPACING_DEFAULT_M: f64 = 2.0;
/// Most points of one segment.
const MAX_POINTS: f64 = 100_000.0;
/// Two poses closer than this (metres and radians) are the same pose.
const SAME_POSE: f64 = 1e-12;
/// A square length (or a cosine) this near its limit is on it: the tangent cases, where rounding decides the sign.
const TANGENT_EPS: f64 = 1e-12;
/// A path whose closed form ends further than this (metres and radians) from the target is not returned.
const END_TOLERANCE: f64 = 1e-9;

type Word = [Turn; 3];
const WORD_LSL: Word = [Turn::Left, Turn::Straight, Turn::Left];
const WORD_RSR: Word = [Turn::Right, Turn::Straight, Turn::Right];
const WORD_LSR: Word = [Turn::Left, Turn::Straight, Turn::Right];
const WORD_RSL: Word = [Turn::Right, Turn::Straight, Turn::Left];
const WORD_RLR: Word = [Turn::Right, Turn::Left, Turn::Right];
const WORD_LRL: Word = [Turn::Left, Turn::Right, Turn::Left];

/// One of the three moves of a Dubins path. The order (left, straight, right) breaks ties between paths of the
/// same length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Turn {
    /// An arc to the left (counter-clockwise).
    Left,
    /// A straight.
    Straight,
    /// An arc to the right (clockwise).
    Right,
}

impl Turn {
    /// The curvature sign: 1 for left, 0 for a straight, -1 for right.
    fn sign(self) -> f64 {
        match self {
            Turn::Left => 1.0,
            Turn::Straight => 0.0,
            Turn::Right => -1.0,
        }
    }
}

/// A Dubins path: three moves with their lengths in metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DubinsPath {
    word: [Turn; 3],
    lengths: [f64; 3],
}

impl DubinsPath {
    /// The three moves.
    #[must_use]
    pub fn word(&self) -> [Turn; 3] {
        self.word
    }

    /// The length of each move, in metres (arcs: along the arc).
    #[must_use]
    pub fn lengths(&self) -> [f64; 3] {
        self.lengths
    }

    /// The length of the path, in metres.
    #[must_use]
    pub fn length(&self) -> f64 {
        self.lengths.iter().sum()
    }

    /// The total heading change of the path, in radians (the sum of the arcs' angles).
    #[must_use]
    pub fn turning(&self, radius: f64) -> f64 {
        self.word
            .iter()
            .zip(self.lengths)
            .filter(|(t, _)| **t != Turn::Straight)
            .map(|(_, l)| l / radius)
            .sum()
    }

    /// The pose at the end of the path that starts at `from`, followed along its arcs and straight.
    #[must_use]
    pub fn end_pose(&self, from: Pose, radius: f64) -> Pose {
        self.word
            .iter()
            .zip(self.lengths)
            .fold(from, |pose, (t, l)| follow(pose, t.sign() / radius, l))
    }

    /// The points of the path that starts at `from`: the start, then every move sampled so that no two points
    /// are more than `step_m` apart (a step that is not a positive number counts as 2 m) and an arc has a point
    /// about every [`ARC_SAMPLE_DEG`] degrees; the end last. A move of no length adds nothing.
    #[must_use]
    pub fn points(&self, from: Pose, radius: f64, step_m: f64) -> Vec<(f64, f64)> {
        self.sampled(from, radius, step_m, true)
    }

    /// The same, sampled by length only: no two points are more than `step_m` apart and an arc has no extra points.
    /// For the search's tests, where a chord of 2 m on a radius of 10 m is 5 cm off the arc; the written route uses
    /// [`DubinsPath::points`].
    #[must_use]
    pub fn points_coarse(&self, from: Pose, radius: f64, step_m: f64) -> Vec<(f64, f64)> {
        self.sampled(from, radius, step_m, false)
    }

    fn sampled(&self, from: Pose, radius: f64, step_m: f64, by_angle: bool) -> Vec<(f64, f64)> {
        let mut out = vec![from.at];
        let mut pose = from;
        for (t, &l) in self.word.iter().zip(&self.lengths) {
            if l <= 0.0 {
                continue;
            }
            let curvature = t.sign() / radius;
            let angle = if by_angle { curvature * l } else { 0.0 };
            let n = segments(angle, l, step_m);
            out.extend(arc_points(pose, curvature, l, n).into_iter().skip(1));
            pose = follow(pose, curvature, l);
        }
        out
    }
}

/// `x` modulo a turn, in `0..TAU`.
fn mod2pi(x: f64) -> f64 {
    let r = x.rem_euclid(TAU);
    if r >= TAU || r.is_nan() { 0.0 } else { r }
}

/// The pose after following an arc of the given curvature (0 for a straight) and length from `from`.
#[must_use]
pub fn follow(from: Pose, curvature: f64, length: f64) -> Pose {
    let (s, c) = from.heading.sin_cos();
    if curvature.abs() < 1e-12 {
        return Pose {
            at: (from.at.0 + length * c, from.at.1 + length * s),
            heading: from.heading,
        };
    }
    let phi = curvature * length;
    let (s2, c2) = (from.heading + phi).sin_cos();
    Pose {
        at: (
            from.at.0 + (s2 - s) / curvature,
            from.at.1 - (c2 - c) / curvature,
        ),
        heading: from.heading + phi,
    }
}

/// How many segments sample an arc of `angle` radians and `length` metres, at most `step_m` metres and
/// [`ARC_SAMPLE_DEG`] degrees apart: at least one, at most 100 000.
#[must_use]
pub fn segments(angle: f64, length: f64, step_m: f64) -> usize {
    let step = if step_m.is_finite() && step_m > 0.0 {
        step_m
    } else {
        SPACING_DEFAULT_M
    };
    let by_length = (length.abs() / step).ceil();
    let by_angle = (angle.abs() / ARC_SAMPLE_DEG.to_radians()).ceil();
    let n = by_length.max(by_angle).clamp(1.0, MAX_POINTS);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 1..=MAX_POINTS just above"
    )]
    let n = n as usize;
    n
}

/// The `n + 1` points of an arc (a straight for a curvature of 0) of the given length that starts at `from`,
/// equally spaced along it, the start first.
#[must_use]
pub fn arc_points(from: Pose, curvature: f64, length: f64, n: usize) -> Vec<(f64, f64)> {
    let n = n.max(1);
    let denom = f64::from(u32::try_from(n).unwrap_or(u32::MAX));
    (0..=n)
        .map(|i| {
            let part = f64::from(u32::try_from(i).unwrap_or(u32::MAX)) / denom;
            follow(from, curvature, length * part).at
        })
        .collect()
}

/// What the six words share: the headings relative to the line between the poses, and the distance, all in units
/// of the radius.
struct Normal {
    alpha: f64,
    beta: f64,
    dist: f64,
    sin_a: f64,
    sin_b: f64,
    cos_a: f64,
    cos_b: f64,
    cos_ab: f64,
}

/// A square length within rounding of 0 is 0: the tangent cases (a quarter circle) would lose their path.
fn snap(v: f64) -> f64 {
    if v.abs() < TANGENT_EPS { 0.0 } else { v }
}

/// The path of a word with a straight in the middle: the arcs before and after it, its square length first.
fn csc(square: f64, arcs: impl FnOnce(f64) -> (f64, f64)) -> Option<[f64; 3]> {
    let square = snap(square);
    (square >= 0.0).then(|| {
        let mid = square.sqrt();
        let (first, last) = arcs(mid);
        [first, mid, last]
    })
}

/// The path of a word of three arcs, from the cosine of its middle arc.
fn ccc(cosine: f64, arcs: impl FnOnce(f64) -> (f64, f64)) -> Option<[f64; 3]> {
    (cosine.abs() <= 1.0 + TANGENT_EPS).then(|| {
        let mid = mod2pi(TAU - cosine.clamp(-1.0, 1.0).acos());
        let (first, last) = arcs(mid);
        [first, mid, last]
    })
}

impl Normal {
    fn lsl(&self) -> Option<[f64; 3]> {
        let square = 2.0 + self.dist.powi(2) - 2.0 * self.cos_ab
            + 2.0 * self.dist * (self.sin_a - self.sin_b);
        csc(square, |_| {
            let tmp = (self.cos_b - self.cos_a).atan2(self.dist + self.sin_a - self.sin_b);
            (mod2pi(tmp - self.alpha), mod2pi(self.beta - tmp))
        })
    }

    fn rsr(&self) -> Option<[f64; 3]> {
        let square = 2.0 + self.dist.powi(2) - 2.0 * self.cos_ab
            + 2.0 * self.dist * (self.sin_b - self.sin_a);
        csc(square, |_| {
            let tmp = (self.cos_a - self.cos_b).atan2(self.dist - self.sin_a + self.sin_b);
            (mod2pi(self.alpha - tmp), mod2pi(tmp - self.beta))
        })
    }

    fn lsr(&self) -> Option<[f64; 3]> {
        let square = -2.0
            + self.dist.powi(2)
            + 2.0 * self.cos_ab
            + 2.0 * self.dist * (self.sin_a + self.sin_b);
        csc(square, |mid| {
            let tmp = (-self.cos_a - self.cos_b).atan2(self.dist + self.sin_a + self.sin_b)
                - (-2.0_f64).atan2(mid);
            (mod2pi(tmp - self.alpha), mod2pi(tmp - mod2pi(self.beta)))
        })
    }

    fn rsl(&self) -> Option<[f64; 3]> {
        let square = -2.0 + self.dist.powi(2) + 2.0 * self.cos_ab
            - 2.0 * self.dist * (self.sin_a + self.sin_b);
        csc(square, |mid| {
            let tmp = (self.cos_a + self.cos_b).atan2(self.dist - self.sin_a - self.sin_b)
                - 2.0_f64.atan2(mid);
            (mod2pi(self.alpha - tmp), mod2pi(self.beta - tmp))
        })
    }

    fn rlr(&self) -> Option<[f64; 3]> {
        let cosine = (6.0 - self.dist.powi(2)
            + 2.0 * self.cos_ab
            + 2.0 * self.dist * (self.sin_a - self.sin_b))
            / 8.0;
        ccc(cosine, |mid| {
            let phi = (self.cos_a - self.cos_b).atan2(self.dist - self.sin_a + self.sin_b);
            let first = mod2pi(self.alpha - phi + mod2pi(mid / 2.0));
            (first, mod2pi(self.alpha - self.beta - first + mod2pi(mid)))
        })
    }

    fn lrl(&self) -> Option<[f64; 3]> {
        let cosine = (6.0 - self.dist.powi(2)
            + 2.0 * self.cos_ab
            + 2.0 * self.dist * (self.sin_b - self.sin_a))
            / 8.0;
        ccc(cosine, |mid| {
            let phi = (self.cos_a - self.cos_b).atan2(self.dist + self.sin_a - self.sin_b);
            let first = mod2pi(-self.alpha - phi + mid / 2.0);
            (
                first,
                mod2pi(mod2pi(self.beta) - self.alpha - first + mod2pi(mid)),
            )
        })
    }
}

/// The Dubins paths of radius `radius` from one pose to another, shortest first, ties in the order of the word
/// (`LSL` before `LSR`, and so on). At most six, fewer where a word has no solution; none for a pose or radius
/// that is not a finite number, or a radius that is not positive. Two equal poses have one path of length 0.
/// The returned paths end at `to`, within 10^-9 m and radians in the tests (10 000 random pairs).
#[must_use]
pub fn dubins_paths(from: Pose, to: Pose, radius: f64) -> Vec<DubinsPath> {
    let finite = |p: Pose| p.at.0.is_finite() && p.at.1.is_finite() && p.heading.is_finite();
    if !finite(from) || !finite(to) || !radius.is_finite() || radius <= 0.0 {
        return Vec::new();
    }
    let (dx, dy) = (to.at.0 - from.at.0, to.at.1 - from.at.1);
    let turn = mod2pi(to.heading - from.heading);
    if dx.abs() <= SAME_POSE
        && dy.abs() <= SAME_POSE
        && (turn <= SAME_POSE || TAU - turn <= SAME_POSE)
    {
        return vec![DubinsPath {
            word: [Turn::Left, Turn::Straight, Turn::Left],
            lengths: [0.0; 3],
        }];
    }
    let distance = dx.hypot(dy);
    let theta = if distance > 0.0 { dy.atan2(dx) } else { 0.0 };
    let alpha = mod2pi(from.heading - theta);
    let beta = mod2pi(to.heading - theta);
    let normal = Normal {
        alpha,
        beta,
        dist: distance / radius,
        sin_a: alpha.sin(),
        sin_b: beta.sin(),
        cos_a: alpha.cos(),
        cos_b: beta.cos(),
        cos_ab: (alpha - beta).cos(),
    };
    let words: [(Word, Option<[f64; 3]>); 6] = [
        (WORD_LSL, normal.lsl()),
        (WORD_RSR, normal.rsr()),
        (WORD_LSR, normal.lsr()),
        (WORD_RSL, normal.rsl()),
        (WORD_RLR, normal.rlr()),
        (WORD_LRL, normal.lrl()),
    ];
    let mut out: Vec<DubinsPath> = words
        .into_iter()
        .filter_map(|(word, lengths)| {
            let lengths = lengths?;
            lengths
                .iter()
                .all(|v| v.is_finite() && *v >= 0.0)
                .then(|| DubinsPath {
                    word,
                    lengths: lengths.map(|v| v * radius),
                })
        })
        .collect();
    // the closed forms are exact away from the tangent cases; a path that misses the target there is dropped
    out.retain(|p| {
        let end = p.end_pose(from, radius);
        let gap = (end.at.0 - to.at.0).hypot(end.at.1 - to.at.1);
        let turn = mod2pi(end.heading - to.heading);
        gap <= END_TOLERANCE && (turn <= END_TOLERANCE || TAU - turn <= END_TOLERANCE)
    });
    out.sort_by(|x, y| {
        x.length()
            .total_cmp(&y.length())
            .then_with(|| x.word.cmp(&y.word))
    });
    out
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A smooth curve fitted to one line inside a corridor.
//!
//! A line is resampled at equal steps ([`resample`]) and every sample gets one unknown, its offset `d_i`
//! along the line's normal at that sample. [`smooth_line`] minimises
//!
//! ```text
//! Σ c_i · |q_{i-1} − 2·q_i + q_{i+1}|²  +  stay · Σ d_i²        with q_i = p_i + d_i · n_i
//! ```
//!
//! subject to `|d_i| ≤ corridor` and `d_i = 0` at fixed samples. The cost is a convex quadratic and the
//! constraints are bounds, so the answer is unique; it is solved by [`crate::qp`], which is deterministic
//! bit for bit.

use crate::qp::{Problem, Row, solve};

/// The step of a resampled line, in metres.
pub const SAMPLE_M: f64 = 2.0;
/// How far a sample may move sideways, in metres.
pub const CORRIDOR_M: f64 = 2.0;
/// The weight of the pull of a sample towards its original position.
pub const W_STAY: f64 = 0.02;
/// The stiffness (`c_i`) callers give to samples inside a bridge stretch.
pub const BRIDGE_STRAIGHT_WEIGHT: f64 = 10.0;

/// The most segments [`resample`] makes of one line (a guard against absurd lengths).
const MAX_SEGMENTS: f64 = 1.0e6;

/// One sample of a line to smooth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// The position.
    pub at: (f64, f64),
    /// A fixed sample does not move.
    pub fixed: bool,
    /// The weight `c_i` of the bend at this sample (1 on plain road).
    pub stiffness: f64,
}

/// The settings of [`smooth_line`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SmoothParams {
    /// The largest sideways move of a sample, in metres.
    pub corridor: f64,
    /// The weight of the pull towards the original position.
    pub stay: f64,
}

impl Default for SmoothParams {
    fn default() -> Self {
        SmoothParams {
            corridor: CORRIDOR_M,
            stay: W_STAY,
        }
    }
}

/// Why a line is not smoothed.
#[derive(Debug, thiserror::Error)]
pub enum SmoothError {
    /// Fewer than three samples.
    #[error("a line needs at least three samples, it has {0}")]
    TooShort(usize),
    /// A coordinate, stiffness or setting is NaN, infinite or (for a weight or the corridor) negative.
    #[error("a coordinate, stiffness or setting is not a finite number")]
    NotFinite,
    /// All samples lie on one point.
    #[error("all samples lie on one point")]
    Degenerate,
    /// The solver did not give a solution.
    #[error("the solver failed: {0}")]
    Solver(String),
}

/// Points at equal steps of about `step` along `line`; first and last are kept exactly.
///
/// The step is adjusted to divide the line's length evenly (at least one step). Zero-length segments are
/// skipped. A line with a non-finite coordinate, or a `step` that is not positive and finite, comes back
/// unchanged.
#[must_use]
pub fn resample(line: &[(f64, f64)], step: f64) -> Vec<(f64, f64)> {
    let finite = line.iter().all(|p| p.0.is_finite() && p.1.is_finite());
    if !finite || !step.is_finite() || step <= 0.0 {
        return line.to_vec();
    }
    let mut points: Vec<(f64, f64)> = Vec::with_capacity(line.len());
    for &p in line {
        if points.last() != Some(&p) {
            points.push(p);
        }
    }
    let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
        return points;
    };
    if points.len() < 2 {
        return points;
    }
    // cumulative length at each point
    let mut cumulative = Vec::with_capacity(points.len());
    let mut total = 0.0;
    cumulative.push(0.0);
    for w in points.windows(2) {
        if let [a, b] = w {
            total += (b.0 - a.0).hypot(b.1 - a.1);
        }
        cumulative.push(total);
    }
    if !total.is_finite() {
        return line.to_vec();
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is rounded, at least 1 and at most MAX_SEGMENTS"
    )]
    let segments = (total / step).round().clamp(1.0, MAX_SEGMENTS) as usize;
    let mut out = Vec::with_capacity(segments + 1);
    out.push(first);
    let mut seg = 1;
    for k in 1..segments {
        #[expect(
            clippy::cast_precision_loss,
            reason = "k is below MAX_SEGMENTS, exact in f64"
        )]
        let target = total * (k as f64) / (segments as f64);
        while cumulative.get(seg).is_some_and(|&c| c < target) {
            seg += 1;
        }
        let (Some(&a), Some(&b), Some(&c0), Some(&c1)) = (
            points.get(seg - 1),
            points.get(seg),
            cumulative.get(seg - 1),
            cumulative.get(seg),
        ) else {
            break;
        };
        // c1 > c0: the loop above stops at the first point whose length reaches the target, and no
        // segment has length zero
        let t = (target - c0) / (c1 - c0);
        out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
    }
    out.push(last);
    out
}

/// The radius of the circle through three consecutive points (infinite on a straight line).
#[must_use]
pub fn radius_at(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
    let cross = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    if cross.abs() <= f64::MIN_POSITIVE {
        return f64::INFINITY;
    }
    let side = |p: (f64, f64), q: (f64, f64)| (q.0 - p.0).hypot(q.1 - p.1);
    side(a, b) * side(b, c) * side(c, a) / (2.0 * cross.abs())
}

/// The left unit normal of the direction from the previous to the next sample (one-sided at the ends);
/// zero where the two coincide.
fn normals(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let last = points.len().saturating_sub(1);
    (0..points.len())
        .map(|i| {
            let prev = points.get(i.saturating_sub(1)).copied().unwrap_or_default();
            let next = points.get((i + 1).min(last)).copied().unwrap_or_default();
            let (dx, dy) = (next.0 - prev.0, next.1 - prev.1);
            let len = dx.hypot(dy);
            if len > 0.0 {
                (-dy / len, dx / len)
            } else {
                (0.0, 0.0)
            }
        })
        .collect()
}

/// The smoothed positions of `samples`, same length and order. Fixed samples do not move; no sample moves
/// more than `params.corridor` along its normal.
///
/// # Errors
///
/// [`SmoothError::TooShort`] for fewer than three samples, [`SmoothError::NotFinite`] for a coordinate,
/// stiffness or setting that is not finite (or a negative weight or corridor),
/// [`SmoothError::Degenerate`] when all samples lie on one point and [`SmoothError::Solver`] when the
/// solver gives no solution.
pub fn smooth_line(
    samples: &[Sample],
    params: &SmoothParams,
) -> Result<Vec<(f64, f64)>, SmoothError> {
    let count = samples.len();
    if count < 3 {
        return Err(SmoothError::TooShort(count));
    }
    let settings_ok = params.corridor.is_finite()
        && params.corridor >= 0.0
        && params.stay.is_finite()
        && params.stay >= 0.0;
    let samples_ok = samples.iter().all(|s| {
        s.at.0.is_finite() && s.at.1.is_finite() && s.stiffness.is_finite() && s.stiffness >= 0.0
    });
    if !settings_ok || !samples_ok {
        return Err(SmoothError::NotFinite);
    }
    let points: Vec<(f64, f64)> = samples.iter().map(|s| s.at).collect();
    if points.windows(2).all(|w| w.first() == w.get(1)) {
        return Err(SmoothError::Degenerate);
    }
    let normal = normals(&points);

    let mut problem = Problem::new(count);
    for (i, (w, nw)) in samples.windows(3).zip(normal.windows(3)).enumerate() {
        // the second difference at sample i + 1, per coordinate
        if let ([before, mid, after], [na, nb, nc]) = (w, nw) {
            let centre = i + 1;
            let along_x = [(i, na.0), (centre, -2.0 * nb.0), (i + 2, nc.0)];
            let along_y = [(i, na.1), (centre, -2.0 * nb.1), (i + 2, nc.1)];
            problem.add_square(
                mid.stiffness,
                &along_x,
                before.at.0 - 2.0 * mid.at.0 + after.at.0,
            );
            problem.add_square(
                mid.stiffness,
                &along_y,
                before.at.1 - 2.0 * mid.at.1 + after.at.1,
            );
        }
    }
    for (i, s) in samples.iter().enumerate() {
        problem.add_square(params.stay, &[(i, 1.0)], 0.0);
        let bound = if s.fixed { 0.0 } else { params.corridor };
        problem.rows.push(Row {
            terms: vec![(i, 1.0)],
            lo: -bound,
            hi: bound,
        });
    }
    let d = solve(&problem).map_err(|e| SmoothError::Solver(e.to_string()))?;
    if d.len() != count || d.iter().any(|v| !v.is_finite()) {
        return Err(SmoothError::Solver(
            "the solution is not finite".to_string(),
        ));
    }
    Ok(samples
        .iter()
        .zip(&normal)
        .zip(&d)
        .map(|((s, nm), &off)| {
            if s.fixed {
                s.at
            } else {
                let off = off.clamp(-params.corridor, params.corridor);
                (s.at.0 + off * nm.0, s.at.1 + off * nm.1)
            }
        })
        .collect())
}

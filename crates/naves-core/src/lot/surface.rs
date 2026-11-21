// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Groups, exclusions and the smooth surface of a lot.
//!
//! [`assess`] decides for every [`LotArea`] what becomes of it ([`Outcome`]), joins the lots that touch into
//! groups and fits one [`Surface`] per group to the original ground at the terrain vertices inside the union of
//! its members: a plane, or, where the ground is curved, a quadratic whose curvature is capped at the gentlest
//! road's vertical radius ([`LOT_MIN_RADIUS_M`]). Nothing here changes the terrain; the shaping and the road
//! stages read the result.

use std::collections::BTreeSet;

use super::{
    LOT_INSIDE_TOL_M, LOT_MAX_MOVE_M, LOT_MIN_RADIUS_M, LOT_MIN_VERTICES, LOT_PLANE_FIT_M,
    LOT_RELIEF_LENGTH_M, LOT_STEEP, LOT_WATER_MAX, LotArea, Placement, contains, edge_distance,
    ring_contains, segment_distance,
};
use crate::heightmap::Heightmap;
use crate::smooth::WaterTest;

/// The group number of a lot that is in no group (excluded or surface only).
pub const NO_GROUP: usize = usize::MAX;

/// The fewest terrain vertices a group needs for a quadratic: fewer would be fitted exactly and say nothing.
const QUADRATIC_MIN_VERTICES: usize = 10;

/// Differences below this are noise, not a rise or a fall (metres).
const NOISE_M: f64 = 1e-9;

/// The form of a lot's surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// `z = h + gx·dx + gy·dy`.
    Plane,
    /// A quadratic with its curvature capped, and the plane part fitted again.
    Quadratic,
}

/// The fitted surface of a group. The height at a point is
/// `coeffs[0] + coeffs[1]·dx + coeffs[2]·dy + coeffs[3]·dx² + coeffs[4]·dx·dy + coeffs[5]·dy²` with
/// `(dx, dy)` the offset from `at`.
#[derive(Debug, Clone, PartialEq)]
pub struct Surface {
    /// The point the coefficients are about, in map metres.
    pub at: (f64, f64),
    /// The coefficients, in metres.
    pub coeffs: [f64; 6],
    /// Plane or quadratic.
    pub form: Form,
    /// 1, unless the move guard scaled the change of the ground by this factor. The guard looks at the terrain
    /// vertices inside the lot only: a vertex beyond the edge (a shoulder) is not covered by it.
    pub lambda: f64,
}

impl Surface {
    /// The height of the fitted surface at `p` (map metres); `lambda` is not applied, see
    /// [`LotSurface::target`].
    #[must_use]
    pub fn height_at(&self, p: (f64, f64)) -> f64 {
        let (dx, dy) = (p.0 - self.at.0, p.1 - self.at.1);
        let [c0, c1, c2, c3, c4, c5] = self.coeffs;
        c0 + c1 * dx + c2 * dy + c3 * dx * dx + c4 * dx * dy + c5 * dy * dy
    }

    /// The gradient (rise per metre towards east and north) of the fitted surface at `p`.
    #[must_use]
    pub fn gradient_at(&self, p: (f64, f64)) -> (f64, f64) {
        let (dx, dy) = (p.0 - self.at.0, p.1 - self.at.1);
        let [_, c1, c2, c3, c4, c5] = self.coeffs;
        (c1 + 2.0 * c3 * dx + c4 * dy, c2 + c4 * dx + 2.0 * c5 * dy)
    }
}

/// What becomes of a lot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Roads inside go, the ground is shaped to the group's surface.
    Smoothed {
        /// The form of the group's surface.
        form: Form,
    },
    /// Painted, but the ground is not shaped. Reasons: `"street_side"`, `"no_vertex"`, `"would_split"`
    /// (set later, when the roads are removed).
    SurfaceOnly {
        /// Why.
        reason: &'static str,
    },
    /// Left alone entirely. Reasons: `"not_at_ground"`, `"in_water"`.
    Excluded {
        /// Why.
        reason: &'static str,
    },
}

/// One lot with what was found out about it. Figures that need a surface are NaN where the lot is not smoothed
/// (the water share is also known for a lot excluded for water, and NaN only where it was not looked at). Moves,
/// slope and relief are those of the lot's own terrain vertices against its group's surface.
#[derive(Debug, Clone, PartialEq)]
pub struct Assessed {
    /// The lot.
    pub area: LotArea,
    /// The index of the lot's [`LotSurface`] in the list [`assess`] returns, [`NO_GROUP`] if it has none.
    pub group: usize,
    /// What becomes of it.
    pub outcome: Outcome,
    /// The 95th percentile of the surface's slope is above [`LOT_STEEP`].
    pub steep: bool,
    /// 95th percentile of the move of the ground, after the move guard, in metres.
    pub move_p95_m: f64,
    /// Largest move of the ground, after the move guard, in metres.
    pub move_max_m: f64,
    /// 95th percentile of the surface's slope.
    pub slope_p95: f64,
    /// Largest local relief of the original ground ([`local_relief`]), in metres.
    pub relief_before_m: f64,
    /// Largest local relief of the result.
    pub relief_after_m: f64,
    /// Lowest and highest original ground at the lot's vertices.
    pub range_before: (f64, f64),
    /// Lowest and highest result at the lot's vertices.
    pub range_after: (f64, f64),
    /// The share (0 to 1) of the lot's 1 m samples on the map that lie in water.
    pub water_share: f64,
}

/// An outer ring and its holes, closed rings of map (x, y) in metres.
pub type Polygon = (Vec<(f64, f64)>, Vec<Vec<(f64, f64)>>);

/// One smooth surface over a group of lots.
#[derive(Debug, Clone, PartialEq)]
pub struct LotSurface {
    /// The group's number: its index in the list, groups ordered by their lowest member id.
    pub group: usize,
    /// The ids of the members, sorted.
    pub members: Vec<String>,
    /// The members' outer rings and holes, in member order.
    pub polygons: Vec<Polygon>,
    /// The surface.
    pub surface: Surface,
}

impl LotSurface {
    /// Whether `p` is inside any member (even-odd, in no hole of that member).
    #[must_use]
    pub fn contains(&self, p: (f64, f64)) -> bool {
        self.polygons.iter().any(|(outer, holes)| {
            ring_contains(outer, p) && !holes.iter().any(|h| ring_contains(h, p))
        })
    }

    /// The height the ground at `p` is changed to: `ground + lambda · (surface − ground)`.
    #[must_use]
    pub fn target(&self, p: (f64, f64), ground: f64) -> f64 {
        ground + self.surface.lambda * (self.surface.height_at(p) - ground)
    }
}

// ----- small numeric helpers -----

#[expect(
    clippy::cast_precision_loss,
    reason = "terrain indices are a few thousand at most"
)]
fn to_f64(i: i64) -> f64 {
    i as f64
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the value is clamped to a range far inside i64"
)]
fn to_index(v: f64) -> i64 {
    if v.is_finite() {
        v.clamp(-1.0e9, 1.0e9) as i64
    } else {
        0
    }
}

/// The 95th percentile by the nearest-rank rule (0 for an empty list).
fn percentile95(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    let rank = (values.len() * 95).div_ceil(100);
    rank.checked_sub(1)
        .and_then(|i| values.get(i))
        .copied()
        .unwrap_or(0.0)
}

fn max_of(values: &[f64]) -> f64 {
    values.iter().copied().fold(0.0, f64::max)
}

fn mat_get(m: &[Vec<f64>], i: usize, j: usize) -> f64 {
    m.get(i).and_then(|row| row.get(j)).copied().unwrap_or(0.0)
}

fn mat_put(m: &mut [Vec<f64>], i: usize, j: usize, value: f64) {
    if let Some(cell) = m.get_mut(i).and_then(|row| row.get_mut(j)) {
        *cell = value;
    }
}

/// Gaussian elimination with partial pivoting (the first of equal pivots wins) on the `k × (k + 1)` augmented
/// matrix; `None` for a singular one.
fn gauss(mut m: Vec<Vec<f64>>, k: usize) -> Option<Vec<f64>> {
    for col in 0..k {
        let mut pivot = col;
        for row in col + 1..k {
            if mat_get(&m, row, col).abs() > mat_get(&m, pivot, col).abs() {
                pivot = row;
            }
        }
        let p = mat_get(&m, pivot, col);
        if !p.is_finite() || p.abs() < 1e-300 {
            return None;
        }
        m.swap(col, pivot);
        for row in col + 1..k {
            let factor = mat_get(&m, row, col) / p;
            for j in col..=k {
                let value = mat_get(&m, row, j) - factor * mat_get(&m, col, j);
                mat_put(&mut m, row, j, value);
            }
        }
    }
    let mut x = vec![0.0; k];
    for i in (0..k).rev() {
        let mut sum = mat_get(&m, i, k);
        for j in i + 1..k {
            sum -= mat_get(&m, i, j) * x.get(j).copied().unwrap_or(0.0);
        }
        let value = sum / mat_get(&m, i, i);
        if !value.is_finite() {
            return None;
        }
        if let Some(slot) = x.get_mut(i) {
            *slot = value;
        }
    }
    Some(x)
}

/// Least squares of `z ≈ Σ coeff_i · design_i` over the points (`u`, `v`, `z`) through the normal equations; the
/// first `k` of the six design terms `1, u, v, u², u·v, v²`. A tiny ridge on every term but the constant keeps a
/// degenerate point set (all on one line) solvable: the direction without data gets no slope.
fn least_squares(points: &[(f64, f64, f64)], k: usize) -> Option<Vec<f64>> {
    let mut m = vec![vec![0.0; k + 1]; k];
    for &(u, v, z) in points {
        let design = [1.0, u, v, u * u, u * v, v * v];
        for (i, di) in design.iter().enumerate().take(k) {
            for (j, dj) in design.iter().enumerate().take(k) {
                let value = mat_get(&m, i, j) + di * dj;
                mat_put(&mut m, i, j, value);
            }
            let value = mat_get(&m, i, k) + di * z;
            mat_put(&mut m, i, k, value);
        }
    }
    let count = to_f64(i64::try_from(points.len()).unwrap_or(i64::MAX));
    for i in 1..k {
        let diag = mat_get(&m, i, i);
        mat_put(&mut m, i, i, diag + 1e-12 * diag.max(count));
    }
    gauss(m, k)
}

/// A plane `[h, gx, gy]` through (`dx`, `dy`, `z`) points about the origin, fitted in coordinates divided by
/// `scale`.
fn fit_plane(points: &[(f64, f64, f64)], scale: f64) -> Option<[f64; 3]> {
    let scaled: Vec<(f64, f64, f64)> = points
        .iter()
        .map(|&(dx, dy, z)| (dx / scale, dy / scale, z))
        .collect();
    match least_squares(&scaled, 3)?.as_slice() {
        [h, gx, gy] => Some([*h, gx / scale, gy / scale]),
        _ => None,
    }
}

/// A quadratic `[h, gx, gy, a, b, c]` about the origin.
fn fit_quadratic(points: &[(f64, f64, f64)], scale: f64) -> Option<[f64; 6]> {
    let scaled: Vec<(f64, f64, f64)> = points
        .iter()
        .map(|&(dx, dy, z)| (dx / scale, dy / scale, z))
        .collect();
    match least_squares(&scaled, 6)?.as_slice() {
        [h, gx, gy, a, b, c] => Some([
            *h,
            gx / scale,
            gy / scale,
            a / (scale * scale),
            b / (scale * scale),
            c / (scale * scale),
        ]),
        _ => None,
    }
}

/// The Hessian `[[2a, b], [b, 2c]]` of `a·dx² + b·dx·dy + c·dy²` with both eigenvalues clipped to
/// `±1 / LOT_MIN_RADIUS_M`, as the new `(a, b, c)`.
fn cap_curvature(xx: f64, xy: f64, yy: f64) -> (f64, f64, f64) {
    let cap = 1.0 / LOT_MIN_RADIUS_M;
    let (e1, e2) = eigenvalues(xx, xy, yy);
    if e1.abs() <= cap && e2.abs() <= cap {
        return (xx, xy, yy);
    }
    let theta = 0.5 * (2.0 * xy).atan2(2.0 * xx - 2.0 * yy);
    let (sin, cos) = theta.sin_cos();
    let (k1, k2) = (e1.clamp(-cap, cap), e2.clamp(-cap, cap));
    (
        f64::midpoint(k1 * cos * cos, k2 * sin * sin),
        (k1 - k2) * cos * sin,
        f64::midpoint(k1 * sin * sin, k2 * cos * cos),
    )
}

/// The eigenvalues, larger first, of the Hessian of `a·dx² + b·dx·dy + c·dy²`.
fn eigenvalues(xx: f64, xy: f64, yy: f64) -> (f64, f64) {
    let (hxx, hyy) = (2.0 * xx, 2.0 * yy);
    let mean = f64::midpoint(hxx, hyy);
    let spread = ((hxx - hyy) / 2.0).hypot(xy);
    (mean + spread, mean - spread)
}

// ----- vertices -----

/// The terrain vertices (column, row) inside the lot with a finite height, on the map, in column-then-row order.
fn vertices_in(area: &LotArea, base: &Heightmap) -> Vec<(i64, i64)> {
    let cell = f64::from(base.cell_m);
    if base.size == 0 || !(cell.is_finite() && cell > 0.0) {
        return Vec::new();
    }
    let last = i64::from(base.size) - 1;
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for &(x, y) in &area.outer {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    let first_col = to_index((min_x / cell).floor()).clamp(0, last);
    let last_col = to_index((max_x / cell).ceil()).clamp(0, last);
    let first_row = to_index((min_y / cell).floor()).clamp(0, last);
    let last_row = to_index((max_y / cell).ceil()).clamp(0, last);
    let mut found = Vec::new();
    for col in first_col..=last_col {
        for row in first_row..=last_row {
            let p = (to_f64(col) * cell, to_f64(row) * cell);
            if contains(area, p) && ground(base, (col, row)).is_some() {
                found.push((col, row));
            }
        }
    }
    found
}

/// The original ground at a vertex, if it is on the map and finite.
fn ground(base: &Heightmap, (col, row): (i64, i64)) -> Option<f64> {
    let col = u32::try_from(col).ok().filter(|c| *c < base.size)?;
    let row = u32::try_from(row).ok().filter(|r| *r < base.size)?;
    Some(f64::from(base.at(col, row))).filter(|h| h.is_finite())
}

/// The share of the lot's 1 m samples on the map (`0..=extent` metres) that are in water; 0 when none are on the
/// map (a lot off the map has area 0).
fn water_share(area: &LotArea, extent: f64, in_water: WaterTest<'_>) -> f64 {
    // a heightmap with a negative or non-finite cell has no map to count on
    if !(extent.is_finite() && extent >= 0.0) {
        return 0.0;
    }
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for &(x, y) in &area.outer {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    let cap = to_index(extent.floor());
    let first_col = to_index(min_x.floor()).clamp(0, cap);
    let last_col = to_index(max_x.ceil()).clamp(0, cap);
    let first_row = to_index(min_y.floor()).clamp(0, cap);
    let last_row = to_index(max_y.ceil()).clamp(0, cap);
    let (mut total, mut wet) = (0_u64, 0_u64);
    for col in first_col..=last_col {
        for row in first_row..=last_row {
            let p = (to_f64(col) + 0.5, to_f64(row) + 0.5);
            if p.0 > extent || p.1 > extent || !contains(area, p) {
                continue;
            }
            total += 1;
            if in_water(p) {
                wet += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        // counts of samples on a map, far below 2^53
        #[expect(clippy::cast_precision_loss, reason = "sample counts are small")]
        let share = wet as f64 / total as f64;
        share
    }
}

// ----- touching -----

fn bounds(ring: &[(f64, f64)]) -> (f64, f64, f64, f64) {
    ring.iter().fold(
        (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ),
        |(x0, y0, x1, y1), &(x, y)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
    )
}

fn cross(o: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

fn opposite(a: f64, b: f64) -> bool {
    (a > 0.0 && b < 0.0) || (a < 0.0 && b > 0.0)
}

/// The distance between two segments: 0 if they cross, else the nearest end to the other segment.
fn segment_gap(p1: (f64, f64), p2: (f64, f64), q1: (f64, f64), q2: (f64, f64)) -> f64 {
    if opposite(cross(q1, q2, p1), cross(q1, q2, p2))
        && opposite(cross(p1, p2, q1), cross(p1, p2, q2))
    {
        return 0.0;
    }
    segment_distance(p1, q1, q2)
        .min(segment_distance(p2, q1, q2))
        .min(segment_distance(q1, p1, p2))
        .min(segment_distance(q2, p1, p2))
}

/// Whether two lots touch or overlap: an outline point of one inside the other, or edges within `tol` metres.
pub(crate) fn touches(a: &LotArea, b: &LotArea, tol: f64) -> bool {
    let (ax0, ay0, ax1, ay1) = bounds(&a.outer);
    let (bx0, by0, bx1, by1) = bounds(&b.outer);
    if ax1 + tol < bx0 || bx1 + tol < ax0 || ay1 + tol < by0 || by1 + tol < ay0 {
        return false;
    }
    if a.outer.iter().any(|&p| contains(b, p)) || b.outer.iter().any(|&p| contains(a, p)) {
        return true;
    }
    let rings = |lot: &LotArea| -> Vec<Vec<(f64, f64)>> {
        std::iter::once(lot.outer.clone())
            .chain(lot.holes.iter().cloned())
            .collect()
    };
    for ra in rings(a) {
        for rb in rings(b) {
            for sa in ra.windows(2) {
                for sb in rb.windows(2) {
                    if let ([p1, p2], [q1, q2]) = (sa, sb)
                        && segment_gap(*p1, *p2, *q1, *q2) <= tol
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

// ----- relief -----

/// The local relief of a set of terrain vertices: for each vertex the difference between its height and the
/// plane fitted to the listed vertices within `LOT_RELIEF_LENGTH_M / 2` of it (itself included), as the 95th
/// percentile (nearest rank) and the largest over the vertices. `heights` is asked only for listed vertices and
/// may refuse (`None`); a vertex with fewer than three such neighbours has no relief. Vertex (`col`, `row`) lies at
/// (`col`, `row`) · `cell_m`.
#[must_use]
pub fn local_relief(
    heights: &dyn Fn(i64, i64) -> Option<f64>,
    vertices: &[(i64, i64)],
    cell_m: f64,
) -> (f64, f64) {
    if !(cell_m.is_finite() && cell_m > 0.0) {
        return (0.0, 0.0);
    }
    let set: BTreeSet<(i64, i64)> = vertices.iter().copied().collect();
    let reach = LOT_RELIEF_LENGTH_M / 2.0;
    let steps = to_index((reach / cell_m).floor()).clamp(0, 64);
    let mut residuals = Vec::with_capacity(set.len());
    for &(col, row) in &set {
        let Some(here) = heights(col, row).filter(|h| h.is_finite()) else {
            continue;
        };
        let mut points = Vec::new();
        for dr in -steps..=steps {
            for dc in -steps..=steps {
                let (dx, dy) = (to_f64(dc) * cell_m, to_f64(dr) * cell_m);
                if dx.hypot(dy) > reach + 1e-9 || !set.contains(&(col + dc, row + dr)) {
                    continue;
                }
                if let Some(h) = heights(col + dc, row + dr).filter(|h| h.is_finite()) {
                    points.push((dx, dy, h));
                }
            }
        }
        let residual = if points.len() < 3 {
            0.0
        } else {
            fit_plane(&points, reach).map_or(0.0, |[h, _, _]| (here - h).abs())
        };
        residuals.push(residual);
    }
    let largest = max_of(&residuals);
    (percentile95(residuals), largest)
}

// ----- the fit -----

/// Whether any vertex with all eight neighbours in the set is higher or lower than all of them, by more than noise.
fn has_interior_extremum(vertices: &BTreeSet<(i64, i64)>, cell: f64, surface: &Surface) -> bool {
    let at = |(c, r): (i64, i64)| surface.height_at((to_f64(c) * cell, to_f64(r) * cell));
    vertices.iter().any(|&(col, row)| {
        let neighbours: Vec<(i64, i64)> = (-1..=1)
            .flat_map(|dr| (-1..=1).map(move |dc| (dc, dr)))
            .filter(|&d| d != (0, 0))
            .map(|(dc, dr)| (col + dc, row + dr))
            .collect();
        if !neighbours.iter().all(|n| vertices.contains(n)) {
            return false;
        }
        let here = at((col, row));
        let diffs: Vec<f64> = neighbours.iter().map(|&n| at(n) - here).collect();
        diffs.iter().all(|d| *d > NOISE_M) || diffs.iter().all(|d| *d < -NOISE_M)
    })
}

/// Whether the quadratic's stationary point is within two cells of the group while its Hessian is definite: an
/// interior hump or hollow, which a lot must not have. Inside means inside a member's outer ring: a summit in a
/// large hole counts.
fn definite_near_group(surface: &Surface, members: &[&LotArea], cell: f64) -> bool {
    let [_, gx, gy, xx, xy, yy] = surface.coeffs;
    let (e1, e2) = eigenvalues(xx, xy, yy);
    if e1 * e2 <= 0.0 || e1.abs().min(e2.abs()) < 1e-9 {
        return false;
    }
    let (hxx, hyy) = (2.0 * xx, 2.0 * yy);
    let det = hxx * hyy - xy * xy;
    let (dx, dy) = (-(hyy * gx - xy * gy) / det, -(hxx * gy - xy * gx) / det);
    let stop = (surface.at.0 + dx, surface.at.1 + dy);
    members
        .iter()
        .any(|m| ring_contains(&m.outer, stop) || edge_distance(m, stop) <= 2.0 * cell)
}

/// Fits the group's surface to the ground at `vertices`; `None` if the points cannot be fitted at all.
fn fit_surface(
    vertices: &BTreeSet<(i64, i64)>,
    base: &Heightmap,
    members: &[&LotArea],
) -> Option<Surface> {
    let cell = f64::from(base.cell_m);
    let points: Vec<(f64, f64, f64)> = vertices
        .iter()
        .filter_map(|&v| Some((to_f64(v.0) * cell, to_f64(v.1) * cell, ground(base, v)?)))
        .collect();
    let count = to_f64(i64::try_from(points.len()).ok()?);
    if points.len() < 3 {
        return None;
    }
    let at = (
        points.iter().map(|p| p.0).sum::<f64>() / count,
        points.iter().map(|p| p.1).sum::<f64>() / count,
    );
    let local: Vec<(f64, f64, f64)> = points
        .iter()
        .map(|&(x, y, z)| (x - at.0, y - at.1, z))
        .collect();
    let scale = local
        .iter()
        .fold(1.0_f64, |s, &(dx, dy, _)| s.max(dx.abs()).max(dy.abs()));
    let [h, gx, gy] = fit_plane(&local, scale)?;
    let plane = Surface {
        at,
        coeffs: [h, gx, gy, 0.0, 0.0, 0.0],
        form: Form::Plane,
        lambda: 1.0,
    };
    let misfit: Vec<f64> = local
        .iter()
        .map(|&(x, y, z)| (z - plane.height_at((at.0 + x, at.1 + y))).abs())
        .collect();
    if misfit.is_empty() || percentile95(misfit) <= LOT_PLANE_FIT_M {
        return Some(plane);
    }
    if local.len() < QUADRATIC_MIN_VERTICES {
        return Some(plane);
    }
    let Some([_, _, _, a, b, c]) = fit_quadratic(&local, scale) else {
        return Some(plane);
    };
    let (a, b, c) = cap_curvature(a, b, c);
    let rest: Vec<(f64, f64, f64)> = local
        .iter()
        .map(|&(x, y, z)| (x, y, z - (a * x * x + b * x * y + c * y * y)))
        .collect();
    let Some([h, gx, gy]) = fit_plane(&rest, scale) else {
        return Some(plane);
    };
    let quadratic = Surface {
        at,
        coeffs: [h, gx, gy, a, b, c],
        form: Form::Quadratic,
        lambda: 1.0,
    };
    if definite_near_group(&quadratic, members, cell)
        || has_interior_extremum(vertices, cell, &quadratic)
    {
        return Some(plane);
    }
    Some(quadratic)
}

// ----- assess -----

/// What was found out about one lot before the groups are made.
struct Entry<'a> {
    area: &'a LotArea,
    /// `None` for a lot that is a candidate for smoothing.
    outcome: Option<Outcome>,
    vertices: Vec<(i64, i64)>,
    water: f64,
}

fn unsmoothed(entry: &Entry<'_>, outcome: Outcome) -> Assessed {
    Assessed {
        area: entry.area.clone(),
        group: NO_GROUP,
        outcome,
        steep: false,
        move_p95_m: f64::NAN,
        move_max_m: f64::NAN,
        slope_p95: f64::NAN,
        relief_before_m: f64::NAN,
        relief_after_m: f64::NAN,
        range_before: (f64::NAN, f64::NAN),
        range_after: (f64::NAN, f64::NAN),
        water_share: entry.water,
    }
}

fn first_look<'a>(area: &'a LotArea, base: &Heightmap, in_water: WaterTest<'_>) -> Entry<'a> {
    let mut entry = Entry {
        area,
        outcome: None,
        vertices: Vec::new(),
        water: f64::NAN,
    };
    if area.placement == Placement::NotAtGround {
        entry.outcome = Some(Outcome::Excluded {
            reason: "not_at_ground",
        });
        return entry;
    }
    let extent = f64::from(base.size.saturating_sub(1)) * f64::from(base.cell_m);
    entry.water = water_share(area, extent, in_water);
    if entry.water > LOT_WATER_MAX {
        entry.outcome = Some(Outcome::Excluded { reason: "in_water" });
    } else if area.placement == Placement::StreetSide {
        entry.outcome = Some(Outcome::SurfaceOnly {
            reason: "street_side",
        });
    } else {
        entry.vertices = vertices_in(area, base);
        if entry.vertices.len() < LOT_MIN_VERTICES {
            entry.outcome = Some(Outcome::SurfaceOnly {
                reason: "no_vertex",
            });
        }
    }
    entry
}

/// The groups of candidates (indices into `entries`, each sorted, ordered by the lowest index): lots that touch
/// or overlap, directly or through others.
fn groups_of(entries: &[Entry<'_>]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (i, me) in entries.iter().enumerate() {
        if me.outcome.is_some() {
            continue;
        }
        let hits: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter(|(_, g)| {
                g.iter().any(|&j| {
                    entries
                        .get(j)
                        .is_some_and(|other| touches(me.area, other.area, LOT_INSIDE_TOL_M))
                })
            })
            .map(|(k, _)| k)
            .collect();
        let mut merged = vec![i];
        for &k in hits.iter().rev() {
            merged.extend(groups.remove(k));
        }
        merged.sort_unstable();
        groups.push(merged);
    }
    groups.sort_by_key(|g| g.first().copied());
    groups
}

/// A vertex with its ground, its result, the move and the slope of the surface there.
type Row = ((i64, i64), f64, f64, f64, f64);

/// One member's figures against its group's surface.
fn assess_member(entry: &Entry<'_>, group: usize, surface: &Surface, base: &Heightmap) -> Assessed {
    let cell = f64::from(base.cell_m);
    let rows: Vec<Row> = entry
        .vertices
        .iter()
        .filter_map(|&v| {
            let p = (to_f64(v.0) * cell, to_f64(v.1) * cell);
            let before = ground(base, v)?;
            let after = before + surface.lambda * (surface.height_at(p) - before);
            let (gx, gy) = surface.gradient_at(p);
            Some((v, before, after, (after - before).abs(), gx.hypot(gy)))
        })
        .collect();
    let moves: Vec<f64> = rows.iter().map(|r| r.3).collect();
    let slopes: Vec<f64> = rows.iter().map(|r| r.4).collect();
    let range = |pick: &dyn Fn(&Row) -> f64| {
        rows.iter()
            .map(pick)
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                (lo.min(v), hi.max(v))
            })
    };
    let results: std::collections::BTreeMap<(i64, i64), (f64, f64)> =
        rows.iter().map(|r| (r.0, (r.1, r.2))).collect();
    let listed: Vec<(i64, i64)> = results.keys().copied().collect();
    let relief_before = local_relief(&|c, r| results.get(&(c, r)).map(|v| v.0), &listed, cell).1;
    let relief_after = local_relief(&|c, r| results.get(&(c, r)).map(|v| v.1), &listed, cell).1;
    let slope_p95 = percentile95(slopes);
    Assessed {
        area: entry.area.clone(),
        group,
        outcome: Outcome::Smoothed { form: surface.form },
        steep: slope_p95 > LOT_STEEP,
        move_p95_m: percentile95(moves.clone()),
        move_max_m: max_of(&moves),
        slope_p95,
        relief_before_m: relief_before,
        relief_after_m: relief_after,
        range_before: range(&|r| r.1),
        range_after: range(&|r| r.2),
        water_share: entry.water,
    }
}

/// Decides what becomes of every lot, groups the lots that touch and fits one surface per group.
///
/// In this order, the first rule that applies: `NotAtGround` is excluded (`"not_at_ground"`); more than
/// [`LOT_WATER_MAX`] of the lot's samples in water is excluded (`"in_water"`); a street-side lot is surface only
/// (`"street_side"`); fewer than [`LOT_MIN_VERTICES`] terrain vertices inside is surface only (`"no_vertex"`);
/// everything else is smoothed. Lots that touch (edges within [`LOT_INSIDE_TOL_M`], or one inside the other) form
/// one group, and only smoothed lots are in groups. The surface is fitted to the vertices inside the union of the
/// members, each vertex once, so the same area mapped twice does not count twice. A surface that would move a
/// vertex by more than [`LOT_MAX_MOVE_M`] is scaled (`Surface::lambda`).
///
/// The result is independent of the order of `areas`: the [`Assessed`] list is sorted by lot id and the groups
/// are numbered by their lowest member id. `base` is read only at vertices on the map.
#[must_use]
pub fn assess(
    areas: &[LotArea],
    base: &Heightmap,
    in_water: WaterTest<'_>,
) -> (Vec<Assessed>, Vec<LotSurface>) {
    let mut sorted: Vec<&LotArea> = areas.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    let entries: Vec<Entry<'_>> = sorted
        .into_iter()
        .map(|area| first_look(area, base, in_water))
        .collect();
    let mut assessed: Vec<Option<Assessed>> = entries
        .iter()
        .map(|e| e.outcome.map(|outcome| unsmoothed(e, outcome)))
        .collect();
    let cell = f64::from(base.cell_m);
    let mut surfaces = Vec::new();
    for members in groups_of(&entries) {
        let group_entries: Vec<&Entry<'_>> =
            members.iter().filter_map(|&i| entries.get(i)).collect();
        let vertices: BTreeSet<(i64, i64)> = group_entries
            .iter()
            .flat_map(|e| e.vertices.iter().copied())
            .collect();
        let areas: Vec<&LotArea> = group_entries.iter().map(|e| e.area).collect();
        let Some(mut surface) = fit_surface(&vertices, base, &areas) else {
            // cannot happen with three finite vertices; leave the lot's vertices unshaped and say why
            for (&i, e) in members.iter().zip(&group_entries) {
                if let Some(slot) = assessed.get_mut(i) {
                    *slot = Some(unsmoothed(
                        e,
                        Outcome::SurfaceOnly {
                            reason: "no_vertex",
                        },
                    ));
                }
            }
            continue;
        };
        let largest = vertices
            .iter()
            .filter_map(|&v| {
                let before = ground(base, v)?;
                let p = (to_f64(v.0) * cell, to_f64(v.1) * cell);
                Some((surface.height_at(p) - before).abs())
            })
            .fold(0.0, f64::max);
        if largest > LOT_MAX_MOVE_M {
            surface.lambda = LOT_MAX_MOVE_M / largest;
        }
        let number = surfaces.len();
        for (&i, e) in members.iter().zip(&group_entries) {
            if let Some(slot) = assessed.get_mut(i) {
                *slot = Some(assess_member(e, number, &surface, base));
            }
        }
        surfaces.push(LotSurface {
            group: number,
            members: group_entries.iter().map(|e| e.area.id.clone()).collect(),
            polygons: group_entries
                .iter()
                .map(|e| (e.area.outer.clone(), e.area.holes.clone()))
                .collect(),
            surface,
        });
    }
    (assessed.into_iter().flatten().collect(), surfaces)
}

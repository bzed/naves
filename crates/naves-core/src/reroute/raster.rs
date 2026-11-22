// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The obstacle raster of a leg: what a planned road must keep clear of (spec: obstacles and the distance raster,
//! clean-room notes 3.2).
//!
//! One raster is built per leg over its box. Cells are stamped as water, as another road's earthwork, as the map
//! edge, and as the cells the original line touches; a two-pass chamfer transform then gives the distance of every
//! cell to water, to roads and to the original line. A segment is tested by walking every cell it touches with an
//! exact grid traversal (Amanatides and Woo). Wall lines (streams, exempt roads) are tested segment against
//! segment, not on the raster.
//!
//! All input is untrusted: NaN and infinite coordinates are answered with an error or ignored, never a panic.

use crate::smooth::WaterTest;

use super::{CHAMFER_ERROR, SAMPLE_M};

const F_WATER: u8 = 1;
const F_ROAD: u8 = 2;
const F_EDGE: u8 = 4;
const F_ORIG: u8 = 8;

/// The saturated distance value (6553.5 m, in decimetres).
const FAR: u16 = u16::MAX;
/// How far inside a cell a second grid walk is offset, in cells, so that a segment that runs along a grid line
/// touches the cells on both sides of it.
const WALK_EPS: f64 = 1e-7;
/// Slack when a stored distance is compared with a threshold, in decimetres: the water cap is computed from the
/// same stored value, and rounding must not make the line it came from fail.
const COMPARE_EPS_DM: f64 = 1e-6;
/// The most cells `chamfer` will transform.
const CHAMFER_MAX_CELLS: usize = 1 << 28;
/// The most samples of one line.
const MAX_SAMPLES: f64 = 1.0e7;
/// A proper crossing is at least this far from the ends of both segments, as a fraction of their length.
pub(crate) const CROSS_EPS: f64 = 1e-9;

/// What blocked a segment. The order is the order of the checks of a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Blocked {
    /// Water, or too near to it.
    Water,
    /// Another road's earthwork, or too near to it.
    Road,
    /// A cell outside the map or inside its margin.
    Edge,
    /// A wall line (a stream, an exempt road's centre line).
    Wall,
    /// An end of the segment is outside the raster, or is not a number. The search treats it as a step not
    /// taken; the final check of a route as a failed check; it is never turned into a reason for keeping a zone
    /// (`held`, `blocked_by_*`).
    Outside,
}

/// The clearances of a segment test, in metres. Zero means only obstacle cells block. Built by
/// [`Clearance::new`], which refuses a value that is not a number or is negative.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clearance {
    water_m: f64,
    road_m: f64,
}

impl Clearance {
    /// No clearance: only obstacle cells block.
    pub const NONE: Clearance = Clearance {
        water_m: 0.0,
        road_m: 0.0,
    };

    /// The clearance to water and to other roads; `None` if either is not finite or is negative. The water test
    /// is also limited by the original line's own distance, see [`Obstacles::water_cap_m`].
    #[must_use]
    pub fn new(water_m: f64, road_m: f64) -> Option<Clearance> {
        let ok = |v: f64| v.is_finite() && v >= 0.0;
        (ok(water_m) && ok(road_m)).then_some(Clearance { water_m, road_m })
    }

    /// The clearance to water, in metres.
    #[must_use]
    pub fn water_m(&self) -> f64 {
        self.water_m
    }

    /// The clearance to other roads, in metres.
    #[must_use]
    pub fn road_m(&self) -> f64 {
        self.road_m
    }
}

/// The obstacle raster of a leg with its distance maps.
#[derive(Debug, Clone)]
pub struct Obstacles {
    min: (f64, f64),
    res: f64,
    cols: usize,
    rows: usize,
    cols_f: f64,
    rows_f: f64,
    flags: Vec<u8>,
    water: Vec<u16>,
    road: Vec<u16>,
    orig: Vec<u16>,
    water_cap: Option<f64>,
}

/// A cell count as `f64` (exact: a raster has far fewer than 2^32 cells per side).
fn fl(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// `floor(v)` as an index in `0..n`: below 0 and NaN give 0, above the end gives the last.
fn index_clamped(v: f64, n: usize) -> usize {
    let last = n.saturating_sub(1);
    if v.is_nan() || v <= 0.0 {
        return 0;
    }
    if v >= fl(n) {
        return last;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked to be in 0..n just above"
    )]
    let i = v.floor() as usize;
    i.min(last)
}

fn finite(p: (f64, f64)) -> bool {
    p.0.is_finite() && p.1.is_finite()
}

/// Smallest raster of at most `max_cells` cells at the resolution `res`, grown by 1.25 until it fits.
fn fit_grid(width: f64, height: f64, mut res: f64, max_cells: usize) -> (f64, usize, usize) {
    let max = fl(max_cells.max(1));
    let (width, height) = if width.is_finite() && height.is_finite() {
        (width.max(0.0), height.max(0.0))
    } else {
        (0.0, 0.0)
    };
    for _ in 0..400 {
        let cols = (width / res).ceil().max(1.0);
        let rows = (height / res).ceil().max(1.0);
        if cols * rows <= max {
            return (
                res,
                index_clamped(cols, usize::MAX),
                index_clamped(rows, usize::MAX),
            );
        }
        res *= 1.25;
    }
    (res, 1, 1)
}

impl Obstacles {
    /// A raster over the box `min`..`max` for terrain cells of `cell_m` metres. The map is `map_extent_m` square
    /// from the origin; cells outside it, and cells within `edge_margin_m` of its edge, are obstacle cells. The
    /// resolution is half the terrain cell, 1 to 2.5 m, multiplied by 1.25 until the raster has at most
    /// `max_cells` cells.
    #[must_use]
    pub fn new(
        min: (f64, f64),
        max: (f64, f64),
        cell_m: f64,
        map_extent_m: f64,
        edge_margin_m: f64,
        max_cells: usize,
    ) -> Obstacles {
        let min = if finite(min) { min } else { (0.0, 0.0) };
        let res0 = if cell_m.is_finite() {
            (cell_m / 2.0).clamp(1.0, 2.5)
        } else {
            2.0
        };
        let (res, cols, rows) = fit_grid(max.0 - min.0, max.1 - min.1, res0, max_cells);
        let margin = if edge_margin_m.is_finite() {
            edge_margin_m.max(0.0)
        } else {
            0.0
        };
        let inside = |v: f64| v >= margin && v <= map_extent_m - margin;
        let col_ok: Vec<bool> = (0..cols)
            .map(|i| inside(min.0 + (fl(i) + 0.5) * res))
            .collect();
        let mut flags = Vec::with_capacity(cols.saturating_mul(rows));
        for j in 0..rows {
            let row_ok = inside(min.1 + (fl(j) + 0.5) * res);
            flags.extend(
                col_ok
                    .iter()
                    .map(|&ok| if ok && row_ok { 0 } else { F_EDGE }),
            );
        }
        let n = flags.len();
        Obstacles {
            min,
            res,
            cols,
            rows,
            cols_f: fl(cols),
            rows_f: fl(rows),
            flags,
            water: vec![FAR; n],
            road: vec![FAR; n],
            orig: vec![FAR; n],
            water_cap: None,
        }
    }

    /// The resolution of the raster, in metres.
    #[must_use]
    pub fn resolution_m(&self) -> f64 {
        self.res
    }

    /// The number of cells.
    #[must_use]
    pub fn cells(&self) -> usize {
        self.flags.len()
    }

    fn centre(&self, col: usize, row: usize) -> (f64, f64) {
        (
            self.min.0 + (fl(col) + 0.5) * self.res,
            self.min.1 + (fl(row) + 0.5) * self.res,
        )
    }

    /// The raster covers this point (its outer edge included).
    fn covers(&self, p: (f64, f64)) -> bool {
        finite(p)
            && p.0 >= self.min.0
            && p.1 >= self.min.1
            && p.0 <= self.min.0 + self.cols_f * self.res
            && p.1 <= self.min.1 + self.rows_f * self.res
    }

    fn index_of(&self, p: (f64, f64)) -> Option<usize> {
        if !self.covers(p) {
            return None;
        }
        let col = index_clamped((p.0 - self.min.0) / self.res, self.cols);
        let row = index_clamped((p.1 - self.min.1) / self.res, self.rows);
        Some(row * self.cols + col)
    }

    /// The flat index of the cell at the float cell coordinates, if it is in the raster.
    fn flat(&self, cx: f64, cy: f64) -> Option<usize> {
        (cx >= 0.0 && cy >= 0.0 && cx < self.cols_f && cy < self.rows_f)
            .then(|| index_clamped(cy, self.rows) * self.cols + index_clamped(cx, self.cols))
    }

    /// One exact grid walk (Amanatides and Woo) of the segment, its coordinates moved by `eps` cells. `visit`
    /// gets the flat index of every cell in the raster and returns false to stop. Both ends must be covered.
    fn walk(
        &self,
        a: (f64, f64),
        b: (f64, f64),
        eps: f64,
        visit: &mut dyn FnMut(usize) -> bool,
    ) -> bool {
        let ga = (
            (a.0 - self.min.0) / self.res + eps,
            (a.1 - self.min.1) / self.res + eps,
        );
        let gb = (
            (b.0 - self.min.0) / self.res + eps,
            (b.1 - self.min.1) / self.res + eps,
        );
        let (mut cx, mut cy) = (ga.0.floor(), ga.1.floor());
        let (ex, ey) = (gb.0.floor(), gb.1.floor());
        let (dx, dy) = (gb.0 - ga.0, gb.1 - ga.1);
        let (sx, sy) = (
            if dx < 0.0 { -1.0 } else { 1.0 },
            if dy < 0.0 { -1.0 } else { 1.0 },
        );
        let (tdx, tdy) = (
            if dx.abs() > 0.0 {
                1.0 / dx.abs()
            } else {
                f64::INFINITY
            },
            if dy.abs() > 0.0 {
                1.0 / dy.abs()
            } else {
                f64::INFINITY
            },
        );
        let mut tx = if dx > 0.0 {
            (cx + 1.0 - ga.0) / dx
        } else if dx < 0.0 {
            (ga.0 - cx) / -dx
        } else {
            f64::INFINITY
        };
        let mut ty = if dy > 0.0 {
            (cy + 1.0 - ga.1) / dy
        } else if dy < 0.0 {
            (ga.1 - cy) / -dy
        } else {
            f64::INFINITY
        };
        let limit = (ex - cx).abs() + (ey - cy).abs() + 2.0;
        let mut steps = 0.0;
        loop {
            if let Some(i) = self.flat(cx, cy)
                && !visit(i)
            {
                return false;
            }
            steps += 1.0;
            if ((cx - ex).abs() < 0.5 && (cy - ey).abs() < 0.5) || steps > limit {
                return true;
            }
            if tx < ty {
                cx += sx;
                tx += tdx;
            } else {
                cy += sy;
                ty += tdy;
            }
        }
    }

    /// Visits every cell the segment touches, a segment along a grid line touching the cells on both sides.
    /// A segment with an end outside the raster is not walked. Returns false if `visit` stopped it.
    fn touches(&self, a: (f64, f64), b: (f64, f64), visit: &mut dyn FnMut(usize) -> bool) -> bool {
        if !self.covers(a) || !self.covers(b) {
            return true;
        }
        self.walk(a, b, WALK_EPS, visit) && self.walk(a, b, -WALK_EPS, visit)
    }

    /// The part of the segment inside the raster (Liang and Barsky), if any.
    fn clip(&self, a: (f64, f64), b: (f64, f64)) -> Option<((f64, f64), (f64, f64))> {
        if !finite(a) || !finite(b) {
            return None;
        }
        let hi = (
            self.min.0 + self.cols_f * self.res,
            self.min.1 + self.rows_f * self.res,
        );
        let (d, mut t0, mut t1) = ((b.0 - a.0, b.1 - a.1), 0.0_f64, 1.0_f64);
        for (p, q) in [
            (-d.0, a.0 - self.min.0),
            (d.0, hi.0 - a.0),
            (-d.1, a.1 - self.min.1),
            (d.1, hi.1 - a.1),
        ] {
            if p.abs() > 0.0 {
                let r = q / p;
                if p < 0.0 {
                    t0 = t0.max(r);
                } else {
                    t1 = t1.min(r);
                }
            } else if q < 0.0 {
                return None;
            }
        }
        // rounding may put an end a hair outside the box: clamp it in
        let inside = |t: f64| {
            (
                (a.0 + t * d.0).clamp(self.min.0, hi.0),
                (a.1 + t * d.1).clamp(self.min.1, hi.1),
            )
        };
        (t0 <= t1).then_some((inside(t0), inside(t1)))
    }

    /// Marks the cells the (clipped) segment touches.
    fn mark_segment(&mut self, a: (f64, f64), b: (f64, f64), flag: u8) {
        let Some((a, b)) = self.clip(a, b) else {
            return;
        };
        let mut hit = Vec::new();
        self.touches(a, b, &mut |i| {
            hit.push(i);
            true
        });
        for i in hit {
            if let Some(f) = self.flags.get_mut(i) {
                *f |= flag;
            }
        }
    }

    /// Marks every cell whose centre `in_water` says is in water.
    pub fn stamp_water(&mut self, in_water: WaterTest<'_>) {
        for row in 0..self.rows {
            for col in 0..self.cols {
                if in_water(self.centre(col, row))
                    && let Some(f) = self.flags.get_mut(row * self.cols + col)
                {
                    *f |= F_WATER;
                }
            }
        }
    }

    /// Stamps another road's earthwork: every cell within `reach_m` of the line, and every cell the line touches.
    /// `exempt` holds one flag per sample of the line (see [`samples`], from [`exempt_samples`]); the piece of the
    /// line between two consecutive samples is stamped only when neither is exempt (a flag missing counts as not
    /// exempt). Cells are kept by their distance to the segments of the line, so a corner whose vertex is not a
    /// sample keeps its tip.
    pub fn stamp_road(&mut self, line: &[(f64, f64)], reach_m: f64, exempt: &[bool]) {
        let samples = samples_with_arc(line);
        let reach = if reach_m.is_finite() {
            reach_m.max(0.0)
        } else {
            0.0
        };
        let skip = |k: usize| exempt.get(k).copied().unwrap_or(false);
        if let [(only, _)] = samples.as_slice() {
            if !skip(0) {
                self.stamp_segment(*only, *only, reach);
            }
            return;
        }
        let vertices = vertices_with_arc(line);
        for (k, pair) in samples.windows(2).enumerate() {
            let [(a, sa), (b, sb)] = pair else {
                continue;
            };
            if skip(k) || skip(k + 1) {
                continue;
            }
            // the piece from sample to sample, through the vertices between them
            let mut from = *a;
            for &(v, sv) in &vertices {
                if sv > *sa && sv < *sb {
                    self.stamp_segment(from, v, reach);
                    from = v;
                }
            }
            self.stamp_segment(from, *b, reach);
        }
    }

    /// Marks the cells whose centre is within `reach` of the segment, and the cells the segment touches.
    fn stamp_segment(&mut self, a: (f64, f64), b: (f64, f64), reach: f64) {
        if !finite(a) || !finite(b) {
            return;
        }
        let hi = (
            self.min.0 + self.cols_f * self.res,
            self.min.1 + self.rows_f * self.res,
        );
        let (lo_x, hi_x) = (a.0.min(b.0) - reach, a.0.max(b.0) + reach);
        let (lo_y, hi_y) = (a.1.min(b.1) - reach, a.1.max(b.1) + reach);
        if hi_x < self.min.0 || hi_y < self.min.1 || lo_x > hi.0 || lo_y > hi.1 {
            return;
        }
        let cols = (
            index_clamped((lo_x - self.min.0) / self.res, self.cols),
            index_clamped((hi_x - self.min.0) / self.res, self.cols),
        );
        let rows = (
            index_clamped((lo_y - self.min.1) / self.res, self.rows),
            index_clamped((hi_y - self.min.1) / self.res, self.rows),
        );
        for row in rows.0..=rows.1 {
            for col in cols.0..=cols.1 {
                if seg_dist(self.centre(col, row), a, b) <= reach
                    && let Some(f) = self.flags.get_mut(row * self.cols + col)
                {
                    *f |= F_ROAD;
                }
            }
        }
        // a reach smaller than a cell still marks the cells the line touches
        self.mark_segment(a, b, F_ROAD);
    }

    /// Marks the cells the leg's original line touches, for the stray distance.
    pub fn set_original(&mut self, line: &[(f64, f64)]) {
        if let [only] = line
            && let Some(i) = self.index_of(*only)
            && let Some(f) = self.flags.get_mut(i)
        {
            *f |= F_ORIG;
        }
        for pair in line.windows(2) {
            if let [a, b] = *pair {
                self.mark_segment(a, b, F_ORIG);
            }
        }
    }

    /// Computes the three distance maps and the water cap from what was stamped. Call it after the last stamp
    /// and after [`Obstacles::set_original`].
    pub fn finish(&mut self) {
        let (cols, rows, res) = (self.cols, self.rows, self.res);
        let mask = |bit: u8| -> Vec<bool> { self.flags.iter().map(|&f| f & bit != 0).collect() };
        let (water, road, orig) = (
            chamfer(&mask(F_WATER), cols, rows, res),
            chamfer(&mask(F_ROAD), cols, rows, res),
            chamfer(&mask(F_ORIG), cols, rows, res),
        );
        // the cap: the smallest stored distance to water of the cells the original line touches
        self.water_cap = self
            .flags
            .iter()
            .zip(&water)
            .filter(|&(&f, _)| f & F_ORIG != 0)
            .map(|(_, &d)| d)
            .min()
            .map(|d| f64::from(d) / 10.0);
        self.water = water;
        self.road = road;
        self.orig = orig;
    }

    /// The distance a segment must have from an obstacle cell for the clearance `clearance_m`, in metres, as
    /// the chamfer distance measures it: `(c + res) · 1.0824 + res · √2`, which keeps at least the clearance from
    /// the real area. Zero for a clearance of zero or less: then only obstacle cells block. The one threshold
    /// for the segment test, the exemption and the water cap.
    #[must_use]
    pub fn threshold(&self, clearance_m: f64) -> f64 {
        if clearance_m.is_nan() || clearance_m <= 0.0 {
            0.0
        } else {
            (clearance_m + self.res) * CHAMFER_ERROR + self.res * std::f64::consts::SQRT_2
        }
    }

    /// Whether a planned segment from `a` to `b` is free: `Err` names the first thing in its way. An end outside
    /// the raster or not a number is `Outside`; `Edge` is only for cells outside the map or in its margin. The
    /// water test uses the threshold of the water clearance, but no more than [`Obstacles::water_cap_m`].
    ///
    /// # Errors
    ///
    /// The kind of obstacle in the way, see [`Blocked`].
    pub fn segment(&self, a: (f64, f64), b: (f64, f64), clear: Clearance) -> Result<(), Blocked> {
        if !self.covers(a) || !self.covers(b) {
            return Err(Blocked::Outside);
        }
        let water_thr = self
            .threshold(clear.water_m)
            .min(self.water_cap.unwrap_or(f64::INFINITY));
        let (road_dm, water_dm) = (self.threshold(clear.road_m) * 10.0, water_thr * 10.0);
        let mut result = Ok(());
        self.touches(a, b, &mut |i| {
            let blocked = self.cell_blocked(i, water_dm, road_dm);
            if let Some(kind) = blocked {
                result = Err(kind);
            }
            blocked.is_none()
        });
        result
    }

    fn cell_blocked(&self, i: usize, water_dm: f64, road_dm: f64) -> Option<Blocked> {
        let flags = self.flags.get(i).copied().unwrap_or(F_EDGE);
        if flags & F_EDGE != 0 {
            return Some(Blocked::Edge);
        }
        if flags & F_ROAD != 0 {
            return Some(Blocked::Road);
        }
        if flags & F_WATER != 0 {
            return Some(Blocked::Water);
        }
        let near = |map: &[u16], limit: f64| {
            limit > 0.0
                && map
                    .get(i)
                    .is_some_and(|&d| f64::from(d) + COMPARE_EPS_DM < limit)
        };
        if near(&self.road, road_dm) {
            return Some(Blocked::Road);
        }
        near(&self.water, water_dm).then_some(Blocked::Water)
    }

    /// The distance map value of the point's cell in metres, with `shift` added (not below 0). `INFINITY`
    /// outside the raster; a saturated cell gives exactly 6553.5 whatever the shift.
    fn distance_at(&self, map: &[u16], p: (f64, f64), shift: f64) -> f64 {
        let Some(&raw) = self.index_of(p).and_then(|i| map.get(i)) else {
            return f64::INFINITY;
        };
        if raw == FAR {
            return f64::from(FAR) / 10.0;
        }
        (f64::from(raw) / 10.0 + shift).max(0.0)
    }

    /// Distance of the point to the original line: the stored chamfer distance of the point's cell, as it is,
    /// the measure the stray weight and the stray cap are calibrated on. It is **not** an upper bound of the
    /// true distance (it may fall short by about a cell), so the search and the final check of a route must
    /// use this same measure for the cap. `INFINITY` outside the raster; saturated at 6553.5 m.
    #[must_use]
    pub fn stray_m(&self, p: (f64, f64)) -> f64 {
        self.distance_at(&self.orig, p, 0.0)
    }

    /// An estimate of the distance of the point to water, for diagnostics: the chamfer distance of the cell
    /// centres less half a cell. It can be off by about a cell and the chamfer excess in either direction
    /// (11 m for a true 10 m in the tests); never use it for a clearance test, that is [`Obstacles::segment`].
    /// `INFINITY` outside the raster; saturated at 6553.5 m.
    #[must_use]
    pub fn water_distance_m(&self, p: (f64, f64)) -> f64 {
        self.distance_at(&self.water, p, -self.res / 2.0)
    }

    /// An estimate of the distance of the point to other roads' earthwork, measured and limited as
    /// [`Obstacles::water_distance_m`]; for diagnostics, never for a clearance test. Saturated at 6553.5 m.
    #[must_use]
    pub fn road_distance_m(&self, p: (f64, f64)) -> f64 {
        self.distance_at(&self.road, p, -self.res / 2.0)
    }

    /// The water cap: the smallest stored chamfer distance to water, in metres, of the cells the original line
    /// touches, kept by [`Obstacles::finish`]. `segment` tests water against `min(threshold(water clearance),
    /// cap)` in this same stored measure, so the original line passes its own test exactly and a road beside
    /// water may stay as near as it was, however near that is. An original that touches water gives 0 (only
    /// water cells block then). `None` when no original was set, or none of it is in the raster: no cap.
    #[must_use]
    pub fn water_cap_m(&self) -> Option<f64> {
        self.water_cap
    }
}

/// The two-pass chamfer distance transform of a mask of `cols` x `rows` cells (row by row): the distance of
/// every cell to the nearest `true` cell, orthogonal steps weighing `res_m` and diagonal steps `res_m · √2`,
/// in decimetres, saturating at `u16::MAX` (6553.5 m; an empty mask is all `u16::MAX`). A mask shorter than the
/// grid counts as `false` beyond its end; a grid of more than 2^28 cells gives an empty result.
#[must_use]
pub fn chamfer(obstacle: &[bool], cols: usize, rows: usize, res_m: f64) -> Vec<u16> {
    let Some(n) = cols.checked_mul(rows).filter(|&n| n <= CHAMFER_MAX_CELLS) else {
        return Vec::new();
    };
    let (orth, diag) = (res_m, res_m * std::f64::consts::SQRT_2);
    let mut d: Vec<f64> = (0..n)
        .map(|i| {
            if obstacle.get(i) == Some(&true) {
                0.0
            } else {
                f64::INFINITY
            }
        })
        .collect();
    let at = |d: &[f64], i: usize| d.get(i).copied().unwrap_or(f64::INFINITY);
    for y in 0..rows {
        for x in 0..cols {
            let i = y * cols + x;
            let mut v = at(&d, i);
            if x > 0 {
                v = v.min(at(&d, i - 1) + orth);
            }
            if y > 0 {
                v = v.min(at(&d, i - cols) + orth);
                if x > 0 {
                    v = v.min(at(&d, i - cols - 1) + diag);
                }
                if x + 1 < cols {
                    v = v.min(at(&d, i - cols + 1) + diag);
                }
            }
            if let Some(slot) = d.get_mut(i) {
                *slot = v;
            }
        }
    }
    for y in (0..rows).rev() {
        for x in (0..cols).rev() {
            let i = y * cols + x;
            let mut v = at(&d, i);
            if x + 1 < cols {
                v = v.min(at(&d, i + 1) + orth);
            }
            if y + 1 < rows {
                v = v.min(at(&d, i + cols) + orth);
                if x + 1 < cols {
                    v = v.min(at(&d, i + cols + 1) + diag);
                }
                if x > 0 {
                    v = v.min(at(&d, i + cols - 1) + diag);
                }
            }
            if let Some(slot) = d.get_mut(i) {
                *slot = v;
            }
        }
    }
    d.into_iter().map(to_decimetres).collect()
}

fn to_decimetres(metres: f64) -> u16 {
    if !metres.is_finite() {
        return FAR;
    }
    let dm = (metres * 10.0).round();
    if dm >= f64::from(FAR) {
        return FAR;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a distance is not negative and is below u16::MAX just above"
    )]
    let v = dm.max(0.0) as u16;
    v
}

fn seg_dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
}

fn poly_dist(p: (f64, f64), line: &[(f64, f64)]) -> f64 {
    match line {
        [] => f64::INFINITY,
        [only] => (p.0 - only.0).hypot(p.1 - only.1),
        _ => line
            .windows(2)
            .filter_map(|s| Some(seg_dist(p, *s.first()?, *s.get(1)?)))
            .fold(f64::INFINITY, f64::min),
    }
}

/// The points of `line` every [`SAMPLE_M`] metres from its start, then its end, each with its distance along
/// the line. A line with a coordinate that is not a number, or longer than 20 000 km, has none.
pub(crate) fn samples_with_arc(line: &[(f64, f64)]) -> Vec<((f64, f64), f64)> {
    if !line.iter().all(|&p| finite(p)) {
        return Vec::new();
    }
    let total: f64 = line
        .windows(2)
        .filter_map(|s| {
            let (a, b) = (*s.first()?, *s.get(1)?);
            Some((b.0 - a.0).hypot(b.1 - a.1))
        })
        .sum();
    if !total.is_finite() || total / SAMPLE_M > MAX_SAMPLES {
        return Vec::new();
    }
    let Some(&first) = line.first() else {
        return Vec::new();
    };
    let mut out = vec![(first, 0.0)];
    let (mut start, mut next) = (0.0, SAMPLE_M);
    for pair in line.windows(2) {
        let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        while len > 0.0 && next <= start + len {
            let t = (next - start) / len;
            out.push(((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)), next));
            next += SAMPLE_M;
        }
        start += len;
    }
    if let Some(&last) = line.last()
        && out.last().is_none_or(|&(_, s)| s < total - CROSS_EPS)
    {
        out.push((last, total));
    }
    out
}

/// The vertices of `line` with their distance along it, leaving out repeated points.
fn vertices_with_arc(line: &[(f64, f64)]) -> Vec<((f64, f64), f64)> {
    let mut out: Vec<((f64, f64), f64)> = Vec::new();
    let mut arc = 0.0;
    for &p in line {
        match out.last() {
            Some(&(q, _)) => {
                let len = (p.0 - q.0).hypot(p.1 - q.1);
                if len > 0.0 {
                    arc += len;
                    out.push((p, arc));
                }
            }
            None => out.push((p, 0.0)),
        }
    }
    out
}

/// The samples of a road, every 2 m from its start and its end: the points [`exempt_samples`] answers for.
/// Empty for a line with a coordinate that is not a number.
#[must_use]
pub fn samples(line: &[(f64, f64)]) -> Vec<(f64, f64)> {
    samples_with_arc(line).into_iter().map(|(p, _)| p).collect()
}

/// For each of the [`samples`] of `other`: whether it is exempt, that is within `near_m` of the `original` line,
/// or within `extend_m` along `other` of a sample that is. Another road's earthwork is not an obstacle where
/// the original line already came as near.
#[must_use]
pub fn exempt_samples(
    other: &[(f64, f64)],
    original: &[(f64, f64)],
    near_m: f64,
    extend_m: f64,
) -> Vec<bool> {
    let pts = samples_with_arc(other);
    let near: Vec<bool> = pts
        .iter()
        .map(|&(p, _)| poly_dist(p, original) <= near_m)
        .collect();
    let mut out = near.clone();
    let reach = |from: f64, at: f64| (at - from).abs() <= extend_m + CROSS_EPS;
    let mut last: Option<f64> = None;
    for (k, &(_, s)) in pts.iter().enumerate() {
        if near.get(k) == Some(&true) {
            last = Some(s);
        }
        if last.is_some_and(|l| reach(l, s))
            && let Some(o) = out.get_mut(k)
        {
            *o = true;
        }
    }
    last = None;
    for (k, &(_, s)) in pts.iter().enumerate().rev() {
        if near.get(k) == Some(&true) {
            last = Some(s);
        }
        if last.is_some_and(|l| reach(l, s))
            && let Some(o) = out.get_mut(k)
        {
            *o = true;
        }
    }
    out
}

/// The point where the segments `a1`-`a2` and `b1`-`b2` cross, if they cross. A crossing is a change of side: of
/// the ends of each segment against the line of the other. An end that lies exactly on that line counts as on the
/// non-positive side, the same way for every segment that shares it, so a route that passes through a vertex of its own
/// polyline, or of the wall, is counted once (by the one of its two segments that changes the side), not never and
/// not twice. A route that only touches the line from the positive side is counted twice, from the other side not at
/// all, and a route that ends on the line from the positive side once: that needs coordinates that agree exactly and
/// does not happen on measured data.
pub(crate) fn cross_point(
    a1: (f64, f64),
    a2: (f64, f64),
    b1: (f64, f64),
    b2: (f64, f64),
) -> Option<(f64, f64)> {
    let (da, db) = ((a2.0 - a1.0, a2.1 - a1.1), (b2.0 - b1.0, b2.1 - b1.1));
    let den = da.0 * db.1 - da.1 * db.0;
    if den.is_nan() || den.abs() < 1e-12 {
        return None;
    }
    // the side of a point of one segment against the line of the other, written the same way for every use of the point
    let side = |origin: (f64, f64), d: (f64, f64), p: (f64, f64)| {
        d.0 * (p.1 - origin.1) - d.1 * (p.0 - origin.0) > 0.0
    };
    if side(a1, da, b1) == side(a1, da, b2) || side(b1, db, a1) == side(b1, db, a2) {
        return None;
    }
    let along_a = (((b1.0 - a1.0) * db.1 - (b1.1 - a1.1) * db.0) / den).clamp(0.0, 1.0);
    Some((a1.0 + along_a * da.0, a1.1 + along_a * da.1))
}

/// What kind of line a [`Wall`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallKind {
    /// The centre line of an exempt road.
    Road,
    /// A stream.
    Stream,
}

/// A line the route may cross only as often as the original line did.
#[derive(Debug, Clone, PartialEq)]
pub struct Wall {
    /// The line.
    pub line: Vec<(f64, f64)>,
    /// How many times the route may cross it, counted away from the anchors.
    pub allowed: usize,
    /// What the line is.
    pub kind: WallKind,
    /// The two anchors of the leg.
    pub anchors: [(f64, f64); 2],
    /// A crossing this near (or nearer) to either anchor neither counts nor blocks, as in [`crossings_away`].
    pub end_m: f64,
}

impl Wall {
    /// Whether the segment is blocked: the wall allows no crossing and the segment crosses its line farther
    /// than `end_m` from both anchors.
    #[must_use]
    pub fn blocks(&self, a: (f64, f64), b: (f64, f64)) -> bool {
        self.allowed == 0
            && self.line.windows(2).any(|s| {
                matches!(s, [r, t] if cross_point(a, b, *r, *t)
                    .is_some_and(|x| away_from(x, self.anchors, self.end_m)))
            })
    }
}

impl Wall {
    /// How often `route` crosses the wall's line farther than `end_m` from both of the wall's anchors: the count
    /// the leg's original line is held to. The route check must count with the same `Wall` that the search
    /// blocked with, so that the two cannot use different anchors.
    #[must_use]
    pub fn crossings(&self, route: &[(f64, f64)]) -> usize {
        crossings_away(route, &self.line, self.anchors, self.end_m)
    }
}

/// The point is farther than `end_m` from both anchors.
fn away_from(x: (f64, f64), anchors: [(f64, f64); 2], end_m: f64) -> bool {
    anchors
        .iter()
        .all(|e| (e.0 - x.0).hypot(e.1 - x.1) > end_m + CROSS_EPS)
}

/// How often `route` crosses `line` farther than `end_m` from both `anchors` (a crossing near an anchor is
/// not counted: a road that shares geometry with the original line near its node made the count unstable).
#[must_use]
pub fn crossings_away(
    route: &[(f64, f64)],
    line: &[(f64, f64)],
    anchors: [(f64, f64); 2],
    end_m: f64,
) -> usize {
    crossings_where(route, line, &|x| away_from(x, anchors, end_m))
}

/// How often `route` crosses `line` at a point `keep` accepts: the one crossing count of the walls, the exemption
/// and the decks, so that two counts of the same line cannot drift apart.
#[must_use]
pub fn crossings_where(
    route: &[(f64, f64)],
    line: &[(f64, f64)],
    keep: &dyn Fn((f64, f64)) -> bool,
) -> usize {
    let mut n = 0;
    for a in route.windows(2) {
        for b in line.windows(2) {
            if let ([p, q], [r, s]) = (a, b)
                && let Some(x) = cross_point(*p, *q, *r, *s)
                && keep(x)
            {
                n += 1;
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::Obstacles;

    /// A fixed-seed generator, so that the test is the same on every run.
    fn next(state: &mut u64) -> f64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        // 53 bits to a number in 0..1
        f64::from(u32::try_from(*state >> 32).unwrap()) / 4_294_967_296.0
    }

    #[test]
    fn a_clipped_segment_lies_inside_the_raster() {
        let o = Obstacles::new((0.0, 0.0), (200.0, 200.0), 4.0, 200.0, 0.0, 4_000_000);
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut clipped = 0;
        for _ in 0..20_000 {
            // awkward coordinates: scattered round the box, some on its boundary
            let mut point = || {
                let v = |s: &mut u64| {
                    let r = next(s);
                    if r < 0.2 {
                        200.0
                    } else if r < 0.3 {
                        0.0
                    } else {
                        -100.0 + 400.0 * next(s) + 1e-13
                    }
                };
                (v(&mut state), v(&mut state))
            };
            let (a, b) = (point(), point());
            if let Some((c, d)) = o.clip(a, b) {
                clipped += 1;
                assert!(o.covers(c) && o.covers(d), "{a:?} {b:?} -> {c:?} {d:?}");
            }
        }
        assert!(clipped > 1000);
    }
}

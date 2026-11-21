// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The cost-to-go field of a leg: for every cell of a coarse grid over the leg's box, how far it is, by a road
//! that the obstacles, the walls and the terrain allow, from the leg's end anchor. The arc search of the leg uses it
//! as its estimate of the road still to build (spec: "The cost-to-go field", clean-room notes 3.4).
//!
//! The field is the band with the grade limit removed. Then the band stops nothing by height, because at an infinite
//! grade every sample may be reached from the one before it, and cut and fill never bind. What shapes the field is
//! the segment test (obstacles, edge, clearances), the terrain (a sample with no height blocks), the walls that
//! allow no crossing and the stray cap (tested at the cell centres). An edge therefore costs one segment test and
//! the terrain at the band's samples, the same answer as `band_step` with an infinite grade (a test pins this) at
//! none of its allocations. It is a lower bound as far as heights go, but only up to the grid's discretisation in
//! the plane: a corridor that no 6 m centre-to-centre segment fits, and the stray cap tested at centres instead of
//! at samples, can make the estimate too high in narrow places. An estimate of 0 does not mean the point is
//! reached: it is also what an unreached cell, an empty field and a target inside an obstacle give.
//!
//! Dijkstra runs with an ordered heap (`total_cmp`, then the cell index), so two builds agree bit for bit. The
//! open set holds one entry per strict improvement of a cell, at most 8 per cell. Crates were considered
//! (`pathfinding`, `petgraph`) and not used: `pathfinding::dijkstra_all` takes its edges lazily, but needs an `Ord`
//! cost (`f64` is not), returns a hash map instead of a flat array, and tests all 8 edges of every cell, including
//! those to settled cells, about twice as many edge tests as relaxing first and testing only on improvement.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use super::band::Band;
use super::raster::{Clearance, Obstacles, Wall};
use super::{CHAMFER_ERROR, PLAN_STEP_M};

/// How often the resolution is multiplied by 1.25 at most while it is made to fit.
const MAX_GROWTHS: usize = 400;
/// Spacing of the terrain samples of an edge when the band's is not usable, in metres (as the band's own).
const SPACING_DEFAULT: f64 = 2.0;
/// Most terrain samples of one edge (as the band's own).
const MAX_SAMPLES: usize = 1_000_000;
/// Tolerance in the number of samples of an edge (as the band's own).
const SLACK: f64 = 1e-9;

/// A cell count as `f64` (exact: a field has at most `max_cells` cells, far below 2^32 per side).
fn fl(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// `v` as a count, if it is a whole number in `0..=limit`.
fn count(v: f64, limit: usize) -> Option<usize> {
    if v.is_finite() && v >= 0.0 && v <= fl(limit) {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "checked to be in 0..=limit just above"
        )]
        let n = v.floor() as usize;
        Some(n)
    } else {
        None
    }
}

/// The cost-to-go field of one leg.
#[derive(Debug, Clone)]
pub struct Field {
    /// Left and lower edge of the grid.
    origin: (f64, f64),
    res: f64,
    cols: usize,
    rows: usize,
    /// Cost to go of each cell, row by row; `INFINITY` where the cell cannot reach the target.
    value: Vec<f64>,
}

/// The grid of a field: its resolution, origin and size, with the target at the centre of cell
/// `(target_col, target_row)`.
struct Grid {
    res: f64,
    origin: (f64, f64),
    cols: usize,
    rows: usize,
    target: (usize, usize),
}

/// The grid of at most `max_cells` cells that covers the box and the target, starting at resolution `res` and
/// multiplied by 1.25 until it fits. The target is at the centre of a cell, so that its start cost is exactly 0.
fn layout(
    lo: (f64, f64),
    hi: (f64, f64),
    target: (f64, f64),
    mut res: f64,
    max_cells: usize,
) -> Option<Grid> {
    let max = fl(max_cells.max(1));
    for _ in 0..MAX_GROWTHS {
        // cells to the lower and the higher side of the target's cell, per axis
        let side = |t: f64, lo: f64, hi: f64| {
            let below = ((t - lo) / res - 0.5).ceil().max(0.0);
            let above = ((hi - t) / res - 0.5).ceil().max(0.0);
            (below, above)
        };
        let (bx, ax) = side(target.0, lo.0, hi.0);
        let (by, ay) = side(target.1, lo.1, hi.1);
        let (cols_f, rows_f) = (bx + ax + 1.0, by + ay + 1.0);
        if cols_f * rows_f <= max {
            let cols = count(cols_f, max_cells.max(1))?;
            let rows = count(rows_f, max_cells.max(1))?;
            return Some(Grid {
                res,
                origin: (target.0 - (bx + 0.5) * res, target.1 - (by + 0.5) * res),
                cols,
                rows,
                target: (count(bx, cols)?, count(by, rows)?),
            });
        }
        res *= 1.25;
    }
    None
}

/// An entry of the open set, ordered so that the `BinaryHeap` pops the lowest cost, then the lowest cell index.
#[derive(Debug, Clone, Copy)]
struct Entry {
    cost: f64,
    cell: usize,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Entry {}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| other.cell.cmp(&self.cell))
    }
}

/// The eight neighbours as column and row offsets.
const NEIGHBOURS: [(isize, isize); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

impl Field {
    /// The field of a leg. `min` and `max` are the corners of the leg's box and `target` is the end anchor; the
    /// grid covers the box and the target, with the target at the centre of a cell, at the resolution
    /// `max(step_m, cell_m)` multiplied by 1.25 until there are at most `max_cells` cells. A `max_cells` above
    /// `u32::MAX` counts as `u32::MAX` (about 34 GB of values), so it is for a trusted constant only. The open set
    /// of the Dijkstra holds at most 8 entries per cell.
    ///
    /// An edge between neighbouring cell centres is allowed if no wall blocks it, the segment test passes, there is
    /// terrain at the band's samples along it (the band with the grade limit removed; see the module), and both
    /// of its cells are within `stray_cap_m` of the original line. It is tested from the farther cell towards the
    /// nearer one, the direction of travel, and its cost is its length. A box or target that is not a number, a
    /// box too large to fit, or a band the band test refuses gives an empty field or the target cell alone, whose
    /// estimate is 0 everywhere.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "the inputs of a leg, as the spec lists them"
    )]
    pub fn build(
        min: (f64, f64),
        max: (f64, f64),
        target: (f64, f64),
        step_m: f64,
        cell_m: f64,
        band: &Band,
        ground: &dyn Fn((f64, f64)) -> Option<f64>,
        obstacles: &Obstacles,
        walls: &[Wall],
        clear: Clearance,
        stray_cap_m: f64,
        max_cells: usize,
    ) -> Field {
        let empty = Field {
            origin: (0.0, 0.0),
            res: 1.0,
            cols: 0,
            rows: 0,
            value: Vec::new(),
        };
        if !(target.0.is_finite() && target.1.is_finite()) {
            return empty;
        }
        let corners = [min, max];
        let (lo, hi) = if corners.iter().all(|c| c.0.is_finite() && c.1.is_finite()) {
            (
                (min.0.min(max.0), min.1.min(max.1)),
                (min.0.max(max.0), min.1.max(max.1)),
            )
        } else {
            (target, target)
        };
        let res0 = [step_m, cell_m]
            .into_iter()
            .filter(|v| v.is_finite())
            .fold(0.0, f64::max);
        let res0 = if res0 > 0.0 { res0 } else { PLAN_STEP_M };
        let Some(grid) = layout(lo, hi, target, res0, max_cells) else {
            return empty;
        };
        let mut field = Field {
            origin: grid.origin,
            res: grid.res,
            cols: grid.cols,
            rows: grid.rows,
            value: vec![f64::INFINITY; grid.cols.saturating_mul(grid.rows)],
        };
        // The band's own validity rules (a grade that is NaN or negative, a cut, fill or tolerance that is not a
        // finite, non-negative number) refuse every edge there, so the field is the target cell alone.
        let finite = |v: f64| v.is_finite() && v >= 0.0;
        let usable = band.grade >= 0.0
            && finite(band.cut_m)
            && finite(band.fill_m)
            && finite(band.anchor_tol_m);
        if usable {
            field.dijkstra(
                grid.target,
                band.spacing_m,
                ground,
                obstacles,
                walls,
                clear,
                stray_cap_m,
            );
        } else if let Some(first) = field.value.get_mut(
            grid.target
                .1
                .saturating_mul(grid.cols)
                .saturating_add(grid.target.0),
        ) {
            *first = 0.0;
        }
        field
    }

    /// The centre of a cell.
    fn centre(&self, col: usize, row: usize) -> (f64, f64) {
        (
            self.origin.0 + (fl(col) + 0.5) * self.res,
            self.origin.1 + (fl(row) + 0.5) * self.res,
        )
    }

    /// The neighbour of a cell at an offset, if it is on the grid.
    fn neighbour(&self, col: usize, row: usize, d: (isize, isize)) -> Option<(usize, usize)> {
        let c = col.checked_add_signed(d.0).filter(|&c| c < self.cols)?;
        let r = row.checked_add_signed(d.1).filter(|&r| r < self.rows)?;
        Some((c, r))
    }

    /// Whether a road may be built from the centre `from` to the centre `to`: no wall blocks it, the segment test
    /// passes, and there is terrain at every sample of the band (`spacing_m` apart, the end last, placed as the
    /// band places them). This is `band_step` with an infinite grade, without its allocations.
    fn edge_ok(
        from: (f64, f64),
        to: (f64, f64),
        spacing_m: f64,
        ground: &dyn Fn((f64, f64)) -> Option<f64>,
        obstacles: &Obstacles,
        walls: &[Wall],
        clear: Clearance,
    ) -> bool {
        if walls.iter().any(|w| w.blocks(from, to)) || obstacles.segment(from, to, clear).is_err() {
            return false;
        }
        let has_ground = |p| ground(p).is_some_and(f64::is_finite);
        let len = (to.0 - from.0).hypot(to.1 - from.1);
        if !has_ground(from) {
            return false;
        }
        if !(len > 0.0 && len.is_finite()) {
            return true;
        }
        let spacing = if spacing_m.is_finite() && spacing_m > 0.0 {
            spacing_m
        } else {
            SPACING_DEFAULT
        };
        let n = count((len / spacing - SLACK).ceil().max(1.0), MAX_SAMPLES).unwrap_or(1);
        let ds = len / fl(n);
        (1..=n).all(|i| {
            let p = if i < n {
                let t = (fl(i) * ds / len).clamp(0.0, 1.0);
                (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t)
            } else {
                to
            };
            has_ground(p)
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the leg's inputs, passed on from build"
    )]
    fn dijkstra(
        &mut self,
        target: (usize, usize),
        spacing_m: f64,
        ground: &dyn Fn((f64, f64)) -> Option<f64>,
        obstacles: &Obstacles,
        walls: &[Wall],
        clear: Clearance,
        stray_cap_m: f64,
    ) {
        let cols = self.cols;
        let start = target.1.saturating_mul(cols).saturating_add(target.0);
        let Some(first) = self.value.get_mut(start) else {
            return;
        };
        *first = 0.0;
        let mut heap = BinaryHeap::new();
        heap.push(Entry {
            cost: 0.0,
            cell: start,
        });
        let within_cap = |p: (f64, f64)| obstacles.stray_m(p) <= stray_cap_m;
        while let Some(Entry { cost, cell }) = heap.pop() {
            if self.value.get(cell).is_none_or(|&v| cost > v) {
                continue;
            }
            let (col, row) = (cell % cols.max(1), cell / cols.max(1));
            let here = self.centre(col, row);
            if !within_cap(here) {
                continue;
            }
            for d in NEIGHBOURS {
                let Some((nc, nr)) = self.neighbour(col, row, d) else {
                    continue;
                };
                let next = nr.saturating_mul(cols).saturating_add(nc);
                let len = if d.0 == 0 || d.1 == 0 {
                    self.res
                } else {
                    self.res * std::f64::consts::SQRT_2
                };
                let through = cost + len;
                if self.value.get(next).is_none_or(|&v| through >= v) {
                    continue;
                }
                let there = self.centre(nc, nr);
                if within_cap(there)
                    && Self::edge_ok(there, here, spacing_m, ground, obstacles, walls, clear)
                    && let Some(v) = self.value.get_mut(next)
                {
                    *v = through;
                    heap.push(Entry {
                        cost: through,
                        cell: next,
                    });
                }
            }
        }
    }

    /// The estimate of the road still to build from the point `p`: the value of its cell divided by
    /// [`CHAMFER_ERROR`] (the chamfer distance is up to that much longer than the straight one), less one field
    /// cell (the point may be anywhere in its cell), not below 0. A point outside the field, not a number, or in
    /// a cell that cannot reach the target (enclosed by water, cut off by a wall or by missing terrain, or beyond the
    /// stray cap) gives 0, as do an empty field and a target inside an obstacle: 0 does not mean reached. Up to the
    /// grid's discretisation it is a lower bound of the road still to build.
    #[must_use]
    pub fn estimate(&self, p: (f64, f64)) -> f64 {
        let col = count(((p.0 - self.origin.0) / self.res).floor(), self.cols);
        let row = count(((p.1 - self.origin.1) / self.res).floor(), self.rows);
        let (Some(col), Some(row)) = (col, row) else {
            return 0.0;
        };
        if col >= self.cols || row >= self.rows {
            return 0.0;
        }
        match self.value.get(row * self.cols + col) {
            Some(&v) if v.is_finite() => (v / CHAMFER_ERROR - self.res).max(0.0),
            _ => 0.0,
        }
    }

    /// The number of cells of the grid.
    #[must_use]
    pub fn cells(&self) -> usize {
        self.value.len()
    }
}

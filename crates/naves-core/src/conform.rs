// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Shapes the terrain to fit the roads.
//!
//! The terrain is not smoothed towards a road. It is **held to a slope limit around it, at every grid
//! vertex**: what sticks out above the cutting slope is cut, what is below the embankment slope is filled.
//! Every vertex within half a road width and one and a half cells of a road's centre line takes the road's
//! height, so that every terrain triangle under the road has all three corners on it, whatever way the
//! engine splits a cell. Where the DEM is too coarse to show a valley the road crosses, the road gets a
//! bench.
//!
//! The earthworks reach only as far as they are connected to the road: a flood from the road through the
//! vertices that break the limits stops where the ground meets the slope again, so that a cliff beyond that
//! stays a cliff.

use std::collections::{BTreeMap, VecDeque};

use crate::heightmap::Heightmap;
use crate::profile::RoadStrip;

/// Rise over run of a cutting: 45°.
pub const CUT_SLOPE: f64 = 1.0;
/// Rise over run of an embankment: 34°.
pub const FILL_SLOPE: f64 = 0.667;
/// How far earthworks reach from a road's core, in metres.
pub const MAX_EARTHWORK_M: f64 = 60.0;
/// Cores of different roads that differ by more than this are a conflict.
const CONFLICT_M: f64 = 0.3;
/// Passes of averaging over the changed vertices.
const ROUNDING_PASSES: usize = 4;
/// Impossible limits are reported once per square of this many metres.
const CONFLICT_REGION_M: f64 = 50.0;

/// Two roads that want different heights at the same place.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadConflict {
    /// Where.
    pub at: (f64, f64),
    /// By how much the heights differ.
    pub height_difference_m: f64,
}

/// The shaped terrain and what it took.
#[derive(Debug, Clone, PartialEq)]
pub struct Conformed {
    /// The terrain with the earthworks.
    pub heightmap: Heightmap,
    /// The deepest cut.
    pub max_cut_m: f32,
    /// The highest fill.
    pub max_fill_m: f32,
    /// Places where two roads cannot both be met.
    pub conflicts: Vec<RoadConflict>,
    /// Places where the earthworks hit [`MAX_EARTHWORK_M`] and a step is left.
    pub capped: Vec<(f64, f64)>,
}

/// Why the terrain cannot be shaped.
#[derive(Debug, thiserror::Error)]
pub enum ConformError {
    /// The heightmap does not hold `size * size` heights.
    #[error("the heightmap has {len} heights but a size of {size}")]
    BadHeightmap {
        /// Vertices per side.
        size: u32,
        /// Heights present.
        len: usize,
    },
}

fn narrow(v: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "heights and slopes fit f32"
    )]
    let narrowed = v as f32;
    narrowed
}

/// The vertex grid: indices, positions and neighbours.
#[derive(Clone, Copy)]
struct Grid {
    size: usize,
    cell: f64,
}

impl Grid {
    fn count(self) -> usize {
        self.size * self.size
    }

    fn coord(value: usize) -> f64 {
        u32::try_from(value).map_or(f64::MAX, f64::from)
    }

    /// The position of a vertex in map metres.
    fn position(self, vertex: usize) -> (f64, f64) {
        (
            Self::coord(vertex % self.size) * self.cell,
            Self::coord(vertex / self.size) * self.cell,
        )
    }

    /// The column or row nearest to a position, clamped to the grid.
    fn nearest(self, metres: f64) -> usize {
        let last = Self::coord(self.size.saturating_sub(1));
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a grid index from a clamped position"
        )]
        let index = (metres / self.cell).clamp(0.0, last) as usize;
        index
    }

    /// The neighbour of `vertex` at the offset (`dc`, `dr`), if it is on the grid.
    fn neighbour(self, vertex: usize, dc: i64, dr: i64) -> Option<usize> {
        let span = i64::try_from(self.size).ok()?;
        let col = i64::try_from(vertex % self.size).ok()? + dc;
        let row = i64::try_from(vertex / self.size).ok()? + dr;
        if col < 0 || row < 0 || col >= span || row >= span {
            return None;
        }
        usize::try_from(row * span + col).ok()
    }
}

/// A vertex that belongs to a road's core.
#[derive(Clone, Copy)]
struct Core {
    height: f32,
    strip: usize,
}

/// The vertices within `reach` of the centre line of `strip`, with the height of the closest point.
fn core_vertices(strip: &RoadStrip, grid: Grid, reach: f64) -> BTreeMap<usize, (f64, f64)> {
    let mut found: BTreeMap<usize, (f64, f64)> = BTreeMap::new();
    let last = grid.size.saturating_sub(1);
    for pair in strip.joints.windows(2) {
        let [from, to] = pair else { continue };
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let length2 = dx * dx + dy * dy;
        let cols = (
            grid.nearest(from.0.min(to.0) - reach).saturating_sub(1),
            (grid.nearest(from.0.max(to.0) + reach) + 1).min(last),
        );
        let rows = (
            grid.nearest(from.1.min(to.1) - reach).saturating_sub(1),
            (grid.nearest(from.1.max(to.1) + reach) + 1).min(last),
        );
        for row in rows.0..=rows.1 {
            for col in cols.0..=cols.1 {
                let (px, py) = (Grid::coord(col) * grid.cell, Grid::coord(row) * grid.cell);
                let along = if length2 > 0.0 {
                    (((px - from.0) * dx + (py - from.1) * dy) / length2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let distance = (px - from.0 - along * dx).hypot(py - from.1 - along * dy);
                if distance <= reach {
                    let height = from.2 + along * (to.2 - from.2);
                    let slot = found
                        .entry(row * grid.size + col)
                        .or_insert((distance, height));
                    if distance < slot.0 {
                        *slot = (distance, height);
                    }
                }
            }
        }
    }
    found
}

/// The fields the earthworks are held to.
struct Fields {
    /// Distance to the nearest core in metres.
    dist: Vec<f32>,
    /// Index of the nearest core vertex.
    nearest: Vec<u32>,
    upper: Vec<f32>,
    lower: Vec<f32>,
}

impl Fields {
    /// Lets `vertex` take what is better of its own values and those of `from`, `step` metres away.
    fn relax(&mut self, vertex: usize, from: usize, step: f32) {
        let (Some(&dist), Some(&upper), Some(&lower), Some(&nearest)) = (
            self.dist.get(from),
            self.upper.get(from),
            self.lower.get(from),
            self.nearest.get(from),
        ) else {
            return;
        };
        if dist + step < self.dist.get(vertex).copied().unwrap_or(f32::INFINITY)
            && let (Some(d), Some(n)) = (self.dist.get_mut(vertex), self.nearest.get_mut(vertex))
        {
            *d = dist + step;
            *n = nearest;
        }
        if let Some(value) = self.upper.get_mut(vertex) {
            *value = value.min(upper + narrow(CUT_SLOPE) * step);
        }
        if let Some(value) = self.lower.get_mut(vertex) {
            *value = value.max(lower - narrow(FILL_SLOPE) * step);
        }
    }
}

/// Offsets that have been visited before a vertex in a forward sweep, and after it in a backward one.
const FORWARD: [(i64, i64); 4] = [(-1, 0), (-1, -1), (0, -1), (1, -1)];
const BACKWARD: [(i64, i64); 4] = [(1, 0), (1, 1), (0, 1), (-1, 1)];

/// Two sweeps of an eight-neighbour chamfer over the grid.
fn sweep(cores: &[Option<Core>], grid: Grid) -> Fields {
    let count = grid.count();
    let mut fields = Fields {
        dist: vec![f32::INFINITY; count],
        nearest: vec![u32::MAX; count],
        upper: vec![f32::INFINITY; count],
        lower: vec![f32::NEG_INFINITY; count],
    };
    for (index, core) in cores.iter().enumerate() {
        if let (Some(core), Some(d), Some(u), Some(l), Some(n)) = (
            core,
            fields.dist.get_mut(index),
            fields.upper.get_mut(index),
            fields.lower.get_mut(index),
            fields.nearest.get_mut(index),
        ) {
            *d = 0.0;
            *u = core.height;
            *l = core.height;
            *n = u32::try_from(index).unwrap_or(u32::MAX);
        }
    }
    let straight = narrow(grid.cell);
    let diagonal = narrow(grid.cell * std::f64::consts::SQRT_2);
    for backward in [false, true] {
        let offsets = if backward { &BACKWARD } else { &FORWARD };
        let order: Vec<usize> = if backward {
            (0..count).rev().collect()
        } else {
            (0..count).collect()
        };
        for vertex in order {
            for &(dc, dr) in offsets {
                if let Some(from) = grid.neighbour(vertex, dc, dr) {
                    let step = if dc != 0 && dr != 0 {
                        diagonal
                    } else {
                        straight
                    };
                    fields.relax(vertex, from, step);
                }
            }
        }
    }
    fields
}

/// Gives every vertex near a road the height of the road, the higher rank winning where two roads clash.
/// Returns the cores and, per pair of roads, the worst clash and where it is.
#[expect(
    clippy::type_complexity,
    reason = "a map from a pair of strips to (difference, place)"
)]
fn claim_cores(
    strips: &[RoadStrip],
    grid: Grid,
) -> (
    Vec<Option<Core>>,
    BTreeMap<(usize, usize), (f64, (f64, f64))>,
) {
    let mut order: Vec<usize> = (0..strips.len()).collect();
    order.sort_by(|&left, &right| {
        let rank = |i: usize| strips.get(i).map_or(0, |s| s.rank);
        rank(right).cmp(&rank(left)).then(left.cmp(&right))
    });
    let mut cores: Vec<Option<Core>> = vec![None; grid.count()];
    let mut clashes: BTreeMap<(usize, usize), (f64, (f64, f64))> = BTreeMap::new();
    for &index in &order {
        let Some(strip) = strips.get(index) else {
            continue;
        };
        let reach = strip.half_width + 1.5 * grid.cell;
        for (vertex, (_, height)) in core_vertices(strip, grid, reach) {
            let height = narrow(height);
            let Some(slot) = cores.get_mut(vertex) else {
                continue;
            };
            match slot {
                None => {
                    *slot = Some(Core {
                        height,
                        strip: index,
                    });
                }
                Some(owner) => {
                    let difference = f64::from((owner.height - height).abs());
                    if difference > CONFLICT_M && owner.strip != index {
                        let key = (owner.strip.min(index), owner.strip.max(index));
                        let worst = clashes
                            .entry(key)
                            .or_insert((difference, grid.position(vertex)));
                        if difference > worst.0 {
                            *worst = (difference, grid.position(vertex));
                        }
                    }
                }
            }
        }
    }
    (cores, clashes)
}

/// The vertices connected to a core through ground that breaks the limits, and the places where that
/// reaches [`MAX_EARTHWORK_M`].
fn flood(
    base: &Heightmap,
    cores: &[Option<Core>],
    fields: &Fields,
    grid: Grid,
) -> (Vec<bool>, Vec<(f64, f64)>) {
    let mut flooded = vec![false; grid.count()];
    let mut seen: Vec<bool> = cores.iter().map(Option::is_some).collect();
    let mut queue: VecDeque<usize> = cores
        .iter()
        .enumerate()
        .filter_map(|(index, core)| core.map(|_| index))
        .collect();
    let mut capped = Vec::new();
    while let Some(vertex) = queue.pop_front() {
        for dr in -1..=1_i64 {
            for dc in -1..=1_i64 {
                let Some(next) = grid
                    .neighbour(vertex, dc, dr)
                    .filter(|_| dc != 0 || dr != 0)
                else {
                    continue;
                };
                if seen.get(next).copied().unwrap_or(true) {
                    continue;
                }
                if let Some(flag) = seen.get_mut(next) {
                    *flag = true;
                }
                let ground = base.h.get(next).copied().unwrap_or(0.0);
                let upper = fields.upper.get(next).copied().unwrap_or(f32::INFINITY);
                let lower = fields.lower.get(next).copied().unwrap_or(f32::NEG_INFINITY);
                if ground <= upper && ground >= lower {
                    continue;
                }
                if f64::from(fields.dist.get(next).copied().unwrap_or(0.0)) > MAX_EARTHWORK_M {
                    capped.push(grid.position(next));
                    continue;
                }
                if let Some(flag) = flooded.get_mut(next) {
                    *flag = true;
                }
                queue.push_back(next);
            }
        }
    }
    (flooded, capped)
}

/// Shapes `base` to fit `strips`; `base` itself is not changed.
///
/// # Errors
/// [`ConformError::BadHeightmap`] if the heightmap is not `size * size`.
pub fn conform(base: &Heightmap, strips: &[RoadStrip]) -> Result<Conformed, ConformError> {
    let size = usize::try_from(base.size).unwrap_or(0);
    if base.h.len() != size * size {
        return Err(ConformError::BadHeightmap {
            size: base.size,
            len: base.h.len(),
        });
    }
    let grid = Grid {
        size,
        cell: f64::from(base.cell_m),
    };
    let (cores, clashes) = claim_cores(strips, grid);
    if cores.iter().all(Option::is_none) {
        return Ok(Conformed {
            heightmap: base.clone(),
            max_cut_m: 0.0,
            max_fill_m: 0.0,
            conflicts: Vec::new(),
            capped: Vec::new(),
        });
    }
    let fields = sweep(&cores, grid);
    let (flooded, capped) = flood(base, &cores, &fields, grid);
    let mut heights = base.h.clone();
    for (vertex, core) in cores.iter().enumerate() {
        if let (Some(core), Some(height)) = (core, heights.get_mut(vertex)) {
            *height = core.height;
        }
    }
    let impossible = clamp_flooded(base, &cores, &fields, &flooded, grid, &mut heights);
    round(base, &fields, &flooded, grid, &mut heights);
    let mut max_cut = 0.0_f32;
    let mut max_fill = 0.0_f32;
    for (new, old) in heights.iter().zip(&base.h) {
        max_cut = max_cut.max(old - new);
        max_fill = max_fill.max(new - old);
    }
    let mut conflicts: Vec<RoadConflict> = clashes
        .into_values()
        .map(|(difference, at)| RoadConflict {
            at,
            height_difference_m: difference,
        })
        .collect();
    conflicts.extend(impossible);
    Ok(Conformed {
        heightmap: Heightmap {
            size: base.size,
            cell_m: base.cell_m,
            h: heights,
        },
        max_cut_m: max_cut,
        max_fill_m: max_fill,
        conflicts,
        capped,
    })
}

/// The flooded vertices take the base height clamped into the limits. Between two roads that cannot both be
/// met (the limits do not overlap) a vertex follows the nearest road, and the clash is reported once per
/// region.
fn clamp_flooded(
    base: &Heightmap,
    cores: &[Option<Core>],
    fields: &Fields,
    flooded: &[bool],
    grid: Grid,
    heights: &mut [f32],
) -> Vec<RoadConflict> {
    let mut impossible: BTreeMap<(i64, i64), RoadConflict> = BTreeMap::new();
    for vertex in 0..grid.count() {
        if !flooded.get(vertex).copied().unwrap_or(false) {
            continue;
        }
        let ground = base.h.get(vertex).copied().unwrap_or(0.0);
        let upper = fields.upper.get(vertex).copied().unwrap_or(f32::INFINITY);
        let lower = fields
            .lower
            .get(vertex)
            .copied()
            .unwrap_or(f32::NEG_INFINITY);
        let value = if lower <= upper {
            ground.clamp(lower, upper)
        } else {
            let at = grid.position(vertex);
            let region = |metres: f64| {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "a region number of a map position"
                )]
                let number = (metres / CONFLICT_REGION_M).floor() as i64;
                number
            };
            let difference = f64::from(lower - upper);
            let entry = impossible
                .entry((region(at.0), region(at.1)))
                .or_insert(RoadConflict {
                    at,
                    height_difference_m: difference,
                });
            if difference > entry.height_difference_m {
                *entry = RoadConflict {
                    at,
                    height_difference_m: difference,
                };
            }
            let nearest = fields
                .nearest
                .get(vertex)
                .and_then(|&i| cores.get(usize::try_from(i).unwrap_or(usize::MAX)))
                .and_then(|core| *core);
            nearest.map_or(ground, |core| {
                let distance = fields.dist.get(vertex).copied().unwrap_or(0.0);
                core.height
                    + (ground - core.height)
                        .clamp(-narrow(FILL_SLOPE) * distance, narrow(CUT_SLOPE) * distance)
            })
        };
        if let Some(height) = heights.get_mut(vertex) {
            *height = value;
        }
    }
    impossible.into_values().collect()
}

/// Four passes of averaging over the changed vertices, clamped back into the limits after each pass.
fn round(base: &Heightmap, fields: &Fields, flooded: &[bool], grid: Grid, heights: &mut [f32]) {
    let changed: Vec<usize> = (0..grid.count())
        .filter(|&vertex| {
            flooded.get(vertex).copied().unwrap_or(false)
                && heights.get(vertex) != base.h.get(vertex)
        })
        .collect();
    for _ in 0..ROUNDING_PASSES {
        let previous = heights.to_vec();
        for &vertex in &changed {
            let around: Vec<f32> = [(-1, 0), (1, 0), (0, -1), (0, 1)]
                .iter()
                .filter_map(|&(dc, dr)| grid.neighbour(vertex, dc, dr))
                .filter_map(|next| previous.get(next).copied())
                .collect();
            if around.is_empty() {
                continue;
            }
            let mean = around.iter().sum::<f32>() / narrow(Grid::coord(around.len()));
            let own = previous.get(vertex).copied().unwrap_or(0.0);
            let averaged = 0.5 * own + 0.5 * mean;
            let upper = fields.upper.get(vertex).copied().unwrap_or(f32::INFINITY);
            let lower = fields
                .lower
                .get(vertex)
                .copied()
                .unwrap_or(f32::NEG_INFINITY);
            let value = if lower <= upper {
                averaged.clamp(lower, upper)
            } else {
                averaged
            };
            if let Some(height) = heights.get_mut(vertex) {
                *height = value;
            }
        }
    }
}

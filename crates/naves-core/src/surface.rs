// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ground class of every map cell. This module decides; `naves-export` only draws the result.

use rayon::prelude::*;

use crate::heightmap::{Heightmap, Sample};

/// Slope in degrees above which ground counts as rock.
const ROCK_SLOPE_DEG: f32 = 35.0;

/// Real altitude in metres above which bare ground takes over from grass. Measured with ESA `WorldCover` around
/// Berchtesgaden: 27 % of the ground is bare at 1900 to 2100 m, 58 % at 2100 to 2300 m, 87 % at 2300 to 2500 m.
const GRASS_LIMIT_REAL_M: f64 = 2100.0;

/// Largest side of a [`SurfaceGrid`] in cells.
const MAX_GRID_SIZE: u32 = 8192;

/// The ground surfaces; the slot is the layer slot (and with it the colour in the surface mask).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SurfaceClass {
    /// Grass (slot 0, black in the mask).
    Grass,
    /// Rock and bare ground (slot 1, red).
    Rock,
    /// Forest floor (slot 2, green).
    Forest,
    /// Soil: fields, built-up areas, water beds (slot 3, blue).
    Soil,
}

impl SurfaceClass {
    /// The layer slot, 0 to 3.
    #[must_use]
    pub fn slot(self) -> usize {
        match self {
            SurfaceClass::Grass => 0,
            SurfaceClass::Rock => 1,
            SurfaceClass::Forest => 2,
            SurfaceClass::Soil => 3,
        }
    }
}

/// A class per map cell, south row first.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceGrid {
    /// Cells per side.
    pub size: u32,
    /// Side of a cell in metres.
    pub cell_m: f64,
    cells: Vec<SurfaceClass>,
}

impl SurfaceGrid {
    /// A grid whose cell (`col`, `row`) has the class `f(col, row)`; row 0 is the southern row.
    #[must_use]
    pub fn from_fn(
        size: u32,
        cell_m: f64,
        f: impl Fn(u32, u32) -> SurfaceClass + Sync,
    ) -> SurfaceGrid {
        let expected = usize::try_from(size).unwrap_or(0).pow(2);
        let cells = (0..size)
            .into_par_iter()
            .flat_map_iter(|row| {
                let f = &f;
                (0..size).map(move |col| f(col, row))
            })
            .collect::<Vec<_>>();
        debug_assert_eq!(cells.len(), expected);
        SurfaceGrid {
            size,
            cell_m,
            cells,
        }
    }

    /// The class at the map position (`x`, `y`) in metres from the south-west corner: the nearest cell,
    /// clamped to the map.
    #[must_use]
    pub fn at(&self, x: f64, y: f64) -> SurfaceClass {
        let side = usize::try_from(self.size).unwrap_or(0);
        let index = |v: f64| {
            let last = side.saturating_sub(1);
            let cell = (v / self.cell_m).floor();
            if cell.is_nan() || cell <= 0.0 {
                0
            } else {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "positive and clamped to the grid below"
                )]
                let c = cell.min(f64::from(MAX_GRID_SIZE)) as usize;
                c.min(last)
            }
        };
        self.cells
            .get(index(y) * side + index(x))
            .copied()
            .unwrap_or(SurfaceClass::Grass)
    }
}

/// Cells per side of the [`SurfaceGrid`] of a map with `terrain_size` heightmap vertices:
/// `min(terrain_size * 4, 8192)`.
#[must_use]
pub fn surface_grid_size(terrain_size: u32) -> u32 {
    terrain_size.saturating_mul(4).min(MAX_GRID_SIZE)
}

fn narrow(v: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "heights and sizes are far inside the f32 range"
    )]
    let v = v as f32;
    v
}

/// The grass limit in map metres for a project's `scale.height`.
#[must_use]
pub fn grass_limit_m(height_scale: f64) -> f32 {
    narrow(GRASS_LIMIT_REAL_M * height_scale)
}

/// Rock on slopes above 35° and above `grass_limit` metres of map height, else grass.
#[must_use]
pub fn procedural_class(sample: &Sample, grass_limit: f32) -> SurfaceClass {
    if sample.slope_deg() > ROCK_SLOPE_DEG || sample.height > grass_limit {
        SurfaceClass::Rock
    } else {
        SurfaceClass::Grass
    }
}

/// The ground from slope and altitude alone.
#[must_use]
pub fn procedural_grid(hm: &Heightmap, grass_limit: f32) -> SurfaceGrid {
    let size = surface_grid_size(hm.size);
    let map_size_m = f64::from(hm.size) * f64::from(hm.cell_m);
    let cell_m = map_size_m / f64::from(size.max(1));
    SurfaceGrid::from_fn(size, cell_m, |col, row| {
        let x = (f64::from(col) + 0.5) * cell_m;
        let y = (f64::from(row) + 0.5) * cell_m;
        procedural_class(&hm.sample(x, y), grass_limit)
    })
}

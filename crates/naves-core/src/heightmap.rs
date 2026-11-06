// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The scaled heightmap sampled from a DEM.
//!
//! Vertex (`col`, `row`) sits at map position `(x, y) = (col, row) * cell_m`, x east and y north from
//! the south-west corner. The real-world offset from the area centre is
//! `((x, y) - map_size / 2) / scale.horizontal`, and the height is the DEM height times
//! `scale.height`. Rows are stored south row first.

use rayon::prelude::*;

use crate::dem::Dem;
use crate::project::Project;
use crate::projection::{LocalProjection, ProjectionError};

/// A square grid of heights.
#[derive(Debug, Clone, PartialEq)]
pub struct Heightmap {
    /// Vertices per side.
    pub size: u32,
    /// Distance between vertices in map metres.
    pub cell_m: f32,
    /// `size * size` heights, south row first.
    pub h: Vec<f32>,
}

impl Heightmap {
    /// Height at vertex (`col`, `row`); coordinates beyond the grid hold the nearest edge value.
    #[must_use]
    pub fn at(&self, col: u32, row: u32) -> f32 {
        let last = self.size.saturating_sub(1);
        let index = u64::from(row.min(last)) * u64::from(self.size) + u64::from(col.min(last));
        usize::try_from(index)
            .ok()
            .and_then(|i| self.h.get(i))
            .copied()
            .unwrap_or(0.0)
    }

    /// Lowest and highest height (both 0 for an empty map).
    #[must_use]
    pub fn min_max(&self) -> (f32, f32) {
        let min = self.h.iter().copied().min_by(f32::total_cmp);
        let max = self.h.iter().copied().max_by(f32::total_cmp);
        (min.unwrap_or(0.0), max.unwrap_or(0.0))
    }
}

/// Errors building the heightmap.
#[derive(Debug, thiserror::Error)]
pub enum HeightmapError {
    /// The DEM does not cover a heightmap vertex (or has no data there).
    #[error("no DEM data at {lat:.5},{lon:.5}")]
    MissingDem {
        /// Latitude of the first uncovered vertex in degrees.
        lat: f64,
        /// Longitude of the first uncovered vertex in degrees.
        lon: f64,
    },
    /// The coordinate conversion failed.
    #[error(transparent)]
    Projection(#[from] ProjectionError),
}

fn narrow(v: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "heights and distances are far inside the f32 range"
    )]
    let v = v as f32;
    v
}

/// Samples `dem` at every vertex of the project's heightmap grid.
///
/// # Errors
///
/// [`HeightmapError::MissingDem`] for the first vertex (south row first, west to east) that the DEM
/// does not cover; nothing is filled in with zeros.
pub fn build_heightmap(project: &Project, dem: &Dem) -> Result<Heightmap, HeightmapError> {
    let area = &project.area;
    let center = LocalProjection::for_center(area.center_lat, area.center_lon)?;
    let (center_e, center_n) = center.to_utm(area.center_lat, area.center_lon)?;
    let size = project.grid.terrain_size;
    let cell = project.terrain_cell_m();
    let half = project.map_size_m() / 2.0;
    let horizontal = project.scale.horizontal;
    let height_scale = project.scale.height;

    let rows: Vec<Result<Vec<f32>, HeightmapError>> = (0..size)
        .into_par_iter()
        .map(|row| {
            // One projection per row: proj4rs objects are cheap to build and not shared.
            let proj = LocalProjection::for_center(area.center_lat, area.center_lon)?;
            let north = center_n + (f64::from(row) * cell - half) / horizontal;
            (0..size)
                .map(|col| {
                    let east = center_e + (f64::from(col) * cell - half) / horizontal;
                    let (lat, lon) = proj.to_geo(east, north)?;
                    let h = dem
                        .height(lat, lon)
                        .ok_or(HeightmapError::MissingDem { lat, lon })?;
                    Ok(narrow(f64::from(h) * height_scale))
                })
                .collect()
        })
        .collect();

    let mut h = Vec::new();
    for row in rows {
        h.extend(row?);
    }
    Ok(Heightmap {
        size,
        cell_m: narrow(cell),
        h,
    })
}

/// Height and slope of the terrain at one map position, from [`Heightmap::sample`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Height in metres.
    pub height: f32,
    /// Rise per metre towards the east.
    pub dz_dx: f32,
    /// Rise per metre towards the north.
    pub dz_dy: f32,
}

impl Sample {
    /// Steepest slope in degrees.
    #[must_use]
    pub fn slope_deg(&self) -> f32 {
        self.dz_dx.hypot(self.dz_dy).atan().to_degrees()
    }

    /// Hillshade in 0..=1 with the sun at azimuth 315° and altitude 45°.
    #[must_use]
    pub fn hillshade(&self) -> f32 {
        let slope = self.dz_dx.hypot(self.dz_dy).atan();
        // direction the ground faces (down-slope) as a mathematical angle: 0 = east, 90 = north
        let aspect = (-self.dz_dy).atan2(-self.dz_dx);
        let zenith = 45.0_f32.to_radians();
        let azimuth = 135.0_f32.to_radians(); // 315° compass in mathematical angles
        (zenith.cos() * slope.cos() + zenith.sin() * slope.sin() * (azimuth - aspect).cos())
            .clamp(0.0, 1.0)
    }
}

impl Heightmap {
    fn vertex(&self, col: i64, row: i64) -> f32 {
        let last = i64::from(self.size) - 1;
        let c = u32::try_from(col.clamp(0, last)).unwrap_or(0);
        let r = u32::try_from(row.clamp(0, last)).unwrap_or(0);
        self.at(c, r)
    }

    /// Bilinear height and the exact gradient of the bilinear patch at the map position (`x`, `y`) in
    /// metres from the south-west corner. Positions outside the grid hold the nearest edge value.
    #[must_use]
    pub fn sample(&self, x: f64, y: f64) -> Sample {
        let cell = f64::from(self.cell_m).max(f64::MIN_POSITIVE);
        let max = f64::from(self.size.saturating_sub(1));
        let gx = (x / cell).clamp(0.0, max);
        let gy = (y / cell).clamp(0.0, max);
        let (c0, r0) = (gx.floor(), gy.floor());
        let (fx, fy) = (narrow(gx - c0), narrow(gy - r0));
        #[expect(
            clippy::cast_possible_truncation,
            reason = "c0 and r0 are in 0..=8191 after the clamp"
        )]
        let (c0, r0) = (c0 as i64, r0 as i64);
        let h00 = self.vertex(c0, r0);
        let h10 = self.vertex(c0 + 1, r0);
        let h01 = self.vertex(c0, r0 + 1);
        let h11 = self.vertex(c0 + 1, r0 + 1);
        let south = h00 + (h10 - h00) * fx;
        let north = h01 + (h11 - h01) * fx;
        Sample {
            height: south + (north - south) * fy,
            dz_dx: ((h10 - h00) * (1.0 - fy) + (h11 - h01) * fy) / self.cell_m,
            dz_dy: ((h01 - h00) * (1.0 - fx) + (h11 - h10) * fx) / self.cell_m,
        }
    }
}

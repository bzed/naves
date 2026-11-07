// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ground class of every map cell. This module decides; `naves-export` only draws the result.

use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::heightmap::{Heightmap, MapGeo, Sample};
use crate::landcover::{LandCover, LandCoverError, surface_of};
use crate::osm::{LandPolygon, OsmError, OsmLand, read_land};
use crate::project::Project;
use crate::projection::ProjectionError;

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
    /// Every class, in slot order.
    pub const ALL: [SurfaceClass; 4] = [
        SurfaceClass::Grass,
        SurfaceClass::Rock,
        SurfaceClass::Forest,
        SurfaceClass::Soil,
    ];

    /// The colour the class has on the satellite image and in the surface overlay.
    #[must_use]
    pub const fn rgb(self) -> [u8; 3] {
        match self {
            SurfaceClass::Grass => [0x4a, 0x7a, 0x3a],
            SurfaceClass::Rock => [0x8a, 0x87, 0x80],
            SurfaceClass::Forest => [0x2e, 0x4d, 0x2a],
            SurfaceClass::Soil => [0x7a, 0x68, 0x4e],
        }
    }

    /// A short name for legends.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            SurfaceClass::Grass => "Grass",
            SurfaceClass::Rock => "Rock",
            SurfaceClass::Forest => "Forest",
            SurfaceClass::Soil => "Soil",
        }
    }

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

impl SurfaceGrid {
    /// Paints `class` on every cell whose centre lies within half of `width_m` of the polyline `line`
    /// (map metres). A line thinner than a cell paints a trace one cell wide. Parts outside the map are
    /// clipped; a line with fewer than two points, or a number that is not finite, paints nothing.
    pub fn stroke(&mut self, line: &[(f64, f64)], width_m: f64, class: SurfaceClass) {
        if line.len() < 2
            || !width_m.is_finite()
            || line.iter().any(|p| !p.0.is_finite() || !p.1.is_finite())
        {
            return;
        }
        let side = usize::try_from(self.size).unwrap_or(0);
        let reach = (width_m / 2.0).max(self.cell_m / 2.0);
        let last = side.saturating_sub(1);
        let index = |metres: f64| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a cell index from a position clamped to the grid"
            )]
            let cell = (metres / self.cell_m)
                .floor()
                .clamp(0.0, f64::from(MAX_GRID_SIZE)) as usize;
            cell.min(last)
        };
        let centre = |cell: usize| {
            let cell = u32::try_from(cell).map_or(f64::MAX, f64::from);
            (cell + 0.5) * self.cell_m
        };
        for pair in line.windows(2) {
            let [from, to] = pair else { continue };
            let (lo_x, hi_x) = (from.0.min(to.0) - reach, from.0.max(to.0) + reach);
            let (lo_y, hi_y) = (from.1.min(to.1) - reach, from.1.max(to.1) + reach);
            let map_end = self.cell_m * f64::from(self.size);
            if hi_x < 0.0 || hi_y < 0.0 || lo_x > map_end || lo_y > map_end {
                continue;
            }
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            let length2 = dx * dx + dy * dy;
            for row in index(lo_y)..=index(hi_y) {
                for col in index(lo_x)..=index(hi_x) {
                    let (px, py) = (centre(col), centre(row));
                    let along = if length2 > 0.0 {
                        (((px - from.0) * dx + (py - from.1) * dy) / length2).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let distance = (px - from.0 - along * dx).hypot(py - from.1 - along * dy);
                    if distance <= reach
                        && let Some(cell) = self.cells.get_mut(row * side + col)
                    {
                        *cell = class;
                    }
                }
            }
        }
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

/// Cells per side and cell size in metres of the [`SurfaceGrid`] of a heightmap.
fn grid_geometry(hm: &Heightmap) -> (u32, f64) {
    let size = surface_grid_size(hm.size);
    let map_size_m = f64::from(hm.size) * f64::from(hm.cell_m);
    (size, map_size_m / f64::from(size.max(1)))
}

/// The ground from slope and altitude alone.
#[must_use]
pub fn procedural_grid(hm: &Heightmap, grass_limit: f32) -> SurfaceGrid {
    let (size, cell_m) = grid_geometry(hm);
    SurfaceGrid::from_fn(size, cell_m, |col, row| {
        let x = (f64::from(col) + 0.5) * cell_m;
        let y = (f64::from(row) + 0.5) * cell_m;
        procedural_class(&hm.sample(x, y), grass_limit)
    })
}

/// Errors deciding the ground.
#[derive(Debug, thiserror::Error)]
pub enum SurfaceError {
    /// A land cover file cannot be read.
    #[error(transparent)]
    LandCover(#[from] LandCoverError),
    /// An OpenStreetMap file cannot be read.
    #[error(transparent)]
    Osm(#[from] OsmError),
    /// A map position or an OSM point cannot be converted to or from latitude and longitude.
    #[error(transparent)]
    Projection(#[from] ProjectionError),
}

/// The data that decides the ground; each part is optional.
#[derive(Debug, Clone, Copy, Default)]
pub struct SurfaceSources<'a> {
    /// ESA `WorldCover` classes: the complete base.
    pub land_cover: Option<&'a LandCover>,
    /// OpenStreetMap areas: water, rock and built-up land on top of it.
    pub osm: Option<&'a OsmLand>,
}

/// The decided ground and how much of it no source covered.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceBuild {
    /// The class of every map cell.
    pub grid: SurfaceGrid,
    /// Share of the cells (0 to 1) that neither OSM nor land cover decided, and that the slope and altitude
    /// rules decided instead. Only counted when a land cover source was given, else 0.
    pub uncovered_share: f64,
}

/// Cells between two points of the latitude and longitude lattice.
const LATTICE_STEP: u32 = 16;

/// Latitude and longitude of the cell centres, from a coarse lattice of exact conversions that is
/// interpolated: over a few tens of metres the conversion is linear to well below a millimetre.
struct GeoLattice {
    side: usize,
    points: Vec<(f64, f64)>,
}

impl GeoLattice {
    fn new(project: &Project, size: u32, cell_m: f64) -> Result<GeoLattice, SurfaceError> {
        let points_per_side = size / LATTICE_STEP + 2;
        let step_m = f64::from(LATTICE_STEP) * cell_m;
        let rows: Vec<Result<Vec<(f64, f64)>, SurfaceError>> = (0..points_per_side)
            .into_par_iter()
            .map(|lattice_row| {
                // One projection per row: it is cheap to build and not shared.
                let geo = MapGeo::new(project)?;
                (0..points_per_side)
                    .map(|lattice_col| {
                        Ok(geo.to_geo(
                            f64::from(lattice_col) * step_m,
                            f64::from(lattice_row) * step_m,
                        )?)
                    })
                    .collect()
            })
            .collect();
        let mut points = Vec::new();
        for row in rows {
            points.extend(row?);
        }
        Ok(GeoLattice {
            side: usize::try_from(points_per_side).unwrap_or(0),
            points,
        })
    }

    /// Latitude and longitude of the centre of cell (`col`, `row`).
    fn at(&self, col: u32, row: u32) -> Option<(f64, f64)> {
        let locate = |cell: u32| {
            let u = (f64::from(cell) + 0.5) / f64::from(LATTICE_STEP);
            (usize::try_from(cell / LATTICE_STEP).ok(), u - u.floor())
        };
        let ((lattice_col, frac_x), (lattice_row, frac_y)) = (locate(col), locate(row));
        let (lattice_col, lattice_row) = (lattice_col?, lattice_row?);
        let point = |c: usize, r: usize| self.points.get(r * self.side + c).copied();
        let sw = point(lattice_col, lattice_row)?;
        let se = point(lattice_col + 1, lattice_row)?;
        let nw = point(lattice_col, lattice_row + 1)?;
        let ne = point(lattice_col + 1, lattice_row + 1)?;
        let mix =
            |p: (f64, f64), q: (f64, f64), t: f64| (p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t);
        Some(mix(mix(sw, se, frac_x), mix(nw, ne, frac_x), frac_y))
    }
}

/// `v` as an index in `0..=max`; NaN counts as 0.
fn clamp_to(v: f64, max: usize) -> usize {
    if v.is_nan() || v <= 0.0 {
        0
    } else {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "positive, and clamped to max below"
        )]
        let i = v.min(f64::from(u32::MAX)) as usize;
        i.min(max)
    }
}

/// A polygon edge in map metres, from one (x, y) point to the next.
type Edge = ((f64, f64), (f64, f64));

/// The areas of OSM as a class and priority per cell.
struct OsmLayer {
    size: usize,
    priority: Vec<u8>,
    class: Vec<SurfaceClass>,
}

impl OsmLayer {
    /// Fills every polygon once, with an even-odd scanline per cell row; a cell keeps the highest
    /// priority, and among equal priorities the polygon that comes first.
    fn new(
        project: &Project,
        land: &OsmLand,
        size: u32,
        cell_m: f64,
    ) -> Result<OsmLayer, SurfaceError> {
        let cells_per_side = usize::try_from(size).unwrap_or(0);
        let mut layer = OsmLayer {
            size: cells_per_side,
            priority: vec![0; cells_per_side * cells_per_side],
            class: vec![SurfaceClass::Grass; cells_per_side * cells_per_side],
        };
        let geo = MapGeo::new(project)?;
        for polygon in &land.polygons {
            let mut edges: Vec<Edge> = Vec::new();
            for ring in std::iter::once(&polygon.outer).chain(&polygon.holes) {
                let points = ring
                    .iter()
                    .map(|&(lat, lon)| geo.to_map(lat, lon))
                    .collect::<Result<Vec<_>, _>>()?;
                edges.extend(points.windows(2).filter_map(|w| match *w {
                    [a, b] => Some((a, b)),
                    _ => None,
                }));
            }
            layer.fill(&edges, polygon, cell_m);
        }
        Ok(layer)
    }

    fn fill(&mut self, edges: &[Edge], polygon: &LandPolygon, cell_m: f64) {
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        for &((_, ya), (_, yb)) in edges {
            low = low.min(ya.min(yb));
            high = high.max(ya.max(yb));
        }
        if !(low.is_finite() && high.is_finite()) {
            return;
        }
        let last = self.size.saturating_sub(1);
        let first_row = clamp_to((low / cell_m - 0.5).ceil(), self.size);
        let end_row = clamp_to((high / cell_m - 0.5).ceil(), self.size);
        let mut crossings: Vec<f64> = Vec::new();
        for row in first_row..end_row.min(last + 1) {
            #[expect(clippy::cast_precision_loss, reason = "row indices are below 2^13")]
            let y = (row as f64 + 0.5) * cell_m;
            crossings.clear();
            for &((xa, ya), (xb, yb)) in edges {
                if (ya <= y) != (yb <= y) {
                    crossings.push(xa + (y - ya) / (yb - ya) * (xb - xa));
                }
            }
            crossings.sort_by(f64::total_cmp);
            for pair in crossings.chunks_exact(2) {
                let [x0, x1] = *pair else { continue };
                let first_col = clamp_to((x0 / cell_m - 0.5).ceil(), self.size);
                let end_col = clamp_to((x1 / cell_m - 0.5).ceil(), self.size);
                for col in first_col..end_col {
                    let at = row * self.size + col;
                    if let (Some(p), Some(c)) = (self.priority.get_mut(at), self.class.get_mut(at))
                        && polygon.priority > *p
                    {
                        *p = polygon.priority;
                        *c = polygon.class;
                    }
                }
            }
        }
    }

    fn at(&self, col: u32, row: u32) -> Option<SurfaceClass> {
        let at = usize::try_from(row).ok()? * self.size + usize::try_from(col).ok()?;
        (self.priority.get(at).copied()? > 0)
            .then(|| self.class.get(at).copied())
            .flatten()
    }
}

/// Decides the ground of every map cell, first answer wins:
///
/// 1. an OSM area, highest priority first (inside a hole of an area it does not count);
/// 2. the `WorldCover` class at the cell;
/// 3. the slope and altitude rules ([`procedural_class`]): this counts in [`SurfaceBuild::uncovered_share`]
///    when a land cover source was given.
///
/// Slope and altitude do not override real data: grassland on a steep slope stays grass.
///
/// # Errors
///
/// [`SurfaceError::Projection`] if a position cannot be converted.
pub fn build_surfaces(
    project: &Project,
    hm: &Heightmap,
    sources: &SurfaceSources<'_>,
) -> Result<SurfaceBuild, SurfaceError> {
    let (size, cell_m) = grid_geometry(hm);
    let limit = grass_limit_m(project.scale.height);
    let osm = sources
        .osm
        .map(|land| OsmLayer::new(project, land, size, cell_m))
        .transpose()?;
    let lattice = sources
        .land_cover
        .map(|_| GeoLattice::new(project, size, cell_m))
        .transpose()?;

    let rows: Vec<(Vec<SurfaceClass>, u32)> = (0..size)
        .into_par_iter()
        .map(|row| {
            let mut uncovered = 0_u32;
            let classes = (0..size)
                .map(|col| {
                    if let Some(class) = osm.as_ref().and_then(|layer| layer.at(col, row)) {
                        return class;
                    }
                    if let (Some(land_cover), Some(lattice)) = (sources.land_cover, &lattice)
                        && let Some(class) = lattice
                            .at(col, row)
                            .and_then(|(lat, lon)| land_cover.class_code(lat, lon))
                            .and_then(surface_of)
                    {
                        return class;
                    }
                    if sources.land_cover.is_some() {
                        uncovered += 1;
                    }
                    let x = (f64::from(col) + 0.5) * cell_m;
                    let y = (f64::from(row) + 0.5) * cell_m;
                    procedural_class(&hm.sample(x, y), limit)
                })
                .collect();
            (classes, uncovered)
        })
        .collect();

    let mut cells = Vec::with_capacity(rows.iter().map(|(r, _)| r.len()).sum());
    let mut uncovered = 0_u64;
    for (classes, count) in rows {
        cells.extend(classes);
        uncovered += u64::from(count);
    }
    let total = f64::from(size) * f64::from(size);
    #[expect(
        clippy::cast_precision_loss,
        reason = "at most 2^26 cells, exact in f64"
    )]
    let uncovered_share = if total > 0.0 {
        uncovered as f64 / total
    } else {
        0.0
    };
    Ok(SurfaceBuild {
        grid: SurfaceGrid {
            size,
            cell_m,
            cells,
        },
        uncovered_share,
    })
}

/// The land cover and OSM data of a project, read for the window of its map.
#[derive(Debug, Clone, Default)]
pub struct LoadedSources {
    /// The land cover, if the project lists files.
    pub land_cover: Option<LandCover>,
    /// The OSM areas, if the project lists files.
    pub osm: Option<OsmLand>,
}

impl LoadedSources {
    /// Reads the land cover and OSM files of `project` (relative to `project_dir`) for the window of the
    /// whole map plus a margin of 100 m. `progress` gets a short line before each file group.
    ///
    /// # Errors
    ///
    /// [`SurfaceError::LandCover`] or [`SurfaceError::Osm`], naming the first file that cannot be read.
    pub fn open(
        project: &Project,
        project_dir: &Path,
        progress: &dyn Fn(&str),
    ) -> Result<LoadedSources, SurfaceError> {
        let window = MapGeo::new(project)?.window(SOURCE_MARGIN_M)?;
        let resolve = |files: &[PathBuf]| -> Vec<PathBuf> {
            files.iter().map(|p| project_dir.join(p)).collect()
        };
        let land_cover = if project.sources.land_cover.is_empty() {
            None
        } else {
            progress("Reading the land cover");
            Some(LandCover::open(
                &resolve(&project.sources.land_cover),
                window,
            )?)
        };
        let osm = if project.sources.osm.is_empty() {
            None
        } else {
            progress("Reading the OpenStreetMap data");
            Some(read_land(&resolve(&project.sources.osm), window)?)
        };
        Ok(LoadedSources { land_cover, osm })
    }

    /// The sources as [`build_surfaces`] takes them.
    #[must_use]
    pub fn as_sources(&self) -> SurfaceSources<'_> {
        SurfaceSources {
            land_cover: self.land_cover.as_ref(),
            osm: self.osm.as_ref(),
        }
    }
}

/// Margin around the map in metres in which land cover and OSM data are read.
const SOURCE_MARGIN_M: f64 = 100.0;

/// Reads the sources of `project` and decides the ground: what the map views need.
///
/// # Errors
///
/// As [`LoadedSources::open`] and [`build_surfaces`].
pub fn project_surfaces(
    project: &Project,
    project_dir: &Path,
    hm: &Heightmap,
) -> Result<SurfaceGrid, SurfaceError> {
    let sources = LoadedSources::open(project, project_dir, &|_| {})?;
    Ok(build_surfaces(project, hm, &sources.as_sources())?.grid)
}

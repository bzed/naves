// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Land cover input: ESA `WorldCover` GeoTIFF files (one band of class codes, EPSG:4326, 10 m pixels).
//!
//! A tile has 36 000 × 36 000 pixels, so files are never read whole: [`LandCover::open`] decodes only
//! the pixels inside the [`GeoWindow`] of the map. Row 0 of a file is its northern row.

use std::path::PathBuf;

use geotiff_reader::GeoTiffFile;

use crate::surface::SurfaceClass;

/// Largest accepted window side in pixels.
const MAX_WINDOW_SIDE: u32 = 16_000;

/// A rectangle in degrees (WGS84).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoWindow {
    /// Southern edge, latitude.
    pub south: f64,
    /// Western edge, longitude.
    pub west: f64,
    /// Northern edge, latitude.
    pub north: f64,
    /// Eastern edge, longitude.
    pub east: f64,
}

/// Errors reading land cover files.
#[derive(Debug, thiserror::Error)]
pub enum LandCoverError {
    /// The file is not a GeoTIFF this reader supports.
    #[error("unsupported land cover file: {0}")]
    Unsupported(String),
    /// The GeoTIFF decoder rejected the file.
    #[error("cannot decode GeoTIFF: {0}")]
    GeoTiff(geotiff_reader::Error),
    /// An error in one of the files given to [`LandCover::open`].
    #[error("{}: {source}", path.display())]
    File {
        /// The file concerned.
        path: PathBuf,
        /// What went wrong.
        source: Box<LandCoverError>,
    },
}

fn unsupported<T>(msg: impl Into<String>) -> Result<T, LandCoverError> {
    Err(LandCoverError::Unsupported(msg.into()))
}

fn geotiff_error(e: geotiff_reader::Error) -> LandCoverError {
    match e {
        geotiff_reader::Error::NotGeoTiff => LandCoverError::Unsupported(e.to_string()),
        other => LandCoverError::GeoTiff(other),
    }
}

/// The decoded window of one file.
#[derive(Debug, Clone)]
struct Tile {
    west: f64,
    north: f64,
    pixel_x: f64,
    pixel_y: f64,
    col0: u32,
    row0: u32,
    cols: u32,
    rows: u32,
    data: Vec<u8>,
}

impl Tile {
    fn code(&self, lat: f64, lon: f64) -> Option<u8> {
        let col = index(((lon - self.west) / self.pixel_x).floor())?;
        let row = index(((self.north - lat) / self.pixel_y).floor())?;
        let col = col.checked_sub(self.col0).filter(|&c| c < self.cols)?;
        let row = row.checked_sub(self.row0).filter(|&r| r < self.rows)?;
        let at = usize::try_from(row)
            .ok()?
            .checked_mul(usize::try_from(self.cols).ok()?)?
            .checked_add(usize::try_from(col).ok()?)?;
        self.data.get(at).copied()
    }
}

/// `v` as a pixel index if it is a finite value in `0..=u32::MAX`.
fn index(v: f64) -> Option<u32> {
    if v.is_finite() && v >= 0.0 && v <= f64::from(u32::MAX) {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "range-checked above"
        )]
        Some(v as u32)
    } else {
        None
    }
}

/// `v` clamped to `0..=max` (NaN counts as 0).
fn clamp_index(v: f64, max: u32) -> u32 {
    if v.is_nan() || v <= 0.0 {
        0
    } else {
        index(v).map_or(max, |i| i.min(max))
    }
}

/// The land cover inside a window, from any number of files.
#[derive(Debug, Clone, Default)]
pub struct LandCover {
    tiles: Vec<Tile>,
}

impl LandCover {
    /// Reads the pixels inside `window` from every file; a file that does not touch the window is skipped.
    ///
    /// # Errors
    ///
    /// [`LandCoverError::File`], naming the first file that is not a single-band GeoTIFF in EPSG:4326, cannot
    /// be decoded, or needs a window above 16 000 pixels per side.
    pub fn open(paths: &[PathBuf], window: GeoWindow) -> Result<LandCover, LandCoverError> {
        let mut tiles = Vec::new();
        for path in paths {
            let tile = GeoTiffFile::open(path)
                .map_err(geotiff_error)
                .and_then(|file| read_window(&file, window))
                .map_err(|source| LandCoverError::File {
                    path: path.clone(),
                    source: Box::new(source),
                })?;
            tiles.extend(tile);
        }
        Ok(LandCover { tiles })
    }

    /// Like [`LandCover::open`] for a file that is already in memory (the entry point of the fuzz target).
    ///
    /// # Errors
    ///
    /// As [`LandCover::open`], without the file name.
    pub fn from_bytes(bytes: Vec<u8>, window: GeoWindow) -> Result<LandCover, LandCoverError> {
        let file = GeoTiffFile::from_bytes(bytes).map_err(geotiff_error)?;
        Ok(LandCover {
            tiles: read_window(&file, window)?.into_iter().collect(),
        })
    }

    /// The `WorldCover` code of the pixel that contains (`lat`, `lon`): `None` outside the window and the
    /// files, for code 0 (no data) and for non-finite input.
    #[must_use]
    pub fn class_code(&self, lat: f64, lon: f64) -> Option<u8> {
        self.tiles
            .iter()
            .find_map(|t| t.code(lat, lon))
            .filter(|&code| code != 0)
    }

    /// Pixels held in memory.
    #[must_use]
    pub fn pixel_count(&self) -> usize {
        self.tiles.iter().map(|t| t.data.len()).sum()
    }
}

fn read_window(file: &GeoTiffFile, window: GeoWindow) -> Result<Option<Tile>, LandCoverError> {
    let (width, height) = (file.width(), file.height());
    if file.band_count() != 1 {
        return unsupported("not a single-band image");
    }
    if file.epsg() != Some(4326) {
        return unsupported("only geographic WGS84 (EPSG:4326) GeoTIFFs are supported");
    }
    let Some(transform) = file.transform() else {
        return unsupported("no geo-referencing");
    };
    let (west, north) = transform.pixel_to_geo(0.0, 0.0);
    let (next_lon, next_lon_lat) = transform.pixel_to_geo(1.0, 0.0);
    let (next_row_lon, next_row_lat) = transform.pixel_to_geo(0.0, 1.0);
    let (pixel_x, pixel_y) = (next_lon - west, north - next_row_lat);
    let rotated = (next_lon_lat - north).abs() > 1e-12 || (next_row_lon - west).abs() > 1e-12;
    let valid = |v: f64| v.is_finite() && v > 0.0;
    if rotated || !(valid(pixel_x) && valid(pixel_y) && west.is_finite() && north.is_finite()) {
        return unsupported("rotated or invalid geo-transform");
    }
    if ![window.south, window.west, window.north, window.east]
        .iter()
        .all(|v| v.is_finite())
    {
        return unsupported("the map window is not finite");
    }
    let first_col = clamp_index(((window.west - west) / pixel_x).floor(), width);
    let end_col = clamp_index(((window.east - west) / pixel_x).ceil(), width);
    let first_row = clamp_index(((north - window.north) / pixel_y).floor(), height);
    let end_row = clamp_index(((north - window.south) / pixel_y).ceil(), height);
    if end_col <= first_col || end_row <= first_row {
        return Ok(None);
    }
    let (win_w, win_h) = (end_col - first_col, end_row - first_row);
    if win_w > MAX_WINDOW_SIDE || win_h > MAX_WINDOW_SIDE {
        return unsupported(format!(
            "the map needs a window of {win_w} x {win_h} pixels, more than the limit of {MAX_WINDOW_SIDE} x {MAX_WINDOW_SIDE}"
        ));
    }
    let to_usize = |v: u32| usize::try_from(v).or_else(|_| unsupported("window too large"));
    let raster = file
        .read_band_window::<u8>(
            0,
            to_usize(first_row)?,
            to_usize(first_col)?,
            to_usize(win_h)?,
            to_usize(win_w)?,
        )
        .map_err(geotiff_error)?;
    let data: Vec<u8> = raster.iter().copied().collect();
    if data.len() != to_usize(win_w)?.saturating_mul(to_usize(win_h)?) {
        return unsupported("decoded size does not match the window");
    }
    Ok(Some(Tile {
        west,
        north,
        pixel_x,
        pixel_y,
        col0: first_col,
        row0: first_row,
        cols: win_w,
        rows: win_h,
        data,
    }))
}

/// The ground class for a `WorldCover` code, `None` for codes that say nothing useful.
///
/// Tree cover and mangroves are forest; shrubland, grassland and wetland are grass; cropland, built-up
/// and water are soil; bare ground, snow and moss are rock (no snow texture exists among the vanilla
/// surfaces).
#[must_use]
pub fn surface_of(code: u8) -> Option<SurfaceClass> {
    match code {
        10 | 95 => Some(SurfaceClass::Forest),
        20 | 30 | 90 => Some(SurfaceClass::Grass),
        40 | 50 | 80 => Some(SurfaceClass::Soil),
        60 | 70 | 100 => Some(SurfaceClass::Rock),
        _ => None,
    }
}

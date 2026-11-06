// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Digital elevation model input: single-band Float32 GeoTIFF in EPSG:4326 (Copernicus GLO-30).
//!
//! Geo-referencing comes from `ModelTiepointTag` and `ModelPixelScaleTag`; the pixel at column `c`,
//! row `r` has its centre at `origin + (c + 0.5, r + 0.5) * scale` (pixel-is-area; pixel-is-point
//! files are shifted by half a pixel). Row 0 is the northern row.

use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::{Path, PathBuf};

use geotiff_reader::GeoTiffFile;

/// Largest accepted image side in pixels (GLO-30 tiles are 3600).
const MAX_SIDE: u32 = 16_384;
/// Largest accepted pixel count (256 MiB of Float32).
const MAX_PIXELS: u64 = 1 << 26;

/// Errors reading DEM files.
#[derive(Debug, thiserror::Error)]
pub enum DemError {
    /// The file is not a GeoTIFF this reader supports.
    #[error("unsupported DEM file: {0}")]
    Unsupported(String),
    /// The GeoTIFF decoder rejected the file.
    #[error("cannot decode GeoTIFF: {0}")]
    GeoTiff(geotiff_reader::Error),
    /// Reading the file failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// An error in one of the files given to [`Dem::open`].
    #[error("{}: {source}", path.display())]
    File {
        /// The file concerned.
        path: PathBuf,
        /// What went wrong.
        source: Box<DemError>,
    },
}

fn unsupported<T>(msg: impl Into<String>) -> Result<T, DemError> {
    Err(DemError::Unsupported(msg.into()))
}

/// A file without GeoTIFF metadata is unsupported input; anything else is a decoding error.
fn geotiff_error(e: geotiff_reader::Error) -> DemError {
    match e {
        geotiff_reader::Error::NotGeoTiff => DemError::Unsupported(e.to_string()),
        other => DemError::GeoTiff(other),
    }
}

/// Largest index magnitude that is converted from floating point.
const MAX_INDEX: f64 = 1.0e9;

/// Floors `v` to an `i64`, or `None` if it is not finite or absurdly large.
fn floor_index(v: f64) -> Option<i64> {
    if v.is_finite() && v.abs() < MAX_INDEX {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "range-checked above: |v| < 1e9 fits an i64 exactly after floor()"
        )]
        Some(v.floor() as i64)
    } else {
        None
    }
}

/// One GeoTIFF: a grid of heights in degrees-of-latitude/longitude space.
#[derive(Debug, Clone)]
pub struct DemTile {
    west: f64,
    north: f64,
    pixel_x: f64,
    pixel_y: f64,
    width: u32,
    height: u32,
    data: Vec<f32>,
}

/// Largest accepted file size.
const MAX_FILE_BYTES: u64 = 1 << 29;

impl DemTile {
    /// Decodes a GeoTIFF with the `geotiff-reader` crate; the input is untrusted.
    ///
    /// # Errors
    ///
    /// [`DemError::Unsupported`] for anything but a single-band Float32 GeoTIFF in EPSG:4326 within
    /// the size limits, [`DemError::GeoTiff`] / [`DemError::Io`] for corrupt or truncated files.
    pub fn from_reader<R: Read + Seek>(mut reader: R) -> Result<DemTile, DemError> {
        let mut bytes = Vec::new();
        reader
            .by_ref()
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if u64::try_from(bytes.len()).map_or(true, |n| n > MAX_FILE_BYTES) {
            return unsupported("file larger than 512 MiB");
        }
        let file = GeoTiffFile::from_bytes(bytes).map_err(geotiff_error)?;
        let (width, height) = (file.width(), file.height());
        if width == 0
            || height == 0
            || width > MAX_SIDE
            || height > MAX_SIDE
            || u64::from(width) * u64::from(height) > MAX_PIXELS
        {
            return unsupported(format!(
                "image size {width}x{height} outside the supported range"
            ));
        }
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
        let raster = file.read_band::<f32>(0).map_err(geotiff_error)?;
        // A sentinel such as -9999 is not a height: from here on, "no data" is NaN.
        let nodata = file.nodata().and_then(|s| s.trim().parse::<f64>().ok());
        let data: Vec<f32> = raster
            .iter()
            .map(|&v| {
                let void = nodata.is_some_and(|n| (f64::from(v) - n).abs() <= n.abs() * 1e-6);
                if void { f32::NAN } else { v }
            })
            .collect();
        let expected = usize::try_from(u64::from(width) * u64::from(height))
            .or_else(|_| unsupported("image too large"))?;
        if data.len() != expected {
            return unsupported("decoded size does not match the image size");
        }
        Ok(DemTile {
            west,
            north,
            pixel_x,
            pixel_y,
            width,
            height,
            data,
        })
    }

    fn contains(&self, lat: f64, lon: f64) -> bool {
        let east = self.west + f64::from(self.width) * self.pixel_x;
        let south = self.north - f64::from(self.height) * self.pixel_y;
        lon >= self.west && lon < east && lat <= self.north && lat > south
    }

    /// Raw sample at pixel (`ix`, `iy`), `None` outside the image.
    fn pixel(&self, ix: i64, iy: i64) -> Option<f32> {
        let x = usize::try_from(ix)
            .ok()
            .filter(|&x| x < self.width as usize)?;
        let y = usize::try_from(iy)
            .ok()
            .filter(|&y| y < self.height as usize)?;
        self.data.get(y * self.width as usize + x).copied()
    }

    /// Sample of the pixel that contains (`lat`, `lon`).
    fn nearest(&self, lat: f64, lon: f64) -> Option<f32> {
        let ix = floor_index((lon - self.west) / self.pixel_x)?;
        let iy = floor_index((self.north - lat) / self.pixel_y)?;
        self.pixel(ix, iy)
    }

    /// Like [`pixel`](Self::pixel), but holds the nearest edge value outside the image.
    fn clamped(&self, ix: i64, iy: i64) -> Option<f32> {
        let max_x = i64::from(self.width) - 1;
        let max_y = i64::from(self.height) - 1;
        self.pixel(ix.clamp(0, max_x), iy.clamp(0, max_y))
    }
}

/// A set of DEM tiles.
#[derive(Debug, Clone)]
pub struct Dem {
    tiles: Vec<DemTile>,
}

impl Dem {
    /// Reads every file with [`DemTile::from_reader`].
    ///
    /// # Errors
    ///
    /// [`DemError::File`], naming the first file that cannot be read.
    pub fn open(paths: &[PathBuf]) -> Result<Dem, DemError> {
        let mut tiles = Vec::with_capacity(paths.len());
        for path in paths {
            tiles.push(open_tile(path).map_err(|source| DemError::File {
                path: path.clone(),
                source: Box::new(source),
            })?);
        }
        Ok(Dem { tiles })
    }

    /// A synthetic DEM for tests: `f(lat, lon)` sampled at the pixel centres of a grid with
    /// `pixel_deg` spacing that covers `west..east` and `south..north` (degrees).
    #[cfg(any(test, feature = "test-util"))]
    #[must_use]
    pub fn from_fn(
        west: f64,
        south: f64,
        east: f64,
        north: f64,
        pixel_deg: f64,
        f: impl Fn(f64, f64) -> f32,
    ) -> Dem {
        let side = |span: f64| {
            floor_index((span / pixel_deg).round())
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0)
        };
        let (width, height) = (side(east - west), side(north - south));
        let mut data = Vec::new();
        for row in 0..height {
            let lat = north - (f64::from(row) + 0.5) * pixel_deg;
            for col in 0..width {
                data.push(f(lat, west + (f64::from(col) + 0.5) * pixel_deg));
            }
        }
        Dem {
            tiles: vec![DemTile {
                west,
                north,
                pixel_x: pixel_deg,
                pixel_y: pixel_deg,
                width,
                height,
                data,
            }],
        }
    }

    /// Bilinear height at a WGS84 position; `None` if no tile covers it or a neighbouring sample
    /// is missing (NaN).
    ///
    /// Neighbouring samples across a tile border come from the adjacent tile.
    #[must_use]
    pub fn height(&self, lat: f64, lon: f64) -> Option<f32> {
        if !(lat.is_finite() && lon.is_finite()) {
            return None;
        }
        let tile = self.tiles.iter().find(|t| t.contains(lat, lon))?;
        let fx = (lon - tile.west) / tile.pixel_x - 0.5;
        let fy = (tile.north - lat) / tile.pixel_y - 0.5;
        let x0 = floor_index(fx)?;
        let y0 = floor_index(fy)?;
        #[expect(
            clippy::cast_precision_loss,
            reason = "indices are tiny compared with 2^52"
        )]
        let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
        let v00 = self.sample(tile, x0, y0)?;
        let v10 = self.sample(tile, x0 + 1, y0)?;
        let v01 = self.sample(tile, x0, y0 + 1)?;
        let v11 = self.sample(tile, x0 + 1, y0 + 1)?;
        let [v00, v10, v01, v11] = [v00, v10, v01, v11].map(f64::from);
        let top = v00 + (v10 - v00) * tx;
        let bottom = v01 + (v11 - v01) * tx;
        let h = top + (bottom - top) * ty;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "an interpolation of four f32 values is representable as f32 (up to rounding)"
        )]
        let h = h as f32;
        h.is_finite().then_some(h)
    }

    /// Sample at pixel (`ix`, `iy`) of `tile`; outside it, from the adjacent tile or the held edge.
    fn sample(&self, tile: &DemTile, ix: i64, iy: i64) -> Option<f32> {
        let v = tile
            .pixel(ix, iy)
            .or_else(|| self.outside_sample(tile, ix, iy))?;
        v.is_finite().then_some(v)
    }

    /// Sample for a pixel index outside `tile`: from the tile that covers its centre, else the
    /// nearest edge value of `tile`.
    fn outside_sample(&self, tile: &DemTile, ix: i64, iy: i64) -> Option<f32> {
        #[expect(
            clippy::cast_precision_loss,
            reason = "indices are tiny compared with 2^52"
        )]
        let (cx, cy) = (ix as f64 + 0.5, iy as f64 + 0.5);
        let lon = tile.west + cx * tile.pixel_x;
        let lat = tile.north - cy * tile.pixel_y;
        self.tiles
            .iter()
            .find(|t| !std::ptr::eq(*t, tile) && t.contains(lat, lon))
            .map_or_else(|| tile.clamped(ix, iy), |other| other.nearest(lat, lon))
    }
}

fn open_tile(path: &Path) -> Result<DemTile, DemError> {
    DemTile::from_reader(BufReader::new(File::open(path)?))
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fetching a height model for a project from Mapterhorn's tile server.
//!
//! `naves dem fetch` reads Web Mercator tiles (Terrarium encoding, 512 pixels per tile) of the map's window plus a
//! real-world margin, resamples them to a GeoTIFF in the project's `dem` folder and puts that file first in
//! `sources.dem`. The tile maths here is pure: the network and the file writing come in later. Nothing in
//! `naves build` calls this module.

use std::collections::BTreeMap;

use naves_core::heightmap::MapGeo;
use naves_core::landcover::GeoWindow;
use naves_core::project::Project;

use crate::imagery_fetch::FetchError;

/// Tile pixels per side.
pub const TILE_PX: u32 = 512;
/// Real metres around the map that are fetched as well, as the reader needs them.
pub const MARGIN_M: f64 = 1000.0;
/// Coarsest zoom that is fetched when a tile is missing at the finer ones: the floor of the fallback.
pub const Z_MIN: u8 = 12;
/// Finest zoom that is probed for a tile.
pub const Z_MAX: u8 = 18;

/// Latitude limit of Web Mercator, in degrees.
const MERCATOR_LAT: f64 = 85.0511;
/// Largest zoom whose tile indices fit a `u32` with room to spare.
const TILE_ZOOM_LIMIT: u8 = 31;

/// The window the height model must cover: the map plus [`MARGIN_M`] real metres on every side.
///
/// # Errors
///
/// [`FetchError::Projection`] if a corner of the map cannot be projected.
pub fn fetch_window(project: &Project) -> Result<GeoWindow, FetchError> {
    Ok(MapGeo::new(project)?.window(MARGIN_M * project.scale.horizontal)?)
}

/// Fractional world-pixel coordinates of a position at `z` (Web Mercator, [`TILE_PX`] pixels per tile):
/// `x = (lon + 180)/360 · 2^z · TILE_PX`, `y = (1 − asinh(tan φ)/π)/2 · 2^z · TILE_PX`. Latitudes are clamped to
/// the Web Mercator limit of ±85.0511° first.
#[must_use]
pub fn world_px(lat: f64, lon: f64, z: u8) -> (f64, f64) {
    let size = f64::powi(2.0, i32::from(z)) * f64::from(TILE_PX);
    let lat = lat.clamp(-MERCATOR_LAT, MERCATOR_LAT).to_radians();
    let x = (lon + 180.0) / 360.0 * size;
    let y = (1.0 - lat.tan().asinh() / std::f64::consts::PI) / 2.0 * size;
    (x, y)
}

/// Tiles covering `window` at `z`, grown by one tile on each side (the bilinear halo) and clamped to
/// `0..2^z`: `(x0, x1, y0, y1)`, inclusive. `west` must not be east of `east`. Zooms above 31 are treated as 31.
#[must_use]
pub fn tile_range(window: &GeoWindow, z: u8) -> (u32, u32, u32, u32) {
    let z = z.min(TILE_ZOOM_LIMIT);
    let tiles = 1u32 << z;
    let tile = f64::from(TILE_PX);
    let index = |px: f64| tile_index(px / tile, tiles);

    let (xw, yn) = world_px(window.north, window.west, z);
    let (xe, ys) = world_px(window.south, window.east, z);
    let x0 = index(xw).saturating_sub(1);
    let x1 = (index(xe) + 1).min(tiles - 1);
    let y0 = index(yn).saturating_sub(1);
    let y1 = (index(ys) + 1).min(tiles - 1);
    (x0, x1, y0, y1)
}

/// The tile index containing `tile_coord` (tile units), clamped to `0..tiles`.
fn tile_index(tile_coord: f64, tiles: u32) -> u32 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..tiles, which fits a u32, before the cast"
    )]
    let index = tile_coord.clamp(0.0, f64::from(tiles - 1)).floor() as u32;
    index
}

/// Decodes one Terrarium tile (webp bytes) to `TILE_PX * TILE_PX` heights in metres, row 0 north:
/// `h = R * 256 + G + B / 256 - 32768`. Only 512 x 512 pixel images of 8-bit RGB or RGBA are accepted.
///
/// # Errors
///
/// [`FetchError::Other`] if the bytes are not a webp image, the image is not [`TILE_PX`] pixels square or it
/// is not 8-bit RGB(A).
pub fn terrarium(bytes: &[u8]) -> Result<Vec<f32>, FetchError> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes));
    reader.set_format(image::ImageFormat::WebP);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(TILE_PX);
    limits.max_image_height = Some(TILE_PX);
    limits.max_alloc = Some(8 << 20);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|e| FetchError::Other(format!("not a Terrarium tile: {e}")))?;
    if decoded.width() != TILE_PX || decoded.height() != TILE_PX {
        return Err(FetchError::Other(format!(
            "a tile of {} x {} pixels, not {TILE_PX} x {TILE_PX}",
            decoded.width(),
            decoded.height()
        )));
    }
    let (data, channels) = match decoded {
        image::DynamicImage::ImageRgb8(img) => (img.into_raw(), 3),
        image::DynamicImage::ImageRgba8(img) => (img.into_raw(), 4),
        other => {
            return Err(FetchError::Other(format!(
                "a tile of colour type {:?}, not 8-bit RGB",
                other.color()
            )));
        }
    };
    Ok(data
        .chunks_exact(channels)
        .map(|px| match px {
            [r, g, b, ..] => {
                let h = f64::from(*r) * 256.0 + f64::from(*g) + f64::from(*b) / 256.0 - 32768.0;
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "heights of a few km fit f32 exactly enough"
                )]
                let h = h as f32;
                h
            }
            _ => 0.0,
        })
        .collect())
}

/// Decoded tiles by `(z, x, y)`, sampled with a per-zoom fallback.
#[derive(Debug, Default)]
pub struct TilePyramid {
    tiles: BTreeMap<(u8, u32, u32), Vec<f32>>,
}

impl TilePyramid {
    /// Decodes `bytes` with [`terrarium`] and stores the tile at `(z, x, y)`, replacing an earlier one.
    ///
    /// # Errors
    ///
    /// Whatever [`terrarium`] returns.
    pub fn insert(&mut self, z: u8, x: u32, y: u32, bytes: &[u8]) -> Result<(), FetchError> {
        self.tiles.insert((z, x, y), terrarium(bytes)?);
        Ok(())
    }

    /// The height of the world pixel `(px, py)` at `z`, if its tile is held.
    fn pixel(&self, z: u8, px: i64, py: i64) -> Option<f64> {
        let tile = i64::from(TILE_PX);
        let side = 1i64.checked_shl(u32::from(z))?;
        let (tx, ty) = (px.div_euclid(tile), py.div_euclid(tile));
        if !(0..side).contains(&tx) || !(0..side).contains(&ty) {
            return None;
        }
        let col = usize::try_from(px.rem_euclid(tile)).ok()?;
        let row = usize::try_from(py.rem_euclid(tile)).ok()?;
        let heights = self
            .tiles
            .get(&(z, u32::try_from(tx).ok()?, u32::try_from(ty).ok()?))?;
        heights
            .get(row * usize::try_from(TILE_PX).ok()? + col)
            .map(|h| f64::from(*h))
    }

    /// Bilinear height at a position, taken from the finest zoom from `zoom_max` down to [`Z_MIN`] at which all
    /// four pixels around it are held (pixel centres, as `Dem::height`); `None` if there is none.
    #[must_use]
    pub fn sample(&self, lat: f64, lon: f64, zoom_max: u8) -> Option<f64> {
        if !lat.is_finite() || !lon.is_finite() {
            return None;
        }
        (Z_MIN..=zoom_max.min(TILE_ZOOM_LIMIT))
            .rev()
            .find_map(|z| self.sample_at(lat, lon, z))
    }

    /// The bilinear value at zoom `z` alone.
    fn sample_at(&self, lat: f64, lon: f64, z: u8) -> Option<f64> {
        let (wx, wy) = world_px(lat, lon, z);
        let (fx, fy) = (wx - 0.5, wy - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        if !(x0.abs() < 1e12 && y0.abs() < 1e12) {
            return None;
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "finite and below 1e12 in magnitude"
        )]
        let (ix, iy) = (x0 as i64, y0 as i64);
        let (tx, ty) = (fx - x0, fy - y0);
        let top = self.pixel(z, ix, iy)? * (1.0 - tx) + self.pixel(z, ix + 1, iy)? * tx;
        let bottom = self.pixel(z, ix, iy + 1)? * (1.0 - tx) + self.pixel(z, ix + 1, iy + 1)? * tx;
        Some(top * (1.0 - ty) + bottom * ty)
    }
}

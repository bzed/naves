// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fetching a height model for a project from Mapterhorn's tile server.
//!
//! `naves dem fetch` reads Web Mercator tiles (Terrarium encoding, 512 pixels per tile) of the map's window plus a
//! real-world margin, resamples them to a GeoTIFF in the project's `dem` folder and puts that file first in
//! `sources.dem`. The tile maths here is pure: the network and the file writing come in later. Nothing in
//! `naves build` calls this module.

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

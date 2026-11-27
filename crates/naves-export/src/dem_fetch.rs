// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fetching a height model for a project from Mapterhorn's tile server.
//!
//! `naves dem fetch` reads Web Mercator tiles (Terrarium encoding, 512 pixels per tile) of the map's window plus a
//! real-world margin, resamples them to a GeoTIFF in the project's `dem` folder and puts that file first in
//! `sources.dem`. The tile maths here is pure: the network and the file writing come in later. Nothing in
//! `naves build` calls this module.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rayon::prelude::*;

use naves_core::heightmap::MapGeo;
use naves_core::landcover::GeoWindow;
use naves_core::project::Project;

use crate::geotiff_write::{DemMosaic, write_atomic, write_dem};
use crate::imagery_fetch::{FetchError, Limited, http_error, today, too_large, with_tries};

/// Tile pixels per side.
pub const TILE_PX: u32 = 512;
/// Real metres around the map that are fetched as well, as the reader needs them.
pub const MARGIN_M: f64 = 1000.0;
/// Coarsest zoom that is fetched when a tile is missing at the finer ones: the floor of the fallback.
pub const Z_MIN: u8 = 12;
/// Finest zoom that is probed for a tile.
pub const Z_MAX: u8 = 18;

/// Target pixel of the fetched raster, in real metres.
pub const DEFAULT_PIXEL_M: f64 = 2.5;
/// Most pixels the raster may have (`dem.rs`'s limit).
pub const MAX_PIXELS: u64 = 1 << 26;
/// Most pixels along either side of the raster (`dem.rs`'s limit).
pub const MAX_SIDE_PX: u32 = 16_384;

/// Parallel tile requests (be polite to the server).
const TILE_JOBS: usize = 4;
/// Largest answer accepted for one tile, in bytes.
pub const MAX_TILE_BYTES: u64 = 8 << 20;
/// Bytes per tile that the size note of [`describe_dem_fetch`] assumes.
pub const TILE_BYTES_ESTIMATE: u64 = 160_000;

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

/// Raw tile bytes by `(z, x, y)`, as resampling reads them: the project's tile cache in production.
pub trait TileStore: Sync {
    /// The bytes of tile `(z, x, y)`; `Ok(None)` if the tile does not exist at that zoom.
    ///
    /// # Errors
    ///
    /// [`FetchError`] if the tile cannot be read.
    fn load(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, FetchError>;
}

/// Real metres per degree of latitude and (at the equator) of longitude, as the pixel size is derived.
const METRES_PER_DEGREE_LAT: f64 = 111_132.0;
const METRES_PER_DEGREE_LON: f64 = 111_320.0;
/// Pixel counts are capped here before the conversion to integers.
const COUNT_CAP: f64 = 1e9;

/// Pixel size in degrees `(x, y)` and the counts `(columns, rows)` of `window` at `pixel_m` real metres.
fn grid_counts(window: &GeoWindow, pixel_m: f64) -> Result<(f64, f64, u64, u64), FetchError> {
    if !(pixel_m.is_finite() && pixel_m > 0.0) {
        return Err(FetchError::Other(format!(
            "the pixel size must be a positive number of metres, not {pixel_m}"
        )));
    }
    let GeoWindow {
        south,
        west,
        north,
        east,
    } = *window;
    if !(south.is_finite()
        && west.is_finite()
        && north.is_finite()
        && east.is_finite()
        && west < east
        && south < north)
    {
        return Err(FetchError::Other(format!(
            "the window {south}..{north} N, {west}..{east} E is not a box"
        )));
    }
    let mid = f64::midpoint(south, north).to_radians();
    let px = pixel_m / (METRES_PER_DEGREE_LON * mid.cos().abs().max(1e-6));
    let py = pixel_m / METRES_PER_DEGREE_LAT;
    let count = |span: f64, step: f64| -> u64 {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "positive, and capped below"
        )]
        let n = (span / step).ceil().clamp(1.0, COUNT_CAP) as u64;
        n
    };
    Ok((px, py, count(east - west, px), count(north - south, py)))
}

/// Whether a grid of `cols` x `rows` fits the limits, as `u32`s.
fn fit_grid(cols: u64, rows: u64) -> Result<(u32, u32), FetchError> {
    let too_big = || {
        FetchError::Other(format!(
            "a raster of {cols} x {rows} pixels is larger than the limits of {MAX_SIDE_PX} pixels per side \
             and {MAX_PIXELS} pixels in all; use a coarser pixel or a smaller map"
        ))
    };
    let side = u64::from(MAX_SIDE_PX);
    if cols > side || rows > side || cols.checked_mul(rows).is_none_or(|p| p > MAX_PIXELS) {
        return Err(too_big());
    }
    Ok((
        u32::try_from(cols).map_err(|_| too_big())?,
        u32::try_from(rows).map_err(|_| too_big())?,
    ))
}

/// Columns and rows of the grid of `window` at `pixel_m` real metres, growing `pixel_m` until it fits
/// [`MAX_SIDE_PX`] and [`MAX_PIXELS`]: `(columns, rows, pixel_m)`. `requested` (default [`DEFAULT_PIXEL_M`]) is a
/// floor: the pixel only grows.
///
/// # Errors
///
/// [`FetchError::Other`] if `requested` is not a positive finite number, the window is not a box or the grid
/// does not fit after growing.
pub fn dem_grid(window: &GeoWindow, requested: Option<f64>) -> Result<(u32, u32, f64), FetchError> {
    let mut pixel_m = requested.unwrap_or(DEFAULT_PIXEL_M);
    for _ in 0..4 {
        let (_, _, cols, rows) = grid_counts(window, pixel_m)?;
        #[expect(
            clippy::cast_precision_loss,
            reason = "pixel counts are capped at 1e9 and MAX_PIXELS is a power of two"
        )]
        let (longest, area, max_pixels) = (
            cols.max(rows) as f64,
            cols as f64 * rows as f64,
            MAX_PIXELS as f64,
        );
        let factor = (longest / f64::from(MAX_SIDE_PX)).max((area / max_pixels).sqrt());
        if factor <= 1.0 {
            break;
        }
        // a little extra so that the next count fits in spite of rounding up
        pixel_m *= factor * 1.001;
    }
    let (_, _, cols, rows) = grid_counts(window, pixel_m)?;
    let (cols, rows) = fit_grid(cols, rows)?;
    Ok((cols, rows, pixel_m))
}

/// Loads the tiles one band needs into a fresh pyramid: row `ty - 1 ..= ty + 1` at `zoom_max` (clamped to
/// `rows`) for the columns `cols`, and the ancestors of every position missing there.
fn load_band(
    store: &dyn TileStore,
    zoom_max: u8,
    ty: u32,
    cols: (u32, u32),
    rows: (u32, u32),
) -> Result<TilePyramid, FetchError> {
    let mut pyramid = TilePyramid::default();
    let mut ancestors = BTreeSet::new();
    for y in ty.saturating_sub(1).max(rows.0)..=ty.saturating_add(1).min(rows.1) {
        for x in cols.0..=cols.1 {
            if let Some(bytes) = store.load(zoom_max, x, y)? {
                insert_tile(&mut pyramid, zoom_max, x, y, &bytes)?;
            } else {
                for z in (Z_MIN..zoom_max).rev() {
                    let shift = u32::from(zoom_max - z);
                    ancestors.insert((z, x >> shift, y >> shift));
                }
            }
        }
    }
    for (z, x, y) in ancestors {
        if let Some(bytes) = store.load(z, x, y)? {
            insert_tile(&mut pyramid, z, x, y, &bytes)?;
        }
    }
    Ok(pyramid)
}

/// [`TilePyramid::insert`], with the tile named in the error.
fn insert_tile(
    pyramid: &mut TilePyramid,
    z: u8,
    x: u32,
    y: u32,
    bytes: &[u8],
) -> Result<(), FetchError> {
    pyramid
        .insert(z, x, y, bytes)
        .map_err(|e| FetchError::Other(format!("tile {z}/{x}/{y}: {e}")))
}

/// Samples the tiles at the centre of every pixel of the grid of `window` at `pixel_m` real metres (see
/// [`dem_grid`] for the geometry) and returns the raster with the bounds it covers. The rows are processed in
/// bands, each the rows whose centres lie in one tile row at `zoom_max`; only the tiles of that row and its two
/// neighbours (and the ancestors of the missing ones) are held at a time. A tile that is missing at `zoom_max`
/// is replaced by its ancestors down to [`Z_MIN`].
///
/// # Errors
///
/// [`FetchError::Other`] if `zoom_max` is outside [`Z_MIN`]..=[`Z_MAX`], the grid does not fit [`MAX_SIDE_PX`] and
/// [`MAX_PIXELS`] (checked before any tile is loaded) or a pixel is covered by no tile at any zoom; whatever
/// `store` or [`terrarium`] return.
pub fn resample(
    store: &dyn TileStore,
    window: &GeoWindow,
    pixel_m: f64,
    zoom_max: u8,
) -> Result<(DemMosaic, GeoWindow), FetchError> {
    if !(Z_MIN..=Z_MAX).contains(&zoom_max) {
        return Err(FetchError::Other(format!(
            "zoom {zoom_max} is outside {Z_MIN}..={Z_MAX}"
        )));
    }
    let (px, py, cols, rows) = grid_counts(window, pixel_m)?;
    let (width, height) = fit_grid(cols, rows)?;
    let (x0, x1, y0, y1) = tile_range(window, zoom_max);
    let tiles = 1u32 << zoom_max;
    let tile = f64::from(TILE_PX);
    let lat_of = |r: u32| window.north - (f64::from(r) + 0.5) * py;
    let band_of = |r: u32| tile_index(world_px(lat_of(r), 0.0, zoom_max).1 / tile, tiles);

    let mut data =
        Vec::with_capacity(usize::try_from(u64::from(width) * u64::from(height)).unwrap_or(0));
    let mut first = 0u32;
    while first < height {
        let ty = band_of(first);
        let mut end = first + 1;
        while end < height && band_of(end) == ty {
            end += 1;
        }
        let pyramid = load_band(store, zoom_max, ty, (x0, x1), (y0, y1))?;
        let lines: Vec<Result<Vec<f32>, FetchError>> = (first..end)
            .into_par_iter()
            .map(|r| {
                let lat = lat_of(r);
                (0..width)
                    .map(|c| {
                        let lon = window.west + (f64::from(c) + 0.5) * px;
                        pyramid.sample(lat, lon, zoom_max).map_or_else(
                            || Err(uncovered(lat, lon, zoom_max)),
                            |h| {
                                #[expect(
                                    clippy::cast_possible_truncation,
                                    reason = "heights of a few km fit f32"
                                )]
                                let h = h as f32;
                                Ok(h)
                            },
                        )
                    })
                    .collect()
            })
            .collect();
        for line in lines {
            data.extend(line?);
        }
        first = end;
    }
    let bounds = GeoWindow {
        west: window.west,
        north: window.north,
        east: window.west + f64::from(width) * px,
        south: window.north - f64::from(height) * py,
    };
    Ok((
        DemMosaic {
            width,
            height,
            data,
        },
        bounds,
    ))
}

/// The error for a pixel centre no tile serves, naming its tile at `zoom_max`.
fn uncovered(lat: f64, lon: f64, zoom_max: u8) -> FetchError {
    let (wx, wy) = world_px(lat, lon, zoom_max);
    let tiles = 1u32 << zoom_max;
    let (tx, ty) = (
        tile_index(wx / f64::from(TILE_PX), tiles),
        tile_index(wy / f64::from(TILE_PX), tiles),
    );
    FetchError::Other(format!(
        "no tile covers {lat} N {lon} E (tile {zoom_max}/{tx}/{ty} and its parents are missing)"
    ))
}

/// A remote Terrarium-encoded XYZ tile source. Sources with the same shape plug in as new
/// constants; sources with other wire formats (COG, .hgt, WMS) get their own fetch later --
/// the imagery catalogue (`naves_core::imagery::catalog`) is the pattern that grows into.
#[derive(Debug)]
pub struct TerrariumSource {
    /// The id used in file names and messages.
    pub id: &'static str,
    /// The human name.
    pub name: &'static str,
    /// The XYZ template with `{z}`, `{x}`, `{y}`.
    pub tile_url: &'static str,
    /// Where the source list and licences live.
    pub attribution_url: &'static str,
}

/// The only source v1 speaks.
pub const MAPTERHORN: TerrariumSource = TerrariumSource {
    id: "mapterhorn",
    name: "Mapterhorn",
    tile_url: "https://tiles.mapterhorn.com/{z}/{x}/{y}.webp",
    attribution_url: "https://mapterhorn.com/attribution",
};

/// Where tiles come from. `Ok(None)` is a 404: absent at this zoom.
pub trait TileGet: Sync {
    /// The bytes of tile `(z, x, y)`; `Ok(None)` if the source has no such tile.
    ///
    /// # Errors
    ///
    /// [`FetchError`] if the request fails or the answer cannot be used.
    fn tile(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, FetchError>;
}

/// Production source: a `ureq` agent like `RealHttp` (https only, user agent `naves/<version>`, connect
/// timeout 30 s, 2 minutes per tile), 404 is `Ok(None)`, other failures go through three tries, answers are
/// capped at [`MAX_TILE_BYTES`].
pub struct RealTiles {
    agent: ureq::Agent,
    source: &'static TerrariumSource,
}

impl RealTiles {
    fn with_agent(source: &'static TerrariumSource, https_only: bool) -> RealTiles {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .https_only(https_only)
            .max_redirects(5)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_mins(2)))
            .timeout_connect(Some(Duration::from_secs(30)))
            .user_agent(concat!("naves/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        RealTiles { agent, source }
    }

    /// The real network, `https` only.
    #[must_use]
    pub fn new(source: &'static TerrariumSource) -> RealTiles {
        Self::with_agent(source, true)
    }

    /// For tests against a server on the local machine, which speaks plain `http`.
    #[cfg(any(test, feature = "test-util"))]
    #[must_use]
    pub fn plain_for_tests(source: &'static TerrariumSource) -> RealTiles {
        Self::with_agent(source, false)
    }
}

impl TileGet for RealTiles {
    fn tile(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, FetchError> {
        let url = self
            .source
            .tile_url
            .replace("{z}", &z.to_string())
            .replace("{x}", &x.to_string())
            .replace("{y}", &y.to_string());
        let response = with_tries(&url, || {
            let response = self.agent.get(&url).call()?;
            match response.status().as_u16() {
                200 | 404 => Ok(response),
                code => Err(ureq::Error::StatusCode(code)),
            }
        })?;
        if response.status().as_u16() == 404 {
            return Ok(None);
        }
        let mut body = response.into_body();
        let mut reader = body
            .with_config()
            .limit(MAX_TILE_BYTES.saturating_add(1))
            .reader();
        let mut bytes = Vec::new();
        let mut limited = Limited::new(&mut bytes, MAX_TILE_BYTES);
        std::io::copy(&mut reader, &mut limited).map_err(|e| {
            if limited.exceeded {
                too_large(&url, MAX_TILE_BYTES)
            } else {
                http_error(&url, e)
            }
        })?;
        Ok(Some(bytes))
    }
}

/// What to fetch: `None` and `false` mean "discover the zoom", "2.5 m" and "refuse when only the world
/// fallback exists".
#[derive(Debug, Clone, Default)]
pub struct DemFetchRequest {
    /// The zoom to fetch at; skips the discovery and the refusal.
    pub zoom: Option<u8>,
    /// Target pixel in real metres (a floor: it grows until the raster fits).
    pub pixel_m: Option<f64>,
    /// Write the 30 m world fallback even when nothing finer covers the project.
    pub force: bool,
}

/// The refusal for a region with nothing finer than the world fallback.
const GLO_ONLY: &str = "Mapterhorn has nothing finer than Copernicus GLO-30 here; sources.dem is unchanged. \
Other sources exist (national portals, Sonny's 20 m, FABDEM if non-commercial), but none is implemented \u{2014} \
TileGet sources slot into dem_fetch.rs, other formats need their own fetcher. Pass --force to fetch the 30 m \
world fallback anyway.";

/// The project's tile cache, `<project>/dem/<id>/{z}/{x}/{y}.webp`, with an empty `{y}.404` for a tile the
/// source does not have. As a [`TileStore`] it never asks the network: a tile that is neither cached nor
/// marked reads as absent.
struct TileCache {
    root: PathBuf,
    source_id: &'static str,
}

impl TileCache {
    fn path(&self, z: u8, x: u32, y: u32, extension: &str) -> PathBuf {
        self.root
            .join(z.to_string())
            .join(x.to_string())
            .join(format!("{y}.{extension}"))
    }

    /// `Some(true)` if cached, `Some(false)` if marked absent, `None` if never asked.
    fn known(&self, z: u8, x: u32, y: u32) -> Option<bool> {
        if self.path(z, x, y, "webp").is_file() {
            Some(true)
        } else if self.path(z, x, y, "404").is_file() {
            Some(false)
        } else {
            None
        }
    }

    /// Whether tile `(z, x, y)` exists, asking `tiles` (and caching the answer) only when the cache does not know.
    fn ensure(&self, tiles: &dyn TileGet, z: u8, x: u32, y: u32) -> Result<bool, FetchError> {
        if let Some(known) = self.known(z, x, y) {
            return Ok(known);
        }
        if let Some(bytes) = tiles.tile(z, x, y)? {
            self.check(z, x, y, &bytes)?;
            write_atomic(&self.path(z, x, y, "webp"), &bytes)?;
            Ok(true)
        } else {
            write_atomic(&self.path(z, x, y, "404"), &[])?;
            Ok(false)
        }
    }

    /// Checks an answer before it is cached: a webp header of [`TILE_PX`] squared pixels, not too large.
    fn check(&self, z: u8, x: u32, y: u32, bytes: &[u8]) -> Result<(), FetchError> {
        let bad = |why: String| FetchError::Service {
            source_id: self.source_id.to_string(),
            why: format!("tile {z}/{x}/{y}: {why}"),
        };
        if u64::try_from(bytes.len()).is_ok_and(|n| n > MAX_TILE_BYTES) {
            return Err(bad(format!("larger than {MAX_TILE_BYTES} bytes")));
        }
        let size =
            image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::WebP)
                .into_dimensions()
                .map_err(|e| bad(format!("not a webp image: {e}")))?;
        if size != (TILE_PX, TILE_PX) {
            return Err(bad(format!(
                "an image of {} x {} pixels, not {TILE_PX} x {TILE_PX}",
                size.0, size.1
            )));
        }
        Ok(())
    }
}

impl TileStore for TileCache {
    fn load(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, FetchError> {
        let path = self.path(z, x, y, "webp");
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(FetchError::Io { path, source }),
        };
        let mut bytes = Vec::new();
        file.take(MAX_TILE_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|source| FetchError::Io {
                path: path.clone(),
                source,
            })?;
        if u64::try_from(bytes.len()).is_ok_and(|n| n > MAX_TILE_BYTES) {
            return Err(FetchError::Other(format!(
                "{}: larger than {MAX_TILE_BYTES} bytes",
                path.display()
            )));
        }
        Ok(Some(bytes))
    }
}

/// The tiles containing the window's four corners and its centre at `z`.
fn probe_tiles(window: &GeoWindow, z: u8) -> BTreeSet<(u32, u32)> {
    let tiles = 1u32 << z;
    let tile = f64::from(TILE_PX);
    let centre = (
        f64::midpoint(window.south, window.north),
        f64::midpoint(window.west, window.east),
    );
    [
        (window.north, window.west),
        (window.north, window.east),
        (window.south, window.west),
        (window.south, window.east),
        centre,
    ]
    .into_iter()
    .map(|(lat, lon)| {
        let (px, py) = world_px(lat, lon, z);
        (tile_index(px / tile, tiles), tile_index(py / tile, tiles))
    })
    .collect()
}

/// The zoom to fetch at: the requested one, or the finest at which the probes all exist.
fn choose_zoom(
    cache: &TileCache,
    tiles: &dyn TileGet,
    window: &GeoWindow,
    request: &DemFetchRequest,
    name: &str,
) -> Result<u8, FetchError> {
    if let Some(z) = request.zoom {
        if !(Z_MIN..=Z_MAX).contains(&z) {
            return Err(FetchError::Other(format!(
                "zoom {z} is outside {Z_MIN}..={Z_MAX}"
            )));
        }
        return Ok(z);
    }
    for z in (Z_MIN..=Z_MAX).rev() {
        let mut all = true;
        for (x, y) in probe_tiles(window, z) {
            if !cache.ensure(tiles, z, x, y)? {
                all = false;
                break;
            }
        }
        if all {
            return Ok(z);
        }
    }
    Err(FetchError::Other(format!(
        "{name} has no tiles for this area, not even at zoom {Z_MIN}"
    )))
}

/// The tiles of `range` as a list of `(x, y)`.
fn coords(range: (u32, u32, u32, u32)) -> Vec<(u32, u32)> {
    let (x0, x1, y0, y1) = range;
    (y0..=y1)
        .flat_map(|y| (x0..=x1).map(move |x| (x, y)))
        .collect()
}

/// What a fetch will do, for the user before anything is downloaded: zoom, tile count, how many are
/// cached, the expected download size ([`TILE_BYTES_ESTIMATE`] per missing tile), the grid and pixel. The
/// discovery may request, and cache, a few probe tiles.
///
/// # Errors
///
/// [`FetchError`] if the project cannot be read, the request is not valid or the discovery fails.
pub fn describe_dem_fetch(
    project_dir: &Path,
    source: &'static TerrariumSource,
    request: &DemFetchRequest,
    tiles: &dyn TileGet,
) -> Result<String, FetchError> {
    let project = Project::load(project_dir)?;
    let window = fetch_window(&project)?;
    let (cols, rows, pixel_m) = dem_grid(&window, request.pixel_m)?;
    let cache = TileCache {
        root: project_dir.join("dem").join(source.id),
        source_id: source.id,
    };
    let zoom = choose_zoom(&cache, tiles, &window, request, source.name)?;
    let all = coords(tile_range(&window, zoom));
    let cached = all
        .iter()
        .filter(|(x, y)| cache.known(zoom, *x, *y).is_some())
        .count();
    let total = u64::try_from(all.len()).unwrap_or(u64::MAX);
    let missing = total.saturating_sub(u64::try_from(cached).unwrap_or(u64::MAX));
    let megabytes = missing
        .saturating_mul(TILE_BYTES_ESTIMATE)
        .saturating_add(500_000)
        / 1_000_000;
    Ok(format!(
        "zoom {zoom}: {total} tiles, {cached} already cached, about {megabytes} MB to download; \
         a raster of {cols} x {rows} pixels at {pixel_m:.2} m"
    ))
}

/// Fetches the tiles `wanted` at zoom `z` into the cache on a pool of [`TILE_JOBS`] threads and returns which
/// of them exist.
fn fetch_level(
    pool: &rayon::ThreadPool,
    cache: &TileCache,
    tiles: &dyn TileGet,
    z: u8,
    wanted: &[(u32, u32)],
) -> Result<Vec<bool>, FetchError> {
    pool.install(|| {
        wanted
            .par_iter()
            .map(|&(x, y)| cache.ensure(tiles, z, x, y))
            .collect()
    })
}

/// Fetches every tile of the range at `zoom` and, for the missing ones, their ancestors (once each) down to
/// [`Z_MIN`]. Returns how many tiles of the range are served only at [`Z_MIN`], below `zoom`.
fn fetch_tiles(
    cache: &TileCache,
    tiles: &dyn TileGet,
    window: &GeoWindow,
    zoom: u8,
    note: &dyn Fn(&str),
) -> Result<(usize, usize), FetchError> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(TILE_JOBS)
        .build()
        .map_err(|e| FetchError::Other(format!("cannot start the download threads: {e}")))?;
    let all = coords(tile_range(window, zoom));
    let wanted: Vec<(u32, u32)> = all
        .iter()
        .copied()
        .filter(|&(x, y)| cache.known(zoom, x, y).is_none())
        .collect();
    note(&format!(
        "fetching {} of {} tiles at zoom {zoom}",
        wanted.len(),
        all.len()
    ));
    fetch_level(&pool, cache, tiles, zoom, &wanted)?;
    // (tile of the range, its ancestor at the level being looked at) of the tiles absent so far
    let mut pending: Vec<((u32, u32), (u32, u32))> = all
        .iter()
        .copied()
        .filter(|&(x, y)| cache.known(zoom, x, y) != Some(true))
        .map(|c| (c, c))
        .collect();
    let mut fallback = 0usize;
    for z in (Z_MIN..zoom).rev() {
        if pending.is_empty() {
            break;
        }
        for item in &mut pending {
            item.1 = (item.1.0 >> 1, item.1.1 >> 1);
        }
        let parents: Vec<(u32, u32)> = pending
            .iter()
            .map(|item| item.1)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let missing: Vec<(u32, u32)> = parents
            .iter()
            .copied()
            .filter(|&(x, y)| cache.known(z, x, y).is_none())
            .collect();
        note(&format!(
            "fetching {} tiles at zoom {z} for tiles missing at zoom {}",
            missing.len(),
            z + 1
        ));
        fetch_level(&pool, cache, tiles, z, &missing)?;
        let before = pending.len();
        pending.retain(|&(_, (x, y))| cache.known(z, x, y) != Some(true));
        if z == Z_MIN {
            fallback = before - pending.len();
        }
    }
    Ok((fallback, all.len()))
}

/// Today as `YYYY-MM-DD`.
fn today_text() -> String {
    let (y, m, d) = today();
    format!("{y:04}-{m:02}-{d:02}")
}

/// Fetches the bare-earth DEM of a project into `dem/<id>_dtm.tif` and lists it first in `sources.dem` of
/// `naves.toml`. Sequence: discover the zoom (probes, cached), refuse a region with only the world fallback
/// unless forced, fetch the missing tiles (four at a time, each cached atomically), resample from the cache,
/// write the file, write the attribution sidecar, upsert `sources.dem` and save the project. `note` gets short
/// progress lines; it is only called from the calling thread. Returns the paths listed in `naves.toml`
/// (relative, `/`-separated).
///
/// A failed fetch leaves `naves.toml` as it was (the tile cache may keep what was downloaded).
///
/// # Errors
///
/// [`FetchError`] if the project cannot be read or saved, the request is not valid, the source has no tiles for
/// the area (or only the world fallback and `force` is not set), a request fails or an answer is not a tile,
/// or the raster cannot be written.
pub fn fetch_dem(
    project_dir: &Path,
    source: &'static TerrariumSource,
    request: &DemFetchRequest,
    tiles: &dyn TileGet,
    note: &dyn Fn(&str),
) -> Result<Vec<PathBuf>, FetchError> {
    let project = Project::load(project_dir)?;
    let window = fetch_window(&project)?;
    let (_, _, pixel_m) = dem_grid(&window, request.pixel_m)?;
    let cache = TileCache {
        root: project_dir.join("dem").join(source.id),
        source_id: source.id,
    };
    let zoom = choose_zoom(&cache, tiles, &window, request, source.name)?;
    note(&format!("zoom {zoom}"));
    if request.zoom.is_none() && zoom == Z_MIN && !request.force {
        note(GLO_ONLY);
        return Err(FetchError::Other(GLO_ONLY.to_string()));
    }
    let (fallback, total) = fetch_tiles(&cache, tiles, &window, zoom, note)?;
    if fallback > 0 {
        note(&format!(
            "{fallback} of {total} tiles only have the 30 m world fallback"
        ));
    }
    note("resampling");
    let (mosaic, bounds) = resample(&cache, &window, pixel_m, zoom)?;
    let tif = format!("dem/{}_dtm.tif", source.id);
    write_dem(&project_dir.join(&tif), &mosaic, bounds)?;

    let extra = if fallback > 0 {
        format!("{fallback} of {total} tiles only have the 30 m world fallback.\n")
    } else {
        String::new()
    };
    let credit = format!(
        "Terrain elevation: {name} \u{2014} https://mapterhorn.com\n\
         Source list and licences: {url}\n\
         Retrieved: {date}  Window: {s:.6} {w:.6} to {n:.6} {e:.6}  Native zoom: up to z{zoom}\n\
         The national data is bare-earth (CC BY 4.0, Etalab 2.0, DL-DE, OGL where present); \
         the world fallback is Copernicus GLO-30.\n{extra}",
        name = source.name,
        url = source.attribution_url,
        date = today_text(),
        s = window.south,
        w = window.west,
        n = window.north,
        e = window.east,
    );
    write_atomic(
        &project_dir.join(format!("dem/{}_dtm.attribution.txt", source.id)),
        credit.as_bytes(),
    )?;

    let listed = PathBuf::from(&tif);
    let mut project = Project::load(project_dir)?;
    project.sources.dem.retain(|p| *p != listed);
    project.sources.dem.insert(0, listed.clone());
    project.save(project_dir)?;
    note(&format!("wrote {tif}"));
    Ok(vec![listed])
}

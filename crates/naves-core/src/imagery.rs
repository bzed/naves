// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Imagery input: satellite or aerial GeoTIFF files (8 bit RGB, optionally with an alpha band).
//!
//! Files are never read whole: [`ImageryRaster::open`] decodes only the pixels inside the window of the
//! map, from the coarsest overview that is still sharp enough for the map. Row 0 of a file is its northern
//! row. Colours are read through a [`RasterSampler`], which converts a latitude and longitude into the
//! coordinate system of the file.

use std::path::{Path, PathBuf};

use geotiff_reader::GeoTiffFile;
use proj4rs::Proj;

use crate::heightmap::MapGeo;
use crate::project::{ImageryFile, Project};
use crate::projection::{ProjectionError, utm_epsg};

pub mod catalog;
mod merge;

pub use merge::{Composite, MapRect, MergeReport, SourceReport, TileImagery, merge};

/// Largest accepted window side in pixels.
const MAX_WINDOW_SIDE: u32 = 16_000;

/// Real metres around the map that are read as well, so that the edge of the map has neighbours to
/// interpolate with.
const WINDOW_MARGIN_M: f64 = 100.0;

/// Points per side with which the border of the map is converted into the coordinates of a file.
const BORDER_STEPS: u32 = 32;

/// Real metres per degree of latitude (WGS84 equatorial radius times pi / 180).
const METRES_PER_DEGREE: f64 = 111_319.49;

/// A neighbour pixel with less weight than this is left out of the interpolation, so that a position at
/// the centre of an edge pixel has a colour even when rounding puts it a hair beyond.
const EDGE_WEIGHT: f64 = 1e-6;

/// Image pixels per land cell in the surface tiles of the terrain.
const PX_PER_LAND_CELL: f64 = 60.0;

/// Errors reading imagery files.
#[derive(Debug, thiserror::Error)]
pub enum ImageryError {
    /// The file is not an imagery GeoTIFF this reader supports; the message says how to convert it.
    #[error("{0}")]
    Unsupported(String),
    /// The GeoTIFF decoder rejected the file.
    #[error("cannot decode GeoTIFF: {0}")]
    GeoTiff(geotiff_reader::Error),
    /// A file of the project is not there; when a fetch wrote it, the message says how to get it again.
    #[error("{}: the file is missing{hint}", path.display())]
    Missing {
        /// The file, as the project folder has it.
        path: PathBuf,
        /// What to do about it: empty, or " (fetch it again with `naves imagery fetch <id>`)".
        hint: String,
    },
    /// The coordinates of the map cannot be converted.
    #[error(transparent)]
    Projection(#[from] ProjectionError),
    /// An error in one file.
    #[error("{}: {source}", path.display())]
    File {
        /// The file concerned.
        path: PathBuf,
        /// What went wrong.
        source: Box<ImageryError>,
    },
}

fn unsupported<T>(msg: impl Into<String>) -> Result<T, ImageryError> {
    Err(ImageryError::Unsupported(msg.into()))
}

fn geotiff_error(e: geotiff_reader::Error) -> ImageryError {
    match e {
        geotiff_reader::Error::NotGeoTiff => ImageryError::Unsupported(e.to_string()),
        other => {
            let text = other.to_string();
            if text.to_lowercase().contains("compress") {
                // the usual cause: JPEG inside the TIFF, which the decoder does not read
                ImageryError::Unsupported(format!(
                    "cannot decode GeoTIFF: {text} (a file with JPEG compression can be converted with \
                     `gdal_translate -co COMPRESS=DEFLATE in.tif out.tif`)"
                ))
            } else {
                ImageryError::GeoTiff(other)
            }
        }
    }
}

/// " (fetch it again with `naves imagery fetch <id>`)" if the file name is one that a fetch of a catalogue source
/// writes (`<id>.tif` or `<id>-<scene>.tif`), otherwise nothing.
fn fetch_hint(path: &Path) -> String {
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return String::new();
    };
    // the user's own catalogue counts too; a broken one is not this error's business
    let sources = catalog::catalog(catalog::user_catalog_path().as_deref())
        .or_else(|_| catalog::catalog(None))
        .unwrap_or_default();
    sources
        .iter()
        .filter(|s| stem == s.id || stem.strip_prefix(&s.id).is_some_and(|r| r.starts_with('-')))
        .max_by_key(|s| s.id.len())
        .map_or_else(String::new, |s| {
            format!(" (fetch it again with `naves imagery fetch {}`)", s.id)
        })
}

/// The imagery files of a project, read and merged.
#[derive(Debug)]
pub struct LoadedImagery {
    /// The merged picture of the map.
    pub composite: Composite,
    /// The files that touch the map, in the order of the project.
    pub used: Vec<ImageryFile>,
    /// What the merge found out about each file.
    pub report: MergeReport,
    /// Things the user should know: the merge's warnings and an empty result.
    pub warnings: Vec<String>,
}

/// Reads every file of `sources.imagery` that touches the map and merges them. `None` if the project lists no
/// imagery. Nothing is fetched: only the files in the project folder are read.
///
/// # Errors
///
/// [`ImageryError::File`] naming the first file that cannot be read, [`ImageryError::Projection`] if the map
/// cannot be converted.
pub fn load_imagery(
    project: &Project,
    project_dir: &Path,
    progress: &dyn Fn(&str),
) -> Result<Option<LoadedImagery>, ImageryError> {
    if project.sources.imagery.is_empty() {
        return Ok(None);
    }
    progress("Reading the imagery");
    let target_m = real_px_m(project);
    let (mut rasters, mut used) = (Vec::new(), Vec::new());
    for file in &project.sources.imagery {
        let path = project_dir.join(&file.path);
        if !path.is_file() {
            return Err(ImageryError::Missing {
                path: file.path.clone(),
                hint: fetch_hint(&file.path),
            });
        }
        if let Some(raster) = ImageryRaster::open(&path, project, target_m)? {
            rasters.push(raster.with_label(file.path.display().to_string()));
            used.push(file.clone());
        }
    }
    progress("Merging the imagery");
    let (composite, report) = merge(rasters, project)?;
    let mut warnings = report.warnings.clone();
    if used.is_empty() {
        warnings.push(format!(
            "none of the {} imagery file(s) touches the map; the class colours are drawn",
            project.sources.imagery.len()
        ));
    }
    Ok(Some(LoadedImagery {
        composite,
        used,
        report,
        warnings,
    }))
}

/// The size in real metres of one pixel of the surface images of the terrain: the `target_m` to pass to
/// [`ImageryRaster::open`].
#[must_use]
pub fn real_px_m(project: &Project) -> f64 {
    let cell = project.map_size_m() / f64::from(project.grid.land_size);
    cell / PX_PER_LAND_CELL / project.scale.horizontal
}

/// The coordinate systems of imagery files that are read.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Crs {
    /// EPSG:4326, degrees.
    Geographic,
    /// UTM on WGS84 (326xx, 327xx) or on GRS80 (ETRS89, 258xx).
    Utm { zone: u8, south: bool, grs80: bool },
    /// EPSG:3857, "Web Mercator".
    WebMercator,
}

impl Crs {
    fn from_epsg(code: u32) -> Option<Crs> {
        let utm = |zone: u32, south: bool, grs80: bool| {
            u8::try_from(zone)
                .ok()
                .filter(|z| (1..=60).contains(z))
                .map(|zone| Crs::Utm { zone, south, grs80 })
        };
        match code {
            4326 => Some(Crs::Geographic),
            3857 => Some(Crs::WebMercator),
            32_601..=32_660 => utm(code - 32_600, false, false),
            32_701..=32_760 => utm(code - 32_700, true, false),
            25_828..=25_838 => utm(code - 25_800, false, true),
            _ => None,
        }
    }

    fn proj(self) -> Option<Result<Proj, ProjectionError>> {
        let definition = match self {
            Crs::Geographic => return None,
            Crs::Utm { zone, south, grs80 } => format!(
                "+proj=utm +zone={zone}{}{} +units=m +no_defs",
                if south { " +south" } else { "" },
                if grs80 {
                    " +ellps=GRS80 +towgs84=0,0,0"
                } else {
                    " +datum=WGS84"
                }
            ),
            Crs::WebMercator => "+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 +x_0=0 +y_0=0 +k=1 +units=m +nadgrids=@null +no_defs".to_string(),
        };
        Some(Proj::from_proj_string(&definition).map_err(ProjectionError::from))
    }
}

/// The decoded window of one file.
#[derive(Debug, Clone)]
pub struct ImageryRaster {
    crs: Crs,
    /// Upper left corner of pixel (0, 0) of the whole image, in the units of the CRS.
    x0: f64,
    y0: f64,
    /// Pixel size at the level that was read.
    px: f64,
    py: f64,
    col0: u32,
    row0: u32,
    cols: u32,
    rows: u32,
    /// RGBA, alpha 0 for no data.
    data: Vec<u8>,
    pixel_m: f64,
    label: String,
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

/// Where the map is, in the coordinates of a file.
#[derive(Debug, Clone, Copy)]
struct Bounds {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    /// Latitude of the middle of the map, for the size of a pixel in metres.
    lat: f64,
}

/// Converts latitude and longitude (degrees) into the coordinates of a file.
struct ToCrs {
    crs: Crs,
    geo: Option<Proj>,
    file: Option<Proj>,
}

impl ToCrs {
    fn new(crs: Crs) -> Result<ToCrs, ImageryError> {
        let file = crs.proj().transpose()?;
        let geo = match file {
            Some(_) => Some(
                Proj::from_proj_string("+proj=longlat +datum=WGS84 +no_defs")
                    .map_err(ProjectionError::from)?,
            ),
            None => None,
        };
        Ok(ToCrs { crs, geo, file })
    }

    fn convert(&self, lat: f64, lon: f64) -> Option<(f64, f64)> {
        if !(lat.is_finite() && lon.is_finite()) {
            return None;
        }
        if let (Some(geo), Some(file)) = (&self.geo, &self.file) {
            let mut p = (lon.to_radians(), lat.to_radians());
            proj4rs::transform::transform(geo, file, &mut p).ok()?;
            (p.0.is_finite() && p.1.is_finite()).then_some(p)
        } else {
            debug_assert_eq!(self.crs, Crs::Geographic);
            Some((lon, lat))
        }
    }
}

/// The border of the map plus the margin, converted into the coordinates of the file.
fn map_bounds(project: &Project, crs: Crs) -> Result<Bounds, ImageryError> {
    let geo = MapGeo::new(project)?;
    let to_crs = ToCrs::new(crs)?;
    let margin = WINDOW_MARGIN_M * project.scale.horizontal;
    let (low, high) = (-margin, project.map_size_m() + margin);
    let mut b = Bounds {
        min_x: f64::INFINITY,
        min_y: f64::INFINITY,
        max_x: f64::NEG_INFINITY,
        max_y: f64::NEG_INFINITY,
        lat: project.area.center_lat,
    };
    for i in 0..=BORDER_STEPS {
        let t = low + (high - low) * f64::from(i) / f64::from(BORDER_STEPS);
        for (x, y) in [(t, low), (t, high), (low, t), (high, t)] {
            let (lat, lon) = geo.to_geo(x, y)?;
            let Some((fx, fy)) = to_crs.convert(lat, lon) else {
                return unsupported("the map cannot be converted into the coordinates of the file");
            };
            b.min_x = b.min_x.min(fx);
            b.max_x = b.max_x.max(fx);
            b.min_y = b.min_y.min(fy);
            b.max_y = b.max_y.max(fy);
        }
    }
    Ok(b)
}

/// Real metres of one pixel (the geometric mean of its width and height) at latitude `lat`.
fn real_pixel_m(crs: Crs, px: f64, py: f64, lat: f64) -> f64 {
    let cos = lat.to_radians().cos().abs().max(1e-6);
    let (x_m, y_m) = match crs {
        Crs::Geographic => (px * METRES_PER_DEGREE * cos, py * METRES_PER_DEGREE),
        Crs::Utm { .. } => (px, py),
        Crs::WebMercator => (px * cos, py * cos),
    };
    (x_m * y_m).sqrt()
}

/// The conversion hint for a file in a coordinate system that is not read.
fn crs_hint(project: &Project) -> String {
    let code = utm_epsg(project.area.center_lat, project.area.center_lon);
    format!("convert it with `gdalwarp -t_srs EPSG:{code} in.tif out.tif`")
}

const BYTE_HINT: &str = "convert it with `gdal_translate -ot Byte -scale in.tif out.tif` (separate band files: `gdal_merge.py -separate` first)";

impl ImageryRaster {
    /// Reads the pixels of `path` that lie inside the map, from the coarsest overview whose pixels are
    /// at most `target_m` real metres. `None` if the file does not touch the map.
    ///
    /// # Errors
    ///
    /// [`ImageryError::File`], naming the file, for a file that is not an 8 bit RGB GeoTIFF in a
    /// supported coordinate system, cannot be decoded, or needs a window above 16 000 pixels per side.
    pub fn open(
        path: &Path,
        project: &Project,
        target_m: f64,
    ) -> Result<Option<ImageryRaster>, ImageryError> {
        GeoTiffFile::open(path)
            .map_err(geotiff_error)
            .and_then(|file| read(&file, project, target_m))
            .map(|raster| {
                raster.map(|mut r| {
                    r.label = path.display().to_string();
                    r
                })
            })
            .map_err(|source| ImageryError::File {
                path: path.to_path_buf(),
                source: Box::new(source),
            })
    }

    /// Like [`ImageryRaster::open`] for a file that is already in memory (the entry point of the fuzz
    /// target).
    ///
    /// # Errors
    ///
    /// As [`ImageryRaster::open`], without the file name.
    pub fn from_bytes(
        bytes: Vec<u8>,
        project: &Project,
        target_m: f64,
    ) -> Result<Option<ImageryRaster>, ImageryError> {
        let file = GeoTiffFile::from_bytes(bytes).map_err(geotiff_error)?;
        read(&file, project, target_m)
    }

    /// Synthetic imagery for tests: `f(lat, lon)` is the colour at the pixel centres of a geographic grid
    /// with `pixel_deg` spacing that covers `west..east` and `south..north`; `None` is no data.
    #[cfg(any(test, feature = "test-util"))]
    #[must_use]
    pub fn from_fn(
        west: f64,
        south: f64,
        east: f64,
        north: f64,
        pixel_deg: f64,
        f: impl Fn(f64, f64) -> Option<[u8; 3]>,
    ) -> ImageryRaster {
        let side = |span: f64| index((span / pixel_deg).round()).unwrap_or(0);
        let (cols, rows) = (side(east - west), side(north - south));
        let mut data = Vec::new();
        for row in 0..rows {
            let lat = north - (f64::from(row) + 0.5) * pixel_deg;
            for col in 0..cols {
                let lon = west + (f64::from(col) + 0.5) * pixel_deg;
                match f(lat, lon) {
                    Some([r, g, b]) => data.extend_from_slice(&[r, g, b, 255]),
                    None => data.extend_from_slice(&[0, 0, 0, 0]),
                }
            }
        }
        ImageryRaster {
            crs: Crs::Geographic,
            x0: west,
            y0: north,
            px: pixel_deg,
            py: pixel_deg,
            col0: 0,
            row0: 0,
            cols,
            rows,
            data,
            pixel_m: real_pixel_m(
                Crs::Geographic,
                pixel_deg,
                pixel_deg,
                f64::midpoint(south, north),
            ),
            label: "synthetic imagery".to_string(),
        }
    }

    /// What the raster is called in messages: the file path for rasters from [`ImageryRaster::open`].
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The raster with another label.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> ImageryRaster {
        self.label = label.into();
        self
    }

    /// Real metres per pixel of what was read (the geometric mean of width and height).
    #[must_use]
    pub fn pixel_m(&self) -> f64 {
        self.pixel_m
    }

    /// Pixels held in memory.
    #[must_use]
    pub fn pixel_count(&self) -> usize {
        self.data.len() / 4
    }

    /// A reader of colours. One per thread: the coordinate conversion inside it is not shared.
    ///
    /// # Errors
    ///
    /// [`ImageryError::Projection`] if the coordinate system cannot be set up.
    pub fn sampler(&self) -> Result<RasterSampler<'_>, ImageryError> {
        Ok(RasterSampler {
            raster: self,
            to_crs: ToCrs::new(self.crs)?,
        })
    }

    /// The RGBA pixel at window-relative `col` and `row`, `None` outside or for no data.
    fn pixel(&self, col: i64, row: i64) -> Option<[u8; 3]> {
        let col = u32::try_from(col).ok().filter(|&c| c < self.cols)?;
        let row = u32::try_from(row).ok().filter(|&r| r < self.rows)?;
        let at = usize::try_from(row)
            .ok()?
            .checked_mul(usize::try_from(self.cols).ok()?)?
            .checked_add(usize::try_from(col).ok()?)?
            .checked_mul(4)?;
        match self.data.get(at..at.checked_add(4)?)? {
            [r, g, b, a] if *a != 0 => Some([*r, *g, *b]),
            _ => None,
        }
    }
}

/// Reads colours from an [`ImageryRaster`] at geographic positions.
pub struct RasterSampler<'a> {
    raster: &'a ImageryRaster,
    to_crs: ToCrs,
}

impl RasterSampler<'_> {
    /// The colour at (`lat`, `lon`) in degrees, bilinear between pixel centres. `None` outside the raster,
    /// for non-finite input, and if any pixel that contributes is no data.
    #[must_use]
    pub fn rgb_at(&self, lat: f64, lon: f64) -> Option<[f32; 3]> {
        let r = self.raster;
        let (x, y) = self.to_crs.convert(lat, lon)?;
        let fx = (x - r.x0) / r.px - 0.5 - f64::from(r.col0);
        let fy = (r.y0 - y) / r.py - 0.5 - f64::from(r.row0);
        if !(fx.is_finite() && fy.is_finite()) || fx.abs() > 1e9 || fy.abs() > 1e9 {
            return None;
        }
        let (c0, r0) = (fx.floor(), fy.floor());
        let (wx, wy) = (fx - c0, fy - r0);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "bounded to 1e9 above, floors of such values fit"
        )]
        let (c0, r0) = (c0 as i64, r0 as i64);
        let mut sum = [0.0_f64; 3];
        let mut weight = 0.0;
        for (dc, dr, w) in [
            (0, 0, (1.0 - wx) * (1.0 - wy)),
            (1, 0, wx * (1.0 - wy)),
            (0, 1, (1.0 - wx) * wy),
            (1, 1, wx * wy),
        ] {
            if w <= EDGE_WEIGHT {
                continue; // a pixel with (almost) no weight does not have to exist
            }
            let px = r.pixel(c0 + dc, r0 + dr)?;
            for (s, v) in sum.iter_mut().zip(px) {
                *s += w * f64::from(v);
            }
            weight += w;
        }
        if weight <= 0.0 {
            return None;
        }
        #[expect(clippy::cast_possible_truncation, reason = "colours are 0..=255")]
        Some([
            (sum[0] / weight) as f32,
            (sum[1] / weight) as f32,
            (sum[2] / weight) as f32,
        ])
    }
}

/// The nodata value of a file as a byte, if it has one.
fn nodata_byte(file: &GeoTiffFile) -> Option<u8> {
    let v: f64 = file.nodata()?.trim().parse().ok()?;
    index(v)
        .and_then(|i| u8::try_from(i).ok())
        .filter(|&b| (f64::from(b) - v).abs() < 1e-9)
}

#[expect(
    clippy::too_many_lines,
    reason = "one sequence of checks and the window arithmetic"
)]
fn read(
    file: &GeoTiffFile,
    project: &Project,
    target_m: f64,
) -> Result<Option<ImageryRaster>, ImageryError> {
    let (width, height) = (file.width(), file.height());
    let bands = file.band_count();
    if bands < 3 {
        return unsupported(format!("it has {bands} band(s), RGB needs 3: {BYTE_HINT}"));
    }
    let ifd = file
        .tiff()
        .ifd(file.base_ifd_index())
        .map_err(|e| geotiff_error(e.into()))?;
    let bits = ifd.bits_per_sample().map_err(|e| geotiff_error(e.into()))?;
    if bits.iter().any(|&b| b != 8) {
        return unsupported(format!(
            "it has {bits:?} bits per sample, not 8: {BYTE_HINT}"
        ));
    }
    let Some(epsg) = file.epsg() else {
        return unsupported(format!(
            "it has no EPSG code for its coordinate system; {}",
            crs_hint(project)
        ));
    };
    let Some(crs) = Crs::from_epsg(epsg) else {
        return unsupported(format!(
            "its coordinate system EPSG:{epsg} is not supported (EPSG:4326, 326xx, 327xx, 258xx and 3857 are); {}",
            crs_hint(project)
        ));
    };
    let Some(transform) = file.transform() else {
        return unsupported("it has no geo-referencing");
    };
    let (x0, y0) = transform.pixel_to_geo(0.0, 0.0);
    let (next_x, next_x_y) = transform.pixel_to_geo(1.0, 0.0);
    let (next_row_x, next_y) = transform.pixel_to_geo(0.0, 1.0);
    let (cell_w, cell_h) = (next_x - x0, y0 - next_y);
    let rotated = (next_x_y - y0).abs() > 1e-12 || (next_row_x - x0).abs() > 1e-12;
    let valid = |v: f64| v.is_finite() && v > 0.0;
    if rotated || !(valid(cell_w) && valid(cell_h) && x0.is_finite() && y0.is_finite()) {
        return unsupported("it has a rotated or invalid geo-transform");
    }
    let bounds = map_bounds(project, crs)?;

    // the coarsest level whose pixels are still at most as large as the map's own
    let base_m = real_pixel_m(crs, cell_w, cell_h, bounds.lat);
    let mut level: Option<usize> = None;
    let mut level_m = base_m;
    let mut dims = (width, height);
    for i in 0..file.overview_count() {
        let ov = file.overview_ifd(i).map_err(geotiff_error)?;
        let (ow, oh) = (ov.width(), ov.height());
        if ow == 0 || oh == 0 || ow > width || oh > height {
            continue;
        }
        let m = base_m * f64::from(width) / f64::from(ow);
        if m <= target_m && m > level_m {
            level = Some(i);
            level_m = m;
            dims = (ow, oh);
        }
    }
    let px = cell_w * f64::from(width) / f64::from(dims.0);
    let py = cell_h * f64::from(height) / f64::from(dims.1);

    let first_col = clamp_index(((bounds.min_x - x0) / px).floor(), dims.0);
    let end_col = clamp_index(((bounds.max_x - x0) / px).ceil(), dims.0);
    let first_row = clamp_index(((y0 - bounds.max_y) / py).floor(), dims.1);
    let end_row = clamp_index(((y0 - bounds.min_y) / py).ceil(), dims.1);
    if end_col <= first_col || end_row <= first_row {
        return Ok(None);
    }
    let (win_w, win_h) = (end_col - first_col, end_row - first_row);
    if win_w > MAX_WINDOW_SIDE || win_h > MAX_WINDOW_SIDE {
        return unsupported(format!(
            "the map needs a window of {win_w} x {win_h} pixels, more than the limit of {MAX_WINDOW_SIDE} x {MAX_WINDOW_SIDE}; \
             add or extend overviews with `gdaladdo -r average in.tif 2 4 8 16 32`"
        ));
    }
    let to_usize = |v: u32| usize::try_from(v).or_else(|_| unsupported("window too large"));
    let (row_off, col_off, rows, cols) = (
        to_usize(first_row)?,
        to_usize(first_col)?,
        to_usize(win_h)?,
        to_usize(win_w)?,
    );
    let raster = match level {
        None => file.read_window::<u8>(row_off, col_off, rows, cols),
        Some(i) => file.read_overview_window::<u8>(i, row_off, col_off, rows, cols),
    }
    .map_err(geotiff_error)?;
    let bands = to_usize(bands)?;
    if raster.shape() != [rows, cols, bands] {
        return unsupported(format!(
            "decoded shape {:?} does not match the window {:?}",
            raster.shape(),
            [rows, cols, bands]
        ));
    }
    let samples: Vec<u8> = raster.iter().copied().collect();
    let nodata = nodata_byte(file);
    let mut data = Vec::with_capacity(samples.len() / bands * 4);
    for px in samples.chunks_exact(bands) {
        let [r, g, b, rest @ ..] = px else {
            return unsupported("decoded pixel with fewer than three bands");
        };
        let alpha_zero = rest.first().is_some_and(|&a| a == 0);
        let is_nodata = nodata.is_some_and(|n| *r == n && *g == n && *b == n);
        data.extend_from_slice(&[*r, *g, *b, if alpha_zero || is_nodata { 0 } else { 255 }]);
    }
    Ok(Some(ImageryRaster {
        crs,
        x0,
        y0,
        px,
        py,
        col0: first_col,
        row0: first_row,
        cols: win_w,
        rows: win_h,
        data,
        pixel_m: level_m,
        label: "imagery".to_string(),
    }))
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fetching imagery into a project.
//!
//! `naves imagery fetch` (also in the GUI and over MCP) turns a catalogue entry into ordinary GeoTIFF files in the
//! `imagery` folder of the project and lists them in `naves.toml` with their licence. `naves build` never calls
//! anything in this module: its signature has no [`Http`] to do it with.
//!
//! The server's answers are untrusted: every download has a size limit, images are decoded with limits and must
//! have the size that was asked for, scene files may only come from the hosts of the catalogue entry over
//! `https`, scene ids are checked before they become part of a file name, and nothing is replaced until the new
//! file is whole.

use std::collections::BTreeMap;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use naves_core::heightmap::MapGeo;
use naves_core::imagery::catalog::{CatalogSource, SourceKind};
use naves_core::imagery::{ImageryError, ImageryRaster, real_px_m};
use naves_core::landcover::GeoWindow;
use naves_core::project::{ImageryFile, Project, ProjectError};
use naves_core::projection::ProjectionError;
use serde::Deserialize;

use crate::geotiff_write::{RgbMosaic, write_rgb};

/// Largest answer to one `GetMap` request.
const MAX_IMAGE_BYTES: u64 = 64 << 20;
/// Largest search answer.
const MAX_JSON_BYTES: u64 = 16 << 20;
/// Largest scene file.
const MAX_SCENE_BYTES: u64 = 3 << 30;
/// Largest side of a fetched picture in pixels, as for the land cover.
const MAX_SIDE_PX: u32 = 16_000;
/// Largest side of one `GetMap` tile in pixels, whatever the service allows.
const MAX_TILE_PX: u32 = 2048;
/// Most scenes of one fetch.
const MAX_SCENES: usize = 6;
/// Scenes with more cloud than this (percent) are not used.
const MAX_CLOUD_PERCENT: f64 = 20.0;
/// Points per side of the lattice that decides whether scenes cover the map.
const LATTICE: usize = 33;
/// Real metres around the map that are fetched as well, as the reader needs them.
const WINDOW_MARGIN_M: f64 = 100.0;
/// Real metres per degree of latitude.
const METRES_PER_DEGREE: f64 = 111_319.49;
/// Scenes per search answer that are looked at.
const SEARCH_LIMIT: u32 = 100;
/// Tries of one request, with growing pauses.
const TRIES: u32 = 3;

/// Why a fetch failed.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// The project cannot be read or saved.
    #[error(transparent)]
    Project(#[from] ProjectError),
    /// An imagery file cannot be read.
    #[error(transparent)]
    Imagery(#[from] ImageryError),
    /// A position of the map cannot be converted.
    #[error(transparent)]
    Projection(#[from] ProjectionError),
    /// A request failed.
    #[error("{url}: {why}")]
    Http {
        /// The address.
        url: String,
        /// What went wrong.
        why: String,
    },
    /// The source answered something that cannot be used; the message names the source.
    #[error("{source_id}: {why}")]
    Service {
        /// The id of the catalogue entry.
        source_id: String,
        /// What is wrong with the answer.
        why: String,
    },
    /// A file cannot be written.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file.
        path: PathBuf,
        /// What went wrong.
        source: std::io::Error,
    },
    /// Anything else, in words.
    #[error("{0}")]
    Other(String),
}

/// The network, as far as fetching needs it. The tests use a fake; [`RealHttp`] is the real one.
///
/// Implementations must stop reading after `max_bytes` and fail with a message that says "larger than"; callers
/// do not trust them to, and cut the data off themselves.
pub trait Http: Sync {
    /// Downloads `url` into `out`.
    ///
    /// # Errors
    ///
    /// [`FetchError::Http`] if the request fails or the answer is larger than `max_bytes`.
    fn get(&self, url: &str, max_bytes: u64, out: &mut dyn Write) -> Result<(), FetchError>;

    /// Posts `body` (JSON) to `url` and returns the answer.
    ///
    /// # Errors
    ///
    /// [`FetchError::Http`] if the request fails or the answer is larger than `max_bytes`.
    fn post_json(&self, url: &str, body: &str, max_bytes: u64) -> Result<Vec<u8>, FetchError>;
}

/// The real network: `https` only, with timeouts, three tries and the user agent `naves/<version>`.
pub struct RealHttp {
    agent: ureq::Agent,
}

impl RealHttp {
    fn with_agent(https_only: bool) -> RealHttp {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .https_only(https_only)
            .max_redirects(5)
            .timeout_global(Some(Duration::from_mins(30)))
            .timeout_connect(Some(Duration::from_secs(30)))
            .user_agent(concat!("naves/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        RealHttp { agent }
    }

    /// The real network, `https` only.
    #[must_use]
    pub fn new() -> RealHttp {
        Self::with_agent(true)
    }

    /// For tests against a server on the local machine, which speaks plain `http`.
    #[cfg(any(test, feature = "test-util"))]
    #[must_use]
    pub fn plain_for_tests() -> RealHttp {
        Self::with_agent(false)
    }
}

impl Default for RealHttp {
    fn default() -> Self {
        Self::new()
    }
}

fn http_error(url: &str, why: impl std::fmt::Display) -> FetchError {
    FetchError::Http {
        url: url.to_string(),
        why: why.to_string(),
    }
}

/// Whether a failure is worth another try: the network, a timeout or a server error.
fn transient(error: &ureq::Error) -> bool {
    match error {
        ureq::Error::StatusCode(code) => *code >= 500 || *code == 429,
        ureq::Error::Io(_) | ureq::Error::Timeout(_) | ureq::Error::ConnectionFailed => true,
        _ => false,
    }
}

/// Runs `request` up to [`TRIES`] times, with one second more of pause each time.
fn with_tries<T>(
    url: &str,
    mut request: impl FnMut() -> Result<T, ureq::Error>,
) -> Result<T, FetchError> {
    let mut last = None;
    for attempt in 0..TRIES {
        match request() {
            Ok(done) => return Ok(done),
            Err(e) if transient(&e) && attempt + 1 < TRIES => {
                last = Some(e);
                std::thread::sleep(Duration::from_secs(u64::from(attempt) + 1));
            }
            Err(e) => return Err(http_error(url, e)),
        }
    }
    Err(http_error(
        url,
        last.map_or_else(|| "no answer".to_string(), |e| e.to_string()),
    ))
}

impl Http for RealHttp {
    fn get(&self, url: &str, max_bytes: u64, out: &mut dyn Write) -> Result<(), FetchError> {
        let response = with_tries(url, || self.agent.get(url).call())?;
        let mut body = response.into_body();
        let mut reader = body
            .with_config()
            .limit(max_bytes.saturating_add(1))
            .reader();
        let mut limited = Limited::new(out, max_bytes);
        std::io::copy(&mut reader, &mut limited).map_err(|e| {
            if limited.exceeded {
                too_large(url, max_bytes)
            } else {
                http_error(url, e)
            }
        })?;
        Ok(())
    }

    fn post_json(&self, url: &str, body: &str, max_bytes: u64) -> Result<Vec<u8>, FetchError> {
        let response = with_tries(url, || {
            self.agent
                .post(url)
                .content_type("application/json")
                .send(body)
        })?;
        let mut body = response.into_body();
        let mut reader = body
            .with_config()
            .limit(max_bytes.saturating_add(1))
            .reader();
        let mut bytes = Vec::new();
        let mut limited = Limited::new(&mut bytes, max_bytes);
        std::io::copy(&mut reader, &mut limited).map_err(|e| {
            if limited.exceeded {
                too_large(url, max_bytes)
            } else {
                http_error(url, e)
            }
        })?;
        Ok(bytes)
    }
}

fn too_large(url: &str, max_bytes: u64) -> FetchError {
    http_error(url, format!("the answer is larger than {max_bytes} bytes"))
}

/// A writer that fails after `left` bytes.
struct Limited<'a> {
    inner: &'a mut dyn Write,
    left: u64,
    exceeded: bool,
}

impl<'a> Limited<'a> {
    fn new(inner: &'a mut dyn Write, limit: u64) -> Limited<'a> {
        Limited {
            inner,
            left: limit,
            exceeded: false,
        }
    }
}

impl Write for Limited<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let len = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        if len > self.left {
            self.exceeded = true;
            return Err(std::io::Error::other("answer too large"));
        }
        self.left -= len;
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Downloads `url` into memory, at most `limit` bytes.
fn get_bytes(http: &dyn Http, url: &str, limit: u64) -> Result<Vec<u8>, FetchError> {
    let mut bytes = Vec::new();
    let mut limited = Limited::new(&mut bytes, limit);
    match http.get(url, limit, &mut limited) {
        Ok(()) => Ok(bytes),
        Err(_) if limited.exceeded => Err(too_large(url, limit)),
        Err(e) => Err(e),
    }
}

/// What to fetch beyond the source: dates for a source of scenes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FetchRequest {
    /// First day of the search, `YYYY-MM-DD`.
    pub from: Option<String>,
    /// Last day of the search, `YYYY-MM-DD`.
    pub to: Option<String>,
}

/// The windows of the last two complete years in which scenes are searched by default: May to September north
/// of the equator, November to March south of it. `today` is (year, month, day).
#[must_use]
pub fn default_range(lat: f64, today: (i32, u32, u32)) -> Vec<(String, String)> {
    let year = today.0;
    [year - 2, year - 1]
        .into_iter()
        .map(|y| {
            if lat >= 0.0 {
                (format!("{y:04}-05-01"), format!("{y:04}-09-30"))
            } else {
                (format!("{:04}-11-01", y - 1), format!("{y:04}-03-31"))
            }
        })
        .collect()
}

/// Today's date as (year, month, day), in UTC.
fn today() -> (i32, u32, u32) {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    // days since 1970-01-01 to a civil date (H. Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let year = i32::try_from(yoe + era * 400 + i64::from(month <= 2)).unwrap_or(1970);
    (year, month, day)
}

/// `YYYY-MM-DD` with a real-looking month and day.
fn valid_date(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return false;
    };
    let number = |s: &str, len: usize| s.len() == len && s.bytes().all(|b| b.is_ascii_digit());
    number(year, 4)
        && number(month, 2)
        && number(day, 2)
        && month.parse::<u32>().is_ok_and(|m| (1..=12).contains(&m))
        && day.parse::<u32>().is_ok_and(|d| (1..=31).contains(&d))
}

/// The window of the map plus the margin, in degrees.
fn window_of(project: &Project) -> Result<GeoWindow, FetchError> {
    Ok(MapGeo::new(project)?.window(WINDOW_MARGIN_M * project.scale.horizontal)?)
}

fn service<T>(source: &CatalogSource, why: impl Into<String>) -> Result<T, FetchError> {
    Err(FetchError::Service {
        source_id: source.id.clone(),
        why: why.into(),
    })
}

/// The number of pixels across (columns) and down (rows) of a picture of `window` with `pixel_m` real metres per
/// pixel.
fn counts(window: GeoWindow, pixel_m: f64) -> (u64, u64) {
    let mid = f64::midpoint(window.south, window.north).to_radians();
    let width_m = (window.east - window.west) * METRES_PER_DEGREE * mid.cos().abs().max(1e-6);
    let height_m = (window.north - window.south) * METRES_PER_DEGREE;
    let count = |metres: f64| -> u64 {
        let pixels = (metres / pixel_m).ceil().max(1.0);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "positive, and capped below"
        )]
        let pixels = pixels.min(1e9) as u64;
        pixels
    };
    (count(width_m), count(height_m))
}

/// The pixel grid of a picture of `window` with `pixel_m` real metres per pixel: (columns, rows).
fn grid_of(
    window: GeoWindow,
    pixel_m: f64,
    source: &CatalogSource,
) -> Result<(u32, u32), FetchError> {
    let (cols, rows) = counts(window, pixel_m);
    let side = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
    if cols > u64::from(MAX_SIDE_PX) || rows > u64::from(MAX_SIDE_PX) {
        return service(
            source,
            format!(
                "the area needs a picture of {cols} x {rows} pixels, more than the limit of {MAX_SIDE_PX}; \
                 use a coarser source or a smaller map"
            ),
        );
    }
    Ok((side(cols), side(rows)))
}

/// The picture a WMS fetch makes of the map: (columns, rows, real metres per pixel). The pixel is the sharper of the
/// source's and the map's own; where the picture would be wider than 16 000 pixels (a large map) the pixel is made
/// larger until it fits, because a service can draw at any scale.
///
/// # Errors
///
/// [`FetchError`] if the map cannot be converted into degrees.
pub fn picture_grid(
    project: &Project,
    source: &CatalogSource,
) -> Result<(u32, u32, f64), FetchError> {
    let window = window_of(project)?;
    let mut pixel_m = source.native_m.max(real_px_m(project));
    for _ in 0..4 {
        let (cols, rows) = counts(window, pixel_m);
        let longest = cols.max(rows);
        if longest <= u64::from(MAX_SIDE_PX) {
            break;
        }
        // one step is enough but for rounding up: a little extra so that the next count fits
        #[expect(clippy::cast_precision_loss, reason = "pixel counts are below 1e9")]
        let factor = longest as f64 / f64::from(MAX_SIDE_PX);
        pixel_m *= factor * 1.001;
    }
    let (cols, rows) = grid_of(window, pixel_m, source)?;
    Ok((cols, rows, pixel_m))
}

/// What a fetch of `source` will do for the map of `project`, in a sentence: the size of the picture or the
/// number and size of scenes. For the user to read before anything is downloaded.
///
/// # Errors
///
/// [`FetchError`] if the map cannot be converted or the area is too large for the source.
pub fn describe_fetch(project: &Project, source: &CatalogSource) -> Result<String, FetchError> {
    match &source.kind {
        SourceKind::Wms { max_px, .. } => {
            let (cols, rows, pixel_m) = picture_grid(project, source)?;
            let wanted = source.native_m.max(real_px_m(project));
            let tile = (*max_px).clamp(1, MAX_TILE_PX);
            let requests = cols.div_ceil(tile) * rows.div_ceil(tile);
            let megabytes = u64::from(cols) * u64::from(rows) / 1_000_000;
            let coarser = if pixel_m > wanted * 1.01 {
                format!(
                    "; the map is too large for {wanted:.2} m per pixel, so it is fetched coarser, at {pixel_m:.2} m"
                )
            } else {
                String::new()
            };
            Ok(format!(
                "a picture of {cols} x {rows} pixels in {requests} request(s), about {} MB before it is compressed{coarser}",
                megabytes * 3
            ))
        }
        SourceKind::Stac { .. } => Ok(format!(
            "up to {MAX_SCENES} scenes of about 340 MB each; only the part of a scene that lies on the map is kept"
        )),
    }
}

/// Fetches `source` for the map of the project in `project_dir`: writes GeoTIFF files into its `imagery`
/// folder, lists them in `naves.toml` (replacing entries with the same path) and returns what was listed.
/// `progress` is called with a short line before every step; lines that start with `warning` are for the user.
///
/// Nothing is replaced until the new data is whole, and the project is saved last, so a failed fetch leaves the
/// project and its earlier files as they were.
///
/// # Errors
///
/// [`FetchError`], naming the source, if the service fails, answers something that is not usable, or has nothing
/// for the area.
pub fn fetch(
    project_dir: &Path,
    source: &CatalogSource,
    request: &FetchRequest,
    http: &dyn Http,
    progress: &dyn Fn(&str),
) -> Result<Vec<ImageryFile>, FetchError> {
    let project = Project::load(project_dir)?;
    let window = window_of(&project)?;
    let files = match &source.kind {
        SourceKind::Wms {
            url,
            layer,
            max_px,
            blank,
        } => fetch_wms(
            &Context {
                project_dir,
                project: &project,
                window,
                source,
                http,
                progress,
            },
            &WmsParams {
                url,
                layer,
                max_px: *max_px,
                blank: *blank,
            },
        )?,
        SourceKind::Stac {
            url,
            collection,
            asset,
            asset_hosts,
        } => fetch_stac(
            &Context {
                project_dir,
                project: &project,
                window,
                source,
                http,
                progress,
            },
            &StacParams {
                url,
                collection,
                asset,
                asset_hosts,
                request,
            },
        )?,
    };
    let mut project = Project::load(project_dir)?;
    for file in &files {
        match project
            .sources
            .imagery
            .iter_mut()
            .find(|f| f.path == file.path)
        {
            Some(slot) => *slot = file.clone(),
            None => project.sources.imagery.push(file.clone()),
        }
    }
    project.save(project_dir)?;
    Ok(files)
}

/// What every kind of fetch needs.
struct Context<'a> {
    project_dir: &'a Path,
    project: &'a Project,
    window: GeoWindow,
    source: &'a CatalogSource,
    http: &'a dyn Http,
    progress: &'a dyn Fn(&str),
}

impl Context<'_> {
    fn imagery_dir(&self) -> PathBuf {
        self.project_dir.join("imagery")
    }

    fn listed(&self, file_name: &str, attribution: String) -> ImageryFile {
        ImageryFile {
            path: PathBuf::from("imagery").join(file_name),
            attribution,
            licence: self.source.licence.clone(),
            non_commercial: self.source.non_commercial,
        }
    }
}

// ---- WMS ----

struct WmsParams<'a> {
    url: &'a str,
    layer: &'a str,
    max_px: u32,
    blank: Option<[u8; 3]>,
}

/// `text` with everything but letters, digits and `-._~` percent-encoded.
fn percent_encode(text: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            // writing to a String cannot fail
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// A decoded tile of exactly `width` x `height` pixels.
fn decode_tile(
    source: &CatalogSource,
    bytes: &[u8],
    (width, height): (u32, u32),
) -> Result<image::RgbaImage, FetchError> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| FetchError::Other(e.to_string()))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(width);
    limits.max_image_height = Some(height);
    limits.max_alloc = Some(512 << 20);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| {
        let head: String =
            String::from_utf8_lossy(bytes.get(..bytes.len().min(300)).unwrap_or(bytes))
                .chars()
                .filter(|c| !c.is_control() || *c == ' ')
                .collect();
        FetchError::Service {
            source_id: source.id.clone(),
            why: format!("the service answered something that is not an image ({e}): {head}"),
        }
    })?;
    let rgba = decoded.to_rgba8();
    if rgba.dimensions() != (width, height) {
        return service(
            source,
            format!(
                "the service answered an image of {} x {} pixels, not the {width} x {height} that was asked for",
                rgba.width(),
                rgba.height()
            ),
        );
    }
    Ok(rgba)
}

/// The colour of a pixel in the mosaic: no data is `[0, 0, 0]`, so a real black becomes `[1, 1, 1]`.
fn mosaic_colour(pixel: image::Rgba<u8>, blank: Option<[u8; 3]>) -> [u8; 3] {
    let [r, g, b, a] = pixel.0;
    if a == 0 || blank == Some([r, g, b]) {
        [0, 0, 0]
    } else if [r, g, b] == [0, 0, 0] {
        [1, 1, 1]
    } else {
        [r, g, b]
    }
}

fn fetch_wms(cx: &Context<'_>, wms: &WmsParams<'_>) -> Result<Vec<ImageryFile>, FetchError> {
    let source = cx.source;
    let window = cx.window;
    let (cols, rows, pixel_m) = picture_grid(cx.project, source)?;
    let wanted = source.native_m.max(real_px_m(cx.project));
    if pixel_m > wanted * 1.01 {
        (cx.progress)(&format!(
            "warning: the map is too large for {wanted:.2} m per pixel; fetching coarser, at {pixel_m:.2} m"
        ));
    }
    let tile = wms.max_px.clamp(1, MAX_TILE_PX);
    let (dx, dy) = (
        (window.east - window.west) / f64::from(cols),
        (window.north - window.south) / f64::from(rows),
    );
    let side = |v: u32| usize::try_from(v).unwrap_or(usize::MAX);
    let total = side(cols)
        .checked_mul(side(rows))
        .and_then(|p| p.checked_mul(3))
        .ok_or_else(|| FetchError::Other("the picture is too large".to_string()))?;
    let mut data = vec![0_u8; total];
    let tiles_x = cols.div_ceil(tile);
    let tiles_y = rows.div_ceil(tile);
    let mut number = 0_u32;
    let separator = if wms.url.contains('?') { '&' } else { '?' };
    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            number += 1;
            let (col, row) = (tx * tile, ty * tile);
            let (width, height) = (tile.min(cols - col), tile.min(rows - row));
            (cx.progress)(&format!(
                "Fetching {} tile {number} of {}",
                source.id,
                tiles_x * tiles_y
            ));
            let west = window.west + f64::from(col) * dx;
            let east = window.west + f64::from(col + width) * dx;
            let north = window.north - f64::from(row) * dy;
            let south = window.north - f64::from(row + height) * dy;
            // WMS 1.3.0 with EPSG:4326 orders the box as latitude, longitude
            let url = format!(
                "{}{separator}service=WMS&version=1.3.0&request=GetMap&layers={}&styles=&crs=EPSG:4326\
                 &bbox={south},{west},{north},{east}&width={width}&height={height}&format=image/png&transparent=true",
                wms.url,
                percent_encode(wms.layer)
            );
            let bytes = get_bytes(cx.http, &url, MAX_IMAGE_BYTES)?;
            let image = decode_tile(source, &bytes, (width, height))?;
            for (x, y, pixel) in image.enumerate_pixels() {
                let at = (side(row + y) * side(cols) + side(col + x)) * 3;
                if let Some(slot) = data.get_mut(at..at + 3) {
                    slot.copy_from_slice(&mosaic_colour(*pixel, wms.blank));
                }
            }
        }
    }
    if data.iter().all(|&b| b == 0) {
        return service(
            source,
            "this source has nothing for this area (every pixel is empty or blank)",
        );
    }
    let name = format!("{}.tif", source.id);
    write_rgb(
        &cx.imagery_dir().join(&name),
        &RgbMosaic {
            width: cols,
            height: rows,
            data,
        },
        window,
    )?;
    Ok(vec![cx.listed(&name, source.attribution.clone())])
}

// ---- STAC ----

struct StacParams<'a> {
    url: &'a str,
    collection: &'a str,
    asset: &'a str,
    asset_hosts: &'a [String],
    request: &'a FetchRequest,
}

#[derive(Deserialize)]
struct SearchAnswer {
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    id: String,
    geometry: Option<Geometry>,
    #[serde(default)]
    properties: Properties,
    #[serde(default)]
    assets: BTreeMap<String, Asset>,
}

#[derive(Deserialize)]
struct Geometry {
    #[serde(rename = "type")]
    kind: String,
    coordinates: serde_json::Value,
}

#[derive(Deserialize, Default)]
struct Properties {
    datetime: Option<String>,
    #[serde(rename = "eo:cloud_cover")]
    cloud: Option<f64>,
    #[serde(rename = "s2:snow_ice_percentage")]
    snow: Option<f64>,
}

#[derive(Deserialize)]
struct Asset {
    href: String,
}

/// The outer rings of a `Polygon` or `MultiPolygon`, as (longitude, latitude) points.
fn rings_of(geometry: &Geometry) -> Vec<Vec<(f64, f64)>> {
    fn ring(value: &serde_json::Value) -> Option<Vec<(f64, f64)>> {
        value
            .as_array()?
            .iter()
            .map(|p| {
                let p = p.as_array()?;
                Some((p.first()?.as_f64()?, p.get(1)?.as_f64()?))
            })
            .collect()
    }
    let outer = |polygon: &serde_json::Value| polygon.as_array()?.first().and_then(ring);
    match geometry.kind.as_str() {
        "Polygon" => outer(&geometry.coordinates).into_iter().collect(),
        "MultiPolygon" => geometry
            .coordinates
            .as_array()
            .map(|polygons| polygons.iter().filter_map(outer).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Whether the point is inside the ring (even-odd rule).
fn inside(ring: &[(f64, f64)], (x, y): (f64, f64)) -> bool {
    let mut inside = false;
    for (i, &(x1, y1)) in ring.iter().enumerate() {
        let Some(&(x2, y2)) = ring.get((i + 1) % ring.len()) else {
            continue;
        };
        if (y1 > y) != (y2 > y) && x < (x2 - x1) * (y - y1) / (y2 - y1) + x1 {
            inside = !inside;
        }
    }
    inside
}

/// A scene id that is safe as part of a file name: letters, digits, `.`, `_` and `-`, not starting with a dot.
fn valid_scene_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// The host of an `https://` address, if it has no user name or port.
fn https_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty() && !host.contains(['@', ':', ' '])).then_some(host)
}

struct Candidate {
    id: String,
    cloud: f64,
    snow: f64,
    year: Option<String>,
    href: String,
    rings: Vec<Vec<(f64, f64)>>,
}

/// The scenes of one search answer that have everything that is needed to use them.
fn candidates(
    source: &CatalogSource,
    asset: &str,
    bytes: &[u8],
) -> Result<Vec<Candidate>, FetchError> {
    let answer: SearchAnswer = serde_json::from_slice(bytes).map_err(|e| FetchError::Service {
        source_id: source.id.clone(),
        why: format!("cannot read the search answer: {e}"),
    })?;
    Ok(answer
        .features
        .into_iter()
        .filter_map(|f| {
            let href = f.assets.get(asset)?.href.clone();
            let rings = rings_of(f.geometry.as_ref()?);
            let year = f
                .properties
                .datetime
                .as_deref()
                .and_then(|d| d.get(..4))
                .filter(|y| y.bytes().all(|b| b.is_ascii_digit()))
                .map(str::to_string);
            Some(Candidate {
                id: f.id,
                cloud: f.properties.cloud.unwrap_or(100.0),
                snow: f.properties.snow.unwrap_or(0.0),
                year,
                href,
                rings,
            })
        })
        .collect())
}

/// How many scenes of a search answer have everything that is needed to use them: the entry point of the fuzz
/// target and of the test against the real service.
///
/// # Errors
///
/// [`FetchError::Service`] naming `source` if `bytes` is not a search answer.
pub fn search_scene_count(
    source: &CatalogSource,
    asset: &str,
    bytes: &[u8],
) -> Result<usize, FetchError> {
    Ok(candidates(source, asset, bytes)?.len())
}

/// The search body: the window as a polygon, the dates, the clearest scenes first.
fn search_body(stac: &StacParams<'_>, window: GeoWindow, from: &str, to: &str) -> String {
    let (w, s, e, n) = (window.west, window.south, window.east, window.north);
    let mut sortby =
        vec![serde_json::json!({"field": "properties.eo:cloud_cover", "direction": "asc"})];
    if stac.collection.starts_with("sentinel-2") {
        sortby.push(
            serde_json::json!({"field": "properties.s2:snow_ice_percentage", "direction": "asc"}),
        );
    }
    serde_json::json!({
        "collections": [stac.collection],
        "intersects": {"type": "Polygon", "coordinates": [[[w, s], [e, s], [e, n], [w, n], [w, s]]]},
        "datetime": format!("{from}T00:00:00Z/{to}T23:59:59Z"),
        "limit": SEARCH_LIMIT,
        "sortby": sortby,
    })
    .to_string()
}

/// Searches the scenes of the dates, the clearest first.
fn search_scenes(cx: &Context<'_>, stac: &StacParams<'_>) -> Result<Vec<Candidate>, FetchError> {
    let source = cx.source;
    let ranges = match (&stac.request.from, &stac.request.to) {
        (Some(from), Some(to)) => {
            if !valid_date(from) || !valid_date(to) {
                return Err(FetchError::Other(
                    "dates are written YYYY-MM-DD, for example 2025-05-01".to_string(),
                ));
            }
            vec![(from.clone(), to.clone())]
        }
        (None, None) => default_range(cx.project.area.center_lat, today()),
        _ => {
            return Err(FetchError::Other(
                "give --from and --to together, or neither of them".to_string(),
            ));
        }
    };
    let search_url = format!("{}/search", stac.url.trim_end_matches('/'));
    let mut all: Vec<Candidate> = Vec::new();
    for (from, to) in &ranges {
        (cx.progress)(&format!(
            "Searching {} scenes from {from} to {to}",
            source.id
        ));
        let body = search_body(stac, cx.window, from, to);
        let answer = cx.http.post_json(&search_url, &body, MAX_JSON_BYTES)?;
        if u64::try_from(answer.len()).unwrap_or(u64::MAX) > MAX_JSON_BYTES {
            return Err(too_large(&search_url, MAX_JSON_BYTES));
        }
        for candidate in candidates(source, stac.asset, &answer)? {
            if !all.iter().any(|c| c.id == candidate.id) {
                all.push(candidate);
            }
        }
    }
    all.sort_by(|a, b| {
        a.cloud
            .total_cmp(&b.cloud)
            .then(a.snow.total_cmp(&b.snow))
            .then(a.id.cmp(&b.id))
    });
    Ok(all)
}

/// The scenes to download: the clearest first, until every point of a lattice over the map is inside one, at
/// most [`MAX_SCENES`]. Also the share of the map they cover.
fn choose_scenes<'a>(
    cx: &Context<'_>,
    stac: &StacParams<'_>,
    all: &'a [Candidate],
) -> Result<Vec<&'a Candidate>, FetchError> {
    let source = cx.source;
    // the lattice of the window: the scenes are taken until every point is inside one
    let lattice: Vec<(f64, f64)> = (0..LATTICE)
        .flat_map(|j| (0..LATTICE).map(move |i| (i, j)))
        .map(|(i, j)| {
            let step = |a: f64, b: f64, k: usize| {
                #[expect(clippy::cast_precision_loss, reason = "at most 33")]
                let t = k as f64 / (LATTICE - 1) as f64;
                a + (b - a) * t
            };
            (
                step(cx.window.west, cx.window.east, i),
                step(cx.window.south, cx.window.north, j),
            )
        })
        .collect();
    let mut covered = vec![false; lattice.len()];
    let mut chosen: Vec<&Candidate> = Vec::new();
    let mut rejected = 0_usize;
    for candidate in all {
        if chosen.len() >= MAX_SCENES || covered.iter().all(|&c| c) {
            break;
        }
        if candidate.cloud > MAX_CLOUD_PERCENT {
            continue;
        }
        let host_ok = https_host(&candidate.href)
            .is_some_and(|h| stac.asset_hosts.iter().any(|a| a.eq_ignore_ascii_case(h)));
        if !valid_scene_id(&candidate.id) || !host_ok {
            rejected += 1;
            (cx.progress)(&format!(
                "warning: scene {:?} is not used: its id or its address is not acceptable",
                candidate.id
            ));
            continue;
        }
        let new: Vec<usize> = lattice
            .iter()
            .enumerate()
            .filter(|&(k, &point)| {
                !covered.get(k).copied().unwrap_or(true)
                    && candidate.rings.iter().any(|r| inside(r, point))
            })
            .map(|(k, _)| k)
            .collect();
        if new.is_empty() {
            continue;
        }
        for k in new {
            if let Some(slot) = covered.get_mut(k) {
                *slot = true;
            }
        }
        chosen.push(candidate);
    }
    if chosen.is_empty() {
        let clearest = all.iter().map(|c| c.cloud).fold(f64::INFINITY, f64::min);
        let advice = if clearest > MAX_CLOUD_PERCENT && clearest.is_finite() {
            format!(
                "; the clearest scene has {clearest:.0} % cloud and the limit is {MAX_CLOUD_PERCENT:.0} %: try \
                 other dates with --from and --to, for example the dry season of the area"
            )
        } else {
            String::new()
        };
        return service(
            source,
            format!(
                "no usable scene for this area and these dates ({} found, {rejected} rejected, others too cloudy \
                 or outside the map){advice}",
                all.len()
            ),
        );
    }
    let covered_share = {
        #[expect(clippy::cast_precision_loss, reason = "at most 1089 points")]
        let share = covered.iter().filter(|&&c| c).count() as f64 / covered.len() as f64;
        share
    };
    if covered_share < 1.0 {
        (cx.progress)(&format!(
            "warning: the scenes cover {:.0} % of the map; fetch again with other dates for the rest",
            covered_share * 100.0
        ));
    }
    Ok(chosen)
}

fn fetch_stac(cx: &Context<'_>, stac: &StacParams<'_>) -> Result<Vec<ImageryFile>, FetchError> {
    let all = search_scenes(cx, stac)?;
    let chosen = choose_scenes(cx, stac, &all)?;
    let source = cx.source;
    let mut files = Vec::new();
    for (index, candidate) in chosen.iter().enumerate() {
        (cx.progress)(&format!(
            "Downloading scene {} ({} of {})",
            candidate.id,
            index + 1,
            chosen.len()
        ));
        if let Some(file) = scene_to_file(cx, candidate, stac)? {
            files.push(file);
        }
    }
    if files.is_empty() {
        return service(source, "none of the scenes has any picture of the map");
    }
    Ok(files)
}

/// Downloads one scene, crops it to the window and writes it as a GeoTIFF in EPSG:4326. `None` if the scene
/// does not hold a valid pixel of the map.
fn scene_to_file(
    cx: &Context<'_>,
    candidate: &Candidate,
    _stac: &StacParams<'_>,
) -> Result<Option<ImageryFile>, FetchError> {
    let source = cx.source;
    let dir = cx.imagery_dir();
    std::fs::create_dir_all(&dir).map_err(|e| FetchError::Io {
        path: dir.clone(),
        source: e,
    })?;
    let mut download = tempfile::Builder::new()
        .prefix(".download-")
        .suffix(".tmp")
        .tempfile_in(&dir)
        .map_err(|e| FetchError::Io {
            path: dir.clone(),
            source: e,
        })?;
    {
        let mut limited = Limited::new(download.as_file_mut(), MAX_SCENE_BYTES);
        match cx.http.get(&candidate.href, MAX_SCENE_BYTES, &mut limited) {
            Ok(()) => {}
            Err(_) if limited.exceeded => return Err(too_large(&candidate.href, MAX_SCENE_BYTES)),
            Err(e) => return Err(e),
        }
    }
    let Some(raster) = ImageryRaster::open(download.path(), cx.project, 0.0)? else {
        (cx.progress)(&format!(
            "warning: scene {} does not touch the map",
            candidate.id
        ));
        return Ok(None);
    };
    let sampler = raster.sampler()?;
    let (cols, rows) = grid_of(cx.window, source.native_m, source)?;
    let (dx, dy) = (
        (cx.window.east - cx.window.west) / f64::from(cols),
        (cx.window.north - cx.window.south) / f64::from(rows),
    );
    let mut data = Vec::new();
    for row in 0..rows {
        let lat = cx.window.north - (f64::from(row) + 0.5) * dy;
        for col in 0..cols {
            let lon = cx.window.west + (f64::from(col) + 0.5) * dx;
            let rgb = sampler.rgb_at(lat, lon).map_or([0, 0, 0], |c| {
                let byte = |v: f32| {
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "rounded and clamped to 0..=255"
                    )]
                    let b = v.round().clamp(0.0, 255.0) as u8;
                    b
                };
                let rgb = [byte(c[0]), byte(c[1]), byte(c[2])];
                if rgb == [0, 0, 0] { [1, 1, 1] } else { rgb }
            });
            data.extend_from_slice(&rgb);
        }
    }
    if data.iter().all(|&b| b == 0) {
        (cx.progress)(&format!(
            "warning: scene {} has no valid pixel on the map",
            candidate.id
        ));
        return Ok(None);
    }
    let name = format!("{}-{}.tif", source.id, candidate.id);
    write_rgb(
        &dir.join(&name),
        &RgbMosaic {
            width: cols,
            height: rows,
            data,
        },
        cx.window,
    )?;
    let year = candidate
        .year
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    Ok(Some(cx.listed(
        &name,
        source.attribution.replace("{year}", &year),
    )))
}

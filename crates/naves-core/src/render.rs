// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Top-down rendering of a heightmap, shared by the GUI map view and the MCP server.

use image::{Rgba, RgbaImage};
use rayon::prelude::*;

use crate::checks::{DriveState, PieceCheck};
use crate::features::{Feature, FeatureKind};
use crate::heightmap::Heightmap;
use crate::imagery::{MapRect, TileImagery};
use crate::surface::SurfaceGrid;

/// Largest accepted output width in pixels.
pub const MAX_WIDTH: u32 = 4096;
/// Largest accepted output height in pixels.
const MAX_HEIGHT: u32 = 8192;

/// What the picture shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    /// Shaded relief, sun from the north-west.
    Hillshade,
    /// Green for flat to red for steep ground (red from 45°).
    Slope,
    /// Grey by height, white is the highest point of the map.
    Height,
    /// The ground class of every cell, in the colours of the satellite image.
    Surface,
    /// The merged satellite or aerial imagery; ground it does not cover is drawn checkered.
    Imagery,
}

/// A rectangle of the map and the output width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderRequest {
    /// West edge in map metres.
    pub x0: f64,
    /// South edge in map metres.
    pub y0: f64,
    /// East edge in map metres.
    pub x1: f64,
    /// North edge in map metres.
    pub y1: f64,
    /// Output width in pixels; the height follows from the aspect ratio of the rectangle.
    pub px: u32,
    /// What to draw.
    pub overlay: Overlay,
}

/// Why a request cannot be rendered.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RenderError {
    /// The rectangle is inverted, empty or not finite.
    #[error("the bounds must be finite with x0 < x1 and y0 < y1")]
    Bounds,
    /// The width is zero or too large.
    #[error("the width must be 1..={MAX_WIDTH} pixels, not {0}")]
    Width(u32),
    /// The rectangle is so elongated that the image would be too tall or have no rows.
    #[error("the aspect ratio of the rectangle gives an image of unsupported height")]
    Aspect,
    /// The surface overlay was asked for without surfaces.
    #[error("the surface overlay needs the ground surfaces")]
    NoSurfaces,
    /// The imagery overlay was asked for without imagery.
    #[error("the imagery overlay needs imagery files: list them in sources.imagery")]
    NoImagery,
    /// The imagery could not be drawn.
    #[error("cannot draw the imagery: {0}")]
    Imagery(String),
}

/// Rows of the picture that are painted from the imagery in one piece.
const IMAGERY_BAND_ROWS: u32 = 16;

/// Side of a square of the checkered pattern that marks ground without imagery, in pixels.
const CHECKER_PX: u32 = 8;

/// The imagery of a picture, row by row; `None` where no image covers a pixel.
fn paint_imagery(
    imagery: &TileImagery<'_>,
    req: &RenderRequest,
    (width, height): (u32, u32),
) -> Result<Vec<Option<[u8; 3]>>, RenderError> {
    let step_x = (req.x1 - req.x0) / f64::from(width);
    let step_y = (req.y1 - req.y0) / f64::from(height);
    let bands: Vec<u32> = (0..height).step_by(IMAGERY_BAND_ROWS as usize).collect();
    let painted: Vec<Result<Vec<Option<[u8; 3]>>, RenderError>> = bands
        .par_iter()
        .map(|&first| {
            let rows = IMAGERY_BAND_ROWS.min(height - first);
            imagery
                .composite
                .paint(
                    imagery.project,
                    &MapRect {
                        west: req.x0,
                        north: req.y1 - f64::from(first) * step_y,
                        step_x,
                        step_y,
                        width,
                        height: rows,
                    },
                )
                .map_err(|e| RenderError::Imagery(e.to_string()))
        })
        .collect();
    // what the build makes of it: the colours times the project's brightness
    let brightness = imagery.project.imagery.brightness;
    let bright = |c: u8| -> u8 {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "rounded and clamped to 0..=255"
        )]
        let v = (f64::from(c) * brightness).round().clamp(0.0, 255.0) as u8;
        v
    };
    let mut all = Vec::new();
    for band in painted {
        all.extend(
            band?
                .into_iter()
                .map(|pixel| pixel.map(|[r, g, b]| [bright(r), bright(g), bright(b)])),
        );
    }
    Ok(all)
}

fn narrow(v: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "map coordinates are far inside the f32 range"
    )]
    let v = v as f32;
    v
}

impl RenderRequest {
    /// The output size, or why the request is invalid.
    ///
    /// # Errors
    ///
    /// [`RenderError`] for inverted, empty or non-finite bounds, a width outside `1..=4096` or an
    /// aspect ratio that would give a height outside `1..=8192`.
    pub fn size(&self) -> Result<(u32, u32), RenderError> {
        let finite = [self.x0, self.y0, self.x1, self.y1]
            .iter()
            .all(|v| v.is_finite());
        if !finite || self.x1 <= self.x0 || self.y1 <= self.y0 {
            return Err(RenderError::Bounds);
        }
        if !(1..=MAX_WIDTH).contains(&self.px) {
            return Err(RenderError::Width(self.px));
        }
        let height = (f64::from(self.px) * (self.y1 - self.y0) / (self.x1 - self.x0)).round();
        if !(1.0..=f64::from(MAX_HEIGHT)).contains(&height) {
            return Err(RenderError::Aspect);
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "range-checked to 1..=8192 above"
        )]
        let height = height as u32;
        Ok((self.px, height))
    }
}

fn grey(v: f32) -> [u8; 4] {
    let g = narrow(f64::from(v)).clamp(0.0, 255.0);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=255"
    )]
    let g = g.round() as u8;
    [g, g, g, 255]
}

/// Blends `from` into `to` by `amount` (0..=1) and returns an opaque RGBA pixel.
fn mix(from: [f32; 3], to: [f32; 3], amount: f32) -> [u8; 4] {
    let [r, g, b] = [0, 1, 2].map(|i| {
        let start = from.get(i).copied().unwrap_or(0.0);
        let end = to.get(i).copied().unwrap_or(0.0);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 0..=255"
        )]
        let channel = (start + (end - start) * amount).round().clamp(0.0, 255.0) as u8;
        channel
    });
    [r, g, b, 255]
}

/// Paints an image of the request's rectangle: `pixel(row, col, east, north)` gives the pixel whose centre is at
/// (`east`, `north`) in map metres. Rows are computed in parallel.
fn paint_grid(
    req: &RenderRequest,
    (width, height): (u32, u32),
    pixel: impl Fn(u32, u32, f64, f64) -> [u8; 4] + Sync,
) -> RgbaImage {
    let step_x = (req.x1 - req.x0) / f64::from(width);
    let step_y = (req.y1 - req.y0) / f64::from(height);
    let mut rows: Vec<Vec<[u8; 4]>> = (0..height)
        .into_par_iter()
        .map(|row| {
            let north = req.y1 - (f64::from(row) + 0.5) * step_y;
            (0..width)
                .map(|col| pixel(row, col, req.x0 + (f64::from(col) + 0.5) * step_x, north))
                .collect()
        })
        .collect();
    let mut img = RgbaImage::new(width, height);
    for (y, row) in rows.drain(..).enumerate() {
        let y = u32::try_from(y).unwrap_or(0);
        for (x, px) in row.into_iter().enumerate() {
            img.put_pixel(u32::try_from(x).unwrap_or(0), y, Rgba(px));
        }
    }
    img
}

/// Renders the rectangle of the heightmap, north up.
///
/// `surfaces` is only used by [`Overlay::Surface`] and `imagery` only by [`Overlay::Imagery`].
///
/// # Errors
///
/// [`RenderError`] if the request is invalid, see [`RenderRequest::size`], or if the surface or imagery
/// overlay is asked for without `surfaces` or `imagery`.
pub fn render_topdown(
    hm: &Heightmap,
    req: &RenderRequest,
    surfaces: Option<&SurfaceGrid>,
    imagery: Option<&TileImagery<'_>>,
) -> Result<RgbaImage, RenderError> {
    let (width, height) = req.size()?;
    if req.overlay == Overlay::Surface && surfaces.is_none() {
        return Err(RenderError::NoSurfaces);
    }
    let painted = match (req.overlay, imagery) {
        (Overlay::Imagery, None) => return Err(RenderError::NoImagery),
        (Overlay::Imagery, Some(imagery)) => Some(paint_imagery(imagery, req, (width, height))?),
        _ => None,
    };
    let (low, high) = hm.min_max();
    let span = if high > low { high - low } else { 1.0 };
    Ok(paint_grid(req, (width, height), |row, col, east, north| {
        let ground = hm.sample(east, north);
        match req.overlay {
            Overlay::Imagery => {
                let at = usize::try_from(row)
                    .ok()
                    .zip(usize::try_from(width).ok())
                    .and_then(|(r, w)| r.checked_mul(w)?.checked_add(usize::try_from(col).ok()?));
                let seen = painted
                    .as_ref()
                    .zip(at)
                    .and_then(|(pixels, at)| pixels.get(at).copied().flatten());
                seen.map_or_else(
                    || {
                        if (row / CHECKER_PX + col / CHECKER_PX).is_multiple_of(2) {
                            [96, 96, 96, 255]
                        } else {
                            [128, 128, 128, 255]
                        }
                    },
                    |[red, green, blue]| [red, green, blue, 255],
                )
            }
            Overlay::Surface => {
                let [red, green, blue] = surfaces.map_or([0; 3], |grid| grid.at(east, north).rgb());
                [red, green, blue, 255]
            }
            Overlay::Hillshade => grey(255.0 * ground.hillshade()),
            Overlay::Height => grey(40.0 + 190.0 * (ground.height - low) / span),
            Overlay::Slope => mix(
                [70.0, 140.0, 70.0],
                [220.0, 40.0, 40.0],
                (ground.slope_deg() / 45.0).min(1.0),
            ),
        }
    }))
}

/// Ground steeper than this cannot be walked up, in degrees. **Provisional**: 60° is the maximum slope of the
/// Livonia navigation mesh, an upper bound, so the layer under-reports but never marks walkable ground. The roads plan
/// measures the real limit (its Task 1) and replaces this value.
pub const WALK_LIMIT_DEG: f64 = 60.0;

/// A layer over the base map: an image that is transparent where it draws nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MapLayer {
    /// What the roads did to the terrain: cut and fill, tinted by how much.
    TerrainChanges,
    /// Hatching over ground above [`WALK_LIMIT_DEG`].
    TooSteepToWalk,
    /// The roads at their true width, with a white casing.
    Roads,
    /// Footpaths, dashed.
    Paths,
    /// Whether each piece of road can be driven.
    Drivable,
}

impl MapLayer {
    /// Every layer, in the order they are drawn: bottom first.
    pub const ALL: &'static [MapLayer] = &[
        MapLayer::TerrainChanges,
        MapLayer::TooSteepToWalk,
        MapLayer::Roads,
        MapLayer::Paths,
        MapLayer::Drivable,
    ];

    /// The stable name, for the settings file and the MCP tools.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            MapLayer::TerrainChanges => "terrain_changes",
            MapLayer::TooSteepToWalk => "too_steep_to_walk",
            MapLayer::Roads => "roads",
            MapLayer::Paths => "paths",
            MapLayer::Drivable => "drivable",
        }
    }
}

/// What the layers are drawn from.
#[derive(Debug, Clone, Copy)]
pub struct LayerData<'a> {
    /// The heights of the map, with the roads' earthworks.
    pub heightmap: &'a Heightmap,
    /// The heights before the roads shaped them; `None` where there are no roads.
    pub base: Option<&'a Heightmap>,
    /// The roads and paths in map metres.
    pub features: &'a [Feature],
    /// The measurements of every road piece.
    pub pieces: &'a [PieceCheck],
}

impl<'a> LayerData<'a> {
    /// Data with a heightmap and no roads.
    #[must_use]
    pub fn new(heightmap: &'a Heightmap) -> LayerData<'a> {
        LayerData {
            heightmap,
            base: None,
            features: &[],
            pieces: &[],
        }
    }
}

/// The ink of the hatching, `1d2924`, at 55 %.
const HATCH: [u8; 4] = [0x1d, 0x29, 0x24, 140];
/// Distance of the hatching lines in image pixels.
const HATCH_PX: u32 = 6;
const ROAD_INK: [u8; 4] = [0x1d, 0x29, 0x24, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const PATH_BROWN: [u8; 4] = [0x8c, 0x6a, 0x45, 255];
const FINE: [u8; 4] = [0x2e, 0x6a, 0x4c, 255];
const STEEP: [u8; 4] = [0xd0, 0x8a, 0x1e, 255];
const NOT_DRIVABLE: [u8; 4] = [0xb0, 0x3a, 0x2e, 255];
const WALK_ONLY: [u8; 4] = [0x5a, 0x66, 0x60, 255];
const CUT: [u8; 3] = [0x8c, 0x6a, 0x45];
const FILL: [u8; 3] = [0x3f, 0x6f, 0x9e];
/// A road is never drawn thinner than this, in pixels.
const MIN_ROAD_PX: f64 = 1.5;
/// A path is never drawn thinner than this, in pixels.
const MIN_PATH_PX: f64 = 1.0;
/// Dashes of a path: pixels drawn, pixels left out.
const DASH_PX: (f64, f64) = (4.0, 3.0);
/// Distance of the cross ticks on a road that cannot be driven, in pixels.
const TICK_PX: f64 = 6.0;
/// Changes of the terrain below this many metres are not shown.
const CHANGE_MIN_M: f64 = 0.25;
/// Changes of this many metres are shown at full strength (half opaque).
const CHANGE_FULL_M: f64 = 5.0;

/// Blends `over` onto `below` (straight alpha).
fn blend(below: [u8; 4], over: [u8; 4]) -> [u8; 4] {
    let over_alpha = f32::from(over[3]) / 255.0;
    let below_alpha = f32::from(below[3]) / 255.0;
    let out_alpha = over_alpha + below_alpha * (1.0 - over_alpha);
    if out_alpha <= 0.0 {
        return [0; 4];
    }
    let channel = |index: usize| {
        let on_top = f32::from(over.get(index).copied().unwrap_or(0));
        let underneath = f32::from(below.get(index).copied().unwrap_or(0));
        let mixed =
            (on_top * over_alpha + underneath * below_alpha * (1.0 - over_alpha)) / out_alpha;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 0..=255"
        )]
        let value = mixed.round().clamp(0.0, 255.0) as u8;
        value
    };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=255"
    )]
    let alpha = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
    [channel(0), channel(1), channel(2), alpha]
}

/// `above` drawn over `below`, where `above` is not transparent. Both must have the same size; a part of
/// `above` outside `below` is ignored.
#[must_use]
pub fn composite(below: &RgbaImage, above: &RgbaImage) -> RgbaImage {
    let mut out = below.clone();
    for (x, y, pixel) in above.enumerate_pixels() {
        if pixel[3] == 0 {
            continue;
        }
        if let Some(target) = out.get_pixel_mut_checked(x, y) {
            *target = Rgba(blend(target.0, pixel.0));
        }
    }
    out
}

/// `image` with its transparency multiplied by `opacity` (0 to 1); the colours stay.
#[must_use]
pub fn fade(image: &RgbaImage, opacity: f32) -> RgbaImage {
    let mut out = image.clone();
    for pixel in out.pixels_mut() {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 0..=255"
        )]
        let alpha = (f32::from(pixel[3]) * opacity).round().clamp(0.0, 255.0) as u8;
        pixel[3] = alpha;
    }
    out
}

/// A picture of the request's rectangle that lines are drawn on, in image pixels.
struct Canvas {
    image: RgbaImage,
    x0: f64,
    y1: f64,
    step_x: f64,
    step_y: f64,
}

impl Canvas {
    fn new(req: &RenderRequest, (width, height): (u32, u32)) -> Canvas {
        Canvas {
            image: RgbaImage::new(width, height),
            x0: req.x0,
            y1: req.y1,
            step_x: (req.x1 - req.x0) / f64::from(width),
            step_y: (req.y1 - req.y0) / f64::from(height),
        }
    }

    /// The image position of a map position.
    fn at(&self, map: (f64, f64)) -> (f64, f64) {
        (
            (map.0 - self.x0) / self.step_x,
            (self.y1 - map.1) / self.step_y,
        )
    }

    /// Paints the pixels whose centres lie within `half` pixels of the segment; with `dash` only the parts
    /// that fall on a dash, counted from `along` pixels along the line.
    fn line(
        &mut self,
        from: (f64, f64),
        to: (f64, f64),
        half: f64,
        colour: [u8; 4],
        dash: Option<f64>,
    ) {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let length = dx.hypot(dy);
        let (width, height) = (
            f64::from(self.image.width()),
            f64::from(self.image.height()),
        );
        let lo = |a: f64, b: f64, limit: f64| (a.min(b) - half - 1.0).clamp(0.0, limit);
        let hi = |a: f64, b: f64, limit: f64| (a.max(b) + half + 1.0).clamp(0.0, limit);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "pixel bounds clamped to the image"
        )]
        let (c0, c1, r0, r1) = (
            lo(from.0, to.0, width) as u32,
            hi(from.0, to.0, width) as u32,
            lo(from.1, to.1, height) as u32,
            hi(from.1, to.1, height) as u32,
        );
        for row in r0..r1.min(self.image.height()) {
            for col in c0..c1.min(self.image.width()) {
                let (px, py) = (f64::from(col) + 0.5, f64::from(row) + 0.5);
                let along = if length > 0.0 {
                    (((px - from.0) * dx + (py - from.1) * dy) / (length * length)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                if (px - from.0 - along * dx).hypot(py - from.1 - along * dy) > half {
                    continue;
                }
                if let Some(start) = dash {
                    let s = start + along * length;
                    if s % (DASH_PX.0 + DASH_PX.1) >= DASH_PX.0 {
                        continue;
                    }
                }
                self.image.put_pixel(col, row, Rgba(colour));
            }
        }
    }
}

fn roads_layer(data: &LayerData<'_>, req: &RenderRequest, size: (u32, u32)) -> RgbaImage {
    let mut canvas = Canvas::new(req, size);
    let roads: Vec<(&Feature, f64)> = data
        .features
        .iter()
        .filter_map(|f| match f.kind {
            FeatureKind::Road {
                family,
                tunnel: false,
                ..
            } => Some((f, (family.width_m() / canvas.step_x).max(MIN_ROAD_PX) / 2.0)),
            _ => None,
        })
        .collect();
    // all the casings first, so that a casing never cuts into another road
    for (casing, colour) in [(1.0, WHITE), (0.0, ROAD_INK)] {
        for (feature, half) in &roads {
            for pair in feature.points.windows(2) {
                if let [a, b] = pair {
                    let (a, b) = (canvas.at(*a), canvas.at(*b));
                    canvas.line(a, b, half + casing, colour, None);
                }
            }
        }
    }
    canvas.image
}

fn paths_layer(data: &LayerData<'_>, req: &RenderRequest, size: (u32, u32)) -> RgbaImage {
    let mut canvas = Canvas::new(req, size);
    for feature in data.features {
        let FeatureKind::Path { width_m } = feature.kind else {
            continue;
        };
        let half = (width_m / canvas.step_x).max(MIN_PATH_PX) / 2.0;
        let mut along = 0.0;
        for pair in feature.points.windows(2) {
            if let [a, b] = pair {
                let (a, b) = (canvas.at(*a), canvas.at(*b));
                canvas.line(a, b, half, PATH_BROWN, Some(along));
                along += (b.0 - a.0).hypot(b.1 - a.1);
            }
        }
    }
    canvas.image
}

fn drivable_layer(data: &LayerData<'_>, req: &RenderRequest, size: (u32, u32)) -> RgbaImage {
    let mut canvas = Canvas::new(req, size);
    for piece in data.pieces {
        let (colour, half, ticks) = match piece.state {
            DriveState::Fine => (FINE, 1.0, false),
            DriveState::Steep => (STEEP, 1.0, false),
            DriveState::WalkOnly => (WALK_ONLY, 1.0, false),
            DriveState::NotDrivable | DriveState::TooSteepToWalk => (NOT_DRIVABLE, 2.0, true),
        };
        let (a, b) = (canvas.at(piece.start), canvas.at(piece.end));
        canvas.line(a, b, half, colour, None);
        if ticks {
            let length = (b.0 - a.0).hypot(b.1 - a.1);
            if length > 0.0 {
                let (dir, across) = (
                    ((b.0 - a.0) / length, (b.1 - a.1) / length),
                    (-(b.1 - a.1) / length, (b.0 - a.0) / length),
                );
                let mut at = TICK_PX / 2.0;
                while at < length {
                    let centre = (a.0 + dir.0 * at, a.1 + dir.1 * at);
                    let reach = 3.0;
                    canvas.line(
                        (centre.0 - across.0 * reach, centre.1 - across.1 * reach),
                        (centre.0 + across.0 * reach, centre.1 + across.1 * reach),
                        0.5,
                        colour,
                        None,
                    );
                    at += TICK_PX;
                }
            }
        }
    }
    canvas.image
}

fn terrain_changes_layer(data: &LayerData<'_>, req: &RenderRequest, size: (u32, u32)) -> RgbaImage {
    let Some(base) = data.base else {
        return RgbaImage::new(size.0, size.1);
    };
    paint_grid(req, size, |_, _, east, north| {
        let change = f64::from(data.heightmap.sample(east, north).height)
            - f64::from(base.sample(east, north).height);
        if change.abs() < CHANGE_MIN_M {
            return [0; 4];
        }
        let strength = (change.abs() / CHANGE_FULL_M).min(1.0);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "at most 127"
        )]
        let alpha = (127.0 * strength).round().max(1.0) as u8;
        let [red, green, blue] = if change < 0.0 { CUT } else { FILL };
        [red, green, blue, alpha]
    })
}

/// Renders one layer for the rectangle of `req` (its `overlay` is not used). The picture is transparent where
/// the layer draws nothing. The request can be the whole map, so that a 3D view can drape the same image.
///
/// # Errors
///
/// [`RenderError`] if the request is invalid, see [`RenderRequest::size`].
pub fn render_layer(
    layer: MapLayer,
    data: &LayerData<'_>,
    req: &RenderRequest,
) -> Result<RgbaImage, RenderError> {
    let size = req.size()?;
    Ok(match layer {
        MapLayer::TooSteepToWalk => paint_grid(req, size, |row, col, east, north| {
            let steep = f64::from(data.heightmap.sample(east, north).slope_deg()) > WALK_LIMIT_DEG;
            // diagonal lines, two pixels wide, every HATCH_PX pixels
            if steep && (row + col) % HATCH_PX < 2 {
                HATCH
            } else {
                [0; 4]
            }
        }),
        MapLayer::TerrainChanges => terrain_changes_layer(data, req, size),
        MapLayer::Roads => roads_layer(data, req, size),
        MapLayer::Paths => paths_layer(data, req, size),
        MapLayer::Drivable => drivable_layer(data, req, size),
    })
}

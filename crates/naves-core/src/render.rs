// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Top-down rendering of a heightmap, shared by the GUI map view and the MCP server.

use image::{Rgba, RgbaImage};
use rayon::prelude::*;

use crate::heightmap::Heightmap;
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

/// Renders the rectangle of the heightmap, north up.
///
/// `surfaces` is only used by [`Overlay::Surface`].
///
/// # Errors
///
/// [`RenderError`] if the request is invalid, see [`RenderRequest::size`], or if the surface overlay is
/// asked for without `surfaces`.
pub fn render_topdown(
    hm: &Heightmap,
    req: &RenderRequest,
    surfaces: Option<&SurfaceGrid>,
) -> Result<RgbaImage, RenderError> {
    let (width, height) = req.size()?;
    if req.overlay == Overlay::Surface && surfaces.is_none() {
        return Err(RenderError::NoSurfaces);
    }
    let (low, high) = hm.min_max();
    let span = if high > low { high - low } else { 1.0 };
    let step_x = (req.x1 - req.x0) / f64::from(width);
    let step_y = (req.y1 - req.y0) / f64::from(height);
    let mut rows: Vec<Vec<[u8; 4]>> = (0..height)
        .into_par_iter()
        .map(|row| {
            let north = req.y1 - (f64::from(row) + 0.5) * step_y;
            (0..width)
                .map(|col| {
                    let east = req.x0 + (f64::from(col) + 0.5) * step_x;
                    let ground = hm.sample(east, north);
                    match req.overlay {
                        Overlay::Surface => {
                            let [red, green, blue] =
                                surfaces.map_or([0; 3], |grid| grid.at(east, north).rgb());
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
                })
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
    Ok(img)
}

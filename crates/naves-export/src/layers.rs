// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Procedural terrain layers: satellite image, surface mask and tile material of every terrain tile.
//!
//! Geometry (tile size, overlap, texture-coordinate transform) follows Livonia, see
//! `docs/formats/terrain-layers.md`. Tiles are numbered from the north-west corner; the land grid and the
//! material index are south row first like the heightmap.

use std::io::Cursor;

use image::{ImageFormat, RgbImage, RgbaImage};
use naves_core::heightmap::Heightmap;
use rayon::prelude::*;

use crate::rvmat::tile_material;

/// Land cells per tile side.
const CELLS_PER_TILE: u32 = 32;
/// Image pixels per land cell.
const PX_PER_CELL: f64 = 15.0;
/// Tile image side in pixels.
const TILE_PX: u32 = 512;
/// Overlap on every side of a tile image in pixels.
const OVERLAP_PX: u32 = 16;
/// Slope in degrees above which ground counts as rock.
const ROCK_SLOPE_DEG: f32 = 35.0;

/// The tile grid of a map.
#[derive(Debug, Clone, PartialEq)]
pub struct TileGrid {
    /// Tiles from west to east.
    pub tiles_x: u32,
    /// Tiles from north to south.
    pub tiles_y: u32,
    /// Image side in pixels.
    pub tile_px: u32,
    /// Overlap on every side in pixels.
    pub overlap_px: u32,
    /// Metres per image pixel.
    pub px_m: f64,
}

/// The tile grid for a square map of `map_size_m` metres with `land_size` × `land_size` land cells.
///
/// A tile is 32 × 32 land cells with 15 image pixels per cell, so `px_m = cell / 15`.
#[must_use]
pub fn tile_grid(map_size_m: f64, land_size: u32) -> TileGrid {
    let tiles = (land_size / CELLS_PER_TILE).max(1);
    let cell = map_size_m / f64::from(land_size.max(1));
    TileGrid {
        tiles_x: tiles,
        tiles_y: tiles,
        tile_px: TILE_PX,
        overlap_px: OVERLAP_PX,
        px_m: cell / PX_PER_CELL,
    }
}

fn narrow(v: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "texture coordinates and sizes are far inside the f32 range"
    )]
    let v = v as f32;
    v
}

impl TileGrid {
    /// Side of the area a tile covers, in metres (the image minus its overlap).
    fn tile_m(&self) -> f64 {
        f64::from(self.tile_px - 2 * self.overlap_px) * self.px_m
    }

    /// Texture-coordinate scale: image units per metre.
    #[must_use]
    pub fn aside(&self) -> f32 {
        narrow(1.0 / (f64::from(self.tile_px) * self.px_m))
    }

    /// The `pos` of the texture-coordinate transform of tile (`tx`, `ty`); `ty` counts from the north.
    #[must_use]
    pub fn uv_pos(&self, tx: u32, ty: u32) -> (f32, f32) {
        let overlap = narrow(f64::from(self.overlap_px) / f64::from(self.tile_px));
        let step = self.aside() * narrow(self.tile_m());
        let x = overlap - narrow(f64::from(tx)) * step;
        let y = overlap + narrow(f64::from(self.tiles_y.saturating_sub(ty))) * step;
        (x, y)
    }
}

/// A surface layer: vanilla terrain textures and the colour it has on the satellite image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Surface {
    /// Short name.
    pub name: &'static str,
    /// Normal/height texture path (`_nopx`).
    pub nopx: &'static str,
    /// Colour texture path (`_ca`).
    pub ca: &'static str,
    /// Colour on the satellite image.
    pub sat_rgb: [u8; 3],
}

/// The surfaces of milestone 1, in layer-slot order: slot 0 is grass (mask channel R), slot 1 rock (G).
/// There is no snow texture among the vanilla terrain surfaces.
pub const SURFACES: &[Surface] = &[
    Surface {
        name: "grass",
        nopx: "dz\\surfaces_bliss\\data\\terrain\\en_grass1_nopx.paa",
        ca: "dz\\surfaces_bliss\\data\\terrain\\en_grass1_ca.paa",
        sat_rgb: [0x4a, 0x7a, 0x3a],
    },
    Surface {
        name: "rock",
        nopx: "dz\\surfaces_bliss\\data\\terrain\\en_stones_nopx.paa",
        ca: "dz\\surfaces_bliss\\data\\terrain\\en_stones_ca.paa",
        sat_rgb: [0x8a, 0x87, 0x80],
    },
];

fn vertex(hm: &Heightmap, col: i64, row: i64) -> f32 {
    let last = i64::from(hm.size) - 1;
    let c = u32::try_from(col.clamp(0, last)).unwrap_or(0);
    let r = u32::try_from(row.clamp(0, last)).unwrap_or(0);
    hm.at(c, r)
}

/// Height and gradient (dz/dx east, dz/dy north) at the map position (`x`, `y`) in metres.
fn height_and_gradient(hm: &Heightmap, x: f64, y: f64) -> (f32, f32, f32) {
    let cell = f64::from(hm.cell_m).max(f64::MIN_POSITIVE);
    let max = f64::from(hm.size.saturating_sub(1));
    let gx = (x / cell).clamp(0.0, max);
    let gy = (y / cell).clamp(0.0, max);
    let (c0, r0) = (gx.floor(), gy.floor());
    let (fx, fy) = (narrow(gx - c0), narrow(gy - r0));
    #[expect(
        clippy::cast_possible_truncation,
        reason = "c0 and r0 are in 0..=8191 after the clamp"
    )]
    let (c0, r0) = (c0 as i64, r0 as i64);
    let h00 = vertex(hm, c0, r0);
    let h10 = vertex(hm, c0 + 1, r0);
    let h01 = vertex(hm, c0, r0 + 1);
    let h11 = vertex(hm, c0 + 1, r0 + 1);
    let south = h00 + (h10 - h00) * fx;
    let north = h01 + (h11 - h01) * fx;
    let height = south + (north - south) * fy;
    // exact gradient of the bilinear patch
    let dx = ((h10 - h00) * (1.0 - fy) + (h11 - h01) * fy) / hm.cell_m;
    let dy = ((h01 - h00) * (1.0 - fx) + (h11 - h10) * fx) / hm.cell_m;
    (height, dx, dy)
}

fn slope_deg(dx: f32, dy: f32) -> f32 {
    dx.hypot(dy).atan().to_degrees()
}

/// Hillshade in 0..=1 for the gradient, sun at azimuth 315° and altitude 45°.
fn hillshade(dx: f32, dy: f32) -> f32 {
    let slope = dx.hypot(dy).atan();
    let aspect = dy.atan2(-dx);
    let zenith = 45.0_f32.to_radians();
    let azimuth = 135.0_f32.to_radians(); // 315° compass in mathematical angles
    (zenith.cos() * slope.cos() + zenith.sin() * slope.sin() * (azimuth - aspect).cos())
        .clamp(0.0, 1.0)
}

/// Index into [`SURFACES`] for ground with the gradient (`dx`, `dy`).
fn surface_index(dx: f32, dy: f32) -> u8 {
    u8::from(slope_deg(dx, dy) > ROCK_SLOPE_DEG)
}

/// The surface of every heightmap vertex: an index into [`SURFACES`] (rock on slopes above 35°, else grass).
#[must_use]
pub fn classify(hm: &Heightmap) -> Vec<u8> {
    let cell = f64::from(hm.cell_m);
    let mut out = Vec::with_capacity(hm.h.len());
    for row in 0..i64::from(hm.size) {
        for col in 0..i64::from(hm.size) {
            #[expect(clippy::cast_precision_loss, reason = "vertex indices are below 2^13")]
            let (x, y) = (col as f64 * cell, row as f64 * cell);
            let (_, dx, dy) = height_and_gradient(hm, x, y);
            // The bilinear gradient inside a cell is one-sided at a vertex; average both sides.
            let (_, ex, ey) = height_and_gradient(hm, x - cell * 0.5, y - cell * 0.5);
            out.push(surface_index(f32::midpoint(dx, ex), f32::midpoint(dy, ey)));
        }
    }
    out
}

/// Everything the terrain layers produce.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerSet {
    /// In-game paths of the tile materials; entry 0 is the empty string (no material), as in the 8WVR.
    pub materials: Vec<String>,
    /// `land_size * land_size` indices into `materials`, south row first.
    pub material_index: Vec<u16>,
    /// Files relative to the world folder, with `/` separators: PNG images and tile materials.
    pub files: Vec<(String, Vec<u8>)>,
}

fn encode_png(img: &image::DynamicImage) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    // Writing into memory cannot fail; a failure would leave the buffer empty and fail the layer tests.
    img.write_to(&mut buf, ImageFormat::Png).ok();
    buf.into_inner()
}

struct TileImages {
    satellite: Vec<u8>,
    mask: Vec<u8>,
}

fn render_tile(hm: &Heightmap, grid: &TileGrid, map_size_m: f64, tx: u32, ty: u32) -> TileImages {
    let size = grid.tile_px;
    let mut sat = RgbImage::new(size, size);
    let mut mask = RgbaImage::new(size, size);
    let tile_m = grid.tile_m();
    let west = f64::from(tx) * tile_m;
    let north = map_size_m - f64::from(ty) * tile_m;
    let overlap = f64::from(grid.overlap_px);
    for py in 0..size {
        let y = north - (f64::from(py) - overlap + 0.5) * grid.px_m;
        for px in 0..size {
            let x = west + (f64::from(px) - overlap + 0.5) * grid.px_m;
            let (_, dx, dy) = height_and_gradient(hm, x, y);
            let surface = surface_index(dx, dy);
            let shade = 0.35 + 0.65 * hillshade(dx, dy);
            let rgb = SURFACES
                .get(usize::from(surface))
                .map_or([0; 3], |s| s.sat_rgb);
            let lit = |c: u8| {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "the product is rounded and within 0..=255"
                )]
                let v = (f32::from(c) * shade).round().clamp(0.0, 255.0) as u8;
                v
            };
            sat.put_pixel(px, py, image::Rgb(rgb.map(lit)));
            let (r, g) = if surface == 0 { (255, 0) } else { (0, 255) };
            mask.put_pixel(px, py, image::Rgba([r, g, 0, 0]));
        }
    }
    TileImages {
        satellite: encode_png(&image::DynamicImage::ImageRgb8(sat)),
        mask: encode_png(&image::DynamicImage::ImageRgba8(mask)),
    }
}

/// Builds satellite images, masks and tile materials for the whole map.
///
/// `land_size` is the material grid size (a multiple of 32); `prefix` is the world's in-game path, for
/// example `naves\worlds\berchtesgaden`.
#[must_use]
pub fn build_layers(hm: &Heightmap, land_size: u32, prefix: &str) -> LayerSet {
    let map_size_m = f64::from(hm.size) * f64::from(hm.cell_m);
    let grid = tile_grid(map_size_m, land_size);
    let coords: Vec<(u32, u32)> = (0..grid.tiles_y)
        .flat_map(|ty| (0..grid.tiles_x).map(move |tx| (tx, ty)))
        .collect();
    let rendered: Vec<TileImages> = coords
        .par_iter()
        .map(|&(tx, ty)| render_tile(hm, &grid, map_size_m, tx, ty))
        .collect();

    let mut materials = vec![String::new()];
    let mut files = Vec::with_capacity(coords.len() * 3);
    for (&(tx, ty), images) in coords.iter().zip(rendered) {
        materials.push(format!("{prefix}\\data\\layers\\p_{tx:03}-{ty:03}.rvmat"));
        files.push((
            format!("data/layers/s_{tx:03}_{ty:03}_lco.png"),
            images.satellite,
        ));
        files.push((
            format!("data/layers/m_{tx:03}_{ty:03}_lca.png"),
            images.mask,
        ));
        files.push((
            format!("data/layers/p_{tx:03}-{ty:03}.rvmat"),
            tile_material(&grid, tx, ty, prefix).into_bytes(),
        ));
    }

    let land = usize::try_from(land_size).unwrap_or(0);
    let cells = usize::try_from(CELLS_PER_TILE).unwrap_or(32);
    let tiles_x = usize::try_from(grid.tiles_x).unwrap_or(1);
    let tiles_y = usize::try_from(grid.tiles_y).unwrap_or(1);
    let mut material_index = Vec::with_capacity(land * land);
    for row in 0..land {
        let tile_row = (tiles_y - 1).saturating_sub(row / cells).min(tiles_y - 1);
        for col in 0..land {
            let tile_col = (col / cells).min(tiles_x - 1);
            let index = 1 + tile_row * tiles_x + tile_col;
            material_index.push(u16::try_from(index).unwrap_or(0));
        }
    }
    LayerSet {
        materials,
        material_index,
        files,
    }
}

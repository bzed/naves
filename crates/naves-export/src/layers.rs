// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Procedural terrain layers: satellite image, surface mask and tile material of every terrain tile.
//!
//! Geometry (tile size, overlap, texture-coordinate transform) follows Livonia, see
//! `docs/formats/terrain-layers.md`. Tiles are numbered from the north-west corner; the land grid and the
//! material index are south row first like the heightmap.

use std::io::Cursor;

use image::{ImageFormat, RgbImage, RgbaImage};
use naves_core::heightmap::{Heightmap, Sample};
use rayon::prelude::*;

use crate::rvmat::tile_material;

/// Land cells per tile side, as on Livonia. The game client sizes its land segments from this (the
/// land cells that share one satellite texture); with bigger tiles it overflows a fixed 128-height buffer
/// and crashes.
const CELLS_PER_TILE: u32 = 8;
/// Image pixels per land cell (Livonia: 50 m cells, 0.83 m per pixel).
const PX_PER_CELL: f64 = 60.0;
/// Tile image side in pixels.
const TILE_PX: u32 = 512;
/// Overlap on every side of a tile image in pixels.
const OVERLAP_PX: u32 = 16;
/// Slope in degrees above which ground counts as rock.
const ROCK_SLOPE_DEG: f32 = 35.0;

/// Real altitude in metres above which bare ground takes over from grass. Measured with ESA `WorldCover` around
/// Berchtesgaden: 27 % of the ground is bare at 1900 to 2100 m, 58 % at 2100 to 2300 m, 87 % at 2300 to 2500 m.
const GRASS_LIMIT_REAL_M: f64 = 2100.0;

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
/// A tile is 8 × 8 land cells with 60 image pixels per cell, so `px_m = cell / 60`.
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

/// The surfaces of milestone 1, in layer-slot order: slot 0 is grass (black in the mask), slot 1 rock (red).
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

/// The grass limit in map metres for a project's `scale.height`.
#[must_use]
pub fn grass_limit_m(height_scale: f64) -> f32 {
    narrow(GRASS_LIMIT_REAL_M * height_scale)
}

/// Index into [`SURFACES`] for ground with this slope and height.
fn surface_index(s: &Sample, grass_limit: f32) -> u8 {
    u8::from(s.slope_deg() > ROCK_SLOPE_DEG || s.height > grass_limit)
}

/// The surface of every heightmap vertex: an index into [`SURFACES`] (rock on slopes above 35° and above
/// `grass_limit` metres of map height, else grass).
#[must_use]
pub fn classify(hm: &Heightmap, grass_limit: f32) -> Vec<u8> {
    let cell = f64::from(hm.cell_m);
    let mut out = Vec::with_capacity(hm.h.len());
    for row in 0..i64::from(hm.size) {
        for col in 0..i64::from(hm.size) {
            #[expect(clippy::cast_precision_loss, reason = "vertex indices are below 2^13")]
            let (x, y) = (col as f64 * cell, row as f64 * cell);
            let here = hm.sample(x, y);
            // The bilinear gradient inside a cell is one-sided at a vertex; average both sides.
            let there = hm.sample(x - cell * 0.5, y - cell * 0.5);
            out.push(surface_index(
                &Sample {
                    height: here.height,
                    dz_dx: f32::midpoint(here.dz_dx, there.dz_dx),
                    dz_dy: f32::midpoint(here.dz_dy, there.dz_dy),
                },
                grass_limit,
            ));
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

fn render_tile(
    hm: &Heightmap,
    grid: &TileGrid,
    map_size_m: f64,
    (tx, ty): (u32, u32),
    grass_limit: f32,
) -> TileImages {
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
            let here = hm.sample(x, y);
            let surface = surface_index(&here, grass_limit);
            let shade = 0.35 + 0.65 * here.hillshade();
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
            // the mask selects the layer slot: black, red, green, blue at full alpha are slots 0 to 3
            let r = if surface == 0 { 0 } else { 255 };
            mask.put_pixel(px, py, image::Rgba([r, 0, 0, 255]));
        }
    }
    TileImages {
        satellite: encode_png(&image::DynamicImage::ImageRgb8(sat)),
        mask: encode_png(&image::DynamicImage::ImageRgba8(mask)),
    }
}

/// Builds satellite images, masks and tile materials for the whole map.
///
/// `land_size` is the material grid size (a multiple of 8); `prefix` is the world's in-game path, for
/// example `naves\worlds\berchtesgaden`; `grass_limit` comes from [`grass_limit_m`].
#[must_use]
pub fn build_layers(hm: &Heightmap, land_size: u32, prefix: &str, grass_limit: f32) -> LayerSet {
    let map_size_m = f64::from(hm.size) * f64::from(hm.cell_m);
    let grid = tile_grid(map_size_m, land_size);
    let coords: Vec<(u32, u32)> = (0..grid.tiles_y)
        .flat_map(|ty| (0..grid.tiles_x).map(move |tx| (tx, ty)))
        .collect();
    let rendered: Vec<TileImages> = coords
        .par_iter()
        .map(|&tile| render_tile(hm, &grid, map_size_m, tile, grass_limit))
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
    let cells = usize::try_from(CELLS_PER_TILE).unwrap_or(8);
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

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Terrain layers: tile geometry, surface classification, satellite/mask images and tile materials.

#![expect(
    clippy::panic,
    clippy::cast_precision_loss,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::heightmap::Heightmap;
use naves_export::layers::{SURFACES, build_layers, classify, tile_grid};

const PREFIX: &str = "naves\\worlds\\t";

/// A flat heightmap of `size` vertices, 4 m apart.
fn flat(size: u32, height: f32) -> Heightmap {
    Heightmap {
        size,
        cell_m: 4.0,
        h: vec![height; (size * size) as usize],
    }
}

fn file<'a>(files: &'a [(String, Vec<u8>)], name: &str) -> &'a [u8] {
    &files
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no file {name}"))
        .1
}

#[test]
fn livonia_tile_geometry() {
    // Values observed in Livonia's tile materials and world (docs/formats/terrain-layers.md): 256 land
    // cells of 50 m, 8 cells per tile, 60 image pixels per cell.
    let g = tile_grid(12800.0, 256);
    assert_eq!((g.tiles_x, g.tiles_y), (32, 32));
    assert_eq!((g.tile_px, g.overlap_px), (512, 16));
    assert!((g.px_m - 400.0 / 480.0).abs() < 1e-9);
    // Livonia's f32 values are one ulp off the correctly rounded ones (an f32 near 30 resolves 2e-6).
    let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
    assert!(close(g.aside(), 0.002_343_749_9));
    let (x00, y00) = g.uv_pos(0, 0);
    assert!(close(x00, 0.03125) && close(y00, 30.031_248));
    assert!(close(g.uv_pos(1, 0).0, -0.906_249_94));
    assert!(close(g.uv_pos(0, 1).1, 29.093_748));
}

#[test]
fn grid_follows_the_land_size() {
    let g = tile_grid(1600.0, 64);
    assert_eq!((g.tiles_x, g.tiles_y), (8, 8));
    // 25 m cells, 60 px per cell
    assert!((g.px_m - 25.0 / 60.0).abs() < 1e-9);
}

#[test]
fn every_land_cell_has_material() {
    let set = build_layers(&flat(256, 100.0), 32, PREFIX);
    assert_eq!(set.material_index.len(), 32 * 32);
    assert_eq!(set.materials[0], "");
    assert_eq!(set.materials.len(), 1 + 16);
    assert!(
        set.material_index
            .iter()
            .all(|&i| (1..set.materials.len()).contains(&usize::from(i)))
    );
}

#[test]
fn tiles_are_numbered_from_the_north() {
    let set = build_layers(&flat(256, 100.0), 32, PREFIX);
    // land rows are south first: row 0 is the southern tile row, which is tile row 3 (from the north)
    let south_west = set.material_index[0];
    let north_west = set.material_index[31 * 32];
    assert_eq!(
        set.materials[usize::from(south_west)],
        format!("{PREFIX}\\data\\layers\\p_000-003.rvmat")
    );
    assert_eq!(
        set.materials[usize::from(north_west)],
        format!("{PREFIX}\\data\\layers\\p_000-000.rvmat")
    );
    // all cells of one tile (8 x 8 land cells) share one material
    assert_eq!(set.material_index[7], south_west);
    assert_ne!(set.material_index[8], south_west);
}

#[test]
fn steep_is_rock() {
    // a 60 degree wall: height rises by tan(60) * 4 m per vertex along x from column 128 on
    let mut hm = flat(256, 0.0);
    for row in 0..256usize {
        for col in 128..256usize {
            hm.h[row * 256 + col] = ((col - 128) as f32) * 4.0 * 3.0_f32.sqrt();
        }
    }
    let surf = classify(&hm);
    let grass = SURFACES.iter().position(|s| s.name == "grass").unwrap();
    let rock = SURFACES.iter().position(|s| s.name == "rock").unwrap();
    assert_eq!(usize::from(surf[100 * 256 + 20]), grass, "flat ground");
    assert_eq!(usize::from(surf[100 * 256 + 200]), rock, "on the wall");
}

#[test]
fn files_cover_every_tile() {
    let set = build_layers(&flat(256, 100.0), 32, PREFIX);
    for (tx, ty) in [(0, 0), (3, 0), (0, 3), (3, 3)] {
        for name in [
            format!("data/layers/s_{tx:03}_{ty:03}_lco.png"),
            format!("data/layers/m_{tx:03}_{ty:03}_lca.png"),
            format!("data/layers/p_{tx:03}-{ty:03}.rvmat"),
        ] {
            assert!(set.files.iter().any(|(n, _)| *n == name), "missing {name}");
        }
    }
    assert_eq!(set.files.len(), 3 * 16);
}

#[test]
fn images_are_512_square_pngs() {
    let set = build_layers(&flat(256, 100.0), 64, PREFIX);
    for name in [
        "data/layers/s_000_000_lco.png",
        "data/layers/m_000_000_lca.png",
    ] {
        let img =
            image::load_from_memory_with_format(file(&set.files, name), image::ImageFormat::Png)
                .unwrap();
        assert_eq!((img.width(), img.height()), (512, 512), "{name}");
    }
}

#[test]
fn flat_ground_is_grass_in_the_mask() {
    let set = build_layers(&flat(256, 100.0), 64, PREFIX);
    let mask = image::load_from_memory(file(&set.files, "data/layers/m_000_000_lca.png"))
        .unwrap()
        .to_rgba8();
    let px = mask.get_pixel(256, 256);
    // Bohemia's sample: a tile with one layer is all black, red appears with the second layer, then green and blue
    assert_eq!(px.0, [0, 0, 0, 255], "grass is slot 0: black");
}

#[test]
fn rock_is_red_in_the_mask() {
    let mut hm = flat(256, 0.0);
    for row in 0..256usize {
        for col in 0..256usize {
            hm.h[row * 256 + col] = (col as f32) * 4.0 * 3.0_f32.sqrt();
        }
    }
    let set = build_layers(&hm, 32, PREFIX);
    let mask = image::load_from_memory(file(&set.files, "data/layers/m_000_000_lca.png"))
        .unwrap()
        .to_rgba8();
    assert_eq!(
        mask.get_pixel(256, 256).0,
        [255, 0, 0, 255],
        "rock is slot 1: red"
    );
}

#[test]
fn rvmat_shape() {
    let set = build_layers(&flat(256, 100.0), 64, PREFIX);
    let text = String::from_utf8(file(&set.files, "data/layers/p_000-000.rvmat").to_vec()).unwrap();
    assert!(text.contains("PixelShaderID = \"TerrainX\";"));
    assert!(text.contains(&format!(
        "texture = \"{PREFIX}\\data\\layers\\s_000_000_lco.paa\";"
    )));
    assert!(text.contains(&format!(
        "texture = \"{PREFIX}\\data\\layers\\m_000_000_lca.paa\";"
    )));
    assert!(text.contains("en_grass1_nopx.paa") && text.contains("en_stones_ca.paa"));
    // the flat test heightmap is 256 vertices of 4 m: a 1024 m map
    let aside = tile_grid(1024.0, 64).aside();
    assert!(
        text.contains(&format!("aside[] = {{{aside}, 0, 0}};")),
        "{text}"
    );
}

#[test]
fn output_is_deterministic() {
    let a = build_layers(&flat(256, 100.0), 64, PREFIX);
    let b = build_layers(&flat(256, 100.0), 64, PREFIX);
    assert_eq!(a.files, b.files);
    assert_eq!(a.material_index, b.material_index);
}

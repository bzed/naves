// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Terrain layers: tile geometry, surface classification, satellite/mask images and tile materials.

#![expect(
    clippy::unwrap_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::heightmap::{Heightmap, MapGeo};
use naves_core::imagery::{ImageryRaster, TileImagery, merge};
use naves_core::project::Project;
use naves_core::surface::{SurfaceClass, SurfaceGrid, procedural_grid};
use naves_export::layers::{SURFACES, build_layers, tile_grid};

const PREFIX: &str = "naves\\worlds\\t";
/// Grass limit in map metres for these tests.
const LIMIT: f32 = 1000.0;

/// A flat heightmap of `size` vertices, 4 m apart.
fn flat(size: u32, height: f32) -> Heightmap {
    Heightmap {
        size,
        cell_m: 4.0,
        h: vec![height; (size * size) as usize],
    }
}

/// The layers of `hm` with the procedural ground.
fn layers(hm: &Heightmap, land_size: u32) -> naves_export::layers::LayerSet {
    build_layers(hm, land_size, PREFIX, &procedural_grid(hm, LIMIT), None).unwrap()
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
    let set = layers(&flat(256, 100.0), 32);
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
    let set = layers(&flat(256, 100.0), 32);
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
fn files_cover_every_tile() {
    let set = layers(&flat(256, 100.0), 32);
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
    let set = layers(&flat(256, 100.0), 64);
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
    let set = layers(&flat(256, 100.0), 64);
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
    let set = layers(&hm, 32);
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
    let set = layers(&flat(256, 100.0), 64);
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
    let a = layers(&flat(256, 100.0), 64);
    let b = layers(&flat(256, 100.0), 64);
    assert_eq!(a.files, b.files);
    assert_eq!(a.material_index, b.material_index);
}

#[test]
fn nothing_grows_above_the_grass_limit() {
    for (height, red) in [(900.0, 0), (1100.0, 255)] {
        let set = layers(&flat(256, height), 32);
        let mask = image::load_from_memory(file(&set.files, "data/layers/m_000_000_lca.png"))
            .unwrap()
            .to_rgba8();
        assert_eq!(
            mask.get_pixel(256, 256).0,
            [red, 0, 0, 255],
            "mask at {height} m"
        );
    }
}

/// A grid with one class per quadrant of the map.
fn quadrants() -> SurfaceGrid {
    SurfaceGrid::from_fn(64, 16.0, |col, row| match (col >= 32, row >= 32) {
        (false, false) => SurfaceClass::Grass,
        (true, false) => SurfaceClass::Rock,
        (false, true) => SurfaceClass::Forest,
        (true, true) => SurfaceClass::Soil,
    })
}

#[test]
fn every_class_has_its_mask_colour() {
    // a 1024 m map of 4 tiles; tile (tx, ty) counts from the north-west
    let set = build_layers(&flat(256, 100.0), 16, PREFIX, &quadrants(), None).unwrap();
    for (tile, expected) in [
        ("000_000", [0, 255, 0, 255]), // north-west: forest
        ("001_000", [0, 0, 255, 255]), // north-east: soil
        ("000_001", [0, 0, 0, 255]),   // south-west: grass
        ("001_001", [255, 0, 0, 255]), // south-east: rock
    ] {
        let mask =
            image::load_from_memory(file(&set.files, &format!("data/layers/m_{tile}_lca.png")))
                .unwrap()
                .to_rgba8();
        assert_eq!(mask.get_pixel(256, 256).0, expected, "tile {tile}");
    }
}

#[test]
fn surfaces_are_indexed_by_slot() {
    let names: Vec<_> = SURFACES.iter().map(|s| s.name).collect();
    assert_eq!(names, ["grass", "rock", "forest", "soil"]);
    assert_eq!(SURFACES.len(), 4);
}

#[test]
fn rvmat_lists_the_four_layers() {
    let set = layers(&flat(256, 100.0), 64);
    let text = String::from_utf8(file(&set.files, "data/layers/p_000-000.rvmat").to_vec()).unwrap();
    let dir = "dz\\surfaces_bliss\\data\\terrain\\";
    for (slot, name) in ["en_grass1", "en_stones", "en_forest_con", "en_soil"]
        .iter()
        .enumerate()
    {
        let first = 3 + 2 * slot;
        for (stage, suffix) in [(first, "nopx"), (first + 1, "ca")] {
            let want =
                format!("class Stage{stage} {{\n    texture = \"{dir}{name}_{suffix}.paa\";");
            assert!(text.contains(&want), "stage {stage}: {text}");
        }
    }
    for stage in 11..=14 {
        let want = format!("class Stage{stage} {{\n    texture = \"\";");
        assert!(text.contains(&want), "stage {stage} must stay empty");
    }
}

#[test]
fn baked_shading_stays_light() {
    // a steep wall that falls to the east: it faces east, away from the north-west sun
    let mut hm = flat(256, 0.0);
    for row in 0..256usize {
        for col in 0..256usize {
            hm.h[row * 256 + col] = ((255 - col) as f32) * 4.0 * 3.0_f32.sqrt();
        }
    }
    let rock = SURFACES[SurfaceClass::Rock.slot()].sat_rgb;
    let set = build_layers(
        &hm,
        32,
        PREFIX,
        &SurfaceGrid::from_fn(64, 16.0, |_, _| SurfaceClass::Rock),
        None,
    )
    .unwrap();
    let sat = image::load_from_memory(file(&set.files, "data/layers/s_000_000_lco.png"))
        .unwrap()
        .to_rgb8();
    for (got, class) in sat.get_pixel(256, 256).0.iter().zip(rock) {
        assert!(
            f32::from(*got) >= f32::from(class) * 0.7 - 1.0,
            "pixel channel {got} is darker than 70 % of {class}"
        );
    }
}

// ---- imagery ----

/// A project whose map is the 1024 m square of `flat(256, ..)`.
fn imagery_project() -> Project {
    let mut p = Project::new("t", 47.0, 12.0, 2048.0);
    p.grid.terrain_size = 256;
    p.grid.land_size = 32;
    p
}

/// A raster over (a part of) the map; `colour` gets the map position in metres.
fn raster_over(
    p: &Project,
    (x0, y0, x1, y1): (f64, f64, f64, f64),
    colour: impl Fn(f64, f64) -> Option<[u8; 3]>,
) -> ImageryRaster {
    let geo = MapGeo::new(p).unwrap();
    // the box of the part given in map metres, in degrees
    let corner = |x: f64, y: f64| geo.to_geo(x, y).unwrap();
    let (lat_a, lon_a) = corner(x0, y0);
    let (lat_b, lon_b) = corner(x1, y1);
    ImageryRaster::from_fn(
        lon_a.min(lon_b) - 0.0004,
        lat_a.min(lat_b) - 0.0004,
        lon_a.max(lon_b) + 0.0004,
        lat_a.max(lat_b) + 0.0004,
        1e-4,
        |lat, lon| {
            let (x, y) = geo.to_map(lat, lon).unwrap();
            if x < x0 || x > x1 || y < y0 || y > y1 {
                return None;
            }
            colour(x, y)
        },
    )
}

fn sat_png(set: &naves_export::layers::LayerSet, tile: &str) -> image::RgbImage {
    image::load_from_memory(file(&set.files, &format!("data/layers/s_{tile}_lco.png")))
        .unwrap()
        .to_rgb8()
}

fn with_imagery(
    hm: &Heightmap,
    p: &Project,
    rasters: Vec<ImageryRaster>,
) -> naves_export::layers::LayerSet {
    let (composite, _) = merge(rasters, p).unwrap();
    let imagery = TileImagery {
        composite: &composite,
        project: p,
    };
    build_layers(hm, 32, PREFIX, &procedural_grid(hm, LIMIT), Some(&imagery)).unwrap()
}

#[test]
fn no_imagery_is_todays_image() {
    let hm = flat(256, 100.0);
    let p = imagery_project();
    let plain = layers(&hm, 32);
    // imagery that covers nothing leaves the picture as it was
    let set = with_imagery(&hm, &p, Vec::new());
    assert_eq!(set.files, plain.files);
    assert!(set.imagery_share.abs() < 1e-12);
}

#[test]
fn imagery_replaces_class_colour_without_shading() {
    // a steep wall falling to the east: shaded and rock coloured without imagery
    let mut hm = flat(256, 0.0);
    for row in 0..256usize {
        for col in 0..256usize {
            hm.h[row * 256 + col] = ((255 - col) as f32) * 4.0 * 3.0_f32.sqrt();
        }
    }
    let p = imagery_project();
    let r = raster_over(&p, (-50.0, -50.0, 1074.0, 1074.0), |_, _| {
        Some([90, 120, 60])
    });
    let set = with_imagery(&hm, &p, vec![r]);
    let sat = sat_png(&set, "000_000");
    assert_eq!(sat.get_pixel(256, 256).0, [90, 120, 60]);
    assert!(
        (set.imagery_share - 1.0).abs() < 1e-9,
        "{}",
        set.imagery_share
    );
}

#[test]
fn uncovered_pixels_fall_back() {
    let hm = flat(256, 100.0);
    let p = imagery_project();
    // the western half only
    let r = raster_over(&p, (-50.0, -50.0, 512.0, 1074.0), |_, _| {
        Some([200, 10, 10])
    });
    let set = with_imagery(&hm, &p, vec![r]);
    let plain = layers(&hm, 32);
    // tile (3, 0) is in the east: unchanged, bit for bit
    assert_eq!(
        file(&set.files, "data/layers/s_003_000_lco.png"),
        file(&plain.files, "data/layers/s_003_000_lco.png")
    );
    // tile (0, 0) is in the west: the imagery
    assert_eq!(
        sat_png(&set, "000_000").get_pixel(256, 256).0,
        [200, 10, 10]
    );
    assert!(
        (set.imagery_share - 0.5).abs() < 0.02,
        "{}",
        set.imagery_share
    );
}

#[test]
fn brightness_is_applied_and_clamped() {
    let hm = flat(256, 100.0);
    let mut p = imagery_project();
    p.imagery.brightness = 1.5;
    let r = raster_over(&p, (-50.0, -50.0, 1074.0, 1074.0), |_, _| {
        Some([100, 200, 40])
    });
    let set = with_imagery(&hm, &p, vec![r]);
    assert_eq!(
        sat_png(&set, "000_000").get_pixel(256, 256).0,
        [150, 255, 60]
    );
}

/// The tile and pixel that hold the map position (`x`, `y`) in tile (`tx`, `ty`), `ty` counted from the north.
fn pixel_in(map_m: f64, (tx, ty): (u32, u32), x: f64, y: f64) -> (u32, u32) {
    let g = tile_grid(map_m, 32);
    let tile_m = f64::from(g.tile_px - 2 * g.overlap_px) * g.px_m;
    let overlap = f64::from(g.overlap_px);
    let west = f64::from(tx) * tile_m;
    let north = map_m - f64::from(ty) * tile_m;
    (
        ((x - west) / g.px_m + overlap).floor() as u32,
        ((north - y) / g.px_m + overlap).floor() as u32,
    )
}

#[test]
fn marked_pixel_lands_on_the_tile_position() {
    let hm = flat(256, 100.0);
    let p = imagery_project();
    let marker = |x: f64, y: f64| {
        let inside = (x - 509.0).abs() < 15.0 && (y - 300.0).abs() < 15.0;
        Some(if inside { [250, 10, 10] } else { [50, 60, 70] })
    };
    let r = raster_over(&p, (-50.0, -50.0, 1074.0, 1074.0), marker);
    let set = with_imagery(&hm, &p, vec![r]);
    // (509, 300) is 3 m west of the border between the tile columns 1 and 2: in the overlap of both
    let west_tile = pixel_in(1024.0, (1, 2), 509.0, 300.0);
    let east_tile = pixel_in(1024.0, (2, 2), 509.0, 300.0);
    assert!(
        west_tile.0 < 512 && east_tile.0 < 512,
        "{west_tile:?} {east_tile:?}"
    );
    let a = sat_png(&set, "001_002")
        .get_pixel(west_tile.0, west_tile.1)
        .0;
    let b = sat_png(&set, "002_002")
        .get_pixel(east_tile.0, east_tile.1)
        .0;
    assert_eq!(a, [250, 10, 10], "tile 1");
    assert_eq!(b, [250, 10, 10], "tile 2");
    // 20 m away from the marker: the background, which proves that it is not everywhere
    let off = pixel_in(1024.0, (1, 2), 509.0 - 60.0, 300.0);
    assert_eq!(
        sat_png(&set, "001_002").get_pixel(off.0, off.1).0,
        [50, 60, 70]
    );
}

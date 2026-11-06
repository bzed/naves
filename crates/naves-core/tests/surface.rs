// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ground class of every map cell.

#![expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point and the numbers are tiny"
)]

use naves_core::heightmap::Heightmap;
use naves_core::surface::{
    SurfaceClass, SurfaceGrid, grass_limit_m, procedural_class, procedural_grid, surface_grid_size,
};

const LIMIT: f32 = 1000.0;

fn flat(size: u32, height: f32) -> Heightmap {
    Heightmap {
        size,
        cell_m: 4.0,
        h: vec![height; (size * size) as usize],
    }
}

#[test]
fn grid_size_is_capped() {
    assert_eq!(surface_grid_size(1024), 4096);
    assert_eq!(surface_grid_size(2048), 8192);
    assert_eq!(surface_grid_size(8192), 8192);
}

#[test]
fn slots_follow_the_mask_colours() {
    let slots: Vec<usize> = [
        SurfaceClass::Grass,
        SurfaceClass::Rock,
        SurfaceClass::Forest,
        SurfaceClass::Soil,
    ]
    .iter()
    .map(|c| c.slot())
    .collect();
    assert_eq!(slots, vec![0, 1, 2, 3]);
}

#[test]
fn at_clamps_to_the_map() {
    // 4 x 4 cells of 10 m; the class encodes the cell: column 0 grass, 1 rock, 2 forest, 3 soil
    let grid = SurfaceGrid::from_fn(4, 10.0, |col, _| match col {
        0 => SurfaceClass::Grass,
        1 => SurfaceClass::Rock,
        2 => SurfaceClass::Forest,
        _ => SurfaceClass::Soil,
    });
    assert_eq!(grid.at(5.0, 5.0), SurfaceClass::Grass);
    assert_eq!(grid.at(15.0, 5.0), SurfaceClass::Rock);
    assert_eq!(grid.at(-100.0, 5.0), SurfaceClass::Grass);
    assert_eq!(grid.at(1.0e6, 5.0), SurfaceClass::Soil);
    assert_eq!(grid.at(25.0, -50.0), SurfaceClass::Forest);
}

#[test]
fn from_fn_is_south_row_first() {
    let grid = SurfaceGrid::from_fn(2, 10.0, |_, row| {
        if row == 0 {
            SurfaceClass::Rock
        } else {
            SurfaceClass::Soil
        }
    });
    assert_eq!(grid.at(5.0, 5.0), SurfaceClass::Rock, "south");
    assert_eq!(grid.at(5.0, 15.0), SurfaceClass::Soil, "north");
}

#[test]
fn procedural_rule_is_unchanged() {
    // a 60 degree wall: height rises by tan(60) * 4 m per vertex along x from column 128 on
    let mut hm = flat(256, 0.0);
    for row in 0..256usize {
        for col in 128..256usize {
            hm.h[row * 256 + col] = ((col - 128) as f32) * 4.0 * 3.0_f32.sqrt();
        }
    }
    let grid = procedural_grid(&hm, LIMIT);
    assert_eq!(grid.size, 1024);
    assert_eq!(
        grid.at(80.0, 400.0),
        SurfaceClass::Grass,
        "flat below the limit"
    );
    assert_eq!(grid.at(900.0, 400.0), SurfaceClass::Rock, "on the wall");
    assert_eq!(
        procedural_grid(&flat(256, 1100.0), LIMIT).at(500.0, 500.0),
        SurfaceClass::Rock,
        "flat above the limit"
    );
    assert_eq!(
        procedural_grid(&flat(256, 900.0), LIMIT).at(500.0, 500.0),
        SurfaceClass::Grass
    );
}

#[test]
fn procedural_class_uses_slope_and_height() {
    use naves_core::heightmap::Sample;
    let sample = |height, dz_dx| Sample {
        height,
        dz_dx,
        dz_dy: 0.0,
    };
    assert_eq!(
        procedural_class(&sample(10.0, 0.0), LIMIT),
        SurfaceClass::Grass
    );
    assert_eq!(
        procedural_class(&sample(2000.0, 0.0), LIMIT),
        SurfaceClass::Rock
    );
    assert_eq!(
        procedural_class(&sample(10.0, 1.0), LIMIT),
        SurfaceClass::Rock
    );
}

#[test]
fn the_grass_limit_is_a_real_altitude() {
    // measured with ESA WorldCover around Berchtesgaden; map heights are real heights times scale.height
    assert!((grass_limit_m(1.0) - 2100.0).abs() < 1e-3);
    assert!((grass_limit_m(0.5) - 1050.0).abs() < 1e-3);
}

// ---- build_surfaces ----

use naves_core::heightmap::MapGeo;
use naves_core::landcover::LandCover;
use naves_core::osm::{LandPolygon, OsmLand};
use naves_core::project::Project;
use naves_core::projection::LocalProjection;
use naves_core::surface::{SurfaceSources, build_surfaces};

const LAT: f64 = 47.585;
const LON: f64 = 12.965;

/// 2000 m real extent at half scale: a 1000 m map of 256 vertices (3.9 m) with 1024 surface cells.
fn project() -> Project {
    let mut p = Project::new("t", LAT, LON, 2000.0);
    p.grid.terrain_size = 256;
    p.grid.land_size = 32;
    p.scale.height = 1.0;
    p
}

fn heightmap(f: impl Fn(f64, f64) -> f32) -> Heightmap {
    let size = 256_u32;
    let cell = 1000.0 / f64::from(size);
    let mut h = Vec::new();
    for row in 0..size {
        for col in 0..size {
            h.push(f(f64::from(col) * cell, f64::from(row) * cell));
        }
    }
    Heightmap {
        size,
        cell_m: cell as f32,
        h,
    }
}

/// Flat ground at 100 m: the rules say grass.
fn flat_map() -> Heightmap {
    heightmap(|_, _| 100.0)
}

/// A square polygon around the map position (`x`, `y`), `half` map metres to each side.
fn square_at(
    p: &Project,
    x: f64,
    y: f64,
    half: f64,
    class: SurfaceClass,
    priority: u8,
) -> LandPolygon {
    let geo = MapGeo::new(p).unwrap();
    let corner = |dx: f64, dy: f64| geo.to_geo(x + dx, y + dy).unwrap();
    LandPolygon {
        class,
        priority,
        outer: vec![
            corner(-half, -half),
            corner(half, -half),
            corner(half, half),
            corner(-half, half),
            corner(-half, -half),
        ],
        holes: Vec::new(),
    }
}

fn osm(polygons: Vec<LandPolygon>) -> OsmLand {
    OsmLand {
        polygons,
        skipped_rings: 0,
    }
}

/// Land cover that says `code(lat, lon)` over a window that covers the whole map.
fn land_cover(code: impl Fn(f64, f64) -> u8) -> LandCover {
    LandCover::from_fn(LON - 0.05, LAT - 0.05, LON + 0.05, LAT + 0.05, 0.0001, code)
}

fn sources<'a>(lc: Option<&'a LandCover>, osm: Option<&'a OsmLand>) -> SurfaceSources<'a> {
    SurfaceSources {
        land_cover: lc,
        osm,
    }
}

#[test]
fn osm_beats_land_cover() {
    let p = project();
    let lc = land_cover(|_, _| 10); // forest everywhere
    let lake = osm(vec![square_at(
        &p,
        500.0,
        500.0,
        100.0,
        SurfaceClass::Soil,
        3,
    )]);
    let built = build_surfaces(&p, &flat_map(), &sources(Some(&lc), Some(&lake))).unwrap();
    assert_eq!(built.grid.at(500.0, 500.0), SurfaceClass::Soil);
    assert_eq!(built.grid.at(100.0, 100.0), SurfaceClass::Forest);
}

#[test]
fn higher_priority_polygon_wins() {
    let p = project();
    // overlapping squares: the big one is rock (priority 2), the small one built-up (priority 1), listed first
    let land = osm(vec![
        square_at(&p, 500.0, 500.0, 50.0, SurfaceClass::Soil, 1),
        square_at(&p, 500.0, 500.0, 200.0, SurfaceClass::Rock, 2),
    ]);
    let built = build_surfaces(&p, &flat_map(), &sources(None, Some(&land))).unwrap();
    assert_eq!(
        built.grid.at(500.0, 500.0),
        SurfaceClass::Rock,
        "centre: both, rock wins"
    );
    assert_eq!(
        built.grid.at(500.0, 650.0),
        SurfaceClass::Rock,
        "only the big square"
    );
}

#[test]
fn a_hole_lets_the_land_cover_through() {
    let p = project();
    let mut lake = square_at(&p, 500.0, 500.0, 200.0, SurfaceClass::Soil, 3);
    lake.holes
        .push(square_at(&p, 500.0, 500.0, 50.0, SurfaceClass::Soil, 3).outer);
    let lc = land_cover(|_, _| 30); // grassland
    let land = osm(vec![lake]);
    let built = build_surfaces(&p, &flat_map(), &sources(Some(&lc), Some(&land))).unwrap();
    assert_eq!(
        built.grid.at(500.0, 500.0),
        SurfaceClass::Grass,
        "the island"
    );
    assert_eq!(built.grid.at(500.0, 650.0), SurfaceClass::Soil, "the water");
}

#[test]
fn land_cover_beats_the_rules() {
    let p = project();
    // a 60 degree wall everywhere: the rules say rock, WorldCover says grassland
    let wall = heightmap(|x, _| (x * 3.0_f64.sqrt()) as f32);
    let lc = land_cover(|_, _| 30);
    let built = build_surfaces(&p, &wall, &sources(Some(&lc), None)).unwrap();
    assert_eq!(built.grid.at(500.0, 500.0), SurfaceClass::Grass);
    assert!(built.uncovered_share < 1e-9);
}

#[test]
fn without_any_source_it_is_the_procedural_grid() {
    let p = project();
    let wall = heightmap(|x, _| {
        if x < 500.0 {
            100.0
        } else {
            ((x - 500.0) * 3.0_f64.sqrt()) as f32 + 100.0
        }
    });
    let built = build_surfaces(&p, &wall, &sources(None, None)).unwrap();
    assert_eq!(
        built.grid,
        procedural_grid(&wall, grass_limit_m(p.scale.height))
    );
    assert!(
        built.uncovered_share.abs() < 1e-12,
        "nothing to be uncovered by"
    );
}

#[test]
fn uncovered_part_uses_the_rules_and_is_reported() {
    let p = project();
    // the land cover covers the western half of the map only (forest); the eastern half is high ground
    let geo = MapGeo::new(&p).unwrap();
    let (_, mid_lon) = geo.to_geo(500.0, 500.0).unwrap();
    let lc = LandCover::from_fn(
        LON - 0.05,
        LAT - 0.05,
        mid_lon,
        LAT + 0.05,
        0.0001,
        |_, _| 10,
    );
    let map = heightmap(|x, _| if x < 500.0 { 100.0 } else { 5000.0 });
    let built = build_surfaces(&p, &map, &sources(Some(&lc), None)).unwrap();
    assert_eq!(built.grid.at(250.0, 500.0), SurfaceClass::Forest, "covered");
    assert_eq!(
        built.grid.at(750.0, 500.0),
        SurfaceClass::Rock,
        "uncovered: the rules"
    );
    assert!(
        (built.uncovered_share - 0.5).abs() < 0.02,
        "share {}",
        built.uncovered_share
    );
}

#[test]
fn position_uses_the_map_scale() {
    // at scale.horizontal 0.5 a spot 500 real metres east of the centre is 250 map metres east of the map centre
    let p = project();
    let proj = LocalProjection::for_center(LAT, LON).unwrap();
    let (e, n) = proj.to_utm(LAT, LON).unwrap();
    let corner = |de: f64, dn: f64| proj.to_geo(e + 500.0 + de, n + dn).unwrap();
    let lake = LandPolygon {
        class: SurfaceClass::Soil,
        priority: 3,
        outer: vec![
            corner(-20.0, -20.0),
            corner(20.0, -20.0),
            corner(20.0, 20.0),
            corner(-20.0, 20.0),
            corner(-20.0, -20.0),
        ],
        holes: Vec::new(),
    };
    let land = osm(vec![lake]);
    let built = build_surfaces(&p, &flat_map(), &sources(None, Some(&land))).unwrap();
    assert_eq!(
        built.grid.at(750.0, 500.0),
        SurfaceClass::Soil,
        "250 m east of the map centre"
    );
    assert_eq!(built.grid.at(500.0, 500.0), SurfaceClass::Grass);
    assert_eq!(
        built.grid.at(990.0, 500.0),
        SurfaceClass::Grass,
        "where an unscaled position would land"
    );
}

#[test]
fn surfaces_are_deterministic() {
    let p = project();
    let lc = land_cover(|lat, lon| {
        if (lat * 1000.0 + lon * 1000.0) as i64 % 2 == 0 {
            10
        } else {
            40
        }
    });
    let land = osm(vec![
        square_at(&p, 300.0, 300.0, 80.0, SurfaceClass::Soil, 1),
        square_at(&p, 320.0, 320.0, 80.0, SurfaceClass::Rock, 2),
    ]);
    let a = build_surfaces(&p, &flat_map(), &sources(Some(&lc), Some(&land))).unwrap();
    let b = build_surfaces(&p, &flat_map(), &sources(Some(&lc), Some(&land))).unwrap();
    assert_eq!(a.grid, b.grid);
}

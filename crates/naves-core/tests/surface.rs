// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ground class of every map cell.

#![expect(
    clippy::cast_precision_loss,
    reason = "test helper code; indices are tiny"
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

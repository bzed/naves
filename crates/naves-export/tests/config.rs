// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The generated world `config.cpp`.

#![expect(
    clippy::indexing_slicing,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::heightmap::Heightmap;
use naves_core::project::Project;
use naves_export::config::{prefix, world_config};

fn heightmap(max: f32) -> Heightmap {
    let mut h = vec![10.0; 16];
    h[5] = max;
    Heightmap {
        size: 4,
        cell_m: 250.0,
        h,
    }
}

fn project() -> Project {
    // 2000 m real extent at 0.5 horizontal scale: a 1000 m map
    Project::new("Berchtesgaden", 47.585, 12.965, 2000.0)
}

#[test]
fn prefix_is_the_in_game_path() {
    assert_eq!(prefix("berchtesgaden"), "naves\\worlds\\berchtesgaden");
}

#[test]
fn config_names_world() {
    let c = world_config(&project(), &heightmap(300.0));
    assert!(c.contains("class berchtesgaden: CAWorld"), "{c}");
    assert!(c.contains(r#"worldName = "naves\worlds\berchtesgaden\berchtesgaden.wrp";"#));
    assert!(c.contains("class naves_berchtesgaden"));
    assert!(c.contains("class CfgWorldList"));
    assert!(c.contains("requiredAddons[] = {\"DZ_Data\", \"DZ_Surfaces_Bliss\"};"));
}

#[test]
fn centre_and_grid_follow_the_map_size() {
    let c = world_config(&project(), &heightmap(300.0));
    assert!(c.contains("centerPosition[] = {500, 500, 300};"), "{c}");
    assert!(c.contains("offsetY = 1000;"), "{c}");
}

#[test]
fn names_has_one_location_at_the_centre() {
    let c = world_config(&project(), &heightmap(300.0));
    assert!(c.contains("class Names {"));
    assert!(c.contains("name = \"Berchtesgaden\";"));
    assert!(c.contains("position[] = {500, 500};"));
}

#[test]
fn everything_is_ascii_for_non_ascii_names() {
    let p = Project::new("Königssee \"Test\"", 47.55, 12.97, 2000.0);
    let c = world_config(&p, &heightmap(300.0));
    assert!(c.is_ascii(), "non-ASCII text in the config");
    assert!(c.contains("class konigssee_test: CAWorld"));
    assert!(c.contains("name = \"Konigssee Test\";"), "{c}");
}

#[test]
fn config_is_deterministic() {
    let a = world_config(&project(), &heightmap(300.0));
    let b = world_config(&project(), &heightmap(300.0));
    assert_eq!(a, b);
}

#[test]
#[ignore = "needs armake2"]
fn config_rapifies() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("config.cpp");
    std::fs::write(&src, world_config(&project(), &heightmap(300.0))).unwrap();
    let status = std::process::Command::new("armake2")
        .arg("rapify")
        .arg(&src)
        .arg(dir.path().join("config.bin"))
        .status()
        .unwrap();
    assert!(status.success());
}

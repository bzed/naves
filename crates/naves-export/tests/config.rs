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
    assert!(c.contains("class berchtesgaden: Enoch"), "{c}");
    assert!(c.contains(r#"worldName = "naves\worlds\berchtesgaden\berchtesgaden.wrp";"#));
    assert!(c.contains("class naves_berchtesgaden"));
    assert!(c.contains("class CfgWorldList"));
    assert!(
        c.contains("requiredVersion = 1.14;"),
        "0.1 is reported to break worlds: {c}"
    );
    assert!(
        c.contains(r#"ceFiles = "naves\worlds\berchtesgaden\ce";"#),
        "{c}"
    );
    assert!(
        c.contains("requiredAddons[] = {\"DZ_Data\", \"DZ_Surfaces_Bliss\", \"DZ_Worlds_Enoch\"};")
    );
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
    assert!(c.contains("class konigssee_test: Enoch"));
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

#[test]
fn the_world_inherits_the_sky_and_weather_from_livonia() {
    // The client crashes without the sky, cloud and weather entries; they are inherited, not copied.
    let c = world_config(&project(), &heightmap(300.0));
    assert!(c.contains("class Enoch: CAWorld {"), "{c}");
    assert!(
        c.contains("class Navmesh {") && c.contains("class GenParams;"),
        "base classes must be declared for binarize: {c}"
    );
    assert!(
        !c.contains("skyTexture") && !c.contains("horizonObject"),
        "no copied game values"
    );
    // Livonia's holes must not leak into the new map
    assert!(c.contains("class Holes {};"), "{c}");
}

#[test]
fn the_sun_follows_the_project_location() {
    let c = world_config(&project(), &heightmap(300.0));
    assert!(
        // the game counts latitude positive to the south (Livonia -54, Chernarus -56); with the wrong sign the
        // noon sun stands in the north
        c.contains("latitude = -47.585;") && c.contains("longitude = 12.965;"),
        "{c}"
    );
}

#[test]
fn navmesh_class_inherits_livonia_and_points_at_our_file() {
    // The DayZDiag navmesh data server takes tile width, agent and link types from Navmesh/GenParams;
    // they are inherited by reference, only the file name and the seed position are ours.
    let c = world_config(&project(), &heightmap(300.0));
    assert!(c.contains("class Navmesh: Navmesh {"), "{c}");
    assert!(
        c.contains(r#"navmeshName = "naves\worlds\berchtesgaden\navmesh\berchtesgaden.nm";"#),
        "{c}"
    );
    assert!(c.contains("class GenParams: GenParams {"), "{c}");
    assert!(c.contains("seedPosition[] = {500, 0, 500};"), "{c}");
    assert!(!c.contains("class Navmesh {};"), "{c}");
}

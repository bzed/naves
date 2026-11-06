// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Runs the fuzz seed corpora as ordinary tests: every seed must give `Ok` or `Err`, never a panic.

#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helper code; failing loudly is the point"
)]

use std::path::PathBuf;

use naves_core::dem::DemTile;
use naves_core::landcover::{GeoWindow, LandCover};
use naves_core::project::Project;

fn seeds(target: &str) -> Vec<(String, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/corpus")
        .join(target);
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no seeds in {}", dir.display());
    files
        .iter()
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read(p).unwrap())
        })
        .collect()
}

#[test]
fn dem_tile_seeds() {
    for (name, data) in seeds("dem_tile") {
        let tile = DemTile::from_reader(std::io::Cursor::new(data));
        assert!(tile.is_ok(), "seed {name}: {:?}", tile.err());
    }
}

#[test]
fn project_parse_seeds() {
    for (name, data) in seeds("project_parse") {
        let result = Project::parse(&String::from_utf8_lossy(&data));
        assert_eq!(result.is_ok(), name == "roundtrip", "seed {name}");
    }
}

#[test]
fn mcp_project_set_seeds() {
    for (name, data) in seeds("mcp_project_set") {
        let request: serde_json::Value = serde_json::from_slice(&data).unwrap();
        let path = request["path"].as_str().unwrap();
        let result =
            Project::new("Seed", 47.0, 12.0, 4000.0).with_value(path, request["value"].clone());
        assert_eq!(
            result.is_ok(),
            name == "valid" || name == "list",
            "seed {name}: {result:?}"
        );
    }
}

#[test]
fn osm_read_seeds() {
    let window = GeoWindow {
        south: 47.54,
        west: 12.96,
        north: 47.56,
        east: 12.99,
    };
    for (name, data) in seeds("osm_read") {
        let result = naves_core::osm::read_land_from_bytes(&data, window);
        assert_eq!(result.is_ok(), name == "valid", "seed {name}: {result:?}");
    }
}

#[test]
fn imagery_catalog_seeds() {
    for (name, data) in seeds("imagery_catalog") {
        let text = String::from_utf8(data).unwrap();
        let result = naves_core::imagery::catalog::parse_sources(&text);
        assert_eq!(result.is_ok(), name == "valid", "seed {name}");
    }
}

#[test]
fn imagery_tile_seeds() {
    let project = Project::new("Seed", 47.557, 12.974, 800.0);
    for (name, data) in seeds("imagery_tile") {
        let result = naves_core::imagery::ImageryRaster::from_bytes(data, &project, 2.0);
        assert_eq!(result.is_ok(), name == "valid", "seed {name}");
    }
}

#[test]
fn landcover_tile_seeds() {
    let window = GeoWindow {
        south: 47.54,
        west: 12.96,
        north: 47.56,
        east: 12.99,
    };
    for (name, data) in seeds("landcover_tile") {
        let result = LandCover::from_bytes(data, window);
        assert_eq!(result.is_ok(), name == "valid", "seed {name}");
    }
}

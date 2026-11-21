// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Runs the fuzz seed corpora as ordinary tests: every seed must give `Ok` or `Err`, never a panic.

#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helper code; failing loudly is the point"
)]

use std::path::PathBuf;

use naves_core::catalog::PieceCatalog;
use naves_core::dem::DemTile;
use naves_core::landcover::{GeoWindow, LandCover};
use naves_core::prefs::Preferences;
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
fn prefs_parse_seeds() {
    for (name, data) in seeds("prefs_parse") {
        let result = Preferences::parse(&String::from_utf8_lossy(&data));
        assert_eq!(
            result.is_ok(),
            !matches!(name.as_str(), "damaged" | "wrong_type"),
            "seed {name}"
        );
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
fn osm_ways_seeds() {
    let window = GeoWindow {
        south: 47.596,
        west: 12.978,
        north: 47.606,
        east: 12.992,
    };
    for (name, data) in seeds("osm_ways") {
        let result = naves_core::osm::read_ways_from_bytes(&data, window);
        assert_eq!(
            result.is_ok(),
            name.starts_with("valid"),
            "seed {name}: {result:?}"
        );
    }
}

#[test]
fn overrides_parse_seeds() {
    for (name, data) in seeds("overrides_parse") {
        let text = String::from_utf8(data).unwrap();
        let result = naves_core::objects::Overrides::parse(&text);
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

#[test]
fn odol_read_seeds_measure_as_bridge_parts() {
    // what the odol_read target does with a model that reads: never a panic
    for (_, data) in seeds("odol_read") {
        let Ok(model) = naves_formats::odol::read_odol(&data) else {
            continue;
        };
        let models: std::collections::BTreeMap<String, _> = [
            "dz\\structures\\roads\\bridges\\bridge_metal_25.p3d",
            "dz\\structures\\roads\\panels\\panel_concrete_1.p3d",
            "dz\\structures\\industrial\\misc\\misc_concretepipe.p3d",
        ]
        .into_iter()
        .map(|k| (k.to_string(), model.clone()))
        .collect();
        drop(PieceCatalog::from_models(&models));
    }
}

#[test]
fn smooth_line_seeds() {
    use naves_core::smooth::{Sample, SmoothParams, resample, smooth_line};
    for (name, data) in seeds("smooth_line") {
        let samples: Vec<Sample> = data
            .as_chunks::<17>()
            .0
            .iter()
            .map(|c| Sample {
                at: (
                    f64::from_le_bytes(c[0..8].try_into().unwrap()),
                    f64::from_le_bytes(c[8..16].try_into().unwrap()),
                ),
                fixed: c[16] & 1 != 0,
                stiffness: if c[16] & 2 != 0 { 10.0 } else { 1.0 },
            })
            .collect();
        let result = smooth_line(&samples, &SmoothParams::default());
        let good = matches!(name.as_str(), "corner" | "straight");
        assert_eq!(result.is_ok(), good, "seed {name}: {:?}", result.err());
        let line: Vec<(f64, f64)> = samples.iter().map(|s| s.at).collect();
        let _ = resample(&line, 2.0);
    }
}

#[test]
fn reroute_raster_seeds() {
    use naves_core::reroute::raster::{Clearance, Obstacles, chamfer, exempt_samples, samples};
    for (name, data) in seeds("reroute_raster") {
        let mut road = Vec::new();
        let mut original = Vec::new();
        let mut probes = Vec::new();
        for c in data.as_chunks::<17>().0 {
            let p = (
                f64::from_le_bytes(c[0..8].try_into().unwrap()),
                f64::from_le_bytes(c[8..16].try_into().unwrap()),
            );
            match c[16] {
                0 => road.push(p),
                1 => original.push(p),
                3 => probes.push(p),
                _ => {}
            }
        }
        let mut o = Obstacles::new((-50.0, -50.0), (250.0, 250.0), 4.0, 200.0, 3.0, 20_000);
        o.stamp_water(&|p| p.0 > 100.0 && p.0 < 120.0 && p.1 > 100.0 && p.1 < 120.0);
        let flags: Vec<bool> = samples(&road).iter().map(|p| p.0 < 100.0).collect();
        o.stamp_road(&road, 6.0, &flags);
        o.set_original(&original);
        o.finish();
        let clear = Clearance::new(5.0, 12.0).unwrap();
        for pair in probes.windows(2) {
            let blocked = o.segment(pair[0], pair[1], clear);
            if name == "scene" {
                assert!(
                    blocked.is_err(),
                    "seed {name}: a probe through the lake is free"
                );
                break;
            }
        }
        let _ = (samples(&road), exempt_samples(&road, &original, 20.0, 10.0));
        let _ = o.water_cap_m();
        let _ = chamfer(&[true, false, true], 3, 1, 2.0);
    }
}

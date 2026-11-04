// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Behaviour of the project file (`naves.toml`).

#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::ident::world_ident;
use naves_core::project::{Project, ProjectError};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn invalid_field(toml: &str, field: &str) {
    match Project::parse(toml) {
        Err(ProjectError::Invalid(msg)) => {
            assert!(msg.contains(field), "message {msg:?} does not name {field}");
        }
        other => panic!("expected Invalid({field}), got {other:?}"),
    }
}

fn base() -> String {
    let p = Project::new("Berchtesgaden", 47.585, 12.965, 12800.0);
    toml::to_string(&p).unwrap()
}

#[test]
fn ident_is_ascii() {
    assert_eq!(world_ident("Náves"), "naves");
    assert_eq!(world_ident("Königssee Test"), "konigssee_test");
    assert_eq!(world_ident("3 Gipfel"), "w3_gipfel");
    assert_eq!(world_ident(""), "w");
    assert_eq!(world_ident("  --  "), "w");
}

#[test]
fn roundtrip_toml() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Project::new("Berchtesgaden", 47.585, 12.965, 12800.0);
    p.sources.dem = vec!["dem/a.tif".into(), "dem/b.tif".into()];
    p.save(dir.path()).unwrap();
    assert_eq!(Project::load(dir.path()).unwrap(), p);
}

#[test]
fn save_leaves_only_the_project_file() {
    let dir = tempfile::tempdir().unwrap();
    Project::new("x", 0.0, 0.0, 1000.0)
        .save(dir.path())
        .unwrap();
    Project::new("y", 0.0, 0.0, 1000.0)
        .save(dir.path())
        .unwrap();
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("naves.toml")]);
    assert_eq!(Project::load(dir.path()).unwrap().name, "y");
}

#[test]
fn new_uses_documented_defaults() {
    let p = Project::new("Königssee Test", 1.0, 2.0, 4000.0);
    assert_eq!(p.world, "konigssee_test");
    assert!(close(p.scale.horizontal, 0.5) && close(p.scale.height, 0.5));
    assert_eq!((p.grid.terrain_size, p.grid.land_size), (1024, 256));
    assert!(p.sources.dem.is_empty());
}

#[test]
fn derived_sizes() {
    let p = Project::new("x", 0.0, 0.0, 12800.0);
    assert!(close(p.map_size_m(), 6400.0));
    assert!(close(p.terrain_cell_m(), 6.25));
}

#[test]
fn rejects_bad_grid() {
    let t = base().replace("terrain_size = 1024", "terrain_size = 1000");
    invalid_field(&t, "terrain_size");
    let t = base().replace("terrain_size = 1024", "terrain_size = 128");
    invalid_field(&t, "terrain_size");
    let t = base().replace("terrain_size = 1024", "terrain_size = 16384");
    invalid_field(&t, "terrain_size");
    let t = base().replace("land_size = 256", "land_size = 300");
    invalid_field(&t, "land_size");
    let t = base().replace("land_size = 256", "land_size = 2048");
    invalid_field(&t, "land_size");
    // a tile is 32 land cells, and tile materials must fit the 16-bit material index
    let t = base().replace("land_size = 256", "land_size = 16");
    invalid_field(&t, "land_size");
    let t = base()
        .replace("terrain_size = 1024", "terrain_size = 8192")
        .replace("land_size = 256", "land_size = 8192");
    invalid_field(&t, "land_size");
}

#[test]
fn rejects_bad_scale() {
    invalid_field(
        &base().replace("horizontal = 0.5", "horizontal = 3.0"),
        "horizontal",
    );
    invalid_field(
        &base().replace("horizontal = 0.5", "horizontal = 0.0"),
        "horizontal",
    );
    invalid_field(
        &base().replace("horizontal = 0.5", "horizontal = nan"),
        "horizontal",
    );
    invalid_field(&base().replace("height = 0.5", "height = 2.5"), "height");
}

#[test]
fn rejects_bad_world_name() {
    invalid_field(
        &base().replace("world = \"berchtesgaden\"", "world = \"Náves\""),
        "world",
    );
    invalid_field(
        &base().replace("world = \"berchtesgaden\"", "world = \"1abc\""),
        "world",
    );
    invalid_field(
        &base().replace("world = \"berchtesgaden\"", "world = \"\""),
        "world",
    );
}

#[test]
fn rejects_bad_area() {
    invalid_field(
        &base().replace("center_lat = 47.585", "center_lat = 91.0"),
        "center_lat",
    );
    invalid_field(
        &base().replace("center_lon = 12.965", "center_lon = 181.0"),
        "center_lon",
    );
    invalid_field(
        &base().replace("real_extent_m = 12800.0", "real_extent_m = 0.0"),
        "real_extent_m",
    );
}

#[test]
fn rejects_absolute_or_escaping_dem_paths() {
    let t = base().replace("dem = []", "dem = [\"/etc/passwd\"]");
    invalid_field(&t, "sources.dem");
    let t = base().replace("dem = []", "dem = [\"../x.tif\"]");
    invalid_field(&t, "sources.dem");
}

#[test]
fn garbage_is_a_parse_error() {
    assert!(matches!(
        Project::parse("this is not toml ="),
        Err(ProjectError::Parse(_))
    ));
    assert!(matches!(Project::parse(""), Err(ProjectError::Parse(_))));
}

#[test]
fn load_reports_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        Project::load(dir.path()),
        Err(ProjectError::Io { .. })
    ));
}

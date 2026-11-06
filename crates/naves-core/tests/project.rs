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
fn ascii_name_keeps_the_look_of_the_name() {
    use naves_core::ident::ascii_name;
    assert_eq!(ascii_name("Königssee \"Test\""), "Konigssee Test");
    assert_eq!(ascii_name("Náves"), "Naves");
    assert_eq!(ascii_name("  a\tb  "), "a b");
    assert_eq!(ascii_name("日本"), "");
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
    assert_eq!((p.grid.terrain_size, p.grid.land_size), (1024, 128));
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
    let t = base().replace("land_size = 128", "land_size = 300");
    invalid_field(&t, "land_size");
}

/// The game client needs 8 terrain cells per land cell (as Livonia and Chernarus have): with fewer it
/// overflows a fixed 128-height buffer when it builds the land segments and crashes.
#[test]
fn land_grid_must_be_an_eighth_of_the_terrain_grid() {
    for (terrain, land) in [(256, 32), (1024, 128), (8192, 1024)] {
        let t = base()
            .replace("terrain_size = 1024", &format!("terrain_size = {terrain}"))
            .replace("land_size = 128", &format!("land_size = {land}"));
        let p = Project::parse(&t).unwrap();
        assert_eq!((p.grid.terrain_size, p.grid.land_size), (terrain, land));
    }
    for land in [256, 64, 1024] {
        let t = base().replace("land_size = 128", &format!("land_size = {land}"));
        match Project::parse(&t) {
            Err(ProjectError::Invalid(msg)) => {
                assert!(msg.contains("land_size"), "{msg}");
                assert!(msg.contains("terrain_size / 8"), "{msg}");
                assert!(
                    msg.contains("128"),
                    "the message suggests the right value: {msg}"
                );
            }
            other => panic!("land_size = {land}: expected Invalid, got {other:?}"),
        }
    }
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

#[test]
fn with_value_sets_a_nested_field() {
    let p = Project::new("x", 47.0, 12.0, 4000.0);
    let q = p
        .with_value("scale.height", serde_json::json!(0.75))
        .unwrap();
    assert!(close(q.scale.height, 0.75));
    assert!(close(p.scale.height, 0.5), "the original is unchanged");
    let q = p
        .with_value("sources.dem", serde_json::json!(["dem/a.tif"]))
        .unwrap();
    assert_eq!(q.sources.dem, vec![std::path::PathBuf::from("dem/a.tif")]);
    let q = p.with_value("name", serde_json::json!("Other")).unwrap();
    assert_eq!(q.name, "Other");
}

#[test]
fn with_value_validates_like_parse() {
    let p = Project::new("x", 47.0, 12.0, 4000.0);
    let err = p
        .with_value("scale.horizontal", serde_json::json!(3))
        .unwrap_err();
    assert!(err.to_string().contains("horizontal"), "{err}");
    let err = p
        .with_value("grid.terrain_size", serde_json::json!(1000))
        .unwrap_err();
    assert!(err.to_string().contains("terrain_size"), "{err}");
    let err = p
        .with_value("sources.dem", serde_json::json!(["/etc/passwd"]))
        .unwrap_err();
    assert!(err.to_string().contains("sources.dem"), "{err}");
}

#[test]
fn with_value_rejects_unknown_paths_and_wrong_types() {
    let p = Project::new("x", 47.0, 12.0, 4000.0);
    for (path, value) in [
        ("", serde_json::json!(1)),
        ("nope", serde_json::json!(1)),
        ("scale.nope", serde_json::json!(1)),
        ("scale.height.deeper", serde_json::json!(1)),
        ("scale..height", serde_json::json!(1)),
        ("scale.height", serde_json::json!("high")),
        ("scale", serde_json::json!(5)),
        ("scale.height", serde_json::Value::Null),
    ] {
        assert!(
            matches!(
                p.with_value(path, value.clone()),
                Err(ProjectError::Invalid(_))
            ),
            "{path} = {value}"
        );
    }
}

#[test]
fn setting_the_terrain_grid_sets_the_land_grid() {
    let p = Project::new("x", 0.0, 0.0, 12800.0);
    let q = p
        .with_value("grid.terrain_size", serde_json::json!(2048))
        .unwrap();
    assert_eq!((q.grid.terrain_size, q.grid.land_size), (2048, 256));
    // the land grid alone can only be set to the one value that fits
    let err = p
        .with_value("grid.land_size", serde_json::json!(256))
        .unwrap_err();
    assert!(err.to_string().contains("terrain_size / 8"), "{err}");
}

#[cfg(unix)]
#[test]
fn saving_keeps_the_file_mode_and_a_new_file_is_readable() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("naves.toml");
    let mode = |f: &std::path::Path| std::fs::metadata(f).unwrap().permissions().mode() & 0o777;
    let p = Project::new("x", 0.0, 0.0, 12800.0);
    p.save(dir.path()).unwrap();
    assert_eq!(mode(&file), 0o644, "a new project file is not private");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
    p.save(dir.path()).unwrap();
    assert_eq!(mode(&file), 0o640, "saving keeps what the user set");
}

#[test]
fn old_project_files_still_load() {
    let seed = include_str!("../../../fuzz/corpus/project_parse/roundtrip");
    let p = Project::parse(seed).unwrap();
    assert!(p.sources.land_cover.is_empty() && p.sources.osm.is_empty());
    let saved = toml::to_string(&p).unwrap();
    assert!(
        !saved.contains("land_cover") && !saved.contains("osm"),
        "{saved}"
    );
}

#[test]
fn land_cover_and_osm_paths_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Project::new("Berchtesgaden", 47.585, 12.965, 12800.0);
    p.sources.land_cover = vec!["landcover/a.tif".into()];
    p.sources.osm = vec!["osm/b.osm.pbf".into(), "osm/c.osm.pbf".into()];
    p.save(dir.path()).unwrap();
    assert_eq!(Project::load(dir.path()).unwrap(), p);
}

#[test]
fn rejects_absolute_or_escaping_land_cover_and_osm_paths() {
    let t = base().replace("[sources]", "[sources]\nland_cover = [\"/etc/passwd\"]");
    invalid_field(&t, "sources.land_cover");
    let t = base().replace("[sources]", "[sources]\nosm = [\"../x.pbf\"]");
    invalid_field(&t, "sources.osm");
}

fn with_imagery(extra: &str) -> String {
    format!(
        "{}\n[[sources.imagery]]\npath = \"imagery/a.tif\"\nattribution = \"Contains modified Copernicus Sentinel data 2025\"\nlicence = \"Copernicus Sentinel data, free and open\"\n{extra}",
        base()
    )
}

#[test]
fn old_project_has_no_imagery() {
    let p = Project::parse(&base()).unwrap();
    assert!(p.sources.imagery.is_empty());
    let saved = toml::to_string(&p).unwrap();
    assert!(
        !saved.contains("imagery"),
        "saving must not add the keys:\n{saved}"
    );
}

#[test]
fn imagery_round_trips() {
    use naves_core::project::ImageryFile;
    let dir = tempfile::tempdir().unwrap();
    let mut p = Project::new("Berchtesgaden", 47.585, 12.965, 12800.0);
    p.sources.imagery = vec![
        ImageryFile {
            path: "imagery/a.tif".into(),
            attribution: "Contains modified Copernicus Sentinel data 2025".into(),
            licence: "Copernicus Sentinel data, free and open".into(),
            non_commercial: false,
        },
        ImageryFile {
            path: "imagery/b.tif".into(),
            attribution: "EOxCloudless".into(),
            licence: "CC BY-NC-SA 4.0".into(),
            non_commercial: true,
        },
    ];
    p.imagery.brightness = 1.25;
    p.save(dir.path()).unwrap();
    assert_eq!(Project::load(dir.path()).unwrap(), p);
}

#[test]
fn imagery_paths_stay_inside_the_project() {
    for bad in ["/etc/x.tif", "../x.tif", ""] {
        let t = with_imagery("").replace("imagery/a.tif", bad);
        invalid_field(&t, "sources.imagery");
    }
    assert!(Project::parse(&with_imagery("")).is_ok());
}

#[test]
fn attribution_and_licence_are_required() {
    let t = with_imagery("").replace(
        "attribution = \"Contains modified Copernicus Sentinel data 2025\"",
        "attribution = \"\"",
    );
    invalid_field(&t, "attribution");
    let t = with_imagery("").replace(
        "licence = \"Copernicus Sentinel data, free and open\"",
        "licence = \" \"",
    );
    invalid_field(&t, "licence");
}

#[test]
fn attribution_is_one_short_line() {
    let t = with_imagery("").replace("Sentinel data 2025", "Sentinel\\ndata 2025");
    invalid_field(&t, "attribution");
    let long = "x".repeat(501);
    let t = with_imagery("").replace("Sentinel data 2025", &long);
    invalid_field(&t, "attribution");
    let ok = "x".repeat(470);
    let t = with_imagery("").replace("Sentinel data 2025", &ok);
    assert!(Project::parse(&t).is_ok());
}

#[test]
fn nc_licence_must_be_marked() {
    let nc = with_imagery("").replace("Copernicus Sentinel data, free and open", "CC BY-NC-SA 4.0");
    invalid_field(&nc, "non_commercial");
    let marked = format!("{nc}non_commercial = true\n");
    assert!(Project::parse(&marked).is_ok());
    // "Licence" and "Sync" contain the letters but not the token
    let t = with_imagery("").replace(
        "Copernicus Sentinel data, free and open",
        "Licence NCA Sync",
    );
    assert!(Project::parse(&t).is_ok());
    let lower = with_imagery("").replace("Copernicus Sentinel data, free and open", "cc by-nc 4.0");
    invalid_field(&lower, "non_commercial");
}

#[test]
fn brightness_range() {
    let mut p = Project::new("x", 47.0, 12.0, 4000.0);
    for ok in [0.25, 1.0, 4.0] {
        p.imagery.brightness = ok;
        let dir = tempfile::tempdir().unwrap();
        p.save(dir.path()).unwrap();
    }
    for bad in [0.0, 0.24, 4.01, f64::NAN, f64::INFINITY] {
        p.imagery.brightness = bad;
        let dir = tempfile::tempdir().unwrap();
        let err = p.save(dir.path()).unwrap_err();
        assert!(err.to_string().contains("imagery.brightness"), "{err}");
    }
}

#[test]
fn imagery_can_be_set_over_project_set() {
    let p = Project::new("x", 47.0, 12.0, 4000.0);
    let q = p
        .with_value(
            "sources.imagery",
            serde_json::json!([{
                "path": "imagery/a.tif",
                "attribution": "A",
                "licence": "CC BY 4.0"
            }]),
        )
        .unwrap();
    assert_eq!(q.sources.imagery.len(), 1);
    assert!(!q.sources.imagery[0].non_commercial);
    let q = q
        .with_value("imagery.brightness", serde_json::json!(1.5))
        .unwrap();
    assert!((q.imagery.brightness - 1.5).abs() < 1e-9);
}

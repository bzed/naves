// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The MCP server: `naves mcp <dir>` driven over stdio by an rmcp client.

#![cfg(unix)]
#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]

use std::path::{Path, PathBuf};

use base64::Engine as _;
use naves_export::testing::FakeEnv;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};

type Client = RunningService<RoleClient, ()>;

struct Fixture {
    _root: tempfile::TempDir,
    proj: PathBuf,
    client: Client,
}

/// A project with the Königssee DEM and a running `naves mcp` on it, using fake external tools.
async fn start() -> Fixture {
    start_with(false).await
}

/// Like [`start`], with the land cover and OSM crops of the Königssee as sources when `sources` is set.
async fn start_with(sources: bool) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let fakes = FakeEnv::new(&root.path().join("fake")).unwrap();
    let proj = root.path().join("proj");
    std::fs::create_dir_all(proj.join("dem")).unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/dem/koenigssee.tif");
    std::fs::copy(fixture, proj.join("dem/koenigssee.tif")).unwrap();
    let mut p = naves_core::project::Project::new("Test", 47.55, 12.975, 1500.0);
    p.grid.terrain_size = 256;
    p.grid.land_size = 32;
    p.sources.dem = vec!["dem/koenigssee.tif".into()];
    if sources {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        for (dir, name) in [
            ("landcover", "koenigssee.tif"),
            ("osm", "koenigssee.osm.pbf"),
        ] {
            std::fs::create_dir_all(proj.join(dir)).unwrap();
            std::fs::copy(fixtures.join(dir).join(name), proj.join(dir).join(name)).unwrap();
        }
        p.sources.land_cover = vec!["landcover/koenigssee.tif".into()];
        p.sources.osm = vec!["osm/koenigssee.osm.pbf".into()];
    }
    p.save(&proj).unwrap();

    let home = root.path().to_path_buf();
    let command = tokio::process::Command::new(env!("CARGO_BIN_EXE_naves")).configure(|cmd| {
        cmd.arg("mcp")
            .arg(&proj)
            .env_clear()
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_CACHE_HOME", home.join(".cache"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .env("PATH", home.join("no-bin"));
        for (key, value) in fakes.env() {
            cmd.env(key, value);
        }
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            cmd.env("LLVM_PROFILE_FILE", profile);
        }
    });
    let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
    Fixture {
        _root: root,
        proj,
        client,
    }
}

async fn call(f: &Fixture, tool: &'static str, args: Value) -> CallToolResult {
    let params = CallToolRequestParams::new(tool).with_arguments(args.as_object().unwrap().clone());
    f.client.call_tool(params).await.unwrap()
}

fn text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn lists_tools() {
    let f = start().await;
    let mut names: Vec<String> = f
        .client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "build",
            "heightmap_stats",
            "imagery_options",
            "project_get",
            "project_set",
            "render_topdown"
        ]
    );
}

#[tokio::test]
async fn imagery_options_lists_the_same() {
    use naves_core::heightmap::MapGeo;
    use naves_core::imagery::catalog::{catalog, options_for};
    let f = start().await;
    let result = call(&f, "imagery_options", json!({})).await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    let listed: Value = serde_json::from_str(&text(&result)).unwrap();
    let project = naves_core::project::Project::load(&f.proj).unwrap();
    let window = MapGeo::new(&project).unwrap().window(0.0).unwrap();
    let expected = options_for(&catalog(None).unwrap(), window);
    let got: Vec<&str> = listed["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap())
        .collect();
    let want: Vec<&str> = expected.iter().map(|o| o.source.id.as_str()).collect();
    assert_eq!(got, want);
    let nc = listed["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "eox-cloudless-2025")
        .unwrap();
    assert_eq!(nc["non_commercial"], true);
    assert_eq!(nc["licence"], "CC BY-NC-SA 4.0");
    assert!(listed["area"]["west"].as_f64().unwrap() < listed["area"]["east"].as_f64().unwrap());
}

#[tokio::test]
async fn project_get_returns_the_project() {
    let f = start().await;
    let result = call(&f, "project_get", json!({})).await;
    assert_ne!(result.is_error, Some(true));
    let project: Value = serde_json::from_str(&text(&result)).unwrap();
    assert_eq!(project["name"], "Test");
    assert_eq!(project["grid"]["terrain_size"], 256);
}

#[tokio::test]
async fn set_validates() {
    let f = start().await;
    let before = std::fs::read(f.proj.join("naves.toml")).unwrap();
    let bad = call(
        &f,
        "project_set",
        json!({"path": "scale.horizontal", "value": 3}),
    )
    .await;
    assert_eq!(bad.is_error, Some(true));
    assert!(text(&bad).contains("horizontal"), "{}", text(&bad));
    assert_eq!(
        std::fs::read(f.proj.join("naves.toml")).unwrap(),
        before,
        "naves.toml unchanged"
    );

    let good = call(
        &f,
        "project_set",
        json!({"path": "scale.height", "value": 0.75}),
    )
    .await;
    assert_ne!(good.is_error, Some(true), "{}", text(&good));
    let saved = naves_core::project::Project::load(&f.proj).unwrap();
    assert!((saved.scale.height - 0.75).abs() < 1e-9);
    let returned: Value = serde_json::from_str(&text(&good)).unwrap();
    assert_eq!(returned["scale"]["height"], 0.75);
}

#[tokio::test]
async fn set_rejects_unknown_fields_and_survives() {
    let f = start().await;
    for args in [
        json!({"path": "nope", "value": 1}),
        json!({"path": "", "value": 1}),
        json!({"path": "scale.height", "value": "high"}),
    ] {
        let result = call(&f, "project_set", args).await;
        assert_eq!(result.is_error, Some(true), "{}", text(&result));
    }
    // the server still answers
    assert_ne!(
        call(&f, "project_get", json!({})).await.is_error,
        Some(true)
    );
}

#[tokio::test]
async fn heightmap_stats_describe_the_map() {
    let f = start().await;
    let result = call(&f, "heightmap_stats", json!({})).await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    let stats: Value = serde_json::from_str(&text(&result)).unwrap();
    assert_eq!(stats["size"], 256);
    assert!(stats["min"].as_f64().unwrap() < stats["max"].as_f64().unwrap());
    assert!((stats["map_size_m"].as_f64().unwrap() - 750.0).abs() < 1e-6);
}

#[tokio::test]
async fn render_returns_png() {
    let f = start().await;
    let args = json!({"x0": 0, "y0": 0, "x1": 750, "y1": 750, "px": 300, "overlay": "hillshade"});
    let result = call(&f, "render_topdown", args).await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    let image = result
        .content
        .iter()
        .find_map(|c| c.as_image())
        .expect("image content");
    assert_eq!(image.mime_type, "image/png");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&image.data)
        .unwrap();
    let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).unwrap();
    assert_eq!(decoded.width(), 300);
}

/// The colours of a rendered PNG and how often each occurs.
fn colours(result: &CallToolResult) -> std::collections::BTreeMap<[u8; 3], usize> {
    let image = result.content.iter().find_map(|c| c.as_image()).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&image.data)
        .unwrap();
    let mut seen = std::collections::BTreeMap::new();
    for pixel in image::load_from_memory(&bytes).unwrap().to_rgb8().pixels() {
        *seen.entry(pixel.0).or_insert(0) += 1;
    }
    seen
}

#[tokio::test]
async fn render_topdown_shows_surfaces() {
    let f = start_with(true).await;
    let args = json!({"x0": 0, "y0": 0, "x1": 750, "y1": 750, "px": 300, "overlay": "surface"});
    let result = call(&f, "render_topdown", args).await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    let seen = colours(&result);
    assert!(
        seen.contains_key(&[0x7a, 0x68, 0x4e]),
        "the lake is soil: {seen:?}"
    );
    assert!(
        seen.contains_key(&[0x2e, 0x4d, 0x2a]),
        "trees are forest: {seen:?}"
    );
}

/// Points the project at the Sentinel-2 crop (12.9697–12.9789 °E, 47.5542–47.5600 °N) as its imagery.
async fn use_the_sentinel_crop(f: &Fixture) {
    std::fs::create_dir_all(f.proj.join("imagery")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/imagery/sentinel2_crop.tif"),
        f.proj.join("imagery/s2.tif"),
    )
    .unwrap();
    for (path, value) in [
        ("area.center_lat", json!(47.557)),
        ("area.center_lon", json!(12.9743)),
        ("area.real_extent_m", json!(500)),
        (
            "sources.imagery",
            json!([{
                "path": "imagery/s2.tif",
                "attribution": "Contains modified Copernicus Sentinel data 2025",
                "licence": "Copernicus Sentinel data, free and open"
            }]),
        ),
    ] {
        let result = call(f, "project_set", json!({"path": path, "value": value})).await;
        assert_ne!(result.is_error, Some(true), "{path}: {}", text(&result));
    }
}

#[tokio::test]
async fn render_topdown_shows_imagery() {
    let f = start().await;
    use_the_sentinel_crop(&f).await;
    let args = json!({"x0": 0, "y0": 0, "x1": 250, "y1": 250, "px": 200, "overlay": "imagery"});
    let result = call(&f, "render_topdown", args).await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    let seen = colours(&result);
    assert!(
        seen.len() > 20,
        "a photograph has many colours, not a checkered gap: {} colours",
        seen.len()
    );
}

#[tokio::test]
async fn imagery_overlay_follows_the_file_list() {
    let f = start().await;
    let args = json!({"x0": 0, "y0": 0, "x1": 750, "y1": 750, "px": 50, "overlay": "imagery"});
    let result = call(&f, "render_topdown", args.clone()).await;
    assert_eq!(result.is_error, Some(true));
    assert!(
        text(&result).contains("sources.imagery"),
        "{}",
        text(&result)
    );
    // the same server notices the new list
    use_the_sentinel_crop(&f).await;
    let args = json!({"x0": 0, "y0": 0, "x1": 250, "y1": 250, "px": 50, "overlay": "imagery"});
    let result = call(&f, "render_topdown", args).await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    // and an empty list again
    let result = call(
        &f,
        "project_set",
        json!({"path": "sources.imagery", "value": []}),
    )
    .await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    let args = json!({"x0": 0, "y0": 0, "x1": 250, "y1": 250, "px": 50, "overlay": "imagery"});
    assert_eq!(call(&f, "render_topdown", args).await.is_error, Some(true));
}

#[tokio::test]
async fn surfaces_without_sources_follow_the_rules() {
    let f = start().await;
    let args = json!({"x0": 0, "y0": 0, "x1": 750, "y1": 750, "px": 100, "overlay": "surface"});
    let seen = colours(&call(&f, "render_topdown", args).await);
    assert!(
        seen.keys()
            .all(|c| *c == [0x4a, 0x7a, 0x3a] || *c == [0x8a, 0x87, 0x80]),
        "{seen:?}"
    );
}

#[tokio::test]
async fn render_caps_the_width() {
    let f = start().await;
    let args = json!({"x0": 0, "y0": 0, "x1": 750, "y1": 750, "px": 100_000, "overlay": "height"});
    let result = call(&f, "render_topdown", args).await;
    let image = result
        .content
        .iter()
        .find_map(|c| c.as_image())
        .expect("image content");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&image.data)
        .unwrap();
    assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 2048);
}

#[tokio::test]
async fn render_rejects_bad_bounds() {
    let f = start().await;
    for args in [
        json!({"x0": 10, "y0": 0, "x1": 5, "y1": 750, "px": 100, "overlay": "slope"}),
        json!({"x0": 0, "y0": 0, "x1": 750, "y1": 0, "px": 100, "overlay": "slope"}),
        json!({"x0": 0, "y0": 0, "x1": 750, "y1": 750, "px": 0, "overlay": "slope"}),
        json!({"x0": 0, "y0": 0, "x1": 1e300, "y1": 1e-300, "px": 100, "overlay": "slope"}),
    ] {
        let result = call(&f, "render_topdown", args.clone()).await;
        assert_eq!(result.is_error, Some(true), "{args}: {}", text(&result));
    }
}

#[tokio::test]
async fn build_runs_and_reports_the_paths() {
    let f = start().await;
    let result = call(&f, "build", json!({})).await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    let out: Value = serde_json::from_str(&text(&result)).unwrap();
    assert!(out["mod_dir"].as_str().unwrap().ends_with("@naves_test"));
    assert!(
        out["mission_dir"]
            .as_str()
            .unwrap()
            .ends_with("dayzOffline.test")
    );
    assert!(out["warnings"].is_array());
    assert!(
        f.proj
            .join("build/@naves_test/addons/naves_test.pbo")
            .is_file()
    );
}

async fn highest(f: &Fixture) -> f64 {
    let stats = call(f, "heightmap_stats", json!({})).await;
    let stats: Value = serde_json::from_str(&text(&stats)).unwrap();
    stats.get("max").unwrap().as_f64().unwrap()
}

#[tokio::test]
async fn a_project_edited_outside_mcp_is_noticed() {
    let f = start().await;
    let before = highest(&f).await;
    let mut project = naves_core::project::Project::load(&f.proj).unwrap();
    project.scale.height *= 2.0;
    project.save(&f.proj).unwrap();
    let after = highest(&f).await;
    assert!((after - before * 2.0).abs() < 1.0, "{before} then {after}");
}

#[tokio::test]
async fn parallel_changes_all_arrive() {
    let f = start().await;
    for round in 1..=20_u32 {
        let value = 0.5 + f64::from(round) / 100.0;
        let (a, b) = tokio::join!(
            call(
                &f,
                "project_set",
                json!({"path": "scale.height", "value": value})
            ),
            call(
                &f,
                "project_set",
                json!({"path": "scale.horizontal", "value": value})
            ),
        );
        assert_ne!(a.is_error, Some(true));
        assert_ne!(b.is_error, Some(true));
        let saved = naves_core::project::Project::load(&f.proj).unwrap();
        assert!((saved.scale.height - value).abs() < 1e-9, "round {round}");
        assert!(
            (saved.scale.horizontal - value).abs() < 1e-9,
            "round {round}"
        );
    }
}

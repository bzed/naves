// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The MCP server: `naves mcp <dir>` driven over stdio by an rmcp client.

#![cfg(unix)]
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
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
    // 1500 m on 256 vertices: terrain cells the game client shows
    p.scale.horizontal = 1.0;
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

/// Like [`start`], with the OSM highways of the Königssee as a source; without `tools` no external tool
/// can be found.
async fn start_roads(tools: bool) -> Fixture {
    start_roads_scaled(tools, 1.0).await
}

/// [`start_roads`] with the heights of the ground multiplied by `height`: at 2 the roads are steep enough
/// for findings (where roads meet without a junction piece one lies on top of the other, so the
/// fixture has none of its own at 1).
async fn start_roads_scaled(tools: bool, height: f64) -> Fixture {
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
    // 1500 m on 256 vertices: terrain cells the game client shows
    p.scale.horizontal = 1.0;
    p.scale.height = height;
    p.sources.dem = vec!["dem/koenigssee.tif".into()];
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    std::fs::create_dir_all(proj.join("osm")).unwrap();
    std::fs::copy(
        fixtures.join("osm/koenigssee_roads.osm.pbf"),
        proj.join("osm/roads.osm.pbf"),
    )
    .unwrap();
    p.sources.osm = vec!["osm/roads.osm.pbf".into()];
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
        if tools {
            for (key, value) in fakes.env() {
                cmd.env(key, value);
            }
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
            "imagery_fetch",
            "imagery_options",
            "project_get",
            "project_set",
            "render_topdown",
            "road_add",
            "road_delete",
            "road_modify",
            "road_reset",
            "roads_list",
            "roads_stats",
            "validation_list"
        ]
    );
}

/// The ids of every rule must be in the tool description of `validation_list`, in `mcp.md` and in
/// `troubleshooting.md`: the lists drifted when a rule was added.
#[tokio::test]
async fn every_rule_is_documented_where_rules_are_listed() {
    use naves_core::validate::Rule;
    let f = start().await;
    let tools = f.client.list_all_tools().await.unwrap();
    let description = tools
        .iter()
        .find(|t| t.name == "validation_list")
        .and_then(|t| t.description.as_deref().map(str::to_string))
        .unwrap();
    let manual = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/manual/src");
    let mcp = std::fs::read_to_string(manual.join("mcp.md")).unwrap();
    let troubleshooting = std::fs::read_to_string(manual.join("troubleshooting.md")).unwrap();
    let mut missing = Vec::new();
    for rule in Rule::ALL {
        let id = rule.id();
        for (place, text) in [
            ("the validation_list description", &description),
            ("mcp.md", &mcp),
            ("troubleshooting.md", &troubleshooting),
        ] {
            if !text.contains(id) {
                missing.push(format!("{id} is not in {place}"));
            }
        }
    }
    assert!(missing.is_empty(), "{missing:#?}");
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
async fn imagery_fetch_takes_no_url() {
    let f = start().await;
    let tools = f.client.list_all_tools().await.unwrap();
    let tool = tools.iter().find(|t| t.name == "imagery_fetch").unwrap();
    let schema = serde_json::to_value(&tool.input_schema).unwrap();
    let mut properties: Vec<&str> = schema["properties"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    properties.sort_unstable();
    assert_eq!(
        properties,
        ["from", "id", "to"],
        "ids from the catalogue only: no address"
    );
    // an unknown id is an error that names the ids there are, and the project stays as it was
    let before = std::fs::read(f.proj.join("naves.toml")).unwrap();
    let result = call(
        &f,
        "imagery_fetch",
        json!({"id": "https://example.org/x.tif"}),
    )
    .await;
    assert_eq!(result.is_error, Some(true));
    assert!(
        text(&result).contains("sentinel2-scenes"),
        "{}",
        text(&result)
    );
    assert_eq!(std::fs::read(f.proj.join("naves.toml")).unwrap(), before);
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
    assert!((stats["map_size_m"].as_f64().unwrap() - 1500.0).abs() < 1e-6);
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
    // the lot stage is on in this release: its line is in the result
    assert!(out["lots"].is_string(), "{out}");
    assert!(out.get("smoothing").is_some(), "{out}");
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

// --- roads ---------------------------------------------------------------------------------------------------------

fn png_of(result: &CallToolResult) -> image::RgbaImage {
    let data = result
        .content
        .iter()
        .find_map(|c| c.as_image().map(|i| i.data.clone()))
        .expect("an image in the result");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap();
    image::load_from_memory(&bytes).unwrap().to_rgba8()
}

fn whole_map(extra: &Value) -> Value {
    let mut args = json!({ "x0": 0.0, "y0": 0.0, "x1": 750.0, "y1": 750.0, "px": 375, "overlay": "hillshade" });
    args.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    args
}

#[tokio::test]
async fn render_topdown_without_layers_does_not_generate() {
    // no tool can be found: a render that generated the roads would fail
    let f = start_roads(false).await;
    let result = call(&f, "render_topdown", whole_map(&json!({}))).await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    let _ = png_of(&result);
    let layers_only = call(
        &f,
        "render_topdown",
        whole_map(&json!({ "layers": ["too_steep_to_walk"] })),
    )
    .await;
    assert_ne!(layers_only.is_error, Some(true), "{}", text(&layers_only));
}

#[tokio::test]
async fn render_topdown_with_layers_matches_composited_layers() {
    let f = start_roads(true).await;
    let plain = png_of(&call(&f, "render_topdown", whole_map(&json!({}))).await);
    let with = png_of(
        &call(
            &f,
            "render_topdown",
            whole_map(&json!({ "layers": ["roads", "drivable"] })),
        )
        .await,
    );
    assert_eq!(plain.dimensions(), with.dimensions());
    let changed = plain
        .pixels()
        .zip(with.pixels())
        .filter(|(a, b)| a != b)
        .count();
    assert!(changed > 50, "{changed} pixels differ");
    // the order of the ids does not matter
    let swapped = png_of(
        &call(
            &f,
            "render_topdown",
            whole_map(&json!({ "layers": ["drivable", "roads"] })),
        )
        .await,
    );
    assert_eq!(with, swapped);
    // and the picture is the base with the layers composited over it
    let alone = png_of(
        &call(
            &f,
            "render_topdown",
            whole_map(&json!({ "layers": ["roads"] })),
        )
        .await,
    );
    assert_ne!(alone, with);
}

#[tokio::test]
async fn an_unknown_layer_is_a_tool_error() {
    let f = start().await;
    let result = call(
        &f,
        "render_topdown",
        whole_map(&json!({ "layers": ["roads", "unicorns"] })),
    )
    .await;
    assert_eq!(result.is_error, Some(true));
    let message = text(&result);
    assert!(
        message.contains("unicorns") && message.contains("drivable"),
        "{message}"
    );
}

#[tokio::test]
async fn roads_stats_counts_the_network() {
    let f = start_roads(true).await;
    let result = call(&f, "roads_stats", json!({})).await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    let stats: Value = serde_json::from_str(&text(&result)).unwrap();
    assert!(stats["pieces"].as_u64().unwrap() > 20, "{stats}");
    assert!(stats["stretches"].as_u64().unwrap() >= 1);
    assert!(stats["km_by_family"]["grav"].as_f64().unwrap() > 0.1);
}

#[tokio::test]
async fn validation_list_filters_by_rule() {
    let f = start_roads_scaled(true, 2.0).await;
    let all: Value =
        serde_json::from_str(&text(&call(&f, "validation_list", json!({})).await)).unwrap();
    let all = all["violations"].as_array().unwrap();
    assert_ne!(all.len(), 0);
    for v in all {
        assert!(
            v["rule"].is_string()
                && v["x"].is_number()
                && v["y"].is_number()
                && v["message"].is_string()
        );
    }
    let rule = all[0]["rule"].as_str().unwrap().to_string();
    let only: Value = serde_json::from_str(&text(
        &call(&f, "validation_list", json!({ "rule": rule })).await,
    ))
    .unwrap();
    let only = only["violations"].as_array().unwrap();
    assert!(!only.is_empty() && only.len() <= all.len());
    assert!(only.iter().all(|v| v["rule"] == rule.as_str()));
    // the filter drops what has another rule
    let others = all.iter().filter(|v| v["rule"] != rule.as_str()).count();
    assert_eq!(only.len() + others, all.len());
    // a rule that exists and has no findings gives an empty list; one that does not exist is an error
    let none = call(&f, "validation_list", json!({ "rule": "road_in_water" })).await;
    assert_ne!(none.is_error, Some(true));
    let bad = call(&f, "validation_list", json!({ "rule": "nonsense" })).await;
    assert_eq!(bad.is_error, Some(true));
    assert!(text(&bad).contains("road_not_drivable"));
}

#[tokio::test]
async fn a_project_without_osm_has_empty_stats() {
    let f = start().await;
    let stats: Value =
        serde_json::from_str(&text(&call(&f, "roads_stats", json!({})).await)).unwrap();
    assert_eq!(stats["pieces"], 0);
    assert_eq!(stats["stretches"], 0);
    let list: Value =
        serde_json::from_str(&text(&call(&f, "validation_list", json!({})).await)).unwrap();
    assert_eq!(list["violations"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn roads_without_the_tools_say_which_are_missing() {
    let f = start_roads(false).await;
    let result = call(&f, "roads_stats", json!({})).await;
    assert_eq!(result.is_error, Some(true));
    assert!(
        text(&result).contains("Roads need dayzmake"),
        "{}",
        text(&result)
    );
}

async fn list_roads(f: &Fixture, args: Value) -> (CallToolResult, Value) {
    let result = call(f, "roads_list", args).await;
    let value = serde_json::from_str(&text(&result)).unwrap_or(Value::Null);
    (result, value)
}

#[tokio::test]
async fn roads_list_lists_the_highways_of_a_rectangle() {
    let f = start_roads(false).await;
    let (result, all) = list_roads(&f, json!({"x0": 0, "y0": 0, "x1": 1500, "y1": 1500})).await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    let ways = all["ways"].as_array().unwrap();
    assert!(!ways.is_empty(), "{all}");
    for way in ways {
        assert!(way["id"].is_string(), "{way}");
        assert_eq!(way["origin"], "osm", "{way}");
        assert!(way["tags"]["highway"].is_string(), "{way}");
        assert!(way["points"].as_array().unwrap().len() >= 2, "{way}");
    }
    let (_, quarter) = list_roads(&f, json!({"x0": 0, "y0": 0, "x1": 750, "y1": 750})).await;
    let n = quarter["ways"].as_array().unwrap().len();
    assert!(n > 0 && n < ways.len(), "{n} of {}", ways.len());
}

#[tokio::test]
async fn roads_list_cuts_at_the_limit() {
    let f = start_roads(false).await;
    let rect = json!({"x0": 0, "y0": 0, "x1": 1500, "y1": 1500, "limit": 1});
    let (result, cut) = list_roads(&f, rect).await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    assert_eq!(cut["ways"].as_array().unwrap().len(), 1);
    assert_eq!(cut["more"], true);
    let zero = json!({"x0": 0, "y0": 0, "x1": 1500, "y1": 1500, "limit": 0});
    let (result, _) = list_roads(&f, zero).await;
    assert_eq!(result.is_error, Some(true));
}

#[tokio::test]
async fn roads_list_without_osm_is_empty() {
    let f = start().await;
    let (result, none) = list_roads(&f, json!({"x0": 0, "y0": 0, "x1": 1500, "y1": 1500})).await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    assert_eq!(none, json!({"ways": [], "more": false}));
}

#[tokio::test]
async fn roads_list_rejects_a_bad_rectangle() {
    let f = start_roads(false).await;
    let (result, _) = list_roads(&f, json!({"x0": 100, "y0": 0, "x1": 50, "y1": 10})).await;
    assert_eq!(result.is_error, Some(true));
}

#[tokio::test]
async fn a_damaged_edit_file_is_reported() {
    let f = start_roads(false).await;
    std::fs::create_dir_all(f.proj.join("edits")).unwrap();
    std::fs::write(f.proj.join("edits/ways.toml"), "not toml").unwrap();
    let (result, _) = list_roads(&f, json!({"x0": 0, "y0": 0, "x1": 1500, "y1": 1500})).await;
    assert_eq!(result.is_error, Some(true));
    assert!(
        text(&result).contains("edits/ways.toml"),
        "{}",
        text(&result)
    );
    let png = call(
        &f,
        "render_topdown",
        json!({"x0": 0, "y0": 0, "x1": 1500, "y1": 1500, "px": 64, "overlay": "hillshade"}),
    )
    .await;
    assert_ne!(png.is_error, Some(true), "{}", text(&png));
}

#[tokio::test]
async fn roads_list_follows_edits_made_outside_the_server() {
    use naves_core::edits::{By, ElementId, WayEdits};

    let f = start_roads(false).await;
    let whole = json!({"x0": 0, "y0": 0, "x1": 1500, "y1": 1500});
    // fills the cache
    let (_, before) = list_roads(&f, whole.clone()).await;
    assert!(
        before["ways"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w["origin"] == "osm")
    );

    let project = naves_core::project::Project::load(&f.proj).unwrap();
    let source = naves_export::generate::read_source(&f.proj, &project).unwrap();
    let way = source
        .extract
        .ways
        .iter()
        .find(|w| w.tags.contains_key("highway") && w.nodes.len() >= 2)
        .unwrap();
    let (node, (lat, lon)) = (way.nodes[0], way.points[0]);
    let mut edits = WayEdits::default();
    edits
        .move_node(
            ElementId::Osm(node),
            lat + 0.0001,
            lon,
            Some((lat, lon)),
            By::Ai,
        )
        .unwrap();
    let a = edits.new_node(47.55, 12.975, By::Ai).unwrap();
    let b = edits.new_node(47.551, 12.976, By::Ai).unwrap();
    let tags = [("highway".to_string(), "track".to_string())].into();
    let drawn = edits.add_way(tags, vec![a, b], By::Ai).unwrap();
    edits.save(&f.proj).unwrap();

    let (result, after) = list_roads(&f, whole.clone()).await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    let ways = after["ways"].as_array().unwrap();
    let way_id = way.id.to_string();
    let edited = ways.iter().find(|w| w["id"] == way_id.as_str()).unwrap();
    assert_eq!(edited["origin"], "edited", "{edited}");
    let drawn_id = drawn.to_string();
    let new = ways.iter().find(|w| w["id"] == drawn_id.as_str()).unwrap();
    assert!(new["id"].as_str().unwrap().starts_with('u'), "{new}");
    assert_eq!(new["origin"], "new", "{new}");

    std::fs::write(f.proj.join("edits/ways.toml"), "not toml").unwrap();
    let (result, _) = list_roads(&f, whole).await;
    assert_eq!(result.is_error, Some(true));
    let message = text(&result);
    assert_eq!(message.matches("edits/ways.toml").count(), 1, "{message}");
}

// --- road edits ----------------------------------------------------------------------------------------------------

async fn all_roads(f: &Fixture) -> Vec<Value> {
    let whole = json!({"x0": 0, "y0": 0, "x1": 1500, "y1": 1500});
    let (result, all) = list_roads(f, whole).await;
    assert_ne!(result.is_error, Some(true), "{}", text(&result));
    all.get("ways").unwrap().as_array().unwrap().clone()
}

fn way_edits(f: &Fixture) -> naves_core::edits::WayEdits {
    naves_core::edits::WayEdits::load(&f.proj).unwrap()
}

fn track() -> Value {
    json!({"highway": "track"})
}

fn json_of(result: &CallToolResult) -> Value {
    assert_ne!(result.is_error, Some(true), "{}", text(result));
    serde_json::from_str(&text(result)).unwrap()
}

#[tokio::test]
async fn road_add_adds_a_way_and_writes_the_edit() {
    use naves_core::edits::{By, WayAction};

    let f = start_roads(false).await;
    let args = json!({"points": [[100, 100], [200, 150], [300, 120]], "tags": track()});
    let added = json_of(&call(&f, "road_add", args).await);
    let id = added["id"].as_str().unwrap().to_string();
    assert!(id.starts_with('u'), "{added}");
    assert_eq!(added["origin"], "new");
    assert_eq!(added["points"].as_array().unwrap().len(), 3);

    let edits = way_edits(&f);
    assert_eq!(edits.ways.len(), 1);
    let edit = &edits.ways[0];
    assert_eq!(edit.action, WayAction::Add);
    assert_eq!(edit.by, By::Ai);
    assert_eq!(edit.id.to_string(), id);
    assert_eq!(edit.tags.get("highway").map(String::as_str), Some("track"));
    assert_eq!(edit.tags.get("naves:keep").map(String::as_str), Some("yes"));

    let (_, listed) = list_roads(&f, json!({"x0": 50, "y0": 50, "x1": 350, "y1": 200})).await;
    assert!(
        listed["ways"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["id"] == id.as_str())
    );
}

#[tokio::test]
async fn road_add_joins_an_existing_node() {
    use naves_core::edits::ElementId;

    let f = start_roads(false).await;
    let ways = all_roads(&f).await;
    let points = ways[0]["points"].as_array().unwrap();
    let (x, y) = (
        points[0][0].as_f64().unwrap(),
        points[0][1].as_f64().unwrap(),
    );
    let (x2, y2) = (
        points[1][0].as_f64().unwrap(),
        points[1][1].as_f64().unwrap(),
    );
    let args = json!({"points": [[x + 0.004, y], [x2, y2 + 0.003]], "tags": track()});
    json_of(&call(&f, "road_add", args).await);
    let edits = way_edits(&f);
    let nodes = &edits.ways[0].nodes;
    assert!(matches!(nodes[0], ElementId::Osm(_)), "{nodes:?}");
    assert!(matches!(nodes[1], ElementId::Osm(_)), "{nodes:?}");
    assert!(edits.nodes.is_empty(), "{:?}", edits.nodes);
}

#[tokio::test]
async fn road_modify_changes_the_tags_and_keeps_the_line() {
    use naves_core::edits::WayAction;

    let f = start_roads(false).await;
    let before = all_roads(&f).await.remove(0);
    let id = before["id"].as_str().unwrap();
    let args = json!({"id": id, "tags": track()});
    let after = json_of(&call(&f, "road_modify", args).await);
    assert_eq!(after["points"], before["points"]);
    assert_eq!(after["origin"], "edited");
    assert_eq!(after["tags"]["highway"], "track");
    let edits = way_edits(&f);
    assert_eq!(edits.ways.len(), 1);
    assert_eq!(edits.ways[0].action, WayAction::Modify);
    assert!(edits.ways[0].base.is_some());
}

#[tokio::test]
async fn road_modify_without_changes_is_an_error() {
    let f = start_roads(false).await;
    let id = all_roads(&f).await[0]["id"].as_str().unwrap().to_string();
    let result = call(&f, "road_modify", json!({"id": id})).await;
    assert_eq!(result.is_error, Some(true));
    assert!(
        text(&result).contains("nothing to change"),
        "{}",
        text(&result)
    );
    assert!(!f.proj.join("edits/ways.toml").exists());
}

#[tokio::test]
async fn road_delete_and_reset_bring_the_way_back() {
    let f = start_roads(false).await;
    let id = all_roads(&f).await[0]["id"].as_str().unwrap().to_string();
    let deleted = json_of(&call(&f, "road_delete", json!({"id": id})).await);
    assert_eq!(deleted, json!({"deleted": id}));
    assert!(all_roads(&f).await.iter().all(|w| w["id"] != id.as_str()));

    let back = json_of(&call(&f, "road_reset", json!({"id": id})).await);
    assert_eq!(back["id"], id.as_str());
    assert_eq!(back["origin"], "osm");
    assert!(all_roads(&f).await.iter().any(|w| w["id"] == id.as_str()));
    assert_eq!(way_edits(&f).ways.len(), 0);
}

#[tokio::test]
async fn road_reset_of_a_drawn_way_removes_it() {
    let f = start_roads(false).await;
    let args = json!({"points": [[100, 100], [200, 150]], "tags": track()});
    let id = json_of(&call(&f, "road_add", args).await)["id"]
        .as_str()
        .unwrap()
        .to_string();
    let removed = json_of(&call(&f, "road_reset", json!({"id": id})).await);
    assert_eq!(removed, json!({"removed": id}));
    assert_eq!(way_edits(&f).ways.len(), 0);
}

#[tokio::test]
async fn road_tools_refuse_bad_ids_and_leave_the_file() {
    let f = start_roads(false).await;
    let args = json!({"points": [[100, 100], [200, 150]], "tags": track()});
    json_of(&call(&f, "road_add", args).await);
    let ways = all_roads(&f).await;
    let deleted = ways[0]["id"].as_str().unwrap().to_string();
    let untouched = ways[1]["id"].as_str().unwrap().to_string();
    json_of(&call(&f, "road_delete", json!({"id": deleted})).await);
    let file = f.proj.join("edits/ways.toml");
    let bytes = std::fs::read(&file).unwrap();

    for id in ["u999", "0", "abc", "999999999999"] {
        for tool in ["road_modify", "road_delete", "road_reset"] {
            let args = if tool == "road_modify" {
                json!({"id": id, "tags": track()})
            } else {
                json!({"id": id})
            };
            let result = call(&f, tool, args).await;
            assert_eq!(result.is_error, Some(true), "{tool} {id}");
        }
    }
    let result = call(&f, "road_modify", json!({"id": deleted, "tags": track()})).await;
    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).contains("is deleted"), "{}", text(&result));
    let result = call(&f, "road_delete", json!({"id": deleted})).await;
    assert_eq!(result.is_error, Some(true));
    let result = call(&f, "road_reset", json!({"id": untouched})).await;
    assert_eq!(result.is_error, Some(true));
    assert!(text(&result).contains("has no edit"), "{}", text(&result));
    assert_eq!(std::fs::read(&file).unwrap(), bytes);
}

#[tokio::test]
async fn road_tools_name_the_limit() {
    let f = start_roads(false).await;
    let many: Vec<[f64; 2]> = (0..2001)
        .map(|i| [10.0 + f64::from(i) * 0.5, 100.0])
        .collect();
    let long = "x".repeat(256);
    let cases = [
        (json!({"points": many, "tags": track()}), "too many nodes"),
        (
            json!({"points": [[-1, 10], [20, 20]], "tags": track()}),
            "outside the map",
        ),
        (
            json!({"points": [[10, 10], [20, 20]], "tags": {"highway": long}}),
            "too long",
        ),
        (
            json!({"points": [[10, 10], [20, 20]], "tags": {"surface": "dirt"}}),
            "highway tag",
        ),
    ];
    for (args, expected) in cases {
        let result = call(&f, "road_add", args).await;
        assert_eq!(result.is_error, Some(true), "{expected}");
        assert!(
            text(&result).contains(expected),
            "{expected}: {}",
            text(&result)
        );
        assert!(!f.proj.join("edits/ways.toml").exists(), "{expected}");
    }
}

#[tokio::test]
async fn a_damaged_edit_file_stays_after_a_refused_add() {
    let f = start_roads(false).await;
    std::fs::create_dir_all(f.proj.join("edits")).unwrap();
    std::fs::write(f.proj.join("edits/ways.toml"), "not toml").unwrap();
    let args = json!({"points": [[100, 100], [200, 150]], "tags": track()});
    let result = call(&f, "road_add", args).await;
    assert_eq!(result.is_error, Some(true));
    assert!(
        text(&result).contains("edits/ways.toml"),
        "{}",
        text(&result)
    );
    assert_eq!(
        std::fs::read(f.proj.join("edits/ways.toml")).unwrap(),
        b"not toml"
    );
}

#[tokio::test]
async fn parallel_road_adds_all_arrive() {
    let f = start_roads(false).await;
    let add = |i: u32| {
        let y = 100.0 + f64::from(i) * 50.0;
        call(
            &f,
            "road_add",
            json!({"points": [[100, y], [300, y + 10.0]], "tags": track()}),
        )
    };
    let r = tokio::join!(
        add(0),
        add(1),
        add(2),
        add(3),
        add(4),
        add(5),
        add(6),
        add(7)
    );
    let results = [r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7];
    let mut ids: Vec<String> = results
        .iter()
        .map(|r| json_of(r)["id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 8);
    assert_eq!(way_edits(&f).ways.len(), 8);
}

fn total_km(stats: &Value) -> f64 {
    stats["km_by_family"]
        .as_object()
        .unwrap()
        .values()
        .map(|v| v.as_f64().unwrap())
        .sum()
}

#[tokio::test]
async fn roads_follow_an_edit() {
    let f = start_roads(true).await;
    let before = json_of(&call(&f, "roads_stats", json!({})).await);
    // only tracks make kilometres of road here; the longest of them
    let ways = all_roads(&f).await;
    let longest = ways
        .iter()
        .filter(|w| w["tags"]["highway"] == "track")
        .max_by_key(|w| w["points"].as_array().unwrap().len())
        .unwrap();
    let id = longest["id"].as_str().unwrap();
    json_of(&call(&f, "road_delete", json!({"id": id})).await);
    let after = json_of(&call(&f, "roads_stats", json!({})).await);
    assert!(
        total_km(&after) < total_km(&before),
        "{} vs {}",
        total_km(&after),
        total_km(&before)
    );
}

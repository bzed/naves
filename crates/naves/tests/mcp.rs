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
            "project_get",
            "project_set",
            "render_topdown"
        ]
    );
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

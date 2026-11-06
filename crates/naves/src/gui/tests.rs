// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Widget tests with `egui_kittest`: the GUI is driven through its accessibility tree.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use naves_core::project::Project;
use naves_export::testing::FakeEnv;

use super::*;

/// A project with the Königssee DEM below `root`.
fn project(root: &Path) -> PathBuf {
    let dir = root.join("proj");
    std::fs::create_dir_all(dir.join("dem")).unwrap();
    let mut p = Project::new("Test", 47.55, 12.975, 1500.0);
    p.grid.terrain_size = 256;
    p.grid.land_size = 32;
    p.sources.dem = vec!["dem/koenigssee.tif".into()];
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/dem/koenigssee.tif");
    std::fs::copy(fixture, dir.join("dem/koenigssee.tif")).unwrap();
    p.save(&dir).unwrap();
    dir
}

fn app(root: &Path, dir: &Path) -> NavesApp {
    let tools = FakeEnv::new(&root.join("fake")).unwrap().tools();
    NavesApp::open(dir, Box::new(move || Ok(tools.clone()))).unwrap()
}

/// Steps the harness until `done` holds or a generous deadline passes.
fn step_until(harness: &mut Harness<'_>, done: impl Fn(&Harness<'_>) -> bool) {
    let deadline = Instant::now() + Duration::from_mins(1);
    while !done(harness) {
        assert!(Instant::now() < deadline, "timed out waiting for the GUI");
        harness.step();
        std::thread::yield_now();
    }
}

#[test]
fn the_title_names_the_project() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    assert_eq!(app(root.path(), &dir).title(), "Náves – Test");
}

#[test]
fn editing_the_name_saves_naves_toml() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    let mut harness = Harness::new_ui(|ui| app.show(ui));
    harness.run_steps(4);
    let field = harness.get_by_role(egui::accesskit::Role::TextInput);
    field.focus();
    field.type_text("X");
    harness.run_steps(8);
    let saved = Project::load(&dir).unwrap();
    assert_eq!(saved.name, "TestX");
}

#[test]
fn the_build_button_shows_progress_and_the_result() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    let mut harness = Harness::new_ui(|ui| app.show(ui));
    harness.run_steps(4);
    harness.get_by_label("Build").click();
    step_until(&mut harness, |h| {
        h.query_by_label_contains("Mod: ").is_some()
    });
    assert!(harness.query_by_label_contains("Mission: ").is_some());
    assert!(
        dir.join("build/@naves_test/addons/naves_test.pbo")
            .is_file()
    );
}

#[test]
fn a_failed_build_is_shown_in_the_log() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = NavesApp::open(
        &dir,
        Box::new(|| Err("armake2 not found; set ARMAKE2".to_string())),
    )
    .unwrap();
    let mut harness = Harness::new_ui(|ui| app.show(ui));
    harness.run_steps(4);
    harness.get_by_label("Build").click();
    step_until(&mut harness, |h| {
        h.query_by_label_contains("armake2 not found").is_some()
    });
}

#[test]
fn a_project_without_dem_says_what_to_do() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("empty-proj");
    std::fs::create_dir_all(&dir).unwrap();
    Project::new("Empty", 1.0, 2.0, 1000.0).save(&dir).unwrap();
    let mut app = app(root.path(), &dir);
    let mut harness = Harness::new_ui(|ui| app.show(ui));
    harness.run_steps(4);
    assert!(
        harness
            .query_by_label_contains("Add elevation files")
            .is_some()
    );
}

#[test]
fn the_land_grid_follows_the_terrain_grid() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    let mut harness = Harness::new_ui(|ui| app.show(ui));
    harness.run_steps(4);
    assert!(harness.query_by_label("32 × 32 land cells").is_some());
    harness.get_by_role(egui::accesskit::Role::ComboBox).click();
    harness.run_steps(4);
    harness.get_by_label("512").click();
    harness.run_steps(8);
    let saved = Project::load(&dir).unwrap();
    assert_eq!((saved.grid.terrain_size, saved.grid.land_size), (512, 64));
    assert!(harness.query_by_label("64 × 64 land cells").is_some());
}

/// Steps the app without a window until its heightmap job has ended.
fn wait_for_the_job(app: &mut NavesApp) {
    let deadline = Instant::now() + Duration::from_mins(1);
    while app.job.is_some() {
        assert!(Instant::now() < deadline, "timed out waiting for the job");
        app.poll_background();
        std::thread::yield_now();
    }
}

#[test]
fn renaming_does_not_reload_the_elevation_data() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    wait_for_the_job(&mut app);
    app.form.project.name = "Renamed".to_string();
    app.settings_changed();
    assert!(app.job.is_none(), "the name has nothing to do with the map");
    assert!(matches!(app.heightmap, HeightmapState::Ready(_)));
    assert_eq!(Project::load(&dir).unwrap().name, "Renamed");
}

#[test]
fn only_one_heightmap_job_runs_and_the_last_setting_wins() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    wait_for_the_job(&mut app);
    for (i, height) in [0.6, 0.7, 0.8].into_iter().enumerate() {
        app.form.project.scale.height = height;
        app.settings_changed();
        assert!(app.job.is_some());
        assert_eq!(
            app.reload_wanted,
            i > 0,
            "later changes wait for the running job"
        );
    }
    wait_for_the_job(&mut app);
    assert!(!app.reload_wanted);
    let HeightmapState::Ready(data) = &app.heightmap else {
        panic!("no heightmap");
    };
    // a fresh computation with the saved project gives the same map
    let saved = Project::load(&dir).unwrap();
    assert!((saved.scale.height - 0.8).abs() < 1e-9);
    let files = [dir.join("dem/koenigssee.tif")];
    let dem = naves_core::dem::Dem::open(&files).unwrap();
    let expected = naves_core::heightmap::build_heightmap(&saved, &dem).unwrap();
    assert_eq!(data.heightmap.min_max(), expected.min_max());
}

/// Adds the land cover and OSM crops to the project in `dir` and lists them as sources.
fn add_sources(dir: &Path) {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut p = Project::load(dir).unwrap();
    for (sub, name) in [
        ("landcover", "koenigssee.tif"),
        ("osm", "koenigssee.osm.pbf"),
    ] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
        std::fs::copy(fixtures.join(sub).join(name), dir.join(sub).join(name)).unwrap();
    }
    p.sources.land_cover = vec!["landcover/koenigssee.tif".into()];
    p.sources.osm = vec!["osm/koenigssee.osm.pbf".into()];
    p.save(dir).unwrap();
}

#[test]
fn land_cover_files_are_listed() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    add_sources(&dir);
    let mut app = app(root.path(), &dir);
    let mut harness = Harness::new_ui(|ui| app.show(ui));
    harness.run_steps(4);
    assert!(harness.query_by_label("Land cover files").is_some());
    assert!(harness.query_by_label("landcover/koenigssee.tif").is_some());
    assert!(harness.query_by_label("OpenStreetMap files").is_some());
    assert!(harness.query_by_label("osm/koenigssee.osm.pbf").is_some());
}

#[test]
fn a_project_without_sources_says_what_the_ground_follows() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    let mut harness = Harness::new_ui(|ui| app.show(ui));
    harness.run_steps(4);
    assert!(
        harness
            .query_by_label_contains("slope and altitude")
            .is_some()
    );
}

#[test]
fn changing_a_source_list_reloads_the_map() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    wait_for_the_job(&mut app);
    add_sources(&dir);
    app.form.project.sources.land_cover = vec!["landcover/koenigssee.tif".into()];
    app.settings_changed();
    assert!(
        app.job.is_some(),
        "the ground depends on the land cover files"
    );
}

#[test]
fn the_surface_overlay_draws_once_the_map_is_ready() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    add_sources(&dir);
    let mut app = app(root.path(), &dir);
    let mut harness = Harness::new_ui(|ui| app.show(ui));
    harness.run_steps(4);
    harness.get_by_label("Surface").click();
    step_until(&mut harness, |h| h.query_by_label("Soil").is_some());
    assert!(
        harness.query_by_label("Forest").is_some(),
        "the legend names the classes"
    );
}

/// Adds the Sentinel-2 crop to the project in `dir` as imagery with the given licence.
fn add_imagery(dir: &Path, licence: &str, non_commercial: bool) {
    let mut p = Project::load(dir).unwrap();
    std::fs::create_dir_all(dir.join("imagery")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/imagery/sentinel2_crop.tif"),
        dir.join("imagery/s2.tif"),
    )
    .unwrap();
    p.sources.imagery = vec![naves_core::project::ImageryFile {
        path: "imagery/s2.tif".into(),
        attribution: "Contains modified Copernicus Sentinel data 2025".to_string(),
        licence: licence.to_string(),
        non_commercial,
    }];
    p.save(dir).unwrap();
}

#[test]
fn imagery_files_are_listed() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    add_imagery(&dir, "Copernicus Sentinel data, free and open", false);
    let mut app = app(root.path(), &dir);
    let mut harness = Harness::new_ui(|ui| app.show(ui));
    harness.run_steps(4);
    assert!(harness.query_by_label("Satellite imagery files").is_some());
    assert!(harness.query_by_label("imagery/s2.tif").is_some());
    assert!(
        harness
            .query_by_label("Copernicus Sentinel data, free and open")
            .is_some()
    );
    assert!(harness.query_by_label("Non-commercial map").is_none());
}

#[test]
fn non_commercial_is_shown() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    add_imagery(&dir, "CC BY-NC-SA 4.0", true);
    let mut app = app(root.path(), &dir);
    let mut harness = Harness::new_ui(|ui| app.show(ui));
    harness.run_steps(4);
    assert!(harness.query_by_label("Non-commercial map").is_some());
    assert!(
        harness
            .query_by_label_contains("CC BY-NC-SA 4.0, non-commercial")
            .is_some(),
        "the file's licence says it in words"
    );
}

#[test]
fn a_project_without_imagery_says_what_the_satellite_image_follows() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    let mut harness = Harness::new_ui(|ui| app.show(ui));
    harness.run_steps(4);
    assert!(
        harness
            .query_by_label_contains("painted from the ground classes")
            .is_some()
    );
}

#[test]
fn changing_the_imagery_list_reloads_the_map() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    wait_for_the_job(&mut app);
    add_imagery(&dir, "CC BY 4.0", false);
    app.form.project.sources.imagery = Project::load(&dir).unwrap().sources.imagery;
    app.settings_changed();
    assert!(
        app.job.is_some(),
        "the imagery overlay depends on the files"
    );
}

#[test]
fn the_imagery_overlay_explains_a_gap_and_a_missing_list() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    let mut harness = Harness::new_ui(|ui| app.show(ui));
    harness.run_steps(4);
    harness.get_by_label("Imagery").click();
    step_until(&mut harness, |h| {
        h.query_by_label_contains("sources.imagery").is_some()
    });
}

#[test]
fn the_imagery_overlay_draws_once_the_map_is_ready() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    add_imagery(&dir, "CC BY 4.0", false);
    let mut app = app(root.path(), &dir);
    let mut harness = Harness::new_ui(|ui| app.show(ui));
    harness.run_steps(4);
    harness.get_by_label("Imagery").click();
    step_until(&mut harness, |h| {
        h.query_by_label_contains("checkered").is_some()
    });
}

#[test]
fn build_warnings_are_shown_in_the_log() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    // land cover for the western half of the map only
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/landcover/koenigssee_west.tif");
    std::fs::create_dir_all(dir.join("landcover")).unwrap();
    std::fs::copy(fixture, dir.join("landcover/koenigssee_west.tif")).unwrap();
    let mut p = Project::load(&dir).unwrap();
    p.sources.land_cover = vec!["landcover/koenigssee_west.tif".into()];
    p.save(&dir).unwrap();
    let mut app = app(root.path(), &dir);
    let mut harness = Harness::new_ui(|ui| app.show(ui));
    harness.run_steps(4);
    harness.get_by_label("Build").click();
    step_until(&mut harness, |h| {
        h.query_by_label_contains("not covered by the land cover")
            .is_some()
    });
}

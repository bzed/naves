// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Widget tests with `egui_kittest`: the GUI is driven through its accessibility tree.

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

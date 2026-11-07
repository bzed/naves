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

mod dock;
mod layers;
mod levels;
mod prefs_window;
mod rail;
mod selection;
mod shortcuts;
mod stack;
mod viewport;

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

/// The window of a normal session: wide and tall enough that nothing folds.
fn gui_harness(app: &mut NavesApp) -> Harness<'_> {
    Harness::builder()
        .with_size((1180.0, 760.0))
        .build_ui(|ui| app.show(ui))
}

/// The same, with the app inside the harness so that tests can look at it between steps.
fn state_harness(app: NavesApp) -> Harness<'static, NavesApp> {
    Harness::builder()
        .with_size((1180.0, 760.0))
        .build_ui_state(|ui, app: &mut NavesApp| app.show(ui), app)
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
    let mut harness = gui_harness(&mut app);
    harness.run_steps(4);
    // the name is the first text field of the panel
    let field = harness
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .next()
        .unwrap();
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
    let mut harness = gui_harness(&mut app);
    harness.run_steps(4);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Build")
        .click();
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
    let mut harness = gui_harness(&mut app);
    harness.run_steps(4);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Build")
        .click();
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
    let mut harness = gui_harness(&mut app);
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
    let mut harness = gui_harness(&mut app);
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
    let mut harness = gui_harness(&mut app);
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
    let mut harness = gui_harness(&mut app);
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
    let mut harness = gui_harness(&mut app);
    harness.run_steps(4);
    harness.get_by_label("Ground").click();
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
    let mut harness = gui_harness(&mut app);
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
    let mut harness = gui_harness(&mut app);
    harness.run_steps(4);
    assert!(harness.query_by_label("Non-commercial map").is_some());
    assert!(
        harness
            .query_all_by_label_contains("CC BY-NC-SA 4.0, non-commercial")
            .count()
            >= 1,
        "the file's licence says it in words"
    );
}

#[test]
fn a_project_without_imagery_says_what_the_satellite_image_follows() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    let mut harness = gui_harness(&mut app);
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
fn the_imagery_brightness_is_set_in_the_panel() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    {
        let mut harness = gui_harness(&mut app);
        harness.run_steps(4);
        assert!(
            harness.query_by_label("Imagery brightness").is_some(),
            "the setting is in the project panel"
        );
    }
    wait_for_the_job(&mut app);
    // a change is saved and the picture is drawn again, since it looks different
    app.form.project.imagery.brightness = 2.0;
    app.settings_changed();
    assert!((Project::load(&dir).unwrap().imagery.brightness - 2.0).abs() < 1e-9);
    assert!(
        app.map_texture_is_empty(),
        "the map is drawn again with the new brightness"
    );
    assert!(app.job.is_none(), "no data is read again for a brightness");
}

#[test]
fn the_imagery_overlay_explains_a_gap_and_a_missing_list() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    let mut harness = gui_harness(&mut app);
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
    let mut harness = gui_harness(&mut app);
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
    let mut harness = gui_harness(&mut app);
    harness.run_steps(4);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Build")
        .click();
    step_until(&mut harness, |h| {
        h.query_by_label_contains("not covered by the land cover")
            .is_some()
    });
}

// ---- choosing and fetching imagery ----

use naves_export::imagery_fetch::{FetchError, FetchRequest, Http};

use super::imagery::{OwnFile, option_line, options_for_project};

/// A WMS that answers every request with a picture of the size asked for; `gate` holds it back.
struct FakeWms {
    gate: Option<Arc<std::sync::atomic::AtomicBool>>,
}

impl Http for FakeWms {
    fn get(&self, url: &str, _max: u64, out: &mut dyn std::io::Write) -> Result<(), FetchError> {
        if let Some(gate) = &self.gate {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !gate.load(std::sync::atomic::Ordering::SeqCst) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let value = |key: &str| -> u32 {
            url.split(['?', '&'])
                .find_map(|p| p.strip_prefix(&format!("{key}=")))
                .and_then(|v| v.parse().ok())
                .unwrap()
        };
        let (width, height) = (value("width"), value("height"));
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([90, 120, 60, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        out.write_all(png.get_ref()).map_err(|e| FetchError::Http {
            url: url.to_string(),
            why: e.to_string(),
        })
    }

    fn post_json(&self, url: &str, _body: &str, _max: u64) -> Result<Vec<u8>, FetchError> {
        Err(FetchError::Http {
            url: url.to_string(),
            why: "no search in this test".to_string(),
        })
    }
}

fn fetching_app(
    root: &Path,
    dir: &Path,
    gate: Option<Arc<std::sync::atomic::AtomicBool>>,
) -> NavesApp {
    let mut app = app(root, dir).with_http(Arc::new(FakeWms { gate }));
    app.user_catalog = None;
    app
}

fn wait_for_the_fetch(app: &mut NavesApp) {
    let deadline = Instant::now() + Duration::from_mins(1);
    while app.fetch_running() {
        assert!(Instant::now() < deadline, "timed out waiting for the fetch");
        app.poll_background();
        std::thread::yield_now();
    }
}

fn log_text(app: &NavesApp) -> String {
    app.log
        .iter()
        .map(|line| match line {
            Line::Info(t) | Line::Warning(t) | Line::Error(t) | Line::Result(t) => t.as_str(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn options_follow_the_area() {
    let bavaria = Project::new("B", 47.585, 12.965, 12_800.0);
    let ids = |p: &Project| -> Vec<String> {
        options_for_project(p, None)
            .unwrap()
            .into_iter()
            .map(|o| o.source.id)
            .collect()
    };
    assert!(ids(&bavaria).contains(&"bayern-dop40".to_string()));
    let peru = Project::new("P", -13.1, -72.0, 5000.0);
    assert!(!ids(&peru).contains(&"bayern-dop40".to_string()));
    assert!(ids(&peru).contains(&"sentinel2-scenes".to_string()));
}

#[test]
fn non_commercial_mark_is_text() {
    let project = Project::new("B", 47.585, 12.965, 12_800.0);
    let options = options_for_project(&project, None).unwrap();
    let line = |id: &str| option_line(options.iter().find(|o| o.source.id == id).unwrap());
    assert!(
        line("eox-cloudless-2025").contains("non-commercial"),
        "{}",
        line("eox-cloudless-2025")
    );
    assert!(line("eox-cloudless-2025").contains("CC BY-NC-SA 4.0"));
    assert!(!line("eox-cloudless-2017").contains("non-commercial"));
    assert!(
        line("sentinel2-scenes").contains("10 m"),
        "{}",
        line("sentinel2-scenes")
    );
    assert!(line("sentinel2-scenes").contains("whole map"));
}

#[test]
fn fetch_job_reports_then_lists_the_file() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = fetching_app(root.path(), &dir, None);
    wait_for_the_job(&mut app);
    app.start_fetch("eox-cloudless-2017", FetchRequest::default())
        .unwrap();
    assert!(app.fetch_running());
    wait_for_the_fetch(&mut app);
    assert!(dir.join("imagery/eox-cloudless-2017.tif").is_file());
    let saved = Project::load(&dir).unwrap().sources.imagery;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].licence, "CC BY 4.0");
    // the form follows the file, so that the next edit does not undo the fetch
    assert_eq!(app.form.project.sources.imagery, saved);
    let log = log_text(&app);
    assert!(
        log.contains("Fetching eox-cloudless-2017"),
        "progress is reported: {log}"
    );
    assert!(log.contains("imagery/eox-cloudless-2017.tif"), "{log}");
    // the map reloads with the new file
    assert!(app.job.is_some() || matches!(app.heightmap, HeightmapState::Ready(_)));
}

#[test]
fn cancel_leaves_no_file() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut app = fetching_app(root.path(), &dir, Some(Arc::clone(&gate)));
    let before = std::fs::read(dir.join("naves.toml")).unwrap();
    app.start_fetch("eox-cloudless-2017", FetchRequest::default())
        .unwrap();
    app.cancel_fetch();
    gate.store(true, std::sync::atomic::Ordering::SeqCst);
    wait_for_the_fetch(&mut app);
    let files: Vec<_> = std::fs::read_dir(dir.join("imagery"))
        .map(|d| d.map(|e| e.unwrap().file_name()).collect())
        .unwrap_or_default();
    assert!(files.is_empty(), "{files:?}");
    assert_eq!(std::fs::read(dir.join("naves.toml")).unwrap(), before);
    assert!(log_text(&app).contains("cancelled"), "{}", log_text(&app));
    assert!(!app.fetch_running());
}

#[test]
fn one_job_at_a_time() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut app = fetching_app(root.path(), &dir, Some(Arc::clone(&gate)));
    app.start_fetch("eox-cloudless-2017", FetchRequest::default())
        .unwrap();
    let second = app.start_fetch("eox-cloudless-2025", FetchRequest::default());
    assert!(
        second.unwrap_err().contains("already"),
        "a second fetch waits for the first"
    );
    gate.store(true, std::sync::atomic::Ordering::SeqCst);
    wait_for_the_fetch(&mut app);
    assert!(
        app.start_fetch("no-such-source", FetchRequest::default())
            .unwrap_err()
            .contains("sentinel2-scenes")
    );
}

#[test]
fn order_buttons_change_the_project() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    add_imagery(&dir, "CC BY 4.0", false);
    let mut app = app(root.path(), &dir);
    wait_for_the_job(&mut app);
    std::fs::copy(dir.join("imagery/s2.tif"), dir.join("imagery/second.tif")).unwrap();
    app.add_own_imagery(&OwnFile {
        path: "imagery/second.tif".into(),
        attribution: "Second".into(),
        licence: "CC BY 4.0".into(),
        non_commercial: false,
    })
    .unwrap();
    let paths = |p: &Project| -> Vec<String> {
        p.sources
            .imagery
            .iter()
            .map(|f| f.path.display().to_string())
            .collect()
    };
    assert_eq!(
        paths(&Project::load(&dir).unwrap()),
        ["imagery/s2.tif", "imagery/second.tif"]
    );
    app.move_imagery(1, true);
    assert_eq!(
        paths(&Project::load(&dir).unwrap()),
        ["imagery/second.tif", "imagery/s2.tif"]
    );
    app.move_imagery(1, false); // the last one cannot go down: nothing changes
    assert_eq!(
        paths(&Project::load(&dir).unwrap()),
        ["imagery/second.tif", "imagery/s2.tif"]
    );
    // removing takes the file off the list and leaves it on disk, and says so
    app.remove_imagery(0);
    assert_eq!(paths(&Project::load(&dir).unwrap()), ["imagery/s2.tif"]);
    assert!(dir.join("imagery/second.tif").is_file());
    assert!(
        log_text(&app).contains("stays in the project folder"),
        "{}",
        log_text(&app)
    );
}

#[test]
fn own_file_needs_licence_and_attribution() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    wait_for_the_job(&mut app);
    let own = |path: &str, attribution: &str, licence: &str, nc: bool| OwnFile {
        path: path.into(),
        attribution: attribution.into(),
        licence: licence.into(),
        non_commercial: nc,
    };
    let before = std::fs::read(dir.join("naves.toml")).unwrap();
    for (bad, field) in [
        (own("imagery/a.tif", "A", "", false), "licence"),
        (own("imagery/a.tif", "", "CC BY 4.0", false), "attribution"),
        (
            own("/etc/passwd", "A", "CC BY 4.0", false),
            "sources.imagery",
        ),
        (
            own("imagery/a.tif", "A", "CC BY-NC 4.0", false),
            "non_commercial",
        ),
    ] {
        let err = app.add_own_imagery(&bad).unwrap_err();
        assert!(err.contains(field), "{field}: {err}");
    }
    assert_eq!(
        std::fs::read(dir.join("naves.toml")).unwrap(),
        before,
        "nothing was saved"
    );
    assert_eq!(
        app.form.project.sources.imagery,
        [] as [naves_core::project::ImageryFile; 0]
    );
    app.add_own_imagery(&own("imagery/a.tif", "A", "CC BY 4.0", false))
        .unwrap();
    assert_eq!(Project::load(&dir).unwrap().sources.imagery.len(), 1);
}

#[test]
fn report_is_shown_per_file() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    add_imagery(&dir, "CC BY 4.0", false);
    let mut app = app(root.path(), &dir);
    wait_for_the_job(&mut app);
    let lines = app.imagery_report_lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("imagery/s2.tif") && lines[0].contains("covers"),
        "{lines:?}"
    );
    // and in the window
    let mut harness = gui_harness(&mut app);
    harness.run_steps(4);
    assert!(harness.query_by_label_contains("covers").is_some());
}

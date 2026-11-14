// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Level 4: the row without roads, the tools it needs, the layers that wait, the properties and the problems.

use std::time::{Duration, Instant};

use egui_kittest::kittest::Queryable;
use naves_core::render::MapLayer;

use super::*;
use crate::gui::levels::{BUILT, Level, roads};
use crate::gui::viewport::WorldPoint;
use crate::gui::worker::RoadsState;

/// The project with the OSM highways of the area listed as a source.
fn with_roads(root: &Path) -> PathBuf {
    let dir = project(root);
    std::fs::create_dir_all(dir.join("osm")).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/osm/koenigssee_roads.osm.pbf");
    std::fs::copy(fixture, dir.join("osm/roads.osm.pbf")).unwrap();
    let mut p = Project::load(&dir).unwrap();
    p.sources.osm = vec!["osm/roads.osm.pbf".into()];
    p.save(&dir).unwrap();
    dir
}

fn wait_for_the_roads(app: &mut NavesApp) {
    let deadline = Instant::now() + Duration::from_mins(1);
    while matches!(app.roads, RoadsState::Generating) {
        assert!(Instant::now() < deadline, "timed out waiting for the roads");
        app.poll_background();
        std::thread::yield_now();
    }
}

fn go_to_roads(h: &mut Harness<'_, NavesApp>) {
    h.key_press(egui::Key::Num4);
    h.run_steps(3);
    assert_eq!(h.state().panel.active, Level::Roads);
}

#[test]
fn the_level_is_built_and_has_three_layers() {
    let def = BUILT.iter().find(|d| d.level == Level::Roads).unwrap();
    let names: Vec<&str> = def.layers.iter().map(|l| l.name).collect();
    assert_eq!(names, ["Roads", "Paths", "Drivable"]);
    let ids: Vec<&str> = def.layers.iter().map(|l| l.layer.id()).collect();
    assert_eq!(ids, ["roads", "paths", "drivable"]);
}

#[test]
fn without_osm_the_row_says_what_to_add() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut h = state_harness(app(root.path(), &dir));
    h.run_steps(3);
    go_to_roads(&mut h);
    assert!(
        h.query_by_label("Add an .osm.pbf file to sources.osm to see roads.")
            .is_some()
    );
}

#[test]
fn missing_tools_are_named_in_the_row() {
    let root = tempfile::tempdir().unwrap();
    let dir = with_roads(root.path());
    let mut app = NavesApp::open(
        &dir,
        Box::new(|| Err("armake2 not found; set ARMAKE2".to_string())),
    )
    .unwrap();
    wait_for_the_roads(&mut app);
    assert!(matches!(app.roads, RoadsState::ToolsMissing(_)));
    let mut h = state_harness(app);
    h.run_steps(3);
    go_to_roads(&mut h);
    assert!(
        h.query_by_label("Roads need armake2: armake2 not found; set ARMAKE2")
            .is_some()
    );
    h.get_by_label("Open settings").click();
    h.run_steps(3);
    assert!(h.state().prefs_window.is_some());
}

#[test]
fn road_layers_hide_while_generating() {
    let root = tempfile::tempdir().unwrap();
    let dir = with_roads(root.path());
    let mut app = app(root.path(), &dir);
    wait_for_the_roads(&mut app);
    assert!(matches!(app.roads, RoadsState::Ready(_)));
    app.panel.active = Level::Roads;
    for layer in [
        MapLayer::Roads,
        MapLayer::Paths,
        MapLayer::Drivable,
        MapLayer::TerrainChanges,
    ] {
        app.panel.on.insert(layer);
    }
    let drawn = |app: &NavesApp| {
        app.drawn_layers()
            .iter()
            .map(|l| l.layer)
            .collect::<Vec<_>>()
    };
    assert!(drawn(&app).contains(&MapLayer::Roads));
    // while the roads are placed again, nothing that needs them is drawn
    let ready = std::mem::replace(&mut app.roads, RoadsState::Generating);
    assert!(
        drawn(&app).iter().all(|l| !roads::needs_roads(*l)),
        "{:?}",
        drawn(&app)
    );
    app.panel.on.insert(MapLayer::TooSteepToWalk);
    assert!(drawn(&app).contains(&MapLayer::TooSteepToWalk));
    app.roads = ready;
    assert!(drawn(&app).contains(&MapLayer::Drivable));
}

#[test]
fn the_properties_show_where_a_piece_came_from() {
    let root = tempfile::tempdir().unwrap();
    let dir = with_roads(root.path());
    let mut app = app(root.path(), &dir);
    wait_for_the_roads(&mut app);
    let RoadsState::Ready(generated) = &app.roads else {
        panic!("the roads are not ready");
    };
    let piece = &generated.pieces[0];
    let at = WorldPoint {
        x: f64::midpoint(piece.start.0, piece.end.0),
        y: f64::midpoint(piece.start.1, piece.end.1),
    };
    let pick = (BUILT
        .iter()
        .find(|d| d.level == Level::Roads)
        .unwrap()
        .pick
        .unwrap())(&app, at)
    .expect("a piece of road is picked");
    assert_eq!(pick.title, "Road piece");
    let row = |name: &str| pick.rows.iter().find(|r| r.0 == name).map(|r| r.1.clone());
    assert!(
        row("Model").is_some_and(|m| m.to_ascii_lowercase().ends_with(".p3d")),
        "{:?}",
        pick.rows
    );
    assert_eq!(row("Surface").as_deref(), Some("Gravel"));
    assert!(row("Grade").is_some_and(|g| g.ends_with('%')));
    assert!(
        pick.origin
            .as_deref()
            .is_some_and(|o| o.starts_with("Generated from road/")),
        "{:?}",
        pick.origin
    );
    // and nothing is picked far from any road
    let far = WorldPoint { x: 1.0e6, y: 1.0e6 };
    assert!(
        (BUILT
            .iter()
            .find(|d| d.level == Level::Roads)
            .unwrap()
            .pick
            .unwrap())(&app, far)
        .is_none()
    );
}

#[test]
fn a_violation_is_a_problem_with_a_position() {
    let root = tempfile::tempdir().unwrap();
    let dir = with_roads(root.path());
    let mut app = app(root.path(), &dir);
    wait_for_the_roads(&mut app);
    let RoadsState::Ready(generated) = &app.roads else {
        panic!("the roads are not ready");
    };
    let expected = generated.violations.len();
    assert!(expected > 0, "the fixture has findings");
    let first = generated.violations[0].clone();
    app.refresh_problems();
    let rows: Vec<_> = app
        .problems
        .rows
        .iter()
        .filter(|p| p.level == Some(Level::Roads))
        .collect();
    assert_eq!(rows.len(), expected);
    assert!(rows.iter().all(|p| p.at.is_some()));
    assert_eq!(rows[0].message, first.message);
    assert!((rows[0].at.unwrap().x - first.at.0).abs() < 1e-9);
}

#[test]
fn the_roads_are_generated_again_when_the_sources_change() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    assert!(matches!(app.roads, RoadsState::NoOsm));
    let with = with_roads(root.path());
    assert_eq!(with, dir);
    app.form.project.sources.osm = vec!["osm/roads.osm.pbf".into()];
    app.reload_roads();
    assert!(matches!(app.roads, RoadsState::Generating));
    wait_for_the_roads(&mut app);
    assert!(matches!(app.roads, RoadsState::Ready(_)));
}

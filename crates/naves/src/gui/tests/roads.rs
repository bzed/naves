// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Level 4: the row without roads, the tools it needs, the layers that wait, the properties and the problems.

use egui_kittest::kittest::Queryable;
use naves_core::render::MapLayer;

use super::*;
use crate::gui::levels::{BUILT, Level, roads};
use crate::gui::palette::{ItemKind, ItemSource, Thumbnail};
use crate::gui::shortcuts::ToolId;
use crate::gui::viewport::WorldPoint;
use crate::gui::worker::RoadsState;

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
fn without_osm_the_list_says_what_to_add() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut h = state_harness(app(root.path(), &dir));
    h.run_steps(3);
    go_to_roads(&mut h);
    assert!(
        h.query_by_label(
            "No OpenStreetMap file: draw the roads yourself, or add one to sources.osm."
        )
        .is_some()
    );
}

#[test]
fn missing_tools_are_named_in_the_row() {
    let root = tempfile::tempdir().unwrap();
    let dir = with_roads(root.path());
    let mut app = NavesApp::open(
        &dir,
        Box::new(|| Err("dayzmake not found; set DAYZMAKE".to_string())),
    )
    .unwrap();
    wait_for_the_roads(&mut app);
    assert!(matches!(app.roads, RoadsState::ToolsMissing(_)));
    let mut h = state_harness(app);
    h.run_steps(3);
    go_to_roads(&mut h);
    assert!(
        h.query_by_label("▲ Roads need dayzmake: dayzmake not found; set DAYZMAKE")
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

/// Steps the app without a window until the road pieces are read (or could not be).
fn wait_for_the_catalog(app: &mut NavesApp) {
    let deadline = Instant::now() + crate::gui::testing::HANG_GUARD;
    while app.catalog_job.is_some() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the catalog"
        );
        app.poll_background();
        std::thread::yield_now();
    }
}

#[test]
fn the_roads_level_lists_the_road_classes() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let app = app(root.path(), &dir);
    let items = roads::items(&app);
    let names: Vec<&str> = items
        .iter()
        .filter(|i| i.group == "Roads")
        .map(|i| i.name.as_str())
        .collect();
    assert_eq!(names[..3], ["Primary", "Secondary", "Tertiary"]);
    for class in roads::ROAD_CLASSES {
        let tags: std::collections::BTreeMap<String, String> = class
            .tags
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        let width = match naves_core::features::classify(&tags).unwrap() {
            naves_core::features::FeatureKind::Road { family, .. } => family.width_m(),
            naves_core::features::FeatureKind::Path { width_m, .. } => width_m,
        };
        let item = items
            .iter()
            .find(|i| i.id == format!("roads.class.{}", class.id))
            .unwrap();
        assert_eq!(item.size_m, Some(width), "{}", class.name);
        assert_eq!(item.group, class.group);
        assert_eq!(item.default_tool, ToolId::LINE);
        assert_eq!(item.fits, [Level::Roads]);
    }
    assert!(items.iter().any(|i| i.group == "Paths"));
}

#[test]
fn picking_a_class_switches_to_draw() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut h = state_harness(app(root.path(), &dir));
    h.run_steps(3);
    go_to_roads(&mut h);
    h.get_by_label("Secondary").click();
    h.run_steps(3);
    assert_eq!(h.state().tool, ToolId::LINE);
    assert_eq!(h.state().item.as_deref(), Some("roads.class.secondary"));
    assert!(
        h.query_by_label(
            "Draw: click to add points, click a node to join, Space to finish, Ctrl+Z to take one back."
        )
        .is_some()
    );
}

#[test]
fn catalog_models_are_listed_once_the_catalog_is_read() {
    let root = tempfile::tempdir().unwrap();
    let dir = with_roads(root.path());
    let mut app = app(root.path(), &dir);
    assert!(
        roads::items(&app)
            .iter()
            .all(|i| !i.group.starts_with("Pieces")),
        "nothing before the catalog is read"
    );
    wait_for_the_catalog(&mut app);
    let items = roads::items(&app);
    let models: Vec<_> = items.iter().filter(|i| i.kind == ItemKind::Model).collect();
    assert!(models.iter().any(|i| i.group == "Bridges"));
    // hundreds of pieces are listed by family, not as one group
    assert!(models.iter().all(|i| i.group != "Pieces"));
    let families: std::collections::BTreeSet<_> = models
        .iter()
        .filter(|i| i.group.starts_with("Pieces, "))
        .map(|i| i.group.as_str())
        .collect();
    assert!(families.len() > 1, "{families:?}");
    assert!(families.contains("Pieces, asphalt"), "{families:?}");
    for item in &models {
        assert!(matches!(item.thumbnail, Thumbnail::Footprint { .. }));
        assert!(item.id.starts_with("model."));
        assert_eq!(item.source, ItemSource::Game);
        assert_eq!(item.fits, [Level::Roads]);
        assert_eq!(item.default_tool, ToolId::SELECT);
    }
    // the same ids reach the Everything list once, with the classes of line first (not after the models)
    let all = app.all_items();
    let listed =
        crate::gui::palette::in_scope(&all, Level::Roads, crate::gui::palette::Scope::Everything);
    let first_model = listed
        .iter()
        .position(|i| i.kind == ItemKind::Model)
        .unwrap();
    assert!(
        listed
            .iter()
            .enumerate()
            .filter(|(_, i)| i.kind == ItemKind::LineClass)
            .all(|(n, _)| n < first_model)
    );
    for item in &models {
        assert_eq!(all.iter().filter(|a| a.id == item.id).count(), 1);
    }
}

#[test]
fn picking_a_model_shows_its_card() {
    let root = tempfile::tempdir().unwrap();
    let dir = with_roads(root.path());
    let mut app = app(root.path(), &dir);
    wait_for_the_catalog(&mut app);
    let item = roads::items(&app)
        .into_iter()
        .find(|i| i.group == "Bridges")
        .unwrap();
    let mut h = state_harness(app);
    h.run_steps(3);
    go_to_roads(&mut h);
    // the list has hundreds of rows: narrow it so that the row is in view
    h.state_mut().palette.search = item.name.clone();
    h.run_steps(3);
    h.get_by_label(&item.name).click();
    h.run_steps(3);
    assert_eq!(h.state().tool, ToolId::SELECT);
    assert_eq!(h.state().item.as_deref(), Some(item.id.as_str()));
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::Label, "Roads and paths")
            .is_some(),
        "fits"
    );
    // the size is in the row and in the card
    let size = format!("{:.0} m", item.size_m.unwrap());
    assert_eq!(h.query_all_by_label(&size).count(), 2);
    assert!(
        h.query_by_label("Placing models comes in a later version.")
            .is_some()
    );
}

#[test]
fn drawing_works_without_osm() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut h = state_harness(app(root.path(), &dir));
    h.run_steps(3);
    go_to_roads(&mut h);
    assert!(h.query_by_label("Primary").is_some(), "the selector shows");
    assert!(
        h.query_by_label("Add an .osm.pbf file to sources.osm to see roads.")
            .is_none()
    );
    assert!(
        h.query_by_label(
            "No OpenStreetMap file: draw the roads yourself, or add one to sources.osm."
        )
        .is_some()
    );
}

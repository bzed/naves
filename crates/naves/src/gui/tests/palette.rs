// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The item selector: what fits, the two scopes, the level chip and the shared thumbnail scale.

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use naves_core::prefs::{Preferences, SelectorScope, default_path};

use super::*;
use crate::gui::levels::Level;
use crate::gui::palette::{
    Scope, THUMB_COLUMN_PX, THUMB_PX_PER_M, Thumbnail, in_scope, thumbnail_size,
};
use crate::gui::shortcuts::ToolId;
use crate::gui::testing::FAKE_LEVEL;

/// The fake level (6) is active; the settings live below `root`.
fn started(root: &Path) -> (Harness<'static, NavesApp>, PathBuf) {
    let dir = project(root);
    let path = default_path(&root.join("config/naves"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let app = app(root, &dir)
        .with_extra_level(&FAKE_LEVEL)
        .with_preferences(path.clone());
    let mut h = state_harness(app);
    h.run_steps(3);
    h.key_press(Key::Num6);
    h.run_steps(3);
    (h, path)
}

fn everything(h: &mut Harness<'_, NavesApp>) {
    h.get_by_label("Everything").click();
    h.run_steps(3);
}

#[test]
fn the_scope_switch_is_named_after_the_level() {
    let root = tempfile::tempdir().unwrap();
    let (h, _) = started(root.path());
    assert!(h.query_by_label("Buildings").is_some());
    assert!(h.query_by_label("Everything").is_some());
    assert!(h.query_by_label("This level").is_none());
}

#[test]
fn level_scope_shows_what_fits() {
    let root = tempfile::tempdir().unwrap();
    let (h, _) = started(root.path());
    assert!(h.query_by_label("Small house").is_some());
    assert!(h.query_by_label("Fake road").is_none());
}

#[test]
fn everything_shows_all_with_the_band_chip() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    everything(&mut h);
    assert!(h.query_by_label("Small house").is_some());
    assert!(h.query_by_label("Fake road").is_some());
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::Label, "4 Roads and paths")
            .is_some(),
        "the Roads item carries the chip of level 4"
    );
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::Label, "6 Buildings")
            .is_none(),
        "an item of the active level has no chip"
    );
}

#[test]
fn an_item_of_another_level_keeps_the_active_level() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    everything(&mut h);
    h.get_by_label("Fake road").click();
    h.run_steps(3);
    assert_eq!(h.state().panel.active, Level::Buildings);
    assert_eq!(h.state().item_level, Some(Level::Roads));
    assert_eq!(h.state().tool, ToolId::ADD);
    // the rail and the keys know the tools of the item's level too
    assert!(h.state().has_tool(ToolId::SELECT));
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().tool, ToolId::SELECT);
    assert_eq!(h.state().item_level, None);
}

#[test]
fn an_item_of_the_level_leaves_no_item_level() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.get_by_label("Small house").click();
    h.run_steps(3);
    assert_eq!(h.state().item_level, None);
}

#[test]
fn the_scope_is_kept_per_level() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, path) = started(root.path());
    everything(&mut h);
    h.key_press(Key::Num4);
    h.run_steps(3);
    h.key_press(Key::Num6);
    h.run_steps(3);
    assert!(h.query_by_label("Fake road").is_some(), "still Everything");
    let saved = Preferences::load(&path).unwrap();
    assert_eq!(
        saved.map.selector_scope.get("level.buildings"),
        Some(&SelectorScope::Everything)
    );
    assert_eq!(saved.map.selector_scope.len(), 1);
    // and back
    h.get_by_label("Buildings").click();
    h.run_steps(3);
    assert!(h.query_by_label("Fake road").is_none());
    assert!(
        Preferences::load(&path)
            .unwrap()
            .map
            .selector_scope
            .is_empty()
    );
}

#[test]
fn search_spans_the_scope() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    let search = |h: &mut Harness<'_, NavesApp>| {
        h.state_mut().palette.search = "fake road".to_string();
        h.run_steps(3);
    };
    search(&mut h);
    assert!(h.query_by_label("Fake road").is_none(), "Level scope");
    everything(&mut h);
    assert!(h.query_by_label("Fake road").is_some(), "Everything");
    assert!(h.query_by_label("Small house").is_none());
}

#[test]
fn everything_lists_the_level_first_then_the_stack_order() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let app = app(root.path(), &dir).with_extra_level(&FAKE_LEVEL);
    let items = (FAKE_LEVEL.items)(&app);
    let ids = |level, scope| -> Vec<String> {
        in_scope(&items, level, scope)
            .iter()
            .map(|i| i.id.clone())
            .collect()
    };
    assert_eq!(ids(Level::Roads, Scope::Level), ["road.fake"]);
    assert_eq!(
        ids(Level::Buildings, Scope::Level),
        ["house.small", "house.big", "bunker"]
    );
    // the level's own first, by group then name; then Roads
    assert_eq!(
        ids(Level::Buildings, Scope::Everything),
        ["bunker", "house.big", "house.small", "road.fake"]
    );
    // seen from Roads: Roads first, then Buildings
    assert_eq!(
        ids(Level::Roads, Scope::Everything),
        ["road.fake", "bunker", "house.big", "house.small"]
    );
}

#[test]
fn thumbnails_share_one_scale() {
    assert!((THUMB_PX_PER_M - 2.0).abs() < f32::EPSILON);
    let line = |width_m| Thumbnail::Line {
        width_m,
        rgb: [200, 200, 200],
    };
    assert!((thumbnail_size(&line(12.0)).y - 24.0).abs() < 1e-4);
    assert!((thumbnail_size(&line(5.0)).y - 10.0).abs() < 1e-4);
    assert!((thumbnail_size(&line(5.0)).x - THUMB_COLUMN_PX).abs() < 1e-4);
    let footprint = Thumbnail::Footprint {
        outline: vec![(-20.0, -3.0), (20.0, -3.0), (20.0, 3.0), (-20.0, 3.0)],
    };
    let size = thumbnail_size(&footprint);
    assert!((size.x - THUMB_COLUMN_PX).abs() < 1e-4, "40 m is clipped");
    assert!((size.y - 12.0).abs() < 1e-4);
    let short = Thumbnail::Footprint {
        outline: vec![(-6.0, -2.0), (6.0, -2.0), (6.0, 2.0), (-6.0, 2.0)],
    };
    assert!((thumbnail_size(&short).x - 24.0).abs() < 1e-4);
}

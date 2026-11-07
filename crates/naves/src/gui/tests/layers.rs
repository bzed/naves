// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Switchable map layers: the checkboxes, fade and hide, and that switching renders nothing.

use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use naves_core::render::{MapLayer, WALK_LIMIT_DEG};

use super::*;
use crate::gui::layers::visible;
use crate::gui::levels::{BUILT, Level};
use crate::gui::stack::LayerPanel;
use crate::gui::testing::FakeViewport;
use crate::gui::viewport::map::MapViewport;
use crate::gui::viewport::{Viewport, VisibleLayer};

const NAME: &str = "Too steep to walk";

fn run_until(harness: &mut Harness<'_, NavesApp>, done: impl Fn(&NavesApp) -> bool) {
    let deadline = Instant::now() + Duration::from_mins(1);
    while !done(harness.state()) {
        assert!(Instant::now() < deadline, "timed out waiting for the GUI");
        harness.step();
        std::thread::yield_now();
    }
}

fn ready(app: &NavesApp) -> bool {
    matches!(app.heightmap, HeightmapState::Ready(_))
}

fn started(root: &Path) -> (Harness<'static, NavesApp>, PathBuf) {
    let dir = project(root);
    (state_harness(app(root, &dir)), dir)
}

fn go(h: &mut Harness<'_, NavesApp>, key: egui::Key) {
    h.key_press(key);
    h.run_steps(3);
}

fn box_of<'a>(h: &'a Harness<'_, NavesApp>) -> egui_kittest::Node<'a> {
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, NAME)
}

#[test]
fn layers_are_checkboxes_with_their_names() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    go(&mut h, egui::Key::Num1);
    assert_eq!(
        box_of(&h).accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    box_of(&h).click();
    h.run_steps(3);
    assert_eq!(
        box_of(&h).accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    assert!(h.state().panel.on.contains(&MapLayer::TooSteepToWalk));
}

#[test]
fn the_key_shows_only_when_the_layer_is_on() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    go(&mut h, egui::Key::Num1);
    assert!(h.query_by_label_contains("Steeper than").is_none());
    box_of(&h).click();
    h.run_steps(3);
    assert!(h.query_by_label_contains("Steeper than").is_some());
}

#[test]
fn a_level_s_layers_show_only_when_it_is_open() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    assert!(h.query_by_label(NAME).is_none());
    go(&mut h, egui::Key::Num1);
    assert!(h.query_by_label(NAME).is_some());
    go(&mut h, egui::Key::Num2);
    assert!(h.query_by_label(NAME).is_none());
}

#[test]
fn visible_follows_fade_hide_and_full() {
    let mut panel = LayerPanel::default();
    panel.on.insert(MapLayer::TooSteepToWalk);
    let opacity = |panel: &LayerPanel| -> Vec<f32> {
        visible(panel, BUILT, 0.4)
            .iter()
            .map(|v| v.opacity)
            .collect()
    };
    panel.active = Level::Terrain;
    assert_eq!(opacity(&panel), [1.0]);
    panel.active = Level::Ground;
    assert_eq!(opacity(&panel), [0.4]);
    panel.show_others_in_full = true;
    assert_eq!(opacity(&panel), [1.0]);
    panel.show_others_in_full = false;
    panel.hidden.insert(Level::Terrain);
    assert_eq!(
        visible(&panel, BUILT, 0.4),
        [] as [crate::gui::viewport::VisibleLayer; 0]
    );
    panel.hidden.clear();
    panel.on.clear();
    assert_eq!(
        visible(&panel, BUILT, 0.4),
        [] as [crate::gui::viewport::VisibleLayer; 0]
    );
}

#[test]
fn h_hides_the_active_level_and_shift_h_shows_the_others_in_full() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    go(&mut h, egui::Key::Num1);
    go(&mut h, egui::Key::H);
    assert!(h.state().panel.hidden.contains(&Level::Terrain));
    go(&mut h, egui::Key::H);
    assert!(h.state().panel.hidden.is_empty());
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::H);
    h.run_steps(3);
    assert!(h.state().panel.show_others_in_full);
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::H);
    h.run_steps(3);
    assert!(!h.state().panel.show_others_in_full);
}

#[test]
fn making_a_hidden_level_active_shows_it() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    go(&mut h, egui::Key::Num1);
    go(&mut h, egui::Key::H);
    go(&mut h, egui::Key::Num2);
    assert!(h.state().panel.hidden.contains(&Level::Terrain));
    go(&mut h, egui::Key::Num1);
    assert!(h.state().panel.hidden.is_empty());
    go(&mut h, egui::Key::H);
    go(&mut h, egui::Key::Num2);
    h.get_by_label("1 Terrain").click();
    h.run_steps(3);
    assert!(h.state().panel.hidden.is_empty(), "a click shows it too");
}

fn renders(h: &Harness<'_, NavesApp>) -> u64 {
    h.state().views.active().renders()
}

#[test]
fn switching_renders_nothing() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    run_until(&mut h, ready);
    h.run_steps(3);
    go(&mut h, egui::Key::Num1);
    box_of(&h).click();
    h.run_steps(4);
    let before = renders(&h);
    assert!(
        before >= 2,
        "the base map and the layer were drawn: {before}"
    );
    go(&mut h, egui::Key::H);
    go(&mut h, egui::Key::H);
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::H);
    h.run_steps(3);
    go(&mut h, egui::Key::Num2);
    go(&mut h, egui::Key::Num1);
    assert_eq!(renders(&h), before);
}

#[test]
fn the_view_gets_the_same_layers_in_both_modes() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let (fake, handle) = FakeViewport::new();
    let views: Vec<Box<dyn Viewport>> = vec![Box::new(MapViewport::default()), Box::new(fake)];
    let mut app = app(root.path(), &dir).with_views(views);
    app.panel.on.insert(MapLayer::TooSteepToWalk);
    app.panel.active = Level::Ground;
    let mut h = state_harness(app);
    run_until(&mut h, ready);
    h.state_mut().views.switch();
    h.run_steps(3);
    let expected = vec![VisibleLayer {
        layer: MapLayer::TooSteepToWalk,
        opacity: 0.4,
    }];
    assert_eq!(handle.borrow().layers, expected);
}

#[test]
fn layer_state_does_not_touch_the_project_file() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, dir) = started(root.path());
    h.run_steps(3);
    let before = std::fs::read(dir.join("naves.toml")).unwrap();
    go(&mut h, egui::Key::Num1);
    box_of(&h).click();
    h.run_steps(3);
    go(&mut h, egui::Key::H);
    assert_eq!(std::fs::read(dir.join("naves.toml")).unwrap(), before);
}

#[test]
fn the_tooltip_has_the_number_of_the_constant() {
    let def = BUILT.iter().find(|d| d.level == Level::Terrain).unwrap();
    let tip = (def.layers[0].tooltip)();
    assert!(tip.contains(&format!("{WALK_LIMIT_DEG:.0}°")), "{tip}");
}

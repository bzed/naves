// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading a point, framing it, and moving the map.

use std::time::{Duration, Instant};

use egui::{Event, Modifiers, PointerButton};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;
use crate::gui::levels::Level;
use crate::gui::testing::{FAKE_LEVEL, FakeHandle, FakeViewport};
use crate::gui::viewport::map::MapViewport;
use crate::gui::viewport::{ViewEvent, ViewFocus, Viewport, WorldPoint};

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

/// A harness with data loaded and the map drawn.
fn started(root: &Path, extra: bool) -> Harness<'static, NavesApp> {
    let dir = project(root);
    add_sources(&dir);
    let mut app = app(root, &dir);
    if extra {
        app = app.with_extra_level(&FAKE_LEVEL);
    }
    let mut h = state_harness(app);
    run_until(&mut h, ready);
    h.run_steps(3);
    h
}

fn with_fake(root: &Path) -> (Harness<'static, NavesApp>, FakeHandle) {
    let dir = project(root);
    let (fake, handle) = FakeViewport::new();
    let views: Vec<Box<dyn Viewport>> = vec![Box::new(MapViewport::default()), Box::new(fake)];
    let mut h = state_harness(app(root, &dir).with_views(views));
    run_until(&mut h, ready);
    h.run_steps(3);
    (h, handle)
}

fn go(h: &mut Harness<'_, NavesApp>, key: egui::Key) {
    h.key_press(key);
    h.run_steps(3);
}

fn centre(h: &Harness<'_, NavesApp>) -> egui::Pos2 {
    h.state().seen_rect.center() + egui::vec2(40.0, -30.0)
}

fn button(h: &Harness<'_, NavesApp>, pos: egui::Pos2, button: PointerButton, pressed: bool) {
    h.event(Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: Modifiers::NONE,
    });
}

fn click(h: &mut Harness<'_, NavesApp>, pos: egui::Pos2, modifiers: Modifiers) {
    h.hover_at(pos);
    h.run_steps(1);
    h.event_modifiers(
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers,
        },
        modifiers,
    );
    h.run_steps(1);
    h.event_modifiers(
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers,
        },
        modifiers,
    );
    h.run_steps(3);
}

fn drag(h: &mut Harness<'_, NavesApp>, from: egui::Pos2, by: egui::Vec2, with: PointerButton) {
    h.hover_at(from);
    h.run_steps(1);
    button(h, from, with, true);
    h.run_steps(1);
    h.hover_at(from + by * 0.5);
    h.run_steps(1);
    h.hover_at(from + by);
    h.run_steps(1);
    button(h, from + by, with, false);
    h.run_steps(3);
}

fn click_centre(h: &mut Harness<'_, NavesApp>, modifiers: Modifiers) {
    let pos = centre(h);
    click(h, pos, modifiers);
}

fn drag_centre(h: &mut Harness<'_, NavesApp>, by: egui::Vec2, with: PointerButton) {
    let pos = centre(h);
    drag(h, pos, by, with);
}

fn clicked_at(h: &Harness<'_, NavesApp>) -> WorldPoint {
    h.state()
        .seen_events
        .iter()
        .rev()
        .find_map(|e| match e {
            ViewEvent::Click { at, .. } => Some(*at),
            _ => None,
        })
        .expect("a click was reported")
}

fn focus(h: &Harness<'_, NavesApp>) -> ViewFocus {
    h.state().views.active().focus().unwrap()
}

fn row(selection: &crate::gui::selection::Selection, name: &str) -> String {
    selection
        .rows
        .iter()
        .find(|(k, _)| k == name)
        .unwrap_or_else(|| panic!("no row {name}: {:?}", selection.rows))
        .1
        .clone()
}

#[test]
fn a_click_selects_only_in_the_active_level() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), false);
    click_centre(&mut h, Modifiers::NONE);
    assert!(h.state().selection.is_none(), "level 0 has nothing to pick");
    go(&mut h, egui::Key::Num1);
    click_centre(&mut h, Modifiers::NONE);
    assert_eq!(h.state().selection.as_ref().unwrap().level, Level::Terrain);
}

#[test]
fn a_click_reads_height_and_slope_in_terrain() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), false);
    go(&mut h, egui::Key::Num1);
    click_centre(&mut h, Modifiers::NONE);
    let at = clicked_at(&h);
    let HeightmapState::Ready(data) = &h.state().heightmap else {
        unreachable!()
    };
    let sample = data.heightmap.sample(at.x, at.y);
    let selection = h.state().selection.as_ref().unwrap();
    assert_eq!(row(selection, "Height"), format!("{:.1} m", sample.height));
    assert_eq!(
        row(selection, "Slope"),
        format!("{:.1}°", sample.slope_deg())
    );
    assert!(row(selection, "Latitude").ends_with('°'));
    assert!(selection.origin.is_none());
}

#[test]
fn a_click_reads_the_class_in_ground() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), false);
    go(&mut h, egui::Key::Num2);
    click_centre(&mut h, Modifiers::NONE);
    let at = clicked_at(&h);
    let HeightmapState::Ready(data) = &h.state().heightmap else {
        unreachable!()
    };
    let class = data.surfaces.at(at.x, at.y);
    let selection = h.state().selection.as_ref().unwrap();
    assert_eq!(row(selection, "Ground"), class.name());
}

#[test]
fn the_properties_are_shown_under_the_stack() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), false);
    go(&mut h, egui::Key::Num1);
    click_centre(&mut h, Modifiers::NONE);
    assert!(h.query_by_label_contains(" m east, ").is_some());
    assert_eq!(
        h.query_all_by_label("Slope").count(),
        2,
        "the base map and the row"
    );
    assert!(h.query_all_by_label_contains("°").count() >= 1);
}

#[test]
fn alt_click_switches_to_the_level_it_hit() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), true);
    click_centre(&mut h, Modifiers::ALT);
    assert_eq!(
        h.state().selection.as_ref().unwrap().level,
        Level::Buildings
    );
    assert_eq!(h.state().panel.active, Level::Buildings);
}

#[test]
fn alt_click_prefers_the_higher_level() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), true);
    go(&mut h, egui::Key::Num1);
    click_centre(&mut h, Modifiers::ALT);
    // Terrain would hit too, but Buildings is above it
    assert_eq!(
        h.state().selection.as_ref().unwrap().level,
        Level::Buildings
    );
    go(&mut h, egui::Key::Num1);
    click_centre(&mut h, Modifiers::NONE);
    assert_eq!(h.state().selection.as_ref().unwrap().level, Level::Terrain);
}

#[test]
fn the_fake_view_can_select_too() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, fake) = with_fake(root.path());
    go(&mut h, egui::Key::Num1);
    h.state_mut().views.switch();
    fake.borrow_mut().queued.push(ViewEvent::Click {
        at: WorldPoint { x: 400.0, y: 500.0 },
        modifiers: Modifiers::NONE,
    });
    h.run_steps(3);
    let selection = h.state().selection.as_ref().unwrap();
    assert_eq!(selection.level, Level::Terrain);
    assert!((selection.at.x - 400.0).abs() < 1e-9);
}

#[test]
fn esc_clears_the_selection() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), false);
    go(&mut h, egui::Key::Num1);
    click_centre(&mut h, Modifiers::NONE);
    assert!(h.state().selection.is_some());
    go(&mut h, egui::Key::Escape);
    assert!(h.state().selection.is_none());
}

#[test]
fn f_frames_the_selection() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), false);
    go(&mut h, egui::Key::Num1);
    click_centre(&mut h, Modifiers::NONE);
    let at = h.state().selection.as_ref().unwrap().at;
    let span = focus(&h).span_m;
    go(&mut h, egui::Key::F);
    let now = focus(&h);
    assert!((now.center.x - at.x).abs() < 1e-6 && (now.center.y - at.y).abs() < 1e-6);
    assert!((now.span_m - span).abs() < 1e-6, "the zoom stays");
}

#[test]
fn shift_f_frames_the_map() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), false);
    let map = h.state().form.project.map_size_m();
    drag_centre(&mut h, egui::vec2(80.0, 0.0), PointerButton::Middle);
    h.key_press_modifiers(Modifiers::SHIFT, egui::Key::F);
    h.run_steps(3);
    let now = focus(&h);
    assert!((now.center.x - map / 2.0).abs() < 1e-6 && (now.span_m - map).abs() < 1e-6);
}

#[test]
fn framing_without_data_does_nothing() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("empty-proj");
    std::fs::create_dir_all(&dir).unwrap();
    Project::new("Empty", 1.0, 2.0, 1000.0).save(&dir).unwrap();
    let mut h = state_harness(app(root.path(), &dir));
    h.run_steps(3);
    h.state_mut().selection = Some(crate::gui::selection::Selection {
        level: Level::Terrain,
        title: "x".into(),
        rows: Vec::new(),
        origin: None,
        at: WorldPoint { x: 1.0, y: 1.0 },
        span_m: None,
    });
    go(&mut h, egui::Key::F);
    h.key_press_modifiers(Modifiers::SHIFT, egui::Key::F);
    h.run_steps(3);
    assert!(h.state().views.active().focus().is_none());
}

#[test]
fn a_changed_map_drops_the_selection() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), false);
    go(&mut h, egui::Key::Num1);
    click_centre(&mut h, Modifiers::NONE);
    assert!(h.state().selection.is_some());
    h.state_mut().form.project.area.real_extent_m += 100.0;
    h.state_mut().settings_changed();
    h.run_steps(2);
    assert!(h.state().selection.is_none());
}

#[test]
fn the_marker_uses_project_of_the_view() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, fake) = with_fake(root.path());
    go(&mut h, egui::Key::Num1);
    h.state_mut().views.switch();
    fake.borrow_mut().queued.push(ViewEvent::Click {
        at: WorldPoint { x: 400.0, y: 500.0 },
        modifiers: Modifiers::NONE,
    });
    h.run_steps(3);
    // the view says the point is not visible: nothing is drawn and nothing breaks
    fake.borrow_mut().projected = None;
    h.run_steps(2);
    fake.borrow_mut().projected = Some(egui::pos2(500.0, 300.0));
    h.run_steps(2);
    assert!(h.state().selection.is_some());
}

#[test]
fn space_drag_pans_and_a_left_drag_does_not() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), false);
    let before = focus(&h).center;
    drag_centre(&mut h, egui::vec2(60.0, 0.0), PointerButton::Primary);
    let plain = focus(&h).center;
    assert_eq!(
        plain, before,
        "a plain left drag leaves the map where it is"
    );
    h.key_down(egui::Key::Space);
    h.run_steps(1);
    drag_centre(&mut h, egui::vec2(60.0, 0.0), PointerButton::Primary);
    h.key_up(egui::Key::Space);
    h.run_steps(2);
    assert!(
        focus(&h).center.x < before.x - 1.0,
        "the map follows the pointer east"
    );
}

#[test]
fn middle_drag_pans() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), false);
    let before = focus(&h).center;
    drag_centre(&mut h, egui::vec2(60.0, 0.0), PointerButton::Middle);
    assert!(focus(&h).center.x < before.x - 1.0);
}

#[test]
fn space_on_a_focused_checkbox_does_not_pan() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), false);
    go(&mut h, egui::Key::Num1);
    let before = focus(&h).center;
    let boxed = h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Too steep to walk");
    boxed.focus();
    h.run_steps(2);
    go(&mut h, egui::Key::Space);
    assert!(
        h.state().panel.on.iter().next().is_some(),
        "Space toggled the checkbox"
    );
    assert_eq!(focus(&h).center, before);
}

#[test]
fn a_hidden_active_level_selects_nothing() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), false);
    go(&mut h, egui::Key::Num1);
    go(&mut h, egui::Key::H);
    click_centre(&mut h, Modifiers::NONE);
    assert!(h.state().selection.is_none());
}

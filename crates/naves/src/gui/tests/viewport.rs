// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The seam between the shell and the views.

use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;
use crate::gui::testing::{FakeHandle, FakeViewport};
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

fn two_views(app: NavesApp) -> (NavesApp, FakeHandle) {
    let (fake, handle) = FakeViewport::new();
    let views: Vec<Box<dyn Viewport>> = vec![Box::new(MapViewport::default()), Box::new(fake)];
    (app.with_views(views), handle)
}

fn harness(app: NavesApp) -> Harness<'static, NavesApp> {
    state_harness(app)
}

#[test]
fn the_map_reports_clicks_in_metres() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut harness = harness(app(root.path(), &dir));
    run_until(&mut harness, ready);
    harness.run_steps(2);
    let rect = harness.state().seen_rect;
    let pos = rect.center() + egui::vec2(40.0, -30.0);
    harness.hover_at(pos);
    harness.run_steps(1);
    harness.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(1);
    harness.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(2);
    let app = harness.state();
    let focus = app.views.active().focus().unwrap();
    let mpp = focus.span_m / f64::from(rect.width().min(rect.height()));
    let want = WorldPoint {
        x: focus.center.x + f64::from(pos.x - rect.center().x) * mpp,
        y: focus.center.y - f64::from(pos.y - rect.center().y) * mpp,
    };
    let at = app
        .seen_events
        .iter()
        .find_map(|e| match e {
            ViewEvent::Click { at, .. } => Some(*at),
            _ => None,
        })
        .expect("a click was reported");
    assert!(
        (at.x - want.x).abs() < 0.01 && (at.y - want.y).abs() < 0.01,
        "{at:?} {want:?}"
    );
}

#[test]
fn switching_keeps_the_place() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let (app, fake) = two_views(app(root.path(), &dir));
    let mut harness = harness(app);
    run_until(&mut harness, ready);
    harness.run_steps(2);
    let here = harness.state().views.active().focus().unwrap();
    harness.state_mut().views.switch();
    assert_eq!(fake.borrow().focus, Some(here));
    let moved = ViewFocus {
        center: WorldPoint { x: 700.0, y: 900.0 },
        span_m: 400.0,
    };
    fake.borrow_mut().focus = Some(moved);
    harness.state_mut().views.switch();
    let back = harness.state().views.active().focus().unwrap();
    assert!((back.center.x - 700.0).abs() < 1e-6 && (back.span_m - 400.0).abs() < 1e-6);
}

#[test]
fn a_view_is_told_the_place_before_it_was_drawn() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    let want = ViewFocus {
        center: WorldPoint { x: 300.0, y: 400.0 },
        span_m: 500.0,
    };
    app.views.active_mut().set_focus(want);
    let mut harness = harness(app);
    run_until(&mut harness, ready);
    harness.run_steps(2);
    let got = harness.state().views.active().focus().unwrap();
    assert!(
        (got.center.x - 300.0).abs() < 1e-6 && (got.center.y - 400.0).abs() < 1e-6,
        "{got:?}"
    );
    assert!((got.span_m - 500.0).abs() < 1e-6, "{got:?}");
}

#[test]
fn switching_needs_two_views() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    assert!(!app.views.can_switch());
    let name = app.views.active().name();
    app.views.switch();
    assert_eq!(app.views.active().name(), name);
    let (app, _) = two_views(app);
    assert!(app.views.can_switch());
}

#[test]
fn the_empty_state_is_the_same_in_both_views() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("empty-proj");
    std::fs::create_dir_all(&dir).unwrap();
    Project::new("Empty", 1.0, 2.0, 1000.0).save(&dir).unwrap();
    let (mut app, fake) = two_views(app(root.path(), &dir));
    app.views.switch();
    assert_eq!(app.views.active().name(), "3D");
    let mut harness = harness(app);
    harness.run_steps(4);
    assert!(
        harness
            .query_by_label_contains("Add elevation files")
            .is_some()
    );
    assert_eq!(
        fake.borrow().shown,
        0,
        "the view is not asked to draw nothing"
    );
}

#[test]
fn reloading_resets_every_view() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let (mut app, fake) = two_views(app(root.path(), &dir));
    assert_eq!(fake.borrow().resets, 0);
    app.form.project.area.real_extent_m += 100.0;
    app.settings_changed();
    assert_eq!(fake.borrow().resets, 1);
}

#[test]
fn the_wheel_zoom_is_one_rule_for_every_view() {
    use crate::gui::viewport::zoom_factor;
    let (normal, inverted) = (zoom_factor(10.0, false), zoom_factor(10.0, true));
    assert!(normal < 1.0 && inverted > 1.0, "{normal} {inverted}");
    assert!((normal * inverted - 1.0).abs() < 1e-12);
    assert!((zoom_factor(0.0, true) - 1.0).abs() < 1e-12);
}

#[test]
fn space_with_the_left_button_or_the_middle_button_pans() {
    use crate::gui::viewport::pans;
    use egui::PointerButton::{Middle, Primary, Secondary};
    assert!(pans(Middle, false) && pans(Middle, true));
    assert!(pans(Primary, true));
    assert!(
        !pans(Primary, false),
        "a plain left drag belongs to the tools"
    );
    assert!(!pans(Secondary, true));
}

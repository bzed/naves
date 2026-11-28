// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The seam between the shell and the views.

use std::time::Instant;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;
use crate::gui::testing::{FakeHandle, FakeViewport};
use crate::gui::viewport::map::MapViewport;
use crate::gui::viewport::{ViewEvent, ViewFocus, Viewport, WorldPoint};

fn run_until(harness: &mut Harness<'_, NavesApp>, done: impl Fn(&NavesApp) -> bool) {
    let deadline = Instant::now() + crate::gui::testing::HANG_GUARD;
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
fn the_wheel_over_the_map_zooms_it() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut harness = harness(app(root.path(), &dir));
    run_until(&mut harness, ready);
    harness.run_steps(2);
    let before = harness.state().views.active().focus().unwrap().span_m;
    let pos = harness.state().seen_rect.center();
    harness.hover_at(pos);
    harness.run_steps(1);
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 120.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: egui::Modifiers::NONE,
    });
    // egui spreads a wheel turn over a few frames; the steps, not the clock, decide how far it got
    harness.run_steps(8);
    let after = harness.state().views.active().focus().unwrap().span_m;
    assert!(
        after < before,
        "a wheel turn up zooms in: {before} -> {after}"
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

// ---- the keys that move the view ----

/// A harness whose active view is the fake 3D view, standing at a known place.
fn standing(root: &Path) -> (Harness<'static, NavesApp>, FakeHandle) {
    let dir = project(root);
    let (mut app, fake) = two_views(app(root, &dir));
    app.views.switch();
    fake.borrow_mut().focus = Some(ViewFocus {
        center: WorldPoint { x: 750.0, y: 750.0 },
        span_m: 1000.0,
    });
    let mut h = harness(app);
    h.run_steps(3);
    (h, fake)
}

/// Holds `key` for `frames` frames (with `modifiers` held too) and returns the sum of the frame times.
fn hold(
    h: &mut Harness<'_, NavesApp>,
    key: egui::Key,
    modifiers: egui::Modifiers,
    frames: u32,
) -> f64 {
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.key_down(key);
    let mut seconds = 0.0;
    for _ in 0..frames {
        h.run_steps(1);
        seconds += f64::from(h.ctx.input(|i| i.stable_dt));
    }
    h.key_up(key);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(2);
    seconds
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 0.01 * b.abs().max(1e-9)
}

#[test]
fn holding_d_pans_east() {
    use crate::gui::shortcuts::{FAST, PAN_VIEWS_PER_S, SLOW};
    for (modifiers, speed) in [
        (egui::Modifiers::NONE, 1.0),
        (egui::Modifiers::SHIFT, FAST),
        (egui::Modifiers::CTRL, SLOW),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (mut h, fake) = standing(root.path());
        let seconds = hold(&mut h, egui::Key::D, modifiers, 30);
        let focus = fake.borrow().focus.unwrap();
        let want = seconds * PAN_VIEWS_PER_S * 1000.0 * speed;
        assert!(
            near(focus.center.x - 750.0, want),
            "{modifiers:?}: moved {}, wanted {want}",
            focus.center.x - 750.0
        );
        assert!((focus.center.y - 750.0).abs() < 1e-9, "y stays");
        assert!((focus.span_m - 1000.0).abs() < 1e-9, "the span stays");
    }
}

#[test]
fn w_a_and_s_pan_the_other_ways() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, fake) = standing(root.path());
    hold(&mut h, egui::Key::W, egui::Modifiers::NONE, 10);
    assert!(fake.borrow().focus.unwrap().center.y > 750.0, "W is north");
    let (mut h, fake) = standing(root.path());
    hold(&mut h, egui::Key::S, egui::Modifiers::NONE, 10);
    assert!(fake.borrow().focus.unwrap().center.y < 750.0, "S is south");
    let (mut h, fake) = standing(root.path());
    hold(&mut h, egui::Key::A, egui::Modifiers::NONE, 10);
    assert!(fake.borrow().focus.unwrap().center.x < 750.0, "A is west");
}

#[test]
fn q_and_e_zoom() {
    use crate::gui::shortcuts::ZOOM_PER_S;
    let root = tempfile::tempdir().unwrap();
    let (mut h, fake) = standing(root.path());
    let seconds = hold(&mut h, egui::Key::E, egui::Modifiers::NONE, 30);
    let span = fake.borrow().focus.unwrap().span_m;
    assert!(
        near(span, 1000.0 / ZOOM_PER_S.powf(seconds)),
        "E zooms in: {span}"
    );
    let (mut h, fake) = standing(root.path());
    let seconds = hold(&mut h, egui::Key::Q, egui::Modifiers::NONE, 30);
    let span = fake.borrow().focus.unwrap().span_m;
    assert!(
        near(span, 1000.0 * ZOOM_PER_S.powf(seconds)),
        "Q zooms out: {span}"
    );
}

#[test]
fn wasd_does_nothing_while_typing() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, fake) = standing(root.path());
    let field = h
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .next()
        .unwrap();
    field.focus();
    h.run_steps(2);
    hold(&mut h, egui::Key::W, egui::Modifiers::NONE, 10);
    assert_eq!(
        fake.borrow().focus.unwrap().center,
        WorldPoint { x: 750.0, y: 750.0 }
    );
}

#[test]
fn a_released_key_stops_the_move() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, fake) = standing(root.path());
    hold(&mut h, egui::Key::D, egui::Modifiers::NONE, 5);
    let at = fake.borrow().focus.unwrap();
    h.run_steps(5);
    assert_eq!(fake.borrow().focus.unwrap(), at);
}

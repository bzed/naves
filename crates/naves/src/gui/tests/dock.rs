// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The bottom dock: its tabs, Build and Problems.

use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;
use crate::gui::dock::{DockTabDef, TABS, tab_row};
use crate::gui::levels::Level;
use crate::gui::problems::{Problem, ProblemList};
use crate::gui::testing::FakeViewport;
use crate::gui::viewport::map::MapViewport;
use crate::gui::viewport::{Viewport, WorldPoint};

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

fn started(root: &Path) -> Harness<'static, NavesApp> {
    let dir = project(root);
    let mut h = state_harness(app(root, &dir));
    h.run_steps(3);
    h
}

fn problem(message: &str, at: Option<(f64, f64)>, level: Option<Level>) -> Problem {
    Problem {
        message: message.to_string(),
        at: at.map(|(x, y)| WorldPoint { x, y }),
        level,
    }
}

fn click_tab(h: &mut Harness<'_, NavesApp>, text: &str) {
    let tab = h
        .get_all_by_role(Role::Tab)
        .find(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with(text))
        })
        .unwrap_or_else(|| panic!("no tab {text}"));
    tab.click();
    h.run_steps(3);
}

fn tab_labels(h: &Harness<'_, NavesApp>) -> Vec<String> {
    h.get_all_by_role(Role::Tab)
        .filter_map(|n| n.accesskit_node().label())
        .collect()
}

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} is not {b}");
}

fn centre(h: &Harness<'_, NavesApp>) -> WorldPoint {
    let f = h.state().views.active().focus().unwrap();
    f.center
}

#[test]
fn the_tabs_are_build_and_problems() {
    let ids: Vec<&str> = TABS.iter().map(|t| t.id).collect();
    assert_eq!(ids, ["build", "problems"]);
    let root = tempfile::tempdir().unwrap();
    let h = started(root.path());
    assert_eq!(tab_labels(&h), ["Build", "Problems"]);
}

#[test]
fn a_new_tab_needs_only_an_entry() {
    fn label(_: &NavesApp) -> String {
        "Fake tab".to_string()
    }
    fn show(_: &mut NavesApp, ui: &mut egui::Ui) {
        ui.label("inside the fake tab");
    }
    static FAKE: &[DockTabDef] = &[
        DockTabDef {
            id: "one",
            label,
            show,
        },
        DockTabDef {
            id: "two",
            label,
            show,
        },
    ];
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut h = Harness::builder().with_size((600.0, 200.0)).build_ui_state(
        |ui, app: &mut NavesApp| tab_row(ui, FAKE, app),
        app(root.path(), &dir),
    );
    h.run_steps(3);
    assert_eq!(tab_labels(&h), ["Fake tab", "Fake tab"]);
}

#[test]
fn build_warnings_become_problems() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/landcover/koenigssee_west.tif");
    std::fs::create_dir_all(dir.join("landcover")).unwrap();
    std::fs::copy(fixture, dir.join("landcover/koenigssee_west.tif")).unwrap();
    let mut p = Project::load(&dir).unwrap();
    p.sources.land_cover = vec!["landcover/koenigssee_west.tif".into()];
    p.save(&dir).unwrap();
    let mut h = state_harness(app(root.path(), &dir));
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Build").click();
    run_until(&mut h, |a| !a.build_problems.is_empty());
    assert!(
        h.state().build_problems[0]
            .message
            .contains("not covered by the land cover")
    );
    click_tab(&mut h, "Problems");
    assert!(
        h.query_by_label_contains("not covered by the land cover")
            .is_some()
    );
}

#[test]
fn a_failed_build_is_a_problem_that_goes_when_the_next_build_starts() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let app = NavesApp::open(&dir, Box::new(|| Err("armake2 not found".to_string()))).unwrap();
    let mut h = state_harness(app);
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Build").click();
    run_until(&mut h, |a| !a.build_problems.is_empty());
    assert!(
        h.state().build_problems[0]
            .message
            .contains("armake2 not found")
    );
    h.state_mut()
        .build_problems
        .push(problem("old", None, None));
    h.get_by_role_and_label(Role::Button, "Build").click();
    h.run_steps(2);
    assert!(h.state().build_problems.iter().all(|p| p.message != "old"));
}

#[test]
fn merge_warnings_become_problems_and_are_replaced_when_the_map_reloads() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut h = state_harness(app(root.path(), &dir));
    run_until(&mut h, ready);
    h.state_mut()
        .merge_problems
        .push(problem("a merge note", None, None));
    h.run_steps(2);
    click_tab(&mut h, "Problems");
    assert!(h.query_by_label_contains("a merge note").is_some());
    h.state_mut().form.project.area.real_extent_m += 100.0;
    h.state_mut().settings_changed();
    run_until(&mut h, ready);
    h.run_steps(3);
    assert_eq!(
        h.state().merge_problems,
        [] as [crate::gui::problems::Problem; 0]
    );
    assert!(h.query_by_label_contains("a merge note").is_none());
}

#[test]
fn a_form_error_is_a_problem_until_fixed() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path());
    h.state_mut().form.error = Some("latitude out of range".to_string());
    click_tab(&mut h, "Problems");
    // the line under the settings form and the row of the list
    assert_eq!(
        h.query_all_by_label_contains("latitude out of range")
            .count(),
        2
    );
    h.state_mut().form.error = None;
    h.run_steps(3);
    assert_eq!(
        h.query_all_by_label_contains("latitude out of range")
            .count(),
        0
    );
}

#[test]
fn the_count_matches_the_list() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path());
    for i in 0..3 {
        h.state_mut()
            .build_problems
            .push(problem(&format!("note {i}"), None, None));
    }
    h.run_steps(3);
    assert_eq!(tab_labels(&h), ["Build", "Problems 3"]);
    assert!(
        h.query_by_role_and_label(Role::CheckBox, "Problems 3")
            .is_some()
    );
    click_tab(&mut h, "Problems");
    for i in 0..3 {
        assert!(h.query_by_label_contains(&format!("note {i}")).is_some());
    }
}

#[test]
fn no_problems_says_so_and_shows_no_zero() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path());
    assert!(
        h.query_by_role_and_label(Role::CheckBox, "Problems")
            .is_some()
    );
    click_tab(&mut h, "Problems");
    assert!(h.query_by_label("No problems").is_some());
    assert!(h.query_by_label_contains("Problems 0").is_none());
}

#[test]
fn problems_follow_the_active_level_unless_all_is_on() {
    let mut list = ProblemList {
        rows: vec![
            problem("t", None, Some(Level::Terrain)),
            problem("g", None, Some(Level::Ground)),
            problem("none", None, None),
        ],
        ..ProblemList::default()
    };
    assert_eq!(list.shown(Level::Terrain), [0, 2]);
    assert_eq!(list.shown(Level::Ground), [1, 2]);
    list.all = true;
    assert_eq!(list.shown(Level::Terrain), [0, 1, 2]);
}

#[test]
fn a_row_goes_to_the_map() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path());
    run_until(&mut h, ready);
    h.run_steps(3);
    h.state_mut()
        .build_problems
        .push(problem("over there", Some((900.0, 400.0)), None));
    click_tab(&mut h, "Problems");
    h.get_by_label_contains("over there").click();
    h.run_steps(3);
    let c = centre(&h);
    assert!(
        (c.x - 900.0).abs() < 1e-6 && (c.y - 400.0).abs() < 1e-6,
        "{c:?}"
    );
}

#[test]
fn f8_goes_to_the_next_problem_and_centres_the_map() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path());
    run_until(&mut h, ready);
    h.run_steps(3);
    let span = h.state().views.active().focus().unwrap().span_m;
    h.state_mut()
        .build_problems
        .push(problem("one", Some((100.0, 200.0)), None));
    h.state_mut()
        .build_problems
        .push(problem("two", Some((300.0, 400.0)), None));
    h.run_steps(2);
    h.key_press(egui::Key::F8);
    h.run_steps(3);
    near(centre(&h).x, 100.0);
    near(centre(&h).y, 200.0);
    assert!((h.state().views.active().focus().unwrap().span_m - span).abs() < 1e-6);
    h.key_press(egui::Key::F8);
    h.run_steps(3);
    near(centre(&h).x, 300.0);
}

#[test]
fn f8_also_moves_the_other_view() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let (fake, handle) = FakeViewport::new();
    let views: Vec<Box<dyn Viewport>> = vec![Box::new(MapViewport::default()), Box::new(fake)];
    let mut h = state_harness(app(root.path(), &dir).with_views(views));
    run_until(&mut h, ready);
    h.state_mut().views.switch();
    h.state_mut()
        .build_problems
        .push(problem("one", Some((100.0, 200.0)), None));
    h.run_steps(3);
    h.key_press(egui::Key::F8);
    h.run_steps(3);
    near(handle.borrow().focus.unwrap().center.x, 100.0);
}

#[test]
fn f8_wraps_and_shift_f8_goes_back() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path());
    run_until(&mut h, ready);
    h.run_steps(3);
    for (name, x) in [("a", 10.0), ("b", 20.0), ("c", 30.0)] {
        h.state_mut()
            .build_problems
            .push(problem(name, Some((x, 5.0)), None));
    }
    h.run_steps(2);
    for want in [10.0, 20.0, 30.0, 10.0] {
        h.key_press(egui::Key::F8);
        h.run_steps(3);
        near(centre(&h).x, want);
    }
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::F8);
    h.run_steps(3);
    near(centre(&h).x, 30.0);
}

#[test]
fn f8_with_no_position_does_not_move_the_map() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path());
    run_until(&mut h, ready);
    h.run_steps(3);
    let before = centre(&h);
    h.state_mut()
        .build_problems
        .push(problem("nowhere", None, None));
    h.run_steps(2);
    h.key_press(egui::Key::F8);
    h.run_steps(3);
    assert_eq!(centre(&h), before);
}

#[test]
fn f8_with_no_problems_does_nothing() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path());
    run_until(&mut h, ready);
    h.run_steps(3);
    let before = centre(&h);
    h.key_press(egui::Key::F8);
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::F8);
    h.run_steps(3);
    assert_eq!(centre(&h), before);
}

#[test]
fn the_dock_folds_to_its_tabs_in_a_low_window() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    let mut h = Harness::builder()
        .with_size((1180.0, 500.0))
        .build_ui(|ui| app.show(ui));
    h.run_steps(4);
    assert_eq!(tab_labels_plain(&h), ["Build", "Problems"]);
    assert!(h.query_by_role_and_label(Role::Button, "Build").is_none());
}

fn tab_labels_plain(h: &Harness<'_>) -> Vec<String> {
    h.get_all_by_role(Role::Tab)
        .filter_map(|n| n.accesskit_node().label())
        .collect()
}

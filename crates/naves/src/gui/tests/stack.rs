// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The stack of levels, the panels by level, the band and the status line.

use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use naves_core::surface::{SurfaceClass, SurfaceGrid};

use super::*;
use crate::gui::layout::{Fold, fold_for};
use crate::gui::levels::Level;
use crate::gui::stack::ground_shares;
use crate::gui::testing::FAKE_LEVEL;

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
    add_sources(&dir);
    let h = state_harness(app(root, &dir));
    (h, dir)
}

fn y_of(h: &Harness<'_, NavesApp>, label: &str) -> f32 {
    h.get_by_label(label).rect().min.y
}

#[test]
fn only_built_levels_are_shown() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    for label in ["0 Project", "1 Terrain", "2 Ground"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    assert!(h.query_by_label("3 Water").is_none());
}

#[test]
fn the_stack_is_bottom_up() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    assert!(y_of(&h, "2 Ground") < y_of(&h, "1 Terrain"));
    assert!(y_of(&h, "1 Terrain") < y_of(&h, "0 Project"));
}

#[test]
fn an_added_level_takes_its_place_without_changing_the_digits() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut h = state_harness(app(root.path(), &dir).with_extra_level(&FAKE_LEVEL));
    h.run_steps(3);
    assert!(y_of(&h, "6 Buildings") < y_of(&h, "2 Ground"));
    assert!(y_of(&h, "2 Ground") < y_of(&h, "1 Terrain"));
    assert!(h.query_by_label("3 Water").is_none());
}

#[test]
fn clicking_a_row_makes_it_active() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    assert_eq!(h.state().panel.active, Level::Project);
    h.get_by_label("1 Terrain").click();
    h.run_steps(3);
    assert_eq!(h.state().panel.active, Level::Terrain);
}

#[test]
fn the_left_panel_follows_the_level() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    run_until(&mut h, ready);
    h.run_steps(3);
    assert!(h.query_by_label("Name").is_some());
    assert!(h.query_by_label("Lowest point").is_none());
    h.get_by_label("1 Terrain").click();
    h.run_steps(3);
    assert!(h.query_by_label("Lowest point").is_some());
    assert!(h.query_by_label("Name").is_none());
    h.get_by_label("2 Ground").click();
    h.run_steps(3);
    assert!(h.query_by_label("Grass").is_some());
}

#[test]
fn level_0_keeps_the_settings_working() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, dir) = started(root.path());
    h.run_steps(3);
    h.get_by_label("1 Terrain").click();
    h.run_steps(3);
    h.get_by_label("0 Project").click();
    h.run_steps(3);
    let field = h
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .next()
        .unwrap();
    field.focus();
    field.type_text("X");
    h.run_steps(8);
    assert_eq!(Project::load(&dir).unwrap().name, "TestX");
}

#[test]
fn relief_is_the_default() {
    let root = tempfile::tempdir().unwrap();
    let (h, _) = started(root.path());
    assert_eq!(h.state().panel.base, Overlay::Hillshade);
}

#[test]
fn base_maps_are_exclusive() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    for (overlay, name) in crate::gui::stack::BASE_MAPS {
        h.get_by_label(name).click();
        h.run_steps(2);
        assert_eq!(h.state().panel.base, overlay, "{name}");
    }
}

#[test]
fn ground_unfolds_its_legend() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    assert!(h.query_by_label("Soil").is_none());
    h.get_by_label("Ground").click();
    h.run_steps(3);
    assert!(h.query_by_label("Soil").is_some());
}

#[test]
fn imagery_says_what_checkered_means() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    h.get_by_label("Imagery").click();
    h.run_steps(3);
    assert!(h.query_by_label_contains("checkered").is_some());
}

#[test]
fn the_band_has_the_level_s_colour() {
    let mut panel = crate::gui::stack::LayerPanel::default();
    for level in Level::ALL {
        panel.active = level;
        assert_eq!(panel.band_colour(), level.colour());
    }
}

#[test]
fn the_status_line_follows_the_level() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    assert!(
        h.query_by_label_contains("Press 1 to read the terrain")
            .is_some()
    );
    h.get_by_label("2 Ground").click();
    h.run_steps(3);
    assert!(
        h.query_by_label_contains("Press 1 to read the terrain")
            .is_none()
    );
    assert!(h.query_by_label_contains("read the ground").is_some());
}

#[test]
fn switching_levels_does_not_touch_the_project_file() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, dir) = started(root.path());
    h.run_steps(3);
    let before = std::fs::read(dir.join("naves.toml")).unwrap();
    for label in ["1 Terrain", "2 Ground", "0 Project", "Slope"] {
        h.get_by_label(label).click();
        h.run_steps(3);
    }
    assert_eq!(std::fs::read(dir.join("naves.toml")).unwrap(), before);
}

#[test]
fn ground_shares_add_up_to_one() {
    let grid = SurfaceGrid::from_fn(64, 10.0, |col, row| {
        SurfaceClass::ALL[((col / 8 + row / 16) % 4) as usize]
    });
    let shares = ground_shares(&grid, 640.0);
    assert!(
        (shares.iter().sum::<f64>() - 1.0).abs() < 1e-9,
        "{shares:?}"
    );
    assert!(shares.iter().all(|s| *s > 0.0));
    let flat = SurfaceGrid::from_fn(8, 10.0, |_, _| SurfaceClass::Rock);
    let flat = ground_shares(&flat, 80.0);
    assert!((flat[1] - 1.0).abs() < 1e-12 && flat[0] + flat[2] + flat[3] < 1e-12);
}

#[test]
fn terrain_numbers_show_the_heights() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    run_until(&mut h, ready);
    let HeightmapState::Ready(data) = &h.state().heightmap else {
        unreachable!()
    };
    let (low, high) = data.heightmap.min_max();
    h.get_by_label("1 Terrain").click();
    h.run_steps(3);
    assert!(h.query_by_label(&format!("{low:.0} m")).is_some());
    assert!(h.query_by_label(&format!("{high:.0} m")).is_some());
}

#[test]
fn without_data_the_panels_say_what_to_do() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("empty-proj");
    std::fs::create_dir_all(&dir).unwrap();
    Project::new("Empty", 1.0, 2.0, 1000.0).save(&dir).unwrap();
    let mut h = state_harness(app(root.path(), &dir));
    for label in ["1 Terrain", "2 Ground"] {
        h.run_steps(3);
        h.get_by_label(label).click();
        h.run_steps(3);
        assert!(
            h.query_all_by_label_contains("Add elevation files").count() >= 2,
            "{label}: the panel and the map both say it"
        );
    }
}

#[test]
fn fold_rules() {
    let big = egui::vec2(1180.0, 760.0);
    assert_eq!(
        fold_for(big),
        Fold {
            dock: false,
            left: false
        }
    );
    assert_eq!(
        fold_for(egui::vec2(1180.0, 540.0)),
        Fold {
            dock: true,
            left: false
        }
    );
    assert_eq!(
        fold_for(egui::vec2(880.0, 760.0)),
        Fold {
            dock: false,
            left: true
        }
    );
    assert_eq!(
        fold_for(egui::vec2(200.0, 150.0)),
        Fold {
            dock: true,
            left: true
        }
    );
    assert_eq!(
        fold_for(egui::vec2(900.0, 560.0)),
        Fold {
            dock: false,
            left: false
        }
    );
}

#[test]
fn a_tiny_window_does_not_panic() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut app = app(root.path(), &dir);
    let mut h = Harness::builder()
        .with_size((200.0, 150.0))
        .build_ui(|ui| app.show(ui));
    h.run_steps(8);
    drop(h);
    wait_for_the_job(&mut app);
    let mut h = Harness::builder()
        .with_size((200.0, 150.0))
        .build_ui(|ui| app.show(ui));
    h.run_steps(8);
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The settings window, the shortcut bindings and the per-user file behind them.

use std::collections::BTreeMap;

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use naves_core::prefs::{Preferences, ToolPaths, default_path};
use naves_core::render::MapLayer;
use naves_export::tools::{Origin, ToolOrigin};

use super::*;
use crate::gui::levels::Level;
use crate::gui::shortcuts::{Action, Binding, Bindings, SHORTCUTS, ToolId};
use crate::gui::testing::{FAKE_LEVEL, FakeViewport};
use crate::gui::viewport::Viewport;
use crate::gui::viewport::map::MapViewport;

fn settings_file(root: &Path) -> PathBuf {
    default_path(&root.join("config/naves"))
}

fn write_settings(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// An app that keeps its settings in a file below `root`.
fn started(root: &Path, text: Option<&str>) -> (Harness<'static, NavesApp>, PathBuf) {
    let dir = project(root);
    let path = settings_file(root);
    if let Some(text) = text {
        write_settings(&path, text);
    }
    let app = app(root, &dir).with_preferences(path.clone());
    let mut h = state_harness(app);
    h.run_steps(3);
    (h, path)
}

fn open_window(h: &mut Harness<'_, NavesApp>) {
    h.get_by_role_and_label(Role::Button, "Settings").click();
    // the window takes a few frames to find its size
    h.run_steps(8);
}

fn tab(h: &mut Harness<'_, NavesApp>, name: &str) {
    h.get_by_role_and_label(Role::Tab, name).click();
    h.run_steps(3);
}

fn is_open(h: &Harness<'_, NavesApp>) -> bool {
    h.query_by_label_contains("These apply to every project")
        .is_some()
}

fn rebind(h: &mut Harness<'_, NavesApp>, row: &str, key: Key, modifiers: Modifiers) {
    h.get_by_label_contains(&format!("{row} shortcut")).click();
    h.run_steps(2);
    h.key_press_modifiers(modifiers, key);
    h.run_steps(3);
}

fn saved(path: &Path) -> Preferences {
    Preferences::load(path).unwrap()
}

fn level(h: &Harness<'_, NavesApp>) -> Level {
    h.state().panel.active
}

// ---- opening and closing ----

#[test]
fn the_button_and_f2_open_the_window() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path(), None);
    assert!(!is_open(&h));
    open_window(&mut h);
    assert!(is_open(&h));
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(!is_open(&h));
    h.key_press(Key::F2);
    h.run_steps(3);
    assert!(is_open(&h));
}

#[test]
fn esc_closes_it() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path(), None);
    open_window(&mut h);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(!is_open(&h));
}

#[test]
fn shortcuts_do_not_fire_while_it_is_open() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path(), None);
    open_window(&mut h);
    h.key_press(Key::Num1);
    h.key_press_modifiers(Modifiers::CTRL, Key::B);
    h.run_steps(3);
    assert_eq!(level(&h), Level::Project);
    assert!(h.state().log.is_empty(), "no build was started");
}

#[test]
fn no_setting_changes_the_project_file() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path(), None);
    let before = std::fs::read(root.path().join("proj/naves.toml")).unwrap();
    open_window(&mut h);
    rebind(&mut h, "Build", Key::F9, Modifiers::NONE);
    tab(&mut h, "Map");
    h.get_by_label("Invert the zoom direction of the wheel")
        .click();
    h.run_steps(3);
    assert_eq!(
        std::fs::read(root.path().join("proj/naves.toml")).unwrap(),
        before
    );
}

#[test]
fn tabs_are_in_the_spec_s_order() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path(), None);
    open_window(&mut h);
    let y = |name: &str| h.get_by_role_and_label(Role::Tab, name).rect().min.y;
    assert!(y("Keyboard") < y("Map") && y("Map") < y("External tools"));
}

#[test]
fn rows_are_grouped_by_level_in_the_stack_s_order_and_only_what_exists() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let path = settings_file(root.path());
    let app = app(root.path(), &dir)
        .with_extra_level(&FAKE_LEVEL)
        .with_preferences(path);
    let mut h = state_harness(app);
    h.run_steps(3);
    open_window(&mut h);
    let y = |label: &str| h.get_by_label(label).rect().min.y;
    assert!(y("Everywhere") < y("Buildings"));
    assert!(h.query_by_label("Make a composition").is_some());
    assert!(
        h.query_by_label("Terrain").is_none(),
        "Terrain has no keys of its own yet"
    );
    assert!(h.query_by_label("Undo").is_none(), "not built");
    assert!(h.query_by_label("Go to Terrain").is_some());
}

// ---- rebinding ----

#[test]
fn pressing_a_key_rebinds_and_saves() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, path) = started(root.path(), None);
    open_window(&mut h);
    h.get_by_label_contains("Build shortcut").click();
    h.run_steps(2);
    assert!(h.query_by_label_contains("Press a key").is_some());
    h.key_press_modifiers(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::B);
    h.run_steps(3);
    assert_eq!(saved(&path).shortcuts["build"], "Ctrl+Shift+B");
    assert!(h.query_by_label_contains("changed").is_some());
}

#[test]
fn the_new_key_works_and_the_old_one_does_not() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path(), None);
    open_window(&mut h);
    rebind(&mut h, "Build", Key::F9, Modifiers::NONE);
    h.key_press(Key::Escape);
    h.run_steps(3);
    h.key_press_modifiers(Modifiers::CTRL, Key::B);
    h.run_steps(3);
    assert!(h.state().log.is_empty(), "the old key does nothing");
    h.key_press(Key::F9);
    h.run_steps(3);
    assert!(!h.state().log.is_empty(), "the new key builds");
}

#[test]
fn esc_cancels_a_capture() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, path) = started(root.path(), None);
    open_window(&mut h);
    h.get_by_label_contains("Build shortcut").click();
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(is_open(&h), "the window stays");
    assert!(h.state().prefs_window.as_ref().unwrap().capturing.is_none());
    assert!(!path.exists() || saved(&path).shortcuts.is_empty());
}

#[test]
fn esc_enter_and_f1_cannot_be_changed() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path(), None);
    open_window(&mut h);
    for row in [
        "Cancel and go back to Select",
        "Show all shortcuts for this level",
    ] {
        let name = format!("{row} shortcut");
        let button = h.get_by_label_contains(&name);
        assert!(button.accesskit_node().is_disabled(), "{row}");
    }
    // and no other action can take them
    h.get_by_label_contains("Build shortcut").click();
    h.run_steps(2);
    for key in [Key::Enter, Key::F1] {
        h.key_press(key);
        h.run_steps(2);
        assert!(
            h.state().prefs_window.as_ref().unwrap().capturing.is_some(),
            "{key:?}"
        );
    }
    assert!(h.query_by_label_contains("cannot be assigned").is_some());
}

#[test]
fn tab_and_shift_tab_cannot_be_assigned() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path(), None);
    open_window(&mut h);
    h.get_by_label_contains("Build shortcut").click();
    h.run_steps(2);
    h.key_press(Key::Tab);
    h.run_steps(2);
    assert!(
        h.query_by_label_contains("Tab and Shift+Tab cannot be assigned")
            .is_some()
    );
    assert!(
        h.state().prefs_window.as_ref().unwrap().capturing.is_some(),
        "it keeps listening"
    );
    h.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
    h.run_steps(2);
    assert!(h.state().prefs_window.as_ref().unwrap().capturing.is_some());
    // Ctrl+Tab is an ordinary key
    let (bindings, _) = Bindings::new(&BTreeMap::from([(
        "build".to_string(),
        "Ctrl+Tab".to_string(),
    )]));
    assert_eq!(
        bindings
            .key_for(Action::Build, Level::Project)
            .unwrap()
            .to_string(),
        "Ctrl+Tab"
    );
}

#[test]
fn a_used_key_asks_before_replacing() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, path) = started(root.path(), None);
    open_window(&mut h);
    rebind(&mut h, "Build", Key::R, Modifiers::NONE);
    assert!(h.query_by_label("R is Scale in every level.").is_some());
    assert!(h.query_by_role_and_label(Role::Button, "Replace").is_some());
    assert!(
        !path.exists() || saved(&path).shortcuts.is_empty(),
        "nothing changed yet"
    );
}

#[test]
fn cancel_changes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, path) = started(root.path(), None);
    open_window(&mut h);
    rebind(&mut h, "Build", Key::R, Modifiers::NONE);
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run_steps(3);
    assert!(h.query_by_label("R is Scale in every level.").is_none());
    assert!(!path.exists() || saved(&path).shortcuts.is_empty());
    assert_eq!(
        h.state()
            .bindings
            .key_for(Action::Build, Level::Project)
            .unwrap()
            .to_string(),
        "Ctrl+B"
    );
}

#[test]
fn replace_leaves_the_other_action_with_no_key() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, path) = started(root.path(), None);
    open_window(&mut h);
    rebind(&mut h, "Build", Key::H, Modifiers::NONE);
    assert!(
        h.query_by_label("H is Hide the active level in every level.")
            .is_some()
    );
    h.get_by_role_and_label(Role::Button, "Replace").click();
    h.run_steps(3);
    let prefs = saved(&path);
    assert_eq!(prefs.shortcuts["build"], "H");
    assert_eq!(prefs.shortcuts["level.hide"], "");
    assert!(h.query_by_label_contains(", no key").is_some());
}

#[test]
fn the_same_key_in_two_levels_is_no_conflict() {
    let (bindings, _) = Bindings::new(&BTreeMap::new());
    let smooth = SHORTCUTS
        .iter()
        .find(|s| s.id == "tool.terrain.smooth")
        .unwrap();
    // K is Roads' cut and Vegetation's cut-out: neither is Terrain's
    let k = Binding {
        key: Key::K,
        modifiers: Modifiers::NONE,
    };
    assert!(bindings.conflict(k, smooth.id).is_none());
    let cut = SHORTCUTS
        .iter()
        .find(|s| s.id == "tool.vegetation.cut")
        .unwrap();
    let roads_k = SHORTCUTS.iter().find(|s| s.id == "tool.roads.cut").unwrap();
    assert_eq!(roads_k.binding, k);
    assert!(
        bindings.conflict(roads_k.binding, cut.id).is_none(),
        "different levels"
    );
}

#[test]
fn a_global_key_conflicts_with_every_level() {
    let (bindings, _) = Bindings::new(&BTreeMap::new());
    let q = Binding {
        key: Key::Q,
        modifiers: Modifiers::NONE,
    };
    let hit = bindings.conflict(q, "tool.terrain.smooth").unwrap();
    assert_eq!(hit.action, Action::Tool(ToolId::SELECT));
    let own = bindings.conflict(q, "tool.select");
    assert!(own.is_none(), "an action does not conflict with itself");
}

#[test]
fn reset_restores_one_and_reset_all_restores_all() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, path) = started(root.path(), None);
    open_window(&mut h);
    rebind(&mut h, "Build", Key::F9, Modifiers::NONE);
    rebind(&mut h, "Next problem", Key::F6, Modifiers::NONE);
    assert_eq!(saved(&path).shortcuts.len(), 2);
    h.get_all_by_role_and_label(Role::Button, "Reset")
        .next()
        .unwrap()
        .click();
    h.run_steps(3);
    assert_eq!(saved(&path).shortcuts.len(), 1);
    h.get_by_role_and_label(Role::Button, "Reset all to defaults")
        .click();
    h.run_steps(3);
    assert!(saved(&path).shortcuts.is_empty());
}

#[test]
fn search_filters_the_rows() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path(), None);
    open_window(&mut h);
    assert!(h.query_by_label("Go to Terrain").is_some());
    // the search field is the last one: the project's own fields are behind the window
    let field = h.get_all_by_role(Role::TextInput).last().unwrap();
    field.focus();
    field.type_text("build");
    h.run_steps(3);
    assert!(h.query_by_label("Go to Terrain").is_none());
    assert!(h.query_by_label_contains("Build shortcut").is_some());
}

#[test]
fn tooltips_and_f1_show_the_new_key() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path(), None);
    open_window(&mut h);
    rebind(&mut h, "Go to Terrain", Key::F6, Modifiers::NONE);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(
        crate::gui::stack::tooltip(Level::Terrain, &h.state().bindings),
        "Go to Terrain (F6)"
    );
    h.key_press(Key::F1);
    h.run_steps(3);
    assert!(h.query_by_label("F6").is_some());
}

#[test]
fn a_new_default_reaches_a_file_that_does_not_mention_it() {
    let (bindings, notes) = Bindings::new(&BTreeMap::from([
        ("build".to_string(), "F9".to_string()),
        ("tool.from_the_future".to_string(), "Ctrl+J".to_string()),
    ]));
    assert_eq!(notes, [] as [String; 0]);
    assert_eq!(
        bindings
            .key_for(Action::Build, Level::Project)
            .unwrap()
            .to_string(),
        "F9"
    );
    assert_eq!(
        bindings
            .key_for(Action::Help, Level::Project)
            .unwrap()
            .to_string(),
        "F1"
    );
    assert_eq!(
        bindings.overrides(),
        BTreeMap::from([("build".to_string(), "F9".to_string())])
    );
}

#[test]
fn bindings_survive_text_and_back() {
    for s in SHORTCUTS {
        let text = s.binding.to_string();
        assert_eq!(Binding::parse(&text), Some(s.binding), "{text}");
    }
    assert!(Binding::parse("Ctrl+Banana").is_none());
    assert!(Binding::parse("").is_none());
    for key in Key::ALL {
        let b = Binding {
            key: *key,
            modifiers: Modifiers::CTRL,
        };
        if let Some(back) = Binding::parse(&b.to_string()) {
            assert_eq!(back, b, "{key:?}");
        }
    }
}

// ---- the Map tab ----

#[test]
fn invert_zoom_inverts_the_wheel() {
    let normal = crate::gui::viewport::zoom_factor(10.0, false);
    let inverted = crate::gui::viewport::zoom_factor(10.0, true);
    assert!(normal < 1.0 && inverted > 1.0, "{normal} {inverted}");
    assert!((normal * inverted - 1.0).abs() < 1e-12);
}

#[test]
fn fade_percent_changes_the_other_levels() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let path = settings_file(root.path());
    write_settings(
        &path,
        "[map]\nfade_percent = 25\nstart_layers = [\"too_steep_to_walk\"]\n",
    );
    let (fake, handle) = FakeViewport::new();
    let views: Vec<Box<dyn Viewport>> = vec![Box::new(MapViewport::default()), Box::new(fake)];
    let mut h = state_harness(
        app(root.path(), &dir)
            .with_views(views)
            .with_preferences(path),
    );
    while !matches!(h.state().heightmap, HeightmapState::Ready(_)) {
        h.step();
        std::thread::yield_now();
    }
    h.state_mut().views.switch();
    h.run_steps(3);
    let layers = handle.borrow().layers.clone();
    assert_eq!(layers.len(), 1);
    assert!((layers[0].opacity - 0.25).abs() < 1e-6, "{layers:?}");
}

#[test]
fn start_layers_are_used_when_a_project_opens() {
    let root = tempfile::tempdir().unwrap();
    let (h, _) = started(
        root.path(),
        Some("[map]\nstart_base = \"slope\"\nstart_layers = [\"too_steep_to_walk\"]\n"),
    );
    assert_eq!(h.state().panel.base, naves_core::render::Overlay::Slope);
    assert!(h.state().panel.on.contains(&MapLayer::TooSteepToWalk));
}

#[test]
fn a_start_layer_that_does_not_exist_is_ignored() {
    let root = tempfile::tempdir().unwrap();
    let (h, _) = started(
        root.path(),
        Some("[map]\nstart_base = \"nonsense\"\nstart_layers = [\"no_such_layer\"]\n"),
    );
    assert_eq!(h.state().panel.base, naves_core::render::Overlay::Hillshade);
    assert!(h.state().panel.on.is_empty());
}

#[test]
fn the_zoom_factor_follows_the_scale() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, path) = started(root.path(), Some("[map]\ninterface_scale = 1.5\n"));
    h.run_steps(2);
    assert!((h.ctx.zoom_factor() - 1.5).abs() < 1e-6);
    assert!(path.exists());
}

#[test]
fn unfolding_without_animation_sets_the_animation_time_to_zero() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path(), Some("[map]\nno_unfold_animation = true\n"));
    h.run_steps(2);
    assert!(h.ctx.global_style().animation_time.abs() < 1e-9);
}

// ---- the file ----

#[test]
fn a_damaged_file_shows_its_error_and_is_not_overwritten() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, path) = started(root.path(), Some("this is [not toml"));
    open_window(&mut h);
    assert!(h.query_by_label_contains("settings.toml").is_some());
    assert!(
        h.query_by_label_contains("not saved").is_some()
            || h.query_by_label_contains("Not saved").is_some()
    );
    rebind(&mut h, "Build", Key::F9, Modifiers::NONE);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "this is [not toml");
    assert_eq!(
        h.state()
            .bindings
            .key_for(Action::Build, Level::Project)
            .unwrap()
            .to_string(),
        "F9",
        "the change holds for the session"
    );
}

#[test]
fn a_bad_key_name_keeps_the_default_and_says_so() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path(), Some("[shortcuts]\nbuild = \"Ctrl+Banana\"\n"));
    open_window(&mut h);
    assert_eq!(
        h.state()
            .bindings
            .key_for(Action::Build, Level::Project)
            .unwrap()
            .to_string(),
        "Ctrl+B"
    );
    assert!(h.query_by_label_contains("Ctrl+Banana").is_some());
}

#[test]
fn a_duplicate_in_the_file_is_shown() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path(), Some("[shortcuts]\nhelp = \"Ctrl+B\"\n"));
    open_window(&mut h);
    assert!(h.query_by_label_contains("Ctrl+B is used for").is_some());
}

#[test]
fn an_unwritable_file_is_reported() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let (mut h, path) = started(root.path(), Some("[map]\nfade_percent = 30\n"));
    open_window(&mut h);
    let folder = path.parent().unwrap().to_path_buf();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
    rebind(&mut h, "Build", Key::F9, Modifiers::NONE);
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
    let root_user = std::fs::read_to_string("/proc/self/status")
        .is_ok_and(|s| s.lines().any(|l| l.starts_with("Uid:\t0\t")));
    if !root_user {
        assert!(h.query_by_label_contains("Not saved:").is_some());
    }
}

#[test]
fn tests_never_touch_the_user_s_file() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut h = state_harness(app(root.path(), &dir));
    h.run_steps(3);
    assert!(h.state().prefs_path.is_none());
    open_window(&mut h);
    rebind(&mut h, "Build", Key::F9, Modifiers::NONE);
    assert!(!root.path().join("config").exists());
}

// ---- external tools ----

fn fake_origins(_: &ToolPaths) -> Vec<ToolOrigin> {
    let row = |tool, path: Option<&str>, from| ToolOrigin {
        tool,
        path: path.map(PathBuf::from),
        from,
    };
    vec![
        row("armake2", Some("/opt/armake2"), Origin::Settings),
        row("Steam", Some("/home/x/.steam/steam"), Origin::Found),
        row("DayZ Tools", Some("/steam/DayZ Tools"), Origin::Environment),
        row("DayZ", None, Origin::Missing),
        row("DayZ Server", None, Origin::Missing),
        row("Proton", Some("/steam/proton"), Origin::Found),
    ]
}

#[test]
fn external_tools_show_the_origin() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let app = app(root.path(), &dir)
        .with_preferences(settings_file(root.path()))
        .with_tool_origins(fake_origins);
    let mut h = state_harness(app);
    h.run_steps(3);
    open_window(&mut h);
    tab(&mut h, "External tools");
    assert!(h.query_by_label_contains("/opt/armake2").is_some());
    assert_eq!(h.query_all_by_label("From the settings").count(), 1);
    assert_eq!(
        h.query_all_by_label("From an environment variable").count(),
        1
    );
    assert_eq!(h.query_all_by_label("Found by searching").count(), 2);
    assert_eq!(h.query_all_by_label("Not found").count(), 2);
    assert!(
        h.query_by_label("DayZ Server").is_some(),
        "a build needs the server too"
    );
}

#[test]
fn changing_a_path_is_used_by_the_next_build() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let fakes = naves_export::testing::FakeEnv::new(&root.path().join("fake")).unwrap();
    let path = settings_file(root.path());
    write_settings(
        &path,
        &format!(
            "[tools]\nsteam_root = {:?}\ndayz_server = {:?}\n",
            fakes.steam, fakes.server
        ),
    );
    let reread = path.clone();
    let locate = Box::new(move || {
        let prefs = Preferences::load(&reread).map_err(|e| e.to_string())?;
        naves_export::tools::locate_preferring_with(&prefs.tools, &|_| None, None, None)
            .map_err(|e| e.to_string())
    });
    let app = NavesApp::open(&dir, locate)
        .unwrap()
        .with_preferences(path.clone())
        .with_tool_origins(fake_origins);
    let mut h = state_harness(app);
    h.run_steps(3);
    open_window(&mut h);
    tab(&mut h, "External tools");
    // without armake2 a build cannot start
    // the path of armake2 is the first of the six fields of the tab, the last in the tree are the tab's
    let fields: Vec<_> = h.get_all_by_role(Role::TextInput).collect();
    let field = &fields[fields.len() - 6];
    field.focus();
    field.type_text(fakes.armake2.to_str().unwrap());
    h.run_steps(4);
    assert_eq!(
        saved(&path).tools.armake2.as_deref(),
        Some(fakes.armake2.as_path())
    );
    h.key_press(Key::Escape);
    h.run_steps(3);
    h.key_press_modifiers(Modifiers::CTRL, Key::B);
    let deadline = std::time::Instant::now() + std::time::Duration::from_mins(1);
    while h.query_by_label_contains("Mod: ").is_none() && std::time::Instant::now() < deadline {
        h.step();
        std::thread::yield_now();
    }
    // the log is in the Build tab
    assert!(h.query_by_label_contains("Mod: ").is_some());
}

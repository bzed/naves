// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The per-user settings file.

#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::prefs::{Preferences, PrefsError, default_path};
use naves_core::project::Project;

fn dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn a_missing_file_gives_the_defaults() {
    let d = dir();
    let prefs = Preferences::load(&default_path(d.path())).unwrap();
    assert_eq!(prefs, Preferences::default());
    assert!(prefs.shortcuts.is_empty());
    assert!(!prefs.map.invert_zoom);
    assert_eq!(prefs.map.fade_percent, 40);
    assert!((prefs.map.interface_scale - 1.0).abs() < f32::EPSILON);
    assert_eq!(prefs.map.start_base, "hillshade");
    assert!(prefs.tools.armake2.is_none());
}

#[test]
fn values_round_trip() {
    let d = dir();
    let path = default_path(d.path());
    let mut prefs = Preferences::default();
    prefs
        .shortcuts
        .insert("build".into(), "Ctrl+Shift+B".into());
    prefs.shortcuts.insert("tool.move".into(), String::new());
    prefs.map.invert_zoom = true;
    prefs.map.fade_percent = 25;
    prefs.map.interface_scale = 1.5;
    prefs.map.no_unfold_animation = true;
    prefs.map.start_base = "slope".into();
    prefs.map.start_layers = vec!["too_steep_to_walk".into()];
    prefs.tools.armake2 = Some("/opt/armake2".into());
    prefs.tools.dayz_server = Some("/srv/dayz".into());
    prefs.save(&path).unwrap();
    assert_eq!(Preferences::load(&path).unwrap(), prefs);
}

#[test]
fn only_changed_values_are_written() {
    let d = dir();
    let path = default_path(d.path());
    Preferences::default().save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        !text.contains("fade_percent") && !text.contains("invert_zoom"),
        "{text}"
    );
    let mut prefs = Preferences::default();
    prefs.map.invert_zoom = true;
    prefs.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("invert_zoom = true") && !text.contains("fade_percent"),
        "{text}"
    );
}

#[test]
fn unknown_ids_survive_a_save() {
    let d = dir();
    let path = default_path(d.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "[shortcuts]\n\"tool.from_the_future\" = \"Ctrl+J\"\nbuild = \"F9\"\n",
    )
    .unwrap();
    let mut prefs = Preferences::load(&path).unwrap();
    prefs.shortcuts.insert("help".into(), "F3".into());
    prefs.save(&path).unwrap();
    let again = Preferences::load(&path).unwrap();
    assert_eq!(again.shortcuts["tool.from_the_future"], "Ctrl+J");
    assert_eq!(again.shortcuts["build"], "F9");
    assert_eq!(again.shortcuts["help"], "F3");
}

#[test]
fn unknown_keys_survive_a_save() {
    let d = dir();
    let path = default_path(d.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "future = 1\n[map]\ninvert_zoom = true\nfuture_map = \"x\"\n[tools]\nfuture_tool = \"/x\"\n[ai]\nmode = \"approve\"\n",
    )
    .unwrap();
    let prefs = Preferences::load(&path).unwrap();
    prefs.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    for needle in [
        "future = 1",
        "future_map = \"x\"",
        "future_tool = \"/x\"",
        "[ai]",
        "mode = \"approve\"",
        "invert_zoom = true",
    ] {
        assert!(text.contains(needle), "{needle} is lost: {text}");
    }
}

#[test]
fn a_damaged_file_is_an_error_and_is_not_overwritten() {
    let d = dir();
    let path = default_path(d.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "this is [not toml").unwrap();
    let err = Preferences::load(&path).unwrap_err();
    assert!(matches!(err, PrefsError::Syntax { .. }), "{err}");
    assert!(err.to_string().contains("settings.toml"), "{err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "this is [not toml");
}

#[test]
fn saving_is_atomic() {
    let d = dir();
    let path = default_path(d.path());
    let mut prefs = Preferences::default();
    prefs.map.invert_zoom = true;
    prefs.save(&path).unwrap();
    prefs.map.invert_zoom = false;
    prefs.map.fade_percent = 10;
    prefs.save(&path).unwrap();
    let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["settings.toml"], "no temporary file is left");
    assert_eq!(Preferences::load(&path).unwrap().map.fade_percent, 10);
}

#[test]
fn an_unwritable_folder_is_an_error_and_keeps_the_file() {
    use std::os::unix::fs::PermissionsExt;
    let d = dir();
    let path = default_path(d.path());
    let mut prefs = Preferences::default();
    prefs.map.fade_percent = 20;
    prefs.save(&path).unwrap();
    let folder = path.parent().unwrap();
    std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o555)).unwrap();
    prefs.map.fade_percent = 30;
    let result = prefs.save(&path);
    std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o755)).unwrap();
    if nix_is_root() {
        return; // root writes anywhere; the case cannot be made
    }
    assert!(matches!(result, Err(PrefsError::Io { .. })), "{result:?}");
    assert_eq!(Preferences::load(&path).unwrap().map.fade_percent, 20);
}

fn nix_is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .is_ok_and(|s| s.lines().any(|l| l.starts_with("Uid:\t0\t")))
}

#[test]
fn out_of_range_values_are_clamped() {
    let parse = |text: &str| Preferences::parse(text).unwrap();
    assert_eq!(parse("[map]\nfade_percent = 200\n").map.fade_percent, 100);
    assert!((parse("[map]\ninterface_scale = 0.0\n").map.interface_scale - 0.5).abs() < 1e-6);
    assert!((parse("[map]\ninterface_scale = 9.0\n").map.interface_scale - 3.0).abs() < 1e-6);
    assert!((parse("[map]\ninterface_scale = nan\n").map.interface_scale - 1.0).abs() < 1e-6);
    let mut prefs = Preferences::default();
    prefs.map.fade_percent = 250;
    prefs.map.interface_scale = f32::NAN;
    let d = dir();
    let path = default_path(d.path());
    prefs.save(&path).unwrap();
    let back = Preferences::load(&path).unwrap();
    assert_eq!(back.map.fade_percent, 100);
    assert!((back.map.interface_scale - 1.0).abs() < 1e-6);
}

#[test]
fn the_project_file_format_is_untouched() {
    // the settings never reach naves.toml: a project with none of them still parses, and says nothing of them
    let text = toml::to_string(&Project::new("X", 1.0, 2.0, 1000.0)).unwrap();
    assert!(!text.contains("shortcuts") && !text.contains("invert_zoom"));
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

// SPDX-License-Identifier: AGPL-3.0-or-later

//! The command palette: one key, then words, then Enter.

use std::path::Path;

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use naves_core::prefs::default_path;

use super::*;
use crate::gui::levels::Level;
use crate::gui::shortcuts::ToolId;

/// The settings file of the test app, below `root`.
fn settings_file(root: &Path) -> std::path::PathBuf {
    default_path(&root.join("config/naves"))
}

/// An app with its settings in a file below `root`, with `text` in the settings file.
fn started(root: &Path, text: Option<&str>) -> Harness<'static, NavesApp> {
    let dir = project(root);
    let path = settings_file(root);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    if let Some(text) = text {
        std::fs::write(&path, text).unwrap();
    }
    let app = app(root, &dir).with_preferences(path);
    state_harness(app)
}

/// Types `words` into whatever text field has the keyboard (the palette's search).
fn type_into_focused(h: &Harness<'_, NavesApp>, words: &str) {
    let focused = h
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .find(egui_kittest::Node::is_focused)
        .expect("the palette's search has the keyboard");
    focused.type_text(words);
}

fn open_and_pick(h: &mut Harness<'_, NavesApp>, words: &str) {
    h.key_press_modifiers(Modifiers::CTRL, Key::P);
    h.run_steps(3);
    type_into_focused(h, words);
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
}

#[test]
fn the_default_key_opens_it_and_enter_selects() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), None);
    open_and_pick(&mut h, "terrain");
    assert_eq!(h.state().panel.active, Level::Terrain);
}

#[test]
fn it_opens_with_the_key_from_the_settings() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(
        root.path(),
        Some("[shortcuts]\ncommand_palette = \"Ctrl+Shift+P\"\n"),
    );
    // the default key must not open it
    h.key_press_modifiers(Modifiers::CTRL, Key::P);
    h.run_steps(3);
    h.key_press_modifiers(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::P);
    h.run_steps(3);
    type_into_focused(&h, "terrain");
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().panel.active, Level::Terrain);
}

#[test]
fn typing_filters_and_enter_selects() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), None);
    open_and_pick(&mut h, "grou");
    assert_eq!(h.state().panel.active, Level::Ground);
}

#[test]
fn selecting_build_starts_a_build() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), None);
    open_and_pick(&mut h, "build");
    assert!(
        h.state().building(),
        "the palette's Build must start a build"
    );
}

#[test]
fn escape_closes_it_and_changes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path(), None);
    h.key_press_modifiers(Modifiers::CTRL, Key::P);
    h.run_steps(3);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().panel.active, Level::Project);
    assert_eq!(h.state().tool, ToolId::SELECT);
    // the keyboard reaches the map again
    h.key_press(Key::Num2);
    h.run_steps(3);
    assert_eq!(h.state().panel.active, Level::Ground);
}

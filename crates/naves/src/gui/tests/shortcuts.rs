// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The one table of keyboard shortcuts, and what the keys do.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;
use crate::gui::levels::Level;
use crate::gui::shortcuts::{Action, Binding, Bindings, SHORTCUTS, Scope, ToolId};
use crate::gui::testing::{FAKE_LEVEL, FakeViewport};
use crate::gui::viewport::Viewport;
use crate::gui::viewport::map::MapViewport;

fn started(root: &Path) -> (Harness<'static, NavesApp>, PathBuf) {
    let dir = project(root);
    (state_harness(app(root, &dir)), dir)
}

fn level(h: &Harness<'_, NavesApp>) -> Level {
    h.state().panel.active
}

fn text(binding: Binding) -> String {
    binding.to_string()
}

// ---- the table ----

#[test]
fn no_key_is_assigned_twice_in_a_scope() {
    for (i, a) in SHORTCUTS.iter().enumerate() {
        for b in &SHORTCUTS[i + 1..] {
            if a.binding == b.binding {
                assert_ne!(
                    a.scope,
                    b.scope,
                    "{} and {} share {}",
                    a.id,
                    b.id,
                    text(a.binding)
                );
            }
        }
    }
}

#[test]
fn no_level_key_repeats_an_everywhere_key() {
    for a in SHORTCUTS.iter().filter(|s| s.scope == Scope::Everywhere) {
        for b in SHORTCUTS.iter().filter(|s| s.scope != Scope::Everywhere) {
            assert_ne!(a.binding, b.binding, "{} and {}", a.id, b.id);
        }
    }
}

#[test]
fn every_key_of_the_spec_is_in_the_table() {
    let spec = [
        "Q",
        "W",
        "E",
        "R",
        "A",
        "N",
        "L",
        "B",
        "S",
        "T",
        "M",
        "O",
        "K",
        "J",
        "G",
        "C",
        "0",
        "1",
        "2",
        "3",
        "4",
        "5",
        "6",
        "7",
        "8",
        "9",
        "H",
        "Shift+H",
        "Ctrl+Tab",
        "F",
        "Shift+F",
        "X",
        "Shift+T",
        "Delete",
        "Ctrl+D",
        "Ctrl+Z",
        "Ctrl+Shift+Z",
        "Ctrl+F",
        ",",
        ".",
        "Ctrl+,",
        "Ctrl+.",
        "Enter",
        "Esc",
        "F5",
        "Ctrl+B",
        "F8",
        "Shift+F8",
        "F1",
        "F2",
    ];
    let table: Vec<String> = SHORTCUTS.iter().map(|s| text(s.binding)).collect();
    for key in spec {
        assert!(table.iter().any(|t| t == key), "{key} is missing");
    }
}

#[test]
fn ids_are_unique_and_pinned() {
    let ids: Vec<&str> = SHORTCUTS.iter().map(|s| s.id).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "an id is used twice");
    let pinned = [
        "level.project",
        "level.terrain",
        "level.ground",
        "level.water",
        "level.roads",
        "level.vegetation",
        "level.buildings",
        "level.objects",
        "level.life",
        "level.economy",
        "tool.select",
        "tool.move",
        "tool.rotate",
        "tool.scale",
        "tool.add",
        "tool.line",
        "tool.area",
        "tool.brush",
        "tool.terrain.smooth",
        "tool.terrain.flatten",
        "tool.terrain.ramp",
        "tool.terrain.hole",
        "tool.ground.soften",
        "tool.roads.cut",
        "tool.roads.join",
        "tool.roads.bridge",
        "tool.vegetation.cut",
        "tool.buildings.composition",
        "tool.buildings.entrance",
        "tool.buildings.hole",
        "tool.objects.composition",
        "level.hide",
        "level.show_others",
        "view.switch",
        "frame.selection",
        "frame.map",
        "axes.world",
        "follow.ground",
        "edit.delete",
        "edit.duplicate",
        "edit.undo",
        "edit.redo",
        "search.items",
        "brush.smaller",
        "brush.larger",
        "brush.weaker",
        "brush.stronger",
        "finish",
        "cancel",
        "regenerate",
        "build",
        "problem.next",
        "problem.previous",
        "help",
        "settings",
    ];
    assert_eq!(ids, pinned);
}

#[test]
fn no_default_needs_shift_or_altgr_on_qwertz() {
    use egui::Key;
    for s in SHORTCUTS {
        assert!(
            !matches!(
                s.binding.key,
                Key::OpenBracket | Key::CloseBracket | Key::Slash | Key::Questionmark
            ),
            "{}",
            s.id
        );
        if matches!(s.action, Action::Tool(_)) {
            assert!(!matches!(s.binding.key, Key::Y | Key::Z), "{}", s.id);
        }
    }
}

#[test]
fn esc_enter_and_f1_are_as_the_spec_says() {
    let find = |id: &str| SHORTCUTS.iter().find(|s| s.id == id).unwrap();
    assert_eq!(text(find("cancel").binding), "Esc");
    assert_eq!(text(find("finish").binding), "Enter");
    assert_eq!(text(find("help").binding), "F1");
    assert_eq!(find("tool.select").binding.key, egui::Key::Q);
    assert_eq!(find("tool.select").action, Action::Tool(ToolId::SELECT));
}

// ---- behaviour ----

#[test]
fn a_digit_makes_a_level_active() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    h.key_press(egui::Key::Num1);
    h.run_steps(2);
    assert_eq!(level(&h), Level::Terrain);
    h.key_press(egui::Key::Num2);
    h.run_steps(2);
    assert_eq!(level(&h), Level::Ground);
    h.key_press(egui::Key::Num0);
    h.run_steps(2);
    assert_eq!(level(&h), Level::Project);
}

#[test]
fn a_level_that_is_not_built_ignores_its_digit() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    h.key_press(egui::Key::Num4);
    h.run_steps(2);
    assert_eq!(level(&h), Level::Project);
}

#[test]
fn an_action_that_does_not_exist_does_nothing() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    h.key_press(egui::Key::W);
    h.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(level(&h), Level::Project);
}

#[test]
fn ctrl_b_builds_and_b_does_not() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    h.key_press(egui::Key::B);
    h.run_steps(2);
    assert!(h.state().worker.is_none());
    h.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::B);
    h.run_steps(2);
    assert!(h.state().worker.is_some() || !h.state().log.is_empty());
}

fn name_field<'a>(h: &'a Harness<'_, NavesApp>) -> egui_kittest::Node<'a> {
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .next()
        .unwrap()
}

#[test]
fn typing_in_a_field_fires_no_shortcut() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    name_field(&h).focus();
    h.run_steps(2);
    h.key_press(egui::Key::Num1);
    name_field(&h).type_text("1");
    h.run_steps(4);
    assert_eq!(level(&h), Level::Project);
    assert!(h.state().form.project.name.ends_with('1'));
}

#[test]
fn typing_in_a_drag_value_fires_nothing() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    h.get_by_label("Latitude").click();
    h.run_steps(3);
    h.key_press(egui::Key::Num1);
    h.run_steps(2);
    assert_eq!(level(&h), Level::Project);
}

#[test]
fn a_clicked_checkbox_does_not_block_shortcuts() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    h.get_by_label("2 Ground").click();
    h.run_steps(3);
    assert_eq!(level(&h), Level::Ground);
    h.key_press(egui::Key::Num1);
    h.run_steps(2);
    assert_eq!(level(&h), Level::Terrain);
}

#[test]
fn f1_shows_the_shortcuts_of_this_level_only() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    h.key_press(egui::Key::F1);
    h.run_steps(3);
    assert!(h.query_by_label("Go to Terrain").is_some());
    assert_eq!(
        h.query_all_by_label("Build").count(),
        3,
        "the tab, the button and the row"
    );
    assert!(
        h.query_by_label("Frame the selection").is_none(),
        "not built yet"
    );
    assert!(h.query_by_label("Go to Water").is_none(), "no such level");
    assert!(
        h.query_by_label("Hide the active level").is_some(),
        "built with the layers"
    );
    h.key_press(egui::Key::F1);
    h.run_steps(3);
    assert!(
        h.query_by_label("Go to Terrain").is_none(),
        "F1 closes the sheet"
    );
}

fn with_two_views(root: &Path) -> (Harness<'static, NavesApp>, crate::gui::testing::FakeHandle) {
    let dir = project(root);
    let (fake, handle) = FakeViewport::new();
    let views: Vec<Box<dyn Viewport>> = vec![Box::new(MapViewport::default()), Box::new(fake)];
    (state_harness(app(root, &dir).with_views(views)), handle)
}

#[test]
fn ctrl_tab_is_listed_only_with_two_views() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    h.key_press(egui::Key::F1);
    h.run_steps(3);
    assert!(
        h.query_by_label_contains("Switch between map and 3D")
            .is_none()
    );
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = with_two_views(root.path());
    h.run_steps(3);
    h.key_press(egui::Key::F1);
    h.run_steps(3);
    assert!(
        h.query_by_label_contains("Switch between map and 3D")
            .is_some()
    );
}

#[test]
fn ctrl_tab_switches_views() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = with_two_views(root.path());
    h.run_steps(3);
    assert_eq!(h.state().views.active().name(), "Map");
    h.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Tab);
    h.run_steps(2);
    assert_eq!(h.state().views.active().name(), "3D");
    h.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Tab);
    h.run_steps(2);
    assert_eq!(h.state().views.active().name(), "Map");
}

#[test]
fn ctrl_tab_does_nothing_with_one_view() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Tab);
    h.run_steps(2);
    assert_eq!(h.state().views.active().name(), "Map");
}

#[test]
fn ctrl_tab_does_not_move_focus() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = with_two_views(root.path());
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Tab);
    h.run_steps(2);
    assert!(h.ctx.memory(egui::Memory::focused).is_none());
}

#[test]
fn ctrl_tab_works_while_typing() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = with_two_views(root.path());
    h.run_steps(3);
    name_field(&h).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Tab);
    h.run_steps(2);
    assert_eq!(h.state().views.active().name(), "3D");
    assert_eq!(h.state().form.project.name, "Test");
}

#[test]
fn tab_still_moves_focus() {
    for two in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut h = if two {
            with_two_views(root.path()).0
        } else {
            started(root.path()).0
        };
        h.run_steps(3);
        h.key_press(egui::Key::Tab);
        h.run_steps(2);
        assert!(
            h.ctx.memory(egui::Memory::focused).is_some(),
            "two views: {two}"
        );
        assert_eq!(h.state().views.active().name(), "Map");
    }
}

#[test]
fn tooltips_show_the_key() {
    assert_eq!(
        crate::gui::stack::tooltip(Level::Terrain, &Bindings::default()),
        "Go to Terrain (1)"
    );
    assert_eq!(
        crate::gui::stack::tooltip(Level::Project, &Bindings::default()),
        "Go to Project (0)"
    );
}

#[test]
fn the_manual_lists_every_shortcut() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/manual/src/shortcuts.md");
    let page = std::fs::read_to_string(path).unwrap();
    for s in SHORTCUTS {
        let key = format!("`{}`", text(s.binding));
        assert!(
            page.lines()
                .any(|l| l.contains(&key) && l.contains(s.label)),
            "{} ({key}) is not in shortcuts.md",
            s.label
        );
    }
}

#[test]
fn fake_level_scope_is_respected() {
    // a key of another level does nothing: Smooth is Terrain's S
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut h = state_harness(app(root.path(), &dir).with_extra_level(&FAKE_LEVEL));
    h.run_steps(3);
    h.key_press(egui::Key::Num6);
    h.run_steps(2);
    assert_eq!(level(&h), Level::Buildings);
}

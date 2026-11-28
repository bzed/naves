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

/// The table of the editor spec, section 4 (revised 2026-11-28): id, key, scope.
const SPEC: &[(&str, &str, &str)] = &[
    ("level.project", "Alt+4", "all"),
    ("level.project.alt", "0", "all"),
    ("level.terrain", "1", "all"),
    ("level.ground", "2", "all"),
    ("level.water", "3", "all"),
    ("level.roads", "4", "all"),
    ("level.vegetation", "5", "all"),
    ("level.buildings", "6", "all"),
    ("level.objects", "Alt+1", "all"),
    ("level.objects.alt", "7", "all"),
    ("level.life", "Alt+2", "all"),
    ("level.life.alt", "8", "all"),
    ("level.economy", "Alt+3", "all"),
    ("level.economy.alt", "9", "all"),
    ("view.pan.north", "W", "all"),
    ("view.pan.west", "A", "all"),
    ("view.pan.south", "S", "all"),
    ("view.pan.east", "D", "all"),
    ("view.zoom.out", "Q", "all"),
    ("view.zoom.in", "E", "all"),
    ("tool.move", "G", "all"),
    ("tool.line", "C", "all"),
    ("tool.area", "V", "all"),
    ("tool.brush", "B", "all"),
    ("tool.rotate", "R", "Buildings"),
    ("tool.scale", "Shift+R", "Buildings"),
    ("tool.add", "T", "Buildings"),
    ("tool.terrain.smooth", "R", "Terrain"),
    ("tool.terrain.flatten", "T", "Terrain"),
    ("tool.terrain.ramp", "Shift+R", "Terrain"),
    ("tool.terrain.hole", "Shift+C", "Terrain"),
    ("tool.ground.soften", "R", "Ground"),
    ("tool.roads.cut", "R", "Roads"),
    ("tool.roads.join", "T", "Roads"),
    ("tool.roads.bridge", "Shift+T", "Roads"),
    ("tool.vegetation.cut", "R", "Vegetation"),
    ("tool.vegetation.add", "T", "Vegetation"),
    ("tool.buildings.composition", "Shift+C", "Buildings"),
    ("tool.buildings.entrance", "Shift+T", "Buildings"),
    ("tool.buildings.hole", "Shift+B", "Buildings"),
    ("tool.objects.rotate", "R", "Objects"),
    ("tool.objects.scale", "Shift+R", "Objects"),
    ("tool.objects.add", "T", "Objects"),
    ("tool.objects.composition", "Shift+C", "Objects"),
    ("tool.life.add", "T", "Life"),
    ("tool.economy.add", "T", "Economy"),
    ("level.hide", "H", "all"),
    ("level.show_others", "Shift+H", "all"),
    ("view.switch", "Ctrl+Tab", "all"),
    ("frame.selection", "F", "all"),
    ("frame.map", "Shift+F", "all"),
    ("earthwork.shape", "Shift+V", "all"),
    ("axes.world", "Shift+X", "all"),
    ("follow.ground", "Shift+G", "all"),
    ("edit.delete", "X", "all"),
    ("edit.delete.alt", "Delete", "all"),
    ("edit.duplicate", "Ctrl+G", "all"),
    ("edit.undo", "Ctrl+Z", "all"),
    ("edit.redo", "Ctrl+Shift+Z", "all"),
    ("search.items", "Ctrl+F", "all"),
    ("command_palette", "Ctrl+P", "all"),
    ("brush.smaller", ",", "all"),
    ("brush.larger", ".", "all"),
    ("brush.weaker", "Ctrl+,", "all"),
    ("brush.stronger", "Ctrl+.", "all"),
    ("finish", "Space", "all"),
    ("finish.alt", "Enter", "all"),
    ("cancel", "Esc", "all"),
    ("regenerate", "Ctrl+R", "all"),
    ("regenerate.alt", "F5", "all"),
    ("build", "Ctrl+B", "all"),
    ("problem.next", "F8", "all"),
    ("problem.previous", "Shift+F8", "all"),
    ("help", "F1", "all"),
    ("settings", "F2", "all"),
];

fn scope_name(scope: Scope) -> String {
    match scope {
        Scope::Everywhere => "all".to_string(),
        Scope::Level(level) => format!("{level:?}"),
    }
}

#[test]
fn the_table_is_the_spec() {
    let table: Vec<(&str, String, String)> = SHORTCUTS
        .iter()
        .map(|s| (s.id, text(s.binding), scope_name(s.scope)))
        .collect();
    let spec: Vec<(&str, String, String)> = SPEC
        .iter()
        .map(|(id, key, scope)| (*id, (*key).to_string(), (*scope).to_string()))
        .collect();
    assert_eq!(table, spec);
}

const LEFT: &[egui::Key] = &[
    egui::Key::Num1,
    egui::Key::Num2,
    egui::Key::Num3,
    egui::Key::Num4,
    egui::Key::Num5,
    egui::Key::Num6,
    egui::Key::Q,
    egui::Key::W,
    egui::Key::E,
    egui::Key::R,
    egui::Key::T,
    egui::Key::A,
    egui::Key::S,
    egui::Key::D,
    egui::Key::F,
    egui::Key::G,
    egui::Key::X,
    egui::Key::C,
    egui::Key::V,
    egui::Key::B,
    egui::Key::Escape,
    egui::Key::Tab,
    egui::Key::Space,
];

/// The sets that move to the right half together, and Undo and Redo (Ctrl+Z is the key everybody knows).
const RIGHT_HALF: &[Action] = &[
    Action::HideLevel,
    Action::ShowOthersInFull,
    Action::BrushSmaller,
    Action::BrushLarger,
    Action::BrushWeaker,
    Action::BrushStronger,
    Action::NextProblem,
    Action::PreviousProblem,
    Action::Help,
    Action::Settings,
    Action::CommandPalette,
    Action::Undo,
    Action::Redo,
];

#[test]
fn every_default_used_with_the_mouse_is_on_the_left_half() {
    for s in SHORTCUTS.iter().filter(|s| {
        s.id.rsplit('.').next().is_none_or(|last| last != "alt") && !RIGHT_HALF.contains(&s.action)
    }) {
        assert!(
            LEFT.contains(&s.binding.key),
            "{} uses {}, which is not on the left half",
            s.id,
            text(s.binding)
        );
    }
}

#[test]
fn a_right_half_set_moves_whole() {
    let sets: [&[Action]; 3] = [
        &[Action::HideLevel, Action::ShowOthersInFull],
        &[
            Action::BrushSmaller,
            Action::BrushLarger,
            Action::BrushWeaker,
            Action::BrushStronger,
        ],
        &[Action::NextProblem, Action::PreviousProblem],
    ];
    for set in sets {
        let left: Vec<bool> = set
            .iter()
            .map(|action| {
                let s = SHORTCUTS.iter().find(|s| s.action == *action).unwrap();
                LEFT.contains(&s.binding.key)
            })
            .collect();
        assert!(
            left.iter().all(|l| *l) || left.iter().all(|l| !*l),
            "{set:?} is split between the halves"
        );
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
        "level.project.alt",
        "level.terrain",
        "level.ground",
        "level.water",
        "level.roads",
        "level.vegetation",
        "level.buildings",
        "level.objects",
        "level.objects.alt",
        "level.life",
        "level.life.alt",
        "level.economy",
        "level.economy.alt",
        "view.pan.north",
        "view.pan.west",
        "view.pan.south",
        "view.pan.east",
        "view.zoom.out",
        "view.zoom.in",
        "tool.move",
        "tool.line",
        "tool.area",
        "tool.brush",
        "tool.rotate",
        "tool.scale",
        "tool.add",
        "tool.terrain.smooth",
        "tool.terrain.flatten",
        "tool.terrain.ramp",
        "tool.terrain.hole",
        "tool.ground.soften",
        "tool.roads.cut",
        "tool.roads.join",
        "tool.roads.bridge",
        "tool.vegetation.cut",
        "tool.vegetation.add",
        "tool.buildings.composition",
        "tool.buildings.entrance",
        "tool.buildings.hole",
        "tool.objects.rotate",
        "tool.objects.scale",
        "tool.objects.add",
        "tool.objects.composition",
        "tool.life.add",
        "tool.economy.add",
        "level.hide",
        "level.show_others",
        "view.switch",
        "frame.selection",
        "frame.map",
        "earthwork.shape",
        "axes.world",
        "follow.ground",
        "edit.delete",
        "edit.delete.alt",
        "edit.duplicate",
        "edit.undo",
        "edit.redo",
        "search.items",
        "command_palette",
        "brush.smaller",
        "brush.larger",
        "brush.weaker",
        "brush.stronger",
        "finish",
        "finish.alt",
        "cancel",
        "regenerate",
        "regenerate.alt",
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
    assert_eq!(text(find("finish").binding), "Space");
    assert_eq!(text(find("finish.alt").binding), "Enter");
    assert_eq!(text(find("help").binding), "F1");
    assert!(
        SHORTCUTS
            .iter()
            .all(|s| s.action != Action::Tool(ToolId::SELECT)),
        "Select has no key of its own: Esc is Select"
    );
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
    // 3 is Water, which is not built
    h.key_press(egui::Key::Num3);
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
        "Go to Project (Alt+4)"
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

#[test]
fn a_held_key_fires_once() {
    // H hides the active level: the repeats of a held H must not toggle it back and forth
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    h.key_press(egui::Key::Num1);
    h.run_steps(2);
    let key_event = |repeat| egui::Event::Key {
        key: egui::Key::H,
        physical_key: Some(egui::Key::H),
        pressed: true,
        repeat,
        modifiers: egui::Modifiers::NONE,
    };
    h.event(key_event(false));
    h.run_steps(1);
    for _ in 0..3 {
        h.event(key_event(true));
        h.run_steps(1);
    }
    assert!(
        h.state().panel.hidden.contains(&Level::Terrain),
        "one press hid it; three repeats did not show it again and hide it again"
    );
}

#[test]
fn esc_is_select() {
    // the fake level (6) has Place; Draw is not built yet
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut h = state_harness(app(root.path(), &dir).with_extra_level(&FAKE_LEVEL));
    h.run_steps(3);
    h.key_press(egui::Key::Num6);
    h.run_steps(3);
    assert!(
        h.query_by_label("Esc Select").is_some(),
        "the rail shows Esc over Select"
    );
    h.key_press(egui::Key::T);
    h.run_steps(3);
    assert_eq!(h.state().tool, ToolId::ADD);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().tool, ToolId::SELECT);
}

/// A level 7 that only tests build.
static FAKE_OBJECTS: crate::gui::levels::LevelDef = crate::gui::levels::LevelDef {
    level: Level::Objects,
    panel: |_, ui| {
        ui.label("The panel of the fake objects level.");
    },
    layers: &[],
    status: "Fake objects level.",
    pick: None,
    tools: &[],
    items: |_| Vec::new(),
};

#[test]
fn alt_digits_reach_the_upper_levels() {
    let root = tempfile::tempdir().unwrap();
    let (mut h, _) = started(root.path());
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::Num1);
    h.run_steps(2);
    assert_eq!(level(&h), Level::Project, "level 7 is not built");
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut h = state_harness(app(root.path(), &dir).with_extra_level(&FAKE_OBJECTS));
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::Num1);
    h.run_steps(2);
    assert_eq!(level(&h), Level::Objects);
    h.key_press(egui::Key::Num1);
    h.run_steps(2);
    assert_eq!(
        level(&h),
        Level::Terrain,
        "the plain digit is still level 1"
    );
    h.key_press(egui::Key::Num7);
    h.run_steps(2);
    assert_eq!(level(&h), Level::Objects, "7 is the second key of level 7");
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::Num4);
    h.run_steps(2);
    assert_eq!(level(&h), Level::Project, "Alt+4 is the Project (0)");
}

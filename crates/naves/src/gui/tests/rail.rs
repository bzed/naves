// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The tool rail and the items panel. Production levels have neither yet; a fake level has both.

use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;
use crate::gui::levels::Level;
use crate::gui::palette::{PaletteItem, filter};
use crate::gui::shortcuts::ToolId;
use crate::gui::testing::{FAKE_LEVEL, FakeViewport, take_tool_events};
use crate::gui::viewport::map::MapViewport;
use crate::gui::viewport::{ViewEvent, Viewport, WorldPoint};

/// The fake level (6) is active.
fn started(root: &Path) -> Harness<'static, NavesApp> {
    let dir = project(root);
    let mut h = state_harness(app(root, &dir).with_extra_level(&FAKE_LEVEL));
    h.run_steps(3);
    h.key_press(egui::Key::Num6);
    h.run_steps(3);
    h
}

fn tool(h: &Harness<'_, NavesApp>) -> ToolId {
    h.state().tool
}

#[test]
fn production_levels_show_no_rail_and_no_palette() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut h = state_harness(app(root.path(), &dir));
    for key in [egui::Key::Num0, egui::Key::Num1, egui::Key::Num2] {
        h.key_press(key);
        h.run_steps(3);
        assert!(h.query_by_label_contains(" Select").is_none(), "{key:?}");
        assert!(h.query_by_label("Search items").is_none(), "{key:?}");
    }
}

#[test]
fn the_rail_shows_the_level_s_tools() {
    let root = tempfile::tempdir().unwrap();
    let h = started(root.path());
    for label in ["Q Select", "A Place", "C Compose"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
}

#[test]
fn only_letters_that_apply_are_drawn() {
    let root = tempfile::tempdir().unwrap();
    let h = started(root.path());
    for label in ["W Move", "E Rotate", "R Scale"] {
        assert!(h.query_by_label(label).is_none(), "{label}");
    }
}

#[test]
fn a_letter_picks_the_tool() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path());
    assert_eq!(tool(&h), ToolId::SELECT);
    h.key_press(egui::Key::A);
    h.run_steps(3);
    assert_eq!(tool(&h), ToolId::ADD);
    h.key_press(egui::Key::C);
    h.run_steps(3);
    assert_eq!(tool(&h), ToolId::COMPOSITION);
    h.key_press(egui::Key::W);
    h.run_steps(3);
    assert_eq!(tool(&h), ToolId::COMPOSITION, "this level has no Move");
}

#[test]
fn a_click_on_the_rail_picks_it_too() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path());
    h.get_by_label("A Place").click();
    h.run_steps(3);
    assert_eq!(tool(&h), ToolId::ADD);
}

#[test]
fn esc_returns_to_select() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path());
    h.key_press(egui::Key::A);
    h.run_steps(3);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(tool(&h), ToolId::SELECT);
}

#[test]
fn changing_the_level_returns_to_select() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path());
    h.key_press(egui::Key::A);
    h.run_steps(3);
    h.key_press(egui::Key::Num1);
    h.run_steps(3);
    assert_eq!(tool(&h), ToolId::SELECT);
}

#[test]
fn the_status_line_follows_the_tool() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path());
    assert!(h.query_by_label_contains("Fake level:").is_some());
    h.key_press(egui::Key::A);
    h.run_steps(3);
    assert!(h.query_by_label_contains("Place: click").is_some());
    assert!(h.query_by_label_contains("Fake level:").is_none());
}

fn click_at(h: &mut Harness<'_, NavesApp>, pos: egui::Pos2) {
    h.hover_at(pos);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(1);
    }
    h.run_steps(2);
}

#[test]
fn tool_events_arrive_in_metres_from_the_map() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut h = state_harness(app(root.path(), &dir).with_extra_level(&FAKE_LEVEL));
    h.run_steps(2);
    while !matches!(h.state().heightmap, HeightmapState::Ready(_)) {
        h.step();
        std::thread::yield_now();
    }
    h.key_press(egui::Key::Num6);
    h.run_steps(3);
    h.key_press(egui::Key::A);
    h.run_steps(3);
    take_tool_events();
    let pos = h.state().seen_rect.center();
    click_at(&mut h, pos);
    let events = take_tool_events();
    assert!(
        events.iter().any(|e| matches!(e, ViewEvent::Click { .. })),
        "{events:?}"
    );
}

#[test]
fn tool_events_arrive_in_metres_from_either_view() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let (fake, handle) = FakeViewport::new();
    let views: Vec<Box<dyn Viewport>> = vec![Box::new(MapViewport::default()), Box::new(fake)];
    let mut h = state_harness(
        app(root.path(), &dir)
            .with_extra_level(&FAKE_LEVEL)
            .with_views(views),
    );
    while !matches!(h.state().heightmap, HeightmapState::Ready(_)) {
        h.step();
        std::thread::yield_now();
    }
    h.key_press(egui::Key::Num6);
    h.run_steps(3);
    h.key_press(egui::Key::A);
    h.run_steps(3);
    h.state_mut().views.switch();
    take_tool_events();
    let at = WorldPoint { x: 12.0, y: 34.0 };
    handle.borrow_mut().queued.push(ViewEvent::Click {
        at,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(3);
    assert_eq!(
        take_tool_events(),
        [ViewEvent::Click {
            at,
            modifiers: Modifiers::NONE
        }]
    );
    assert!(
        h.state().selection.is_none(),
        "an active tool does not select"
    );
}

#[test]
fn the_palette_replaces_the_panel() {
    let root = tempfile::tempdir().unwrap();
    let h = started(root.path());
    assert!(h.query_by_label("Small house").is_some());
    assert!(h.query_by_label("The panel of the fake level.").is_none());
}

#[test]
fn picking_an_item_switches_to_its_tool() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path());
    h.get_by_label("Bunker").click();
    h.run_steps(3);
    assert_eq!(tool(&h), ToolId::COMPOSITION);
    assert_eq!(h.state().item.as_deref(), Some("bunker"));
}

#[test]
fn another_tool_can_be_picked_after() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path());
    h.get_by_label("Small house").click();
    h.run_steps(3);
    assert_eq!(tool(&h), ToolId::ADD);
    h.key_press(egui::Key::C);
    h.run_steps(3);
    assert_eq!(tool(&h), ToolId::COMPOSITION);
    assert_eq!(h.state().item.as_deref(), Some("house.small"));
}

#[test]
fn ctrl_f_focuses_the_search_and_letters_then_type() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path());
    h.key_press_modifiers(Modifiers::CTRL, egui::Key::F);
    h.run_steps(3);
    h.key_press(egui::Key::W);
    h.run_steps(2);
    h.get_by_role(Role::TextInput).type_text("big");
    h.run_steps(3);
    assert_eq!(tool(&h), ToolId::SELECT, "typing is typing");
    assert!(h.query_by_label("Big house").is_some());
    assert!(h.query_by_label("Small house").is_none());
}

fn item(id: &str, name: &str, group: &str) -> PaletteItem {
    PaletteItem {
        id: id.to_string(),
        name: name.to_string(),
        group: group.to_string(),
        thumbnail: crate::gui::palette::Thumbnail::Swatch([1, 2, 3]),
        kind: crate::gui::palette::ItemKind::Preset,
        default_tool: ToolId::ADD,
        source: crate::gui::palette::ItemSource::Game,
    }
}

#[test]
fn search_filters_by_name_and_group() {
    let items = vec![
        item("a", "Small house", "Residential"),
        item("b", "Bunker", "Military"),
    ];
    let names =
        |s: &str| -> Vec<&str> { filter(&items, s).iter().map(|i| i.id.as_str()).collect() };
    assert_eq!(names(""), ["a", "b"]);
    assert_eq!(names("HOUSE"), ["a"]);
    assert_eq!(names("milit"), ["b"]);
    assert_eq!(names("zzz"), [] as [&str; 0]);
}

#[test]
fn last_used_comes_first() {
    let root = tempfile::tempdir().unwrap();
    let mut h = started(root.path());
    h.get_by_label("Bunker").click();
    h.run_steps(3);
    let y = |label: &str, h: &Harness<'_, NavesApp>| {
        h.get_all_by_label(label).next().unwrap().rect().min.y
    };
    // the bunker is listed under Last used above its group, and so before the houses
    assert!(y("Bunker", &h) < y("Small house", &h));
    assert!(h.query_by_label("Last used").is_some());
}

#[test]
fn groups_are_in_stable_order() {
    let root = tempfile::tempdir().unwrap();
    let h = started(root.path());
    let y = |label: &str| h.get_by_label(label).rect().min.y;
    assert!(
        y("Military") < y("Residential"),
        "alphabetical, not in order of arrival"
    );
}

#[test]
fn each_item_says_what_it_is() {
    let root = tempfile::tempdir().unwrap();
    let h = started(root.path());
    assert_eq!(h.query_all_by_label("Preset").count(), 2);
    assert_eq!(h.query_all_by_label("Single model").count(), 1);
}

#[test]
fn the_level_is_the_fake_one() {
    let root = tempfile::tempdir().unwrap();
    let h = started(root.path());
    assert_eq!(h.state().panel.active, Level::Buildings);
}

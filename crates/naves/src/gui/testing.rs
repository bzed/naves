// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Test helpers: a second view that proves the shell does not depend on the map.

use std::cell::RefCell;
use std::rc::Rc;

use eframe::egui;

use super::viewport::{ViewEvent, ViewFocus, ViewFrame, Viewport, WorldPoint};

/// What the fake view saw and what it answers.
#[derive(Default)]
pub struct FakeState {
    /// The place it was told to show.
    pub focus: Option<ViewFocus>,
    /// How many times `show` was called.
    pub shown: u32,
    /// The generation of every frame it got.
    pub generations: Vec<u64>,
    /// How many times it was reset.
    pub resets: u32,
    /// The layers of the last frame.
    pub layers: Vec<super::viewport::VisibleLayer>,
    /// Events it reports in its next `show`.
    pub queued: Vec<ViewEvent>,
    /// What `project` answers.
    pub projected: Option<egui::Pos2>,
}

/// Shared access to a [`FakeViewport`] that the app owns.
pub type FakeHandle = Rc<RefCell<FakeState>>;

/// A view named "3D" that draws nothing.
pub struct FakeViewport(FakeHandle);

impl FakeViewport {
    /// The view and a handle to look at it.
    pub fn new() -> (Self, FakeHandle) {
        let state = FakeHandle::default();
        (Self(Rc::clone(&state)), state)
    }
}

impl Viewport for FakeViewport {
    fn name(&self) -> &'static str {
        "3D"
    }

    fn show(
        &mut self,
        _ui: &mut egui::Ui,
        _rect: egui::Rect,
        frame: &ViewFrame<'_>,
    ) -> Vec<ViewEvent> {
        let mut state = self.0.borrow_mut();
        state.shown += 1;
        state.generations.push(frame.generation);
        state.layers = frame.layers.to_vec();
        std::mem::take(&mut state.queued)
    }

    fn focus(&self) -> Option<ViewFocus> {
        self.0.borrow().focus
    }

    fn set_focus(&mut self, focus: ViewFocus) {
        self.0.borrow_mut().focus = Some(focus);
    }

    fn project(&self, _at: WorldPoint, _rect: egui::Rect) -> Option<egui::Pos2> {
        self.0.borrow().projected
    }

    fn reset(&mut self) {
        let mut state = self.0.borrow_mut();
        state.resets += 1;
        state.focus = None;
    }

    fn invalidate(&mut self) {}

    fn holds_picture(&self) -> bool {
        false
    }

    fn renders(&self) -> u64 {
        0
    }
}

use super::NavesApp;
use super::levels::{Level, LevelDef, ToolDef};
use super::palette::{ItemKind, ItemSource, PaletteItem, Thumbnail};
use super::shortcuts::ToolId;

/// A level 6 that only tests build.
pub static FAKE_LEVEL: LevelDef = LevelDef {
    level: Level::Buildings,
    panel: fake_panel,
    layers: &[],
    status: "Fake level: the status line of a level that tests add.",
    pick: Some(fake_pick),
    tools: &[
        ToolDef {
            tool: ToolId::ADD,
            word: "Place",
            status: "Place: click to place the item.",
            on_event: Some(fake_tool_event),
        },
        ToolDef {
            tool: ToolId::COMPOSITION,
            word: "Compose",
            status: "Compose: click the pieces to join.",
            on_event: Some(fake_tool_event),
        },
    ],
    items: fake_items,
};

thread_local! {
    static TOOL_EVENTS: RefCell<Vec<ViewEvent>> = const { RefCell::new(Vec::new()) };
}

fn fake_tool_event(_app: &mut NavesApp, event: &ViewEvent) {
    TOOL_EVENTS.with(|e| e.borrow_mut().push(*event));
}

/// What the fake tools were told since the last call (on this thread, so tests do not meet).
pub fn take_tool_events() -> Vec<ViewEvent> {
    TOOL_EVENTS.with(|e| std::mem::take(&mut *e.borrow_mut()))
}

fn fake_items(_app: &NavesApp) -> Vec<PaletteItem> {
    let item = |id: &str, name: &str, group: &str, kind, tool| PaletteItem {
        id: id.to_string(),
        name: name.to_string(),
        group: group.to_string(),
        thumbnail: Thumbnail::Swatch([154, 79, 60]),
        kind,
        default_tool: tool,
        source: ItemSource::Game,
    };
    vec![
        item(
            "house.small",
            "Small house",
            "Residential",
            ItemKind::Preset,
            ToolId::ADD,
        ),
        item(
            "house.big",
            "Big house",
            "Residential",
            ItemKind::Model,
            ToolId::ADD,
        ),
        item(
            "bunker",
            "Bunker",
            "Military",
            ItemKind::Preset,
            ToolId::COMPOSITION,
        ),
    ]
}

#[expect(clippy::unnecessary_wraps, reason = "the signature of LevelDef::pick")]
fn fake_pick(
    _app: &NavesApp,
    at: super::viewport::WorldPoint,
) -> Option<super::selection::Selection> {
    Some(super::selection::Selection {
        level: Level::Buildings,
        title: "Fake thing".to_string(),
        rows: Vec::new(),
        origin: None,
        at,
        span_m: Some(100.0),
    })
}

fn fake_panel(_app: &mut NavesApp, ui: &mut egui::Ui) {
    ui.label("The panel of the fake level.");
}

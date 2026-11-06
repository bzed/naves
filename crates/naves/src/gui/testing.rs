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
use super::levels::{Level, LevelDef};

/// A level 6 that only tests build.
pub static FAKE_LEVEL: LevelDef = LevelDef {
    level: Level::Buildings,
    panel: fake_panel,
    layers: &[],
    status: "Fake level: the status line of a level that tests add.",
};

fn fake_panel(_app: &mut NavesApp, ui: &mut egui::Ui) {
    ui.label("The panel of the fake level.");
}

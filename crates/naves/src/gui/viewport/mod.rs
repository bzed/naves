// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Views of the map. Everything outside a view speaks map metres, never pixels, so that the shell, the tools and
//! the markers work the same on the top-down map and on a view that looks at the ground from the side.

pub mod map;

use std::sync::Arc;

use eframe::egui::{self, Modifiers};
use naves_core::project::Project;
use naves_core::render::Overlay;

use super::worker::MapData;

/// A point on the map: metres east and north of the south-west corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldPoint {
    /// Metres east.
    pub x: f64,
    /// Metres north.
    pub y: f64,
}

/// A place on the map that both views can show: the centre and how many metres the view spans.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewFocus {
    /// The middle of the view.
    pub center: WorldPoint,
    /// The metres covered by the shorter side of a top-down view.
    pub span_m: f64,
}

/// What happened in a view, in map metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewEvent {
    /// The pointer moved over the map.
    Hover(WorldPoint),
    /// The pointer left the view.
    Left,
    /// A button went down.
    Press {
        /// Where.
        at: WorldPoint,
        /// The modifier keys.
        modifiers: Modifiers,
    },
    /// The pointer moved with the button down.
    Drag {
        /// Where.
        at: WorldPoint,
        /// The modifier keys.
        modifiers: Modifiers,
    },
    /// The button went up after a drag.
    Release {
        /// Where.
        at: WorldPoint,
        /// The modifier keys.
        modifiers: Modifiers,
    },
    /// A click.
    Click {
        /// Where.
        at: WorldPoint,
        /// The modifier keys.
        modifiers: Modifiers,
    },
}

/// What the shell hands to a view each frame.
pub struct ViewFrame<'a> {
    /// The data of the map.
    pub data: &'a Arc<MapData>,
    /// The project.
    pub project: &'a Project,
    /// The base map.
    pub base: Overlay,
    /// Counts up with every new `MapData` (and every change of what the pictures show).
    pub generation: u64,
}

/// A way of looking at the map.
pub trait Viewport {
    /// The name, for the status line and the shortcut sheet.
    fn name(&self) -> &'static str;
    /// Draws the view into `rect` and reports what happened there.
    fn show(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        frame: &ViewFrame<'_>,
    ) -> Vec<ViewEvent>;
    /// The place on the map this view shows, once it knows.
    fn focus(&self) -> Option<ViewFocus>;
    /// Moves the view to a place; before the first `show` it is remembered.
    fn set_focus(&mut self, focus: ViewFocus);
    /// Where a map point is on the screen inside `rect`, or `None` when it is not visible (also behind a camera).
    fn project(&self, at: WorldPoint, rect: egui::Rect) -> Option<egui::Pos2>;
    /// Forgets the camera and every picture: the map under it has changed.
    fn reset(&mut self);
    /// Forgets the pictures only: they look different now, but the data is the same.
    fn invalidate(&mut self);
    /// Whether a picture of the map is held (for tests).
    #[cfg(test)]
    fn holds_picture(&self) -> bool;
}

/// The views of the editor and which one is shown.
pub struct ViewportSet {
    views: Vec<Box<dyn Viewport>>,
    active: usize,
}

impl ViewportSet {
    /// A set of views; the first is shown.
    #[must_use]
    pub fn new(views: Vec<Box<dyn Viewport>>) -> Self {
        Self { views, active: 0 }
    }

    /// The view that is shown.
    #[must_use]
    pub fn active(&self) -> &dyn Viewport {
        self.views[self.active].as_ref()
    }

    /// The view that is shown.
    pub fn active_mut(&mut self) -> &mut dyn Viewport {
        self.views[self.active].as_mut()
    }

    /// Whether there is another view to switch to.
    #[must_use]
    pub fn can_switch(&self) -> bool {
        self.views.len() > 1
    }

    /// Shows the next view at the place the current one shows.
    pub fn switch(&mut self) {
        if !self.can_switch() {
            return;
        }
        let focus = self.active().focus();
        self.active = (self.active + 1) % self.views.len();
        if let Some(focus) = focus {
            self.active_mut().set_focus(focus);
        }
    }

    /// Resets every view.
    pub fn reset_all(&mut self) {
        for view in &mut self.views {
            view.reset();
        }
    }

    /// Drops the pictures of every view.
    pub fn invalidate_all(&mut self) {
        for view in &mut self.views {
            view.invalidate();
        }
    }
}

/// The views of the editor today: the map, alone.
#[must_use]
pub fn default_views() -> Vec<Box<dyn Viewport>> {
    vec![Box::new(map::MapViewport::default())]
}

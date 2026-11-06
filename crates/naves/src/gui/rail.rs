// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The tool rail between the left panel and the map. Each tool is drawn as its key, a letter with its word below,
//! so that the rail teaches the keyboard and needs no icon set.

use eframe::egui::{self, Align2, FontId, Sense, WidgetInfo};

use super::NavesApp;
use super::shortcuts::{self, Action, ToolId};
use super::theme::{GREEN, INK};

/// Width of the rail.
pub const RAIL_WIDTH: f32 = 56.0;
const BUTTON_HEIGHT: f32 = 50.0;

/// The tools of the rail, one per row.
pub fn rail_ui(app: &mut NavesApp, ui: &mut egui::Ui) {
    let level = app.panel.active;
    let tools = app.rail_tools();
    for (tool, word) in tools {
        let key = shortcuts::key_text(Action::Tool(tool), level).unwrap_or_default();
        let active = app.tool == tool;
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), BUTTON_HEIGHT),
            Sense::click(),
        );
        let label = format!("{key} {word}");
        response
            .widget_info(|| WidgetInfo::selected(egui::Role::Button, true, active, label.clone()));
        let painter = ui.painter();
        let (letter_colour, word_colour) = if active {
            painter.rect_filled(rect, 0.0, level.colour());
            (level.band_text(), level.band_text())
        } else {
            (INK, INK)
        };
        painter.text(
            rect.center_top() + egui::vec2(0.0, 6.0),
            Align2::CENTER_TOP,
            &key,
            FontId::proportional(22.0),
            letter_colour,
        );
        painter.text(
            rect.center_bottom() - egui::vec2(0.0, 5.0),
            Align2::CENTER_BOTTOM,
            word,
            FontId::proportional(11.0),
            word_colour,
        );
        if response.has_focus() {
            painter.rect_stroke(
                rect.shrink(1.0),
                0.0,
                egui::Stroke::new(2.0, GREEN),
                egui::StrokeKind::Inside,
            );
        }
        if response.clicked() {
            app.tool = tool;
        }
    }
}

impl NavesApp {
    /// The tools to draw: Select when the level can pick, then the level's own. Letters that do not apply are not drawn.
    pub(super) fn rail_tools(&self) -> Vec<(ToolId, &'static str)> {
        let Some(def) = self.levels.iter().find(|d| d.level == self.panel.active) else {
            return Vec::new();
        };
        let mut tools = Vec::new();
        if def.pick.is_some() {
            tools.push((ToolId::SELECT, "Select"));
        }
        tools.extend(def.tools.iter().map(|t| (t.tool, t.word)));
        tools
    }

    /// Whether the rail is drawn: the level has a tool other than Select.
    pub(super) fn has_rail(&self) -> bool {
        self.levels
            .iter()
            .find(|d| d.level == self.panel.active)
            .is_some_and(|d| !d.tools.is_empty())
    }
}

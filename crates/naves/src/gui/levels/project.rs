// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Level 0: the project's own settings.

use eframe::egui;

use super::{Level, LevelDef};
use crate::gui::NavesApp;

/// The level.
pub const DEF: LevelDef = LevelDef {
    level: Level::Project,
    panel,
    status: "Project settings are on the left. Press 1 to read the terrain.",
};

fn panel(app: &mut NavesApp, ui: &mut egui::Ui) {
    app.settings_ui(ui);
}

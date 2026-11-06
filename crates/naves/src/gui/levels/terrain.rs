// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Level 1: the shape of the ground.

use eframe::egui::{self, RichText};

use super::{Level, LevelDef, number_row, waiting};
use crate::gui::{NavesApp, dem_files_ui};

/// The level.
pub const DEF: LevelDef = LevelDef {
    level: Level::Terrain,
    panel,
    status: "Terrain: the shape of the ground. Press 2 to read what covers it.",
};

fn panel(app: &mut NavesApp, ui: &mut egui::Ui) {
    ui.heading("Terrain");
    ui.add_space(6.0);
    let data = waiting(app, ui);
    let p = &app.form.project;
    egui::Grid::new("terrain-numbers")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            number_row(ui, "Map size", &format!("{:.0} m", p.map_size_m()));
            number_row(
                ui,
                "Vertex spacing",
                &format!("{:.2} m", p.terrain_cell_m()),
            );
            if let Some(data) = &data {
                let (low, high) = data.heightmap.min_max();
                number_row(ui, "Lowest point", &format!("{low:.0} m"));
                number_row(ui, "Highest point", &format!("{high:.0} m"));
            }
        });
    ui.add_space(8.0);
    ui.label(RichText::new("Elevation files").strong());
    dem_files_ui(ui, p);
}

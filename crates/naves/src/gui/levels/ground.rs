// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Level 2: what covers the ground.

use eframe::egui::{self, Color32};
use naves_core::surface::SurfaceClass;

use super::{Level, LevelDef, number_row, waiting};
use crate::gui::stack::ground_shares;
use crate::gui::{NavesApp, ground_files_ui};

/// The level.
pub const DEF: LevelDef = LevelDef {
    level: Level::Ground,
    panel,
    status: "Ground: what covers the terrain. Press 0 for the project settings.",
    layers: &[],
};

fn panel(app: &mut NavesApp, ui: &mut egui::Ui) {
    ui.heading("Ground");
    ui.add_space(6.0);
    if let Some(data) = waiting(app, ui) {
        let shares = ground_shares(&data.surfaces, app.form.project.map_size_m());
        egui::Grid::new("ground-shares")
            .num_columns(3)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                for (class, share) in SurfaceClass::ALL.iter().zip(shares) {
                    let [r, g, b] = class.rgb();
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(rect, 2.0, Color32::from_rgb(r, g, b));
                    number_row(ui, class.name(), &format!("{:.1} %", share * 100.0));
                }
            });
    }
    ui.add_space(8.0);
    ground_files_ui(ui, &app.form.project);
}

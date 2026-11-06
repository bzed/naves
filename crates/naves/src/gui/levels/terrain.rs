// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Level 1: the shape of the ground.

use eframe::egui::{self, RichText};
use naves_core::render::{MapLayer, WALK_LIMIT_DEG};

use super::{Level, LevelDef, number_row, waiting};
use crate::gui::layers::LayerDef;
use crate::gui::theme::INK;
use crate::gui::{NavesApp, dem_files_ui};

/// The level.
pub const DEF: LevelDef = LevelDef {
    level: Level::Terrain,
    panel,
    status: "Terrain: the shape of the ground. Press 2 to read what covers it.",
    layers: &[LayerDef {
        layer: MapLayer::TooSteepToWalk,
        name: "Too steep to walk",
        tooltip: steep_tooltip,
        key: steep_key,
    }],
};

fn steep_tooltip() -> String {
    format!("Ground steeper than {WALK_LIMIT_DEG:.0}°. Players cannot walk up it.")
}

/// The hatching as drawn on the map: diagonal lines in ink.
fn steep_key(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 12.0), egui::Sense::hover());
        let painter = ui.painter().with_clip_rect(rect);
        let stroke = egui::Stroke::new(1.5, INK);
        let mut x = rect.left() - rect.height();
        while x < rect.right() {
            painter.line_segment(
                [
                    egui::pos2(x, rect.bottom()),
                    egui::pos2(x + rect.height(), rect.top()),
                ],
                stroke,
            );
            x += 5.0;
        }
        ui.label(format!("Steeper than {WALK_LIMIT_DEG:.0}°"));
    });
}

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

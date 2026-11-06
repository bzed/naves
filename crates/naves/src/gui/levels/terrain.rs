// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Level 1: the shape of the ground.

use eframe::egui::{self, RichText};
use naves_core::render::{MapLayer, WALK_LIMIT_DEG};

use naves_core::heightmap::MapGeo;

use super::{Level, LevelDef, number_row, ready_data, waiting};
use crate::gui::layers::LayerDef;
use crate::gui::selection::Selection;
use crate::gui::theme::INK;
use crate::gui::viewport::WorldPoint;
use crate::gui::{NavesApp, dem_files_ui};

/// The level.
pub const DEF: LevelDef = LevelDef {
    level: Level::Terrain,
    panel,
    status: "Select: click to read the height and slope here.",
    pick: Some(pick),
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

/// Position, latitude and longitude, height and slope at a point.
fn pick(app: &NavesApp, at: WorldPoint) -> Option<Selection> {
    let data = ready_data(app)?;
    let map = app.form.project.map_size_m();
    if !(0.0..=map).contains(&at.x) || !(0.0..=map).contains(&at.y) {
        return None;
    }
    let sample = data.heightmap.sample(at.x, at.y);
    let mut rows = vec![(
        "Position".to_string(),
        format!("{:.0} m east, {:.0} m north", at.x, at.y),
    )];
    if let Ok(geo) = MapGeo::new(&app.form.project)
        && let Ok((lat, lon)) = geo.to_geo(at.x, at.y)
    {
        rows.push(("Latitude".to_string(), format!("{lat:.5}°")));
        rows.push(("Longitude".to_string(), format!("{lon:.5}°")));
    }
    rows.push(("Height".to_string(), format!("{:.1} m", sample.height)));
    rows.push(("Slope".to_string(), format!("{:.1}°", sample.slope_deg())));
    Some(Selection {
        level: Level::Terrain,
        title: "Terrain".to_string(),
        rows,
        origin: None,
        at,
        span_m: None,
    })
}

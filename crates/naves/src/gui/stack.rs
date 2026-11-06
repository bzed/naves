// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The right column: base maps and the stack of levels, drawn like a geologist's column.
//!
//! The strata touch, with no gap and no rounding. A closed level is 24 px: a 28 px block in its colour holding the
//! digit, the name beside it on the panel. The active level is thicker and fills its whole row.

use eframe::egui::{self, Align2, Color32, FontId, Sense, WidgetInfo};
use naves_core::render::Overlay;
use naves_core::surface::{SurfaceClass, SurfaceGrid};

use super::NavesApp;
use super::levels::Level;
use super::theme::{GREEN, INK};

/// The base maps, in the order they are listed.
pub const BASE_MAPS: [(Overlay, &str); 5] = [
    (Overlay::Hillshade, "Relief"),
    (Overlay::Slope, "Slope"),
    (Overlay::Height, "Height"),
    (Overlay::Surface, "Ground"),
    (Overlay::Imagery, "Imagery"),
];

const CLOSED_ROW: f32 = 24.0;
const ACTIVE_ROW: f32 = 32.0;
const BLOCK: f32 = 28.0;

/// What the stack shows: the base map and the level you are working on. Per session, never saved with the project.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerPanel {
    /// The base map.
    pub base: Overlay,
    /// The level you are working on.
    pub active: Level,
}

impl Default for LayerPanel {
    fn default() -> Self {
        Self {
            base: Overlay::Hillshade,
            active: Level::Project,
        }
    }
}

impl LayerPanel {
    /// The colour of the band on top of the map.
    #[must_use]
    pub fn band_colour(self) -> Color32 {
        self.active.colour()
    }
}

/// The share of the map each ground class covers, in the order of [`SurfaceClass::ALL`], sampled on a fixed grid.
#[must_use]
pub fn ground_shares(surfaces: &SurfaceGrid, map_size_m: f64) -> [f64; 4] {
    const SAMPLES: u32 = 256;
    let mut counts = [0u32; 4];
    for row in 0..SAMPLES {
        for col in 0..SAMPLES {
            let x = (f64::from(col) + 0.5) / f64::from(SAMPLES) * map_size_m;
            let y = (f64::from(row) + 0.5) / f64::from(SAMPLES) * map_size_m;
            let class = surfaces.at(x, y);
            if let Some(i) = SurfaceClass::ALL.iter().position(|c| *c == class)
                && let Some(n) = counts.get_mut(i)
            {
                *n += 1;
            }
        }
    }
    let total = f64::from(SAMPLES * SAMPLES);
    counts.map(|n| f64::from(n) / total)
}

/// The base maps, the stack and what belongs to the base map in use.
pub fn stack_ui(app: &mut NavesApp, ui: &mut egui::Ui) {
    for (overlay, name) in BASE_MAPS {
        ui.radio_value(&mut app.panel.base, overlay, name);
        if overlay == app.panel.base {
            ui.indent(name, |ui| match overlay {
                Overlay::Surface => legend_ui(ui),
                Overlay::Imagery => {
                    ui.label("Ground without imagery is checkered.");
                }
                _ => {}
            });
        }
    }
    ui.add_space(14.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    let levels: Vec<Level> = app.levels.iter().map(|d| d.level).rev().collect();
    for level in levels {
        if row(ui, level, app.panel.active == level) {
            app.panel.active = level;
        }
    }
}

fn legend_ui(ui: &mut egui::Ui) {
    for class in SurfaceClass::ALL {
        let [r, g, b] = class.rgb();
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
            ui.painter()
                .rect_filled(rect, 2.0, Color32::from_rgb(r, g, b));
            ui.label(class.name());
        });
    }
}

/// One stratum. Returns whether it was clicked.
fn row(ui: &mut egui::Ui, level: Level, active: bool) -> bool {
    let height = if active { ACTIVE_ROW } else { CLOSED_ROW };
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::click());
    let label = format!("{} {}", level.digit(), level.name());
    response.widget_info(|| WidgetInfo::selected(egui::Role::Button, true, active, label.clone()));
    let painter = ui.painter();
    let font = FontId::proportional(14.0);
    if active {
        painter.rect_filled(rect, 0.0, level.colour());
        painter.text(
            rect.left_center() + egui::vec2(10.0, 0.0),
            Align2::LEFT_CENTER,
            &label,
            font,
            level.band_text(),
        );
    } else {
        let block = egui::Rect::from_min_size(rect.min, egui::vec2(BLOCK, height));
        painter.rect_filled(block, 0.0, level.colour());
        painter.text(
            block.center(),
            Align2::CENTER_CENTER,
            level.digit().to_string(),
            font.clone(),
            level.band_text(),
        );
        painter.text(
            rect.left_center() + egui::vec2(BLOCK + 8.0, 0.0),
            Align2::LEFT_CENTER,
            level.name(),
            font,
            INK,
        );
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect.shrink(1.0),
            0.0,
            egui::Stroke::new(2.0, GREEN),
            egui::StrokeKind::Inside,
        );
    }
    response
        .on_hover_text(format!("Go to {}", level.name()))
        .clicked()
}

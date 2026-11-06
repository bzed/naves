// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The right column: base maps and the stack of levels, drawn like a geologist's column.
//!
//! The strata touch, with no gap and no rounding. A closed level is 24 px: a 28 px block in its colour holding the
//! digit, the name beside it on the panel. The active level is thicker and fills its whole row.

use eframe::egui::{self, Align2, Color32, FontId, Sense, WidgetInfo};
use std::collections::BTreeSet;

use naves_core::prefs::MapPrefs;
use naves_core::render::{MapLayer, Overlay};
use naves_core::surface::{SurfaceClass, SurfaceGrid};

use super::levels::Level;
use super::theme::{GREEN, INK};
use super::{NavesApp, shortcuts};

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
const EDGE: f32 = 4.0;

/// What the stack shows: the base map and the level you are working on. Per session, never saved with the project.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerPanel {
    /// The base map.
    pub base: Overlay,
    /// The level you are working on.
    pub active: Level,
    /// The layers that are switched on.
    pub on: BTreeSet<MapLayer>,
    /// The levels that are hidden (H).
    pub hidden: BTreeSet<Level>,
    /// The other levels are drawn in full, not faded (Shift+H).
    pub show_others_in_full: bool,
}

impl Default for LayerPanel {
    fn default() -> Self {
        Self {
            base: Overlay::Hillshade,
            active: Level::Project,
            on: BTreeSet::new(),
            hidden: BTreeSet::new(),
            show_others_in_full: false,
        }
    }
}

/// The id of a base map, as the settings file spells it.
#[must_use]
pub fn overlay_id(overlay: Overlay) -> &'static str {
    match overlay {
        Overlay::Hillshade => "hillshade",
        Overlay::Slope => "slope",
        Overlay::Height => "height",
        Overlay::Surface => "surface",
        Overlay::Imagery => "imagery",
    }
}

impl LayerPanel {
    /// What a project opens with: the base map and the layers of the settings; names that do not exist are ignored.
    #[must_use]
    pub fn from_prefs(prefs: &MapPrefs) -> LayerPanel {
        let mut panel = LayerPanel::default();
        if let Some((overlay, _)) = BASE_MAPS
            .iter()
            .find(|(o, _)| overlay_id(*o) == prefs.start_base)
        {
            panel.base = *overlay;
        }
        panel.on = MapLayer::ALL
            .iter()
            .filter(|l| prefs.start_layers.iter().any(|id| id == l.id()))
            .copied()
            .collect();
        panel
    }

    /// The colour of the band on top of the map.
    #[must_use]
    pub fn band_colour(&self) -> Color32 {
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
    ui.add_space(6.0);
    let label = super::dock::problems_label(app);
    ui.checkbox(&mut app.problems.markers, label)
        .on_hover_text("Mark the problems on the map");
    ui.add_space(14.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    let levels: Vec<Level> = app.levels.iter().map(|d| d.level).rev().collect();
    for level in levels {
        let active = app.panel.active == level;
        if row(ui, level, active, &app.bindings) {
            app.activate(level);
        }
        let unfolded = ui
            .ctx()
            .animate_bool(egui::Id::unique(("unfold", level.digit())), active);
        layers_ui(app, ui, level, unfolded);
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
fn row(ui: &mut egui::Ui, level: Level, active: bool, bindings: &shortcuts::Bindings) -> bool {
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
            egui::Stroke::new(2.0, if active { level.band_text() } else { GREEN }),
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_text(tooltip(level, bindings)).clicked()
}

/// The tooltip of a level's row: where it goes and the key.
#[must_use]
pub fn tooltip(level: Level, bindings: &shortcuts::Bindings) -> String {
    match bindings.key_text(shortcuts::Action::Level(level), level) {
        Some(key) => format!("Go to {} ({key})", level.name()),
        None => format!("Go to {}", level.name()),
    }
}

/// The layers of the open level: a checkbox each, the key under it while it is on. A 4 px edge in the level's colour
/// runs down the left, so that they read as part of the stratum above.
fn layers_ui(app: &mut NavesApp, ui: &mut egui::Ui, level: Level, unfolded: f32) {
    let Some(def) = app.levels.iter().find(|d| d.level == level) else {
        return;
    };
    if def.layers.is_empty() || unfolded <= 0.0 {
        return;
    }
    // The one motion of the window: the layers unfold under their level. They are laid out at full size in a
    // child, clipped to the part that has unfolded, and their height is remembered for the next frame.
    let height_id = egui::Id::unique(("unfolded-height", level.digit()));
    let known: f32 = ui.ctx().data(|d| d.get_temp(height_id)).unwrap_or(0.0);
    let top = ui.cursor().min;
    let width = ui.available_width();
    let shown = if unfolded >= 1.0 {
        f32::MAX
    } else {
        known * unfolded
    };
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
        top,
        egui::vec2(width, f32::MAX / 4.0),
    )));
    child.set_clip_rect(
        egui::Rect::from_min_size(top, egui::vec2(width, shown)).intersect(ui.clip_rect()),
    );
    child.horizontal(|ui| {
        ui.add_space(EDGE + 8.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.add_space(4.0);
            for layer in def.layers {
                let mut on = app.panel.on.contains(&layer.layer);
                if ui
                    .checkbox(&mut on, layer.name)
                    .on_hover_text((layer.tooltip)())
                    .changed()
                {
                    if on {
                        app.panel.on.insert(layer.layer);
                    } else {
                        app.panel.on.remove(&layer.layer);
                    }
                }
                if on {
                    ui.indent(layer.name, |ui| (layer.key)(ui));
                }
            }
            ui.add_space(4.0);
        });
    });
    let full = child.min_rect().height();
    ui.ctx().data_mut(|d| d.insert_temp(height_id, full));
    let height = if unfolded >= 1.0 {
        full
    } else {
        known * unfolded
    };
    ui.allocate_space(egui::vec2(width, height));
    let edge = egui::Rect::from_min_size(top, egui::vec2(EDGE, height));
    ui.painter().rect_filled(edge, 0.0, level.colour());
}

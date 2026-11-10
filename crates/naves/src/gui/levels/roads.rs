// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Level 4: roads and paths.

use eframe::egui::{self, Color32, RichText};
use naves_core::catalog::Family;
use naves_core::checks::{DriveState, PieceCheck};
use naves_core::render::MapLayer;
use naves_export::generate::Generated;

use super::{Level, LevelDef, no_items, number_row};
use crate::gui::NavesApp;
use crate::gui::layers::LayerDef;
use crate::gui::prefs_window::{PrefsTab, PrefsWindow};
use crate::gui::problems::Problem;
use crate::gui::selection::Selection;
use crate::gui::theme::{ERROR, INK};
use crate::gui::viewport::WorldPoint;
use crate::gui::worker::RoadsState;

/// The level.
pub const DEF: LevelDef = LevelDef {
    level: Level::Roads,
    panel,
    status: "Select: click a road to read it.",
    layers: &[
        LayerDef {
            layer: MapLayer::Roads,
            name: "Roads",
            tooltip: roads_tooltip,
            key: roads_key,
        },
        LayerDef {
            layer: MapLayer::Paths,
            name: "Paths",
            tooltip: paths_tooltip,
            key: paths_key,
        },
        LayerDef {
            layer: MapLayer::Drivable,
            name: "Drivable",
            tooltip: drivable_tooltip,
            key: drivable_key,
        },
    ],
    pick: Some(pick),
    tools: &[],
    items: no_items,
};

const BROWN: Color32 = Color32::from_rgb(0x8c, 0x6a, 0x45);
const FINE: Color32 = Color32::from_rgb(0x2e, 0x6a, 0x4c);
const STEEP: Color32 = Color32::from_rgb(0xd0, 0x8a, 0x1e);
const NOT_DRIVABLE: Color32 = Color32::from_rgb(0xb0, 0x3a, 0x2e);
const WALK_ONLY: Color32 = Color32::from_rgb(0x5a, 0x66, 0x60);

/// The layers that are drawn from the generated roads and not from the heightmap alone.
#[must_use]
pub fn needs_roads(layer: MapLayer) -> bool {
    matches!(
        layer,
        MapLayer::Roads | MapLayer::Paths | MapLayer::Drivable | MapLayer::TerrainChanges
    )
}

fn roads_tooltip() -> String {
    "The roads, at their true width. Hidden while they are being placed.".to_string()
}

fn paths_tooltip() -> String {
    "Footpaths from OpenStreetMap. They are painted on the ground and get no road pieces."
        .to_string()
}

fn drivable_tooltip() -> String {
    "Whether vehicles can drive each piece of road, measured on the final terrain.".to_string()
}

fn swatch(ui: &mut egui::Ui, colour: Color32, label: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 12.0), egui::Sense::hover());
        ui.painter().line_segment(
            [rect.left_center(), rect.right_center()],
            egui::Stroke::new(3.0, colour),
        );
        ui.label(label);
    });
}

fn roads_key(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 12.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.line_segment(
            [rect.left_center(), rect.right_center()],
            egui::Stroke::new(8.0, Color32::WHITE),
        );
        painter.line_segment(
            [rect.left_center(), rect.right_center()],
            egui::Stroke::new(6.0, INK),
        );
        ui.label("Road, at its width");
    });
}

fn paths_key(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 12.0), egui::Sense::hover());
        let painter = ui.painter();
        let mut x = rect.left();
        while x < rect.right() {
            painter.line_segment(
                [
                    egui::pos2(x, rect.center().y),
                    egui::pos2((x + 4.0).min(rect.right()), rect.center().y),
                ],
                egui::Stroke::new(1.5, BROWN),
            );
            x += 7.0;
        }
        ui.label("Footpath");
    });
}

fn drivable_key(ui: &mut egui::Ui) {
    swatch(ui, FINE, "Can be driven");
    swatch(ui, STEEP, "Steep or tilted, still drivable");
    swatch(ui, NOT_DRIVABLE, "Cannot be driven");
    swatch(ui, WALK_ONLY, "Only on foot");
}

fn open_external_tools(app: &mut NavesApp) {
    let mut window = PrefsWindow::default();
    window.tab = PrefsTab::ExternalTools;
    app.prefs_window = Some(window);
}

fn panel(app: &mut NavesApp, ui: &mut egui::Ui) {
    ui.heading("Roads and paths");
    ui.add_space(6.0);
    match &app.roads {
        RoadsState::NoOsm => {
            ui.label("Add an .osm.pbf file to sources.osm to see roads.");
        }
        RoadsState::Generating => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Placing roads");
            });
        }
        RoadsState::ToolsMissing(error) => {
            ui.colored_label(ERROR, format!("Roads need armake2: {error}"));
            if ui.button("Open settings").clicked() {
                open_external_tools(app);
            }
        }
        RoadsState::Failed(error) => {
            ui.colored_label(ERROR, format!("The roads cannot be built: {error}"));
        }
        RoadsState::Ready(generated) => {
            let generated = std::sync::Arc::clone(generated);
            stats_ui(ui, &generated);
        }
    }
}

/// A short name for a road family.
fn surface_name(family: &str) -> &str {
    match family {
        "asf1" => "Wide asphalt",
        "asf2" => "Asphalt",
        "asf3" => "Narrow asphalt",
        "city" => "Streets",
        "grav" => "Gravel",
        "mud" => "Dirt tracks",
        other => other,
    }
}

fn stats_ui(ui: &mut egui::Ui, generated: &Generated) {
    let stats = &generated.stats;
    egui::Grid::new("road-stats")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            for (family, km) in &stats.km_by_family {
                number_row(ui, surface_name(family), &format!("{km:.1} km"));
            }
            number_row(ui, "Pieces", &stats.pieces.to_string());
            number_row(
                ui,
                "Stretches",
                &format!("{} ({} not closed)", stats.stretches, stats.open),
            );
            number_row(
                ui,
                "Off the OSM line",
                &format!("{:.1} m, median", stats.median_deviation_m),
            );
            number_row(
                ui,
                "Steepest grade",
                &format!("{:.0} %", stats.max_grade * 100.0),
            );
            number_row(ui, "Deepest cut", &format!("{:.1} m", stats.max_cut_m));
            number_row(ui, "Highest fill", &format!("{:.1} m", stats.max_fill_m));
            number_row(
                ui,
                "Cannot be driven",
                &format!("{:.0} m", stats.not_drivable_m),
            );
            number_row(
                ui,
                "Paths too steep",
                &format!("{:.0} m", stats.paths_too_steep_m),
            );
        });
    ui.add_space(8.0);
    if generated.violations.is_empty() {
        ui.label("Nothing to fix.");
    } else {
        ui.label(
            RichText::new(format!(
                "{} things to look at, see Problems.",
                generated.violations.len()
            ))
            .strong(),
        );
    }
}

/// The problems of the roads, each with its position on the map.
#[must_use]
pub fn problems(generated: &Generated) -> Vec<Problem> {
    generated
        .violations
        .iter()
        .map(|v| Problem {
            message: v.message.clone(),
            at: Some(WorldPoint {
                x: v.at.0,
                y: v.at.1,
            }),
            level: Some(Level::Roads),
        })
        .collect()
}

fn family_name(family: Family) -> &'static str {
    match family {
        Family::Asf1 => "Wide asphalt",
        Family::Asf2 => "Asphalt",
        Family::Asf3 => "Narrow asphalt",
        Family::City => "Street",
        Family::Grav => "Gravel",
        Family::Mud => "Dirt track",
    }
}

fn state_text(state: DriveState) -> &'static str {
    match state {
        DriveState::Fine => "Yes",
        DriveState::Steep => "Yes, steeper or more tilted than aimed",
        DriveState::NotDrivable => "No",
        DriveState::WalkOnly => "Only on foot",
        DriveState::TooSteepToWalk => "No, too steep to walk",
    }
}

fn distance_to(piece: &PieceCheck, at: WorldPoint) -> f64 {
    let (dx, dy) = (piece.end.0 - piece.start.0, piece.end.1 - piece.start.1);
    let length2 = dx * dx + dy * dy;
    let along = if length2 > 0.0 {
        (((at.x - piece.start.0) * dx + (at.y - piece.start.1) * dy) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (at.x - piece.start.0 - along * dx).hypot(at.y - piece.start.1 - along * dy)
}

/// The piece of road at a point: its model, surface, grade and whether it can be driven.
fn pick(app: &NavesApp, at: WorldPoint) -> Option<Selection> {
    let RoadsState::Ready(generated) = &app.roads else {
        return None;
    };
    let (piece, distance) = generated
        .pieces
        .iter()
        .map(|p| (p, distance_to(p, at)))
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    if distance > piece.width.max(4.0) {
        return None;
    }
    let object = generated
        .objects
        .iter()
        .filter(|o| o.id.is_some())
        .min_by(|a, b| {
            let d = |o: &naves_core::objects::PlacedObject| {
                (o.x - piece.start.0).hypot(o.y - piece.start.1)
            };
            d(a).total_cmp(&d(b))
        });
    let model = object.map_or("", |o| o.model.rsplit('\\').next().unwrap_or(&o.model));
    let rows = vec![
        ("Model".to_string(), model.to_string()),
        ("Surface".to_string(), family_name(piece.family).to_string()),
        ("Width".to_string(), format!("{:.0} m", piece.width)),
        ("Grade".to_string(), format!("{:.0} %", piece.grade * 100.0)),
        (
            "Cross fall".to_string(),
            format!("{:.0} %", piece.cross_fall * 100.0),
        ),
        (
            "Off the ground".to_string(),
            format!("{:.2} m", piece.miss_m),
        ),
        (
            "Can be driven".to_string(),
            state_text(piece.state).to_string(),
        ),
    ];
    Some(Selection {
        level: Level::Roads,
        title: "Road piece".to_string(),
        rows,
        origin: object
            .and_then(|o| o.id.as_ref())
            .map(|id| format!("Generated from {id}.")),
        at: WorldPoint {
            x: f64::midpoint(piece.start.0, piece.end.0),
            y: f64::midpoint(piece.start.1, piece.end.1),
        },
        span_m: Some(120.0),
    })
}

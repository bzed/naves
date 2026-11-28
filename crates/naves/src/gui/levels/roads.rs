// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Level 4: roads and paths.

use eframe::egui::{self, Color32, RichText};
use std::collections::BTreeMap;

use naves_core::catalog::{Family, PieceCatalog};
use naves_core::checks::{DriveState, PieceCheck};
use naves_core::features::{FeatureKind, classify};
use naves_core::render::MapLayer;
use naves_export::generate::Generated;

use super::{Level, LevelDef, ToolDef, number_row};
use crate::gui::NavesApp;
use crate::gui::edit::tools;
use crate::gui::layers::LayerDef;
use crate::gui::palette::{ItemKind, ItemSource, PaletteItem, Thumbnail};
use crate::gui::prefs_window::{PrefsTab, PrefsWindow};
use crate::gui::problems::Problem;
use crate::gui::selection::Selection;
use crate::gui::shortcuts::ToolId;
use crate::gui::theme::problem;
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
    tools: &[
        ToolDef {
            tool: ToolId::MOVE,
            word: "Move",
            status: "Move: drag a node.",
            on_event: Some(tools::move_event),
        },
        ToolDef {
            tool: ToolId::LINE,
            word: "Draw",
            status: DRAW_STATUS,
            on_event: Some(tools::draw_event),
        },
        ToolDef {
            tool: ToolId::CUT,
            word: "Cut",
            status: "Cut: click a node inside a road.",
            on_event: Some(tools::cut_event),
        },
        ToolDef {
            tool: ToolId::JOIN,
            word: "Join",
            status: "Join: click the end of a road, then the end of another.",
            on_event: Some(tools::join_event),
        },
        ToolDef {
            tool: ToolId::BRIDGE,
            word: "Bridge",
            status: "Bridge: click two nodes of a road to make the part between them a bridge.",
            on_event: Some(tools::bridge_event),
        },
    ],
    items,
};

/// What the status line says while Draw is the tool. Draw is a tool of this level; its rail entry uses the same text.
pub const DRAW_STATUS: &str =
    "Draw: click to add points, click a node to join, Space to finish, Ctrl+Z to take one back.";

/// A class of line: what a way becomes when it is drawn with it.
#[derive(Debug, Clone, Copy)]
pub struct RoadClass {
    /// The id; the item is `roads.class.<id>`.
    pub id: &'static str,
    /// The name in the list.
    pub name: &'static str,
    /// The group in the list.
    pub group: &'static str,
    /// The OpenStreetMap tags that make a way of this class.
    pub tags: &'static [(&'static str, &'static str)],
}

const fn class(
    id: &'static str,
    name: &'static str,
    group: &'static str,
    tags: &'static [(&'static str, &'static str)],
) -> RoadClass {
    RoadClass {
        id,
        name,
        group,
        tags,
    }
}

/// The classes of line, widest first. A road's width is its surface family's, a path's its trace's.
pub const ROAD_CLASSES: &[RoadClass] = &[
    class("primary", "Primary", "Roads", &[("highway", "primary")]),
    class(
        "secondary",
        "Secondary",
        "Roads",
        &[("highway", "secondary")],
    ),
    class("tertiary", "Tertiary", "Roads", &[("highway", "tertiary")]),
    class(
        "residential",
        "Residential street",
        "Roads",
        &[("highway", "residential")],
    ),
    class(
        "service",
        "Service road",
        "Roads",
        &[("highway", "service")],
    ),
    class(
        "track1",
        "Track, grade 1",
        "Roads",
        &[("highway", "track"), ("tracktype", "grade1")],
    ),
    class(
        "track2",
        "Track, grade 2",
        "Roads",
        &[("highway", "track"), ("tracktype", "grade2")],
    ),
    class(
        "track3",
        "Track, grade 3",
        "Roads",
        &[("highway", "track"), ("tracktype", "grade3")],
    ),
    class(
        "track4",
        "Track, grade 4",
        "Roads",
        &[("highway", "track"), ("tracktype", "grade4")],
    ),
    class(
        "track5",
        "Track, grade 5",
        "Roads",
        &[("highway", "track"), ("tracktype", "grade5")],
    ),
    class("footway", "Footpath", "Paths", &[("highway", "footway")]),
    class("path", "Path", "Paths", &[("highway", "path")]),
];

/// The width in metres of what `tags` make, or 2 m when they make nothing the generator knows.
fn class_width(tags: &[(&str, &str)]) -> f64 {
    let tags: BTreeMap<String, String> = tags
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    match classify(&tags) {
        Some(FeatureKind::Road { family, .. }) => family.width_m(),
        Some(FeatureKind::Path { width_m, .. }) => width_m,
        None => 2.0,
    }
}

/// The ink of a road's line in the list; paths are quieter.
const ROAD_INK: [u8; 3] = [0xe4, 0xe8, 0xec];
const PATH_INK: [u8; 3] = [0x9a, 0xa4, 0xae];

/// The items of the level: the classes of line, then the game's pieces, junctions and bridges once they are read.
#[must_use]
pub fn items(app: &NavesApp) -> Vec<PaletteItem> {
    let mut items: Vec<PaletteItem> = ROAD_CLASSES
        .iter()
        .map(|c| {
            let width_m = class_width(c.tags);
            PaletteItem {
                id: format!("roads.class.{}", c.id),
                name: c.name.to_string(),
                group: c.group.to_string(),
                thumbnail: Thumbnail::Line {
                    width_m,
                    rgb: if c.group == "Paths" {
                        PATH_INK
                    } else {
                        ROAD_INK
                    },
                },
                kind: ItemKind::LineClass,
                default_tool: ToolId::LINE,
                source: ItemSource::Game,
                fits: vec![Level::Roads],
                size_m: Some(width_m),
            }
        })
        .collect();
    items.extend(app.models());
    items
}

/// The plan view of a model: the rectangle of its width between its ports, lying along the row.
#[expect(
    clippy::cast_possible_truncation,
    reason = "metres of a model, far inside f32"
)]
fn footprint(length_m: f64, width_m: f64) -> Thumbnail {
    let (x, y) = ((length_m / 2.0) as f32, (width_m / 2.0) as f32);
    Thumbnail::Footprint {
        outline: vec![(-x, -y), (x, -y), (x, y), (-x, y)],
    }
}

/// The last part of a model path, without the extension: the name in the list.
fn model_name(model: &str) -> String {
    let file = model.rsplit(['\\', '/']).next().unwrap_or(model);
    file.strip_suffix(".p3d").unwrap_or(file).to_string()
}

fn model_item(model: &str, group: &str, length_m: f64, width_m: f64) -> PaletteItem {
    PaletteItem {
        id: format!("model.{model}"),
        name: model_name(model),
        group: group.to_string(),
        thumbnail: footprint(length_m, width_m),
        kind: ItemKind::Model,
        default_tool: ToolId::SELECT, // placing comes in a later version
        source: ItemSource::Game,
        fits: vec![Level::Roads],
        size_m: Some(length_m),
    }
}

/// One item for every road piece, junction and bridge of the game's catalogue.
#[must_use]
pub fn catalog_items(catalog: &PieceCatalog) -> Vec<PaletteItem> {
    let mut items: Vec<PaletteItem> = Vec::new();
    for family in [
        Family::Asf1,
        Family::Asf2,
        Family::Asf3,
        Family::City,
        Family::Grav,
        Family::Mud,
    ] {
        for piece in catalog.pieces(family) {
            items.push(model_item(
                &piece.model,
                "Pieces",
                piece.chord(),
                piece.width,
            ));
        }
    }
    for junction in catalog.junctions() {
        let length = (junction.end.x - junction.start.x).hypot(junction.end.z - junction.start.z);
        items.push(model_item(
            &junction.model,
            "Junctions",
            length,
            junction.through.width_m(),
        ));
    }
    for bridge in catalog.bridges() {
        items.push(model_item(
            &bridge.model,
            "Bridges",
            bridge.length,
            bridge.width,
        ));
    }
    // a model that is both a T and an X junction is listed once
    let mut seen = std::collections::BTreeSet::new();
    items.retain(|i| seen.insert(i.id.clone()));
    items
}

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
            egui::Stroke::new(6.0, Color32::BLACK),
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
    // under the list of classes and models
    ui.add_space(8.0);
    match &app.roads {
        RoadsState::NoOsm => {
            ui.label("No OpenStreetMap file: draw the roads yourself, or add one to sources.osm.");
        }
        RoadsState::Generating => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Placing roads");
            });
        }
        RoadsState::ToolsMissing(error) => {
            ui.colored_label(
                problem(ui.visuals()),
                format!("▲ Roads need dayzmake: {error}"),
            );
            if ui.button("Open settings").clicked() {
                open_external_tools(app);
            }
        }
        RoadsState::Failed(error) => {
            ui.colored_label(
                problem(ui.visuals()),
                format!("▲ The roads cannot be built: {error}"),
            );
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
        target: None,
    })
}

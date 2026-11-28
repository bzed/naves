// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The levels of the editor: what you are working on, from the source data up to what lives on it.

use eframe::egui::{self, Color32};

use super::NavesApp;
use super::layers::LayerDef;
use super::palette::PaletteItem;
use super::selection::Selection;
use super::shortcuts::ToolId;
use super::theme::contrast;
use super::viewport::{ViewEvent, WorldPoint};

/// A level of the stack. The digit is the key and never changes; a level appears in the editor when it is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// 0: area, scales and sources.
    Project,
    /// 1: the shape of the ground.
    Terrain,
    /// 2: surfaces.
    Ground,
    /// 3
    Water,
    /// 4
    Roads,
    /// 5
    Vegetation,
    /// 6
    Buildings,
    /// 7
    Objects,
    /// 8
    Life,
    /// 9
    Economy,
}

impl Level {
    /// All levels, bottom first.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the settings window lists them (Task 10)")
    )]
    pub const ALL: [Level; 10] = [
        Level::Project,
        Level::Terrain,
        Level::Ground,
        Level::Water,
        Level::Roads,
        Level::Vegetation,
        Level::Buildings,
        Level::Objects,
        Level::Life,
        Level::Economy,
    ];

    /// The key that goes to this level.
    #[must_use]
    pub fn digit(self) -> u8 {
        match self {
            Level::Project => 0,
            Level::Terrain => 1,
            Level::Ground => 2,
            Level::Water => 3,
            Level::Roads => 4,
            Level::Vegetation => 5,
            Level::Buildings => 6,
            Level::Objects => 7,
            Level::Life => 8,
            Level::Economy => 9,
        }
    }

    /// The name in the stack.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Level::Project => "Project",
            Level::Terrain => "Terrain",
            Level::Ground => "Ground",
            Level::Water => "Water",
            Level::Roads => "Roads and paths",
            Level::Vegetation => "Vegetation",
            Level::Buildings => "Buildings",
            Level::Objects => "Objects",
            Level::Life => "Life",
            Level::Economy => "Economy",
        }
    }

    /// The stable id, used in the settings file and the command palette.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Level::Project => "level.project",
            Level::Terrain => "level.terrain",
            Level::Ground => "level.ground",
            Level::Water => "level.water",
            Level::Roads => "level.roads",
            Level::Vegetation => "level.vegetation",
            Level::Buildings => "level.buildings",
            Level::Objects => "level.objects",
            Level::Life => "level.life",
            Level::Economy => "level.economy",
        }
    }

    /// The band colour: the four fixed bands (terrain, ground, water, roads) are the colour-blind safe set, the
    /// others keep their hue and have 3:1 on the dark ground.
    #[must_use]
    pub fn colour(self) -> Color32 {
        let rgb = match self {
            Level::Project => [0x6b, 0x79, 0x72],
            Level::Terrain => [0xcc, 0x79, 0xa7],
            Level::Ground => [0x00, 0x9e, 0x73],
            Level::Water => [0x00, 0x72, 0xb2],
            Level::Roads => [0xe4, 0xe8, 0xec],
            Level::Vegetation => [0x3a, 0x85, 0x5f],
            Level::Buildings => [0xb6, 0x5d, 0x47],
            Level::Objects => [0x72, 0x77, 0x7e],
            Level::Life => [0x9d, 0x61, 0xa3],
            Level::Economy => [0xb5, 0x8a, 0x1e],
        };
        Color32::from_rgb(rgb[0], rgb[1], rgb[2])
    }

    /// Text on the band's colour: white, or ink where white would be under 4.5:1.
    #[must_use]
    pub fn band_text(self) -> Color32 {
        if contrast(self.colour(), Color32::WHITE) >= 4.5 {
            Color32::WHITE
        } else {
            Color32::BLACK
        }
    }
}

/// What the shell needs to know about a level that is built.
pub struct LevelDef {
    /// Which level.
    pub level: Level,
    /// The left panel, when the level has no items.
    pub panel: fn(&mut NavesApp, &mut egui::Ui),
    /// The status line while nothing else speaks.
    pub status: &'static str,
    /// The map layers of the level, in the order of the stack.
    pub layers: &'static [LayerDef],
    /// What a click on the map picks in this level, if anything.
    pub pick: Option<fn(&NavesApp, WorldPoint) -> Option<Selection>>,
    /// The tools of the level besides Select, for the rail. Empty until a level has more than one.
    pub tools: &'static [ToolDef],
    /// What the level can add, for the items panel. Empty until a level has any.
    pub items: fn(&NavesApp) -> Vec<PaletteItem>,
}

/// A tool of the rail.
pub struct ToolDef {
    /// Which tool; its key is in the shortcut table.
    pub tool: ToolId,
    /// The word below the letter.
    pub word: &'static str,
    /// The status line while the tool is active: what it does and what to press.
    pub status: &'static str,
    /// What the tool does with what happens in the view, in map metres.
    pub on_event: Option<fn(&mut NavesApp, &ViewEvent)>,
}

/// The empty items function of a level that adds nothing yet.
pub fn no_items(_: &NavesApp) -> Vec<PaletteItem> {
    Vec::new()
}

/// The levels that are built, bottom first. A later plan adds its level here and nowhere else.
pub const BUILT: &[&LevelDef] = &[&project::DEF, &terrain::DEF, &ground::DEF, &roads::DEF];

pub mod ground;
pub mod project;
pub mod roads;
pub mod terrain;

/// Right-aligned number next to its label, in a two-column grid.
pub(super) fn number_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(label);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.label(value);
    });
    ui.end_row();
}

/// The data once it is there.
pub(super) fn ready_data(app: &NavesApp) -> Option<std::sync::Arc<super::worker::MapData>> {
    match &app.heightmap {
        super::worker::HeightmapState::Ready(data) => Some(std::sync::Arc::clone(data)),
        _ => None,
    }
}

/// What a panel shows while the map has no data: what to do, or that it is being read.
pub(super) fn waiting(
    app: &NavesApp,
    ui: &mut egui::Ui,
) -> Option<std::sync::Arc<super::worker::MapData>> {
    use super::worker::HeightmapState;
    match &app.heightmap {
        HeightmapState::Ready(data) => return Some(std::sync::Arc::clone(data)),
        HeightmapState::NoDem => {
            ui.label("Add elevation files to see the map.");
        }
        HeightmapState::Loading => {
            ui.label("Reading the elevation data.");
        }
        HeightmapState::Failed(error) => {
            ui.colored_label(
                super::theme::problem(ui.visuals()),
                format!("The map cannot be read: {error}"),
            );
        }
    }
    None
}

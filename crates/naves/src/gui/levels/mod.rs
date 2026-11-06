// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The levels of the editor: what you are working on, from the source data up to what lives on it.

use eframe::egui::Color32;

use super::theme::{INK, contrast};

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

    /// The stable id, used in the settings file.
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

    /// The band colour.
    #[must_use]
    pub fn colour(self) -> Color32 {
        let rgb = match self {
            Level::Project => [0x5a, 0x66, 0x60],
            Level::Terrain => [0x8c, 0x6a, 0x45],
            Level::Ground => [0xa3, 0x9a, 0x5c],
            Level::Water => [0x3f, 0x6f, 0x9e],
            Level::Roads => [0x1d, 0x29, 0x24],
            Level::Vegetation => [0x2e, 0x6a, 0x4c],
            Level::Buildings => [0x9a, 0x4f, 0x3c],
            Level::Objects => [0x6b, 0x6f, 0x76],
            Level::Life => [0x7a, 0x4a, 0x7f],
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
            INK
        }
    }
}

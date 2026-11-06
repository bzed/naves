// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a click on the map picks: the active level only, or with Alt every level from the top down.

use eframe::egui::Modifiers;

use super::NavesApp;
use super::levels::Level;
use super::viewport::WorldPoint;

/// A thing that was picked, with what to say about it.
///
/// Today a click reads a point of the ground. When there are objects, they return their own `Selection`.
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    /// The level it belongs to.
    pub level: Level,
    /// What it is.
    pub title: String,
    /// Its properties, name and value.
    pub rows: Vec<(String, String)>,
    /// Where it came from, for the last line: "Generated from road 8421, piece 17."
    pub origin: Option<String>,
    /// Where it is, for the marker and for Frame selection.
    pub at: WorldPoint,
    /// How many metres to show when framing it; `None` keeps the zoom.
    pub span_m: Option<f64>,
}

/// What a click at `at` picks. In the active level only, or with Alt in every built level, the highest first;
/// a hidden level picks nothing.
#[must_use]
pub fn pick_at(app: &NavesApp, at: WorldPoint, modifiers: Modifiers) -> Option<Selection> {
    let pick = |def: &&super::levels::LevelDef| {
        if app.panel.hidden.contains(&def.level) {
            return None;
        }
        (def.pick?)(app, at)
    };
    if modifiers.alt {
        app.levels.iter().rev().find_map(pick)
    } else {
        app.levels
            .iter()
            .filter(|d| d.level == app.panel.active)
            .find_map(pick)
    }
}

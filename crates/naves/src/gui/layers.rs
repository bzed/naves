// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Map layers: what a level can draw over the base map, and which of them are visible.

use eframe::egui;
use naves_core::render::MapLayer;

use super::levels::LevelDef;
use super::stack::LayerPanel;
use super::viewport::VisibleLayer;

/// A layer as the stack shows it.
pub struct LayerDef {
    /// Which layer.
    pub layer: MapLayer,
    /// The name of the checkbox.
    pub name: &'static str,
    /// What the row means, with the numbers from the constants.
    pub tooltip: fn() -> String,
    /// The key under the row while the layer is on: the symbol as drawn on the map.
    pub key: fn(&mut egui::Ui),
}

/// The layers to draw, bottom first: nothing from a hidden level, full strength for the active level and for
/// all levels while Shift+H is on, `fade` for the others.
#[must_use]
pub fn visible(panel: &LayerPanel, levels: &[&LevelDef], fade: f32) -> Vec<VisibleLayer> {
    MapLayer::ALL
        .iter()
        .filter(|layer| panel.on.contains(layer))
        .filter_map(|layer| {
            let level = levels
                .iter()
                .find(|d| d.layers.iter().any(|l| l.layer == *layer))?
                .level;
            if panel.hidden.contains(&level) {
                return None;
            }
            let opacity = if level == panel.active || panel.show_others_in_full {
                1.0
            } else {
                fade
            };
            Some(VisibleLayer {
                layer: *layer,
                opacity,
            })
        })
        .collect()
}

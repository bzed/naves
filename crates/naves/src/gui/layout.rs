// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! How the window gives way when it gets small: the dock folds first, then the left panel. The map stays.

use eframe::egui::Vec2;

/// Below this height the dock shows only its tab row.
const DOCK_FOLDS_BELOW: f32 = 560.0;
/// Below this width the left panel is a thin strip (left 300 + rail 56 + right 230 + 300 of map, rounded).
const LEFT_FOLDS_BELOW: f32 = 900.0;

/// What is folded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fold {
    /// The bottom dock shows only its tabs.
    pub dock: bool,
    /// The left panel is a strip.
    pub left: bool,
}

/// What folds in a window of this size.
#[must_use]
pub fn fold_for(window: Vec2) -> Fold {
    Fold {
        dock: window.y < DOCK_FOLDS_BELOW,
        left: window.x < LEFT_FOLDS_BELOW,
    }
}

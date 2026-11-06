// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The look of the editor: the manual's palette on egui's light theme.

use eframe::egui::{self, Color32};

/// Village green: accents, the Build button, keyboard focus.
pub const GREEN: Color32 = Color32::from_rgb(0x2e, 0x6a, 0x4c);
/// Errors.
pub const ERROR: Color32 = Color32::from_rgb(0xb0, 0x3a, 0x2e);
/// Text.
pub const INK: Color32 = Color32::from_rgb(0x1d, 0x29, 0x24);
/// The ground of the window.
pub const GROUND: Color32 = Color32::from_rgb(0xf2, 0xf5, 0xec);
/// Side panels and the dock.
pub const PANEL: Color32 = Color32::from_rgb(0xe7, 0xed, 0xe0);

/// The background of a selected row: a light green that ink reads on.
pub const SELECTED: Color32 = Color32::from_rgb(0xbd, 0xd6, 0xc2);

/// The outline of boxes, radio buttons and fields.
pub const EDGE: Color32 = Color32::from_rgb(0x6a, 0x75, 0x6d);

/// Sets the theme: the manual's palette. Keyboard focus is a green ring, which shows on the light panel.
pub fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = GROUND;
    visuals.window_fill = GROUND;
    visuals.extreme_bg_color = Color32::WHITE;
    visuals.override_text_color = Some(INK);
    visuals.selection.bg_fill = SELECTED;
    visuals.selection.stroke.color = GREEN;
    visuals.hyperlink_color = GREEN;
    // an unchecked box, a radio button and a field must show on the panel: 3:1 against it
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, EDGE);
    visuals.widgets.hovered.bg_stroke.color = GREEN;
    visuals.widgets.active.bg_fill = GREEN;
    ctx.set_visuals(visuals);
}

fn linear(c: u8) -> f32 {
    let s = f32::from(c) / 255.0;
    if s <= 0.039_28 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(c: Color32) -> f32 {
    0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
}

/// The WCAG contrast ratio of two colours, from 1 to 21.
#[must_use]
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

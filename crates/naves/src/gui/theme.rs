// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

// SPDX-License-Identifier: AGPL-3.0-or-later

//! The look of the editor: the Náves design system's theme, with the manual's green accents.

use eframe::egui::{self, Color32};

/// Village green: accents, the Build button, keyboard focus.
pub const GREEN: Color32 = Color32::from_rgb(0x2e, 0x6a, 0x4c);
/// Errors.
pub const ERROR: Color32 = Color32::from_rgb(0xb0, 0x3a, 0x2e);

/// Sets the theme: the design system installs its tokens (fills, text, selection, spacing, the
/// Inter font); light and dark both work, and egui follows the system theme. What is Náves' own
/// on top of it are the accents: [`GREEN`] and [`ERROR`], which are tested to read on both themes.
pub fn apply_theme(ctx: &egui::Context) {
    naves_ui::apply_style_and_install_loaders(ctx);
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

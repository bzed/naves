// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

// SPDX-License-Identifier: AGPL-3.0-or-later

//! The look of the editor: the Náves design system's theme with the dark chrome palette on top.

use eframe::egui::{self, Color32};
pub use naves_core::prefs::ThemeChoice;

/// The chrome palette of the dark theme. Colour is never the only cue: a problem carries `▲` and a word, a
/// notice `●`. Every colour here is tested against [`tokens::GROUND`].
pub mod tokens {
    use eframe::egui::Color32;

    /// The map's surround and the text fields.
    pub const GROUND: Color32 = Color32::from_rgb(0x1e, 0x23, 0x29);
    /// Panels and windows.
    pub const PANEL: Color32 = Color32::from_rgb(0x27, 0x2d, 0x35);
    /// Raised surfaces: stripes, hovered rows.
    pub const RAISED: Color32 = Color32::from_rgb(0x32, 0x3a, 0x44);
    /// Text.
    pub const INK: Color32 = Color32::from_rgb(0xe4, 0xe8, 0xec);
    /// Secondary text.
    pub const INK_QUIET: Color32 = Color32::from_rgb(0x9a, 0xa4, 0xae);
    /// What is on, selected or focused.
    pub const ACTIVE: Color32 = Color32::from_rgb(0x56, 0xb4, 0xe9);
    /// What is wrong; always with `▲` and a word.
    pub const PROBLEM: Color32 = Color32::from_rgb(0xd5, 0x5e, 0x00);
    /// What needs a look; always with `●`.
    pub const NOTICE: Color32 = Color32::from_rgb(0xf0, 0xe4, 0x42);
    /// The box of a checkbox or radio button.
    pub const BOX: Color32 = Color32::from_rgb(0x75, 0x80, 0x8c);
    /// [`BOX`] under the pointer.
    pub const BOX_HOVERED: Color32 = Color32::from_rgb(0x7a, 0x85, 0x91);
    /// The box of a checkbox or radio button on the light theme.
    pub const BOX_LIGHT: Color32 = Color32::from_rgb(0x85, 0x8d, 0x96);
    /// [`ACTIVE`] on the light theme, where the sky blue would be under 3:1 on white.
    pub const ACTIVE_LIGHT: Color32 = Color32::from_rgb(0x00, 0x72, 0xb2);
}

/// The accent for what is on, selected or focused, for the theme the visuals belong to.
#[must_use]
pub fn active(visuals: &egui::Visuals) -> Color32 {
    if visuals.dark_mode {
        tokens::ACTIVE
    } else {
        tokens::ACTIVE_LIGHT
    }
}

/// The colour for what is wrong. It reads on both themes (3:1 or more), and is always shown with `▲` and a word.
#[must_use]
pub fn problem(_visuals: &egui::Visuals) -> Color32 {
    tokens::PROBLEM
}

/// The text colour on a fill of [`active`].
#[must_use]
pub fn on_active(visuals: &egui::Visuals) -> Color32 {
    if visuals.dark_mode {
        tokens::GROUND
    } else {
        Color32::WHITE
    }
}

/// The frame of a side panel, the rail and the dock: the panel colour of the theme (dark: [`tokens::PANEL`]).
pub fn panel_frame(visuals: &egui::Visuals) -> egui::Frame {
    egui::Frame::new().fill(visuals.panel_fill)
}

/// The frame behind the map and the status line: the ground colour of the theme (dark: [`tokens::GROUND`]).
pub fn ground_frame(visuals: &egui::Visuals) -> egui::Frame {
    egui::Frame::new().fill(visuals.extreme_bg_color)
}

/// Sets the theme: the design system installs its tokens (fills, text, spacing, the Inter font) for both
/// themes, then the dark one gets the chrome palette and `choice` picks the one in use. Call it once; to
/// change the choice later call [`set_choice`].
pub fn apply_theme(ctx: &egui::Context, choice: ThemeChoice) {
    naves_ui::apply_style_and_install_loaders(ctx);
    let mut dark = std::sync::Arc::unwrap_or_clone(ctx.style_of(egui::Theme::Dark));
    let v = &mut dark.visuals;
    v.panel_fill = tokens::PANEL;
    v.window_fill = tokens::PANEL;
    v.extreme_bg_color = tokens::GROUND;
    v.faint_bg_color = tokens::RAISED;
    v.selection.bg_fill = tokens::ACTIVE.gamma_multiply(0.35);
    v.selection.stroke = egui::Stroke::new(1.0, tokens::ACTIVE);
    v.widgets.active.bg_stroke = egui::Stroke::new(1.0, tokens::ACTIVE);
    v.widgets.noninteractive.fg_stroke.color = tokens::INK;
    // the design system draws no edge on a widget, so the box of an unticked checkbox or radio button is its
    // fill: a slate that is 3:1 against the panel and keeps the check mark 3:1 against it
    v.widgets.inactive.bg_fill = tokens::BOX;
    v.widgets.hovered.bg_fill = tokens::BOX_HOVERED;
    v.widgets.inactive.fg_stroke.color = tokens::INK;
    v.weak_text_color = Some(tokens::INK_QUIET);
    v.warn_fg_color = tokens::NOTICE;
    ctx.set_style_of(egui::Theme::Dark, dark);
    let mut light = std::sync::Arc::unwrap_or_clone(ctx.style_of(egui::Theme::Light));
    light.visuals.widgets.inactive.bg_fill = tokens::BOX_LIGHT;
    light.visuals.widgets.hovered.bg_fill = tokens::BOX_LIGHT;
    light.visuals.selection.stroke = egui::Stroke::new(1.0, tokens::ACTIVE_LIGHT);
    ctx.set_style_of(egui::Theme::Light, light);
    set_choice(ctx, choice);
}

/// Dark, light, or the system's.
pub fn set_choice(ctx: &egui::Context, choice: ThemeChoice) {
    ctx.set_theme(preference(choice));
}

/// The egui preference of a choice.
#[must_use]
pub fn preference(choice: ThemeChoice) -> egui::ThemePreference {
    match choice {
        ThemeChoice::Dark => egui::ThemePreference::Dark,
        ThemeChoice::Light => egui::ThemePreference::Light,
        ThemeChoice::System => egui::ThemePreference::System,
    }
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

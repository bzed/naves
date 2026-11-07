// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

// SPDX-License-Identifier: AGPL-3.0-or-later

//! The look: the design system's theme, with Náves' green accents on top of it.

use egui_kittest::Harness;
use naves_ui::egui;

use crate::gui::theme::{ERROR, GREEN, contrast};

/// The theme installs the design system's Inter font.
#[test]
fn apply_theme_installs_inter() {
    let mut harness = Harness::builder()
        .with_context_setup(crate::gui::theme::apply_theme)
        .build_ui(|ui| {
            ui.label("probe");
        });
    harness.run();

    let inter_is_first = harness.ctx.fonts(|fonts| {
        fonts
            .definitions()
            .families
            .get(&egui::FontFamily::Proportional)
            .is_some_and(|fonts| fonts.first().is_some_and(|name| name == "Inter-Variable"))
    });
    assert!(
        inter_is_first,
        "the design system's Inter-Variable must be the proportional font"
    );
}

/// Náves' green and red accents must still read when egui runs the dark theme
/// (a dark system theme is followed automatically).
#[test]
fn the_accents_read_on_the_dark_panel() {
    let dark = naves_ui::design_tokens_of(egui::Theme::Dark);
    for (name, colour) in [("green", GREEN), ("error", ERROR)] {
        let ratio = contrast(colour, dark.panel_bg_color);
        assert!(
            ratio >= 3.0,
            "{name}: {ratio:.2} on {:?}",
            dark.panel_bg_color
        );
    }
}

/// The same accents on the light panel.
#[test]
fn the_accents_read_on_the_light_panel() {
    let light = naves_ui::design_tokens_of(egui::Theme::Light);
    for (name, colour) in [("green", GREEN), ("error", ERROR)] {
        let ratio = contrast(colour, light.panel_bg_color);
        assert!(
            ratio >= 3.0,
            "{name}: {ratio:.2} on {:?}",
            light.panel_bg_color
        );
    }
}

/// Error text is text: it needs the text contrast on the light panel it is drawn on.
#[test]
fn error_text_still_reads_on_the_light_panel() {
    let light = naves_ui::design_tokens_of(egui::Theme::Light);
    let ratio = contrast(ERROR, light.panel_bg_color);
    assert!(ratio >= 4.5, "{ratio:.2}");
}

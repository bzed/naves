// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

// SPDX-License-Identifier: AGPL-3.0-or-later

//! The look: the design system's theme, with the dark chrome palette on top of it.

use egui_kittest::Harness;
use naves_ui::egui::{self, Color32};

use naves_core::prefs::ThemeChoice;

use crate::gui::levels::Level;
use crate::gui::theme::{active, contrast, problem, tokens};

/// The theme installs the design system's Inter font.
#[test]
fn apply_theme_installs_inter() {
    let mut harness = Harness::builder()
        .with_context_setup(|ctx| crate::gui::theme::apply_theme(ctx, ThemeChoice::Dark))
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

/// The chrome reads on the ground: text 7:1 (4.5:1 for the quiet text), accents 3:1.
#[test]
fn chrome_tokens_read() {
    assert!(contrast(tokens::INK, tokens::GROUND) >= 7.0);
    assert!(contrast(tokens::INK_QUIET, tokens::GROUND) >= 4.5);
    assert!(contrast(tokens::ACTIVE, tokens::GROUND) >= 3.0);
    assert!(contrast(tokens::PROBLEM, tokens::GROUND) >= 3.0);
    assert!(contrast(tokens::NOTICE, tokens::GROUND) >= 3.0);
    assert!(contrast(tokens::INK, tokens::PANEL) >= 7.0);
    assert!(contrast(tokens::INK_QUIET, tokens::PANEL) >= 4.5);
}

/// On the light theme the accents are the ones for white.
#[test]
fn the_accents_read_on_white() {
    let light = egui::Visuals::light();
    assert!(contrast(active(&light), Color32::WHITE) >= 3.0);
    assert!(contrast(problem(&light), Color32::WHITE) >= 3.0);
    let dark = egui::Visuals::dark();
    assert_eq!(active(&dark), tokens::ACTIVE);
    assert!(
        contrast(Color32::WHITE, active(&light)) >= 4.5,
        "white text on the light fill"
    );
    assert!(
        contrast(tokens::GROUND, active(&dark)) >= 4.5,
        "text on the dark fill"
    );
}

/// The level bands read on the ground; the four fixed ones are the colour-blind safe set.
#[test]
fn bands_read_on_ground() {
    for level in Level::ALL {
        let ratio = contrast(level.colour(), tokens::GROUND);
        assert!(ratio >= 3.0, "{level:?}: {ratio:.2}");
        assert!(
            contrast(level.colour(), level.band_text()) >= 4.5,
            "{level:?}"
        );
    }
    let rgb = |l: Level| {
        let c = l.colour();
        [c.r(), c.g(), c.b()]
    };
    assert_eq!(rgb(Level::Terrain), [0xcc, 0x79, 0xa7]);
    assert_eq!(rgb(Level::Ground), [0x00, 0x9e, 0x73]);
    assert_eq!(rgb(Level::Water), [0x00, 0x72, 0xb2]);
    assert_eq!(rgb(Level::Roads), [0xe4, 0xe8, 0xec]);
}

#[test]
fn the_default_theme_is_dark() {
    let root = tempfile::tempdir().unwrap();
    let dir = super::project(root.path());
    let app = super::app(root.path(), &dir);
    let mut h = super::state_harness(app);
    h.run_steps(3);
    let style = h.ctx.global_style();
    assert!(style.visuals.dark_mode);
    assert_eq!(style.visuals.panel_fill, tokens::PANEL);
}

#[test]
fn light_can_be_chosen_in_the_map_tab() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let root = tempfile::tempdir().unwrap();
    let dir = super::project(root.path());
    let path = naves_core::prefs::default_path(&root.path().join("config/naves"));
    let app = super::app(root.path(), &dir).with_preferences(path.clone());
    let mut h = super::state_harness(app);
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Settings").click();
    h.run_steps(8);
    h.get_by_role_and_label(Role::Tab, "Map").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::RadioButton, "Light").click();
    h.run_steps(3);
    assert!(!h.ctx.global_style().visuals.dark_mode);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("theme = \"light\""), "{text}");
}

/// The panels are painted with the palette: egui's `Frame::new()` is transparent, which showed the window's
/// clear colour (#080808) instead of the slate chrome.
#[test]
fn the_dark_chrome_paints_the_slate_palette() {
    let mut harness = Harness::builder()
        .with_context_setup(|ctx| crate::gui::theme::apply_theme(ctx, ThemeChoice::Dark))
        .build_ui(|ui| {
            ui.label("probe");
        });
    harness.run();
    let visuals = harness.ctx.global_style().visuals.clone();
    assert!(visuals.dark_mode);
    assert_eq!(crate::gui::theme::panel_frame(&visuals).fill, tokens::PANEL);
    assert_eq!(
        crate::gui::theme::ground_frame(&visuals).fill,
        tokens::GROUND
    );
    // the active tool is the accent with the text that reads on it
    let (letter, word) = crate::gui::rail::tool_ink(true, Level::Roads, &visuals);
    assert_eq!(letter, crate::gui::theme::on_active(&visuals));
    assert_eq!(word, letter);
    assert!(contrast(letter, active(&visuals)) >= 4.5);
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The level table and the theme's contrast.

use eframe::egui::Color32;

use crate::gui::levels::Level;
use crate::gui::theme::{ERROR, GREEN, INK, PANEL, contrast};

#[test]
fn digits_never_change() {
    let digits: Vec<u8> = Level::ALL.iter().map(|l| l.digit()).collect();
    assert_eq!(digits, (0..10).collect::<Vec<u8>>());
    let names: Vec<&str> = Level::ALL.iter().map(|l| l.name()).collect();
    assert_eq!(
        names,
        [
            "Project",
            "Terrain",
            "Ground",
            "Water",
            "Roads and paths",
            "Vegetation",
            "Buildings",
            "Objects",
            "Life",
            "Economy"
        ]
    );
}

#[test]
fn colours_are_the_spec_s() {
    let hex: Vec<[u8; 3]> = Level::ALL
        .iter()
        .map(|l| {
            let c = l.colour();
            [c.r(), c.g(), c.b()]
        })
        .collect();
    let expected = [
        0x5a6660u32,
        0x8c6a45,
        0xa39a5c,
        0x3f6f9e,
        0x1d2924,
        0x2e6a4c,
        0x9a4f3c,
        0x6b6f76,
        0x7a4a7f,
        0xb58a1e,
    ];
    for (got, want) in hex.iter().zip(expected) {
        assert_eq!(*got, [(want >> 16) as u8, (want >> 8) as u8, want as u8]);
    }
}

#[test]
fn ids_are_pinned() {
    let ids: Vec<&str> = Level::ALL.iter().map(|l| l.id()).collect();
    assert_eq!(
        ids,
        [
            "level.project",
            "level.terrain",
            "level.ground",
            "level.water",
            "level.roads",
            "level.vegetation",
            "level.buildings",
            "level.objects",
            "level.life",
            "level.economy"
        ]
    );
}

#[test]
fn band_text_is_readable() {
    for level in Level::ALL {
        let ratio = contrast(level.colour(), level.band_text());
        assert!(ratio >= 4.5, "{level:?}: {ratio}");
    }
}

#[test]
fn contrast_is_right() {
    let ratio = contrast(Color32::BLACK, Color32::WHITE);
    assert!((ratio - 21.0).abs() < 0.01, "{ratio}");
}

#[test]
fn no_text_is_below_4_5_on_the_panel() {
    for (name, colour) in [("ink", INK), ("error", ERROR), ("green", GREEN)] {
        let ratio = contrast(colour, PANEL);
        assert!(ratio >= 4.5, "{name}: {ratio}");
    }
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The level table and the theme's contrast.

use eframe::egui::Color32;

use crate::gui::levels::Level;
use crate::gui::theme::contrast;

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
    let got: Vec<[u8; 3]> = Level::ALL
        .iter()
        .map(|l| {
            let c = l.colour();
            [c.r(), c.g(), c.b()]
        })
        .collect();
    let want: [[u8; 3]; 10] = [
        [0x6b, 0x79, 0x72],
        [0xcc, 0x79, 0xa7],
        [0x00, 0x9e, 0x73],
        [0x00, 0x72, 0xb2],
        [0xe4, 0xe8, 0xec],
        [0x3a, 0x85, 0x5f],
        [0xb6, 0x5d, 0x47],
        [0x72, 0x77, 0x7e],
        [0x9d, 0x61, 0xa3],
        [0xb5, 0x8a, 0x1e],
    ];
    assert_eq!(got, want);
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

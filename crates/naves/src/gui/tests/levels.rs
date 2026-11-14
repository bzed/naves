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
        [0x5a, 0x66, 0x60],
        [0x8c, 0x6a, 0x45],
        [0xa3, 0x9a, 0x5c],
        [0x3f, 0x6f, 0x9e],
        [0x1d, 0x29, 0x24],
        [0x2e, 0x6a, 0x4c],
        [0x9a, 0x4f, 0x3c],
        [0x6b, 0x6f, 0x76],
        [0x7a, 0x4a, 0x7f],
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

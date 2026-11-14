// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The level patches under junctions without a junction piece.

use std::collections::BTreeSet;

use naves_core::junctions::{Junction, JunctionKind, Junctions};
use naves_export::generate::level_patches;

fn junction(kind: JunctionKind, at: (f64, f64), height: f64) -> Junction {
    Junction {
        at,
        kind,
        ground: height,
        height,
        rank: 4,
        patch_m: 5.0,
        stretches: BTreeSet::new(),
    }
}

#[test]
fn a_join_and_a_chain_end_on_a_road_get_a_patch_one_cell_wider_and_a_piece_gets_none() {
    let junctions = Junctions {
        points: vec![
            junction(JunctionKind::Join, (10.0, 20.0), 101.0),
            junction(JunctionKind::Piece, (30.0, 40.0), 102.0),
            junction(JunctionKind::EndOnRoad, (50.0, 60.0), 103.0),
        ],
        ..Junctions::default()
    };
    let patches = level_patches(&junctions, 3.125);
    assert_eq!(patches.len(), 2);
    assert_eq!(patches[0].at, (10.0, 20.0));
    assert!((patches[0].radius - 8.125).abs() < 1e-9);
    assert!((patches[0].height - 101.0).abs() < 1e-9);
    assert_eq!(patches[0].rank, 4);
    assert_eq!(patches[1].at, (50.0, 60.0));
}

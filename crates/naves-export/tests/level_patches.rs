// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The level patches under junctions without a junction piece.

use std::collections::BTreeSet;

use naves_core::junctions::{Junction, JunctionKind, Junctions, RingPlane};
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
        lower: BTreeSet::new(),
        gradient: (0.0, 0.0),
    }
}

#[test]
fn a_join_an_overlay_and_a_chain_end_on_a_road_get_a_patch_one_cell_wider_and_a_piece_gets_none() {
    let junctions = Junctions {
        points: vec![
            junction(JunctionKind::Join, (10.0, 20.0), 101.0),
            junction(JunctionKind::Piece, (30.0, 40.0), 102.0),
            junction(JunctionKind::EndOnRoad, (50.0, 60.0), 103.0),
            junction(JunctionKind::Overlay, (70.0, 80.0), 104.0),
        ],
        ..Junctions::default()
    };
    let patches = level_patches(&junctions, 3.125);
    assert_eq!(patches.len(), 3);
    assert_eq!(patches[0].at, (10.0, 20.0));
    assert!((patches[0].radius - 8.125).abs() < 1e-9);
    assert!((patches[0].height - 101.0).abs() < 1e-9);
    assert_eq!(patches[0].rank, 4);
    assert_eq!(patches[1].at, (50.0, 60.0));
    // an overlay is levelled like a join
    assert_eq!(patches[2].at, (70.0, 80.0));
    assert!((patches[2].height - 104.0).abs() < 1e-9);
}

#[test]
fn a_ring_gets_one_tilted_patch_and_the_junctions_on_it_none() {
    let ring_ids: BTreeSet<String> = ["1:100-112".to_string()].into();
    let mut on_ring = junction(JunctionKind::Overlay, (400.0, 430.0), 124.0);
    on_ring.stretches = ring_ids.clone();
    let junctions = Junctions {
        points: vec![on_ring, junction(JunctionKind::Join, (10.0, 20.0), 101.0)],
        rings: vec![RingPlane {
            at: (400.0, 400.0),
            radius: 33.5,
            inner_radius: 26.5,
            height: 123.2,
            gradient: (0.04, 0.02),
            rank: 3,
            stretches: ring_ids,
        }],
        ..Junctions::default()
    };
    let patches = level_patches(&junctions, 3.125);
    assert_eq!(patches.len(), 2);
    assert_eq!(patches[0].at, (10.0, 20.0));
    assert!((patches[0].gradient.0).abs() < 1e-12 && (patches[0].gradient.1).abs() < 1e-12);
    assert_eq!(patches[1].at, (400.0, 400.0));
    assert!((patches[1].radius - 36.625).abs() < 1e-9);
    // the band only: the inside of the ring is not levelled
    assert!((patches[1].inner_radius - 23.375).abs() < 1e-9);
    assert!((patches[1].height - 123.2).abs() < 1e-9);
    assert!((patches[1].gradient.0 - 0.04).abs() < 1e-12);
    assert!((patches[1].gradient.1 - 0.02).abs() < 1e-12);
    assert_eq!(patches[1].rank, 3);
}

#[test]
fn a_tilted_junction_patch_shapes_a_plane() {
    use naves_core::conform::conform;
    use naves_core::heightmap::Heightmap;
    // a 10 % slope to the east; an end-on-road junction tilted with it
    let size = 64_u32;
    let cell = 3.125_f32;
    let ground = |x: f64| 100.0 + 0.10 * x;
    let base = Heightmap {
        size,
        cell_m: cell,
        h: (0..size * size)
            .map(|i| {
                #[expect(clippy::cast_possible_truncation, reason = "a test height")]
                let h = ground(f64::from(i % size) * f64::from(cell)) as f32;
                h
            })
            .collect(),
    };
    let mut j = junction(JunctionKind::EndOnRoad, (100.0, 100.0), ground(100.0));
    j.gradient = (0.10, 0.0);
    let junctions = Junctions {
        points: vec![j],
        ..Junctions::default()
    };
    let patches = level_patches(&junctions, f64::from(cell));
    assert_eq!(patches.len(), 1);
    assert!((patches[0].gradient.0 - 0.10).abs() < 1e-12);
    let out = conform(&base, &[], &patches).unwrap();
    let radius = patches[0].radius;
    let mut checked = 0;
    for row in 0..size {
        for col in 0..size {
            let (x, y) = (
                f64::from(col) * f64::from(cell),
                f64::from(row) * f64::from(cell),
            );
            if (x - 100.0).hypot(y - 100.0) <= radius {
                let i = usize::try_from(row * size + col).unwrap();
                let (got, want) = (f64::from(out.heightmap.h[i]), ground(x));
                assert!((got - want).abs() < 0.02, "({x}, {y}): {got} vs {want}");
                checked += 1;
            }
        }
    }
    assert!(checked > 20);
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The side slope of the original ground at the roads.

#![expect(
    clippy::cast_possible_truncation,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::catalog::Family;
use naves_core::heightmap::Heightmap;
use naves_core::roadfit::Pose;
use naves_core::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad, Slot};
use naves_core::sideslope::side_slopes;

fn terrain(size: u32, cell: f32, f: impl Fn(f64, f64) -> f64) -> Heightmap {
    let mut h = Vec::new();
    for row in 0..size {
        for col in 0..size {
            h.push(f(
                f64::from(col) * f64::from(cell),
                f64::from(row) * f64::from(cell),
            ) as f32);
        }
    }
    Heightmap {
        size,
        cell_m: cell,
        h,
    }
}

/// One chain of ten 10 m pieces from `a` to `b`.
fn chain(a: (f64, f64), b: (f64, f64)) -> PlacedNetwork {
    let mut net = PlacedNetwork::default();
    for i in 0..10_u32 {
        let (t0, t1) = (f64::from(i) / 10.0, f64::from(i + 1) / 10.0);
        let p = |t: f64| (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
        let (p0, p1) = (p(t0), p(t1));
        let pose = |q: (f64, f64)| Pose {
            x: q.0,
            y: q.1,
            heading: 0.0,
        };
        net.roads.push(PlacedRoad {
            slot: Slot {
                stretch: "a".to_string(),
                index: i,
            },
            kind: PlacedKind::Piece,
            family: Family::Grav,
            drivable: true,
            model: "m".to_string(),
            origin: pose(p0),
            start: pose(p0),
            end: pose(p1),
            width: 6.0,
            scale: 1.0,
        });
    }
    net.chains.push((0..10).collect());
    net
}

#[test]
fn a_road_across_the_slope_has_its_slope_and_one_along_it_has_none() {
    let base = terrain(128, 3.125, |_, y| 100.0 + 0.4 * y);
    let across = side_slopes(&chain((100.0, 200.0), (200.0, 200.0)), &base);
    let along = side_slopes(&chain((200.0, 100.0), (200.0, 200.0)), &base);
    assert_eq!(across.len(), 10);
    for v in across {
        assert!((v - 0.4).abs() < 0.01, "{v}");
    }
    for v in along {
        assert!(v.abs() < 0.01, "{v}");
    }
}

#[test]
fn one_noisy_vertex_does_not_move_the_median() {
    let flat = terrain(128, 3.125, |_, y| 100.0 + 0.4 * y);
    let mut spiked = flat.clone();
    // the vertex at (143.75, 209.375) is one of the four under the northern sample (145, 210) of the
    // fifth piece
    spiked.h[67 * 128 + 46] += 30.0;
    let net = chain((100.0, 200.0), (200.0, 200.0));
    let before = side_slopes(&net, &flat);
    let after = side_slopes(&net, &spiked);
    for (b, a) in before.iter().zip(&after) {
        assert!((b - a).abs() < 0.05, "{b} {a}");
    }
}

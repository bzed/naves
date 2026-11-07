// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Height profiles of the road chains.

#![expect(
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::catalog::Family;
use naves_core::conform::FILL_SLOPE;
use naves_core::heightmap::Heightmap;
use naves_core::profile::{Profiles, max_grade, max_grade_change, solve_profiles};
use naves_core::roadfit::Pose;
use naves_core::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad, Slot};
use proptest::prelude::*;

/// A terrain of `size` vertices, `cell` metres apart, with height `f(x, y)`.
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

fn piece(
    stretch: &str,
    index: u32,
    kind: PlacedKind,
    family: Family,
    a: (f64, f64),
    b: (f64, f64),
) -> PlacedRoad {
    let heading = (b.0 - a.0).atan2(b.1 - a.1);
    PlacedRoad {
        slot: Slot {
            stretch: stretch.to_string(),
            index,
        },
        kind,
        family,
        drivable: true,
        model: "m".to_string(),
        origin: Pose {
            x: a.0,
            y: a.1,
            heading,
        },
        start: Pose {
            x: a.0,
            y: a.1,
            heading,
        },
        end: Pose {
            x: b.0,
            y: b.1,
            heading,
        },
        width: 7.0,
    }
}

/// A chain of 25 m pieces along the straight line from `a` to `b`.
fn straight(stretch: &str, family: Family, a: (f64, f64), b: (f64, f64)) -> Vec<PlacedRoad> {
    let length = (b.0 - a.0).hypot(b.1 - a.1);
    let n = (length / 25.0).ceil() as u32;
    (0..n)
        .map(|i| {
            let (t0, t1) = (f64::from(i) / f64::from(n), f64::from(i + 1) / f64::from(n));
            let p = |t: f64| (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
            piece(stretch, i, PlacedKind::Piece, family, p(t0), p(t1))
        })
        .collect()
}

fn network(chains: Vec<Vec<PlacedRoad>>) -> PlacedNetwork {
    let mut net = PlacedNetwork::default();
    for chain in chains {
        let base = net.roads.len();
        net.chains.push((base..base + chain.len()).collect());
        net.roads.extend(chain);
    }
    net
}

fn grades(p: &Profiles, strip: usize) -> Vec<f64> {
    let j = &p.strips[strip].joints;
    j.windows(2)
        .map(|w| {
            let d = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
            if d < 1e-9 { 0.0 } else { (w[1].2 - w[0].2) / d }
        })
        .collect()
}

#[test]
fn a_flat_road_stays_on_the_terrain() {
    let hm = terrain(100, 6.25, |_, _| 100.0);
    let p = solve_profiles(
        &network(vec![straight(
            "a",
            Family::Asf3,
            (100.0, 50.0),
            (100.0, 500.0),
        )]),
        &hm,
    );
    assert_eq!(p.strips.len(), 1);
    assert!(p.excesses.is_empty());
    for j in &p.strips[0].joints {
        assert!((j.2 - 100.0).abs() < 1e-6, "{j:?}");
    }
    assert!((p.strips[0].half_width - 3.5).abs() < 1e-9);
}

#[test]
fn a_gentle_slope_is_followed() {
    let hm = terrain(100, 6.25, |_, y| 100.0 + 0.05 * y);
    let p = solve_profiles(
        &network(vec![straight(
            "a",
            Family::Asf3,
            (100.0, 50.0),
            (100.0, 500.0),
        )]),
        &hm,
    );
    for j in &p.strips[0].joints {
        assert!((j.2 - (100.0 + 0.05 * j.1)).abs() < 0.3, "{j:?}");
    }
}

proptest! {
    #[test]
    fn grade_limit_holds(seed in any::<u64>(), family_index in 0usize..6) {
        let family = Family::ALL[family_index];
        let mut state = seed | 1;
        let mut noise = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % 10_000) as f64 / 10_000.0
        };
        let bumps: Vec<(f64, f64, f64)> = (0..12).map(|_| (noise() * 600.0, noise() * 600.0, noise() * 40.0)).collect();
        let hm = terrain(100, 6.25, |x, y| {
            bumps.iter().map(|(bx, by, a)| a * (-((x - bx).powi(2) + (y - by).powi(2)) / 4000.0).exp()).sum()
        });
        let net = network(vec![straight("a", family, (50.0, 50.0), (550.0, 550.0))]);
        let p = solve_profiles(&net, &hm);
        for g in grades(&p, 0) {
            prop_assert!(g.abs() <= max_grade(family) + 1e-6, "grade {g}");
        }
        let g = grades(&p, 0);
        for w in g.windows(2) {
            prop_assert!((w[1] - w[0]).abs() <= max_grade_change(family) + 1e-6, "change {}", w[1] - w[0]);
        }
        prop_assert!(p.excesses.is_empty());
    }
}

#[test]
fn a_steep_ramp_between_fixed_ends_reports_an_excess() {
    // the through road is level at 100 m; the side road ends at a junction 60 m higher
    let hm = terrain(100, 6.25, |_, y| if y < 300.0 { 100.0 } else { 160.0 });
    let through = straight("t", Family::Asf3, (100.0, 100.0), (100.0, 250.0));
    let net = network(vec![through]);
    let p = solve_profiles(&net, &hm);
    assert!(p.excesses.is_empty());
    // a chain whose two ends both lie on placed roads at very different heights
    let hm = terrain(100, 6.25, |_, y| if y < 300.0 { 100.0 } else { 160.0 });
    let low = straight("l", Family::Asf3, (100.0, 100.0), (100.0, 280.0));
    let high = straight("h", Family::Asf3, (100.0, 320.0), (100.0, 480.0));
    let bridge_between = straight("s", Family::Mud, (100.0, 280.0), (100.0, 320.0));
    let p = solve_profiles(&network(vec![low, high, bridge_between]), &hm);
    assert!(!p.excesses.is_empty(), "60 m in 40 m cannot be a road");
    let worst = p.excesses.iter().map(|e| e.grade.abs()).fold(0.0, f64::max);
    assert!(worst > max_grade(Family::Mud));
}

#[test]
fn side_road_starts_at_the_through_road_height() {
    let hm = terrain(100, 6.25, |_, y| 100.0 + 0.1 * y);
    let through = straight("t", Family::Asf3, (50.0, 300.0), (550.0, 300.0));
    let side = straight("s", Family::Asf3, (300.0, 300.0), (300.0, 500.0));
    let p = solve_profiles(&network(vec![through, side]), &hm);
    let at_crossing = p.strips[0]
        .joints
        .iter()
        .min_by(|a, b| {
            ((a.0 - 300.0).hypot(a.1 - 300.0)).total_cmp(&(b.0 - 300.0).hypot(b.1 - 300.0))
        })
        .unwrap()
        .2;
    let start = p.strips[1].joints[0].2;
    assert!(
        (start - at_crossing).abs() < 0.2,
        "side {start}, through {at_crossing}"
    );
}

#[test]
fn a_junction_is_level_along_its_length() {
    let hm = terrain(100, 6.25, |_, y| 100.0 + 0.1 * y);
    let mut chain = straight("a", Family::Asf3, (100.0, 100.0), (100.0, 200.0));
    chain.push(piece(
        "a|b",
        0,
        PlacedKind::Junction,
        Family::Asf3,
        (100.0, 200.0),
        (100.0, 212.5),
    ));
    chain.extend(straight("b", Family::Asf3, (100.0, 212.5), (100.0, 400.0)));
    let junction_index = 4;
    let net = network(vec![chain]);
    let p = solve_profiles(&net, &hm);
    // the joint at the start of the junction and the one at its end have one height
    let j = &p.strips[0].joints;
    let start = j.iter().find(|j| (j.1 - 200.0).abs() < 1e-6).unwrap();
    let end = j.iter().find(|j| (j.1 - 212.5).abs() < 1e-6).unwrap();
    assert!((start.2 - end.2).abs() < 1e-9, "{start:?} {end:?}");
    let _ = junction_index;
}

#[test]
fn switchback_legs_leave_room_for_a_slope() {
    // two legs 30 m apart on a 35 degree flank
    let slope = 35.0_f64.to_radians().tan();
    let hm = terrain(100, 12.5, |x, _| 400.0 - slope * x);
    let a = straight("a", Family::Asf3, (400.0, 100.0), (400.0, 600.0));
    let b = straight("b", Family::Asf3, (430.0, 100.0), (430.0, 600.0));
    let p = solve_profiles(&network(vec![a, b]), &hm);
    let height = |strip: usize, y: f64| {
        p.strips[strip]
            .joints
            .iter()
            .min_by(|u, v| (u.1 - y).abs().total_cmp(&(v.1 - y).abs()))
            .unwrap()
            .2
    };
    let diff = (height(0, 350.0) - height(1, 350.0)).abs();
    assert!(
        diff <= 30.0 * FILL_SLOPE + 0.5,
        "{diff} m between legs 30 m apart"
    );
}

#[test]
fn the_profile_is_deterministic() {
    let hm = terrain(100, 6.25, |x, y| {
        20.0 * (x / 90.0).sin() + 10.0 * (y / 70.0).cos()
    });
    let net = network(vec![straight(
        "a",
        Family::City,
        (60.0, 60.0),
        (560.0, 500.0),
    )]);
    let a = solve_profiles(&net, &hm);
    let b = solve_profiles(&net, &hm);
    assert_eq!(a, b);
}

#[test]
fn the_limits_get_looser_with_the_rank_of_the_road() {
    let order = [
        Family::Asf1,
        Family::Asf2,
        Family::Asf3,
        Family::City,
        Family::Grav,
        Family::Mud,
    ];
    for pair in order.windows(2) {
        assert!(max_grade(pair[0]) <= max_grade(pair[1]));
        assert!(max_grade_change(pair[0]) <= max_grade_change(pair[1]));
    }
    assert!((max_grade(Family::Asf3) - 0.12).abs() < 1e-12);
    assert!((max_grade_change(Family::Mud) - 0.08).abs() < 1e-12);
}

#[test]
fn the_road_meets_the_bridge_at_deck_height() {
    let hm = terrain(100, 6.25, |_, y| {
        100.0 + 0.04 * y
            - if (200.0..320.0).contains(&y) {
                30.0
            } else {
                0.0
            }
    });
    let mut chain = straight("a", Family::Asf3, (100.0, 100.0), (100.0, 200.0));
    for i in 0..4 {
        let (y0, y1) = (200.0 + 30.0 * f64::from(i), 230.0 + 30.0 * f64::from(i));
        chain.push(piece(
            "b",
            i,
            PlacedKind::Bridge,
            Family::Asf3,
            (100.0, y0),
            (100.0, y1),
        ));
    }
    chain.extend(straight("c", Family::Asf3, (100.0, 320.0), (100.0, 500.0)));
    let p = solve_profiles(&network(vec![chain]), &hm);
    let height_at = |y: f64| {
        p.strips[0]
            .joints
            .iter()
            .find(|j| (j.1 - y).abs() < 1e-6)
            .unwrap()
            .2
    };
    // the deck is level from its first joint to its last
    for y in [230.0, 260.0, 290.0, 320.0] {
        assert!(
            (height_at(y) - height_at(200.0)).abs() < 1e-9,
            "joint at {y}"
        );
    }
    // and the road on both sides meets it, with the grade limit kept
    assert!((height_at(175.0) - height_at(200.0)).abs() < max_grade(Family::Asf3) * 25.0 + 1e-6);
    assert!(p.strips[0].bridge.iter().filter(|b| **b).count() == 4);
    assert!(p.excesses.is_empty(), "{:?}", p.excesses);
}

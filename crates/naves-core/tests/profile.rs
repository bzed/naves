// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Height profiles of the road chains.

#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::catalog::Family;
use naves_core::catalog::testing::synthetic_catalog;
use naves_core::conform::{FILL_SLOPE_MAX, conform};
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::heightmap::Heightmap;
use naves_core::junctions::{HOLD_M, Junction, JunctionKind, Junctions, find_junctions};
use naves_core::limits::{drive_grade, grade_aim, grade_ceiling, min_vertical_radius};
use naves_core::profile::{MEETING_GRADE, Profiles, RELAX_OFF_M, solve_profiles};
use naves_core::roadfit::Pose;
use naves_core::roadnet::{RoadGraph, build_graph};
use naves_core::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad, Slot, place_network};
use naves_core::sideslope::side_slopes;
use proptest::prelude::*;
use std::collections::BTreeMap;

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
        scale: 1.0,
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

/// Pieces of `length` metres along the straight line from `a` to `b`.
fn pieces_of(family: Family, a: (f64, f64), b: (f64, f64), length: f64) -> Vec<PlacedRoad> {
    let total = (b.0 - a.0).hypot(b.1 - a.1);
    let n = (total / length).ceil() as u32;
    (0..n)
        .map(|i| {
            let (t0, t1) = (f64::from(i) / f64::from(n), f64::from(i + 1) / f64::from(n));
            let p = |t: f64| (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
            piece("a", i, PlacedKind::Piece, family, p(t0), p(t1))
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

/// The profiles with the junctions that `find_junctions` finds (chain ends on roads; no graph).
fn solve(net: &PlacedNetwork, hm: &Heightmap) -> Profiles {
    solve_profiles(
        net,
        hm,
        &find_junctions(net, &RoadGraph::default(), hm),
        &side_slopes(net, hm),
    )
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

fn ramp_terrain(grade: f64) -> Heightmap {
    terrain(200, 6.25, move |_, y| 100.0 + grade * y)
}

/// A straight, one arc piece with tangent poses, and a straight after it.
fn bend_on_a_ramp(family: Family, radius: f64, turn_deg: f64, _grade: f64) -> Vec<PlacedRoad> {
    let turn = turn_deg.to_radians();
    let start = (300.0, 300.0);
    let end = (
        start.0 + radius * (1.0 - turn.cos()),
        start.1 + radius * turn.sin(),
    );
    let mut roads = straight("a", family, (300.0, 250.0), start);
    let mut arc = piece("b", 0, PlacedKind::Piece, family, start, end);
    arc.origin.heading = 0.0;
    arc.start.heading = 0.0;
    arc.end.heading = turn;
    roads.push(arc);
    let after = (end.0 + 50.0 * turn.sin(), end.1 + 50.0 * turn.cos());
    roads.extend(straight("c", family, end, after));
    roads
}

/// The grade of the arc's segment: the two 25 m pieces of the straight come first.
fn arc_grade(p: &Profiles) -> f64 {
    grades(p, 0)[2]
}

/// One chain: up the west leg, a link at the top, down the east leg.
fn hairpin_legs(family: Family, west: f64, east: f64, from: f64, to: f64) -> Vec<PlacedRoad> {
    let mut roads = straight("a", family, (west, from), (west, to));
    roads.push(piece(
        "a",
        0,
        PlacedKind::Piece,
        family,
        (west, to),
        (east, to),
    ));
    roads.extend(straight("a", family, (east, to), (east, from)));
    roads
}

/// The height of the chain at the joint nearest `(x, y)`.
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a missing joint fails loudly"
)]
fn leg_height(p: &Profiles, x: f64, y: f64) -> f64 {
    p.strips[0]
        .joints
        .iter()
        .min_by(|a, b| ((a.0 - x).hypot(a.1 - y)).total_cmp(&((b.0 - x).hypot(b.1 - y))))
        .unwrap()
        .2
}

#[test]
fn a_flat_road_stays_on_the_terrain() {
    let hm = terrain(100, 6.25, |_, _| 100.0);
    let p = solve(
        &network(vec![straight(
            "a",
            Family::Asf3,
            (100.0, 50.0),
            (100.0, 500.0),
        )]),
        &hm,
    );
    assert_eq!(p.strips.len(), 1);
    assert_eq!(p.excesses.len(), 0);
    for j in &p.strips[0].joints {
        assert!((j.2 - 100.0).abs() < 1e-6, "{j:?}");
    }
    assert!((p.strips[0].half_width - 3.5).abs() < 1e-9);
}

#[test]
fn a_gentle_slope_is_followed() {
    let hm = terrain(100, 6.25, |_, y| 100.0 + 0.05 * y);
    let p = solve(
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

#[test]
fn short_pieces_bend_no_more_per_metre_than_long_ones() {
    // the same crest once in 25 m pieces and once in 6.25 m pieces
    let hm = terrain(200, 6.25, |_, y| {
        100.0 - 0.004 * (y - 600.0).powi(2).min(40_000.0)
    });
    for length in [25.0, 6.25] {
        let p = solve(
            &network(vec![pieces_of(
                Family::Asf3,
                (300.0, 200.0),
                (300.0, 1000.0),
                length,
            )]),
            &hm,
        );
        let g = grades(&p, 0);
        let radius = min_vertical_radius(Family::Asf3);
        for pair in g.windows(2) {
            assert!(
                (pair[1] - pair[0]).abs() <= length / radius + 1e-6,
                "{length}: {pair:?}"
            );
        }
    }
}

#[test]
fn the_piece_before_a_bridge_keeps_the_grade_limit() {
    // a 20 % ramp down to a 25 m bridge: the last piece before the deck is not steeper than the ceiling
    let hm = terrain(200, 6.25, |_, y| 100.0 + 0.20 * (y - 500.0).max(0.0));
    let mut roads = straight("up", Family::Asf3, (300.0, 900.0), (300.0, 525.0));
    roads.push(piece(
        "deck",
        0,
        PlacedKind::Bridge,
        Family::Asf3,
        (300.0, 525.0),
        (300.0, 500.0),
    ));
    roads.extend(straight("on", Family::Asf3, (300.0, 500.0), (300.0, 300.0)));
    let p = solve(&network(vec![roads]), &hm);
    assert!(
        grades(&p, 0)
            .iter()
            .all(|g| g.abs() <= grade_ceiling(Family::Asf3) + 1e-6)
    );
    assert!(p.excesses.is_empty(), "{:?}", p.excesses);
}

#[test]
fn overlapping_pieces_do_not_break_the_solver() {
    // two straights that overlap by all but 1 cm: the distance between two joints is nearly zero
    let hm = terrain(100, 6.25, |_, y| 100.0 + 0.05 * y);
    let roads = vec![
        piece(
            "a",
            0,
            PlacedKind::Piece,
            Family::Asf3,
            (300.0, 100.0),
            (300.0, 125.0),
        ),
        piece(
            "a",
            1,
            PlacedKind::Piece,
            Family::Asf3,
            (300.0, 100.01),
            (300.0, 125.01),
        ),
        piece(
            "a",
            2,
            PlacedKind::Piece,
            Family::Asf3,
            (300.0, 125.01),
            (300.0, 150.01),
        ),
    ];
    let p = solve(&network(vec![roads]), &hm);
    assert!(
        p.strips[0]
            .joints
            .iter()
            .all(|j| j.2.is_finite() && (j.2 - (100.0 + 0.05 * j.1)).abs() < 1.0)
    );
}

#[test]
fn a_short_road_between_two_others_is_all_fixed_and_finite() {
    // two parallel roads 20 m apart and a 20 m link between them
    let hm = terrain(100, 6.25, |x, y| 100.0 + 0.05 * x + 0.02 * y);
    let west = straight("w", Family::Asf3, (300.0, 100.0), (300.0, 500.0));
    let east = straight("e", Family::Asf3, (320.0, 100.0), (320.0, 500.0));
    let link = vec![
        piece(
            "l",
            0,
            PlacedKind::Piece,
            Family::Mud,
            (303.5, 300.0),
            (310.0, 300.0),
        ),
        piece(
            "l",
            1,
            PlacedKind::Piece,
            Family::Mud,
            (310.0, 300.0),
            (316.5, 300.0),
        ),
    ];
    let p = solve(&network(vec![west, east, link]), &hm);
    let (w, e) = (profile_height(&p, 0, 300.0), profile_height(&p, 1, 300.0));
    for joint in &p.strips[2].joints {
        assert!(joint.2.is_finite());
        assert!(
            joint.2 >= w.min(e) - 1e-6 && joint.2 <= w.max(e) + 1e-6,
            "{joint:?}"
        );
    }
}

#[test]
fn a_road_on_ground_steeper_than_its_aim_gives_way() {
    let hm = terrain(200, 6.25, |_, y| 100.0 + 0.20 * y);
    let p = solve(
        &network(vec![straight(
            "a",
            Family::Asf3,
            (300.0, 200.0),
            (300.0, 800.0),
        )]),
        &hm,
    );
    let steepest = grades(&p, 0).iter().fold(0.0_f64, |m, g| m.max(g.abs()));
    assert!(
        steepest > grade_aim(Family::Asf3) && steepest <= grade_ceiling(Family::Asf3) + 1e-6,
        "{steepest}"
    );
    for joint in &p.strips[0].joints {
        assert!(
            (joint.2 - (100.0 + 0.20 * joint.1)).abs() <= RELAX_OFF_M + 0.5,
            "{joint:?}"
        );
    }
}

#[test]
fn a_road_that_stays_near_the_ground_keeps_its_aim() {
    // 14 % for 100 m in the middle of level ground: 2 m off at most with the aim of 12 %
    let hm = terrain(200, 6.25, |_, y| {
        100.0 + 0.14 * (y - 450.0).clamp(0.0, 100.0)
    });
    let p = solve(
        &network(vec![straight(
            "a",
            Family::Asf3,
            (300.0, 200.0),
            (300.0, 800.0),
        )]),
        &hm,
    );
    assert!(
        grades(&p, 0)
            .iter()
            .all(|g| g.abs() <= grade_aim(Family::Asf3) + 1e-6)
    );
}

#[test]
fn giving_way_is_deterministic_and_leaves_earlier_roads_alone() {
    let hm = terrain(200, 6.25, |x, y| 100.0 + 0.20 * y + 0.01 * x);
    let first = straight("a", Family::Asf2, (200.0, 200.0), (800.0, 200.0));
    let steep = straight("b", Family::Asf3, (500.0, 300.0), (500.0, 900.0));
    let alone = solve(&network(vec![first.clone()]), &hm);
    let both = solve(&network(vec![first.clone(), steep.clone()]), &hm);
    assert_eq!(alone.strips[0], both.strips[0]);
    assert_eq!(both, solve(&network(vec![first, steep]), &hm));
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
        let p = solve(&net, &hm);
        for g in grades(&p, 0) {
            prop_assert!(g.abs() <= grade_ceiling(family) + 1e-6, "grade {g}");
        }
        let g = grades(&p, 0);
        let lengths: Vec<f64> = p.strips[0]
            .joints
            .windows(2)
            .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
            .collect();
        for (i, w) in g.windows(2).enumerate() {
            let run = f64::midpoint(lengths[i], lengths[i + 1]);
            prop_assert!(
                (w[1] - w[0]).abs() <= run / min_vertical_radius(family) + 1e-6,
                "change {} over {run} m",
                w[1] - w[0]
            );
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
    let p = solve(&net, &hm);
    assert_eq!(p.excesses.len(), 0);
    // a chain whose two ends both lie on placed roads at very different heights
    let hm = terrain(100, 6.25, |_, y| if y < 300.0 { 100.0 } else { 160.0 });
    let low = straight("l", Family::Asf3, (100.0, 100.0), (100.0, 280.0));
    let high = straight("h", Family::Asf3, (100.0, 320.0), (100.0, 480.0));
    let bridge_between = straight("s", Family::Mud, (100.0, 280.0), (100.0, 320.0));
    // `find_junctions` would repair the two heights toward each other, so the junctions are given as they are:
    // 100 m and 160 m at the two ends of the 40 m road between
    let junction = |y: f64, height: f64| Junction {
        at: (100.0, y),
        kind: JunctionKind::EndOnRoad,
        ground: height,
        height,
        rank: 3,
        patch_m: 3.5,
        stretches: std::collections::BTreeSet::new(),
    };
    let junctions = Junctions {
        points: vec![junction(280.0, 100.0), junction(320.0, 160.0)],
        ..Junctions::default()
    };
    let p = solve_profiles(
        &network(vec![low, high, bridge_between]),
        &hm,
        &junctions,
        &[],
    );
    assert!(!p.excesses.is_empty(), "60 m in 40 m cannot be a road");
    let worst = p.excesses.iter().map(|e| e.grade.abs()).fold(0.0, f64::max);
    assert!(worst > grade_aim(Family::Mud));
}

#[test]
fn side_road_starts_at_the_through_road_height() {
    let hm = terrain(100, 6.25, |_, y| 100.0 + 0.1 * y);
    let through = straight("t", Family::Asf3, (50.0, 300.0), (550.0, 300.0));
    let side = straight("s", Family::Asf3, (300.0, 300.0), (300.0, 500.0));
    let p = solve(&network(vec![through, side]), &hm);
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
fn a_junction_on_a_slope_carries_the_meeting_grade_at_most() {
    let hm = terrain(200, 6.25, |_, y| 100.0 + 0.10 * y);
    let mut roads = straight("a", Family::Asf3, (300.0, 200.0), (300.0, 500.0));
    roads.push(piece(
        "a|b",
        0,
        PlacedKind::Junction,
        Family::Asf3,
        (300.0, 500.0),
        (300.0, 512.5),
    ));
    roads.extend(straight("c", Family::Asf3, (300.0, 512.5), (300.0, 800.0)));
    let p = solve(&network(vec![roads]), &hm);
    let g = grades(&p, 0);
    let at_junction = g[12]; // 300 m of 25 m pieces come first
    assert!(
        at_junction > 0.03 && at_junction <= MEETING_GRADE + 1e-6,
        "{at_junction}"
    );
    assert!(g.iter().all(|v| v.abs() <= grade_aim(Family::Asf3) + 1e-6));
    assert!(p.excesses.is_empty(), "{:?}", p.excesses);
}

#[test]
fn a_through_road_is_gentle_where_a_side_road_ends_on_it() {
    // no junction piece: a track ends on the road's edge at y = 500
    let hm = terrain(200, 6.25, |_, y| 100.0 + 0.11 * y);
    let through = straight("a", Family::Asf3, (300.0, 200.0), (300.0, 800.0));
    let side = straight("s", Family::Mud, (303.5, 500.0), (500.0, 500.0));
    let p = solve(&network(vec![through, side]), &hm);
    for (i, pair) in p.strips[0].joints.windows(2).enumerate() {
        let near = pair[0].1 <= 512.5 && pair[1].1 >= 487.5;
        let grade = (pair[1].2 - pair[0].2) / 25.0;
        if near {
            assert!(grade.abs() <= MEETING_GRADE + 1e-6, "segment {i}: {grade}");
        }
    }
}

#[test]
fn a_tight_bend_is_gentler_than_a_straight() {
    // a 60 degree arc of radius 10 m in a 7 m road on a 12 % slope: at most 12 % * 6.5 / 10
    let p = solve(
        &network(vec![bend_on_a_ramp(Family::Asf3, 10.0, 60.0, 0.12)]),
        &ramp_terrain(0.12),
    );
    let on_the_arc = arc_grade(&p);
    assert!(on_the_arc <= 0.12 * 0.65 + 1e-6, "{on_the_arc}");
}

#[test]
fn an_arc_narrower_than_the_road_keeps_half_the_limit() {
    // radius 3 m, road 7 m wide (the helper's width): (r - w/2) / r is negative; the limit is half the aim
    let p = solve(
        &network(vec![bend_on_a_ramp(Family::Asf1, 3.0, 60.0, 0.08)]),
        &ramp_terrain(0.08),
    );
    let on_the_arc = arc_grade(&p);
    assert!(
        on_the_arc.is_finite() && on_the_arc > 0.0 && on_the_arc <= 0.04 + 1e-6,
        "{on_the_arc}"
    );
}

#[test]
fn switchback_legs_leave_room_for_a_slope() {
    // two legs 30 m apart on a 35 degree flank
    let slope = 35.0_f64.to_radians().tan();
    let hm = terrain(100, 6.25, |x, _| 400.0 - slope * x);
    let a = straight("a", Family::Asf3, (400.0, 100.0), (400.0, 600.0));
    let b = straight("b", Family::Asf3, (430.0, 100.0), (430.0, 600.0));
    let p = solve(&network(vec![a, b]), &hm);
    let height = |strip: usize, y: f64| {
        p.strips[strip]
            .joints
            .iter()
            .min_by(|u, v| (u.1 - y).abs().total_cmp(&(v.1 - y).abs()))
            .unwrap()
            .2
    };
    let diff = (height(0, 350.0) - height(1, 350.0)).abs();
    let gap = 3.5 + 3.5 + 3.0 * 6.25;
    let allowed = 0.1 + (30.0 - gap + 6.25) * FILL_SLOPE_MAX;
    assert!(
        diff <= allowed + 0.5,
        "{diff} m between legs 30 m apart, {allowed} allowed"
    );
}

#[test]
fn legs_whose_level_strips_do_not_touch_keep_their_heights() {
    // two legs on a 35 degree flank, 27 m apart; the helper's roads are 7 m wide, so their edges are 20 m
    // apart, 1.25 m more than three cells
    let slope = 35.0_f64.to_radians().tan();
    let hm = terrain(200, 6.25, |x, _| 400.0 - slope * (x - 300.0) * 0.3);
    let legs = hairpin_legs(Family::Mud, 300.0, 327.0, 200.0, 600.0);
    let p = solve(&network(vec![legs]), &hm);
    let apart = (leg_height(&p, 300.0, 400.0) - leg_height(&p, 327.0, 400.0)).abs();
    assert!(apart > 4.0, "{apart}");
    let out = conform(&hm, &p.strips, &[]).unwrap();
    assert!(out.conflicts.is_empty(), "{:?}", out.conflicts);
}

/// The height of a strip's profile at `y` (linear between joints).
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a missing joint fails loudly"
)]
fn profile_height(p: &Profiles, strip: usize, y: f64) -> f64 {
    let joints = &p.strips[strip].joints;
    let pair = joints
        .windows(2)
        .find(|w| (w[0].1 - y) * (w[1].1 - y) <= 0.0 && (w[0].1 - w[1].1).abs() > 1e-9)
        .unwrap();
    let t = (y - pair[0].1) / (pair[1].1 - pair[0].1);
    pair[0].2 + t * (pair[1].2 - pair[0].2)
}

#[test]
fn an_end_between_joints_takes_the_road_height_there() {
    // a side road ends on a through road that climbs: the height it takes is the through road's
    // height where they meet, not the height of the through road's nearest joint
    let hm = terrain(100, 6.25, |x, _| 100.0 + 0.10 * (x - 100.0));
    let through = straight("t", Family::Asf3, (100.0, 100.0), (550.0, 100.0));
    // ends 1.5 m inside the through road's edge, 5 m past its joint at x = 300
    let side = straight("s", Family::Asf3, (305.0, 250.0), (305.0, 101.5));
    let p = solve(&network(vec![through, side]), &hm);
    let end = p.strips[1].joints.last().copied().unwrap();
    let through_height = |x: f64| {
        let joints = &p.strips[0].joints;
        let pair = joints
            .windows(2)
            .find(|w| (w[0].0 - x) * (w[1].0 - x) <= 0.0 && (w[0].0 - w[1].0).abs() > 1e-9)
            .unwrap();
        let t = (x - pair[0].0) / (pair[1].0 - pair[0].0);
        pair[0].2 + t * (pair[1].2 - pair[0].2)
    };
    let wanted = through_height(305.0);
    assert!(
        (end.2 - wanted).abs() < 0.3,
        "side ends at {:.2}, the through road is at {wanted:.2} there",
        end.2
    );
}

#[test]
fn an_end_beside_a_road_keeps_its_own_height() {
    // a dead end 40 m from the centre line of another road does not meet it and does not share its
    // cells: it keeps the height of the terrain it is on, instead of taking the other road's height
    let hm = terrain(100, 6.25, |x, _| 100.0 + 0.10 * (x - 100.0));
    let through = straight("t", Family::Asf3, (100.0, 100.0), (550.0, 100.0));
    let side = straight("s", Family::Asf3, (306.0, 250.0), (306.0, 140.0));
    let p = solve(&network(vec![through, side]), &hm);
    let end = p.strips[1].joints.last().copied().unwrap();
    let wanted = 100.0 + 0.10 * (306.0 - 100.0);
    assert!(
        (end.2 - wanted).abs() < 0.35,
        "side ends at {:.2}, the terrain is at {wanted:.2} there",
        end.2
    );
}

#[test]
fn settling_keeps_the_grade_limits_of_a_road_between_two_others() {
    // a short road between a high one and a low one: settling may pull its ends, but the road
    // itself must keep its grade limit - the ends are free, nothing pins them
    let hm = terrain(200, 6.25, |_, y| {
        if y <= 82.0 {
            100.0
        } else if y >= 128.0 {
            50.0
        } else {
            100.0 - (y - 82.0) * (50.0 / 46.0)
        }
    });
    let high = straight("h", Family::Asf3, (100.0, 80.0), (500.0, 80.0));
    let low = straight("l", Family::Asf3, (100.0, 132.0), (500.0, 132.0));
    let between = vec![piece(
        "b",
        0,
        PlacedKind::Piece,
        Family::Asf3,
        (200.0, 95.0),
        (200.0, 115.0),
    )];
    let p = solve(&network(vec![high, low, between]), &hm);
    let grades = grades(&p, 2);
    let limit = grade_ceiling(Family::Asf3);
    assert!(
        grades.iter().all(|g| g.abs() <= limit + 1e-6),
        "the road between settled to grades {grades:?}, limit {limit}"
    );
}

#[test]
fn the_reported_excesses_are_the_settled_ones() {
    // what the profiles report as over the limits is measured on the heights that are actually
    // used, after settling - not on the heights the chains had before it
    let hm = terrain(200, 6.25, |_, y| {
        if y <= 82.0 {
            100.0
        } else if y >= 128.0 {
            50.0
        } else {
            100.0 - (y - 82.0) * (50.0 / 46.0)
        }
    });
    let high = straight("h", Family::Asf3, (100.0, 80.0), (500.0, 80.0));
    let low = straight("l", Family::Asf3, (100.0, 132.0), (500.0, 132.0));
    let between = vec![piece(
        "b",
        0,
        PlacedKind::Piece,
        Family::Asf3,
        (200.0, 95.0),
        (200.0, 115.0),
    )];
    let p = solve(&network(vec![high, low, between]), &hm);
    let family = Family::Asf3;
    let (grade, radius) = (grade_ceiling(family), min_vertical_radius(family));
    let mut shown = 0;
    for strip in &p.strips {
        let lengths: Vec<f64> = strip
            .joints
            .windows(2)
            .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1).max(0.5))
            .collect();
        let grades: Vec<f64> = strip
            .joints
            .windows(2)
            .zip(&lengths)
            .map(|(w, d)| (w[1].2 - w[0].2) / d)
            .collect();
        for (i, pair) in grades.windows(2).enumerate() {
            let run = f64::midpoint(lengths[i], lengths[i + 1]);
            if (pair[1] - pair[0]).abs() / run > 1.0 / radius + 1e-6 {
                shown += 1;
            }
        }
        for g in &grades {
            if g.abs() > grade + 1e-6 {
                shown += 1;
            }
        }
    }
    assert_eq!(
        shown,
        p.excesses.len(),
        "the strips show {shown} violations, the report has {}",
        p.excesses.len()
    );
}

#[test]
fn a_side_road_ends_at_the_junction_port_height() {
    // the side road ends at the port of a junction piece, which sits beyond the through road's
    // edge: it still takes the junction's height, not the height of the terrain it stands on
    let hm = terrain(200, 6.25, |_, y| if y <= 103.0 { 110.0 } else { 100.0 });
    let mut through = straight("t", Family::Asf3, (100.0, 100.0), (300.0, 100.0));
    through.push(piece(
        "t|s",
        0,
        PlacedKind::Junction,
        Family::Asf3,
        (300.0, 100.0),
        (312.5, 100.0),
    ));
    through.extend(straight("t", Family::Asf3, (312.5, 100.0), (550.0, 100.0)));
    // the port of the junction, 5 m from its centre, past the through road's 3.5 m edge
    let side = straight("s", Family::Asf3, (306.25, 130.0), (306.25, 105.0));
    // `find_junctions` finds a junction piece only where the graph has a crossing: here it is given
    let piece_junction = Junction {
        at: (306.25, 100.0),
        kind: JunctionKind::Piece,
        ground: 110.0,
        height: 110.0,
        rank: 3,
        patch_m: 6.25,
        stretches: std::collections::BTreeSet::new(),
    };
    let junctions = Junctions {
        points: vec![piece_junction],
        ..Junctions::default()
    };
    let p = solve_profiles(&network(vec![through, side]), &hm, &junctions, &[]);
    let end = p.strips[1].joints.last().copied().unwrap();
    assert!(
        end.2 > 108.5,
        "the side road ends at {:.2}, the junction is at 110",
        end.2
    );
    // and the climb onto the junction stays within the grade limit: the whole road tilts up
    let grades = grades(&p, 1);
    assert!(
        grades
            .iter()
            .all(|g| g.abs() <= grade_ceiling(Family::Asf3) + 1e-6),
        "the climb onto the junction breaks the limit: {grades:?}"
    );
}

#[test]
fn roads_closer_than_their_footprints_share_a_height() {
    // two roads 20 m apart on a 20 degree flank: on a 6.25 m terrain grid the cells under both overlap, so
    // they cannot be 7 m apart in height whatever the terrain does around them
    let slope = 20.0_f64.to_radians().tan();
    let hm = terrain(160, 6.25, |x, _| 400.0 - slope * x);
    let a = straight("a", Family::Asf3, (400.0, 100.0), (400.0, 600.0));
    let b = straight("b", Family::Asf3, (420.0, 100.0), (420.0, 600.0));
    let p = solve(&network(vec![a, b]), &hm);
    let diff = (profile_height(&p, 0, 350.0) - profile_height(&p, 1, 350.0)).abs();
    assert!(diff < 0.5, "{diff} m between roads 20 m apart");
}

#[test]
fn settling_keeps_a_road_near_the_terrain() {
    // two legs 30 m apart on a 35 degree flank cannot be brought together; they must not wander off
    let slope = 35.0_f64.to_radians().tan();
    let hm = terrain(160, 6.25, |x, _| 400.0 - slope * x);
    let a = straight("a", Family::Asf3, (400.0, 100.0), (400.0, 600.0));
    let b = straight("b", Family::Asf3, (430.0, 100.0), (430.0, 600.0));
    let p = solve(&network(vec![a, b]), &hm);
    for (strip, x) in [(0, 400.0), (1, 430.0)] {
        let ground = 400.0 - slope * x;
        let off = (profile_height(&p, strip, 350.0) - ground).abs();
        assert!(off <= 5.5, "road {strip} is {off} m off the terrain");
    }
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
    let a = solve(&net, &hm);
    let b = solve(&net, &hm);
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
        assert!(grade_aim(pair[0]) <= grade_aim(pair[1]));
        assert!(min_vertical_radius(pair[0]) >= min_vertical_radius(pair[1]));
    }
    assert!((grade_aim(Family::Asf3) - 0.12).abs() < 1e-12);
    assert!((min_vertical_radius(Family::Mud) - 60.0).abs() < 1e-12);
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
    let p = solve(&network(vec![chain]), &hm);
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
    assert!((height_at(175.0) - height_at(200.0)).abs() < grade_aim(Family::Asf3) * 25.0 + 1e-6);
    assert_eq!(p.strips[0].bridge.iter().filter(|b| **b).count(), 4);
    assert!(p.excesses.is_empty(), "{:?}", p.excesses);
}

/// A level road at 100 m over a gorge `depth` deep from y = 300 to y = 300 + `span`, on bridge pieces.
fn gorge_chain(depth: f64, span: f64) -> (Heightmap, Vec<PlacedRoad>) {
    let hm = terrain(200, 6.25, move |_, y| {
        100.0
            - if y > 300.0 && y < 300.0 + span {
                depth
            } else {
                0.0
            }
    });
    let mut chain = straight("a", Family::Asf3, (300.0, 100.0), (300.0, 300.0));
    let n = (span / 25.0).ceil() as u32;
    for i in 0..n {
        let (y0, y1) = (
            300.0 + span * f64::from(i) / f64::from(n),
            300.0 + span * f64::from(i + 1) / f64::from(n),
        );
        chain.push(piece(
            "b",
            i,
            PlacedKind::Bridge,
            Family::Asf3,
            (300.0, y0),
            (300.0, y1),
        ));
    }
    chain.extend(straight(
        "c",
        Family::Asf3,
        (300.0, 300.0 + span),
        (300.0, 600.0),
    ));
    (hm, chain)
}

#[test]
fn a_deck_over_a_gorge_stays_at_bank_height() {
    for (depth, span) in [(30.0, 100.0), (5.0, 25.0)] {
        let (hm, chain) = gorge_chain(depth, span);
        let p = solve(&network(vec![chain]), &hm);
        for joint in &p.strips[0].joints {
            assert!(
                (joint.2 - 100.0).abs() < 0.05,
                "depth {depth}, span {span}: joint at {} is {}",
                joint.1,
                joint.2
            );
        }
        assert!(p.excesses.is_empty(), "{:?}", p.excesses);
    }
}

#[test]
fn a_deck_whose_ends_lie_on_the_ground_is_solved_at_the_aim() {
    // a ramp at the aim grade with a short bridge over a hollow: the approaches stay on the ground
    let aim = grade_aim(Family::Asf3);
    let hm = terrain(200, 6.25, move |_, y| {
        100.0 + aim * y - if y > 300.0 && y < 325.0 { 6.0 } else { 0.0 }
    });
    let mut chain = straight("a", Family::Asf3, (300.0, 100.0), (300.0, 300.0));
    chain.push(piece(
        "b",
        0,
        PlacedKind::Bridge,
        Family::Asf3,
        (300.0, 300.0),
        (300.0, 325.0),
    ));
    chain.extend(straight("c", Family::Asf3, (300.0, 325.0), (300.0, 600.0)));
    let p = solve(&network(vec![chain]), &hm);
    let joint = |y: f64| {
        p.strips[0]
            .joints
            .iter()
            .find(|j| (j.1 - y).abs() < 1e-6)
            .unwrap()
    };
    // the ground at the abutments is the ground of the approaches
    let (a, b) = (joint(300.0), joint(325.0));
    assert!((a.2 - b.2).abs() < 1e-9);
    assert!((a.2 - (100.0 + aim * 300.0)).abs() < 0.5, "{}", a.2);
    for g in grades(&p, 0) {
        assert!(g.abs() <= aim + 1e-6, "grade {g} over the aim {aim}");
    }
}

fn feature(way: i64, family: Family, nodes: &[i64], points: &[(f64, f64)]) -> Feature {
    Feature {
        id: format!("osm/way/{way}/0"),
        way_id: way,
        source: Source::Osm,
        kind: FeatureKind::Road {
            family,
            drivable: true,
            bridge: false,
            tunnel: false,
        },
        points: points.to_vec(),
        nodes: nodes.to_vec(),
    }
}

/// Places `features` with the synthetic catalog, as the build does.
fn placed(features: &[Feature]) -> (PlacedNetwork, RoadGraph) {
    let graph = build_graph(features);
    let net = place_network(&graph, &synthetic_catalog()).unwrap();
    (net, graph)
}

/// An asf3 through road at y = 200 with a city side road going north from x = 200.
fn t_junction_network() -> (PlacedNetwork, RoadGraph) {
    placed(&[
        feature(
            1,
            Family::Asf3,
            &[1, 2, 3],
            &[(50.0, 200.0), (200.0, 200.0), (350.0, 200.0)],
        ),
        feature(2, Family::City, &[2, 4], &[(200.0, 200.0), (200.0, 350.0)]),
    ])
}

/// Three roads east-west and three north-south, 200 m apart, all crossing.
fn grid_of_roads() -> (PlacedNetwork, RoadGraph) {
    let mut features = Vec::new();
    for h in 0..3_i32 {
        let y = 200.0 + 200.0 * f64::from(h);
        let mut nodes = vec![i64::from(100 + h)];
        let mut points = vec![(50.0, y)];
        for v in 0..3_i32 {
            nodes.push(i64::from(10 + 3 * h + v));
            points.push((200.0 + 200.0 * f64::from(v), y));
        }
        nodes.push(i64::from(200 + h));
        points.push((750.0, y));
        features.push(feature(i64::from(1 + h), Family::Asf3, &nodes, &points));
    }
    for v in 0..3_i32 {
        let x = 200.0 + 200.0 * f64::from(v);
        let mut nodes = vec![i64::from(300 + v)];
        let mut points = vec![(x, 50.0)];
        for h in 0..3_i32 {
            nodes.push(i64::from(10 + 3 * h + v));
            points.push((x, 200.0 + 200.0 * f64::from(h)));
        }
        nodes.push(i64::from(400 + v));
        points.push((x, 750.0));
        features.push(feature(i64::from(10 + v), Family::City, &nodes, &points));
    }
    placed(&features)
}

/// The same network with the roads and the chains in the opposite order.
fn reordered(net: &PlacedNetwork) -> PlacedNetwork {
    let n = net.roads.len();
    let mut out = net.clone();
    out.roads.reverse();
    out.chains = net
        .chains
        .iter()
        .rev()
        .map(|chain| chain.iter().map(|i| n - 1 - i).collect())
        .collect();
    out.joins_without_junction.reverse();
    out
}

/// The heights at the start and the end of every road, by the road's stretch and index.
fn heights_by_slot(net: &PlacedNetwork, p: &Profiles) -> BTreeMap<(String, u32), (f64, f64)> {
    let mut out = BTreeMap::new();
    for strip in &p.strips {
        for (k, slot) in strip.slots.iter().enumerate() {
            let road = &net.roads[*slot];
            out.insert(
                (road.slot.stretch.clone(), road.slot.index),
                (strip.joints[k].2, strip.joints[k + 1].2),
            );
        }
    }
    out
}

/// Whether a point lies where the junction holds a road: in its patch, or at a port beyond it.
fn near(j: &Junction, p: (f64, f64)) -> bool {
    (j.at.0 - p.0).hypot(j.at.1 - p.1) <= j.patch_m + 1.0
}

#[test]
fn all_roads_at_a_junction_have_its_height() {
    let (net, graph) = t_junction_network();
    let hm = terrain(128, 3.125, |x, y| 100.0 + 0.08 * x + 0.03 * y);
    let j = find_junctions(&net, &graph, &hm);
    assert_eq!(j.points.len(), 1);
    let p = solve_profiles(&net, &hm, &j, &[]);
    let want = j.points[0].height;
    let mut seen = 0;
    for strip in &p.strips {
        for joint in strip
            .joints
            .iter()
            .filter(|q| near(&j.points[0], (q.0, q.1)))
        {
            seen += 1;
            assert!((joint.2 - want).abs() < 0.01, "{} vs {want}", joint.2);
        }
    }
    // the start of the side road and the through road's joint at the crossing (a join: the game has no
    // junction piece for this pair)
    assert!(seen >= 2, "{seen}");
}

/// A through road over a 6 m deep, 25 m wide dip that its vertical radius cannot follow, and a side road that
/// ends on it in the middle of the dip.
fn side_road_on_a_drifting_through_road() -> (PlacedNetwork, Heightmap) {
    let hm = terrain(128, 3.125, |x, _| {
        if (x - 300.0).abs() < 12.5 {
            94.0
        } else {
            100.0
        }
    });
    let through = straight("t", Family::Asf3, (100.0, 100.0), (500.0, 100.0));
    let side = straight("s", Family::Asf3, (300.0, 300.0), (300.0, 103.0));
    (network(vec![through, side]), hm)
}

#[test]
fn the_through_road_does_not_drag_the_side_road_off_the_ground() {
    let (net, hm) = side_road_on_a_drifting_through_road();
    let j = find_junctions(&net, &RoadGraph::default(), &hm);
    assert_eq!(j.points.len(), 1, "{j:?}");
    // without the junction the side road takes the through road's height, which is far off the ground
    let drifting = solve_profiles(&net, &hm, &Junctions::default(), &[]);
    let end = drifting.strips[1].joints.last().copied().unwrap();
    assert!(
        end.2 - f64::from(hm.sample(end.0, end.1).height) > 2.0,
        "the through road does not drift: {}",
        end.2
    );
    let p = solve_profiles(&net, &hm, &j, &[]);
    let side_end = p.strips[1].joints.last().copied().unwrap();
    assert!((side_end.2 - f64::from(hm.sample(side_end.0, side_end.1).height)).abs() < 1.0);
}

#[test]
fn the_order_of_the_chains_does_not_change_the_heights() {
    let (net, graph) = grid_of_roads();
    let hm = terrain(256, 3.125, |x, y| 100.0 + 0.2 * x - 0.1 * y);
    let j = find_junctions(&net, &graph, &hm);
    let a = solve_profiles(&net, &hm, &j, &[]);
    let other = reordered(&net);
    let b = solve_profiles(&other, &hm, &j, &[]);
    let (ha, hb) = (heights_by_slot(&net, &a), heights_by_slot(&other, &b));
    assert_eq!(ha.len(), hb.len());
    for (slot, (s0, e0)) in &ha {
        let (s1, e1) = hb[slot];
        assert!(
            (s0 - s1).abs() < 1e-9 && (e0 - e1).abs() < 1e-9,
            "{slot:?}: {s0} {e0} against {s1} {e1}"
        );
    }
}

#[test]
fn a_ring_meets_itself_at_one_height_and_a_lone_road_still_solves() {
    // a loop from node 1 round and back to node 1, and two roads leaving node 1
    let (ring, graph) = placed(&[
        feature(
            1,
            Family::Asf3,
            &[1, 2, 3, 1],
            &[
                (200.0, 200.0),
                (350.0, 200.0),
                (350.0, 320.0),
                (200.0, 200.0),
            ],
        ),
        feature(2, Family::Asf3, &[1, 4], &[(200.0, 200.0), (50.0, 200.0)]),
        feature(3, Family::Asf3, &[1, 5], &[(200.0, 200.0), (200.0, 50.0)]),
    ]);
    let hm = terrain(128, 3.125, |x, _| 100.0 + 0.1 * x);
    let j = find_junctions(&ring, &graph, &hm);
    let p = solve_profiles(&ring, &hm, &j, &[]);
    // the chain runs from the west road through the junction and round the ring back to it: it passes the
    // junction once on the way in and ends there
    let strip = p.strips.iter().find(|s| s.joints.len() > 20).unwrap();
    let last = strip.joints[strip.joints.len() - 1];
    assert!((last.0 - 200.0).hypot(last.1 - 200.0) < 4.0, "{last:?}");
    let through = strip
        .joints
        .iter()
        .find(|q| (q.0 - 200.0).hypot(q.1 - 200.0) < 1e-6)
        .unwrap();
    assert!(
        (through.2 - last.2).abs() < 0.01,
        "{} {}",
        through.2,
        last.2
    );
    assert!((last.2 - j.points[0].height).abs() < 0.01);
    let lone = network(vec![straight(
        "a",
        Family::Asf3,
        (100.0, 50.0),
        (100.0, 300.0),
    )]);
    assert_eq!(
        solve_profiles(&lone, &hm, &Junctions::default(), &[])
            .strips
            .len(),
        1
    );
}

#[test]
fn no_piece_carries_a_step() {
    // the Klingerweg in small: a mud track climbing 30 % between two roads, on the ground
    let hm = terrain(128, 3.125, |_, y| 100.0 + 0.30 * y);
    let track = straight("k", Family::Mud, (300.0, 100.0), (300.0, 325.0));
    let low = straight("l", Family::Asf3, (150.0, 100.0), (450.0, 100.0));
    let high = straight("h", Family::Asf3, (150.0, 325.0), (450.0, 325.0));
    let net = network(vec![low, high, track]);
    let j = find_junctions(&net, &RoadGraph::default(), &hm);
    assert_eq!(j.points.len(), 2, "{j:?}");
    let p = solve_profiles(&net, &hm, &j, &[]);
    for w in p.strips[2].joints.windows(2) {
        let run = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        assert!(((w[1].2 - w[0].2) / run).abs() <= 0.45 + 1e-6, "{w:?}");
    }
    // the track keeps its own grade limit: no meeting grade lifts its lower end out of the ground
    let first = p.strips[2].joints[1];
    let ground = f64::from(hm.sample(first.0, first.1).height);
    assert!((first.2 - ground).abs() < 3.0, "{} {ground}", first.2);
}

#[test]
fn a_through_road_is_fixed_at_the_junction_piece_in_its_middle() {
    // the through road's own limits would lift it off the dip; the piece takes the ground's height
    let hm = terrain(128, 3.125, |x, _| {
        if (x - 300.0).abs() < 12.5 {
            94.0
        } else {
            100.0
        }
    });
    let mut through = straight("t", Family::Asf3, (100.0, 100.0), (300.0, 100.0));
    through.push(piece(
        "t|s",
        0,
        PlacedKind::Junction,
        Family::Asf3,
        (300.0, 100.0),
        (312.5, 100.0),
    ));
    through.extend(straight("t", Family::Asf3, (312.5, 100.0), (500.0, 100.0)));
    let side = straight("s", Family::Asf3, (306.25, 300.0), (306.25, 105.0));
    let net = network(vec![through, side]);
    let j = find_junctions(&net, &RoadGraph::default(), &hm);
    let p = solve_profiles(&net, &hm, &j, &[]);
    let piece_joints: Vec<_> = p.strips[0]
        .joints
        .iter()
        .filter(|q| (q.0 - 306.25).abs() < 7.0)
        .collect();
    assert!(piece_joints.len() >= 2);
    let end = p.strips[1].joints.last().unwrap();
    for q in piece_joints.iter().copied().chain([end]) {
        assert!((q.2 - piece_joints[0].2).abs() < 1e-6, "{q:?}");
    }
    // and the patch lookup reaches the side road's port
    assert!(j.near((end.0, end.1), HOLD_M).is_some());
}

#[test]
fn a_span_between_two_junctions_at_its_ceiling_has_the_same_grade_everywhere() {
    // the ground is level; two junctions 400 m apart differ by exactly what the road may climb: the profile
    // has to be a straight ramp, not a ramp with one short steep piece where the smoothing lagged behind
    let ceiling = grade_ceiling(Family::Asf3);
    let hm = terrain(128, 6.25, |_, _| 100.0);
    let net = network(vec![straight(
        "a",
        Family::Asf3,
        (100.0, 100.0),
        (100.0, 500.0),
    )]);
    let junction = |y: f64, height: f64| Junction {
        at: (100.0, y),
        kind: JunctionKind::Join,
        ground: 100.0,
        height,
        rank: 3,
        patch_m: 3.5,
        stretches: std::collections::BTreeSet::new(),
    };
    let junctions = Junctions {
        points: vec![
            junction(100.0, 100.0),
            junction(500.0, 100.0 + ceiling * 400.0),
        ],
        ..Junctions::default()
    };
    let p = solve_profiles(&net, &hm, &junctions, &[]);
    let steepest = grades(&p, 0).iter().fold(0.0_f64, |m, g| m.max(g.abs()));
    assert!(
        steepest <= 1.1 * ceiling + 1e-6,
        "{steepest} against {ceiling}"
    );
    assert!(p.excesses.is_empty(), "{:?}", p.excesses);
}

#[test]
fn the_grade_is_kept_over_the_run_between_the_joints_not_the_length_of_the_piece() {
    // pieces of 25 m that each start 15 m before the end of the one before: the joints are 10 m apart. On a 50 %
    // slope the profile keeps its limit over those 10 m (it was kept over 25 m and the strip came out at 67 %)
    let hm = terrain(128, 6.25, |_, y| 100.0 + 0.5 * y);
    let roads: Vec<PlacedRoad> = (0..20)
        .map(|i| {
            let y = 100.0 + 10.0 * f64::from(i);
            piece(
                "a",
                u32::try_from(i).unwrap(),
                PlacedKind::Piece,
                Family::Asf3,
                (100.0, y - 15.0),
                (100.0, y + 10.0),
            )
        })
        .collect();
    let p = solve(&network(vec![roads]), &hm);
    let steepest = grades(&p, 0).iter().fold(0.0_f64, |m, g| m.max(g.abs()));
    assert!(steepest <= grade_ceiling(Family::Asf3) + 1e-3, "{steepest}");
}

#[test]
fn a_free_joint_between_two_junction_heights_is_not_held_to_the_meeting_grade() {
    // the same span, with a side road that ends beside its first piece: the span needs 98 % of the ceiling and the
    // 6 % of a meeting would make that impossible
    let ceiling = grade_ceiling(Family::Asf3);
    let hm = terrain(128, 6.25, |_, _| 100.0);
    let mut roads = vec![
        piece(
            "a",
            0,
            PlacedKind::Piece,
            Family::Asf3,
            (100.0, 100.0),
            (100.0, 112.5),
        ),
        piece(
            "a",
            1,
            PlacedKind::Piece,
            Family::Asf3,
            (100.0, 112.5),
            (100.0, 113.6),
        ),
    ];
    roads.extend(straight("a", Family::Asf3, (100.0, 113.6), (100.0, 300.0)));
    let side = straight("s", Family::Asf3, (103.5, 106.0), (200.0, 106.0));
    let net = network(vec![roads, side]);
    let junction = |y: f64, height: f64| Junction {
        at: (100.0, y),
        kind: JunctionKind::Join,
        ground: 100.0,
        height,
        rank: 3,
        patch_m: 0.5,
        stretches: std::collections::BTreeSet::new(),
    };
    let junctions = Junctions {
        points: vec![
            junction(100.0, 100.0),
            junction(113.6, 100.0 - 0.98 * ceiling * 13.6),
        ],
        ..Junctions::default()
    };
    let p = solve_profiles(&net, &hm, &junctions, &[]);
    for (i, g) in grades(&p, 0).iter().take(2).enumerate() {
        assert!(g.abs() <= ceiling + 1e-3, "piece {i}: {g}");
    }
}

/// A level road on ground that rises 7 m just before the held joint at its end: held joints A (100 m) at the start
/// and B (107 m) 4.4 m after the last long piece, and ground 107 m from y = 480.
fn free_span_then_a_short_piece_then_a_held_joint() -> (PlacedNetwork, Heightmap, Junctions) {
    let hm = terrain(128, 6.25, |_, y| if y < 300.0 { 114.0 } else { 107.0 });
    let mut roads = straight("a", Family::Asf3, (100.0, 100.0), (100.0, 500.0));
    roads.push(piece(
        "a",
        99,
        PlacedKind::Piece,
        Family::Asf3,
        (100.0, 500.0),
        (100.0, 504.4),
    ));
    let junction = |y: f64, height: f64| Junction {
        at: (100.0, y),
        kind: JunctionKind::Join,
        ground: height,
        height,
        rank: 3,
        patch_m: 0.5,
        stretches: std::collections::BTreeSet::new(),
    };
    let junctions = Junctions {
        points: vec![junction(100.0, 100.0), junction(504.4, 107.0)],
        ..Junctions::default()
    };
    (network(vec![roads]), hm, junctions)
}

#[test]
fn no_piece_between_two_held_joints_is_over_its_hard_limit() {
    let (net, hm, junctions) = free_span_then_a_short_piece_then_a_held_joint();
    let p = solve_profiles(&net, &hm, &junctions, &[]);
    for (i, g) in grades(&p, 0).iter().enumerate() {
        assert!(
            g.abs() <= drive_grade(Family::Asf3) + 1e-9,
            "piece {i}: {g}"
        );
    }
}

/// A chain of about a kilometre: 450 m north, a 90 degree arc of radius 10 m (a hairpin-like bend that lowers
/// the limit of its piece), 450 m east. Its ends are held at the ground, which rises on a convex ramp along the
/// way (steep first, then levelling off) by what is 96 % of the limit of the whole chain at its tight bend.
fn tight_bend_chain(family: Family) -> (PlacedNetwork, Heightmap, Junctions) {
    let mut roads = straight("a", family, (100.0, 100.0), (100.0, 550.0));
    let mut arc = piece(
        "a",
        100,
        PlacedKind::Piece,
        family,
        (100.0, 550.0),
        (110.0, 560.0),
    );
    arc.origin.heading = 0.0;
    arc.start.heading = 0.0;
    arc.end.heading = std::f64::consts::FRAC_PI_2;
    roads.push(arc);
    roads.extend(straight("b", family, (110.0, 560.0), (560.0, 560.0)));
    let net = network(vec![roads]);
    // the limit at the aim of the arc piece: the aim reduced by (r - w/2) / r
    let limit = grade_aim(family) * (10.0 - 3.5) / 10.0;
    let rise = 0.96 * limit * (450.0 + 14.14 + 450.0);
    let ground = move |x: f64, y: f64| {
        let t = ((x + y - 200.0) / 920.0).clamp(0.0, 1.0);
        100.0 + rise * (1.0 - (1.0 - t).powf(1.5))
    };
    let hm = terrain(200, 6.25, ground);
    let junction = |at: (f64, f64)| Junction {
        at,
        kind: JunctionKind::Join,
        ground: ground(at.0, at.1),
        height: ground(at.0, at.1),
        rank: 1,
        patch_m: 3.5,
        stretches: std::collections::BTreeSet::new(),
    };
    let junctions = Junctions {
        points: vec![junction((100.0, 100.0)), junction((560.0, 560.0))],
        ..Junctions::default()
    };
    (net, hm, junctions)
}

#[test]
fn one_tight_bend_does_not_turn_a_long_span_into_a_straight_ramp() {
    let family = Family::Mud;
    let (net, hm, junctions) = tight_bend_chain(family);
    let p = solve_profiles(&net, &hm, &junctions, &[]);
    let joints = &p.strips[0].joints;
    let ground = |x: f64, y: f64| f64::from(hm.sample(x, y).height);
    let mut worst = 0.0_f64;
    for j in joints
        .iter()
        .filter(|j| (j.0 - 100.0).hypot(j.1 - 560.0) > 40.0)
    {
        worst = worst.max((j.2 - ground(j.0, j.1)).abs());
    }
    assert!(worst <= 4.0, "the profile is {worst} m from the ground");
    assert!(
        grades(&p, 0)
            .iter()
            .all(|g| g.abs() <= drive_grade(family) + 1e-6)
    );
}

#[test]
fn a_profile_that_left_the_ground_for_a_straight_ramp_is_tried_again_at_a_looser_limit() {
    // a convex ground whose rise needs 90 % of the aim over 900 m: at the aim the span is a straight ramp, 25
    // m below the ground in the middle; the ground itself asks for no more than the middle of aim and
    // ceiling, so the next limit has to be tried and follows it
    let family = Family::Mud;
    let rise = 0.9 * grade_aim(family) * 900.0;
    let ground = move |_: f64, y: f64| {
        let t = ((y - 100.0) / 900.0).clamp(0.0, 1.0);
        100.0 + rise * (1.0 - (1.0 - t).powf(1.5))
    };
    let hm = terrain(200, 6.25, ground);
    let net = network(vec![straight("a", family, (100.0, 100.0), (100.0, 1000.0))]);
    let junction = |y: f64| Junction {
        at: (100.0, y),
        kind: JunctionKind::Join,
        ground: ground(100.0, y),
        height: ground(100.0, y),
        rank: 1,
        patch_m: 3.5,
        stretches: std::collections::BTreeSet::new(),
    };
    let junctions = Junctions {
        points: vec![junction(100.0), junction(1000.0)],
        ..Junctions::default()
    };
    let p = solve_profiles(&net, &hm, &junctions, &[]);
    let worst = p.strips[0]
        .joints
        .iter()
        .map(|j| (j.2 - ground(j.0, j.1)).abs())
        .fold(0.0_f64, f64::max);
    assert!(worst <= 12.0, "the profile is {worst} m from the ground");
}

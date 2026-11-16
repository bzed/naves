// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The heights of the road network as one quadratic program (`solve_network`).

#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test helper code; failing loudly is the point"
)]

use std::collections::BTreeSet;

use naves_core::catalog::Family;
use naves_core::catalog::testing::synthetic_catalog;
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::heightmap::Heightmap;
use naves_core::junctions::{Junction, JunctionKind, Junctions, Link, RingPlane, find_junctions};
use naves_core::limits::{grade_aim, grade_ceiling, min_vertical_radius};
use naves_core::profile::{
    GradeExcessKind, ProfileError, Profiles, chain_holds, solve_network, solve_network_with,
};
use naves_core::qp::QpError;
use naves_core::roadfit::Pose;
use naves_core::roadnet::{RoadGraph, build_graph};
use naves_core::roadplace::{
    OVERLAY_DROP_M, PlacedKind, PlacedNetwork, PlacedRoad, Slot, place_network,
};
use naves_core::sideslope::side_slopes;

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
    let pose = |p: (f64, f64)| Pose {
        x: p.0,
        y: p.1,
        heading,
    };
    PlacedRoad {
        slot: Slot {
            stretch: stretch.to_string(),
            index,
        },
        kind,
        family,
        drivable: true,
        model: "m".to_string(),
        origin: pose(a),
        start: pose(a),
        end: pose(b),
        width: 7.0,
        scale: 1.0,
    }
}

/// Pieces of at most `length` metres of the stretch `stretch` along the straight line from `a` to `b`.
fn pieces(
    stretch: &str,
    family: Family,
    a: (f64, f64),
    b: (f64, f64),
    length: f64,
) -> Vec<PlacedRoad> {
    let total = (b.0 - a.0).hypot(b.1 - a.1);
    let n = (total / length - 1e-9).ceil() as u32;
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
            roundabout: false,
            layer: 0,
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

/// An asf3 through road along y = 200 with an asf3 side road going north from x = 200: a junction piece.
fn t_junction_network() -> (PlacedNetwork, RoadGraph) {
    placed(&[
        feature(
            1,
            Family::Asf3,
            &[1, 2, 3],
            &[(50.0, 200.0), (200.0, 200.0), (350.0, 200.0)],
        ),
        feature(2, Family::Asf3, &[2, 4], &[(200.0, 200.0), (200.0, 350.0)]),
    ])
}

/// The new solve with the junctions as `find_junctions` finds them.
fn solve_found(net: &PlacedNetwork, graph: &RoadGraph, hm: &Heightmap) -> (Profiles, Junctions) {
    let junctions = find_junctions(net, graph, hm);
    solve_network(net, hm, &junctions, &side_slopes(net, hm)).unwrap()
}

fn ground(hm: &Heightmap, x: f64, y: f64) -> f64 {
    f64::from(hm.sample(x, y).height)
}

/// The grade of each segment of a strip.
fn grades(p: &Profiles, strip: usize) -> Vec<f64> {
    p.strips[strip]
        .joints
        .windows(2)
        .map(|w| {
            let d = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
            if d < 1e-9 { 0.0 } else { (w[1].2 - w[0].2) / d }
        })
        .collect()
}

/// The heights of the joints less the ground under them.
fn offsets(p: &Profiles, strip: usize, hm: &Heightmap) -> Vec<f64> {
    p.strips[strip]
        .joints
        .iter()
        .map(|j| j.2 - ground(hm, j.0, j.1))
        .collect()
}

/// One asf3 chain of 25 m pieces, 200 m north from (300, 100), on a plane that rises `grade` to the north.
fn one_road_on(grade: f64) -> (PlacedNetwork, Heightmap) {
    let hm = terrain(128, 6.25, move |_, y| 100.0 + grade * y);
    let net = network(vec![pieces(
        "a",
        Family::Asf3,
        (300.0, 100.0),
        (300.0, 300.0),
        25.0,
    )]);
    (net, hm)
}

fn solve_alone(net: &PlacedNetwork, hm: &Heightmap) -> Profiles {
    solve_network(net, hm, &Junctions::default(), &[])
        .unwrap()
        .0
}

#[test]
fn a_road_under_its_aim_lies_on_the_ground() {
    let (net, hm) = one_road_on(0.05);
    let p = solve_alone(&net, &hm);
    for off in offsets(&p, 0, &hm) {
        assert!(off.abs() < 1e-4, "{:?}", offsets(&p, 0, &hm));
    }
    assert!(p.excesses.is_empty(), "{:?}", p.excesses);
}

#[test]
fn a_road_steeper_than_its_ceiling_keeps_the_ceiling() {
    let (net, hm) = one_road_on(0.40);
    let p = solve_alone(&net, &hm);
    let ceiling = grade_ceiling(Family::Asf3);
    for g in grades(&p, 0) {
        assert!(g.abs() <= ceiling + 1e-4, "{:?}", grades(&p, 0));
    }
    assert!(p.excesses.is_empty(), "{:?}", p.excesses);
    let off = offsets(&p, 0, &hm);
    let mean = off.iter().sum::<f64>() / f64::from(u32::try_from(off.len()).unwrap());
    assert!(mean.abs() < 0.5, "{off:?}");
}

#[test]
fn between_aim_and_ceiling_the_road_stays_near_the_ground() {
    let (net, hm) = one_road_on(0.20);
    let p = solve_alone(&net, &hm);
    let aim = grade_aim(Family::Asf3);
    for g in grades(&p, 0) {
        assert!(g > aim && g <= 0.20 + 1e-4, "{:?}", grades(&p, 0));
    }
    for off in offsets(&p, 0, &hm) {
        assert!(off.abs() <= 2.0, "{:?}", offsets(&p, 0, &hm));
    }
}

/// A junction of a junction piece centred at `(x, 100)`, held at `ground`, tilting along x.
fn piece_junction(x: f64, ground: f64) -> Junction {
    Junction {
        at: (x, 100.0),
        kind: JunctionKind::Piece,
        ground,
        height: ground,
        rank: 3,
        patch_m: 6.25,
        stretches: BTreeSet::new(),
        lower: BTreeSet::new(),
        gradient: (0.0, 0.0),
        tilt_axis: Some((1.0, 0.0)),
        tilt_cap: 0.08,
    }
}

#[test]
fn two_junctions_too_steep_for_each_other_both_move() {
    // two junction pieces with centres 20 m apart on an asf3 road along y = 100, each with a side road ending
    // at its port; the ground steps 8 m between them
    let hm = terrain(160, 6.25, |x, _| if x < 316.25 { 100.0 } else { 108.0 });
    let mut through = pieces("t", Family::Asf3, (100.0, 100.0), (300.0, 100.0), 25.0);
    through.push(piece(
        "t|a",
        0,
        PlacedKind::Junction,
        Family::Asf3,
        (300.0, 100.0),
        (312.5, 100.0),
    ));
    through.push(piece(
        "t",
        20,
        PlacedKind::Piece,
        Family::Asf3,
        (312.5, 100.0),
        (320.0, 100.0),
    ));
    through.push(piece(
        "t|b",
        0,
        PlacedKind::Junction,
        Family::Asf3,
        (320.0, 100.0),
        (332.5, 100.0),
    ));
    through.extend(pieces(
        "t",
        Family::Asf3,
        (332.5, 100.0),
        (532.5, 100.0),
        25.0,
    ));
    let side_a = pieces("a", Family::Asf3, (306.25, 300.0), (306.25, 106.25), 25.0);
    let side_b = pieces("b", Family::Asf3, (326.25, 300.0), (326.25, 106.25), 25.0);
    let net = network(vec![through, side_a, side_b]);
    let junctions = Junctions {
        points: vec![piece_junction(306.25, 100.0), piece_junction(326.25, 108.0)],
        ..Junctions::default()
    };
    let (p, solved) = solve_network(&net, &hm, &junctions, &[]).unwrap();
    // the segment between the two pieces, from x = 312.5 to 320
    let joints = &p.strips[0].joints;
    let k = joints
        .iter()
        .position(|j| (j.0 - 312.5).abs() < 1e-6)
        .unwrap();
    let between = (joints[k + 1].2 - joints[k].2) / (joints[k + 1].0 - joints[k].0);
    assert!(
        between.abs() <= grade_ceiling(Family::Asf3) + 1e-4,
        "{between}"
    );
    for (j, before) in solved.points.iter().zip(&junctions.points) {
        assert!((j.height - before.ground).abs() >= 0.5, "{solved:?}");
    }
    assert!(p.excesses.is_empty(), "{:?}", p.excesses);
    assert!(solved.unresolved.is_empty(), "{:?}", solved.unresolved);
}

/// The held joints of every strip and their heights.
fn held_joints(
    net: &PlacedNetwork,
    p: &Profiles,
    hm: &Heightmap,
    junctions: &Junctions,
) -> Vec<(f64, f64, f64)> {
    let mut out = Vec::new();
    for (strip, roads) in p.strips.iter().zip(&net.chains) {
        let holds = chain_holds(net, roads, f64::from(hm.cell_m), junctions);
        for (joint, (_, hold)) in strip.joints.iter().zip(&holds) {
            if hold.is_some() {
                out.push(*joint);
            }
        }
    }
    out
}

#[test]
fn a_junction_on_a_slope_tilts_with_its_through_road() {
    let (net, graph) = t_junction_network();
    let hm = terrain(128, 3.125, |x, _| 100.0 + 0.06 * x);
    let junctions = find_junctions(&net, &graph, &hm);
    assert_eq!(junctions.points.len(), 1, "{junctions:?}");
    assert_eq!(junctions.points[0].kind, JunctionKind::Piece);
    assert!(junctions.points[0].tilt_axis.is_some(), "{junctions:?}");
    let (p, solved) = solve_network(&net, &hm, &junctions, &side_slopes(&net, &hm)).unwrap();
    let j = &solved.points[0];
    assert!((j.gradient.0 - 0.06).abs() <= 1e-3, "{j:?}");
    assert!(j.gradient.1.abs() <= 1e-3, "{j:?}");
    let held = held_joints(&net, &p, &hm, &junctions);
    assert!(held.len() > 1, "{held:?}");
    for q in held {
        assert!((q.2 - ground(&hm, q.0, q.1)).abs() <= 0.01, "{q:?}");
    }
}

#[test]
fn a_junction_tilt_stays_within_its_cap() {
    let (net, graph) = t_junction_network();
    let hm = terrain(128, 3.125, |x, _| 100.0 + 0.20 * x);
    let junctions = find_junctions(&net, &graph, &hm);
    let cap = junctions.points[0].tilt_cap;
    assert!(cap > 0.0, "{junctions:?}");
    let (_, solved) = solve_network(&net, &hm, &junctions, &side_slopes(&net, &hm)).unwrap();
    let j = &solved.points[0];
    assert!(j.gradient.0.hypot(j.gradient.1) <= cap + 1e-6, "{j:?}");
}

/// 12.5 m pieces north from y = 100 to a 25 m deck from y = 300 to 325, and 12.5 m pieces on to y = 525.
fn deck_chain() -> Vec<PlacedRoad> {
    let mut chain = pieces("a", Family::Asf3, (300.0, 100.0), (300.0, 300.0), 12.5);
    chain.push(piece(
        "b",
        0,
        PlacedKind::Bridge,
        Family::Asf3,
        (300.0, 300.0),
        (300.0, 325.0),
    ));
    chain.extend(pieces(
        "c",
        Family::Asf3,
        (300.0, 325.0),
        (300.0, 525.0),
        12.5,
    ));
    chain
}

/// The joint of strip 0 at `y` on the line x = 300.
fn joint_at(p: &Profiles, y: f64) -> (f64, f64, f64) {
    *p.strips[0]
        .joints
        .iter()
        .find(|j| (j.1 - y).abs() < 1e-6)
        .unwrap()
}

#[test]
fn a_deck_bends_at_both_ends() {
    // the ground rises 10 % to the deck and falls 10 % after it
    let hm = terrain(128, 6.25, |_, y| {
        if y <= 300.0 {
            100.0 + 0.1 * y
        } else if y <= 325.0 {
            130.0
        } else {
            130.0 - 0.1 * (y - 325.0)
        }
    });
    let net = network(vec![deck_chain()]);
    let p = solve_alone(&net, &hm);
    let grade = |a: (f64, f64, f64), b: (f64, f64, f64)| (b.2 - a.2) / (b.1 - a.1);
    let run = f64::midpoint(12.5, 25.0);
    let limit = run / min_vertical_radius(Family::Asf3) + 1e-4;
    let (start, end) = (joint_at(&p, 300.0), joint_at(&p, 325.0));
    assert!((start.2 - end.2).abs() < 1e-9, "the deck is level");
    let before = grade(joint_at(&p, 287.5), start);
    let after = grade(end, joint_at(&p, 337.5));
    assert!(before.abs() <= limit, "into the deck: {before}");
    assert!(after.abs() <= limit, "out of the deck: {after}");
    assert!(
        !p.excesses
            .iter()
            .any(|e| e.kind == GradeExcessKind::VerticalBend),
        "{:?}",
        p.excesses
    );
}

#[test]
fn a_sharp_deck_end_is_reported() {
    // the whole chain lies on a ring's plane that rises 20 % to the north: the approaches are at 20 % or more
    // on both sides of the level deck
    let hm = terrain(128, 6.25, |_, _| 100.0);
    let net = network(vec![deck_chain()]);
    let plane = RingPlane {
        at: (300.0, 300.0),
        radius: 10.0,
        inner_radius: 3.0,
        height: 160.0,
        gradient: (0.0, 0.2),
        rank: 3,
        stretches: ["a", "b", "c"].iter().map(|s| (*s).to_string()).collect(),
    };
    let junctions = Junctions {
        rings: vec![plane],
        ..Junctions::default()
    };
    let (p, _) = solve_network(&net, &hm, &junctions, &[]).unwrap();
    let bends_at = |y: f64| {
        p.excesses
            .iter()
            .filter(|e| e.kind == GradeExcessKind::VerticalBend)
            .filter(|e| (e.at.0 - 300.0).hypot(e.at.1 - y) < 1.0)
            .count()
    };
    assert_eq!(bends_at(300.0), 1, "{:?}", p.excesses);
    assert_eq!(bends_at(325.0), 1, "{:?}", p.excesses);
}

/// A plane of a ring of the stretch `id` at `height`, level.
fn level_ring(id: &str, at: (f64, f64), height: f64) -> RingPlane {
    RingPlane {
        at,
        radius: 10.0,
        inner_radius: 3.0,
        height,
        gradient: (0.0, 0.0),
        rank: 3,
        stretches: std::iter::once(id.to_string()).collect(),
    }
}

#[test]
fn fixed_heights_that_cannot_be_joined_give_a_finding() {
    // two ring roads 10 m apart, their planes 10 m apart in height, and a road of 2 m pieces between them
    let hm = terrain(512, 0.25, |_, _| 100.0);
    let low = pieces("r1", Family::Asf3, (20.0, 60.0), (100.0, 60.0), 25.0);
    let high = pieces("r2", Family::Asf3, (20.0, 70.0), (100.0, 70.0), 25.0);
    let road = pieces("s", Family::Asf3, (60.0, 60.0), (60.0, 70.0), 2.0);
    let net = network(vec![low, high, road]);
    let junctions = Junctions {
        rings: vec![
            level_ring("r1", (60.0, 60.0), 100.0),
            level_ring("r2", (60.0, 70.0), 110.0),
        ],
        ..Junctions::default()
    };
    let (p, _) = solve_network(&net, &hm, &junctions, &[]).unwrap();
    let ends = &p.strips[2].joints;
    assert!((ends[0].2 - 100.0).abs() < 1e-6 && (ends[ends.len() - 1].2 - 110.0).abs() < 1e-6);
    assert!(
        p.excesses.iter().any(|e| e.kind == GradeExcessKind::Grade
            && (e.at.0 - 60.0).abs() < 1.0
            && (60.0..=70.0).contains(&e.at.1)),
        "{:?}",
        p.excesses
    );
}

#[test]
fn a_chain_end_on_another_road_takes_its_height() {
    // no junction: a side road starts on the edge of the through road at y = 500
    let hm = terrain(200, 6.25, |x, y| 100.0 + 0.11 * y + 0.03 * x);
    let through = pieces("t", Family::Asf3, (300.0, 200.0), (300.0, 800.0), 25.0);
    let side = pieces("s", Family::Mud, (303.5, 500.0), (500.0, 500.0), 25.0);
    let net = network(vec![through, side]);
    let p = solve_alone(&net, &hm);
    let on_through = joint_at(&p, 500.0).2;
    let start = p.strips[1].joints[0].2;
    assert!((start - on_through).abs() < 1e-6, "{start} {on_through}");
}

#[test]
fn a_road_under_an_overlay_lies_5_cm_below() {
    // a City street along y = 200 with a Mud track ending on it: the game has no T for the pair
    let (net, graph) = placed(&[
        feature(
            1,
            Family::City,
            &[1, 2, 3],
            &[(50.0, 200.0), (200.0, 200.0), (350.0, 200.0)],
        ),
        feature(2, Family::Mud, &[2, 4], &[(200.0, 200.0), (200.0, 350.0)]),
    ]);
    assert_eq!(net.overlays.len(), 1);
    let hm = terrain(128, 3.125, |_, _| 100.0);
    let (p, solved) = solve_found(&net, &graph, &hm);
    assert_eq!(solved.points.len(), 1);
    let height_near_cut = |family: Family| {
        let mut best: Option<(f64, f64)> = None;
        for (strip, chain) in p.strips.iter().zip(&net.chains) {
            if net.roads[chain[0]].family != family {
                continue;
            }
            for joint in &strip.joints {
                let d = (joint.0 - 200.0).hypot(joint.1 - 200.0);
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, joint.2));
                }
            }
        }
        best.unwrap().1
    };
    let (street, track) = (height_near_cut(Family::City), height_near_cut(Family::Mud));
    assert!(
        (street - track - OVERLAY_DROP_M).abs() < 1e-6,
        "street {street}, track {track}"
    );
    assert!((street - solved.points[0].height).abs() < 1e-6);
}

#[test]
fn a_single_piece_network_solves() {
    let hm = terrain(128, 6.25, |_, y| 100.0 + 0.05 * y);
    let net = network(vec![vec![piece(
        "a",
        0,
        PlacedKind::Piece,
        Family::Asf3,
        (300.0, 100.0),
        (300.0, 125.0),
    )]]);
    let p = solve_alone(&net, &hm);
    assert_eq!(p.strips.len(), 1);
    for off in offsets(&p, 0, &hm) {
        assert!(off.abs() < 1e-4, "{:?}", offsets(&p, 0, &hm));
    }
}

#[test]
fn a_ring_alone_solves() {
    let hm = terrain(128, 6.25, |_, _| 100.0);
    let net = network(vec![pieces(
        "r",
        Family::Asf3,
        (300.0, 100.0),
        (300.0, 200.0),
        25.0,
    )]);
    let mut plane = level_ring("r", (300.0, 150.0), 120.0);
    plane.gradient = (0.0, 0.05);
    let junctions = Junctions {
        rings: vec![plane.clone()],
        ..Junctions::default()
    };
    let (p, _) = solve_network(&net, &hm, &junctions, &[]).unwrap();
    for j in &p.strips[0].joints {
        assert!((j.2 - plane.height_at((j.0, j.1))).abs() < 1e-9, "{j:?}");
    }
}

#[test]
fn no_data_under_a_road_is_an_error() {
    let hm = terrain(128, 6.25, |_, y| {
        if (y - 200.0).abs() < 7.0 {
            f64::NAN
        } else {
            100.0
        }
    });
    let (net, _) = one_road_on(0.0);
    let err = solve_network(&net, &hm, &Junctions::default(), &[]).unwrap_err();
    assert!(matches!(err, ProfileError::Invalid { .. }), "{err:?}");
}

#[test]
fn a_solver_that_fails_is_an_error() {
    let (net, hm) = one_road_on(0.05);
    let failing = |_: &naves_core::qp::Problem| {
        Err(QpError::NotSolved {
            status: "MaxIterations".to_string(),
        })
    };
    let err = solve_network_with(&net, &hm, &Junctions::default(), &[], &failing).unwrap_err();
    assert_eq!(
        err,
        ProfileError::NotSolved {
            status: "MaxIterations".to_string()
        }
    );
    assert!(err.to_string().contains("MaxIterations"), "{err}");
}

#[test]
fn the_solve_is_bit_identical() {
    let (net, graph) = t_junction_network();
    let hm = terrain(128, 3.125, |x, y| 100.0 + 0.07 * x + 0.02 * y);
    let (a, ja) = solve_found(&net, &graph, &hm);
    let (b, jb) = solve_found(&net, &graph, &hm);
    let bits = |p: &Profiles| -> Vec<u64> {
        p.strips
            .iter()
            .flat_map(|s| s.joints.iter().map(|j| j.2.to_bits()))
            .collect()
    };
    assert!(bits(&a).len() > 10);
    assert_eq!(bits(&a), bits(&b));
    assert_eq!(ja, jb);
}

// ---- fix round 1: the deck ends, unresolved links, meetings on held joints, non-finite input ----

/// The grades into and out of the deck of [`deck_chain`] and the limit of the change at each deck end.
fn deck_end_grades(p: &Profiles) -> (f64, f64, f64) {
    let grade = |a: (f64, f64, f64), b: (f64, f64, f64)| (b.2 - a.2) / (b.1 - a.1);
    let before = grade(joint_at(p, 287.5), joint_at(p, 300.0));
    let after = grade(joint_at(p, 325.0), joint_at(p, 337.5));
    let limit = f64::midpoint(12.5, 25.0) / min_vertical_radius(Family::Asf3) + 1e-4;
    (before, after, limit)
}

#[test]
fn a_deck_on_a_steady_slope_bends_at_both_ends() {
    // the ground rises 10 % through the deck: a road that kept the approaches at 10 % and compared them with
    // each other (as if the deck had no length) would jump off one deck end and land on the other
    let hm = terrain(128, 6.25, |_, y| 100.0 + 0.1 * y);
    let net = network(vec![deck_chain()]);
    let p = solve_alone(&net, &hm);
    let (before, after, limit) = deck_end_grades(&p);
    assert!(before.abs() <= limit, "into the deck: {before}");
    assert!(after.abs() <= limit, "out of the deck: {after}");
    assert!(
        !p.excesses
            .iter()
            .any(|e| e.kind == GradeExcessKind::VerticalBend),
        "{:?}",
        p.excesses
    );
}

#[test]
fn a_road_levels_into_a_deck_on_a_gentle_slope() {
    // 2 % through the deck: no row is near its limit, so only the cost decides. Its smoothness counts the bend
    // at each deck end (the deck is level), so the approaches are flatter than the ground next to the deck (one
    // stencil across the deck left them steeper than the ground, at 2.6 %)
    let hm = terrain(128, 6.25, |_, y| 100.0 + 0.02 * y);
    let net = network(vec![deck_chain()]);
    let p = solve_alone(&net, &hm);
    let (before, after, _) = deck_end_grades(&p);
    assert!(before.abs() < 0.02, "into the deck: {before}");
    assert!(after.abs() < 0.02, "out of the deck: {after}");
}

/// A junction at `(300, y)` that lies on the ring of the stretch `ring` and belongs to the chain stretch
/// `stretch`: it holds the one variable at `(300, y)`.
fn ring_held(y: f64, stretch: &str, ring: &str) -> Junction {
    Junction {
        at: (300.0, y),
        kind: JunctionKind::Join,
        ground: 100.0,
        height: 100.0,
        rank: 3,
        patch_m: 1.0,
        stretches: [stretch, ring].iter().map(|s| (*s).to_string()).collect(),
        lower: BTreeSet::new(),
        gradient: (0.0, 0.0),
        tilt_axis: None,
        tilt_cap: 0.0,
    }
}

#[test]
fn a_sharp_deck_end_between_held_approaches_is_reported() {
    // the two joints before the deck lie on a plane rising 20 % to the deck, the two after it on one falling
    // 20 %; the deck itself is free, and no height of it keeps both deck ends within the vertical radius
    let hm = terrain(128, 6.25, |_, _| 100.0);
    let net = network(vec![deck_chain()]);
    let mut up = level_ring("r1", (300.0, 275.0), 100.0);
    up.gradient = (0.0, 0.2);
    let mut down = level_ring("r2", (300.0, 350.0), 100.0);
    down.gradient = (0.0, -0.2);
    let junctions = Junctions {
        rings: vec![up, down],
        points: vec![
            ring_held(275.0, "a", "r1"),
            ring_held(287.5, "a", "r1"),
            ring_held(337.5, "c", "r2"),
            ring_held(350.0, "c", "r2"),
        ],
        ..Junctions::default()
    };
    let (p, _) = solve_network(&net, &hm, &junctions, &[]).unwrap();
    assert!((joint_at(&p, 287.5).2 - 102.5).abs() < 1e-9);
    assert!((joint_at(&p, 337.5).2 - 102.5).abs() < 1e-9);
    let bends_at = |y: f64| {
        p.excesses
            .iter()
            .filter(|e| e.kind == GradeExcessKind::VerticalBend)
            .filter(|e| (e.at.0 - 300.0).hypot(e.at.1 - y) < 1.0)
            .count()
    };
    assert_eq!(bends_at(300.0), 1, "{:?}", p.excesses);
    assert_eq!(bends_at(325.0), 1, "{:?}", p.excesses);
}

#[test]
fn a_link_between_junctions_too_far_apart_in_height_is_unresolved() {
    // two junctions on rings 10 m apart in height, joined by a 10 m road of 2 m pieces and a link
    let hm = terrain(512, 0.25, |_, _| 100.0);
    let road = pieces("s", Family::Asf3, (60.0, 60.0), (60.0, 70.0), 2.0);
    let net = network(vec![road]);
    let at = |y: f64, ring: &str| {
        let mut j = ring_held(y, "s", ring);
        j.at = (60.0, y);
        j
    };
    let junctions = Junctions {
        rings: vec![
            level_ring("r1", (60.0, 60.0), 100.0),
            level_ring("r2", (60.0, 70.0), 110.0),
        ],
        points: vec![at(60.0, "r1"), at(70.0, "r2")],
        links: vec![Link {
            a: 0,
            b: 1,
            length: 10.0,
            ceiling: grade_ceiling(Family::Asf3),
            a_at: (60.0, 60.0),
            b_at: (60.0, 70.0),
        }],
        unresolved: Vec::new(),
    };
    let (p, solved) = solve_network(&net, &hm, &junctions, &[]).unwrap();
    let ends = &p.strips[0].joints;
    assert!((ends[0].2 - 100.0).abs() < 1e-9 && (ends[ends.len() - 1].2 - 110.0).abs() < 1e-9);
    assert_eq!(solved.unresolved, vec![0]);
}

#[test]
fn a_chain_end_on_held_joints_takes_their_height() {
    // a side road that does not belong to the junction ends on the port of a tilted junction piece: it is a
    // meeting on the piece, whose two joints are held to the junction's plane
    let hm = terrain(160, 6.25, |x, _| 100.0 + 0.05 * x);
    let mut through = pieces("t", Family::Asf3, (100.0, 100.0), (300.0, 100.0), 25.0);
    through.push(piece(
        "t",
        20,
        PlacedKind::Junction,
        Family::Asf3,
        (300.0, 100.0),
        (312.5, 100.0),
    ));
    through.extend(pieces(
        "t",
        Family::Asf3,
        (312.5, 100.0),
        (512.5, 100.0),
        25.0,
    ));
    let side = pieces("s", Family::Asf3, (306.25, 106.25), (306.25, 300.0), 25.0);
    let net = network(vec![through, side]);
    // the junction's ground lies 2 m above the terrain, so that its plane differs from the ground and a free
    // side end (which follows the ground) would not land on it
    let mut j = piece_junction(306.25, 100.0 + 0.05 * 306.25 + 2.0);
    j.stretches = std::iter::once("t".to_string()).collect();
    j.gradient = (0.05, 0.0);
    let junctions = Junctions {
        points: vec![j],
        ..Junctions::default()
    };
    let (p, solved) = solve_network(&net, &hm, &junctions, &[]).unwrap();
    let held = held_joints(&net, &p, &hm, &junctions);
    assert_eq!(held.len(), 2, "{held:?}");
    // the plane is tilted, so the two held joints differ, and the port lies half way between them
    assert!((held[0].2 - held[1].2).abs() > 0.1, "{held:?}");
    let middle = f64::midpoint(held[0].2, held[1].2);
    let start = p.strips[1].joints[0].2;
    assert!((start - middle).abs() < 1e-6, "{start} against {middle}");
    let terrain_at_start = ground(&hm, p.strips[1].joints[0].0, p.strips[1].joints[0].1);
    assert!(
        (middle - terrain_at_start).abs() > 0.3,
        "the plane must differ from the ground: {middle} against {terrain_at_start}"
    );
    assert!((middle - solved.points[0].height).abs() < 1e-6);
}

#[test]
fn a_ring_plane_that_is_not_a_number_is_an_error() {
    let hm = terrain(128, 6.25, |_, _| 100.0);
    let net = network(vec![pieces(
        "r",
        Family::Asf3,
        (300.0, 100.0),
        (300.0, 200.0),
        25.0,
    )]);
    for (height, gradient) in [(f64::NAN, (0.0, 0.0)), (100.0, (0.0, f64::INFINITY))] {
        let mut plane = level_ring("r", (300.0, 150.0), height);
        plane.gradient = gradient;
        let junctions = Junctions {
            rings: vec![plane],
            ..Junctions::default()
        };
        let err = solve_network(&net, &hm, &junctions, &[]).unwrap_err();
        assert!(
            matches!(&err, ProfileError::Invalid { what } if what.contains("300.0")),
            "{err:?}"
        );
    }
    // a junction on such a ring holds a road that is not a ring
    let road = network(vec![pieces(
        "s",
        Family::Asf3,
        (300.0, 100.0),
        (300.0, 200.0),
        25.0,
    )]);
    let junctions = Junctions {
        rings: vec![level_ring("r", (300.0, 100.0), f64::NAN)],
        points: vec![ring_held(100.0, "s", "r")],
        ..Junctions::default()
    };
    let err = solve_network(&road, &hm, &junctions, &[]).unwrap_err();
    assert!(
        matches!(&err, ProfileError::Invalid { what } if what.contains("300.0")),
        "{err:?}"
    );
}

#[test]
fn infinite_ground_under_a_road_is_an_error() {
    let hm = terrain(128, 6.25, |_, y| {
        if (y - 200.0).abs() < 7.0 {
            f64::INFINITY
        } else {
            100.0
        }
    });
    let (net, _) = one_road_on(0.0);
    let err = solve_network(&net, &hm, &Junctions::default(), &[]).unwrap_err();
    assert!(matches!(err, ProfileError::Invalid { .. }), "{err:?}");
}

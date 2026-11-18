// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The heights of the road network as one quadratic program (`solve_profiles`, `solve_clearing`).

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
use naves_core::conform::FILL_SLOPE_MAX;
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::heightmap::Heightmap;
use naves_core::junctions::{Junction, JunctionKind, Junctions, Link, RingPlane, find_junctions};
use naves_core::limits::{
    GRADE_BREAK_MAX, RAMP_JOINT_MIN_M, grade_aim, grade_ceiling, min_vertical_radius,
};
use naves_core::profile::{
    BRIDGE_CLEARANCE_M, CLEARANCE_TOLERANCE_M, GradeExcessKind, MEETING_GRADE, ProfileError,
    Profiles, chain_holds, nearby_pairs, segment_ceilings, solve_clearing, solve_clearing_with,
};
use naves_core::qp::QpError;
use naves_core::roadfit::Pose;
use naves_core::roadnet::{RoadGraph, build_graph};
use naves_core::roadplace::{
    OVERLAY_DROP_M, PlacedKind, PlacedNetwork, PlacedRoad, Slot, place_network,
};
use naves_core::sideslope::side_slopes;

/// The solve without a road graph (so no bridge over a road): the profiles and the junctions.
fn solve_plain(
    net: &PlacedNetwork,
    hm: &Heightmap,
    junctions: &Junctions,
    side: &[f64],
) -> Result<(Profiles, Junctions), ProfileError> {
    solve_clearing(net, hm, junctions, side, &RoadGraph::default(), &|_| None)
}

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
        abreast: 1,
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
    solve_plain(net, hm, &junctions, &side_slopes(net, hm)).unwrap()
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
    solve_plain(net, hm, &Junctions::default(), &[]).unwrap().0
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
fn the_segment_ceilings_are_what_the_grades_are_held_to() {
    // a road steeper than its ceiling lies at its ceiling on every segment; a 25 m deck is one variable, so the
    // chain of deck_chain has one segment less than joints less one
    let (net, hm) = one_road_on(0.40);
    let p = solve_alone(&net, &hm);
    let ceilings = segment_ceilings(&net, &hm, &Junctions::default()).unwrap();
    assert_eq!(ceilings.len(), 1);
    assert_eq!(ceilings[0].roads, p.strips[0].slots);
    let joints = &p.strips[0].joints;
    assert_eq!(ceilings[0].segments.len(), joints.len() - 1);
    for s in &ceilings[0].segments {
        assert_eq!(s.to, s.from + 1);
        assert!((s.ceiling - grade_ceiling(Family::Asf3)).abs() < 1e-12);
        let grade = (joints[s.to].2 - joints[s.from].2) / s.run;
        assert!((grade - s.ceiling).abs() < 1e-4, "{grade} {s:?}");
    }
    let hm = terrain(128, 6.25, |_, _| 100.0);
    let deck = network(vec![deck_chain()]);
    let ceilings = segment_ceilings(&deck, &hm, &Junctions::default()).unwrap();
    let joints = deck_chain().len() + 1;
    assert_eq!(ceilings[0].segments.len(), joints - 2);
    let across = ceilings[0].segments.iter().find(|s| s.to > s.from + 1);
    assert!(across.is_none(), "{across:?}");
}

/// An asf3 road north along x = 300 of 25 m pieces, and an asf3 side road from the east whose end lies at
/// (`end_x`, 200), on ground that rises 0.8 m per metre to the east beyond x = 300: the ceilings of the road.
fn ceilings_beside(end_x: f64) -> Vec<f64> {
    let hm = terrain(128, 6.25, |x, _| 100.0 + 0.8 * (x - 300.0).max(0.0));
    let road = pieces("a", Family::Asf3, (300.0, 100.0), (300.0, 300.0), 25.0);
    let side = pieces("s", Family::Asf3, (450.0, 200.0), (end_x, 200.0), 25.0);
    let net = network(vec![road, side]);
    let ceilings = segment_ceilings(&net, &hm, &Junctions::default()).unwrap();
    ceilings[0].segments.iter().map(|s| s.ceiling).collect()
}

#[test]
fn a_dead_end_beside_a_road_is_no_meeting() {
    // the side road ends 12.5 m from the road's centre line, 9 m beyond its edge and 10 m up the slope: it does
    // not meet the road, so the road keeps its ceiling there
    for c in ceilings_beside(312.5) {
        assert!((c - grade_ceiling(Family::Asf3)).abs() < 1e-12, "{c}");
    }
}

#[test]
fn a_road_that_ends_on_another_gives_it_the_meeting_grade() {
    // the side road ends on the road's edge at y = 200: the pieces it meets there have the meeting grade
    let ceilings = ceilings_beside(303.5);
    let meeting = ceilings
        .iter()
        .filter(|c| (**c - MEETING_GRADE).abs() < 1e-12)
        .count();
    assert!((1..=3).contains(&meeting), "{ceilings:?}");
    assert!(
        ceilings
            .iter()
            .any(|c| (c - grade_ceiling(Family::Asf3)).abs() < 1e-12),
        "{ceilings:?}"
    );
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
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
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

#[test]
fn a_junction_is_pulled_to_its_ground() {
    // a junction piece on ground that falls 4 % along the through road, with nothing forcing it (no bridge, no
    // ring, no second junction). The ground under the side road's arm is 4 m higher than under the through road,
    // a pull on the junction's joints. The junction's own ground is 4 m above the terrain under it, so that its tie
    // to that ground (`JUNCTION_GROUND`) has a metre-sized offset to decide: measured, the junction ends 1.31 m above
    // the terrain (0.32 m, where the roads alone put it, if `JUNCTION_GROUND` were 0). The bound sits a factor of
    // two from both
    let ground_at =
        |x: f64, y: f64| 100.0 - 0.04 * (x - 306.25) + if y > 108.0 { 4.0 } else { 0.0 };
    let hm = terrain(160, 6.25, ground_at);
    let mut through = pieces("t", Family::Asf3, (100.0, 100.0), (300.0, 100.0), 25.0);
    through.push(piece(
        "t|a",
        0,
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
    let side = pieces("a", Family::Asf3, (306.25, 300.0), (306.25, 106.25), 25.0);
    let net = network(vec![through, side]);
    let junctions = Junctions {
        points: vec![piece_junction(306.25, ground_at(306.25, 100.0) + 4.0)],
        ..Junctions::default()
    };
    let (_, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    let lift = solved.points[0].height - ground_at(306.25, 100.0);
    assert!(lift >= 0.65, "{lift}");
    assert!(
        solved.points[0].height < junctions.points[0].ground,
        "{lift}"
    );
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
    let (p, solved) = solve_plain(&net, &hm, &junctions, &side_slopes(&net, &hm)).unwrap();
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
    let (_, solved) = solve_plain(&net, &hm, &junctions, &side_slopes(&net, &hm)).unwrap();
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
    let (p, _) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    let bends_at = |y: f64| {
        p.excesses
            .iter()
            .filter(|e| e.kind == GradeExcessKind::GradeBreak)
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
    let (p, _) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
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
    let (p, _) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
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
    let err = solve_plain(&net, &hm, &Junctions::default(), &[]).unwrap_err();
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
    let err = solve_clearing_with(
        &net,
        &hm,
        &Junctions::default(),
        &[],
        &RoadGraph::default(),
        &|_| None,
        &failing,
    )
    .unwrap_err();
    let ProfileError::NotSolved { status } = &err else {
        panic!("{err:?}");
    };
    assert!(status.ends_with("MaxIterations"), "{status}");
    assert!(err.to_string().contains("MaxIterations"), "{err}");
}

#[test]
fn a_failed_solve_names_its_size() {
    // the message names the size of the problem the solver was given
    let (net, hm) = one_road_on(0.05);
    let size = std::cell::Cell::new((0, 0));
    let failing = |problem: &naves_core::qp::Problem| {
        size.set((problem.unknowns, problem.rows.len()));
        Err(QpError::NotSolved {
            status: "MaxIterations".to_string(),
        })
    };
    let err = solve_clearing_with(
        &net,
        &hm,
        &Junctions::default(),
        &[],
        &RoadGraph::default(),
        &|_| None,
        &failing,
    )
    .unwrap_err();
    let (unknowns, rows) = size.get();
    assert!(unknowns > 0 && rows > 0, "{unknowns} {rows}");
    assert_eq!(
        err.to_string(),
        format!("the road heights are not solved: {unknowns} unknowns, {rows} rows: MaxIterations")
    );
    // with a bridge over a road the clearance is in the same problem: one solve, with its rows
    let hm = terrain(250, 4.0, |_, _| 100.0);
    let (net, graph) = placed(&bridge_features());
    let junctions = find_junctions(&net, &graph, &hm);
    let slopes = side_slopes(&net, &hm);
    let calls = std::cell::Cell::new(0);
    let fails = |problem: &naves_core::qp::Problem| {
        calls.set(calls.get() + 1);
        size.set((problem.unknowns, problem.rows.len()));
        Err(QpError::NotSolved {
            status: "PrimalInfeasible".to_string(),
        })
    };
    let err = solve_clearing_with(
        &net,
        &hm,
        &junctions,
        &slopes,
        &graph,
        &|_| Some(DECK),
        &fails,
    )
    .unwrap_err();
    assert_eq!(calls.get(), 1);
    let (unknowns, rows) = size.get();
    assert_eq!(
        err.to_string(),
        format!(
            "the road heights are not solved: {unknowns} unknowns, {rows} rows: PrimalInfeasible"
        )
    );
    // the solver that fails is given the problem of the clearing solve: one slack and two rows more than without
    let unchecked = std::cell::Cell::new((0, 0));
    let count = |problem: &naves_core::qp::Problem| {
        unchecked.set((problem.unknowns, problem.rows.len()));
        naves_core::qp::solve(problem)
    };
    solve_clearing_with(&net, &hm, &junctions, &slopes, &graph, &|_| None, &count).unwrap();
    assert_eq!(
        (unchecked.get().0 + 1, unchecked.get().1 + 2),
        (unknowns, rows)
    );
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
    // a deck end is in a ramp zone: the break limit there is GRADE_BREAK_MAX where the radius allows less
    let limit =
        (f64::midpoint(12.5, 25.0) / min_vertical_radius(Family::Asf3)).max(GRADE_BREAK_MAX) + 1e-4;
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
    // the two joints before the deck lie on a plane rising 30 % to the deck, the two after it on one falling
    // 30 %; the deck itself is free, and no height of it keeps both deck ends within the break limit of a ramp zone
    // (GRADE_BREAK_MAX, 10 %: the deck at 15 % to each side makes two breaks of 15 %)
    let hm = terrain(128, 6.25, |_, _| 100.0);
    let net = network(vec![deck_chain()]);
    let mut up = level_ring("r1", (300.0, 275.0), 100.0);
    up.gradient = (0.0, 0.3);
    let mut down = level_ring("r2", (300.0, 350.0), 100.0);
    down.gradient = (0.0, -0.3);
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
    let (p, _) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    assert!((joint_at(&p, 287.5).2 - 103.75).abs() < 1e-9);
    assert!((joint_at(&p, 337.5).2 - 103.75).abs() < 1e-9);
    let bends_at = |y: f64| {
        p.excesses
            .iter()
            .filter(|e| e.kind == GradeExcessKind::GradeBreak)
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
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
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
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
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
        let err = solve_plain(&net, &hm, &junctions, &[]).unwrap_err();
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
    let err = solve_plain(&road, &hm, &junctions, &[]).unwrap_err();
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
    let err = solve_plain(&net, &hm, &Junctions::default(), &[]).unwrap_err();
    assert!(matches!(err, ProfileError::Invalid { .. }), "{err:?}");
}

// ---- the clearance under a bridge over a road (rows C) ----

/// The thickness of every deck in the tests, in metres.
const DECK: f64 = 1.5;

fn way(id: i64, nodes: &[i64], points: &[(f64, f64)], bridge: bool) -> Feature {
    let mut f = feature(id, Family::Asf3, nodes, points);
    if let FeatureKind::Road {
        bridge: b, layer, ..
    } = &mut f.kind
    {
        *b = bridge;
        *layer = i8::from(bridge);
    }
    f
}

/// A road east-west at y = 500 and a 25 m bridge over it, going north, with approaches.
fn bridge_features() -> Vec<Feature> {
    vec![
        way(1, &[1, 2], &[(50.0, 500.0), (550.0, 500.0)], false),
        way(3, &[3, 4], &[(300.0, 100.0), (300.0, 487.5)], false),
        way(4, &[4, 5], &[(300.0, 487.5), (300.0, 512.5)], true),
        way(5, &[5, 6], &[(300.0, 512.5), (300.0, 900.0)], false),
    ]
}

/// The surface height of the strip of the stretch that starts with `id` at the joint nearest `at`.
fn height_on(p: &Profiles, net: &PlacedNetwork, id: &str, at: (f64, f64), bridge: bool) -> f64 {
    let mut best = (f64::INFINITY, 0.0);
    for strip in &p.strips {
        for (k, w) in strip.joints.windows(2).enumerate() {
            let on_it = strip.slots.get(k).is_some_and(|r| {
                net.roads[*r].slot.stretch.starts_with(id)
                    && (net.roads[*r].kind == PlacedKind::Bridge) == bridge
            });
            if !on_it {
                continue;
            }
            for j in w {
                let d = (j.0 - at.0).hypot(j.1 - at.1);
                if d < best.0 {
                    best = (d, j.2);
                }
            }
        }
    }
    assert!(best.0 < 30.0, "no joint of {id} near {at:?}");
    best.1
}

/// The index of the strip that carries a bridge.
fn bridge_strip(p: &Profiles, net: &PlacedNetwork) -> usize {
    p.strips
        .iter()
        .position(|s| {
            s.slots
                .iter()
                .any(|r| net.roads[*r].kind == PlacedKind::Bridge)
        })
        .unwrap()
}

fn clear(
    net: &PlacedNetwork,
    graph: &RoadGraph,
    hm: &Heightmap,
    junctions: &Junctions,
    thickness: f64,
) -> (Profiles, Junctions) {
    solve_clearing(net, hm, junctions, &side_slopes(net, hm), graph, &|_| {
        Some(thickness)
    })
    .unwrap()
}

#[test]
fn a_bridge_over_a_road_clears_it() {
    let hm = terrain(250, 4.0, |_, _| 100.0);
    let (net, graph) = placed(&bridge_features());
    let junctions = find_junctions(&net, &graph, &hm);
    let at = (300.0, 500.0);
    // without the rule the deck lies on the ground, in the road below
    let (flat, _) = solve_plain(&net, &hm, &junctions, &side_slopes(&net, &hm)).unwrap();
    let gap =
        height_on(&flat, &net, "4:", at, true) - DECK - height_on(&flat, &net, "1:", at, false);
    assert!(gap < 1.0, "{gap}");
    let (p, _) = clear(&net, &graph, &hm, &junctions, DECK);
    let low = height_on(&p, &net, "1:", at, false);
    let deck = height_on(&p, &net, "4:", at, true);
    assert!(
        deck - DECK - low >= BRIDGE_CLEARANCE_M - CLEARANCE_TOLERANCE_M,
        "clearance {}",
        deck - DECK - low
    );
    assert_eq!(p.clearances.len(), 1);
    assert!(p.clearances[0].reached >= 4.45, "{:?}", p.clearances);
    let strip = bridge_strip(&p, &net);
    for g in grades(&p, strip) {
        assert!(g.abs() <= grade_ceiling(Family::Asf3) + 1e-4, "{g}");
    }
    assert!(
        p.excesses.iter().all(|e| e.kind != GradeExcessKind::Grade),
        "{:?}",
        p.excesses
    );
}

/// The thickness of the deck of [`short_approaches`], in metres.
const THICK: f64 = 3.0;

/// A road "1:1-2" east-west along y = 500 of 12.5 m pieces from x = `start` to `start + 500`, on the ground
/// `f`, and a bridge "4:4-5" over it at x = 300 from y = 487.5 to 512.5. Its approaches are three 12.5 m pieces
/// each, from and to a level ring at 100 m: their ends cannot rise, and in 37.5 m the vertical radius (250 m)
/// lets the deck rise at most 12.5 · (0.175 + 0.125 + 0.075) = 4.69 m, much less than the 4.5 + [`THICK`] m it
/// needs.
fn short_approaches(start: f64) -> (PlacedNetwork, RoadGraph, Junctions) {
    let low = pieces(
        "1:1-2",
        Family::Asf3,
        (start, 500.0),
        (start + 500.0, 500.0),
        12.5,
    );
    let mut up = pieces("a", Family::Asf3, (300.0, 450.0), (300.0, 487.5), 12.5);
    up.push(piece(
        "4:4-5",
        0,
        PlacedKind::Bridge,
        Family::Asf3,
        (300.0, 487.5),
        (300.0, 512.5),
    ));
    up.extend(pieces(
        "c",
        Family::Asf3,
        (300.0, 512.5),
        (300.0, 550.0),
        12.5,
    ));
    let graph = build_graph(&[
        way(1, &[1, 2], &[(start, 500.0), (start + 500.0, 500.0)], false),
        way(4, &[4, 5], &[(300.0, 487.5), (300.0, 512.5)], true),
    ]);
    let junctions = Junctions {
        rings: vec![
            level_ring("r1", (300.0, 440.0), 100.0),
            level_ring("r2", (300.0, 560.0), 100.0),
        ],
        points: vec![ring_held(450.0, "a", "r1"), ring_held(550.0, "c", "r2")],
        ..Junctions::default()
    };
    (network(vec![low, up]), graph, junctions)
}

#[test]
fn a_bridge_and_the_road_below_share_the_lift() {
    let hm = terrain(250, 4.0, |_, _| 100.0);
    let (net, graph, junctions) = short_approaches(50.0);
    let (p, _) = clear(&net, &graph, &hm, &junctions, THICK);
    let at = (300.0, 500.0);
    let deck = height_on(&p, &net, "4:", at, true);
    let low = height_on(&p, &net, "1:", at, false);
    assert_eq!(p.clearances.len(), 1);
    // the deck alone cannot make it within the vertical radius: the road below dips under the bridge
    assert!(
        p.clearances[0].reached >= BRIDGE_CLEARANCE_M - 0.05,
        "reached {}, deck {deck}, road below {low}",
        p.clearances[0].reached
    );
    assert!(deck > 100.5, "deck {deck}");
    assert!(low < 99.5, "road below {low}");
    // both stay inside their grade ceilings and vertical radii
    assert!(p.excesses.is_empty(), "{:?}", p.excesses);
}

/// The clearance enforced where the crossing lies between two joints of the road below (t = 0.3, on a slope) is
/// the one measured on the strips.
#[test]
fn the_clearance_between_two_joints_of_the_road_below_is_the_one_enforced() {
    let hm = terrain(250, 4.0, |x, _| 100.0 + 0.08 * (x - 300.0));
    // joints of the road below at x = 296.25 and 308.75: the crossing at x = 300 is 0.3 of the way
    let (net, graph, junctions) = short_approaches(46.25);
    let (p, _) = clear(&net, &graph, &hm, &junctions, THICK);
    assert_eq!(p.clearances.len(), 1);
    let reached = p.clearances[0].reached;
    // the row is active (the solve lifts and lowers no more than it must), so what is measured is what it holds
    assert!(
        (reached - BRIDGE_CLEARANCE_M).abs() < 0.05,
        "reached {reached}"
    );
    let low = |x: f64| height_on(&p, &net, "1:", (x, 500.0), false);
    let between = 0.7 * low(296.25) + 0.3 * low(308.75);
    let deck = height_on(&p, &net, "4:", (300.0, 500.0), true);
    assert!(
        (deck - THICK - between - reached).abs() < 1e-6,
        "{deck} {between} {reached}"
    );
    // the road below is not flat there: the joint itself would be another number
    assert!((low(296.25) - between).abs() > 0.1);
    assert!(p.excesses.is_empty(), "{:?}", p.excesses);
}

#[test]
fn a_junction_near_the_bridge_rises_with_it() {
    let hm = terrain(250, 4.0, |_, _| 100.0);
    // a side road ends on the approach 30 m before the bridge
    let features = [
        way(1, &[1, 2], &[(50.0, 500.0), (550.0, 500.0)], false),
        way(
            3,
            &[3, 8, 4],
            &[(300.0, 100.0), (300.0, 457.5), (300.0, 487.5)],
            false,
        ),
        way(4, &[4, 5], &[(300.0, 487.5), (300.0, 512.5)], true),
        way(5, &[5, 6], &[(300.0, 512.5), (300.0, 900.0)], false),
        way(6, &[7, 8], &[(400.0, 457.5), (300.0, 457.5)], false),
    ];
    let (net, graph) = placed(&features);
    let before = find_junctions(&net, &graph, &hm);
    // a thick deck: the bridge end needs 100 + 4.5 + 8 = 112.5, more than the grade ceiling lets a junction
    // 30 m away keep without rising
    let (p, raised) = clear(&net, &graph, &hm, &before, 8.0);
    let near = |j: &Junctions| j.near((300.0, 457.5), 6.0).unwrap().clone();
    assert!(near(&before).height < 101.0);
    let junction = near(&raised);
    assert!(junction.height > junction.ground + 1.0, "{junction:?}");
    let deck = height_on(&p, &net, "4:", (300.0, 487.5), true);
    let grade = (deck - junction.height) / 30.0;
    assert!(grade <= grade_ceiling(Family::Asf3) + 1e-4, "{grade}");
    assert!(p.clearances[0].reached >= 4.45, "{:?}", p.clearances);
}

/// A bridge deck "4:4-5" over a road "1:1-2", both of 12.5 m pieces so that a joint lies at the crossing.
fn crossing_network() -> (PlacedNetwork, RoadGraph) {
    let low = pieces("1:1-2", Family::Asf3, (50.0, 500.0), (550.0, 500.0), 12.5);
    let mut up = pieces("up", Family::Asf3, (300.0, 100.0), (300.0, 487.5), 12.5);
    up.push(piece(
        "4:4-5",
        0,
        PlacedKind::Bridge,
        Family::Asf3,
        (300.0, 487.5),
        (300.0, 512.5),
    ));
    up.extend(pieces(
        "on",
        Family::Asf3,
        (300.0, 512.5),
        (300.0, 900.0),
        12.5,
    ));
    let graph = build_graph(&[
        way(1, &[1, 2], &[(50.0, 500.0), (550.0, 500.0)], false),
        way(4, &[4, 5], &[(300.0, 487.5), (300.0, 512.5)], true),
    ]);
    (network(vec![low, up]), graph)
}

#[test]
fn a_deck_and_the_road_below_on_one_junction_is_a_finding() {
    let hm = terrain(250, 4.0, |_, _| 100.0);
    let (net, graph) = crossing_network();
    // one junction holds the joint of the road below and the deck
    let junctions = Junctions {
        points: vec![Junction {
            at: (300.0, 500.0),
            kind: JunctionKind::Join,
            ground: 100.0,
            height: 100.0,
            rank: 3,
            patch_m: 8.0,
            stretches: BTreeSet::new(),
            lower: BTreeSet::new(),
            gradient: (0.0, 0.0),
            tilt_axis: None,
            tilt_cap: 0.0,
        }],
        ..Junctions::default()
    };
    let (p, _) = clear(&net, &graph, &hm, &junctions, DECK);
    assert_eq!(p.clearances.len(), 1);
    // the deck and the road below are one height there: no lift of either makes room, the whole clearance and
    // the thickness of the deck are short, and the roads stay drivable
    let reached = p.clearances[0].reached;
    assert!((reached + DECK).abs() < 1e-6, "{:?}", p.clearances);
    assert!(
        p.excesses.iter().all(|e| e.kind != GradeExcessKind::Grade),
        "{:?}",
        p.excesses
    );
    for strip in 0..p.strips.len() {
        for g in grades(&p, strip) {
            assert!(g.abs() <= grade_ceiling(Family::Asf3) + 1e-4, "{g}");
        }
    }
}

#[test]
fn a_deck_that_a_ring_holds_is_cleared_by_lowering_the_road_below() {
    let hm = terrain(250, 4.0, |_, _| 100.0);
    let (net, graph) = crossing_network();
    let junctions = Junctions {
        rings: vec![level_ring("4:4-5", (300.0, 500.0), 100.0)],
        ..Junctions::default()
    };
    let (p, _) = clear(&net, &graph, &hm, &junctions, DECK);
    assert_eq!(p.clearances.len(), 1);
    // the deck cannot rise; the road below is free, and dips the whole 6 m within its limits
    let at = (300.0, 500.0);
    assert!((height_on(&p, &net, "4:", at, true) - 100.0).abs() < 1e-6);
    assert!(p.clearances[0].reached >= 4.45, "{:?}", p.clearances);
    assert!(height_on(&p, &net, "1:", at, false) < 100.0 - 5.9);
    assert!(p.excesses.is_empty(), "{:?}", p.excesses);
}

#[test]
fn a_road_below_that_a_ring_holds_is_cleared_by_the_deck() {
    let hm = terrain(250, 4.0, |_, _| 100.0);
    let (net, graph) = crossing_network();
    let junctions = Junctions {
        rings: vec![level_ring("1:1-2", (300.0, 500.0), 100.0)],
        ..Junctions::default()
    };
    let (p, _) = clear(&net, &graph, &hm, &junctions, DECK);
    assert_eq!(p.clearances.len(), 1);
    assert!(p.clearances[0].reached >= 4.45, "{:?}", p.clearances);
    assert!((height_on(&p, &net, "1:", (300.0, 500.0), false) - 100.0).abs() < 1e-6);
}

#[test]
fn an_unknown_thickness_is_not_checked() {
    let hm = terrain(250, 4.0, |_, _| 100.0);
    let (net, graph) = placed(&bridge_features());
    let junctions = find_junctions(&net, &graph, &hm);
    let side = side_slopes(&net, &hm);
    let plain = solve_plain(&net, &hm, &junctions, &side).unwrap();
    let unchecked = solve_clearing(&net, &hm, &junctions, &side, &graph, &|_| None).unwrap();
    assert_eq!(plain, unchecked);
}

#[test]
fn a_crossing_whose_road_below_is_not_placed_is_not_checked() {
    // the graph has the bridge over a road, the network only the bridge's road: no row, no clearance
    let hm = terrain(250, 4.0, |_, _| 100.0);
    let (net, graph, junctions) = short_approaches(50.0);
    let upper_only = PlacedNetwork {
        roads: net.roads.clone(),
        chains: net.chains[1..].to_vec(),
        ..net.clone()
    };
    let (p, _) = clear(&upper_only, &graph, &hm, &junctions, THICK);
    assert!(p.clearances.is_empty(), "{:?}", p.clearances);
    let (plain, _) = solve_plain(&upper_only, &hm, &junctions, &[]).unwrap();
    assert_eq!(p.strips, plain.strips);
}

#[test]
fn the_clearing_solve_is_bit_identical() {
    let hm = terrain(250, 4.0, |_, _| 100.0);
    let (net, graph) = placed(&bridge_features());
    let junctions = find_junctions(&net, &graph, &hm);
    let a = clear(&net, &graph, &hm, &junctions, DECK);
    let b = clear(&net, &graph, &hm, &junctions, DECK);
    assert_eq!(a, b);
}

// ---- roads that share terrain cells (rows N) ----

/// Two straight asf3 chains going north, `apart` metres from each other (x = 300 and 300 + apart): the first
/// from y = 150 to 250, the second longer, from 100 to 300, so that no end of it lies on the first (the end of a
/// chain on another road takes its height).
fn side_by_side(apart: f64) -> PlacedNetwork {
    network(vec![
        pieces("a", Family::Asf3, (300.0, 150.0), (300.0, 250.0), 25.0),
        pieces(
            "b",
            Family::Asf3,
            (300.0 + apart, 100.0),
            (300.0 + apart, 300.0),
            25.0,
        ),
    ])
}

/// The heights of both strips where they are side by side, `(a, b)` per joint of the first.
fn joint_pairs(p: &Profiles) -> Vec<(f64, f64)> {
    p.strips[0]
        .joints
        .iter()
        .map(|a| {
            let b = p.strips[1]
                .joints
                .iter()
                .find(|b| (b.1 - a.1).abs() < 1e-6)
                .unwrap();
            (a.2, b.2)
        })
        .collect()
}

#[test]
fn two_legs_of_a_hairpin_share_one_height() {
    // 4 m apart, the ground 0.6 m higher at the second
    let hm = terrain(160, 4.0, |x, _| 100.0 + 0.15 * (x - 300.0));
    let net = side_by_side(4.0);
    let p = solve_alone(&net, &hm);
    let pairs = joint_pairs(&p);
    assert_eq!(pairs.len(), 5);
    for (a, b) in pairs {
        assert!((a - b).abs() <= 0.1 + 0.05, "{a} {b}");
    }
}

#[test]
fn roads_side_by_side_on_a_hillside_stay_apart() {
    // the ground 4 m higher at the second road, over the whole run
    let hm = terrain(160, 4.0, |x, _| 100.0 + (x - 300.0));
    let net = side_by_side(4.0);
    let p = solve_alone(&net, &hm);
    for strip in 0..2 {
        for o in offsets(&p, strip, &hm) {
            assert!(o.abs() <= 0.5, "strip {strip}: {o}");
        }
    }
}

#[test]
fn the_pairs_of_roads_side_by_side_are_counted() {
    let count = |slope: f64| {
        let hm = terrain(160, 4.0, move |x, _| 100.0 + slope * (x - 300.0));
        nearby_pairs(
            &side_by_side(4.0),
            &hm,
            &Junctions::default(),
            &RoadGraph::default(),
        )
        .unwrap()
    };
    // the first road has 37 spots, each the nearest of the other road's spots to it: 1024 close pairs, 37 mutual
    let gentle = count(0.15);
    assert_eq!((gentle.close, gentle.mutual, gentle.rows), (1024, 37, 37));
    // the ground 4 m apart between the roads: the pairs are found, the run is not tied
    let steep = count(1.0);
    assert_eq!((steep.close, steep.mutual, steep.rows), (1024, 37, 0));
}

#[test]
fn one_untied_pair_unties_the_whole_run() {
    // 0.6 m apart in the ground, but 3 m in the middle third of the run (y = 150 to 250)
    let hm = terrain(160, 4.0, |x, y| {
        let step = if (183.0..=217.0).contains(&y) {
            3.0
        } else {
            0.6
        };
        100.0 + step / 4.0 * (x - 300.0)
    });
    let net = side_by_side(4.0);
    let p = solve_alone(&net, &hm);
    // the first lies on its ground; the second cannot follow the bump in the middle, but is free at the ends
    for o in offsets(&p, 0, &hm) {
        assert!(o.abs() <= 0.5, "first: {o}");
    }
    for (joint, o) in p.strips[1].joints.iter().zip(offsets(&p, 1, &hm)) {
        if joint.1 <= 150.0 || joint.1 >= 250.0 {
            assert!(o.abs() <= 0.5, "second at {}: {o}", joint.1);
        }
    }
    let (a, b) = joint_pairs(&p)[0];
    assert!((a - b).abs() >= 0.5, "the ends are tied: {a} {b}");
}

#[test]
fn roads_further_apart_may_differ_by_the_fill_slope() {
    // 15 m apart on a 2 m grid: beyond the footprints (7 m and three cells), so the step a cell can hold,
    // 0.1 + (15 - 13 + 2) * FILL_SLOPE_MAX = 4.1 m, is allowed; the ground is 5 m higher at the second road
    let hm = terrain(160, 2.0, |x, _| 100.0 + (x - 300.0) / 3.0);
    let net = side_by_side(15.0);
    let p = solve_alone(&net, &hm);
    let allowed = 0.1 + (15.0 - 13.0 + 2.0) * FILL_SLOPE_MAX;
    for (a, b) in joint_pairs(&p) {
        assert!((a - b).abs() <= allowed + 0.05, "{a} {b}");
        assert!((a - b).abs() >= allowed - 0.5, "{a} {b}");
    }
}

#[test]
fn a_bridge_and_the_road_below_are_not_tied() {
    // the approach and the road below are metres apart in the plan and the deck has to stand 4.5 m over it
    let hm = terrain(250, 4.0, |_, _| 100.0);
    let (net, graph) = placed(&bridge_features());
    let junctions = find_junctions(&net, &graph, &hm);
    let (p, _) = clear(&net, &graph, &hm, &junctions, DECK);
    assert_eq!(p.clearances.len(), 1);
    assert!(p.clearances[0].reached >= 4.45, "{:?}", p.clearances);
}

#[test]
fn a_bridge_of_unknown_thickness_is_not_tied_either() {
    // the ground under the road below is 1 m above the ground at the end of the approach: they could be tied
    // (the ground allows it), and are not near the crossing
    let hm = terrain(250, 4.0, |_, y| {
        100.0 + ((y - 482.0) / 18.0).clamp(0.0, 1.0)
    });
    let (net, graph) = placed(&bridge_features());
    let junctions = find_junctions(&net, &graph, &hm);
    let side = side_slopes(&net, &hm);
    let (p, _) = solve_clearing(&net, &hm, &junctions, &side, &graph, &|_| None).unwrap();
    let low = height_on(&p, &net, "1:", (300.0, 500.0), false);
    let approach = height_on(&p, &net, "3:", (300.0, 487.5), false);
    assert!(low - approach >= 0.4, "{low} {approach}");
}

#[test]
fn a_side_road_end_is_not_tied_to_the_road_it_meets() {
    // no junction: a side road starts on the edge of a through road that descends 5.8 %; the pairs of the two
    // near the meeting must not flatten the through road
    let hm = terrain(200, 6.25, |_, y| 100.0 - 0.058 * y);
    let through = pieces("t", Family::Asf3, (300.0, 200.0), (300.0, 800.0), 25.0);
    let side = pieces("s", Family::Mud, (303.5, 500.0), (500.0, 500.0), 25.0);
    let net = network(vec![through, side]);
    let p = solve_alone(&net, &hm);
    for (k, g) in grades(&p, 0).iter().enumerate() {
        let y = p.strips[0].joints[k].1;
        if (440.0..=560.0).contains(&y) {
            assert!((g + 0.058).abs() <= 0.005, "grade {g} at y = {y}");
        }
    }
}

/// The height of a strip at `y` (the strips run north), by linear interpolation between its joints.
fn height_at_y(p: &Profiles, strip: usize, y: f64) -> f64 {
    let w = p.strips[strip]
        .joints
        .windows(2)
        .find(|w| w[0].1 <= y && y <= w[1].1)
        .unwrap();
    w[0].2 + (w[1].2 - w[0].2) * (y - w[0].1) / (w[1].1 - w[0].1)
}

#[test]
fn roads_from_a_junction_running_side_by_side_are_tied_abreast() {
    // a junction piece on a road going north, and a second road that leaves its port, turns north and runs on beside it,
    // 12.5 m away, the ground 0.6 m higher at the second: tied abreast from the junction on
    let hm = terrain(200, 4.0, |x, _| 100.0 + 0.6 / 12.5 * (x - 300.0));
    let mut north = vec![piece(
        "t",
        0,
        PlacedKind::Junction,
        Family::Asf3,
        (300.0, 100.0),
        (300.0, 112.5),
    )];
    north.extend(pieces(
        "t",
        Family::Asf3,
        (300.0, 112.5),
        (300.0, 400.0),
        25.0,
    ));
    let mut side = vec![piece(
        "s",
        0,
        PlacedKind::Piece,
        Family::Asf3,
        (306.25, 106.25),
        (312.5, 106.25),
    )];
    side.extend(pieces(
        "s",
        Family::Asf3,
        (312.5, 106.25),
        (312.5, 400.0),
        25.0,
    ));
    let net = network(vec![north, side]);
    let mut junction = piece_junction(300.0, 100.0);
    junction.at = (300.0, 106.25);
    junction.tilt_axis = Some((0.0, 1.0));
    junction.stretches = ["t", "s"].iter().map(|s| (*s).to_string()).collect();
    let junctions = Junctions {
        points: vec![junction],
        ..Junctions::default()
    };
    let (p, _) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    let held = held_joints(&net, &p, &hm, &junctions);
    assert!(held.len() >= 3, "{held:?}");
    for k in 0..12 {
        let y = 130.0 + 15.0 * f64::from(k);
        let (ha, hb) = (height_at_y(&p, 0, y), height_at_y(&p, 1, y));
        assert!(ha.is_finite());
        assert!((ha - hb).abs() <= 0.1 + 0.05, "y = {y}: {ha} {hb}");
    }
}

#[test]
fn a_shallow_merge_does_not_flatten_the_road_it_joins() {
    // no junction: a side road leaves the edge of a through road that descends 5.8 % and runs on beside it,
    // slowly diverging, so that its first spots take the through road's height and lie within a footprint of
    // through-road spots at other places on the slope
    let hm = terrain(400, 2.0, |_, y| 100.0 - 0.058 * y);
    let through = pieces("t", Family::Asf3, (300.0, 200.0), (300.0, 800.0), 25.0);
    let side = pieces("s", Family::Mud, (303.5, 500.0), (316.0, 600.0), 10.0);
    let net = network(vec![through, side]);
    let p = solve_alone(&net, &hm);
    // only abreast pairs pull, a little (0.04 m at most; with every close pair kept, diagonal ones too, it was left 1.965 m off the ground)
    let worst = offsets(&p, 0, &hm)
        .into_iter()
        .fold(0.0_f64, |m, o| m.max(o.abs()));
    assert!(worst <= 0.1, "the through road is {worst} m off the ground");
}

#[test]
fn a_road_entering_a_tilted_roundabout_is_not_tied_to_its_plane() {
    // a scenario check: with only mutually nearest spots paired, no spot of the arm beyond the entry is paired
    // with the ring
    // a ring chain going north whose plane rises 3 % to the north, and a road that leaves it to the east on ground
    // that falls 2 % in that direction: beside the ring, its spots lie within a footprint of ring spots at other
    // places on the plane, and must not be tied to them
    let hm = terrain(300, 4.0, |x, y| {
        100.0 + 0.03 * (y - 150.0) - 0.02 * (x - 300.0)
    });
    let ring = pieces("r", Family::Asf3, (300.0, 100.0), (300.0, 200.0), 25.0);
    let arm = pieces("a", Family::Asf3, (300.0, 150.0), (500.0, 150.0), 25.0);
    let net = network(vec![ring, arm]);
    let mut plane = level_ring("r", (300.0, 150.0), 100.0);
    plane.gradient = (0.0, 0.03);
    let mut join = ring_held(150.0, "a", "r");
    join.at = (300.0, 150.0);
    let junctions = Junctions {
        points: vec![join],
        rings: vec![plane],
        ..Junctions::default()
    };
    let (p, _) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    for (joint, o) in p.strips[1].joints.iter().zip(offsets(&p, 1, &hm)) {
        assert!(
            o.abs() <= 0.1,
            "the arm is {o} m off the ground at {joint:?}"
        );
    }
}

#[test]
fn a_loop_that_leaves_and_re_enters_a_junction_is_not_tied_to_itself_there() {
    // one chain that goes north, turns and comes back 10 m beside itself to the same junction: its two legs
    // share cells, and the junction plane (held at both ends) tilts with the ground, 6 % to the east
    let hm = terrain(300, 4.0, |x, _| 100.0 + 0.06 * (x - 300.0));
    let mut chain = pieces("l", Family::Asf3, (300.0, 100.0), (300.0, 300.0), 25.0);
    chain.extend(pieces(
        "l",
        Family::Asf3,
        (300.0, 300.0),
        (310.0, 300.0),
        10.0,
    ));
    chain.extend(pieces(
        "l",
        Family::Asf3,
        (310.0, 300.0),
        (310.0, 100.0),
        25.0,
    ));
    let net = network(vec![chain]);
    let mut junction = ring_held(100.0, "l", "l");
    junction.at = (305.0, 100.0);
    junction.patch_m = 8.0;
    junction.ground = 100.0 + 0.06 * 5.0;
    junction.height = junction.ground;
    junction.tilt_axis = Some((1.0, 0.0));
    junction.tilt_cap = 0.08;
    junction.gradient = (0.06, 0.0);
    junction.stretches = std::iter::once("l".to_string()).collect();
    let junctions = Junctions {
        points: vec![junction],
        ..Junctions::default()
    };
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    let held = held_joints(&net, &p, &hm, &junctions);
    assert_eq!(held.len(), 2, "{held:?}");
    let tilt = solved.points[0].gradient.0;
    assert!((tilt - 0.06).abs() <= 0.01, "{tilt}");
}

#[test]
fn roads_side_by_side_on_a_slope_keep_their_grade() {
    // two roads 4 m apart on ground that descends 5.8 % along both, 0.6 m higher at the second: tied abreast,
    // but not flattened by pairs that compare different places along the slope
    let hm = terrain(160, 4.0, |x, y| 100.0 - 0.058 * y + 0.15 * (x - 300.0));
    let net = side_by_side(4.0);
    let p = solve_alone(&net, &hm);
    for (a, b) in joint_pairs(&p) {
        assert!((a - b).abs() <= 0.1 + 0.05, "{a} {b}");
    }
    for strip in 0..2 {
        let middle: Vec<(f64, f64)> = p.strips[strip]
            .joints
            .iter()
            .filter(|j| (175.0..=225.0).contains(&j.1))
            .map(|j| (j.1, j.2))
            .collect();
        assert!(middle.len() >= 2, "{middle:?}");
        let (y0, h0) = middle[0];
        for (y, h) in middle {
            let line = h0 - 0.058 * (y - y0);
            assert!(
                (h - line).abs() <= 0.2,
                "strip {strip} at {y}: {h} against {line}"
            );
        }
    }
}

/// A short and a long road side by side, 12 m apart (footprints overlap, no end lies on the other road), on
/// ground that descends 3 % along both and is 0.3 m higher at the second. The short one is first, or last.
fn short_and_long(short_first: bool) -> (PlacedNetwork, Heightmap) {
    let hm = terrain(160, 4.0, |x, y| 100.0 - 0.03 * y + 0.025 * (x - 300.0));
    let short = pieces("a", Family::Asf3, (300.0, 150.0), (300.0, 250.0), 25.0);
    let long = pieces("b", Family::Asf3, (312.0, 100.0), (312.0, 300.0), 25.0);
    let chains = if short_first {
        vec![short, long]
    } else {
        vec![long, short]
    };
    (network(chains), hm)
}

#[test]
fn the_tie_does_not_depend_on_the_order_of_the_roads() {
    let (first, hm) = short_and_long(true);
    let (last, _) = short_and_long(false);
    let (p, q) = (solve_alone(&first, &hm), solve_alone(&last, &hm));
    // the short road is strip 0 in the first solve and strip 1 in the second, the long one the other way round
    for (a, b) in [(0, 1), (1, 0)] {
        assert_eq!(p.strips[a].joints.len(), q.strips[b].joints.len());
        for (j, k) in p.strips[a].joints.iter().zip(&q.strips[b].joints) {
            assert!((j.2 - k.2).abs() <= 1e-6, "{j:?} {k:?}");
        }
    }
}

#[test]
fn a_road_is_not_tied_to_the_end_of_a_shorter_road_beside_it() {
    // the long road comes first: beyond the end of the short one it follows its own ground
    let (net, hm) = short_and_long(false);
    let p = solve_alone(&net, &hm);
    for (joint, o) in p.strips[0].joints.iter().zip(offsets(&p, 0, &hm)) {
        if joint.1 < 150.0 - 6.0 || joint.1 > 250.0 + 6.0 {
            assert!(o.abs() <= 0.1, "at {joint:?}: {o} m off the ground");
        }
    }
}

#[test]
fn the_grade_limit_of_a_junction_piece_survives_a_merged_joint_before_it() {
    // an asf3 road up a slope of 8 %, a piece of 0.5 m (its joint is merged into the one before: one variable
    // fewer than roads from there on), then a junction piece whose plane tilts with the ground at 8 %, more than
    // the meeting grade of 6 % that a piece next to a side road has outside two junction heights. The piece
    // lies between its own two held joints, so its limit is the ceiling of the road, and nothing is exceeded.
    let slope = 0.08;
    let hm = terrain(160, 6.25, move |x, _| 100.0 + slope * x);
    let mut through = pieces("t", Family::Asf3, (100.0, 100.0), (300.0, 100.0), 25.0);
    through.push(piece(
        "t",
        20,
        PlacedKind::Piece,
        Family::Asf3,
        (300.0, 100.0),
        (300.5, 100.0),
    ));
    through.push(piece(
        "t|a",
        0,
        PlacedKind::Junction,
        Family::Asf3,
        (300.5, 100.0),
        (312.5, 100.0),
    ));
    through.extend(pieces(
        "t",
        Family::Asf3,
        (312.5, 100.0),
        (512.5, 100.0),
        25.0,
    ));
    let net = network(vec![through]);
    let at = 306.5;
    let mut junction = piece_junction(at, 100.0 + slope * at);
    junction.gradient = (slope, 0.0);
    let junctions = Junctions {
        points: vec![junction],
        ..Junctions::default()
    };
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    assert!(p.excesses.is_empty(), "{:?}", p.excesses);
    // the solve would rather lower the tilt than exceed a limit: with the meeting grade as the limit it ends
    // under 6 % (5.8 %), with the ceiling as the limit near the ground's 8 %
    assert!(solved.points[0].gradient.0 > 0.07, "{:?}", solved.points[0]);
}

#[test]
fn a_junction_on_a_ring_keeps_the_plane_and_the_other_end_takes_the_rest() {
    // 100 m of asf3 road (a ceiling of 27 %, 27 m) from a junction on a ring plane at 100 m to a junction whose
    // ground is 140 m: the ring's junction is not moved (it is a constant of the problem), the other one
    // gives way as far as it takes
    let hm = terrain(128, 3.125, |x, _| if x < 150.0 { 100.0 } else { 140.0 });
    let net = network(vec![pieces(
        "a",
        Family::Asf3,
        (100.0, 100.0),
        (200.0, 100.0),
        12.5,
    )]);
    let junction = |x: f64, ground: f64, stretches: &[&str]| Junction {
        at: (x, 100.0),
        kind: JunctionKind::Join,
        ground,
        height: ground,
        rank: 3,
        patch_m: 1.0,
        stretches: stretches.iter().map(|s| (*s).to_string()).collect(),
        lower: BTreeSet::new(),
        gradient: (0.0, 0.0),
        tilt_axis: None,
        tilt_cap: 0.0,
    };
    let junctions = Junctions {
        rings: vec![level_ring("r", (100.0, 100.0), 100.0)],
        points: vec![
            junction(100.0, 100.0, &["a", "r"]),
            junction(200.0, 140.0, &["a"]),
        ],
        ..Junctions::default()
    };
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    assert!((solved.points[0].height - 100.0).abs() < 1e-9, "{solved:?}");
    assert!(solved.points[1].height < 140.0 - 5.0, "{solved:?}");
    let joints = &p.strips[0].joints;
    assert!((joints[0].2 - 100.0).abs() < 1e-9, "{joints:?}");
    assert!(p.excesses.is_empty(), "{:?}", p.excesses);
}

/// A junction of roads that meet without a piece, at `at`, held at `ground`, of the stretches `stretches`.
fn join_at(at: (f64, f64), ground: f64, patch_m: f64, stretches: &[&str]) -> Junction {
    Junction {
        at,
        kind: JunctionKind::Join,
        ground,
        height: ground,
        rank: 3,
        patch_m,
        stretches: stretches.iter().map(|s| (*s).to_string()).collect(),
        lower: BTreeSet::new(),
        gradient: (0.0, 0.0),
        tilt_axis: None,
        tilt_cap: 0.0,
    }
}

/// A road along y = 100 from x = 100: 100 m of road, a deck of `deck` m, 100 m of road, all one chain.
fn road_with_deck(deck: f64) -> Vec<PlacedRoad> {
    let end = 200.0 + deck;
    let mut chain = pieces("a", Family::Asf3, (100.0, 100.0), (200.0, 100.0), 25.0);
    chain.push(piece(
        "b",
        0,
        PlacedKind::Bridge,
        Family::Asf3,
        (200.0, 100.0),
        (end, 100.0),
    ));
    chain.extend(pieces(
        "c",
        Family::Asf3,
        (end, 100.0),
        (end + 100.0, 100.0),
        25.0,
    ));
    chain
}

/// The height of the joint of strip `strip` at `at`.
fn deck_joint(p: &Profiles, strip: usize, at: (f64, f64)) -> f64 {
    p.strips[strip]
        .joints
        .iter()
        .find(|j| (j.0 - at.0).hypot(j.1 - at.1) < 1e-6)
        .unwrap()
        .2
}

#[test]
fn a_long_deck_ending_at_a_junction_is_held_to_its_height() {
    // the deck's middle is 30 m from the junction at its end, far beyond the patch and the reach of the end rule
    let hm = terrain(160, 6.25, |_, _| 100.0);
    let net = network(vec![road_with_deck(60.0)]);
    let junctions = Junctions {
        points: vec![join_at((260.0, 100.0), 106.0, 1.0, &["a", "b", "c"])],
        ..Junctions::default()
    };
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    let deck = deck_joint(&p, 0, (260.0, 100.0));
    assert!(
        (deck - solved.points[0].height).abs() < 1e-6,
        "deck {deck}, junction {}",
        solved.points[0].height
    );
    assert!(
        (deck_joint(&p, 0, (200.0, 100.0)) - deck).abs() < 1e-9,
        "the deck is level"
    );
}

#[test]
fn a_deck_end_on_a_junction_beside_its_middle_takes_the_height_of_the_junction_at_the_end() {
    // the same deck, with the junction's plane tilting along the road: the deck takes the plane's height at
    // its end, not at its middle
    let hm = terrain(160, 6.25, |_, _| 100.0);
    let net = network(vec![road_with_deck(60.0)]);
    let mut junction = join_at((260.0, 100.0), 106.0, 1.0, &["a", "b", "c"]);
    junction.kind = JunctionKind::Piece;
    junction.tilt_axis = Some((1.0, 0.0));
    junction.tilt_cap = 0.05;
    junction.gradient = (0.05, 0.0);
    let junctions = Junctions {
        points: vec![junction],
        ..Junctions::default()
    };
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    let deck = deck_joint(&p, 0, (260.0, 100.0));
    let at_end = solved.points[0].height;
    assert!(
        (deck - at_end).abs() < 1e-6,
        "the deck {deck} against the junction {at_end}"
    );
}

#[test]
fn a_chain_that_continues_from_a_deck_end_takes_the_height_of_the_deck() {
    // the road after the deck is another chain of one short piece, held to a junction 7.5 m from the deck end
    // (inward from its other end, which lies at the junction): the junction does not own the deck, so the end
    // takes the deck's height
    let hm = terrain(160, 6.25, |_, _| 100.0);
    let mut deck = pieces("a", Family::Asf3, (100.0, 100.0), (200.0, 100.0), 25.0);
    deck.push(piece(
        "b",
        0,
        PlacedKind::Bridge,
        Family::Asf3,
        (200.0, 100.0),
        (260.0, 100.0),
    ));
    let after = vec![piece(
        "p",
        0,
        PlacedKind::Piece,
        Family::Asf3,
        (260.0, 100.0),
        (266.2, 100.0),
    )];
    let net = network(vec![deck, after]);
    let junctions = Junctions {
        points: vec![join_at((267.5, 100.0), 106.0, 3.0, &["p"])],
        ..Junctions::default()
    };
    let (p, _) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    let on_deck = deck_joint(&p, 0, (260.0, 100.0));
    let after = deck_joint(&p, 1, (260.0, 100.0));
    assert!(
        (on_deck - after).abs() < 1e-6,
        "deck {on_deck}, road {after}"
    );
}

#[test]
fn a_deck_between_two_junctions_that_want_different_heights_keeps_a_step_at_one_of_them() {
    let hm = terrain(160, 6.25, |_, _| 100.0);
    let net = network(vec![road_with_deck(60.0)]);
    let junctions = Junctions {
        points: vec![
            join_at((200.0, 100.0), 103.0, 1.0, &["a", "b", "c"]),
            join_at((260.0, 100.0), 112.0, 1.0, &["a", "b", "c"]),
        ],
        ..Junctions::default()
    };
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    let deck = deck_joint(&p, 0, (200.0, 100.0));
    let steps: Vec<f64> = solved
        .points
        .iter()
        .map(|j| (j.height - deck).abs())
        .collect();
    // the deck is held to one junction, and the other end keeps what is left, which is more than the report's
    // tolerance
    assert!(steps.iter().any(|s| *s < 1e-6), "{steps:?} at {deck}");
    assert!(steps.iter().any(|s| *s > 0.5), "{steps:?} at {deck}");
}

#[test]
fn a_deck_that_ends_on_an_earlier_road_takes_its_height() {
    // the road runs east along y = 100; the deck starts 2 m north of its centre line and runs away from it; the
    // ground rises to the north, so a deck that is free would stand higher than the road
    let hm = terrain(160, 6.25, |_, y| 100.0 + 0.05 * (y - 100.0));
    let low = pieces("low", Family::Asf3, (100.0, 100.0), (300.0, 100.0), 25.0);
    let deck = vec![
        piece(
            "d",
            0,
            PlacedKind::Bridge,
            Family::Asf3,
            (200.0, 102.0),
            (200.0, 142.0),
        ),
        piece(
            "e",
            0,
            PlacedKind::Piece,
            Family::Asf3,
            (200.0, 142.0),
            (200.0, 200.0),
        ),
    ];
    let net = network(vec![low, deck]);
    let (p, _) = solve_plain(&net, &hm, &Junctions::default(), &[]).unwrap();
    let on_road = deck_joint(&p, 0, (200.0, 100.0));
    let deck = deck_joint(&p, 1, (200.0, 102.0));
    assert!((on_road - deck).abs() < 1e-6, "road {on_road}, deck {deck}");
}

#[test]
fn a_deck_does_not_end_on_the_road_that_passes_under_its_end() {
    // the same road, but it crosses 2.5 m inside the deck's start: it is the road below the deck, which the deck must
    // clear, not a road the deck ends on; the deck keeps its own height (the ground is 0.5 m higher at its middle)
    let hm = terrain(160, 6.25, |_, y| 100.0 + 0.05 * (y - 100.0));
    let low = pieces("low", Family::Asf3, (100.0, 100.0), (300.0, 100.0), 25.0);
    let deck = vec![
        piece(
            "d",
            0,
            PlacedKind::Bridge,
            Family::Asf3,
            (200.0, 97.5),
            (200.0, 137.5),
        ),
        piece(
            "e",
            0,
            PlacedKind::Piece,
            Family::Asf3,
            (200.0, 137.5),
            (200.0, 200.0),
        ),
    ];
    let net = network(vec![low, deck]);
    let (p, _) = solve_plain(&net, &hm, &Junctions::default(), &[]).unwrap();
    let on_road = deck_joint(&p, 0, (200.0, 100.0));
    let deck = deck_joint(&p, 1, (200.0, 97.5));
    assert!(
        deck < on_road - 0.1 || deck > on_road + 0.1,
        "road {on_road}, deck {deck}"
    );
}

/// A road and a deck ending at (260, 100), held to a junction at its start; a one-piece deck chain that starts or
/// ends at the end of that deck, held to a junction of its own beside the other end.
fn two_decks_meeting(reversed: bool) -> (PlacedNetwork, Junctions) {
    let mut a = pieces("a", Family::Asf3, (100.0, 100.0), (200.0, 100.0), 25.0);
    a.push(piece(
        "b",
        0,
        PlacedKind::Bridge,
        Family::Asf3,
        (200.0, 100.0),
        (260.0, 100.0),
    ));
    let (from, to) = if reversed {
        ((260.0, 100.5), (260.0, 160.0))
    } else {
        ((260.0, 160.0), (260.0, 100.5))
    };
    let b = vec![piece("q", 0, PlacedKind::Bridge, Family::Asf3, from, to)];
    let junctions = Junctions {
        points: vec![
            join_at((200.0, 100.0), 106.0, 1.0, &["a", "b"]),
            join_at((262.0, 100.0), 100.0, 3.0, &["q"]),
        ],
        ..Junctions::default()
    };
    (network(vec![a, b]), junctions)
}

#[test]
fn a_chain_that_is_one_deck_takes_the_height_of_an_earlier_deck_at_either_end() {
    let hm = terrain(160, 6.25, |_, _| 100.0);
    for reversed in [true, false] {
        let (net, junctions) = two_decks_meeting(reversed);
        let (p, _) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
        let first = deck_joint(&p, 0, (260.0, 100.0));
        let second = deck_joint(&p, 1, (260.0, 100.5));
        assert!(
            (first - second).abs() < 1e-6,
            "reversed {reversed}: {first} against {second}"
        );
    }
}

/// A junction at (300, `y`) with the patch `patch_m` and `ground`, belonging to the stretches `ids`.
fn junction_on_x300(y: f64, ground: f64, patch_m: f64, ids: &[&str]) -> Junction {
    Junction {
        at: (300.0, y),
        kind: JunctionKind::Piece,
        ground,
        height: ground,
        rank: 3,
        patch_m,
        stretches: ids.iter().map(|s| (*s).to_string()).collect(),
        lower: BTreeSet::new(),
        gradient: (0.0, 0.0),
        tilt_axis: Some((1.0, 0.0)),
        tilt_cap: 0.08,
    }
}

/// A chain along x = 300 that has a deck: a 12.5 m piece "a", a 25 m deck "b", 12.5 m pieces of "c". Going north
/// it starts at y = 300, the deck runs from y = 312.5 to 337.5 and the pieces on to y = 525; `reversed` mirrors
/// it (y to 625 - y, the same chain walked from the north). The start is held by a junction (index 1) 1 m before
/// it, which is nearer than the one (index 0) at `at_y` (north-going numbers), 5 m patch, ground 101: that one is
/// held by no variable, and its patch lies between the start and the deck, 6.2 m from the deck's end if `at_y`
/// is 306.3.
fn junction_on_the_run(at_y: f64, reversed: bool) -> (PlacedNetwork, Junctions) {
    let mirror = |y: f64| if reversed { 625.0 - y } else { y };
    let before = piece(
        "a",
        0,
        PlacedKind::Piece,
        Family::Asf3,
        (300.0, mirror(300.0)),
        (300.0, mirror(312.5)),
    );
    let deck = piece(
        "b",
        0,
        PlacedKind::Bridge,
        Family::Asf3,
        (300.0, mirror(312.5)),
        (300.0, mirror(337.5)),
    );
    let after = pieces(
        "c",
        Family::Asf3,
        (300.0, mirror(337.5)),
        (300.0, mirror(525.0)),
        12.5,
    );
    let chain = if reversed {
        let mut chain: Vec<PlacedRoad> = after
            .iter()
            .rev()
            .map(|r| {
                let mut r = r.clone();
                std::mem::swap(&mut r.start, &mut r.end);
                r
            })
            .collect();
        for mut r in [deck, before] {
            std::mem::swap(&mut r.start, &mut r.end);
            chain.push(r);
        }
        chain
    } else {
        let mut chain = vec![before, deck];
        chain.extend(after);
        chain
    };
    let junctions = Junctions {
        points: vec![
            junction_on_x300(mirror(at_y), 101.0, 5.0, &["a", "b", "c"]),
            junction_on_x300(mirror(299.0), 100.0, 3.0, &["a", "b", "c"]),
        ],
        ..Junctions::default()
    };
    (network(vec![chain]), junctions)
}

/// The height of strip 0 at `y` on the line x = 300, between its joints (in either order).
fn strip_height_at(p: &Profiles, y: f64) -> f64 {
    let joints = &p.strips[0].joints;
    let pair = joints
        .windows(2)
        .find(|w| (w[0].1 - y) * (w[1].1 - y) <= 0.0 && (w[1].1 - w[0].1).abs() > 1e-9)
        .unwrap();
    pair[0].2 + (pair[1].2 - pair[0].2) * (y - pair[0].1) / (pair[1].1 - pair[0].1)
}

#[test]
fn a_deck_whose_junction_lies_beyond_its_patch_is_held_to_it_where_the_strip_crosses() {
    // the junction lies 6.2 m before the deck, 1.2 m outside its 5 m patch, on the strip's run from the held start
    // to the deck: no joint lies in the patch, and the next joint is 12.5 m from the deck's end. Its ground is
    // 1 m above the terrain, so a strip that stays on the terrain is 1 m below its plane. The chain is walked
    // either way: the run is measured from the deck's facing end in both.
    let hm = terrain(250, 4.0, |_, _| 100.0);
    for reversed in [false, true] {
        let m = |y: f64| if reversed { 625.0 - y } else { y };
        let (net, junctions) = junction_on_the_run(306.3, reversed);
        let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
        let plane = solved.points[0].height;
        // the strip is tied to the plane where it enters the patch, 1.2 m from the deck's end (5 m patch around
        // 306.3), where no joint fits between the deck and the patch
        let strip = strip_height_at(&p, m(311.3));
        assert!(
            (strip - plane).abs() < 0.01,
            "reversed {reversed}: the strip is at {strip}, the plane at {plane}"
        );
        // the junction gave way a little to the terrain, the strip came up to the plane
        assert!(plane < 100.9 && strip > 100.1, "{plane} {strip}");
        let steps = naves_core::validate::bridge_end_steps(&net, &p, &solved);
        assert!(
            steps
                .iter()
                .all(|s| s.step.abs() <= naves_core::validate::END_STEP_TOL_M),
            "reversed {reversed}: {steps:?}"
        );
        // the deck stays one level height
        let (start, end) = (joint_at(&p, m(312.5)), joint_at(&p, m(337.5)));
        assert!((start.2 - end.2).abs() < 1e-9, "{start:?} {end:?}");
    }
}

#[test]
fn a_junction_off_the_run_or_that_the_road_does_not_belong_to_holds_nothing() {
    let hm = terrain(250, 4.0, |_, _| 100.0);
    // 8 m beside the strip, outside its 5 m patch
    let (net, mut junctions) = junction_on_the_run(306.3, false);
    junctions.points[0].at = (308.0, 306.3);
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    assert!((solved.points[0].height - 101.0).abs() < 1e-9, "{solved:?}");
    assert!((strip_height_at(&p, 306.3) - 100.0).abs() < 0.01);
    // a junction that belongs to other roads
    let (net, mut junctions) = junction_on_the_run(306.3, false);
    junctions.points[0].stretches = ["x"].map(String::from).into();
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    assert!((solved.points[0].height - 101.0).abs() < 1e-9, "{solved:?}");
    assert!((strip_height_at(&p, 306.3) - 100.0).abs() < 0.01);
}

#[test]
fn a_plain_road_is_held_to_a_junction_it_passes_at_a_joint_of_its_own_not_by_a_crossing_row() {
    // 12.5 m pieces; the junction 3 m from the strip between two joints, 6 m from each, patch 5 m: where the road
    // enters and leaves the patch is 2.3 m from a joint (under RAMP_JOINT_MIN_M) on either side, so none is added and
    // the nearer of the two joints holds the junction, one not both (the crossing rows are for runs that touch a
    // deck and change nothing here)
    let net = network(vec![pieces(
        "a",
        Family::Asf3,
        (300.0, 100.0),
        (300.0, 525.0),
        12.5,
    )]);
    let junctions = Junctions {
        points: vec![{
            let mut j = junction_on_x300(318.75, 101.0, 5.0, &["a"]);
            j.at.0 = 303.0;
            j
        }],
        ..Junctions::default()
    };
    let holds = chain_holds(&net, &net.chains[0], 4.0, &junctions);
    let held: Vec<_> = holds.iter().filter_map(|(_, h)| h.as_ref()).collect();
    assert_eq!(held.len(), 1, "{held:?}");
    assert_eq!(held[0].rule, naves_core::profile::HoldRule::Patch);
}

/// The ring-held version of the run: the junction at (300, `at_y`) lies on a level ring at `ring` metres, so its
/// plane is that height whatever the solve does; the start of the chain is held by a junction on a ring at 100 m.
fn ring_junction_on_the_run(at_y: f64, ring: f64) -> (PlacedNetwork, Junctions) {
    let (net, _) = junction_on_the_run(at_y, false);
    let mut crossing = junction_on_x300(at_y, ring, 5.0, &["a", "b", "c", "r2"]);
    crossing.height = ring;
    let mut start = ring_held(300.0, "a", "r1");
    start.patch_m = 1.0;
    let junctions = Junctions {
        rings: vec![
            level_ring("r1", (300.0, 290.0), 100.0),
            level_ring("r2", (320.0, 303.0), ring),
        ],
        points: vec![crossing, start],
        ..Junctions::default()
    };
    (net, junctions)
}

#[test]
fn a_crossing_hold_the_grade_limits_forbid_is_given_up_and_reported_wherever_the_junction_lies() {
    // a junction on a ring at 105 m lies 3 m from the held start (100 m) and 9.5 m from the deck's end, beyond the
    // patch (5 m) and the reach of the end rule: its plane, 5 m up in 3 m, would need the deck 20 m up, a change of
    // grade at the deck's end far over the bend limit. The solve gives up the hold: that must be a finding.
    let hm = terrain(250, 4.0, |_, _| 100.0);
    let (net, junctions) = ring_junction_on_the_run(303.0, 105.0);
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    let steps = naves_core::validate::bridge_end_steps(&net, &p, &solved);
    let big: Vec<_> = steps
        .iter()
        .filter(|s| s.step.abs() > naves_core::validate::END_STEP_TOL_M)
        .collect();
    assert_eq!(big.len(), 1, "{steps:?}");
    assert!(big[0].what.contains("junction at (300, 303)"), "{big:?}");
    assert!(big[0].step > 2.0, "{big:?}");
}

#[test]
fn a_crossing_hold_that_is_kept_is_not_reported() {
    // the same junction on a ring at 100 m, the strip's own height: nothing to give up
    let hm = terrain(250, 4.0, |_, _| 100.0);
    let (net, junctions) = ring_junction_on_the_run(303.0, 100.0);
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    let steps = naves_core::validate::bridge_end_steps(&net, &p, &solved);
    assert!(
        steps
            .iter()
            .all(|s| s.step.abs() <= naves_core::validate::END_STEP_TOL_M),
        "{steps:?}"
    );
    assert!((strip_height_at(&p, 303.0) - 100.0).abs() < 0.01);
}

#[test]
fn a_crossing_row_does_not_pin_a_run_end_that_another_junction_holds() {
    // a junction 2 m behind the held start of the chain: the run's nearest point to it is the start itself (t = 0),
    // which the junction 1 m from the start already holds, so the first junction is not pinned to it
    let hm = terrain(250, 4.0, |_, _| 100.0);
    let (net, mut junctions) = junction_on_the_run(298.0, false);
    junctions.points[0].patch_m = 5.0;
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    assert!((solved.points[0].height - 101.0).abs() < 1e-9, "{solved:?}");
    assert!((strip_height_at(&p, 300.0) - 100.0).abs() < 0.01);
}

// ---- ramps: a short drivable ramp instead of a step next to junctions and bridge ends ----
// (`docs/superpowers/specs/2026-11-18-ramps-design.md`)

/// A level deck on x = 300 from y = 300 to 325 that a ring-held junction at its start holds at 100 m, 12.5 m pieces
/// before it, one 17 m piece after it (to y = 342) and 12.5 m pieces on to y = 530. A second junction at
/// (300, 330.7), 5.7 m beyond the deck's end, with the patch `patch_m`, lies on a level ring `rise` metres above the
/// deck: its height is fixed, no joint lies in its patch, and the next joint is 17 m from the deck's end (the shape
/// at way 74208396).
fn ramp_case(rise: f64, patch_m: f64) -> (PlacedNetwork, Junctions) {
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
        (300.0, 342.0),
        17.0,
    ));
    chain.extend(pieces(
        "c",
        Family::Asf3,
        (300.0, 342.0),
        (300.0, 530.0),
        12.5,
    ));
    let mut ahead = ring_held(330.7, "c", "r2");
    ahead.patch_m = patch_m;
    ahead.stretches.insert("b".to_string());
    ahead.ground = 100.0 + rise;
    ahead.height = 100.0 + rise;
    let junctions = Junctions {
        rings: vec![
            level_ring("r1", (300.0, 300.0), 100.0),
            level_ring("r2", (320.0, 330.7), 100.0 + rise),
        ],
        points: vec![ahead, ring_held(300.0, "b", "r1")],
        ..Junctions::default()
    };
    (network(vec![chain]), junctions)
}

fn flat_100() -> Heightmap {
    terrain(250, 4.0, |_, _| 100.0)
}

#[test]
fn a_level_deck_next_to_a_junction_that_is_a_little_higher_ends_in_a_ramp_not_a_step() {
    // the junction's plane is 0.87 m above the deck (the deck is held at 100 m); its patch is 2 m, so the road gets
    // one joint at its centre (the road is in the patch for 3.9 m, less than RAMP_JOINT_MIN_M, which is one joint),
    // 5.7 m from the deck's end: a 15 % ramp, under the 27 % of an asphalt road, entered by a break of grade that is
    // over the limit of a ramp (see below)
    let hm = flat_100();
    let (net, junctions) = ramp_case(0.87, 2.0);
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    let plane = 100.87;
    // on the junction's plane inside the patch
    let strip = strip_height_at(&p, 330.7);
    assert!((strip - plane).abs() < 0.01, "{strip} against {plane}");
    // the ramp between the deck's end and the patch is within the family's ceiling
    let ceiling = grade_ceiling(Family::Asf3);
    for w in p.strips[0].joints.windows(2) {
        let run = (w[1].1 - w[0].1).abs();
        if run > 1e-6 {
            let grade = ((w[1].2 - w[0].2) / run).abs();
            assert!(grade <= ceiling + 1e-3, "{w:?}: {grade}");
        }
    }
    // What comes out is not a clean ramp, and this test says so. The 0.87 m is closed in 5.7 m, a 15.3 % grade, and the
    // change of grade where the level deck ends is 15.3 %: over GRADE_BREAK_MAX, so a `GradeBreak` finding at the
    // deck's end (measured 0.1526, the limit 0.1). That is the price of keeping the junction's plane; the findings say
    // it. Beyond the junction the road does not come down at once: a break over GRADE_BREAK_MAX at the patch would be
    // a second finding, and the smoothing (30, which this change does not touch) prefers a long crest. Measured:
    // 1.16 m over the plane at the next joint (342), 1.62 m at 353.75 (the crest, 102.49 m), and 100.50 m again only
    // 94 m after the patch. The pin on the line to the next joint made 2.4 m. The bounds are the measured values
    // plus a margin of 0.1 m.
    let breaks: Vec<_> = p
        .excesses
        .iter()
        .filter(|e| e.kind == GradeExcessKind::GradeBreak)
        .collect();
    assert_eq!(breaks.len(), 1, "{:?}", p.excesses);
    assert!(
        (breaks[0].at.1 - 325.0).abs() < 1e-6
            && (0.15..0.16).contains(&breaks[0].grade)
            && (breaks[0].limit - GRADE_BREAK_MAX).abs() < 1e-9,
        "{breaks:?}"
    );
    let far = joint_at(&p, 342.0);
    assert!(far.2 <= plane + 1.26, "{far:?}");
    let crest = p.strips[0]
        .joints
        .iter()
        .filter(|j| j.1 > 330.0)
        .map(|j| j.2)
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(crest <= plane + 1.72, "the crest is {crest}");
    // the deck is still level and where it was held
    let (start, end) = (joint_at(&p, 300.0), joint_at(&p, 325.0));
    assert!((start.2 - 100.0).abs() < 1e-6 && (end.2 - 100.0).abs() < 1e-6);
    let steps = naves_core::validate::bridge_end_steps(&net, &p, &solved);
    assert!(
        steps
            .iter()
            .all(|s| s.step.abs() <= naves_core::validate::END_STEP_TOL_M),
        "{steps:?}"
    );
}

#[test]
fn a_step_between_two_fixed_heights_at_a_deck_end_is_reported() {
    // the patch is 5 m: the road enters it 0.75 m after the deck's end, under RAMP_JOINT_MIN_M from the deck (which
    // is one level height and cannot hold the junction's plane), so no joint is added there; where it leaves the
    // patch (335.65) a joint is added and held, and the 0.87 m is closed by an 8 % ramp across the patch
    let hm = flat_100();
    let (net, junctions) = ramp_case(0.87, 5.0);
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    let ys: Vec<f64> = p.strips[0].joints.iter().map(|j| j.1).collect();
    assert!(ys.iter().any(|y| (y - 335.65).abs() < 0.1), "{ys:?}");
    assert!((strip_height_at(&p, 335.65) - 100.87).abs() < 0.01);
    let grades = grades(&p, 0);
    assert!(
        grades
            .iter()
            .all(|g| g.abs() <= grade_ceiling(Family::Asf3) + 1e-3),
        "{grades:?}"
    );
    // the deck's height and the junction's plane are both fixed here (rings), so the 0.87 m cannot be closed at the
    // patch's edge, 0.7 m after the deck's end: the report says so (the step is measured where the road enters the
    // patch, not at the middle of it, where the ramp has already come up half way)
    let steps = naves_core::validate::bridge_end_steps(&net, &p, &solved);
    let big: Vec<_> = steps
        .iter()
        .filter(|s| s.step.abs() > naves_core::validate::END_STEP_TOL_M)
        .collect();
    assert_eq!(big.len(), 1, "{steps:?}");
    assert!((big[0].step - 0.81).abs() < 0.05, "{big:?}");
}

#[test]
fn a_height_difference_that_needs_more_than_the_ceiling_is_reported_not_hidden() {
    // 3 m in 3.7 m is 81 %: no ramp makes that, so a step or a grade over the ceiling is in the findings
    let hm = flat_100();
    let (net, junctions) = ramp_case(3.0, 2.0);
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    let steps = naves_core::validate::bridge_end_steps(&net, &p, &solved);
    let step = steps
        .iter()
        .any(|s| s.step.abs() > naves_core::validate::END_STEP_TOL_M);
    let grade = p.excesses.iter().any(|e| e.kind == GradeExcessKind::Grade);
    assert!(step || grade, "{steps:?} {:?}", p.excesses);
}

#[test]
fn a_kept_hold_beats_a_grade_break_over_the_limit_and_the_break_is_reported() {
    // 0.6 m in 3.7 m is a 16 % ramp: both its ends break the grade by more than GRADE_BREAK_MAX, which costs less
    // than giving the hold up, so the road is on the plane and the breaks are findings of their own kind
    let hm = flat_100();
    let (net, junctions) = ramp_case(0.6, 2.0);
    let (p, _) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    let strip = strip_height_at(&p, 330.7);
    assert!((strip - 100.6).abs() < 0.01, "{strip}");
    let breaks: Vec<_> = p
        .excesses
        .iter()
        .filter(|e| e.kind == GradeExcessKind::GradeBreak)
        .collect();
    assert!(!breaks.is_empty(), "{:?}", p.excesses);
    for b in breaks {
        assert!(
            b.grade > GRADE_BREAK_MAX && (b.limit - GRADE_BREAK_MAX).abs() < 1e-9,
            "{b:?}"
        );
    }
}

/// The shape of way 74208396 with junctions that can move: the chain of [`ramp_case`], but the deck's own junction at
/// y = 300 (index 1, 3 m patch, ground 0.9 m below the terrain) and the one 5.7 m beyond the deck's end (index 0,
/// patch 5 m, no joint of the road within 4 m of where the road enters it, a plane 0.78 m above the deck at the port
/// that tilts 6 % down away from the deck) are on soft planes, each pulled to its ground. The terrain is level at 100.
fn tilted_neighbour_case() -> (PlacedNetwork, Junctions) {
    let (net, _) = ramp_case(0.0, 5.0);
    let mut near = join_at((300.0, 330.7), 100.48, 5.0, &["b", "c"]);
    near.gradient = (0.0, -0.06);
    near.tilt_axis = Some((0.0, 1.0));
    near.tilt_cap = 0.08;
    let far = join_at((300.0, 300.0), 99.1, 3.0, &["a", "b"]);
    let junctions = Junctions {
        points: vec![near, far],
        ..Junctions::default()
    };
    (net, junctions)
}

#[test]
fn a_deck_end_is_tied_to_the_plane_of_the_junction_whose_patch_starts_next_to_it() {
    // the patch starts 0.7 m after the deck's end, so no joint fits between them; the junction's plane at the port is
    // 0.78 m above the deck. Before the row that ties the deck's end to that plane nothing in the solve compared the
    // two (the strip ended 0.66 m below the plane). Now a junction that is less than a metre off its ground gives
    // way instead of leaving a step, the junction at the deck's other end stays under a metre off its ground, and no
    // road is over its ceiling
    let hm = flat_100();
    let (net, junctions) = tilted_neighbour_case();
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    let entry = 325.7;
    let strip = strip_height_at(&p, entry);
    let plane = solved.points[0].height_at((300.0, entry));
    assert!(
        (strip - plane).abs() < 0.1,
        "the strip is at {strip}, the plane at {plane}"
    );
    let far = solved.points[1].height - junctions.points[1].ground;
    assert!(
        far.abs() < 1.0,
        "the far junction is {far} m off its ground"
    );
    // measured: the near junction comes down to the deck (0.5 m below its ground, its tilt flattens), the far one is
    // 0.92 m above its ground
    let near = solved.points[0].height - junctions.points[0].ground;
    assert!(
        (-0.7..-0.3).contains(&near),
        "the near junction is {near} m off its ground"
    );
    let ceiling = grade_ceiling(Family::Asf3);
    for g in grades(&p, 0) {
        assert!(g.abs() <= ceiling + 1e-3, "{g}");
    }
    let steps = naves_core::validate::bridge_end_steps(&net, &p, &solved);
    assert!(steps.iter().all(|s| s.step.abs() < 0.1), "{steps:?}");
}

/// The chain of `deck_chain` on a ring whose plane rises `grade` to the north: the level deck makes a break of
/// `grade` at each of its ends.
fn deck_on_plane(grade: f64) -> Profiles {
    let hm = terrain(128, 6.25, |_, _| 100.0);
    let net = network(vec![deck_chain()]);
    let plane = RingPlane {
        at: (300.0, 300.0),
        radius: 10.0,
        inner_radius: 3.0,
        height: 160.0,
        gradient: (0.0, grade),
        rank: 3,
        stretches: ["a", "b", "c"].iter().map(|s| (*s).to_string()).collect(),
    };
    let junctions = Junctions {
        rings: vec![plane],
        ..Junctions::default()
    };
    solve_plain(&net, &hm, &junctions, &[]).unwrap().0
}

#[test]
fn a_grade_break_of_8_percent_next_to_a_deck_end_is_no_finding_but_14_percent_is() {
    // the deck is one level height, held at the plane's height at its middle: on a plane that rises `g` the road
    // meets it at 2 g, so each deck end breaks the grade by 2 g and the joints beside the deck by g. The radius of an
    // asphalt road allows 7.5 % over the 18.75 m run of a deck end; inside the ramp zone the limit is 10 %.
    let eight = deck_on_plane(0.04);
    assert!(eight.excesses.is_empty(), "{:?}", eight.excesses);
    let fourteen = deck_on_plane(0.07);
    assert_eq!(fourteen.excesses.len(), 2, "{:?}", fourteen.excesses);
    for e in &fourteen.excesses {
        assert_eq!(e.kind, GradeExcessKind::GradeBreak);
        assert!((e.grade - 0.14).abs() < 1e-6 && (e.limit - GRADE_BREAK_MAX).abs() < 1e-9);
    }
}

/// A 25 m-piece road along x = 300 from y = 100 to 300 (joints every 25 m) and a junction of a junction piece
/// at `(300, y)` with the patch `patch_m` at the ground height.
fn passing_junction(y: f64, patch_m: f64) -> (PlacedNetwork, Junctions) {
    let net = network(vec![pieces(
        "a",
        Family::Asf3,
        (300.0, 100.0),
        (300.0, 300.0),
        25.0,
    )]);
    let junctions = Junctions {
        points: vec![junction_on_x300(y, 100.0, patch_m, &["a"])],
        ..Junctions::default()
    };
    (net, junctions)
}

#[test]
fn a_road_that_passes_through_a_patch_without_a_joint_gets_joints_where_it_enters_and_leaves() {
    let hm = flat_100();
    let (net, junctions) = passing_junction(137.5, 3.0);
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    // nine joints and two more, 3 m either side of the junction (a little inside the patch)
    let ys: Vec<f64> = p.strips[0].joints.iter().map(|j| j.1).collect();
    assert_eq!(ys.len(), 11, "{ys:?}");
    assert!(
        (ys[2] - 134.5).abs() < 0.1 && (ys[3] - 140.5).abs() < 0.1,
        "{ys:?}"
    );
    // the slots of the strip are the pieces the joints belong to, one per segment
    assert_eq!(p.strips[0].slots.len() + 1, p.strips[0].joints.len());
    let plane = solved.points[0].height;
    for y in [134.6, 137.5, 140.4] {
        assert!((strip_height_at(&p, y) - plane).abs() < 0.01, "{y}");
    }
    let holds = chain_holds(&net, &net.chains[0], 4.0, &junctions);
    let held: Vec<_> = holds.iter().filter_map(|(_, h)| h.as_ref()).collect();
    assert_eq!(held.len(), 2, "{held:?}");
    assert!(
        held.iter()
            .all(|h| h.rule == naves_core::profile::HoldRule::Patch)
    );
}

#[test]
fn a_new_joint_is_not_added_within_the_minimum_distance_of_an_existing_one() {
    // the junction 2 m from the joint at y = 125, patch 1.5 m: where the road enters (125.5) and leaves (128.5) the
    // patch is 0.5 m and 3.5 m from that joint, under RAMP_JOINT_MIN_M, so none is added and the road's own joint is
    // the one the junction holds
    assert!((RAMP_JOINT_MIN_M - 4.0).abs() < 1e-9);
    let hm = flat_100();
    let (net, junctions) = passing_junction(127.0, 1.5);
    let (p, solved) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
    assert_eq!(p.strips[0].joints.len(), 9, "{:?}", p.strips[0].joints);
    let holds = chain_holds(&net, &net.chains[0], 4.0, &junctions);
    let held: Vec<(usize, _)> = holds
        .iter()
        .enumerate()
        .filter_map(|(k, (_, h))| h.as_ref().map(|h| (k, h)))
        .collect();
    assert_eq!(held.len(), 1, "{held:?}");
    assert_eq!(held[0].0, 1, "the joint at y = 125: {held:?}");
    assert_eq!(held[0].1.rule, naves_core::profile::HoldRule::Patch);
    let plane = solved.points[0].height;
    assert!((p.strips[0].joints[1].2 - plane).abs() < 0.01);
}

#[test]
fn a_plain_road_away_from_junctions_and_decks_keeps_the_heights_it_had() {
    // golden heights of the solve before the ramps: a rolling terrain, a road of 12.5 m pieces, no junction, no deck
    let hm = terrain(128, 6.25, |x, y| {
        100.0 + 6.0 * (y / 40.0).sin() + 0.05 * x + 3.0 * (y / 17.0).cos()
    });
    let net = network(vec![pieces(
        "a",
        Family::Asf3,
        (300.0, 100.0),
        (300.0, 300.0),
        12.5,
    )]);
    let p = solve_alone(&net, &hm);
    let heights: Vec<f64> = p.strips[0].joints.iter().map(|j| j.2).collect();
    let golden: Vec<f64> = GOLDEN_PLAIN.to_vec();
    assert_eq!(heights.len(), golden.len());
    for (h, g) in heights.iter().zip(&golden) {
        assert!((h - g).abs() < 1e-3, "{heights:?}");
    }
}

/// Recorded from the solve at da88a23 (before the ramps).
const GOLDEN_PLAIN: [f64; 17] = [
    119.496, 117.340, 115.224, 113.270, 111.651, 110.525, 109.969, 109.954, 110.362, 111.047,
    111.897, 112.865, 113.965, 115.231, 116.668, 118.235, 119.860,
];

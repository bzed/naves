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
use naves_core::limits::{grade_aim, grade_ceiling, min_vertical_radius};
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
    let (p, _) = solve_plain(&net, &hm, &junctions, &[]).unwrap();
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

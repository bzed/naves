// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The junctions of a placed network, their heights from the ground, and the repair.

#![expect(
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::catalog::Family;
use naves_core::catalog::testing::synthetic_catalog;
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::heightmap::Heightmap;
use naves_core::junctions::{
    Junction, JunctionKind, Junctions, Link, find_junctions, ground_median, repair,
};
use naves_core::limits::grade_ceiling;
use naves_core::roadfit::Pose;
use naves_core::roadnet::{RoadGraph, build_graph};
use naves_core::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad, Slot, place_network};

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

/// A through road at y = 200 with a side road of `side` going north from x = 200.
fn t_of(through: Family, side: Family) -> (PlacedNetwork, RoadGraph) {
    placed(&[
        feature(
            1,
            through,
            &[1, 2, 3],
            &[(50.0, 200.0), (200.0, 200.0), (350.0, 200.0)],
        ),
        feature(2, side, &[2, 4], &[(200.0, 200.0), (200.0, 350.0)]),
    ])
}

/// An asf3 through road with a city side road.
fn t_junction_network() -> (PlacedNetwork, RoadGraph) {
    t_of(Family::Asf3, Family::City)
}

/// A through road at y = 200 with two side roads that meet it at the given stations from x = 100.
fn two_side_roads_meeting_a_road_at(a: f64, b: f64) -> (PlacedNetwork, RoadGraph) {
    placed(&[
        feature(
            1,
            Family::Asf3,
            &[1, 2, 3, 4],
            &[
                (50.0, 200.0),
                (100.0 + a, 200.0),
                (100.0 + b, 200.0),
                (350.0, 200.0),
            ],
        ),
        feature(
            2,
            Family::Asf3,
            &[2, 5],
            &[(100.0 + a, 200.0), (100.0 + a, 350.0)],
        ),
        feature(
            3,
            Family::Asf3,
            &[3, 6],
            &[(100.0 + b, 200.0), (100.0 + b, 50.0)],
        ),
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

fn junction(at: (f64, f64), ground: f64, rank: u8) -> Junction {
    Junction {
        at,
        kind: JunctionKind::Join,
        ground,
        height: ground,
        rank,
        patch_m: 3.5,
    }
}

/// Two junctions joined by one link of `length` metres.
fn two_junctions(length: f64, ceiling: f64, g0: f64, g1: f64, r0: u8, r1: u8) -> Junctions {
    Junctions {
        points: vec![
            junction((0.0, 0.0), g0, r0),
            junction((length, 0.0), g1, r1),
        ],
        links: vec![Link {
            a: 0,
            b: 1,
            length,
            ceiling,
        }],
        unresolved: Vec::new(),
    }
}

/// Three city junctions in a ring of links of `length` metres.
fn ring_of_three(length: f64, ceiling: f64, grounds: [f64; 3]) -> Junctions {
    Junctions {
        points: grounds
            .iter()
            .zip([0.0, 1.0, 2.0])
            .map(|(ground, i)| junction((i * length, 0.0), *ground, 4))
            .collect(),
        links: [(0, 1), (1, 2), (0, 2)]
            .into_iter()
            .map(|(a, b)| Link {
                a,
                b,
                length,
                ceiling,
            })
            .collect(),
        unresolved: Vec::new(),
    }
}

#[test]
fn the_median_ignores_one_wild_sample() {
    // flat at 100 with one vertex at 500 inside the circle
    let hm = terrain(64, 2.0, |x, y| {
        if (x - 60.0).abs() < 1e-6 && (y - 60.0).abs() < 1e-6 {
            500.0
        } else {
            100.0
        }
    });
    assert!((ground_median(&hm, (60.0, 60.0), 10.0) - 100.0).abs() < 0.5);
}

#[test]
fn a_junction_at_the_map_edge_uses_the_samples_inside() {
    let hm = terrain(64, 2.0, |x, _| 100.0 + 0.1 * x);
    let h = ground_median(&hm, (1.0, 60.0), 10.0);
    assert!(h.is_finite() && (100.0..=101.2).contains(&h));
    // nothing inside: the height at the point, clamped to the map
    let outside = ground_median(&hm, (-50.0, 60.0), 10.0);
    assert!((outside - 100.0).abs() < 1e-6, "{outside}");
}

#[test]
fn a_small_circle_holds_the_samples_around_the_point() {
    // a radius of 1 m holds the point and its four neighbours on the 1 m grid
    let hm = terrain(64, 2.0, |x, _| 100.0 + x);
    let h = ground_median(&hm, (40.0, 40.0), 1.0);
    assert!((h - 140.0).abs() < 1e-3, "{h}");
}

#[test]
fn every_junction_starts_on_its_ground() {
    let (net, graph) = t_junction_network(); // an asf3 through road with a city side road
    let hm = terrain(128, 3.125, |x, y| 100.0 + 0.05 * x + 0.02 * y);
    let j = find_junctions(&net, &graph, &hm);
    assert_eq!(j.points.len(), 1);
    assert!((j.points[0].height - j.points[0].ground).abs() < 1e-9);
    assert!(j.unresolved.is_empty());
}

#[test]
fn a_placed_junction_piece_is_a_piece_with_a_patch_of_half_its_length() {
    let (net, graph) = t_of(Family::Asf3, Family::Asf3);
    let piece = net
        .roads
        .iter()
        .find(|r| r.kind == PlacedKind::Junction)
        .unwrap();
    let half = 0.5 * (piece.end.x - piece.start.x).hypot(piece.end.y - piece.start.y);
    let hm = terrain(128, 3.125, |_, _| 100.0);
    let j = find_junctions(&net, &graph, &hm);
    assert_eq!(j.points.len(), 1, "{j:?}");
    let p = &j.points[0];
    assert_eq!(p.kind, JunctionKind::Piece);
    assert_eq!(p.rank, 3);
    assert!((p.patch_m - half).abs() < 1e-6, "{} {half}", p.patch_m);
    assert!(((p.at.0 - 200.0).hypot(p.at.1 - 200.0)) < 1.0);
}

#[test]
fn a_join_without_a_junction_piece_is_a_join() {
    // there is no gravel junction
    let (net, graph) = t_of(Family::Grav, Family::Mud);
    assert_eq!(net.joins_without_junction.len(), 1);
    let hm = terrain(128, 3.125, |_, _| 100.0);
    let j = find_junctions(&net, &graph, &hm);
    assert_eq!(j.points.len(), 1, "{j:?}");
    let p = &j.points[0];
    assert_eq!(p.kind, JunctionKind::Join);
    assert_eq!(p.rank, 2);
    // half the width of the widest road
    let widest = net.roads.iter().map(|r| r.width).fold(0.0, f64::max);
    assert!((p.patch_m - 0.5 * widest).abs() < 1e-6, "{}", p.patch_m);
}

#[test]
fn a_chain_end_on_another_road_without_a_crossing_is_a_junction() {
    let through: Vec<PlacedRoad> = (0..8)
        .map(|i| {
            let x = 25.0 * f64::from(i);
            piece(
                "a",
                i,
                PlacedKind::Piece,
                Family::Asf3,
                (x, 100.0),
                (x + 25.0, 100.0),
            )
        })
        .collect();
    let side: Vec<PlacedRoad> = (0..4)
        .map(|i| {
            let y = 103.0 + 25.0 * f64::from(i);
            piece(
                "b",
                i,
                PlacedKind::Piece,
                Family::Mud,
                (100.0, y),
                (100.0, y + 25.0),
            )
        })
        .collect();
    let mut net = PlacedNetwork::default();
    for chain in [through, side] {
        let base = net.roads.len();
        net.chains.push((base..base + chain.len()).collect());
        net.roads.extend(chain);
    }
    let hm = terrain(128, 3.125, |_, _| 100.0);
    let j = find_junctions(&net, &RoadGraph::default(), &hm);
    assert_eq!(j.points.len(), 1, "{j:?}");
    let p = &j.points[0];
    assert_eq!(p.kind, JunctionKind::EndOnRoad);
    assert_eq!(p.rank, 3);
    assert!(((p.at.0 - 100.0).hypot(p.at.1 - 103.0)) < 1e-6);
    assert!((p.patch_m - 3.5).abs() < 1e-6);
}

#[test]
fn two_junctions_too_close_for_their_ground_move_toward_each_other() {
    // two junctions 10 m of city road apart (ceiling 0.27), ground 100 and 108
    let mut j = two_junctions(10.0, 0.27, 100.0, 108.0, /* ranks */ 4, 4);
    repair(&mut j);
    let rise = (j.points[1].height - j.points[0].height).abs();
    assert!(rise <= 0.27 * 10.0 + 0.01);
    assert!(j.unresolved.is_empty());
    // equal rank: both moved by the same amount
    assert!(((j.points[0].height - 100.0) - (108.0 - j.points[1].height)).abs() < 0.01);
}

#[test]
fn the_junction_on_the_more_important_road_moves_less() {
    let mut j = two_junctions(
        10.0, 0.27, 100.0, 108.0, /* asf1 */ 6, /* mud */ 1,
    );
    repair(&mut j);
    assert!((j.points[0].height - 100.0).abs() < (108.0 - j.points[1].height).abs() / 16.0);
}

#[test]
fn junctions_three_metres_apart_are_one() {
    let (net, graph) = two_side_roads_meeting_a_road_at(/* stations */ 50.0, 52.5);
    let hm = terrain(128, 3.125, |_, _| 100.0);
    assert_eq!(find_junctions(&net, &graph, &hm).points.len(), 1);
}

#[test]
fn junctions_far_apart_stay_two_and_are_linked() {
    let (net, graph) = two_side_roads_meeting_a_road_at(/* stations */ 20.0, 120.0);
    let hm = terrain(128, 3.125, |x, _| 100.0 + 0.01 * x);
    let j = find_junctions(&net, &graph, &hm);
    assert_eq!(j.points.len(), 2, "{j:?}");
    assert_eq!(j.links.len(), 1, "{:?}", j.links);
    let link = &j.links[0];
    assert_eq!((link.a, link.b), (0, 1));
    assert!(link.length > 80.0 && link.length < 100.0, "{}", link.length);
    assert!((link.ceiling - grade_ceiling(Family::Asf3)).abs() < 1e-12);
}

#[test]
fn the_result_does_not_depend_on_the_order_of_the_roads() {
    let (net, graph) = grid_of_roads(); // 3 x 3 roads on a tilted plane
    let hm = terrain(256, 3.125, |x, y| 100.0 + 0.2 * x - 0.1 * y);
    let a = find_junctions(&net, &graph, &hm);
    assert_eq!(a.points.len(), 9);
    let b = find_junctions(&reordered(&net), &graph, &hm); // chains and roads in reverse
    assert_eq!(a, b);
}

#[test]
fn a_link_that_cannot_be_repaired_is_reported() {
    // three junctions in a ring of 1 m links with grounds 100, 120, 140: cannot converge to within the ceiling
    // on all three unless they meet; after 50 rounds either all are legal or the rest is listed
    let mut j = ring_of_three(1.0, 0.27, [100.0, 120.0, 140.0]);
    repair(&mut j);
    for (i, link) in j.links.iter().enumerate() {
        let rise = (j.points[link.a].height - j.points[link.b].height).abs();
        assert!(rise <= link.ceiling * link.length + 0.01 || j.unresolved.contains(&i));
    }
}

#[test]
fn a_link_that_is_still_over_after_the_rounds_is_listed() {
    // a long ring edge has no room: 3 junctions, 20 m links would be fine, a 0 m ceiling never is
    let mut j = ring_of_three(10.0, 0.0, [100.0, 120.0, 140.0]);
    repair(&mut j);
    let over: Vec<usize> = j
        .links
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            (j.points[l.a].height - j.points[l.b].height).abs() > l.ceiling * l.length + 0.01
        })
        .map(|(i, _)| i)
        .collect();
    assert_eq!(j.unresolved, over);
}

#[test]
fn at_finds_the_nearest_junction_whose_patch_holds_the_point() {
    let mut j = two_junctions(6.0, 0.27, 100.0, 100.0, 4, 4);
    j.points[0].patch_m = 5.0;
    j.points[1].patch_m = 5.0;
    assert_eq!(j.at((1.0, 0.0)).map(|p| p.at), Some((0.0, 0.0)));
    assert_eq!(j.at((4.0, 0.0)).map(|p| p.at), Some((6.0, 0.0)));
    assert!(j.at((3.0, 20.0)).is_none());
}

#[test]
fn a_link_with_a_negative_ceiling_is_unresolved_and_named() {
    // no height difference satisfies a ceiling below zero; the sound link next to it is not blamed
    let mut j = Junctions {
        points: vec![
            junction((0.0, 0.0), 100.0, 4),
            junction((10.0, 0.0), 100.0, 4),
            junction((100.0, 0.0), 100.0, 4),
            junction((110.0, 0.0), 100.0, 4),
        ],
        links: vec![
            Link {
                a: 0,
                b: 1,
                length: 10.0,
                ceiling: 0.27,
            },
            Link {
                a: 2,
                b: 3,
                length: 10.0,
                ceiling: -1.0,
            },
        ],
        unresolved: Vec::new(),
    };
    repair(&mut j);
    assert_eq!(j.unresolved, vec![1]);
}

#[test]
fn no_roads_have_no_junctions() {
    let hm = terrain(16, 2.0, |_, _| 100.0);
    let j = find_junctions(&PlacedNetwork::default(), &RoadGraph::default(), &hm);
    assert_eq!(j, Junctions::default());
}

#[test]
fn a_link_of_no_length_repairs_without_nan() {
    let mut j = two_junctions(0.0, 0.27, 100.0, 108.0, 4, 4);
    repair(&mut j);
    assert!(j.points.iter().all(|p| p.height.is_finite()), "{j:?}");
    assert!((j.points[0].height - j.points[1].height).abs() < 0.01);
}

#[test]
fn near_reaches_beyond_the_patch_by_the_given_distance() {
    let mut j = two_junctions(60.0, 0.27, 100.0, 100.0, 4, 4);
    j.points[0].patch_m = 5.0;
    assert!(j.at((8.0, 0.0)).is_none());
    assert_eq!(j.near((8.0, 0.0), 4.0).map(|p| p.at), Some((0.0, 0.0)));
    assert!(j.near((10.0, 0.0), 4.0).is_none());
}

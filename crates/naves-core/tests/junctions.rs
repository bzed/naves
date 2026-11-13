// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The junctions of a placed network, their heights from the ground, and the repair.

#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::catalog::Family;
use naves_core::catalog::testing::synthetic_catalog;
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::heightmap::Heightmap;
use naves_core::junctions::{
    Junction, JunctionKind, Junctions, Link, MAX_MOVE_M, find_junctions, ground_median, repair,
};
use naves_core::limits::grade_ceiling;
use naves_core::profile::{HoldRule, chain_holds, chain_holds_unchecked};
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
        stretches: std::collections::BTreeSet::new(),
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
    // 4 m of ground over 10 m of road (the cap of 3 m is not reached)
    let mut j = two_junctions(
        10.0, 0.27, 100.0, 104.0, /* asf1 */ 6, /* mud */ 1,
    );
    repair(&mut j);
    assert!((j.points[0].height - 100.0).abs() < (104.0 - j.points[1].height).abs() / 16.0);
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
    let mut j = two_junctions(0.0, 0.27, 100.0, 105.0, 4, 4);
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

/// For every two consecutive held joints of a chain that take their heights from different junctions: a link
/// between those junctions with the length along the chain between the two joints.
fn assert_links_are_the_spans(net: &PlacedNetwork, j: &Junctions) {
    let mut pairs = 0;
    for chain in &net.chains {
        let holds = chain_holds(net, chain, 3.125, j);
        let mut along = 0.0;
        let mut previous: Option<(usize, f64)> = None;
        for (i, (at, hold)) in holds.iter().enumerate() {
            if i > 0 {
                let before = holds[i - 1].0;
                along += (at.0 - before.0).hypot(at.1 - before.1);
            }
            let Some(hold) = hold else { continue };
            if let Some((junction, station)) = previous
                && junction != hold.junction
            {
                pairs += 1;
                let key = (junction.min(hold.junction), junction.max(hold.junction));
                let span = (along - station).max(1.0);
                assert!(
                    j.links
                        .iter()
                        .any(|l| (l.a, l.b) == key && (l.length - span).abs() < 1e-6),
                    "no link {key:?} of {span} m in {:?}",
                    j.links
                );
            }
            previous = Some((hold.junction, along));
        }
    }
    assert!(pairs > 0);
}

#[test]
fn the_links_are_the_spans_between_the_joints_that_are_held() {
    let (net, graph) = grid_of_roads();
    let hm = terrain(256, 3.125, |x, y| 100.0 + 0.2 * x - 0.1 * y);
    assert_links_are_the_spans(&net, &find_junctions(&net, &graph, &hm));
    let (net, graph) = two_side_roads_meeting_a_road_at(20.0, 120.0);
    let hm = terrain(128, 3.125, |x, _| 100.0 + 0.01 * x);
    assert_links_are_the_spans(&net, &find_junctions(&net, &graph, &hm));
}

#[test]
fn a_junction_between_two_joints_holds_the_nearer_one() {
    // two side tracks end on a through road 12 m after one of its joints: no joint lies in either patch, so the
    // nearer joint of the piece holds the height (one joint each, not both ends of the piece)
    let through: Vec<PlacedRoad> = (0..12)
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
    let side = |name: &str, x: f64| -> Vec<PlacedRoad> {
        (0..4)
            .map(|i| {
                let y = 103.0 + 25.0 * f64::from(i);
                piece(
                    name,
                    i,
                    PlacedKind::Piece,
                    Family::Mud,
                    (x, y),
                    (x, y + 25.0),
                )
            })
            .collect()
    };
    let mut net = PlacedNetwork::default();
    for chain in [through, side("b", 112.0), side("c", 212.0)] {
        let base = net.roads.len();
        net.chains.push((base..base + chain.len()).collect());
        net.roads.extend(chain);
    }
    let hm = terrain(128, 3.125, |_, _| 100.0);
    let j = find_junctions(&net, &RoadGraph::default(), &hm);
    assert_eq!(j.points.len(), 2, "{j:?}");
    let held: Vec<usize> = chain_holds(&net, &net.chains[0], 3.125, &j)
        .iter()
        .enumerate()
        .filter(|(_, (_, h))| h.is_some())
        .map(|(i, _)| i)
        .collect();
    assert_eq!(held, vec![4, 8], "{j:?}");
    let spans: Vec<f64> = j.links.iter().map(|l| l.length).collect();
    assert!(spans.iter().any(|s| (s - 100.0).abs() < 1e-6), "{spans:?}");
    assert_links_are_the_spans(&net, &j);
}

#[test]
fn a_junction_never_moves_more_than_three_metres_from_its_ground() {
    // 10 m of road between grounds 20 m apart: 3 m each is not enough, so both stop at 3 m and the link is
    // reported
    let mut j = two_junctions(10.0, 0.27, 100.0, 120.0, 4, 4);
    repair(&mut j);
    assert!((j.points[0].height - 103.0).abs() < 1e-9, "{:?}", j.points);
    assert!((j.points[1].height - 117.0).abs() < 1e-9, "{:?}", j.points);
    assert_eq!(j.unresolved, vec![0]);
}

#[test]
fn a_link_that_needs_less_than_the_cap_is_still_repaired() {
    let mut j = two_junctions(10.0, 0.27, 100.0, 106.0, 4, 4);
    repair(&mut j);
    assert!(j.unresolved.is_empty());
    assert!(
        j.points
            .iter()
            .all(|p| (p.height - p.ground).abs() <= MAX_MOVE_M)
    );
}

/// A chain of `count` pieces of `length` metres from `from` along `direction` (a unit vector), as one stretch.
fn run_of(
    name: &str,
    family: Family,
    from: (f64, f64),
    direction: (f64, f64),
    length: f64,
    count: u32,
) -> Vec<PlacedRoad> {
    (0..count)
        .map(|i| {
            let at = |k: u32| {
                (
                    from.0 + direction.0 * length * f64::from(k),
                    from.1 + direction.1 * length * f64::from(k),
                )
            };
            piece(name, i, PlacedKind::Piece, family, at(i), at(i + 1))
        })
        .collect()
}

fn net_of(chains: Vec<Vec<PlacedRoad>>) -> PlacedNetwork {
    let mut net = PlacedNetwork::default();
    for chain in chains {
        let base = net.roads.len();
        net.chains.push((base..base + chain.len()).collect());
        net.roads.extend(chain);
    }
    net
}

/// One junction at `at`, to which the stretches `names` belong.
fn one_junction_of(at: (f64, f64), patch_m: f64, names: &[&str], kind: JunctionKind) -> Junctions {
    Junctions {
        points: vec![Junction {
            at,
            kind,
            ground: 100.0,
            height: 100.0,
            rank: 3,
            patch_m,
            stretches: names.iter().map(|n| (*n).to_string()).collect(),
        }],
        ..Junctions::default()
    }
}

fn held_rules(net: &PlacedNetwork, chain: usize, j: &Junctions) -> Vec<Option<HoldRule>> {
    chain_holds(net, &net.chains[chain], 3.125, j)
        .iter()
        .map(|(_, h)| h.map(|h| h.rule))
        .collect()
}

#[test]
fn a_chain_that_ends_beyond_the_patch_is_held_at_its_end() {
    // the junction's patch is 0.5 m, the side road ends 3 m from it: only the end rule holds it
    let net = net_of(vec![
        run_of("t", Family::Asf3, (50.0, 100.0), (1.0, 0.0), 25.0, 8),
        run_of("s", Family::Asf3, (100.0, 103.0), (0.0, 1.0), 25.0, 4),
    ]);
    let j = one_junction_of((100.0, 100.0), 0.5, &["s", "t"], JunctionKind::Join);
    let rules = held_rules(&net, 1, &j);
    assert_eq!(rules[0], Some(HoldRule::End), "{rules:?}");
    assert!(rules[1..].iter().all(Option::is_none), "{rules:?}");
}

#[test]
fn joints_inward_of_a_held_end_are_held_while_they_lie_on_the_other_road() {
    // a side road with joints 3.0, 4.2 and 29 m from a junction with a patch of 3.5 m: the end is in the
    // patch, the second joint is on the through road's surface (3.5 m and 1 m), the third is not
    // the chain runs from the far end to the junction, so the end is the last joint
    let side = vec![
        piece(
            "s",
            0,
            PlacedKind::Piece,
            Family::Asf3,
            (100.0, 129.0),
            (100.0, 104.2),
        ),
        piece(
            "s",
            1,
            PlacedKind::Piece,
            Family::Asf3,
            (100.0, 104.2),
            (100.0, 103.0),
        ),
    ];
    let net = net_of(vec![
        run_of("t", Family::Asf3, (50.0, 100.0), (1.0, 0.0), 25.0, 8),
        side,
    ]);
    let j = one_junction_of((100.0, 100.0), 3.5, &["s", "t"], JunctionKind::Join);
    let rules = held_rules(&net, 1, &j);
    assert_eq!(
        rules,
        vec![None, Some(HoldRule::Inward), Some(HoldRule::Patch)]
    );
}

#[test]
fn a_junction_piece_holds_both_its_joints() {
    let (net, graph) = t_of(Family::Asf3, Family::Asf3);
    let hm = terrain(128, 3.125, |_, _| 100.0);
    let j = find_junctions(&net, &graph, &hm);
    let through = net
        .chains
        .iter()
        .position(|c| c.iter().any(|i| net.roads[*i].kind == PlacedKind::Junction))
        .unwrap();
    let rules = held_rules(&net, through, &j);
    assert_eq!(
        rules
            .iter()
            .filter(|r| **r == Some(HoldRule::Piece))
            .count(),
        2,
        "{rules:?}"
    );
}

#[test]
fn a_chain_that_passes_a_junction_is_held_at_the_one_joint_nearest_it() {
    // a chain of 3 m pieces with five joints in a patch of 7 m: the nearest one holds, not all five
    let net = net_of(vec![run_of(
        "t",
        Family::Asf3,
        (71.5, 100.0),
        (1.0, 0.0),
        3.0,
        20,
    )]);
    let j = one_junction_of((100.0, 100.0), 7.0, &["t"], JunctionKind::Join);
    let rules = held_rules(&net, 0, &j);
    assert_eq!(rules.iter().filter(|r| r.is_some()).count(), 1, "{rules:?}");
    assert!(rules.contains(&Some(HoldRule::Patch)), "{rules:?}");
}

#[test]
fn a_parallel_street_beside_a_junction_keeps_its_own_height() {
    // a street 10 m south of a T junction, with no node in common: within the reach of an arm, but not one of its
    // roads
    let (net, graph) = placed(&[
        feature(
            1,
            Family::Asf3,
            &[1, 2, 3],
            &[(50.0, 200.0), (200.0, 200.0), (350.0, 200.0)],
        ),
        feature(2, Family::City, &[2, 4], &[(200.0, 200.0), (200.0, 350.0)]),
        feature(
            3,
            Family::City,
            &[11, 12, 13],
            &[(50.0, 190.0), (200.0, 190.0), (350.0, 190.0)],
        ),
    ]);
    let hm = terrain(128, 3.125, |_, _| 100.0);
    let j = find_junctions(&net, &graph, &hm);
    let street = net
        .chains
        .iter()
        .position(|c| net.roads[c[0]].slot.stretch.starts_with("3:"))
        .unwrap();
    assert!(held_rules(&net, street, &j).iter().all(Option::is_none));
    // without the check it would be held: the check is what keeps it
    let unchecked = chain_holds_unchecked(&net, &net.chains[street], 3.125, &j);
    assert!(unchecked.iter().any(|(_, h)| h.is_some()));
}

#[test]
fn a_road_that_crosses_a_junction_above_it_is_not_held_to_it() {
    // a road over the junction point (a bridge in the real data), with no node in common
    let (net, graph) = placed(&[
        feature(
            1,
            Family::Asf3,
            &[1, 2, 3],
            &[(50.0, 200.0), (200.0, 200.0), (350.0, 200.0)],
        ),
        feature(2, Family::City, &[2, 4], &[(200.0, 200.0), (200.0, 350.0)]),
        feature(
            3,
            Family::City,
            &[21, 22],
            &[(120.0, 280.0), (280.0, 120.0)],
        ),
    ]);
    let hm = terrain(128, 3.125, |_, _| 100.0);
    let j = find_junctions(&net, &graph, &hm);
    let above = net
        .chains
        .iter()
        .position(|c| net.roads[c[0]].slot.stretch.starts_with("3:"))
        .unwrap();
    assert!(held_rules(&net, above, &j).iter().all(Option::is_none));
    assert!(
        chain_holds_unchecked(&net, &net.chains[above], 3.125, &j)
            .iter()
            .any(|(_, h)| h.is_some())
    );
}

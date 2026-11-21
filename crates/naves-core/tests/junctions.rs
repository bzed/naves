// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The junctions of a placed network, their heights from the ground, and the links between them.

#![expect(
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::catalog::Family;
use naves_core::catalog::testing::synthetic_catalog;
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::heightmap::Heightmap;
use naves_core::junctions::{
    JUNCTION_TILT_SHARE, Junction, JunctionKind, Junctions, Link, find_junctions, ground_median,
};
use naves_core::limits::{CROSS_FALL_AIM, grade_ceiling};
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
        scale: 1.0,
        abreast: 1,
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
            roundabout: false,
            layer: 0,
            use_sidepath: false,
            paving: None,
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
        lower: std::collections::BTreeSet::new(),
        gradient: (0.0, 0.0),
        tilt_axis: None,
        tilt_cap: 0.0,
    }
}

/// Two junctions joined by one link of `length` metres.
fn two_junctions(length: f64, ceiling: f64, g0: f64, g1: f64, r0: u8, r1: u8) -> Junctions {
    Junctions {
        rings: Vec::new(),
        points: vec![
            junction((0.0, 0.0), g0, r0),
            junction((length, 0.0), g1, r1),
        ],
        links: vec![Link {
            a: 0,
            b: 1,
            length,
            ceiling,
            a_at: (0.0, 0.0),
            b_at: (length, 0.0),
        }],
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
    assert_eq!(j.unresolved.len(), 0);
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
fn a_crossing_without_a_junction_piece_is_an_overlay() {
    // there is no gravel junction: the Mud road ends under the Grav road
    let (net, graph) = t_of(Family::Grav, Family::Mud);
    assert_eq!(net.joins_without_junction.len(), 0);
    assert_eq!(net.overlays.len(), 1);
    let hm = terrain(128, 3.125, |_, _| 100.0);
    let j = find_junctions(&net, &graph, &hm);
    assert_eq!(j.points.len(), 1, "{j:?}");
    let p = &j.points[0];
    assert_eq!(p.kind, JunctionKind::Overlay);
    assert_eq!(
        p.lower.len(),
        1,
        "only the Mud road lies under: {:?}",
        p.lower
    );
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
fn at_finds_the_nearest_junction_whose_patch_holds_the_point() {
    let mut j = two_junctions(6.0, 0.27, 100.0, 100.0, 4, 4);
    j.points[0].patch_m = 5.0;
    j.points[1].patch_m = 5.0;
    assert_eq!(j.at((1.0, 0.0)).map(|p| p.at), Some((0.0, 0.0)));
    assert_eq!(j.at((4.0, 0.0)).map(|p| p.at), Some((6.0, 0.0)));
    assert!(j.at((3.0, 20.0)).is_none());
}

#[test]
fn no_roads_have_no_junctions() {
    let hm = terrain(16, 2.0, |_, _| 100.0);
    let j = find_junctions(&PlacedNetwork::default(), &RoadGraph::default(), &hm);
    assert_eq!(j, Junctions::default());
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
fn a_junction_between_two_joints_holds_a_joint_where_the_road_passes_through_its_patch() {
    // two side tracks end on a through road 12 m after one of its joints: no joint lies in either patch, so the
    // solve adds one where the road passes through each patch (the road lies 3 m from the junction's point, in a
    // patch of 3.5 m: a 3.4 m run, one joint at the nearest approach) and holds that, not an end of the piece
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
    assert_eq!(held, vec![5, 10], "{j:?}");
    let spans: Vec<f64> = j.links.iter().map(|l| l.length).collect();
    assert!(spans.iter().any(|s| (s - 100.0).abs() < 1e-6), "{spans:?}");
    assert_links_are_the_spans(&net, &j);
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
            lower: std::collections::BTreeSet::new(),
            gradient: (0.0, 0.0),
            tilt_axis: None,
            tilt_cap: 0.0,
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

/// A roundabout of `radius` metres around (400, 400) with a street that leaves it to the north.
fn ring_with_a_street(radius: f64) -> (PlacedNetwork, RoadGraph) {
    let n = 24_i64;
    let at = |i: i64, r: f64| {
        let a = (i % n) as f64 / n as f64 * std::f64::consts::TAU;
        (400.0 + r * a.sin(), 400.0 + r * a.cos())
    };
    let nodes: Vec<i64> = (0..=n).map(|i| 100 + i % n).collect();
    let points: Vec<(f64, f64)> = (0..=n).map(|i| at(i, radius)).collect();
    let mut ring = feature(1, Family::City, &nodes, &points);
    if let FeatureKind::Road { roundabout, .. } = &mut ring.kind {
        *roundabout = true;
    }
    placed(&[
        ring,
        feature(
            2,
            Family::Asf3,
            &[100, 500],
            &[at(0, radius), at(0, radius + 90.0)],
        ),
        feature(
            3,
            Family::Asf3,
            &[112, 501],
            &[at(12, radius), at(12, radius + 90.0)],
        ),
    ])
}

#[test]
fn the_tilt_is_clamped_to_the_grade_ceiling() {
    // a hillside of 50 %, far over what a city street may have
    let base = terrain(256, 3.125, |x, _| 100.0 + 0.5 * x);
    let (net, graph) = ring_with_a_street(30.0);
    let junctions = find_junctions(&net, &graph, &base);
    assert_eq!(junctions.rings.len(), 1);
    let plane = &junctions.rings[0];
    let magnitude = plane.gradient.0.hypot(plane.gradient.1);
    assert!(
        (magnitude - grade_ceiling(Family::City)).abs() < 1e-9,
        "{magnitude}"
    );
    // it still points uphill
    assert!(plane.gradient.0 > 0.0 && plane.gradient.1.abs() < 1e-6);
}

#[test]
fn a_gentle_hillside_is_the_planes_tilt_as_it_is() {
    let base = terrain(256, 3.125, |x, y| 100.0 + 0.04 * x + 0.02 * y);
    let (net, graph) = ring_with_a_street(30.0);
    let junctions = find_junctions(&net, &graph, &base);
    let plane = &junctions.rings[0];
    assert!(
        (plane.gradient.0 - 0.04).abs() < 1e-3,
        "{:?}",
        plane.gradient
    );
    assert!(
        (plane.gradient.1 - 0.02).abs() < 1e-3,
        "{:?}",
        plane.gradient
    );
    assert!((plane.height - (100.0 + 0.04 * 400.0 + 0.02 * 400.0)).abs() < 0.01);
}

#[test]
fn joining_roads_end_on_the_ring_at_the_plane_height() {
    let base = terrain(256, 3.125, |x, y| 100.0 + 0.04 * x + 0.02 * y);
    let (net, graph) = ring_with_a_street(30.0);
    let junctions = find_junctions(&net, &graph, &base);
    let plane = &junctions.rings[0];
    let node = junctions
        .points
        .iter()
        .find(|j| j.kind == JunctionKind::Overlay && j.lower.iter().any(|id| id.starts_with("2:")))
        .unwrap();
    let want = plane.height_at(node.at);
    assert!((node.height - want).abs() < 1e-6, "{} {want}", node.height);
    let side = naves_core::sideslope::side_slopes(&net, &base);
    let profiles = naves_core::profile::solve_profiles(&net, &base, &junctions, &side).unwrap();
    // the ring follows the plane
    let ring_strip = profiles
        .strips
        .iter()
        .find(|s| net.roads[s.slots[0]].slot.stretch.starts_with("1:"))
        .unwrap();
    for joint in &ring_strip.joints {
        let plane_height = plane.height_at((joint.0, joint.1));
        assert!(
            (joint.2 - plane_height).abs() < 0.1,
            "{joint:?} {plane_height}"
        );
    }
    // the street ends at the plane's height, 5 cm under the ring
    let street = profiles
        .strips
        .iter()
        .find(|s| net.roads[s.slots[0]].slot.stretch.starts_with("2:"))
        .unwrap();
    let end = street.joints.iter().min_by(|a, b| {
        let d = |j: &&(f64, f64, f64)| (j.0 - node.at.0).hypot(j.1 - node.at.1);
        d(a).total_cmp(&d(b))
    });
    let end = end.unwrap();
    // on the plane at the place where the street ends (the solve puts the joint on the plane there; the plane's
    // height at the junction's point, 0.01 m away on this slope, was the reference when the junction's height held
    // the joint)
    let want_end = plane.height_at((end.0, end.1));
    assert!(
        (end.2 - (want_end - 0.05)).abs() < 0.02,
        "{} {want_end}",
        end.2
    );
}

#[test]
fn a_ring_on_flat_ground_is_level_and_a_ring_off_the_map_is_level_too() {
    let flat = terrain(256, 3.125, |_, _| 150.0);
    let (net, graph) = ring_with_a_street(30.0);
    let plane = &find_junctions(&net, &graph, &flat).rings[0];
    assert!(plane.gradient.0.abs() < 1e-9 && plane.gradient.1.abs() < 1e-9);
    assert!((plane.height - 150.0).abs() < 1e-3);
    // a map that does not reach the ring: nothing to fit, level at the nearest ground
    let small = terrain(16, 3.125, |_, _| 120.0);
    let plane = &find_junctions(&net, &graph, &small).rings[0];
    assert!(plane.gradient.0.abs() < 1e-9 && plane.gradient.1.abs() < 1e-9);
    assert!((plane.height - 120.0).abs() < 1e-3);
}

#[test]
fn a_large_ring_has_a_plane_over_its_band_only() {
    let base = terrain(256, 3.125, |x, y| 100.0 + 0.04 * x + 0.02 * y);
    let (net, graph) = ring_with_a_street(120.0);
    let plane = &find_junctions(&net, &graph, &base).rings[0];
    // the band: the centre line less and plus half the width (7 m for the City family here)
    let half = Family::City.width_m() / 2.0;
    assert!(
        (plane.radius - (120.0 + half)).abs() < 0.5,
        "{}",
        plane.radius
    );
    assert!(
        (plane.inner_radius - (120.0 - half)).abs() < 0.5,
        "{}",
        plane.inner_radius
    );
    // fitted over the band only, on ground that is itself a plane
    assert!((plane.gradient.0 - 0.04).abs() < 1e-3 && (plane.gradient.1 - 0.02).abs() < 1e-3);
}

/// A through road of `family` east-west (`vertical` false) or north-south at 200 m, with a side road of the same
/// family, so that the junction is a piece (an X when `cross`).
fn crossing_of(family: Family, vertical: bool, cross: bool) -> (PlacedNetwork, RoadGraph) {
    let at = |along: f64, across: f64| {
        if vertical {
            (across, along)
        } else {
            (along, across)
        }
    };
    let mut features = vec![
        feature(
            1,
            family,
            &[1, 2, 3],
            &[at(50.0, 200.0), at(200.0, 200.0), at(350.0, 200.0)],
        ),
        feature(2, family, &[2, 4], &[at(200.0, 200.0), at(200.0, 350.0)]),
    ];
    if cross {
        features.push(feature(
            3,
            family,
            &[2, 5],
            &[at(200.0, 200.0), at(200.0, 50.0)],
        ));
    }
    placed(&features)
}

#[test]
fn a_junction_on_a_slope_tilts_along_its_through_road() {
    let plane = terrain(256, 3.125, |x, _| 100.0 + 0.05 * x);
    for cross in [false, true] {
        let (net, graph) = crossing_of(Family::City, false, cross);
        let j = find_junctions(&net, &graph, &plane);
        let piece = j
            .points
            .iter()
            .find(|p| p.kind == JunctionKind::Piece)
            .unwrap();
        assert!(
            (piece.gradient.0 - 0.05).abs() < 1e-6,
            "{:?}",
            piece.gradient
        );
        assert!(piece.gradient.1.abs() < 1e-6, "{:?}", piece.gradient);
        // the height stays the height at the centre
        assert!((piece.height - piece.height_at(piece.at)).abs() < 1e-12);
        let (net, graph) = crossing_of(Family::City, true, cross);
        let j = find_junctions(&net, &graph, &plane);
        let piece = j
            .points
            .iter()
            .find(|p| p.kind == JunctionKind::Piece)
            .unwrap();
        assert!(piece.gradient.0.abs() < 1e-6, "{:?}", piece.gradient);
        assert!(piece.gradient.1.abs() < 1e-6, "{:?}", piece.gradient);
    }
}

#[test]
fn the_tilt_stops_at_the_cross_fall_aim() {
    // the plane's fall along the through road is the fall across every road that meets it there, and roads
    // have no fall across of their own: on a 15 % slope the junction tilts by the cross fall aim, not more
    let plane = terrain(256, 3.125, |x, _| 100.0 + 0.15 * x);
    for family in [Family::City, Family::Mud] {
        let (net, graph) = crossing_of(family, false, false);
        let j = find_junctions(&net, &graph, &plane);
        let piece = j
            .points
            .iter()
            .find(|p| p.kind == JunctionKind::Piece)
            .unwrap();
        let limit = CROSS_FALL_AIM.min(JUNCTION_TILT_SHARE * grade_ceiling(family));
        assert!((limit - CROSS_FALL_AIM).abs() < 1e-12);
        assert!(
            (piece.gradient.0 - limit).abs() < 1e-9,
            "{family:?} {:?}",
            piece.gradient
        );
        assert!(piece.gradient.1.abs() < 1e-9, "{:?}", piece.gradient);
    }
}

#[test]
fn a_chain_end_beside_a_road_tilts_with_the_ground_under_the_road() {
    // the end of the side road (the junction's point) lies 3 m north of the through road's centre line, on a
    // hillside that is level along the road itself and rises 6 % to the east 3 m north of it: the through road
    // is level here, and so is the junction
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
    let hm = terrain(128, 3.125, |x, y| 100.0 + 0.02 * (y - 100.0) * (x - 100.0));
    let j = find_junctions(&net, &RoadGraph::default(), &hm);
    assert_eq!(j.points.len(), 1, "{j:?}");
    let p = &j.points[0];
    assert_eq!(p.kind, JunctionKind::EndOnRoad);
    assert!(
        p.gradient.0.abs() < 1e-6 && p.gradient.1.abs() < 1e-6,
        "{:?}",
        p.gradient
    );
}

#[test]
fn a_join_of_ending_roads_stays_level() {
    let plane = terrain(256, 3.125, |x, _| 100.0 + 0.10 * x);
    let (net, graph) = placed(&[
        feature(1, Family::Grav, &[1, 2], &[(100.0, 100.0), (200.0, 200.0)]),
        feature(2, Family::Grav, &[3, 2], &[(300.0, 100.0), (200.0, 200.0)]),
        feature(3, Family::Grav, &[4, 2], &[(200.0, 350.0), (200.0, 200.0)]),
    ]);
    let j = find_junctions(&net, &graph, &plane);
    assert_eq!(j.points.len(), 1, "{j:?}");
    assert_eq!(j.points[0].kind, JunctionKind::Join, "{j:?}");
    assert_eq!(j.points[0].gradient, (0.0, 0.0));
}

#[test]
fn an_overlay_follows_its_top_road_and_a_junction_at_the_map_edge_stays_level() {
    // the Grav road runs east-west and lies on top; the Mud road ends under it
    let plane = terrain(256, 3.125, |x, _| 100.0 + 0.05 * x);
    let (net, graph) = t_of(Family::Grav, Family::Mud);
    let j = find_junctions(&net, &graph, &plane);
    let top = &j.points[0];
    assert_eq!(top.kind, JunctionKind::Overlay);
    assert!((top.gradient.0 - 0.05).abs() < 1e-6, "{:?}", top.gradient);
    assert!(top.gradient.1.abs() < 1e-6, "{:?}", top.gradient);
    // the road runs on east of the map edge, 3 m from the junction: no slope is known
    let small = terrain(66, 3.125, |x, _| 100.0 + 0.05 * x);
    let j = find_junctions(&net, &graph, &small);
    assert_eq!(j.points[0].gradient, (0.0, 0.0));
}

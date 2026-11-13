// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The road graph: stretches from crossing to crossing, and the chains of through roads.

#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::catalog::Family;
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::roadnet::{
    BridgeSpan, RoadGraph, Stretch, build_graph, chains, widen_bridges, with_bridge_spans,
};
use proptest::prelude::*;

fn feature(
    way: i64,
    family: Family,
    bridge: bool,
    nodes: &[i64],
    points: &[(f64, f64)],
) -> Feature {
    Feature {
        id: format!("osm/way/{way}/0"),
        way_id: way,
        source: Source::Osm,
        kind: FeatureKind::Road {
            family,
            drivable: true,
            bridge,
            tunnel: false,
        },
        points: points.to_vec(),
        nodes: nodes.to_vec(),
    }
}

fn road(way: i64, nodes: &[i64], points: &[(f64, f64)]) -> Feature {
    feature(way, Family::Asf3, false, nodes, points)
}

fn ids(graph: &RoadGraph) -> Vec<&str> {
    graph.stretches.iter().map(|s| s.id.as_str()).collect()
}

/// A readable form of the chains: per chain the stretch ids, with a `~` before a stretch that is walked
/// against its stored direction.
fn chain_ids(graph: &RoadGraph) -> Vec<Vec<String>> {
    chains(graph)
        .iter()
        .map(|c| {
            c.links
                .iter()
                .map(|l| {
                    let id = &graph.stretches[l.stretch].id;
                    if l.reversed {
                        format!("~{id}")
                    } else {
                        id.clone()
                    }
                })
                .collect()
        })
        .collect()
}

#[test]
fn degree_two_nodes_are_joined() {
    let g = build_graph(&[
        road(1, &[1, 2, 3], &[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)]),
        road(2, &[3, 4, 5], &[(100.0, 0.0), (150.0, 0.0), (200.0, 0.0)]),
    ]);
    assert_eq!(g.stretches.len(), 1);
    let s = &g.stretches[0];
    assert_eq!(s.id, "1:1-5");
    assert_eq!((s.start, s.end), (1, 5));
    assert_eq!(s.points.len(), 5);
    assert!((s.points[4].0 - 200.0).abs() < 1e-9);
    assert!(g.crossings.iter().all(|c| c.node == 1 || c.node == 5) || g.crossings.is_empty());
}

#[test]
fn a_crossing_ends_stretches() {
    // a T: the side road meets the middle of a straight road
    let g = build_graph(&[
        road(1, &[1, 2, 3], &[(0.0, 0.0), (100.0, 0.0), (200.0, 0.0)]),
        road(2, &[2, 4], &[(100.0, 0.0), (100.0, 100.0)]),
    ]);
    assert_eq!(g.stretches.len(), 3);
    assert_eq!(ids(&g), vec!["1:1-2", "1:2-3", "2:2-4"]);
    let c = g.crossings.iter().find(|c| c.node == 2).unwrap();
    assert_eq!(c.stretches.len(), 3);
    assert!((c.point.0 - 100.0).abs() < 1e-9 && c.point.1.abs() < 1e-9);
}

#[test]
fn two_ways_between_the_same_crossings_stay_two_stretches() {
    let g = build_graph(&[
        road(1, &[1, 2, 9], &[(0.0, 0.0), (50.0, 20.0), (100.0, 0.0)]),
        road(2, &[1, 3, 9], &[(0.0, 0.0), (50.0, -20.0), (100.0, 0.0)]),
        road(3, &[1, 10], &[(0.0, 0.0), (-50.0, 0.0)]),
        road(4, &[9, 11], &[(100.0, 0.0), (150.0, 0.0)]),
    ]);
    let between: Vec<&str> = ids(&g).into_iter().filter(|i| i.ends_with("1-9")).collect();
    assert_eq!(between.len(), 2, "{:?}", ids(&g));
    assert_ne!(between[0], between[1]);
}

#[test]
fn family_change_splits() {
    let g = build_graph(&[
        feature(
            1,
            Family::Asf3,
            false,
            &[1, 2, 3],
            &[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)],
        ),
        feature(
            2,
            Family::City,
            false,
            &[3, 4, 5],
            &[(100.0, 0.0), (150.0, 0.0), (200.0, 0.0)],
        ),
    ]);
    assert_eq!(g.stretches.len(), 2);
    assert_eq!(g.crossings.len(), 1);
    assert_eq!(g.crossings[0].node, 3);
}

#[test]
fn bridge_splits() {
    let g = build_graph(&[
        feature(1, Family::Asf3, false, &[1, 2], &[(0.0, 0.0), (50.0, 0.0)]),
        feature(2, Family::Asf3, true, &[2, 3], &[(50.0, 0.0), (100.0, 0.0)]),
        feature(
            3,
            Family::Asf3,
            false,
            &[3, 4],
            &[(100.0, 0.0), (150.0, 0.0)],
        ),
    ]);
    assert_eq!(g.stretches.len(), 3);
    assert_eq!(g.stretches.iter().filter(|s| s.bridge).count(), 1);
}

#[test]
fn a_tunnel_splits() {
    let mut tunnel = road(2, &[2, 3], &[(50.0, 0.0), (100.0, 0.0)]);
    if let FeatureKind::Road { tunnel: t, .. } = &mut tunnel.kind {
        *t = true;
    }
    let g = build_graph(&[
        road(1, &[1, 2], &[(0.0, 0.0), (50.0, 0.0)]),
        tunnel,
        road(3, &[3, 4], &[(100.0, 0.0), (150.0, 0.0)]),
    ]);
    assert_eq!(g.stretches.len(), 3);
    assert_eq!(g.stretches.iter().filter(|s| s.tunnel).count(), 1);
}

#[test]
fn paths_are_not_in_the_graph() {
    let mut p = road(1, &[1, 2], &[(0.0, 0.0), (50.0, 0.0)]);
    p.kind = FeatureKind::Path { width_m: 1.6 };
    assert!(build_graph(&[p]).stretches.is_empty());
}

#[test]
fn a_t_has_one_through_road() {
    let g = build_graph(&[
        road(1, &[1, 2, 3], &[(0.0, 0.0), (100.0, 0.0), (200.0, 0.0)]),
        road(2, &[2, 4], &[(100.0, 0.0), (100.0, 100.0)]),
    ]);
    let c = chain_ids(&g);
    assert_eq!(
        c,
        vec![
            vec!["1:1-2".to_string(), "1:2-3".to_string()],
            vec!["2:2-4".to_string()]
        ]
    );
}

/// Three arms leave node 0 at the given compass headings (degrees, clockwise from north), 100 m long.
fn star(headings: &[f64], families: &[Family]) -> RoadGraph {
    let features: Vec<Feature> = headings
        .iter()
        .zip(families)
        .enumerate()
        .map(|(i, (h, f))| {
            let (dx, dy) = (h.to_radians().sin() * 100.0, h.to_radians().cos() * 100.0);
            let way = i64::try_from(i).unwrap() + 1;
            feature(way, *f, false, &[0, way * 10], &[(0.0, 0.0), (dx, dy)])
        })
        .collect();
    build_graph(&features)
}

#[test]
fn the_straighter_pair_is_the_through_road() {
    // west, and east turned 30 degrees one way or 60 the other
    let g = star(&[270.0, 120.0, 30.0], &[Family::Asf3; 3]);
    let c = chain_ids(&g);
    assert_eq!(c.len(), 2, "{c:?}");
    assert_eq!(c[0].len(), 2);
    let first: Vec<&str> = c[0].iter().map(|s| s.trim_start_matches('~')).collect();
    assert!(
        first.contains(&"1:0-10") && first.contains(&"2:0-20"),
        "{c:?}"
    );
    assert_eq!(c[1].len(), 1);
}

#[test]
fn no_through_road_above_45_degrees() {
    let g = star(&[0.0, 120.0, 240.0], &[Family::Asf3; 3]);
    let c = chain_ids(&g);
    assert_eq!(c.len(), 3, "{c:?}");
    assert!(c.iter().all(|chain| chain.len() == 1));
}

#[test]
fn different_families_are_not_through() {
    let g = star(
        &[270.0, 90.0, 0.0],
        &[Family::Asf3, Family::City, Family::Asf3],
    );
    let c = chain_ids(&g);
    assert_eq!(c.len(), 3, "{c:?}");
}

#[test]
fn chains_are_walked_in_either_direction() {
    // both ways end at node 5, so the chain has to run through one of them from its end to its start
    let g = build_graph(&[
        road(1, &[1, 5], &[(0.0, 0.0), (100.0, 0.0)]),
        road(2, &[3, 5], &[(200.0, 0.0), (100.0, 0.0)]),
        road(3, &[5, 4], &[(100.0, 0.0), (100.0, 100.0)]),
    ]);
    let c = chain_ids(&g);
    assert_eq!(c.len(), 2, "{c:?}");
    assert_eq!(c[0].len(), 2);
    assert!(
        c[0].iter().filter(|s| s.starts_with('~')).count() == 1,
        "{c:?}"
    );
}

#[test]
fn chains_run_family_by_family_longest_first() {
    let g = build_graph(&[
        feature(1, Family::Mud, false, &[1, 2], &[(0.0, 0.0), (500.0, 0.0)]),
        feature(
            2,
            Family::Asf1,
            false,
            &[3, 4],
            &[(0.0, 100.0), (50.0, 100.0)],
        ),
        feature(
            3,
            Family::Asf3,
            false,
            &[5, 6],
            &[(0.0, 200.0), (50.0, 200.0)],
        ),
        feature(
            4,
            Family::City,
            false,
            &[7, 8],
            &[(0.0, 300.0), (80.0, 300.0)],
        ),
        feature(
            5,
            Family::Asf3,
            false,
            &[9, 10],
            &[(0.0, 400.0), (400.0, 400.0)],
        ),
    ]);
    let order: Vec<i64> = chains(&g)
        .iter()
        .map(|c| {
            let s = &g.stretches[c.links[0].stretch];
            s.id.split(':').next().unwrap().parse().unwrap()
        })
        .collect();
    // Asf1, City, then Asf3 longest first (way 5 before way 3), then Mud
    assert_eq!(order, vec![2, 4, 5, 3, 1]);
}

#[test]
fn a_ring_is_one_chain_that_ends_where_it_starts() {
    let g = build_graph(&[road(
        1,
        &[1, 2, 3, 4, 1],
        &[
            (0.0, 0.0),
            (100.0, 0.0),
            (100.0, 100.0),
            (0.0, 100.0),
            (0.0, 0.0),
        ],
    )]);
    assert_eq!(g.stretches.len(), 1);
    assert_eq!(g.stretches[0].start, g.stretches[0].end);
    let c = chains(&g);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].links.len(), 1);
}

#[test]
fn a_ring_with_a_tail_terminates() {
    let g = build_graph(&[
        road(
            1,
            &[1, 2, 3, 1],
            &[(0.0, 0.0), (100.0, 0.0), (50.0, 80.0), (0.0, 0.0)],
        ),
        road(2, &[1, 9], &[(0.0, 0.0), (-100.0, 0.0)]),
    ]);
    assert_eq!(g.stretches.len(), 2);
    let c = chains(&g);
    assert!(!c.is_empty());
    let total: usize = c.iter().map(|c| c.links.len()).sum();
    assert_eq!(total, 2);
}

/// A small network: a main road with a side road, a ring and a bridge.
fn network() -> Vec<Feature> {
    vec![
        road(
            1,
            &[1, 2, 3, 4],
            &[(0.0, 0.0), (100.0, 0.0), (200.0, 0.0), (300.0, 0.0)],
        ),
        road(
            2,
            &[3, 5, 6],
            &[(200.0, 0.0), (200.0, 80.0), (200.0, 160.0)],
        ),
        feature(
            3,
            Family::City,
            false,
            &[6, 7, 8, 6],
            &[
                (200.0, 160.0),
                (260.0, 200.0),
                (200.0, 240.0),
                (200.0, 160.0),
            ],
        ),
        feature(
            4,
            Family::Asf3,
            true,
            &[4, 9],
            &[(300.0, 0.0), (350.0, 0.0)],
        ),
        road(5, &[10, 2], &[(100.0, -100.0), (100.0, 0.0)]),
    ]
}

proptest! {
    #[test]
    fn ids_do_not_depend_on_input_order(seed in any::<u64>()) {
        let base = network();
        let a = build_graph(&base);
        let mut shuffled = base.clone();
        // a deterministic shuffle from the seed
        let mut state = seed | 1;
        for i in (1..shuffled.len()).rev() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let j = usize::try_from(state % u64::try_from(i + 1).unwrap()).unwrap();
            shuffled.swap(i, j);
        }
        // reversing a way's stored direction changes nothing either
        if seed % 2 == 0 {
            for f in &mut shuffled {
                if f.way_id == 2 {
                    f.points.reverse();
                    f.nodes.reverse();
                }
            }
        }
        let b = build_graph(&shuffled);
        prop_assert_eq!(&a, &b);
        prop_assert_eq!(chain_ids(&a), chain_ids(&b));
    }
}

// --- bridge spans and widening --------------------------------------------------------------------------------

fn len(s: &Stretch) -> f64 {
    s.points
        .windows(2)
        .map(|p| (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1))
        .sum()
}

fn by_id<'a>(g: &'a RoadGraph, id: &str) -> &'a Stretch {
    g.stretches.iter().find(|s| s.id == id).unwrap()
}

fn total(g: &RoadGraph) -> f64 {
    g.stretches.iter().map(len).sum()
}

fn line_200() -> RoadGraph {
    build_graph(&[road(1, &[1, 2], &[(0.0, 0.0), (200.0, 0.0)])])
}

#[test]
fn a_span_becomes_a_bridge_stretch_with_a_stable_id() {
    let g = line_200();
    let id = g.stretches[0].id.clone();
    let out = with_bridge_spans(
        &g,
        &[BridgeSpan {
            stretch: 0,
            from_m: 80.0,
            to_m: 110.0,
        }],
    );
    let found: Vec<_> = out
        .stretches
        .iter()
        .map(|s| (s.id.clone(), s.bridge))
        .collect();
    assert!(found.contains(&(id.clone(), false)), "{found:?}");
    assert!(
        found.contains(&(format!("{id}/bridge0"), true)),
        "{found:?}"
    );
    assert!(
        found.contains(&(format!("{id}/after0"), false)),
        "{found:?}"
    );
    assert!((len(by_id(&out, &format!("{id}/bridge0"))) - 30.0).abs() < 1e-6);
    assert!((len(by_id(&out, &id)) - 80.0).abs() < 1e-6);
    assert!((total(&out) - 200.0).abs() < 1e-6);
    // the pieces meet at crossings of two stretches
    assert_eq!(out.crossings.len(), 2);
    assert!(out.crossings.iter().all(|c| c.stretches.len() == 2));
}

#[test]
fn a_second_span_does_not_rename_the_first() {
    let g = line_200();
    let id = g.stretches[0].id.clone();
    let span = |from_m, to_m| BridgeSpan {
        stretch: 0,
        from_m,
        to_m,
    };
    let one = with_bridge_spans(&g, &[span(80.0, 110.0)]);
    let two = with_bridge_spans(&g, &[span(80.0, 110.0), span(150.0, 170.0)]);
    for name in [id.clone(), format!("{id}/bridge0")] {
        assert_eq!(by_id(&one, &name).points, by_id(&two, &name).points);
    }
    let after0 = by_id(&two, &format!("{id}/after0"));
    assert!((len(after0) - 40.0).abs() < 1e-6, "{}", len(after0));
    assert!(by_id(&two, &format!("{id}/bridge1")).bridge);
    assert!((len(by_id(&two, &format!("{id}/after1"))) - 30.0).abs() < 1e-6);
}

#[test]
fn overlapping_spans_are_merged() {
    let g = line_200();
    let id = g.stretches[0].id.clone();
    let span = |from_m, to_m| BridgeSpan {
        stretch: 0,
        from_m,
        to_m,
    };
    let out = with_bridge_spans(&g, &[span(100.0, 130.0), span(80.0, 110.0)]);
    assert!((len(by_id(&out, &format!("{id}/bridge0"))) - 50.0).abs() < 1e-6);
    assert_eq!(out.stretches.len(), 3);
}

#[test]
fn a_span_stops_short_of_a_junction_and_of_a_dead_end() {
    // a T: the main road 1-2-3 and a side road from node 2... here the end node 3 is a 3-way crossing
    let g = build_graph(&[
        road(1, &[1, 3], &[(0.0, 0.0), (100.0, 0.0)]),
        road(2, &[3, 4], &[(100.0, 0.0), (200.0, 0.0)]),
        road(3, &[3, 5], &[(100.0, 0.0), (100.0, 100.0)]),
    ]);
    let first = g
        .stretches
        .iter()
        .position(|s| s.id.starts_with("1:"))
        .unwrap();
    let id = g.stretches[first].id.clone();
    let out = with_bridge_spans(
        &g,
        &[BridgeSpan {
            stretch: first,
            from_m: -5.0,
            to_m: 150.0,
        }],
    );
    let bridge = by_id(&out, &format!("{id}/bridge0"));
    // 1 m in from the dead end at node 1 and 1 m short of the 3-way crossing
    assert!((len(bridge) - 98.0).abs() < 1e-6, "{}", len(bridge));
    assert!((bridge.points[0].0 - 1.0).abs() < 1e-6);
    assert!((bridge.points.last().unwrap().0 - 99.0).abs() < 1e-6);
    assert!((len(by_id(&out, &id)) - 1.0).abs() < 1e-6);
    assert!((len(by_id(&out, &format!("{id}/after0"))) - 1.0).abs() < 1e-6);
}

#[test]
fn a_span_may_reach_a_crossing_of_two_stretches() {
    let g = build_graph(&[
        road(1, &[1, 2], &[(0.0, 0.0), (100.0, 0.0)]),
        feature(
            2,
            Family::Grav,
            false,
            &[2, 3],
            &[(100.0, 0.0), (200.0, 0.0)],
        ),
    ]);
    let out = with_bridge_spans(
        &g,
        &[BridgeSpan {
            stretch: 0,
            from_m: 40.0,
            to_m: 100.0,
        }],
    );
    let bridge = out.stretches.iter().find(|s| s.bridge).unwrap();
    assert!((len(bridge) - 60.0).abs() < 1e-6);
}

/// 100 m road, a bridge of `bridge_m`, 100 m road, in one line, as three stretches (the family changes).
fn road_bridge_road(bridge_m: f64) -> RoadGraph {
    build_graph(&[
        road(1, &[1, 2], &[(0.0, 0.0), (100.0, 0.0)]),
        feature(
            2,
            Family::Asf3,
            true,
            &[2, 3],
            &[(100.0, 0.0), (100.0 + bridge_m, 0.0)],
        ),
        road(
            3,
            &[3, 4],
            &[(100.0 + bridge_m, 0.0), (200.0 + bridge_m, 0.0)],
        ),
    ])
}

fn bridge_len(g: &RoadGraph) -> f64 {
    g.stretches.iter().filter(|s| s.bridge).map(len).sum()
}

#[test]
fn a_short_osm_bridge_grows_into_its_neighbours() {
    let g = road_bridge_road(8.0);
    let out = widen_bridges(&g, &|_| 25.0);
    assert!((bridge_len(&out) - 25.0).abs() < 0.01);
    assert!((total(&out) - 208.0).abs() < 0.01);
    assert_eq!(ids(&out), ids(&g));
    // evenly: 8.5 m from each side
    let b = out.stretches.iter().find(|s| s.bridge).unwrap();
    assert!((b.points[0].0 - 91.5).abs() < 0.01, "{:?}", b.points);
    assert!((b.points.last().unwrap().0 - 116.5).abs() < 0.01);
    // the crossings moved with it
    assert!(
        out.crossings
            .iter()
            .any(|c| (c.point.0 - 91.5).abs() < 0.01)
    );
    // a bridge that is long enough is left alone
    assert_eq!(widen_bridges(&g, &|_| 8.0), g);
}

#[test]
fn a_bridge_between_two_junctions_grows_only_as_far_as_they_allow() {
    // the road before the bridge is 12 m long and ends at a 3-way crossing (node 1 has a side road)
    let g = build_graph(&[
        road(1, &[1, 2], &[(0.0, 0.0), (12.0, 0.0)]),
        road(9, &[1, 8], &[(0.0, 0.0), (0.0, 50.0)]),
        road(7, &[1, 6], &[(0.0, 0.0), (0.0, -50.0)]),
        feature(2, Family::Asf3, true, &[2, 3], &[(12.0, 0.0), (20.0, 0.0)]),
        road(3, &[3, 4], &[(20.0, 0.0), (27.0, 0.0)]),
    ]);
    // needed 25: 17 m missing, 11 m available before and 6 m after: both give all they can
    let out = widen_bridges(&g, &|_| 25.0);
    assert!(
        (bridge_len(&out) - 25.0).abs() < 0.01,
        "{}",
        bridge_len(&out)
    );
    let before = out
        .stretches
        .iter()
        .find(|s| s.id.starts_with("1:"))
        .unwrap();
    assert!((len(before) - 1.0).abs() < 0.01, "{}", len(before));
    let after = out
        .stretches
        .iter()
        .find(|s| s.id.starts_with("3:"))
        .unwrap();
    assert!((len(after) - 1.0).abs() < 0.01, "{}", len(after));
    // asking for more than they allow together leaves no bridge
    assert!(
        widen_bridges(&g, &|_| 26.0)
            .stretches
            .iter()
            .all(|s| !s.bridge)
    );
}

#[test]
fn a_bridge_without_room_stays_an_ordinary_road_and_its_neighbours_stay() {
    // 3-way crossings 12 m before and 12 m after: 11 + 11 + 8 < 40
    let g = build_graph(&[
        road(1, &[1, 2], &[(0.0, 0.0), (12.0, 0.0)]),
        road(8, &[1, 5], &[(0.0, 0.0), (0.0, 50.0)]),
        road(7, &[1, 6], &[(0.0, 0.0), (0.0, -50.0)]),
        feature(2, Family::Asf3, true, &[2, 3], &[(12.0, 0.0), (20.0, 0.0)]),
        road(3, &[3, 4], &[(20.0, 0.0), (32.0, 0.0)]),
        road(9, &[4, 10], &[(32.0, 0.0), (32.0, 50.0)]),
        road(10, &[4, 11], &[(32.0, 0.0), (32.0, -50.0)]),
    ]);
    let out = widen_bridges(&g, &|_| 40.0);
    assert!(out.stretches.iter().all(|s| !s.bridge));
    for s in &g.stretches {
        if !s.bridge {
            assert_eq!(by_id(&out, &s.id).points, s.points);
        }
    }
}

#[test]
fn a_neighbour_is_never_used_up() {
    let g = road_bridge_road(8.0);
    // the roads are 100 m: asking for 400 m leaves 1 m of each and fails
    let out = widen_bridges(&g, &|_| 400.0);
    assert!(out.stretches.iter().all(|s| !s.bridge));
    let out = widen_bridges(&g, &|_| 206.0);
    assert!((bridge_len(&out) - 206.0).abs() < 0.01);
    assert!(
        out.stretches
            .iter()
            .filter(|s| !s.bridge)
            .all(|s| len(s) >= 1.0 - 1e-9)
    );
}

#[test]
fn a_bridge_that_a_dip_bridge_covers_is_not_lost() {
    use naves_core::roadnet::lost_bridges;
    let mut osm = line_200();
    osm.stretches[0].bridge = true;
    let id = osm.stretches[0].id.clone();
    // the OSM bridge stayed a road in the first round
    let mut road = osm.clone();
    road.stretches[0].bridge = false;
    assert_eq!(lost_bridges(&osm, &road), vec![id.as_str()]);
    // a dip bridge cut from the middle, or covering all of it, is a bridge after all
    for (from_m, to_m) in [(80.0, 110.0), (0.0, 200.0)] {
        let span = BridgeSpan {
            stretch: 0,
            from_m,
            to_m,
        };
        let out = with_bridge_spans(&road, &[span]);
        assert!(lost_bridges(&osm, &out).is_empty(), "{from_m}");
    }
    // and one that stays a bridge is kept
    assert!(lost_bridges(&osm, &osm).is_empty());
}

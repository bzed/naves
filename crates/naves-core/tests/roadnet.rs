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
use naves_core::roadnet::{RoadGraph, build_graph, chains};
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

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The road graph: stretches from crossing to crossing, and the chains of through roads.

#![expect(
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::indexing_slicing,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::catalog::Family;
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::lot::Paving;
use naves_core::roadnet::{
    BridgeSpan, DUPLICATE_MAX_HEADING_DEG, DUPLICATE_MIN_M, RoadGraph, Stretch, build_graph,
    chains, drop_duplicates, over_crossings, overlay_rank, rings, with_bridge_spans,
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
            roundabout: false,
            layer: 0,
            use_sidepath: false,
            paving: None,
            held: false,
            plan: false,
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
fn held_part_is_its_own_stretch() {
    let mut held = road(2, &[3, 4, 5], &[(100.0, 0.0), (150.0, 0.0), (200.0, 0.0)]);
    if let FeatureKind::Road { held, .. } = &mut held.kind {
        *held = true;
    }
    let g = build_graph(&[
        road(1, &[1, 2, 3], &[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)]),
        held,
    ]);
    assert_eq!(g.stretches.len(), 2);
    let flags: Vec<(bool, bool)> = g.stretches.iter().map(|s| (s.held, s.plan)).collect();
    assert_eq!(flags, vec![(false, false), (true, false)]);
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
    p.kind = FeatureKind::Path {
        width_m: 1.6,
        street: false,
    };
    assert_eq!(build_graph(&[p]).stretches.len(), 0);
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
    assert_ne!(c.len(), 0);
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

#[test]
fn a_span_keeps_a_metre_before_a_bend_at_a_crossing_of_two_stretches() {
    // the road after node 2 turns by 30 degrees at once: no straight road to reach into
    let (s30, c30) = 30.0_f64.to_radians().sin_cos();
    let g = build_graph(&[
        road(1, &[1, 2], &[(0.0, 0.0), (100.0, 0.0)]),
        feature(
            2,
            Family::Grav,
            false,
            &[2, 3],
            &[(100.0, 0.0), (100.0 + 100.0 * c30, 100.0 * s30)],
        ),
    ]);
    let span = BridgeSpan {
        stretch: 0,
        from_m: 40.0,
        to_m: 100.0,
    };
    let out = with_bridge_spans(&g, std::slice::from_ref(&span));
    let bridge = out.stretches.iter().find(|s| s.bridge).unwrap();
    assert!((len(bridge) - 59.0).abs() < 1e-6, "{}", len(bridge));
    // a stretch that is an OpenStreetMap bridge built from its own part is not cut
    let mut own = g.clone();
    own.stretches[0].bridge_kind = Some(naves_core::bridgekind::BridgeKind::Panels {
        count: 2,
        abreast: 1,
        scale: 1.0,
    });
    assert_eq!(with_bridge_spans(&own, &[span]), own);
}

#[test]
fn the_overlay_rank_goes_by_family_then_length_then_id() {
    use std::cmp::Ordering;
    // an Asf3 road of 100 m, a City road of 50 m, two Grav roads of 80 m and 80 m, none sharing a node
    let graph = build_graph(&[
        feature(1, Family::Asf3, false, &[1, 2], &[(0.0, 0.0), (100.0, 0.0)]),
        feature(
            2,
            Family::City,
            false,
            &[3, 4],
            &[(0.0, 500.0), (50.0, 500.0)],
        ),
        feature(
            3,
            Family::Grav,
            false,
            &[5, 6],
            &[(0.0, 900.0), (80.0, 900.0)],
        ),
        feature(
            4,
            Family::Grav,
            false,
            &[7, 8],
            &[(0.0, 1300.0), (80.0, 1300.0)],
        ),
        feature(
            5,
            Family::Grav,
            false,
            &[9, 10],
            &[(0.0, 1700.0), (120.0, 1700.0)],
        ),
    ]);
    let all = chains(&graph);
    let of = |way: &str| {
        all.iter()
            .find(|c| graph.stretches[c.links[0].stretch].id.starts_with(way))
            .unwrap()
    };
    let (asf3, city, grav3, grav4, grav5) = (of("1:"), of("2:"), of("3:"), of("4:"), of("5:"));
    // the family decides, not the length
    assert_eq!(overlay_rank(city, asf3, &graph), Ordering::Less);
    assert_eq!(overlay_rank(asf3, city, &graph), Ordering::Greater);
    assert_eq!(overlay_rank(asf3, grav3, &graph), Ordering::Less);
    // the same family: the longer road is higher
    assert_eq!(overlay_rank(grav5, grav3, &graph), Ordering::Less);
    // equal length: the lower stretch id
    assert_eq!(overlay_rank(grav3, grav4, &graph), Ordering::Less);
    assert_eq!(overlay_rank(grav4, grav3, &graph), Ordering::Greater);
    assert_eq!(overlay_rank(grav3, grav3, &graph), Ordering::Equal);
}

// ---- duplicate roads ----

fn way(id: i64, family: Family, nodes: &[i64], points: &[(f64, f64)]) -> Feature {
    feature(id, family, false, nodes, points)
}

#[test]
fn the_duplicate_limits_are_the_documented_ones() {
    assert!((DUPLICATE_MIN_M - 25.0).abs() < 1e-9);
    assert!((DUPLICATE_MAX_HEADING_DEG - 15.0).abs() < 1e-9);
}

#[test]
fn two_roads_drawn_on_top_of_each_other_keep_the_higher_ranked() {
    let g = build_graph(&[
        way(1, Family::Grav, &[1, 2], &[(0.0, 0.0), (200.0, 0.0)]),
        way(2, Family::Asf3, &[3, 4], &[(0.0, 1.0), (200.0, 1.0)]),
    ]);
    let (out, dropped) = drop_duplicates(&g);
    assert_eq!(ids(&out), vec!["2:3-4"]);
    assert_eq!(dropped.len(), 1);
    let d = &dropped[0];
    assert_eq!((d.stretch.as_str(), d.kept.as_str()), ("1:1-2", "2:3-4"));
    assert!(
        d.from_m.abs() < 1e-6 && (d.to_m - 200.0).abs() < 1e-6,
        "{d:?}"
    );
    assert!((d.at.0 - 100.0).abs() < 1.0 && d.at.1.abs() < 1.5, "{d:?}");
}

#[test]
fn a_parallel_road_further_than_half_the_width_is_kept() {
    // two asphalt 3 roads (7 m wide), 5 m apart
    let g = build_graph(&[
        way(1, Family::Asf3, &[1, 2], &[(0.0, 0.0), (200.0, 0.0)]),
        way(2, Family::Asf3, &[3, 4], &[(0.0, 5.0), (200.0, 5.0)]),
    ]);
    let (out, dropped) = drop_duplicates(&g);
    assert_eq!(dropped.len(), 0);
    assert_eq!(out, g);
}

#[test]
fn an_overlap_shorter_than_25_m_is_kept() {
    let g = build_graph(&[
        way(1, Family::Asf3, &[1, 2], &[(0.0, 0.0), (200.0, 0.0)]),
        way(2, Family::Grav, &[3, 4], &[(0.0, 1.0), (20.0, 1.0)]),
    ]);
    let (out, dropped) = drop_duplicates(&g);
    assert_eq!(dropped.len(), 0);
    assert_eq!(out, g);
}

#[test]
fn headings_more_than_15_degrees_apart_are_kept() {
    // asphalt 1 and 2 cross through the origin: within 4.5 m of each other for more than 25 m
    let crossing = |deg: f64| {
        let (s, c) = deg.to_radians().sin_cos();
        build_graph(&[
            way(1, Family::Asf1, &[1, 2], &[(-100.0, 0.0), (100.0, 0.0)]),
            way(
                2,
                Family::Asf2,
                &[3, 4],
                &[(-100.0 * c, -100.0 * s), (100.0 * c, 100.0 * s)],
            ),
        ])
    };
    let (_, near) = drop_duplicates(&crossing(10.0));
    assert_eq!(near.len(), 1, "10 degrees is one road drawn twice");
    let g = crossing(16.0);
    let (out, far) = drop_duplicates(&g);
    assert_eq!(far.len(), 0);
    assert_eq!(out, g);
}

#[test]
fn the_lower_road_goes_on_after_the_overlap() {
    let g = build_graph(&[
        way(1, Family::Asf1, &[1, 2], &[(0.0, 0.0), (200.0, 0.0)]),
        way(
            2,
            Family::Grav,
            &[3, 5, 4],
            &[(0.0, 1.0), (100.0, 1.0), (200.0, 100.0)],
        ),
    ]);
    let (out, dropped) = drop_duplicates(&g);
    assert_eq!(dropped.len(), 1);
    let d = &dropped[0];
    assert_eq!(d.stretch, "2:3-4");
    assert!(
        d.from_m.abs() < 1e-6 && d.to_m > 98.0 && d.to_m < 105.0,
        "{d:?}"
    );
    // the kept road is cut where the other leaves; the rest of the lower road ends there
    let rest = out
        .stretches
        .iter()
        .find(|s| s.id.starts_with("2:3-4"))
        .unwrap_or_else(|| panic!("the rest of the lower road: {:?}", ids(&out)));
    assert_eq!(rest.end, 4);
    let node = rest.start;
    let crossing = out.crossings.iter().find(|c| c.node == node).unwrap();
    assert_eq!(crossing.stretches.len(), 3, "{:?}", ids(&out));
    let kept: Vec<_> = out
        .stretches
        .iter()
        .filter(|s| s.id.starts_with("1:1-2"))
        .collect();
    assert_eq!(kept.len(), 2);
    let on_kept = (crossing.point.0 - 100.0).abs() < 5.0 && crossing.point.1.abs() < 1e-6;
    assert!(on_kept, "{:?}", crossing.point);
    assert!(
        rest.points
            .first()
            .is_some_and(|p| (p.0 - crossing.point.0).hypot(p.1 - crossing.point.1) < 1e-6)
    );
}

#[test]
fn equal_family_and_length_drops_the_higher_stretch_id_every_time() {
    for flip in [false, true] {
        let a = way(3, Family::Asf3, &[1, 2], &[(0.0, 0.0), (200.0, 0.0)]);
        let b = way(5, Family::Asf3, &[3, 4], &[(0.0, 0.0), (200.0, 0.0)]);
        let features = if flip { vec![b, a] } else { vec![a, b] };
        let (out, dropped) = drop_duplicates(&build_graph(&features));
        assert_eq!(ids(&out), vec!["3:1-2"], "flip {flip}");
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].stretch, "5:3-4");
        assert_eq!(dropped[0].kept, "3:1-2");
    }
}

#[test]
fn duplicated_bridges_are_placed_once() {
    let g = build_graph(&[
        feature(1, Family::Grav, true, &[1, 2], &[(0.0, 0.0), (100.0, 0.0)]),
        feature(2, Family::Asf3, true, &[3, 4], &[(0.0, 1.0), (100.0, 1.0)]),
    ]);
    let (out, dropped) = drop_duplicates(&g);
    assert_eq!(ids(&out), vec!["2:3-4"]);
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].stretch, "1:1-2");
    assert!(dropped[0].from_m.abs() < 1e-6 && (dropped[0].to_m - 100.0).abs() < 1e-6);
}

#[test]
fn a_bridge_is_not_compared_with_a_road() {
    let g = build_graph(&[
        feature(1, Family::Asf3, false, &[1, 2], &[(0.0, 0.0), (100.0, 0.0)]),
        feature(2, Family::Grav, true, &[3, 4], &[(0.0, 1.0), (100.0, 1.0)]),
    ]);
    let (out, dropped) = drop_duplicates(&g);
    assert_eq!(dropped.len(), 0);
    assert_eq!(out, g);
}

#[test]
fn cut_pieces_and_bridge_cuts_have_unique_ids() {
    let g = build_graph(&[
        way(1, Family::Asf1, &[1, 2], &[(0.0, 0.0), (200.0, 0.0)]),
        way(
            2,
            Family::Grav,
            &[3, 5, 4],
            &[(0.0, 1.0), (100.0, 1.0), (200.0, 100.0)],
        ),
    ]);
    let (out, dropped) = drop_duplicates(&g);
    assert_eq!(dropped.len(), 1);
    let spans: Vec<BridgeSpan> = (0..out.stretches.len())
        .map(|stretch| BridgeSpan {
            stretch,
            from_m: 20.0,
            to_m: 40.0,
        })
        .collect();
    let cut = with_bridge_spans(&out, &spans);
    let all = ids(&cut);
    let mut unique = all.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), all.len(), "{all:?}");
    assert!(
        ids(&out).iter().any(|i| i.contains("/rest")),
        "{:?}",
        ids(&out)
    );
}

/// A roundabout way: `n` points on a circle of `radius` metres, nodes `first..first + n`, closed when `closed`.
fn circle(way: i64, first: i64, n: i64, radius: f64, closed: bool, roundabout: bool) -> Feature {
    let around = if closed { n + 1 } else { n };
    let points: Vec<(f64, f64)> = (0..around)
        .map(|i| {
            let a = (i % n) as f64 / n as f64 * std::f64::consts::TAU;
            (1000.0 + radius * a.sin(), 1000.0 + radius * a.cos())
        })
        .collect();
    let nodes: Vec<i64> = (0..around).map(|i| first + i % n).collect();
    let mut f = feature(way, Family::City, false, &nodes, &points);
    if let FeatureKind::Road { roundabout: r, .. } = &mut f.kind {
        *r = roundabout;
    }
    f
}

/// A road from the ring node `node` (on the circle at index `index` of `n`) straight outwards.
fn spoke(way: i64, node: i64, index: i64, n: i64, radius: f64) -> Feature {
    let a = index as f64 / n as f64 * std::f64::consts::TAU;
    let at = |r: f64| (1000.0 + r * a.sin(), 1000.0 + r * a.cos());
    road(way, &[node, 9000 + way], &[at(radius), at(radius + 60.0)])
}

#[test]
fn a_closed_roundabout_is_one_ring() {
    let g = build_graph(&[
        circle(1, 100, 16, 25.0, true, true),
        spoke(2, 100, 0, 16, 25.0),
        spoke(3, 104, 4, 16, 25.0),
        spoke(4, 108, 8, 16, 25.0),
    ]);
    let found = rings(&g);
    assert_eq!(found.len(), 1, "{found:?}");
    let ring = &found[0];
    // the ring is cut at the three joining roads
    assert_eq!(ring.stretches.len(), 3);
    for i in &ring.stretches {
        assert!(g.stretches[*i].roundabout);
    }
    assert!((ring.centre.0 - 1000.0).abs() < 0.5 && (ring.centre.1 - 1000.0).abs() < 0.5);
    assert!((ring.radius - 25.0).abs() < 0.5, "{}", ring.radius);
    assert_eq!(ring.family, Family::City);
}

#[test]
fn a_ring_without_joining_roads_is_one_ring_of_one_stretch() {
    let g = build_graph(&[circle(1, 100, 16, 25.0, true, true)]);
    let found = rings(&g);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].stretches.len(), 1);
}

#[test]
fn an_open_or_cut_ring_stays_ordinary_roads() {
    // not closed: three quarters of a circle
    let open = build_graph(&[circle(1, 100, 16, 25.0, false, true)]);
    assert_eq!(rings(&open), []);
    // closed, but one way of it is not tagged as a roundabout
    let mut a = circle(1, 100, 16, 25.0, true, true);
    let b = {
        let (nodes, points) = (a.nodes.split_off(9), a.points.split_off(9));
        let mut b = circle(2, 100, 16, 25.0, true, false);
        b.nodes = [vec![*a.nodes.last().unwrap()], nodes].concat();
        b.points = [vec![*a.points.last().unwrap()], points].concat();
        b
    };
    let cut = build_graph(&[a, b]);
    assert_eq!(rings(&cut), []);
    // a ring that a third roundabout way leaves is not a simple loop
    let branch = build_graph(&[circle(1, 100, 16, 25.0, true, true), {
        let mut f = spoke(2, 100, 0, 16, 25.0);
        if let FeatureKind::Road { roundabout, .. } = &mut f.kind {
            *roundabout = true;
        }
        f
    }]);
    assert_eq!(rings(&branch), []);
    // the roads are still there as ordinary stretches
    assert_ne!(cut.stretches.len(), 0);
}

fn layered(way: i64, bridge: bool, layer: i8, nodes: &[i64], points: &[(f64, f64)]) -> Feature {
    let mut f = feature(way, Family::Asf3, bridge, nodes, points);
    if let FeatureKind::Road { layer: l, .. } = &mut f.kind {
        *l = layer;
    }
    f
}

#[test]
fn a_bridge_over_a_road_is_found_without_a_shared_node() {
    // the road runs east, the bridge north over it, at (100, 0); the bridge is the second way
    let features = [
        road(1, &[1, 2], &[(-100.0, 0.0), (100.0, 0.0)]),
        layered(2, true, 1, &[3, 4], &[(100.0, -60.0), (100.0, 40.0)]),
    ];
    let g = build_graph(&features);
    let found = over_crossings(&g);
    assert_eq!(found.len(), 1);
    let c = &found[0];
    assert!(g.stretches[c.upper].bridge && !g.stretches[c.lower].bridge);
    assert!((c.at.0 - 100.0).abs() < 1e-6 && c.at.1.abs() < 1e-6);
    assert!((c.upper_s - 60.0).abs() < 1e-6, "{}", c.upper_s);
    assert!((c.lower_s - 200.0).abs() < 1e-6, "{}", c.lower_s);
}

#[test]
fn an_over_crossing_has_the_angle_between_the_lines_whichever_way_they_run() {
    // square: a right angle; at 45 degrees: a quarter of a right angle either way round
    for (dir, expected) in [
        ((0.0, 1.0), std::f64::consts::FRAC_PI_2),
        ((0.0, -1.0), std::f64::consts::FRAC_PI_2),
        ((1.0, 1.0), std::f64::consts::FRAC_PI_4),
        ((-1.0, 1.0), std::f64::consts::FRAC_PI_4),
        ((1.0, -1.0), std::f64::consts::FRAC_PI_4),
    ] {
        let features = [
            road(1, &[1, 2], &[(-100.0, 0.0), (100.0, 0.0)]),
            layered(
                2,
                true,
                1,
                &[3, 4],
                &[(-50.0 * dir.0, -50.0 * dir.1), (50.0 * dir.0, 50.0 * dir.1)],
            ),
        ];
        let found = over_crossings(&build_graph(&features));
        assert_eq!(found.len(), 1);
        assert!(
            (found[0].angle - expected).abs() < 1e-9,
            "{dir:?}: {}",
            found[0].angle
        );
    }
}

#[test]
fn a_road_that_leaves_the_bridges_node_and_only_forks_from_it_is_no_crossing() {
    // the bridge runs east from node 2 at the origin to node 3 at (40, 0); a road leaves node 3 at 8, 30, 59 and
    // 90 degrees from the bridge's line, back onto the deck or away from it: the two meet at the node and part
    let bridge = layered(1, true, 1, &[2, 3], &[(0.0, 0.0), (40.0, 0.0)]);
    for degrees in [8.0_f64, 30.0, 59.0, 90.0, 172.0] {
        let a = degrees.to_radians();
        let to = (40.0 - 100.0 * a.cos(), -100.0 * a.sin());
        let g = build_graph(&[bridge.clone(), road(2, &[3, 4], &[(40.0, 0.0), to])]);
        assert_eq!(over_crossings(&g).len(), 0, "{degrees}");
    }
}

#[test]
fn a_road_that_leaves_the_bridges_node_loops_and_crosses_its_line_is_found_where_it_crosses() {
    // the bridge runs east from node 2; the road leaves the same node to the north, turns east and south and crosses
    // the bridge's line 8 m along it, square
    let g = build_graph(&[
        layered(1, true, 1, &[2, 3], &[(0.0, 0.0), (40.0, 0.0)]),
        feature(
            2,
            Family::Mud,
            false,
            &[2, 11, 12, 13],
            &[(0.0, 0.0), (0.0, 20.0), (8.0, 20.0), (8.0, -20.0)],
        ),
    ]);
    let found = over_crossings(&g);
    assert_eq!(found.len(), 1, "{found:?}");
    let c = &found[0];
    assert!(g.stretches[c.upper].bridge && !g.stretches[c.lower].bridge);
    assert!((c.upper_s - 8.0).abs() < 1e-6, "{}", c.upper_s);
    assert!((c.at.0 - 8.0).abs() < 1e-6 && c.at.1.abs() < 1e-6);
    assert!((c.angle - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
}

#[test]
fn roads_crossing_at_a_shared_node_are_not_an_over_crossing() {
    let features = [
        road(1, &[1, 2, 3], &[(-100.0, 0.0), (0.0, 0.0), (100.0, 0.0)]),
        layered(
            2,
            true,
            1,
            &[4, 2, 5],
            &[(0.0, -50.0), (0.0, 0.0), (0.0, 50.0)],
        ),
    ];
    assert_eq!(over_crossings(&build_graph(&features)).len(), 0);
}

#[test]
fn an_untagged_crossing_or_a_tunnel_is_not_an_over_crossing() {
    let a = road(1, &[1, 2], &[(-100.0, 0.0), (100.0, 0.0)]);
    let b = road(2, &[3, 4], &[(0.0, -50.0), (0.0, 50.0)]);
    assert_eq!(over_crossings(&build_graph(&[a.clone(), b])).len(), 0);
    let mut t = layered(2, false, -1, &[3, 4], &[(0.0, -50.0), (0.0, 50.0)]);
    if let FeatureKind::Road { tunnel, .. } = &mut t.kind {
        *tunnel = true;
    }
    assert_eq!(over_crossings(&build_graph(&[a, t])).len(), 0);
}

#[test]
fn of_two_bridges_the_higher_layer_is_the_upper() {
    let features = [
        layered(1, true, 1, &[1, 2], &[(-100.0, 0.0), (100.0, 0.0)]),
        layered(2, true, 2, &[3, 4], &[(0.0, -50.0), (0.0, 50.0)]),
    ];
    let g = build_graph(&features);
    let found = over_crossings(&g);
    assert_eq!(found.len(), 1);
    assert_eq!(g.stretches[found[0].upper].layer, 2);
}

/// One way: 100 m east, a U-turn of radius 4 m to the north, 100 m west. Its nodes are 1 to the number
/// of points.
fn u_turn_way() -> Feature {
    let mut points: Vec<(f64, f64)> = (0..=10)
        .map(|i| (-100.0 + 10.0 * f64::from(i), 0.0))
        .collect();
    points.extend((1..30).map(|i| {
        let t = std::f64::consts::PI * f64::from(i) / 30.0;
        (4.0 * t.sin(), 4.0 - 4.0 * t.cos())
    }));
    points.extend((0..=10).map(|i| (-10.0 * f64::from(i), 8.0)));
    let nodes: Vec<i64> = (1..=i64::try_from(points.len()).unwrap()).collect();
    road(1, &nodes, &points)
}

#[test]
fn a_bridge_span_that_starts_in_a_bend_keeps_the_road_one_chain() {
    let g = build_graph(&[u_turn_way()]);
    assert_eq!(g.stretches.len(), 1);
    // the middle of the bend: the road turns by 57 degrees on each side within 4 m
    let apex = 100.0 + 2.0 * std::f64::consts::PI;
    let out = with_bridge_spans(
        &g,
        &[BridgeSpan {
            stretch: 0,
            from_m: apex,
            to_m: apex + 40.0,
        }],
    );
    assert_eq!(out.stretches.len(), 3, "{:?}", ids(&out));
    let c = chain_ids(&out);
    assert_eq!(c.len(), 1, "{c:?}");
    assert_eq!(c[0].len(), 3, "{c:?}");
}

#[test]
fn two_dead_ends_drawn_into_one_node_stay_apart() {
    // two arms of one family that leave the node 10 degrees apart: a bend of 170 degrees
    // (the second one is a bridge, so that the way is not simply one stretch)
    let g = build_graph(&[
        feature(
            1,
            Family::Asf3,
            false,
            &[0, 10],
            &[(0.0, 0.0), (0.0, 100.0)],
        ),
        feature(2, Family::Asf3, true, &[0, 20], &[(0.0, 0.0), (17.4, 98.5)]),
    ]);
    let c = chain_ids(&g);
    assert_eq!(c.len(), 2, "{c:?}");
}

// ---- the sidepath tags ----

/// A road feature, with `foot=use_sidepath` when `use_sidepath`.
fn tagged(way: i64, nodes: &[i64], points: &[(f64, f64)], use_sidepath: bool) -> Feature {
    let mut f = road(way, nodes, points);
    if let FeatureKind::Road {
        use_sidepath: u, ..
    } = &mut f.kind
    {
        *u = use_sidepath;
    }
    f
}

#[test]
fn the_use_sidepath_tag_reaches_the_stretch_and_does_not_cut_it() {
    let g = build_graph(&[road(1, &[1, 2], &[(0.0, 0.0), (50.0, 0.0)])]);
    assert!(!g.stretches[0].use_sidepath);
    // two ways, one tagged, joined at a plain joint: one stretch that carries the tag
    let g = build_graph(&[
        tagged(1, &[1, 2], &[(0.0, 0.0), (50.0, 0.0)], true),
        tagged(2, &[2, 3], &[(50.0, 0.0), (100.0, 0.0)], false),
    ]);
    assert_eq!(ids(&g), vec!["1:1-3"]);
    assert!(g.stretches[0].use_sidepath);
}

#[test]
fn a_cut_piece_keeps_the_use_sidepath_tag_of_its_stretch() {
    let g = build_graph(&[
        tagged(1, &[1, 2], &[(0.0, 0.0), (200.0, 0.0)], true),
        way(2, Family::Asf3, &[3, 4], &[(0.0, 1.0), (100.0, 1.0)]),
    ]);
    let (out, dropped) = drop_duplicates(&g);
    assert_eq!(dropped.len(), 1, "{out:?}");
    let rest: Vec<_> = out
        .stretches
        .iter()
        .filter(|s| s.id.starts_with("1:"))
        .collect();
    assert_ne!(rest.len(), 0);
    assert!(rest.iter().all(|s| s.use_sidepath), "{rest:?}");
}

// ---- the metres of a stretch whose way is tagged paved or unpaved ----

fn with_paving(mut f: Feature, paving: Paving) -> Feature {
    if let FeatureKind::Road { paving: p, .. } = &mut f.kind {
        *p = Some(paving);
    }
    f
}

/// The graph with the two surface fields cleared, to compare it with one built without the tags.
fn without_metres(g: &RoadGraph) -> RoadGraph {
    let mut g = g.clone();
    for s in &mut g.stretches {
        s.paved_m = 0.0;
        s.unpaved_m = 0.0;
    }
    g
}

#[test]
fn ways_of_one_stretch_count_their_own_metres() {
    let plain = [
        road(1, &[1, 2], &[(0.0, 0.0), (30.0, 0.0)]),
        road(2, &[2, 3], &[(30.0, 0.0), (50.0, 0.0)]),
    ];
    let tagged = [
        with_paving(plain[0].clone(), Paving::Paved),
        with_paving(plain[1].clone(), Paving::Unpaved),
    ];
    let g = build_graph(&tagged);
    assert_eq!(ids(&g), vec!["1:1-3"]);
    assert!(
        (g.stretches[0].paved_m - 30.0).abs() < 1e-6,
        "{:?}",
        g.stretches[0]
    );
    assert!(
        (g.stretches[0].unpaved_m - 20.0).abs() < 1e-6,
        "{:?}",
        g.stretches[0]
    );
    // the tags cut nothing and change nothing else
    assert_eq!(without_metres(&g), build_graph(&plain));
}

#[test]
fn a_way_without_the_tag_counts_for_neither() {
    let g = build_graph(&[
        with_paving(road(1, &[1, 2], &[(0.0, 0.0), (30.0, 0.0)]), Paving::Paved),
        road(2, &[2, 3], &[(30.0, 0.0), (50.0, 0.0)]),
    ]);
    assert_eq!(g.stretches.len(), 1);
    assert!((g.stretches[0].paved_m - 30.0).abs() < 1e-6);
    assert!(g.stretches[0].unpaved_m.abs() < 1e-6);
}

/// Every stretch of an all-asphalt graph has all of its metres paved, whatever was done to its line.
fn all_paved(g: &RoadGraph) {
    assert_ne!(g.stretches.len(), 0);
    for s in &g.stretches {
        let l = len(s);
        assert!(
            (s.paved_m - l).abs() < 1e-6,
            "{}: {} of {l}",
            s.id,
            s.paved_m
        );
        assert!(s.unpaved_m.abs() < 1e-9, "{}: {}", s.id, s.unpaved_m);
    }
}

#[test]
fn a_cut_stretch_shares_its_metres_in_proportion() {
    // 100 m paved, then 100 m unpaved, one stretch of 200 m
    let g = build_graph(&[
        with_paving(road(1, &[1, 2], &[(0.0, 0.0), (100.0, 0.0)]), Paving::Paved),
        with_paving(
            road(2, &[2, 3], &[(100.0, 0.0), (200.0, 0.0)]),
            Paving::Unpaved,
        ),
    ]);
    assert_eq!(g.stretches.len(), 1);
    let span = |from_m, to_m| BridgeSpan {
        stretch: 0,
        from_m,
        to_m,
    };
    let out = with_bridge_spans(&g, &[span(50.0, 150.0)]);
    assert_eq!(out.stretches.len(), 3);
    let (p, u): (f64, f64) = out
        .stretches
        .iter()
        .fold((0.0, 0.0), |a, s| (a.0 + s.paved_m, a.1 + s.unpaved_m));
    assert!(
        (p - 100.0).abs() < 1e-9 && (u - 100.0).abs() < 1e-9,
        "{p} {u}"
    );
    for s in &out.stretches {
        let share = len(s) / 200.0;
        assert!((s.paved_m - 100.0 * share).abs() < 1e-9, "{s:?}");
        assert!((s.unpaved_m - 100.0 * share).abs() < 1e-9, "{s:?}");
    }
}

#[test]
fn a_dropped_duplicate_leaves_the_metres_of_what_is_kept() {
    let paved = |f: Feature| with_paving(f, Paving::Paved);
    let g = build_graph(&[
        paved(way(1, Family::Grav, &[1, 2], &[(0.0, 0.0), (200.0, 0.0)])),
        paved(way(2, Family::Asf3, &[3, 4], &[(0.0, 1.0), (100.0, 1.0)])),
    ]);
    let (out, dropped) = drop_duplicates(&g);
    assert_eq!(dropped.len(), 1);
    assert!(out.stretches.len() >= 2);
    all_paved(&out);
}

#[test]
fn an_unpaved_stretch_stays_unpaved_when_cut() {
    let g = build_graph(&[with_paving(
        road(1, &[1, 2], &[(0.0, 0.0), (200.0, 0.0)]),
        Paving::Unpaved,
    )]);
    let out = with_bridge_spans(
        &g,
        &[BridgeSpan {
            stretch: 0,
            from_m: 80.0,
            to_m: 110.0,
        }],
    );
    for s in &out.stretches {
        assert!((s.unpaved_m - len(s)).abs() < 1e-9 && s.paved_m.abs() < 1e-9);
    }
}

#[test]
fn the_untagged_metres_are_what_no_way_tags_and_never_negative() {
    let g = build_graph(&[
        with_paving(road(1, &[1, 2], &[(0.0, 0.0), (30.0, 0.0)]), Paving::Paved),
        road(2, &[2, 3], &[(30.0, 0.0), (50.0, 0.0)]),
    ]);
    assert!((g.stretches[0].untagged_m() - 20.0).abs() < 1e-6);
    // rounding in the sums can leave a fully tagged stretch a hair over its length
    let mut s = g.stretches[0].clone();
    s.paved_m = len(&s) + 1e-13;
    s.unpaved_m = 0.0;
    assert!(s.untagged_m().abs() < 1e-12 && s.untagged_m() >= 0.0);
}

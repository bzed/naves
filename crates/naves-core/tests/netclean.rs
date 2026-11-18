// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The network clean-up: the components of a graph, the dead-end stubs and specks, and the junctions too close
//! to tell apart.

use naves_core::catalog::Family;
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::netclean::{CleanReport, clean, components, drop_stubs, merge_junctions};
use naves_core::roadnet::{RoadGraph, build_graph};

fn road(way: i64, nodes: &[i64], points: &[(f64, f64)]) -> Feature {
    Feature {
        id: format!("osm/way/{way}/0"),
        way_id: way,
        source: Source::Osm,
        kind: FeatureKind::Road {
            family: Family::Asf3,
            drivable: true,
            bridge: false,
            tunnel: false,
            roundabout: false,
            layer: 0,
            use_sidepath: false,
        },
        points: points.to_vec(),
        nodes: nodes.to_vec(),
    }
}

fn widths(f: Family) -> f64 {
    f.width_m()
}

#[test]
fn an_empty_graph_has_no_components() {
    assert_eq!(components(&RoadGraph::default()), 0);
}

#[test]
fn two_separate_roads_are_two_components() {
    let g = build_graph(&[
        road(1, &[1, 2], &[(0.0, 0.0), (50.0, 0.0)]),
        road(2, &[3, 4], &[(0.0, 100.0), (50.0, 100.0)]),
    ]);
    assert_eq!(components(&g), 2);
}

#[test]
fn a_t_is_one_component() {
    let g = build_graph(&[
        road(1, &[1, 2, 3], &[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)]),
        road(2, &[2, 4], &[(50.0, 0.0), (50.0, 50.0)]),
    ]);
    assert_eq!(g.stretches.len(), 3);
    assert_eq!(components(&g), 1);
}

#[test]
fn a_loop_of_one_stretch_is_one_component() {
    let g = build_graph(&[road(
        1,
        &[1, 2, 3, 1],
        &[(0.0, 0.0), (50.0, 0.0), (50.0, 50.0), (0.0, 0.0)],
    )]);
    assert_eq!(components(&g), 1);
}

#[test]
fn clean_with_empty_steps_returns_the_same_graph_and_an_empty_report() {
    let graphs = [
        RoadGraph::default(),
        build_graph(&[
            road(1, &[1, 2, 3], &[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)]),
            road(2, &[2, 4], &[(50.0, 0.0), (50.0, 50.0)]),
            road(3, &[5, 6], &[(0.0, 300.0), (50.0, 300.0)]),
        ]),
    ];
    for g in &graphs {
        let (out, report) = clean(g, &widths, 8.0);
        assert_eq!(&out, g);
        assert_eq!(
            report,
            CleanReport {
                components_before: components(g),
                components_after: components(g),
                ..CleanReport::default()
            }
        );
        assert!(report.components_after <= report.components_before);
    }
}

fn way(
    id: i64,
    nodes: &[i64],
    points: &[(f64, f64)],
    family: Family,
    drivable: bool,
    bridge: bool,
    roundabout: bool,
) -> Feature {
    Feature {
        kind: FeatureKind::Road {
            family,
            drivable,
            bridge,
            tunnel: false,
            roundabout,
            layer: 0,
            use_sidepath: false,
        },
        ..road(id, nodes, points)
    }
}

fn plain(id: i64, nodes: &[i64], points: &[(f64, f64)], family: Family) -> Feature {
    way(id, nodes, points, family, true, false, false)
}

fn stretch_ids(g: &RoadGraph) -> Vec<String> {
    g.stretches.iter().map(|s| s.id.clone()).collect()
}

// ---- stubs and specks ----

/// A main road with a node at x = 50 and a dead end of 4 m going north from it.
fn road_with_stub(stub: Feature) -> RoadGraph {
    build_graph(&[
        plain(
            1,
            &[1, 2, 3],
            &[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)],
            Family::Asf3,
        ),
        stub,
    ])
}

#[test]
fn a_short_dead_end_track_that_is_not_drivable_is_dropped_and_reported() {
    let g = road_with_stub(way(
        2,
        &[2, 4],
        &[(50.0, 0.0), (50.0, 4.0)],
        Family::Mud,
        false,
        false,
        false,
    ));
    let (out, dropped) = drop_stubs(&g, &widths);
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].stretch, "2:2-4");
    assert!((dropped[0].length_m - 4.0).abs() < 1e-9);
    assert!((dropped[0].at.0 - 50.0).abs() < 1e-9 && (dropped[0].at.1 - 2.0).abs() < 1e-9);
    assert_eq!(stretch_ids(&out), ["1:1-2", "1:2-3"]);
    assert_eq!(components(&out), components(&g));
}

#[test]
fn a_service_road_of_twenty_metres_stays() {
    let g = road_with_stub(way(
        2,
        &[2, 4],
        &[(50.0, 0.0), (50.0, 20.0)],
        Family::Mud,
        false,
        false,
        false,
    ));
    let (out, dropped) = drop_stubs(&g, &widths);
    assert_eq!(dropped, Vec::new());
    assert_eq!(out, g);
}

#[test]
fn a_drivable_dead_end_shorter_than_its_width_is_dropped_but_not_one_on_a_narrow_track() {
    let g = road_with_stub(plain(2, &[2, 4], &[(50.0, 0.0), (50.0, 4.0)], Family::Asf3));
    let (out, dropped) = drop_stubs(&g, &|_| 6.0);
    assert_eq!(dropped.len(), 1);
    assert_eq!(out.stretches.len(), 2);
    let (kept, none) = drop_stubs(&g, &|_| 3.0);
    assert_eq!(none, Vec::new());
    assert_eq!(kept, g);
}

#[test]
fn a_short_stretch_between_two_junctions_is_not_a_dead_end() {
    let g = build_graph(&[
        plain(
            1,
            &[1, 2, 3],
            &[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)],
            Family::Asf3,
        ),
        plain(2, &[2, 5], &[(50.0, 0.0), (50.0, 40.0)], Family::Asf3),
        way(
            3,
            &[2, 6],
            &[(50.0, 0.0), (50.0, -4.0)],
            Family::Mud,
            false,
            false,
            false,
        ),
        way(
            4,
            &[6, 7],
            &[(50.0, -4.0), (80.0, -4.0)],
            Family::Mud,
            false,
            false,
            false,
        ),
        way(
            5,
            &[6, 8],
            &[(50.0, -4.0), (20.0, -4.0)],
            Family::Mud,
            false,
            false,
            false,
        ),
    ]);
    let (out, dropped) = drop_stubs(&g, &widths);
    assert_eq!(dropped, Vec::new());
    assert_eq!(out, g);
}

#[test]
fn a_short_stretch_that_stands_alone_is_not_dropped() {
    let g = build_graph(&[way(
        1,
        &[1, 2],
        &[(0.0, 0.0), (0.0, 4.0)],
        Family::Mud,
        false,
        false,
        false,
    )]);
    let (out, dropped) = drop_stubs(&g, &widths);
    assert_eq!(dropped, Vec::new());
    assert_eq!(out, g);
}

#[test]
fn a_bridge_or_a_roundabout_arm_is_not_a_stub() {
    let g = road_with_stub(way(
        2,
        &[2, 4],
        &[(50.0, 0.0), (50.0, 4.0)],
        Family::Asf3,
        false,
        true,
        false,
    ));
    assert_eq!(drop_stubs(&g, &widths).1, Vec::new());
}

#[test]
fn points_closer_than_half_a_metre_become_one_and_the_ends_stay() {
    let g = build_graph(&[plain(
        1,
        &[1, 11, 12, 13, 14, 2],
        &[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.2, 0.0),
            (30.0, 0.0),
            (49.9, 0.0),
            (50.0, 0.0),
        ],
        Family::Asf3,
    )]);
    let (out, dropped) = drop_stubs(&g, &widths);
    assert_eq!(dropped, Vec::new());
    assert_eq!(
        out.stretches[0].points,
        [(0.0, 0.0), (10.0, 0.0), (30.0, 0.0), (50.0, 0.0)]
    );
    assert_eq!((out.stretches[0].start, out.stretches[0].end), (1, 2));
}

#[test]
fn a_stretch_of_two_points_closer_than_half_a_metre_is_kept_whole() {
    let g = build_graph(&[
        plain(
            1,
            &[1, 2, 3, 4],
            &[(0.0, 0.0), (50.0, 0.0), (50.2, 0.0), (100.0, 0.0)],
            Family::Asf3,
        ),
        plain(2, &[2, 5], &[(50.0, 0.0), (50.0, 40.0)], Family::Asf3),
        plain(3, &[3, 6], &[(50.2, 0.0), (50.2, -40.0)], Family::Asf3),
    ]);
    let (out, dropped) = drop_stubs(&g, &widths);
    assert_eq!(dropped, Vec::new());
    assert_eq!(out, g);
}

#[test]
fn clean_counts_the_specks_it_removes() {
    let g = build_graph(&[plain(
        1,
        &[1, 11, 12, 2],
        &[(0.0, 0.0), (10.0, 0.0), (10.2, 0.0), (50.0, 0.0)],
        Family::Asf3,
    )]);
    let (_, report) = clean(&g, &widths, 8.0);
    assert_eq!(report.specks, 1);
}

// ---- junctions ----

fn merge_names(
    g: &RoadGraph,
) -> (
    RoadGraph,
    Vec<naves_core::netclean::MergedJunction>,
    Vec<naves_core::netclean::NotMerged>,
) {
    merge_junctions(g)
}

/// A main road with junction nodes at x = 50 (node 2, a side road north) and x = `gap` + 50 (node 3, one south).
fn two_ts(gap: f64) -> RoadGraph {
    build_graph(&[
        plain(
            1,
            &[1, 2, 3, 4],
            &[(0.0, 0.0), (50.0, 0.0), (50.0 + gap, 0.0), (150.0, 0.0)],
            Family::Asf1,
        ),
        plain(2, &[2, 5], &[(50.0, 0.0), (50.0, 40.0)], Family::Grav),
        plain(
            3,
            &[3, 60, 6],
            &[(50.0 + gap, 0.0), (50.0 + gap, -3.0), (50.0 + gap, -40.0)],
            Family::Grav,
        ),
    ])
}

fn assert_components_kept(before: &RoadGraph, after: &RoadGraph) {
    assert_eq!(components(after), components(before));
}

#[test]
fn two_t_junctions_four_metres_apart_become_one_four_way_node() {
    let g = two_ts(4.0);
    let (out, merged, not_merged) = merge_names(&g);
    assert_eq!(not_merged, Vec::new());
    assert_eq!(merged.len(), 1);
    assert_eq!((merged[0].kept, merged[0].merged.clone()), (2, vec![3]));
    assert_eq!(merged[0].removed, ["1:2-3"]);
    assert!((merged[0].at.0 - 50.0).abs() < 1e-9);
    assert_eq!(out.stretches.len(), 4);
    assert!(out.stretches.iter().all(|s| s.id != "1:2-3"));
    let node2 = out.crossings.iter().find(|c| c.node == 2).unwrap();
    assert_eq!(node2.stretches.len(), 4);
    assert!(out.crossings.iter().all(|c| c.node != 3));
    // the side road that was at node 3 now starts at the kept node, straight for its last 6 m
    let side = out.stretches.iter().find(|s| s.id == "3:3-6").unwrap();
    assert_eq!((side.start, side.end), (2, 6));
    assert_eq!(side.points, [(50.0, 0.0), (54.0, -40.0)]);
    assert_components_kept(&g, &out);
}

#[test]
fn the_merged_node_sits_on_the_node_whose_roads_rank_highest() {
    // node 2: two gravel roads and the connector; node 3: two motorway pieces and the connector
    let g = build_graph(&[
        plain(
            20,
            &[20, 2, 21],
            &[(50.0, -30.0), (50.0, 0.0), (50.0, 30.0)],
            Family::Grav,
        ),
        plain(
            21,
            &[22, 3, 23],
            &[(54.0, -30.0), (54.0, 0.0), (54.0, 30.0)],
            Family::Asf1,
        ),
        plain(22, &[2, 3], &[(50.0, 0.0), (54.0, 0.0)], Family::Mud),
    ]);
    let (out, merged, _) = merge_names(&g);
    assert_eq!((merged[0].kept, merged[0].merged.clone()), (3, vec![2]));
    assert!((merged[0].at.0 - 54.0).abs() < 1e-9);
    assert!(
        out.crossings
            .iter()
            .any(|c| c.node == 3 && c.stretches.len() == 4)
    );
    assert_components_kept(&g, &out);
}

#[test]
fn two_junctions_ten_metres_apart_stay() {
    let g = two_ts(10.0);
    let (out, merged, not_merged) = merge_names(&g);
    assert!(merged.is_empty() && not_merged.is_empty());
    assert_eq!(out, g);
}

fn row(count: i32, step: f64) -> RoadGraph {
    let mut nodes = vec![1];
    let mut points = vec![(0.0, 0.0)];
    let mut features = Vec::new();
    for i in 0..count {
        let k = i64::from(i);
        let x = 50.0 + step * f64::from(i);
        nodes.push(10 + k);
        points.push((x, 0.0));
        features.push(plain(
            100 + k,
            &[10 + k, 200 + k],
            &[(x, 0.0), (x, if k % 2 == 0 { 40.0 } else { -40.0 })],
            Family::Grav,
        ));
    }
    nodes.push(2);
    points.push((300.0, 0.0));
    features.push(plain(1, &nodes, &points, Family::Asf1));
    build_graph(&features)
}

#[test]
fn three_junctions_in_a_row_five_metres_apart_merge() {
    let g = row(3, 5.0);
    let (out, merged, not_merged) = merge_names(&g);
    assert_eq!(not_merged, Vec::new());
    assert_eq!(merged.len(), 1);
    assert_eq!(
        (merged[0].kept, merged[0].merged.clone()),
        (10, vec![11, 12])
    );
    assert_eq!(merged[0].removed.len(), 2);
    assert_eq!(out.stretches.len(), g.stretches.len() - 2);
    assert_components_kept(&g, &out);
}

#[test]
fn four_junctions_in_a_row_fifteen_metres_across_do_not_merge_and_are_reported() {
    let g = row(4, 5.0);
    let (out, merged, not_merged) = merge_names(&g);
    assert_eq!(merged, Vec::new());
    assert_eq!(not_merged.len(), 1);
    assert_eq!(not_merged[0].nodes, [10, 11, 12, 13]);
    assert_ne!(not_merged[0].reason, "");
    assert_eq!(out, g);
}

/// Node 2 at x = 50 and node 3 at x = 54 on a main road; `left` and `right` say which of the two pieces next to
/// them are bridges; a side road leaves each node.
fn bridge_ends(left: bool, right: bool) -> RoadGraph {
    build_graph(&[
        way(
            1,
            &[1, 2],
            &[(0.0, 0.0), (50.0, 0.0)],
            Family::Asf3,
            true,
            left,
            false,
        ),
        plain(2, &[2, 3], &[(50.0, 0.0), (54.0, 0.0)], Family::Asf3),
        way(
            3,
            &[3, 4],
            &[(54.0, 0.0), (100.0, 0.0)],
            Family::Asf3,
            true,
            right,
            false,
        ),
        plain(4, &[2, 5], &[(50.0, 0.0), (50.0, 40.0)], Family::Grav),
        plain(5, &[3, 6], &[(54.0, 0.0), (54.0, -40.0)], Family::Grav),
    ])
}

#[test]
fn a_cluster_with_one_bridge_end_merges_onto_it() {
    let g = bridge_ends(false, true);
    let (out, merged, not_merged) = merge_names(&g);
    assert_eq!(not_merged, Vec::new());
    assert_eq!((merged[0].kept, merged[0].merged.clone()), (3, vec![2]));
    let bridge = out.stretches.iter().find(|s| s.bridge).unwrap();
    assert_eq!(bridge.points.first(), Some(&(54.0, 0.0)));
    assert_components_kept(&g, &out);
}

#[test]
fn a_cluster_with_two_bridge_ends_is_reported_and_unchanged() {
    let g = bridge_ends(true, true);
    let (out, merged, not_merged) = merge_names(&g);
    assert_eq!(merged, Vec::new());
    assert_eq!(not_merged.len(), 1);
    assert_eq!(not_merged[0].nodes, [2, 3]);
    assert_eq!(out, g);
}

#[test]
fn a_roundabout_node_is_never_merged() {
    let g = build_graph(&[
        plain(
            1,
            &[1, 2, 3, 4],
            &[(0.0, 0.0), (50.0, 0.0), (54.0, 0.0), (100.0, 0.0)],
            Family::Asf1,
        ),
        way(
            2,
            &[2, 7, 8, 2],
            &[(50.0, 0.0), (50.0, 12.0), (44.0, 6.0), (50.0, 0.0)],
            Family::Asf1,
            true,
            false,
            true,
        ),
        plain(3, &[3, 6], &[(54.0, 0.0), (54.0, -40.0)], Family::Grav),
    ]);
    let (out, merged, not_merged) = merge_names(&g);
    assert_eq!(merged, Vec::new());
    assert_eq!(not_merged.len(), 1);
    assert_eq!(out, g);
}

#[test]
fn the_result_does_not_depend_on_the_order_of_the_stretches() {
    for g in [
        two_ts(4.0),
        row(3, 5.0),
        bridge_ends(false, true),
        row(4, 5.0),
    ] {
        let mut reversed = g.clone();
        reversed.stretches.reverse();
        let a = merge_junctions(&g);
        let b = merge_junctions(&reversed);
        let sorted = |mut v: Vec<naves_core::roadnet::Stretch>| {
            v.sort_by(|x, y| x.id.cmp(&y.id));
            v
        };
        assert_eq!(sorted(a.0.stretches), sorted(b.0.stretches));
        assert_eq!(a.1, b.1);
        assert_eq!(a.2, b.2);
    }
}

#[test]
fn a_stretch_that_runs_between_two_cluster_nodes_the_long_way_stops_the_merge() {
    let g = build_graph(&[
        plain(
            1,
            &[1, 2, 3, 4],
            &[(0.0, 0.0), (50.0, 0.0), (54.0, 0.0), (100.0, 0.0)],
            Family::Asf1,
        ),
        plain(
            2,
            &[2, 30, 31, 3],
            &[(50.0, 0.0), (50.0, 30.0), (54.0, 30.0), (54.0, 0.0)],
            Family::Grav,
        ),
    ]);
    let (out, merged, not_merged) = merge_names(&g);
    assert_eq!(merged, Vec::new());
    assert_eq!(not_merged.len(), 1);
    assert_eq!(out, g);
}

#[test]
fn degenerate_input_does_not_panic_and_changes_nothing() {
    let g = build_graph(&[
        plain(1, &[1, 2], &[(0.0, 0.0), (0.0, 0.0)], Family::Asf3),
        plain(2, &[2, 3], &[(f64::NAN, 0.0), (5.0, 0.0)], Family::Asf3),
        plain(3, &[2, 4], &[(0.0, 0.0), (0.0, 3.0)], Family::Asf3),
        plain(4, &[2, 5], &[(0.0, 0.0), (3.0, 0.0)], Family::Asf3),
        plain(5, &[6, 6], &[(9.0, 9.0), (9.0, 9.0)], Family::Asf3),
    ]);
    let _ = drop_stubs(&g, &widths);
    let _ = merge_junctions(&g);
    let _ = clean(&g, &widths, 8.0);
}

#[test]
fn clean_runs_the_stubs_and_the_merge_and_reports_both() {
    let g = build_graph(&[
        plain(
            1,
            &[1, 2, 3, 4],
            &[(0.0, 0.0), (50.0, 0.0), (54.0, 0.0), (150.0, 0.0)],
            Family::Asf1,
        ),
        plain(2, &[2, 5], &[(50.0, 0.0), (50.0, 40.0)], Family::Grav),
        plain(3, &[3, 6], &[(54.0, 0.0), (54.0, -40.0)], Family::Grav),
        way(
            4,
            &[4, 7],
            &[(150.0, 0.0), (150.0, 3.0)],
            Family::Mud,
            false,
            false,
            false,
        ),
    ]);
    let (out, report) = clean(&g, &widths, 8.0);
    assert_eq!(report.stubs.len(), 1);
    assert_eq!(report.merged.len(), 1);
    assert_eq!(report.components_before, report.components_after);
    assert_eq!(out.stretches.len(), g.stretches.len() - 2);
}

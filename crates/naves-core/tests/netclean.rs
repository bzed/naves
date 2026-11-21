// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The network clean-up: the components of a graph, the dead-end stubs and specks, and the junctions too close
//! to tell apart.

use naves_core::catalog::Family;
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::netclean::{
    ApproachKind, CleanReport, DroppedSpeck, KeptSidepath, clean, components, drop_sidepaths,
    drop_stubs, make_approaches, merge_junctions,
};
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
            paving: None,
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
fn a_graph_with_nothing_to_clean_comes_back_the_same_with_an_empty_report() {
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
                components_by_step: vec![components(g); 4],
                drivable_m_before: report.drivable_m_before,
                drivable_m_after: report.drivable_m_before,
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
            paving: None,
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
    assert_eq!(
        report.specks,
        [DroppedSpeck {
            stretch: "1:1-2".to_string(),
            at: (10.2, 0.0)
        }]
    );
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
fn degenerate_input_changes_no_junction_and_drops_only_the_short_dead_ends() {
    let g = build_graph(&[
        plain(1, &[1, 2], &[(0.0, 0.0), (0.0, 0.0)], Family::Asf3),
        plain(2, &[2, 3], &[(f64::NAN, 0.0), (5.0, 0.0)], Family::Asf3),
        plain(3, &[2, 4], &[(0.0, 0.0), (0.0, 3.0)], Family::Asf3),
        plain(4, &[2, 5], &[(0.0, 0.0), (3.0, 0.0)], Family::Asf3),
        plain(5, &[6, 6], &[(9.0, 9.0), (9.0, 9.0)], Family::Asf3),
    ]);
    let (out, dropped) = drop_stubs(&g, &widths);
    // the three short dead ends of node 2 (also the one of no length) go; the line with a NaN stays
    let ids: Vec<&str> = dropped.iter().map(|d| d.stretch.as_str()).collect();
    assert_eq!(ids, ["1:1-2", "3:2-4", "4:2-5"]);
    assert_eq!(out.stretches.len(), g.stretches.len() - 3);
    // (a line with a NaN is not equal to itself, so the graph is compared by its ids)
    let (same, merged, not_merged) = merge_junctions(&g);
    assert_eq!((merged.len(), not_merged.len()), (0, 0));
    assert_eq!(stretch_ids(&same), stretch_ids(&g));
    let (_, report) = clean(&g, &widths, 8.0);
    assert!(
        report
            .components_by_step
            .iter()
            .all(|&c| c <= report.components_before)
    );
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

// ---- fix round 1 ----

#[test]
fn a_stretch_that_goes_on_outside_the_map_is_not_a_stub() {
    // a negative node id: the road is cut at the edge of the map
    let g = road_with_stub(plain(
        2,
        &[2, -77],
        &[(50.0, 0.0), (50.0, 4.0)],
        Family::Mud,
    ));
    let (out, dropped) = drop_stubs(&g, &widths);
    assert_eq!(dropped, Vec::new());
    assert_eq!(out, g);
}

#[test]
fn a_stretch_that_was_not_a_dead_end_in_the_input_is_not_taken_with_the_ones_that_are() {
    // road - S1 (mud) - node 5 - S2 (gravel, dead end): S1 only becomes free when S2 goes
    let g = build_graph(&[
        plain(
            1,
            &[1, 2, 3],
            &[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)],
            Family::Asf3,
        ),
        way(
            3,
            &[2, 5],
            &[(50.0, 0.0), (50.0, 4.0)],
            Family::Mud,
            false,
            false,
            false,
        ),
        way(
            2,
            &[5, 6],
            &[(50.0, 4.0), (50.0, 8.0)],
            Family::Grav,
            false,
            false,
            false,
        ),
    ]);
    assert_eq!(g.stretches.len(), 4);
    let (out, dropped) = drop_stubs(&g, &widths);
    let ids: Vec<&str> = dropped.iter().map(|d| d.stretch.as_str()).collect();
    assert_eq!(ids, ["2:5-6"]);
    assert_eq!(out.stretches.len(), 3);
}

#[test]
fn of_three_short_pieces_in_a_star_two_go_and_the_last_one_stays_joined() {
    let g = build_graph(&[
        way(
            1,
            &[1, 2],
            &[(0.0, 0.0), (0.0, 4.0)],
            Family::Mud,
            false,
            false,
            false,
        ),
        way(
            2,
            &[1, 3],
            &[(0.0, 0.0), (4.0, 0.0)],
            Family::Mud,
            false,
            false,
            false,
        ),
        way(
            3,
            &[1, 4],
            &[(0.0, 0.0), (-4.0, 0.0)],
            Family::Mud,
            false,
            false,
            false,
        ),
    ]);
    let (out, dropped) = drop_stubs(&g, &widths);
    assert_eq!(dropped.len(), 2);
    assert_eq!(out.stretches.len(), 1);
    assert_eq!(components(&out), components(&g));
}

/// A deck end D (node 10) joined by a 5 m stretch to the junction N (node 11) of two gravel roads; M (node 12), 5 m
/// further on, is the junction of two motorway pieces. `second_deck` puts another deck end 5 m beyond M.
fn deck_beside_a_junction(second_deck: bool) -> RoadGraph {
    let mut features = vec![
        way(
            50,
            &[10, 60],
            &[(45.0, 0.0), (10.0, 0.0)],
            Family::Grav,
            true,
            true,
            false,
        ),
        plain(51, &[10, 11], &[(45.0, 0.0), (50.0, 0.0)], Family::Grav),
        plain(
            20,
            &[20, 11, 21],
            &[(50.0, -30.0), (50.0, 0.0), (50.0, 30.0)],
            Family::Grav,
        ),
        plain(52, &[11, 12], &[(50.0, 0.0), (55.0, 0.0)], Family::Grav),
        plain(
            22,
            &[22, 12, 23],
            &[(55.0, -30.0), (55.0, 0.0), (55.0, 30.0)],
            Family::Asf1,
        ),
    ];
    if second_deck {
        features.push(plain(
            53,
            &[12, 13],
            &[(55.0, 0.0), (60.0, 0.0)],
            Family::Grav,
        ));
        features.push(way(
            54,
            &[13, 61],
            &[(60.0, 0.0), (95.0, 0.0)],
            Family::Grav,
            true,
            true,
            false,
        ));
    }
    build_graph(&features)
}

#[test]
fn a_junction_joined_to_a_deck_end_by_a_short_stretch_counts_as_that_deck_end() {
    let g = deck_beside_a_junction(false);
    let (out, merged, not_merged) = merge_junctions(&g);
    assert_eq!(not_merged, Vec::new());
    // the motorway junction ranks higher, but the junction next to the deck end is the one that stays
    assert_eq!((merged[0].kept, merged[0].merged.clone()), (11, vec![12]));
    assert_eq!(components(&out), components(&g));
}

#[test]
fn two_junctions_that_each_hang_on_a_deck_end_are_not_merged_and_are_reported() {
    let g = deck_beside_a_junction(true);
    let (out, merged, not_merged) = merge_junctions(&g);
    assert_eq!(merged, Vec::new());
    assert_eq!(not_merged.len(), 1);
    assert_eq!(not_merged[0].nodes, [11, 12]);
    assert_eq!(out, g);
}

#[test]
fn a_short_pair_that_a_merge_makes_is_reported() {
    // A (2) and B (3) merge; B's side road to the junction J (7) was 8.6 m and is now 5.8 m from A
    let g = build_graph(&[
        plain(
            1,
            &[1, 2, 3, 4],
            &[(0.0, 0.0), (50.0, 0.0), (54.0, 0.0), (100.0, 0.0)],
            Family::Asf1,
        ),
        plain(2, &[2, 5], &[(50.0, 0.0), (50.0, -40.0)], Family::Grav),
        plain(3, &[3, 7], &[(54.0, 0.0), (47.0, 5.0)], Family::Grav),
        plain(
            4,
            &[8, 7, 9],
            &[(47.0, 40.0), (47.0, 5.0), (30.0, 5.0)],
            Family::Grav,
        ),
    ]);
    let (out, merged, not_merged) = merge_junctions(&g);
    assert_eq!(merged.len(), 1);
    assert_eq!(not_merged.len(), 1);
    assert_eq!(not_merged[0].nodes, [2, 7]);
    assert!(not_merged[0].reason.contains("merge"));
    assert_eq!(components(&out), components(&g));
}

#[test]
fn a_cluster_whose_straightened_road_would_cross_another_road_is_not_merged() {
    // the road from B bends away and runs up at x = 59; straight from A it would cut across the road at y = 20
    let g = build_graph(&[
        plain(
            1,
            &[1, 2, 3, 4],
            &[(0.0, 0.0), (50.0, 0.0), (54.0, 0.0), (100.0, 0.0)],
            Family::Asf1,
        ),
        plain(2, &[2, 5], &[(50.0, 0.0), (50.0, -40.0)], Family::Grav),
        plain(
            3,
            &[3, 30, 6],
            &[(54.0, 0.0), (59.0, 3.0), (59.0, 40.0)],
            Family::Grav,
        ),
        plain(4, &[70, 71], &[(52.0, 20.0), (57.0, 20.0)], Family::Grav),
    ]);
    let (out, merged, not_merged) = merge_junctions(&g);
    assert_eq!(merged, Vec::new());
    assert_eq!(not_merged.len(), 1);
    assert!(not_merged[0].reason.contains("cross"));
    assert_eq!(out, g);
}

#[test]
fn clean_reports_the_components_after_every_step_and_the_drivable_length() {
    let g = road_with_stub(plain(2, &[2, 4], &[(50.0, 0.0), (50.0, 4.0)], Family::Asf3));
    let (_, report) = clean(&g, &widths, 8.0);
    assert_eq!(report.components_by_step, vec![components(&g); 4]);
    assert!((report.drivable_m_before - 104.0).abs() < 1e-9);
    assert!((report.drivable_m_after - 100.0).abs() < 1e-9);
}

// ---- sidepaths ----

/// Narrow roads and a fine cell: the earthworks of two roads (half the width plus the shoulder ring of one and a half
/// cells each) keep a cell apart from 11 m on.
fn narrow(_: Family) -> f64 {
    2.0
}

const FINE_CELL_M: f64 = 2.0;
const CELL_M: f64 = 4.0;

/// A road along y = 0 from x = -50 to 150 with nodes at x = 0 and x = 100.
fn main_road(family: Family) -> Feature {
    plain(
        1,
        &[1, 2, 3, 4],
        &[(-50.0, 0.0), (0.0, 0.0), (100.0, 0.0), (150.0, 0.0)],
        family,
    )
}

/// A lesser road that leaves the main road at x = 0, runs `d` metres beside it and joins it again at x = 100.
fn side_road(d: f64, family: Family) -> Feature {
    plain(
        2,
        &[2, 5, 6, 3],
        &[(0.0, 0.0), (0.0, d), (100.0, d), (100.0, 0.0)],
        family,
    )
}

fn scene(road: Feature, side: Feature) -> RoadGraph {
    build_graph(&[road, side])
}

fn kept_reasons(kept: &[KeptSidepath]) -> Vec<&'static str> {
    kept.iter().map(|k| k.reason).collect()
}

#[test]
fn a_gravel_road_five_metres_beside_a_road_and_joined_at_both_ends_is_removed_and_reported() {
    let g = scene(main_road(Family::Asf3), side_road(5.0, Family::Grav));
    assert_eq!(stretch_ids(&g), ["1:1-2", "1:2-3", "1:3-4", "2:2-3"]);
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(stretch_ids(&out), ["1:1-2", "1:2-3", "1:3-4"]);
    assert_eq!(dropped.len(), 1);
    let d = &dropped[0];
    assert_eq!((d.stretch.as_str(), d.road.as_str()), ("2:2-3", "1:2-3"));
    assert!(d.from_m.abs() < 1e-9, "{d:?}");
    assert!(
        (d.to_m
            - g.stretches[3]
                .points
                .windows(2)
                .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
                .sum::<f64>())
        .abs()
            < 1e-6
    );
    assert!(
        (d.at.0 - 50.0).abs() < 1.0 && (d.at.1 - 5.0).abs() < 1e-6,
        "{d:?}"
    );
    assert_eq!(d.connectors.len(), 0);
    assert_eq!(kept.len(), 0);
    assert_eq!(components(&out), components(&g));
}

#[test]
fn a_road_that_keeps_its_distance_stays_with_the_reason_clear() {
    let g = scene(main_road(Family::Asf3), side_road(14.0, Family::Grav));
    let (out, dropped, kept) = drop_sidepaths(&g, &narrow, FINE_CELL_M);
    assert_eq!((&out, dropped.len()), (&g, 0));
    assert_eq!(kept_reasons(&kept), ["clear"]);
    assert_eq!(
        (kept[0].stretch.as_str(), kept[0].road.as_str()),
        ("2:2-3", "1:2-3")
    );
    assert!((kept[0].at.1 - 14.0).abs() < 1e-6, "{kept:?}");
    // the same road with the real widths of a gravel road and an asphalt road leaves less than a cell
    let (out, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!((out.stretches.len(), dropped.len()), (3, 1));
}

#[test]
fn a_road_beside_a_bridge_of_the_road_is_removed_along_the_bridge() {
    // the main road has a bridge between x = 0 and x = 100; the track runs 14 m beside it
    let bridge = way(
        3,
        &[2, 3],
        &[(0.0, 0.0), (100.0, 0.0)],
        Family::Asf3,
        true,
        true,
        false,
    );
    let road = plain(
        1,
        &[1, 2, 3, 4],
        &[(-50.0, 0.0), (0.0, 0.0), (100.0, 0.0), (150.0, 0.0)],
        Family::Asf3,
    );
    // without the bridge the earthworks of the two narrow roads keep their distance
    let with_deck = build_graph(&[
        plain(1, &[1, 2], &[(-50.0, 0.0), (0.0, 0.0)], Family::Asf3),
        bridge,
        plain(4, &[3, 4], &[(100.0, 0.0), (150.0, 0.0)], Family::Asf3),
        side_road(14.0, Family::Grav),
    ]);
    let plain_road = scene(road, side_road(14.0, Family::Grav));
    let (_, dropped, kept) = drop_sidepaths(&plain_road, &narrow, FINE_CELL_M);
    assert_eq!((dropped.len(), kept_reasons(&kept)), (0, vec!["clear"]));
    let (out, dropped, kept) = drop_sidepaths(&with_deck, &narrow, FINE_CELL_M);
    assert_eq!(dropped.len(), 1, "{kept:?}");
    assert!(out.stretches.iter().all(|s| s.id != "2:2-3"));
    assert_eq!(components(&out), components(&with_deck));
}

#[test]
fn a_run_of_twenty_metres_stays_unless_the_road_says_use_sidepath() {
    let short = plain(2, &[5, 6], &[(40.0, 5.0), (60.0, 5.0)], Family::Grav);
    let g = scene(main_road(Family::Asf3), short.clone());
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!((&out, dropped.len(), kept.len()), (&g, 0, 0));
    let mut tagged = main_road(Family::Asf3);
    if let FeatureKind::Road { use_sidepath, .. } = &mut tagged.kind {
        *use_sidepath = true;
    }
    let g = scene(tagged, short);
    let (out, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(dropped.len(), 1);
    assert!(out.stretches.iter().all(|s| s.id != "2:5-6"));
}

#[test]
fn a_road_that_crosses_its_road_is_not_removed_across_the_crossing() {
    let side = plain(2, &[5, 6], &[(0.0, -5.0), (100.0, 5.0)], Family::Grav);
    let g = scene(main_road(Family::Asf3), side);
    let (out, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(dropped.len(), 2, "{dropped:?}");
    for d in &dropped {
        assert!(d.to_m < 50.6 || d.from_m > 49.4, "{d:?}");
    }
    // what is left of it still crosses the road at x = 50
    assert!(
        out.stretches
            .iter()
            .filter(|s| s.id.starts_with("2:"))
            .any(|s| s
                .points
                .iter()
                .any(|p| (p.0 - 50.0).abs() < 2.0 && p.1.abs() < 2.0)),
        "{:?}",
        stretch_ids(&out)
    );
}

/// The main road, a track that leaves it at x = 0 and joins it at x = 100 in two ways (nodes 2 to 7 and 7 to 3) that
/// meet at x = 50, and a side road that leaves the track there and goes north.
fn track_with_a_side_road() -> Vec<Feature> {
    vec![
        main_road(Family::Asf3),
        plain(
            2,
            &[2, 5, 7],
            &[(0.0, 0.0), (5.0, 5.0), (50.0, 5.0)],
            Family::Grav,
        ),
        plain(
            3,
            &[7, 6, 3],
            &[(50.0, 5.0), (95.0, 5.0), (100.0, 0.0)],
            Family::Grav,
        ),
        plain(4, &[7, 8], &[(50.0, 5.0), (50.0, 60.0)], Family::Grav),
    ]
}

#[test]
fn a_road_that_met_the_removed_track_in_the_middle_ends_on_the_road_and_stays_reachable() {
    let g = build_graph(&track_with_a_side_road());
    assert_eq!(components(&g), 1);
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(components(&out), 1);
    assert!(kept.is_empty(), "{kept:?}");
    assert_eq!(dropped.len(), 2, "{dropped:?}");
    let joined: Vec<_> = dropped.iter().flat_map(|d| d.connectors.iter()).collect();
    assert_eq!(joined.len(), 1, "{dropped:?}");
    assert_eq!(joined[0].0, "4:7-8");
    assert!((joined[0].1.0 - 50.0).abs() < 1e-6 && joined[0].1.1.abs() < 1e-6);
    // the stretch keeps its id and the straight connector is its new last piece, on a node of the main road
    let side = out
        .stretches
        .iter()
        .find(|s| s.id == "4:7-8")
        .expect("kept");
    let (start_node, end_node) = (side.start, side.end);
    let far = side
        .points
        .iter()
        .find(|p| p.1 > 59.0)
        .expect("far end kept");
    assert_eq!(*far, (50.0, 60.0));
    let on_road = |p: (f64, f64)| (p.0 - 50.0).abs() < 1e-6 && p.1.abs() < 1e-6;
    assert!(side.points.iter().any(|&p| on_road(p)));
    let node_on_road = if on_road(side.points[0]) {
        start_node
    } else {
        end_node
    };
    let touching_road: Vec<_> = out
        .stretches
        .iter()
        .filter(|s| s.id.starts_with("1:") && (s.start == node_on_road || s.end == node_on_road))
        .collect();
    assert_eq!(
        touching_road.len(),
        2,
        "the road is cut where the stretch joins it"
    );
    // the far end is reachable from the road: the node on the road is in the road's component
    assert_eq!(components(&out), components(&g));
}

#[test]
fn when_a_third_road_lies_between_the_piece_of_the_track_is_kept_instead() {
    let mut features = track_with_a_side_road();
    // a short mud track across the straight connector from (50, 5) to (50, 0)
    features.push(plain(5, &[9, 10], &[(40.0, 2.5), (60.0, 2.5)], Family::Mud));
    let g = build_graph(&features);
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(components(&out), components(&g));
    assert_eq!(kept_reasons(&kept), ["crosses"], "{kept:?}");
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    // the piece next to the blocked joint stays whole with the stretch that met it
    let ids = stretch_ids(&out);
    assert!(ids.contains(&"2:2-7".to_string()), "{ids:?}");
    assert!(ids.contains(&"4:7-8".to_string()));
    assert!(!ids.contains(&"3:3-7".to_string()));
    let side = out.stretches.iter().find(|s| s.id == "4:7-8").unwrap();
    assert_eq!((side.start, side.end), (7, 8));
}

#[test]
fn a_gravel_road_beside_a_paved_road_is_a_candidate_but_not_the_other_way_round() {
    // a track of its own beside the main road, which is longer: the lesser of the two is the sidepath
    let beside = |main: Family, track: Family| {
        let track = plain(2, &[5, 6], &[(0.0, 5.0), (100.0, 5.0)], track);
        let g = scene(main_road(main), track);
        let (_, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
        dropped
            .iter()
            .map(|d| d.stretch.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(beside(Family::Asf3, Family::Grav), ["2:5-6"]);
    assert_eq!(beside(Family::Grav, Family::Asf3), ["1:1-4"]);
    assert_eq!(beside(Family::Asf3, Family::Asf3), Vec::<String>::new());
}

#[test]
fn a_road_between_two_roads_is_removed_against_the_nearer() {
    let upper = plain(3, &[7, 8], &[(-50.0, 20.0), (150.0, 20.0)], Family::Asf3);
    let side = plain(2, &[5, 6], &[(0.0, 8.0), (100.0, 8.0)], Family::Grav);
    let g = build_graph(&[main_road(Family::Asf3), upper, side]);
    let (out, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(dropped[0].road, "1:1-4");
    assert!(out.stretches.iter().all(|s| s.id != "2:5-6"));
}

#[test]
fn the_sidepath_result_does_not_depend_on_the_order_of_the_stretches() {
    let g = build_graph(&track_with_a_side_road());
    let want = drop_sidepaths(&g, &widths, CELL_M);
    let mut shuffled = g.clone();
    shuffled.stretches.reverse();
    shuffled.stretches.swap(0, 2);
    let got = drop_sidepaths(&shuffled, &widths, CELL_M);
    assert_eq!(got, want);
}

#[test]
fn degenerate_input_is_left_alone_without_a_panic() {
    let road = main_road(Family::Asf3);
    let nan = plain(2, &[5, 6], &[(0.0, f64::NAN), (100.0, 5.0)], Family::Grav);
    let point = plain(3, &[7, 8], &[(30.0, 5.0), (30.0, 5.0)], Family::Grav);
    let looped = plain(
        4,
        &[9, 10, 9],
        &[(10.0, 6.0), (60.0, 6.0), (10.0, 6.0)],
        Family::Grav,
    );
    let g = build_graph(&[road, nan, point, looped]);
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(
        (stretch_ids(&out), dropped.len(), kept.len()),
        (stretch_ids(&g), 0, 0)
    );
    let nothing = drop_sidepaths(&RoadGraph::default(), &widths, f64::NAN);
    assert_eq!(nothing.0, RoadGraph::default());
    let (out, dropped, _) = drop_sidepaths(
        &scene(main_road(Family::Asf3), side_road(5.0, Family::Grav)),
        &widths,
        f64::NAN,
    );
    assert_eq!(
        dropped.len(),
        0,
        "a cell that is not a number disturbs nothing: {}",
        out.stretches.len()
    );
}

#[test]
fn a_road_that_runs_beside_in_its_middle_only_keeps_its_approach_joined_to_the_road() {
    // starts free 30 m north of the road, comes down to 5 m beside it at x = 40 and runs on to x = 140
    let side = plain(
        2,
        &[5, 7, 6],
        &[(0.0, 30.0), (40.0, 5.0), (150.0, 5.0)],
        Family::Grav,
    );
    let g = scene(main_road(Family::Asf3), side);
    let (out, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert!(dropped[0].from_m > 15.0);
    // the head stays under its id and ends on a node of the main road
    let head = out
        .stretches
        .iter()
        .find(|s| s.id == "2:5-6")
        .expect("head");
    assert_eq!(head.points.first(), Some(&(0.0, 30.0)));
    let end = head.points.last().unwrap();
    assert!(end.1.abs() < 1e-9, "{end:?}");
    let shared = out
        .stretches
        .iter()
        .filter(|s| {
            s.id.starts_with("1:")
                && [s.start, s.end]
                    .iter()
                    .any(|n| *n == head.end || *n == head.start)
        })
        .count();
    assert_eq!(shared, 2);
}

#[test]
fn clean_removes_sidepaths_first_and_reports_them() {
    let g = scene(main_road(Family::Asf3), side_road(5.0, Family::Grav));
    let (out, report) = clean(&g, &widths, CELL_M);
    assert_eq!(report.sidepaths.len(), 1);
    assert_eq!(out.stretches.len(), 3);
    assert_eq!(report.components_by_step, vec![1; 4]);
    let kept = scene(main_road(Family::Asf3), side_road(14.0, Family::Grav));
    let (_, report) = clean(&kept, &narrow, FINE_CELL_M);
    assert_eq!(kept_reasons(&report.sidepaths_kept), ["clear"]);
    assert_eq!(report.sidepaths.len(), 0);
}

/// Whether nodes `a` and `b` are joined by stretches.
fn connected(g: &RoadGraph, a: i64, b: i64) -> bool {
    let mut seen = vec![a];
    let mut todo = vec![a];
    while let Some(at) = todo.pop() {
        for s in &g.stretches {
            for (from, to) in [(s.start, s.end), (s.end, s.start)] {
                if from == at && !seen.contains(&to) {
                    seen.push(to);
                    todo.push(to);
                }
            }
        }
    }
    seen.contains(&b)
}

/// A main road, an Asf2 road that ends 14 m from it at node 8, and a gravel track from that node that runs beside the
/// main road and joins it at node 3: the track is the only way from the Asf2 road to the main road.
fn road_that_ends_at_a_track() -> Vec<Feature> {
    vec![
        main_road(Family::Asf3),
        plain(3, &[7, 8], &[(0.0, 80.0), (0.0, 14.0)], Family::Asf2),
        plain(
            2,
            &[8, 5, 6, 3],
            &[(0.0, 14.0), (0.0, 5.0), (100.0, 5.0), (100.0, 0.0)],
            Family::Grav,
        ),
    ]
}

#[test]
fn a_road_of_a_higher_family_that_ends_at_the_track_is_joined_to_the_road_it_was_not_on() {
    let g = build_graph(&road_that_ends_at_a_track());
    assert!(connected(&g, 7, 1));
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(dropped.len(), 1, "{kept:?}");
    assert!(
        connected(&out, 7, 1),
        "the Asf2 road is an island: {:?}",
        stretch_ids(&out)
    );
    assert_eq!(components(&out), components(&g));
    // what is left of the track joins the Asf2 road to the main road
    assert!(
        out.stretches.iter().any(|s| s.id.starts_with("2:")),
        "{:?}",
        stretch_ids(&out)
    );
}

#[test]
fn another_track_that_disappears_whole_does_not_hide_a_split() {
    // the same, and a free track 5 m on the other side of the main road: its component goes with it, which
    // must not make up for the Asf2 road that would be cut off
    let mut features = road_that_ends_at_a_track();
    features.push(plain(
        9,
        &[20, 21],
        &[(10.0, -5.0), (90.0, -5.0)],
        Family::Grav,
    ));
    let g = build_graph(&features);
    let (out, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(dropped.len(), 2, "{dropped:?}");
    assert!(
        connected(&out, 7, 1),
        "the Asf2 road is an island: {:?}",
        stretch_ids(&out)
    );
}

#[test]
fn a_track_that_bends_towards_the_road_at_its_end_leaves_no_loop() {
    let road = plain(
        1,
        &[1, 2, 3, 4],
        &[(-50.0, 0.0), (0.0, 0.0), (80.0, 0.0), (150.0, 0.0)],
        Family::Asf3,
    );
    let side = plain(
        2,
        &[5, 6, 3],
        &[(0.0, 8.0), (60.0, 8.0), (80.0, 0.0)],
        Family::Grav,
    );
    let g = build_graph(&[road, side]);
    let (out, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert!(
        out.stretches.iter().all(|s| s.id.starts_with("1:")),
        "{:?}",
        stretch_ids(&out)
    );
}

#[test]
fn a_track_that_wiggles_more_than_fifteen_degrees_is_one_run() {
    let points = [
        (0.0, 3.0),
        (12.0, 3.0),
        (22.0, 9.0),
        (34.0, 9.0),
        (44.0, 3.0),
        (56.0, 3.0),
        (66.0, 9.0),
        (78.0, 9.0),
        (88.0, 3.0),
        (100.0, 3.0),
    ];
    let nodes: Vec<i64> = (20..30).collect();
    let g = build_graph(&[
        main_road(Family::Asf3),
        plain(2, &nodes, &points, Family::Grav),
    ]);
    let (out, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert!(dropped[0].to_m - dropped[0].from_m > 100.0, "{dropped:?}");
    assert_eq!(out.stretches.len(), 1);
}

/// The track from the middle test, with a road that joins it at `x` coming straight down at `x`.
fn track_with_a_road_coming_down(x: f64, road: Feature) -> Vec<Feature> {
    vec![
        road,
        plain(2, &[5, 7], &[(-30.0, 5.0), (x, 5.0)], Family::Grav),
        plain(3, &[7, 6], &[(x, 5.0), (100.0, 5.0)], Family::Grav),
        plain(4, &[7, 8], &[(x, 5.0), (x, 60.0)], Family::Grav),
    ]
}

#[test]
fn a_joint_is_made_on_a_node_of_the_road_within_six_metres_not_next_to_it() {
    // the road has a junction at x = 0 (a side road goes south); the stretch would meet it at x = 5.7
    let road = plain(
        1,
        &[1, 2, 3],
        &[(-50.0, 0.0), (0.0, 0.0), (150.0, 0.0)],
        Family::Asf3,
    );
    let mut features = track_with_a_road_coming_down(5.7, road);
    features.push(plain(8, &[2, 9], &[(0.0, 0.0), (0.0, -30.0)], Family::Asf3));
    let g = build_graph(&features);
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(dropped.len(), 2, "{kept:?}");
    let joined: Vec<_> = dropped.iter().flat_map(|d| d.connectors.iter()).collect();
    assert_eq!(joined.len(), 1, "{dropped:?}");
    assert_eq!(joined[0].1, (0.0, 0.0));
    assert!(
        out.stretches
            .iter()
            .all(|s| !s.id.starts_with("1:") || !s.id.contains("/side")),
        "{:?}",
        stretch_ids(&out)
    );
}

#[test]
fn a_joint_is_never_made_within_a_junction_piece_of_a_deck_end() {
    // a bridge from x = 0 to x = 100; the road that joins the track would meet the approach 0.7 m before it
    let features = vec![
        plain(1, &[1, 2], &[(-50.0, 0.0), (0.0, 0.0)], Family::Asf3),
        way(
            5,
            &[2, 3],
            &[(0.0, 0.0), (100.0, 0.0)],
            Family::Asf3,
            true,
            true,
            false,
        ),
        plain(6, &[3, 4], &[(100.0, 0.0), (150.0, 0.0)], Family::Asf3),
        plain(2, &[10, 11], &[(-30.0, 5.0), (-0.7, 5.0)], Family::Grav),
        plain(3, &[11, 12], &[(-0.7, 5.0), (60.0, 5.0)], Family::Grav),
        plain(4, &[11, 13], &[(-0.7, 5.0), (-0.7, 60.0)], Family::Grav),
    ];
    let g = build_graph(&features);
    let (_, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    let joined: Vec<_> = dropped.iter().flat_map(|d| d.connectors.iter()).collect();
    assert_eq!(joined.len(), 1, "{dropped:?} {kept:?}");
    assert!(
        (joined[0].1.0 + 12.5).abs() < 1e-6 && joined[0].1.1.abs() < 1e-6,
        "{joined:?}"
    );
}

#[test]
fn a_road_that_met_the_track_at_sixty_degrees_goes_on_straight_to_the_road() {
    let features = vec![
        main_road(Family::Asf3),
        plain(2, &[5, 7], &[(0.0, 5.0), (50.0, 5.0)], Family::Grav),
        plain(3, &[7, 6], &[(50.0, 5.0), (100.0, 5.0)], Family::Grav),
        plain(
            4,
            &[7, 8],
            &[(50.0, 5.0), (80.0, 5.0 + 30.0 * 3.0_f64.sqrt())],
            Family::Grav,
        ),
    ];
    let g = build_graph(&features);
    let (out, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    let joined: Vec<_> = dropped.iter().flat_map(|d| d.connectors.iter()).collect();
    assert_eq!(joined.len(), 1, "{dropped:?}");
    let want = (50.0 - 5.0 / 3.0_f64.sqrt(), 0.0);
    assert!(
        (joined[0].1.0 - want.0).abs() < 1e-6 && joined[0].1.1.abs() < 1e-6,
        "{joined:?}"
    );
    let side = out.stretches.iter().find(|s| s.id == "4:7-8").unwrap();
    assert!(
        side.points
            .iter()
            .any(|p| (p.0 - want.0).abs() < 1e-6 && p.1.abs() < 1e-6)
    );
}

#[test]
fn the_rest_of_a_track_meets_the_road_at_a_shallow_angle() {
    let side = plain(
        2,
        &[5, 7, 6],
        &[(0.0, 30.0), (40.0, 5.0), (150.0, 5.0)],
        Family::Grav,
    );
    let g = scene(main_road(Family::Asf3), side);
    let (out, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(dropped.len(), 1);
    let head = out
        .stretches
        .iter()
        .find(|s| s.id == "2:5-6")
        .expect("head");
    let n = head.points.len();
    let (from, to) = (head.points[n - 2], head.points[n - 1]);
    assert!(to.1.abs() < 1e-9, "ends on the road: {to:?}");
    let angle = (to.1 - from.1)
        .abs()
        .atan2((to.0 - from.0).abs())
        .to_degrees();
    assert!(
        angle < 35.0 && angle > 1.0,
        "{angle} degrees, {from:?} -> {to:?}"
    );
}

#[test]
fn only_a_bridge_of_the_road_makes_a_run_disturb() {
    // a fine cell and narrow roads: the earthworks of the track and the road keep a cell apart; a bridge of another road
    // crosses over the main road and ends 4 m from the track, which is no bridge of the road it runs beside
    let other = way(
        5,
        &[30, 31],
        &[(50.0, -20.0), (50.0, 10.0)],
        Family::Asf3,
        true,
        true,
        false,
    );
    let track = plain(2, &[20, 21], &[(0.0, 14.0), (100.0, 14.0)], Family::Grav);
    let g = build_graph(&[main_road(Family::Asf3), other, track]);
    let (out, dropped, kept) = drop_sidepaths(&g, &narrow, 1.0);
    assert_eq!((&out, dropped.len()), (&g, 0));
    assert_eq!(kept_reasons(&kept), ["clear"], "{kept:?}");
    // the bridge of the road itself does
    let deck = way(
        5,
        &[2, 3],
        &[(0.0, 0.0), (100.0, 0.0)],
        Family::Asf3,
        true,
        true,
        false,
    );
    let road =
        |id: i64, nodes: &[i64], points: &[(f64, f64)]| plain(id, nodes, points, Family::Asf3);
    let g = build_graph(&[
        road(1, &[1, 2], &[(-50.0, 0.0), (0.0, 0.0)]),
        deck,
        road(6, &[3, 4], &[(100.0, 0.0), (150.0, 0.0)]),
        plain(2, &[20, 21], &[(0.0, 14.0), (100.0, 14.0)], Family::Grav),
    ]);
    let (_, dropped, _) = drop_sidepaths(&g, &narrow, 1.0);
    assert_eq!(dropped.len(), 1);
}

#[test]
fn a_joint_beside_a_bridge_deck_is_not_made_on_the_deck() {
    // the track meets a side road at x = 50, beside the deck of a bridge from x = 0 to x = 100: the nearest node of
    // the road is the end of the deck, 50 m away, so the run next to the joint stays
    let road = |id: i64, nodes: &[i64], points: &[(f64, f64)], bridge: bool| {
        way(id, nodes, points, Family::Asf3, true, bridge, false)
    };
    let mut features = vec![
        road(1, &[1, 2], &[(-50.0, 0.0), (0.0, 0.0)], false),
        road(5, &[2, 3], &[(0.0, 0.0), (100.0, 0.0)], true),
        road(6, &[3, 4], &[(100.0, 0.0), (150.0, 0.0)], false),
    ];
    features.extend([
        plain(2, &[10, 11], &[(30.0, 5.0), (50.0, 5.0)], Family::Grav),
        plain(3, &[11, 12], &[(50.0, 5.0), (80.0, 5.0)], Family::Grav),
        plain(4, &[11, 13], &[(50.0, 5.0), (50.0, 60.0)], Family::Grav),
    ]);
    let g = build_graph(&features);
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(components(&out), components(&g));
    assert_eq!(kept_reasons(&kept), ["would_disconnect"], "{kept:?}");
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    // the side road is still where it was: nothing was joined to a deck
    let side = out.stretches.iter().find(|s| s.id == "4:11-13").unwrap();
    assert_eq!((side.start, side.end), (11, 13));
    assert!(out.stretches.iter().all(|s| s.id.starts_with("1:")
        || s.id.starts_with("5:")
        || s.id.starts_with("6:")
        || s.id.starts_with("2:")
        || s.id.starts_with("4:")));
}

#[test]
fn the_end_of_a_part_inside_a_stretch_is_not_joined_across_a_third_road() {
    // the track comes down to the road at x = 40; a short mud track lies across where it would be joined to it
    let side = plain(
        2,
        &[5, 7, 6],
        &[(0.0, 30.0), (40.0, 5.0), (150.0, 5.0)],
        Family::Grav,
    );
    let across = plain(3, &[9, 10], &[(15.0, 8.0), (40.0, 8.0)], Family::Mud);
    let g = build_graph(&[main_road(Family::Asf3), side, across]);
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!((&out, dropped.len()), (&g, 0));
    assert_eq!(kept_reasons(&kept), ["crosses"], "{kept:?}");
}

#[test]
fn a_part_of_a_track_beyond_the_end_of_a_road_is_not_beside_it() {
    // the road ends at x = 100 and the track goes on to x = 300: only 100 m of it are beside the road
    let side = plain(2, &[5, 6], &[(0.0, 5.0), (300.0, 5.0)], Family::Grav);
    let g = build_graph(&[
        plain(1, &[1, 2], &[(0.0, 0.0), (100.0, 0.0)], Family::Asf3),
        side,
    ]);
    let (_, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(dropped.len(), 1);
    assert!(dropped[0].to_m < 101.0, "{dropped:?}");
}

#[test]
fn a_track_that_starts_and_ends_beyond_the_ends_of_the_road_is_removed_whole() {
    // free ends 10 m past both ends of the road: the pieces beside nothing go with the run
    let road = plain(1, &[1, 2], &[(0.0, 0.0), (100.0, 0.0)], Family::Asf3);
    let side = plain(2, &[5, 6], &[(-10.0, 5.0), (110.0, 5.0)], Family::Grav);
    let g = build_graph(&[road, side]);
    let (out, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!((dropped[0].from_m, dropped[0].to_m), (0.0, 120.0));
    assert_eq!(out.stretches.len(), 1);
}

#[test]
fn a_joint_beside_the_start_of_a_bridge_is_kept_a_junction_piece_off_the_deck() {
    // the road ends in a bridge at x = 100: a road that joins the track at x = 103 would meet it 3 m after the deck
    let features = vec![
        way(
            5,
            &[2, 3],
            &[(0.0, 0.0), (100.0, 0.0)],
            Family::Asf3,
            true,
            true,
            false,
        ),
        plain(6, &[3, 4], &[(100.0, 0.0), (200.0, 0.0)], Family::Asf3),
        plain(2, &[10, 11], &[(90.0, 5.0), (103.0, 5.0)], Family::Grav),
        plain(3, &[11, 12], &[(103.0, 5.0), (190.0, 5.0)], Family::Grav),
        plain(4, &[11, 13], &[(103.0, 5.0), (103.0, 60.0)], Family::Grav),
    ];
    let g = build_graph(&features);
    let (_, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    let joined: Vec<_> = dropped.iter().flat_map(|d| d.connectors.iter()).collect();
    assert_eq!(joined.len(), 1, "{dropped:?} {kept:?}");
    assert!((joined[0].1.0 - 112.5).abs() < 1e-6, "{joined:?}");
}

// ---- fix round 2 ----

#[test]
fn a_road_of_the_same_network_that_ends_at_the_track_is_joined_to_the_road() {
    // an Asf2 road ends at the node between the two parts of the track and joins the main road 150 m further on
    let main = plain(
        1,
        &[1, 2, 3, 4],
        &[(-50.0, 0.0), (0.0, 0.0), (200.0, 0.0), (300.0, 0.0)],
        Family::Asf3,
    );
    let features = vec![
        main,
        plain(2, &[5, 7], &[(0.0, 5.0), (50.0, 5.0)], Family::Grav),
        plain(3, &[7, 6], &[(50.0, 5.0), (100.0, 5.0)], Family::Grav),
        plain(
            4,
            &[7, 40, 41, 3],
            &[(50.0, 5.0), (50.0, 60.0), (200.0, 60.0), (200.0, 0.0)],
            Family::Asf2,
        ),
    ];
    let g = build_graph(&features);
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(dropped.len(), 2, "{kept:?}");
    let road = out
        .stretches
        .iter()
        .find(|s| s.id.starts_with("4:"))
        .expect("kept");
    // the end that was at x = 50 is on the main road now, not 5 m off it
    let ends = [road.points.first(), road.points.last()];
    let near_track = ends
        .iter()
        .flatten()
        .find(|p| (p.0 - 50.0).abs() < 10.0)
        .expect("an end there");
    assert!(
        near_track.1.abs() < 1e-6,
        "the Asf2 road ends in the air: {:?}",
        road.points
    );
}

#[test]
fn a_junction_approach_with_a_short_parallel_tail_is_not_a_sidepath() {
    // down to the road at about 20 degrees, 4 m along it, then to the junction on the road
    let road = plain(
        1,
        &[1, 2, 3, 4],
        &[(-50.0, 0.0), (0.0, 0.0), (100.0, 0.0), (150.0, 0.0)],
        Family::Asf3,
    );
    let approach = plain(
        2,
        &[5, 6, 7, 3],
        &[(20.0, 30.0), (80.0, 8.0), (84.0, 8.0), (100.0, 0.0)],
        Family::Grav,
    );
    let g = build_graph(&[road, approach]);
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!((&out, dropped.len(), kept.len()), (&g, 0, 0));
}

#[test]
fn a_joint_that_would_have_to_move_to_a_node_is_joined_straight_to_it() {
    // the road coming down at x = 5.7 would meet the main road 5.7 m from its junction at x = 0: it is joined to that
    // junction by a straight connector from where it ended, not bent
    let road = plain(
        1,
        &[1, 2, 3],
        &[(-50.0, 0.0), (0.0, 0.0), (150.0, 0.0)],
        Family::Asf3,
    );
    let mut features = track_with_a_road_coming_down(5.7, road);
    features.push(plain(8, &[2, 9], &[(0.0, 0.0), (0.0, -30.0)], Family::Asf3));
    let g = build_graph(&features);
    let (out, _, _) = drop_sidepaths(&g, &widths, CELL_M);
    let side = out
        .stretches
        .iter()
        .find(|s| s.id == "4:7-8")
        .expect("kept");
    let points = if side.points.first() == Some(&(5.7, 60.0)) {
        side.points.clone()
    } else {
        side.points.iter().rev().copied().collect()
    };
    assert_eq!(points, [(5.7, 60.0), (5.7, 5.0), (0.0, 0.0)]);
}

#[test]
fn a_connector_shorter_than_half_a_metre_is_not_made_the_end_moves_onto_the_road() {
    let features = vec![
        main_road(Family::Asf3),
        plain(2, &[5, 7], &[(0.0, 0.3), (50.0, 0.3)], Family::Grav),
        plain(3, &[7, 6], &[(50.0, 0.3), (100.0, 0.3)], Family::Grav),
        plain(4, &[7, 8], &[(50.0, 0.3), (50.0, 60.0)], Family::Grav),
    ];
    let g = build_graph(&features);
    let (out, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(dropped.len(), 2);
    let side = out
        .stretches
        .iter()
        .find(|s| s.id == "4:7-8")
        .expect("kept");
    let n = side.points.len();
    assert_eq!(n, 2, "{:?}", side.points);
    assert!(side.points.contains(&(50.0, 0.0)), "{:?}", side.points);
}

#[test]
fn a_point_between_two_nodes_of_a_short_road_piece_goes_to_the_nearer_one() {
    // the road piece between the junctions at x = 0 and x = 8 is shorter than twice the merge distance
    let road = plain(
        1,
        &[1, 2, 3, 4],
        &[(-50.0, 0.0), (0.0, 0.0), (8.0, 0.0), (150.0, 0.0)],
        Family::Asf3,
    );
    let mut features = track_with_a_road_coming_down(5.0, road);
    features.push(plain(8, &[2, 9], &[(0.0, 0.0), (0.0, -30.0)], Family::Asf3));
    features.push(plain(
        9,
        &[3, 10],
        &[(8.0, 0.0), (8.0, -30.0)],
        Family::Asf3,
    ));
    let g = build_graph(&features);
    let (_, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    let joined: Vec<_> = dropped.iter().flat_map(|d| d.connectors.iter()).collect();
    assert_eq!(joined.len(), 1, "{dropped:?}");
    assert_eq!(joined[0].1, (8.0, 0.0));
}

#[test]
fn a_removal_that_would_cut_a_piece_off_is_refused_as_a_split() {
    // the joint between the two parts of the track cannot be joined to the road (a mud track lies across), so the
    // shorter part stays with what joins it; the other part's removal would leave that piece without a way to the road
    let features = vec![
        main_road(Family::Asf3),
        plain(2, &[5, 7], &[(20.0, 5.0), (50.0, 5.0)], Family::Grav),
        plain(
            3,
            &[7, 6, 3],
            &[(50.0, 5.0), (100.0, 5.0), (100.0, 0.0)],
            Family::Grav,
        ),
        plain(4, &[7, 8], &[(50.0, 5.0), (50.0, 60.0)], Family::Grav),
        plain(5, &[9, 10], &[(40.0, 2.5), (60.0, 2.5)], Family::Mud),
    ];
    let g = build_graph(&features);
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!((&out, dropped.len()), (&g, 0));
    assert_eq!(kept_reasons(&kept), ["would_split"], "{kept:?}");
}

// ---- fix task 6: a joint does not loop back onto its own road, and a dead-end spur goes whole ----

/// A lower road that leaves the node at x = 0, runs away, turns back and runs west 10 m beside the road from x = 45 to
/// x = -20 (over the node it started from), then leaves to the south.
fn switchback() -> RoadGraph {
    build_graph(&[
        plain(
            1,
            &[1, 2, 3],
            &[(-50.0, 0.0), (0.0, 0.0), (100.0, 0.0)],
            Family::Asf3,
        ),
        plain(
            2,
            &[2, 5, 6, 7, 8],
            &[
                (0.0, 0.0),
                (40.0, -50.0),
                (45.0, -10.0),
                (-20.0, -10.0),
                (-20.0, -60.0),
            ],
            Family::Grav,
        ),
    ])
}

#[test]
fn a_joint_that_would_land_on_the_start_of_its_own_stretch_leaves_the_run_in_place_as_loops_back() {
    let g = switchback();
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!((&out, dropped.len()), (&g, 0), "{dropped:?}");
    assert_eq!(kept_reasons(&kept), ["loops_back"], "{kept:?}");
    assert_eq!(components(&out), components(&g));
}

/// A main road along y = 0, a City road along y = 23 with a node at x = 0, and a dead-end side street that leaves that
/// node, turns after 15 m and runs 100 m beside the main road at 8 m. Its far end joins `joined` if given.
fn side_street(joined: Option<Feature>) -> RoadGraph {
    let mut features = vec![
        plain(1, &[1, 2], &[(-50.0, 0.0), (150.0, 0.0)], Family::Asf1),
        plain(
            3,
            &[10, 11, 12],
            &[(-60.0, 23.0), (0.0, 23.0), (60.0, 23.0)],
            Family::City,
        ),
        plain(
            2,
            &[11, 20, 21],
            &[(0.0, 23.0), (0.0, 8.0), (100.0, 8.0)],
            Family::City,
        ),
    ];
    features.extend(joined);
    build_graph(&features)
}

fn end_nodes(g: &RoadGraph) -> std::collections::BTreeSet<i64> {
    g.stretches.iter().flat_map(|s| [s.start, s.end]).collect()
}

#[test]
fn a_dead_end_side_street_beside_the_road_goes_whole_without_a_joint() {
    let g = side_street(None);
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(kept.len(), 0, "{kept:?}");
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    let d = &dropped[0];
    assert!(d.from_m.abs() < 1e-9, "{d:?}");
    assert!((d.to_m - 115.0).abs() < 1e-6, "{d:?}");
    assert_eq!(d.connectors.len(), 0);
    let expected: Vec<_> = g
        .stretches
        .iter()
        .filter(|s| !s.id.starts_with("2:"))
        .collect();
    assert_eq!(
        out.stretches.len(),
        expected.len(),
        "{:?}",
        stretch_ids(&out)
    );
    for (a, b) in out.stretches.iter().zip(expected) {
        assert_eq!((&a.id, &a.points), (&b.id, &b.points));
    }
    // the junction on the road is not made: the nodes are those of the graph without the spur's own end
    assert_eq!(end_nodes(&out).len(), end_nodes(&g).len() - 1);
}

#[test]
fn a_dead_end_side_street_that_ends_a_few_metres_beyond_the_road_goes_whole_too() {
    // the road ends at x = 100 and the street runs on to x = 110, past the end of the road
    let g = build_graph(&[
        plain(1, &[1, 2], &[(-50.0, 0.0), (100.0, 0.0)], Family::Asf1),
        plain(
            3,
            &[10, 11, 12],
            &[(-60.0, 23.0), (0.0, 23.0), (60.0, 23.0)],
            Family::City,
        ),
        plain(
            2,
            &[11, 20, 21],
            &[(0.0, 23.0), (0.0, 8.0), (110.0, 8.0)],
            Family::City,
        ),
    ]);
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!((dropped.len(), kept.len()), (1, 0), "{dropped:?} {kept:?}");
    assert!(dropped[0].from_m.abs() < 1e-9, "{dropped:?}");
    assert!(
        out.stretches.iter().all(|s| !s.id.starts_with("2:")),
        "{:?}",
        stretch_ids(&out)
    );
    assert_eq!(end_nodes(&out).len(), end_nodes(&g).len() - 1);
}

#[test]
fn a_side_street_whose_far_end_joins_another_road_keeps_its_rest_and_is_joined_to_the_road() {
    let g = side_street(Some(plain(
        4,
        &[21, 30],
        &[(100.0, 8.0), (100.0, 60.0)],
        Family::City,
    )));
    let (out, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    // the 9 m rest at the junction stays; the street's end and the rest are joined to the road by two new nodes
    assert!(
        out.stretches.iter().any(|s| s.id.starts_with("2:")),
        "{:?}",
        stretch_ids(&out)
    );
    assert_eq!(
        end_nodes(&out).len(),
        end_nodes(&g).len() + 2,
        "{:?} {:?}",
        end_nodes(&out),
        end_nodes(&g)
    );
}

#[test]
fn a_dead_end_side_street_whose_near_node_has_one_other_road_goes_whole_and_that_road_is_joined() {
    // the City road ends at the node where the street leaves: the street continues it, so it is still removed whole, but
    // the City road then ends at nothing and is joined to the main road by a connector
    let g = build_graph(&[
        plain(1, &[1, 2], &[(-50.0, 0.0), (150.0, 0.0)], Family::Asf1),
        plain(3, &[10, 11], &[(0.0, 60.0), (0.0, 12.0)], Family::City),
        plain(
            2,
            &[11, 20, 21],
            &[(0.0, 12.0), (0.0, 8.0), (100.0, 8.0)],
            Family::Grav,
        ),
    ]);
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!((dropped.len(), kept.len()), (1, 0), "{dropped:?} {kept:?}");
    assert!(
        out.stretches.iter().all(|s| !s.id.starts_with("2:")),
        "{:?}",
        stretch_ids(&out)
    );
    // the road that continued into the street now ends on the main road
    let city = by_id(&out, "3:10-11");
    assert_eq!(
        city.points.last().map(|p| p.1.abs() < 1e-6),
        Some(true),
        "{:?}",
        city.points
    );
    // joined to the main road: two pieces of road network before, one after
    assert_eq!((components(&g), components(&out)), (2, 1));
}

#[test]
fn a_dead_end_side_street_whose_free_end_is_far_beyond_the_run_does_not_go_whole() {
    // the street runs on 40 m away from the road after the 100 m beside it, to a free end
    let g = build_graph(&[
        plain(1, &[1, 2], &[(-50.0, 0.0), (150.0, 0.0)], Family::Asf1),
        plain(
            3,
            &[10, 11, 12],
            &[(-60.0, 23.0), (0.0, 23.0), (60.0, 23.0)],
            Family::City,
        ),
        plain(
            2,
            &[11, 20, 21, 22],
            &[(0.0, 23.0), (0.0, 8.0), (100.0, 8.0), (100.0, 60.0)],
            Family::City,
        ),
    ]);
    let (out, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    let d = &dropped[0];
    assert!(d.to_m < 115.0 + 15.0, "the run is the part beside: {d:?}");
    // the far part beyond the run is not removed with it
    assert!(
        out.stretches.iter().any(|s| s.id.starts_with("2:")),
        "{:?}",
        stretch_ids(&out)
    );
}

#[test]
fn a_joint_that_lands_on_a_node_of_the_road_is_still_made() {
    // the road has a junction at x = 0, where the rest of the track meets it
    let road = plain(
        1,
        &[1, 2, 3],
        &[(-50.0, 0.0), (0.0, 0.0), (150.0, 0.0)],
        Family::Asf3,
    );
    let side = plain(
        2,
        &[5, 7, 6],
        &[(0.0, 30.0), (0.0, 5.0), (150.0, 5.0)],
        Family::Grav,
    );
    let junction = plain(8, &[2, 9], &[(0.0, 0.0), (0.0, -30.0)], Family::Asf3);
    let g = build_graph(&[road, side, junction]);
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!((dropped.len(), kept.len()), (1, 0), "{dropped:?} {kept:?}");
    let head = out
        .stretches
        .iter()
        .find(|s| s.id == "2:5-6")
        .expect("head");
    assert!(
        head.points.first() == Some(&(0.0, 0.0)) || head.points.last() == Some(&(0.0, 0.0)),
        "{:?}",
        head.points
    );
}

// ---- fix round 3: the spacing of junctions on the road ----

/// A main road of 200 m with a joining road coming down to a track at `x`.
fn road_with_a_joint_at(x: f64) -> RoadGraph {
    build_graph(&track_with_a_road_coming_down(
        x,
        plain(1, &[1, 2], &[(0.0, 0.0), (200.0, 0.0)], Family::Asf3),
    ))
}

fn joint_of(dropped: &[naves_core::netclean::DroppedSidepath]) -> Vec<(f64, f64)> {
    dropped
        .iter()
        .flat_map(|d| d.connectors.iter().map(|c| c.1))
        .collect()
}

#[test]
fn a_joint_within_a_junction_piece_of_a_node_uses_the_node_and_one_beyond_gets_its_own() {
    // the road ends at x = 0 and x = 200 (nodes); a joining road at x = 9 meets it 9 m from the node, one at x = 13 13 m
    let (_, dropped, _) = drop_sidepaths(&road_with_a_joint_at(9.0), &widths, CELL_M);
    assert_eq!(joint_of(&dropped), [(0.0, 0.0)]);
    let (_, dropped, _) = drop_sidepaths(&road_with_a_joint_at(13.0), &widths, CELL_M);
    assert_eq!(joint_of(&dropped), [(13.0, 0.0)]);
}

#[test]
fn a_road_piece_shorter_than_two_junction_pieces_never_gets_a_new_node() {
    // a 20 m piece between two nodes
    let road = plain(
        1,
        &[1, 2, 3, 4],
        &[(-50.0, 0.0), (0.0, 0.0), (20.0, 0.0), (150.0, 0.0)],
        Family::Asf3,
    );
    let mut features = track_with_a_road_coming_down(9.0, road);
    features.push(plain(8, &[2, 9], &[(0.0, 0.0), (0.0, -30.0)], Family::Asf3));
    features.push(plain(
        9,
        &[3, 10],
        &[(20.0, 0.0), (20.0, -30.0)],
        Family::Asf3,
    ));
    let g = build_graph(&features);
    let (out, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(joint_of(&dropped), [(0.0, 0.0)]);
    assert!(
        out.stretches
            .iter()
            .all(|s| !s.id.starts_with("1:") || !s.id.contains("/side"))
    );
}

#[test]
fn two_joints_four_metres_apart_end_on_one_junction_whatever_the_order() {
    let features = vec![
        plain(1, &[1, 2], &[(0.0, 0.0), (200.0, 0.0)], Family::Asf3),
        plain(2, &[5, 7], &[(40.0, 5.0), (96.0, 5.0)], Family::Grav),
        plain(3, &[7, 8], &[(96.0, 5.0), (100.0, 5.0)], Family::Grav),
        plain(4, &[8, 6], &[(100.0, 5.0), (150.0, 5.0)], Family::Grav),
        plain(5, &[7, 9], &[(96.0, 5.0), (96.0, 60.0)], Family::Grav),
        plain(6, &[8, 10], &[(100.0, 5.0), (100.0, 60.0)], Family::Grav),
    ];
    let g = build_graph(&features);
    let want = drop_sidepaths(&g, &widths, CELL_M);
    let joints = joint_of(&want.1);
    assert_eq!(joints.len(), 2, "{:?}", want.1);
    assert_eq!(joints[0], joints[1]);
    let mut shuffled = g.clone();
    shuffled.stretches.reverse();
    assert_eq!(drop_sidepaths(&shuffled, &widths, CELL_M), want);
}

#[test]
fn a_joining_road_whose_heading_leads_to_a_node_but_whose_foot_does_not_is_joined_at_its_foot() {
    // a road comes down to the track at (20, 5) heading for (8, 0), 8 m from the node at x = 0: it is joined at the foot
    // (20, 0) with a straight connector, not bent towards the node
    let features = vec![
        plain(
            1,
            &[1, 2, 3],
            &[(-50.0, 0.0), (0.0, 0.0), (200.0, 0.0)],
            Family::Asf3,
        ),
        plain(2, &[5, 7], &[(-30.0, 5.0), (20.0, 5.0)], Family::Grav),
        plain(3, &[7, 6], &[(20.0, 5.0), (100.0, 5.0)], Family::Grav),
        plain(4, &[7, 8], &[(20.0, 5.0), (68.0, 25.0)], Family::Grav),
        plain(9, &[2, 10], &[(0.0, 0.0), (0.0, -30.0)], Family::Asf3),
    ];
    let g = build_graph(&features);
    let (_, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    assert_eq!(joint_of(&dropped), [(20.0, 0.0)]);
}

// ---- fix round 4: moved joints are checked again, and reach is measured to the road ----

/// Whether the segments `a`-`b` and `c`-`d` cross (not merely touch at an end).
fn segments_cross(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    let side = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| {
        (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
    };
    let (d1, d2) = (side(c, d, a), side(c, d, b));
    let (d3, d4) = (side(a, b, c), side(a, b, d));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

#[test]
fn a_joint_moved_onto_an_earlier_junction_is_not_joined_across_a_third_road() {
    // two joining roads 4 m apart along the track; the later joint would move onto the earlier one at (96, 0), and
    // the connector from (100, 5) to it would cross a short mud track that lies across neither straight connector
    let mud = [(97.5, 2.5), (98.5, 2.5)];
    let features = vec![
        plain(1, &[1, 2], &[(0.0, 0.0), (200.0, 0.0)], Family::Asf3),
        plain(2, &[5, 7], &[(40.0, 5.0), (96.0, 5.0)], Family::Grav),
        plain(3, &[7, 8], &[(96.0, 5.0), (100.0, 5.0)], Family::Grav),
        plain(4, &[8, 6], &[(100.0, 5.0), (150.0, 5.0)], Family::Grav),
        plain(5, &[7, 9], &[(96.0, 5.0), (96.0, 60.0)], Family::Grav),
        plain(6, &[8, 10], &[(100.0, 5.0), (100.0, 60.0)], Family::Grav),
        plain(7, &[11, 12], &mud, Family::Mud),
    ];
    let g = build_graph(&features);
    let want = drop_sidepaths(&g, &widths, CELL_M);
    let (out, dropped, kept) = &want;
    assert!(components(out) <= components(&g));
    for s in out.stretches.iter().filter(|s| !s.id.starts_with("7:")) {
        for w in s.points.windows(2) {
            assert!(
                !segments_cross(w[0], w[1], mud[0], mud[1]),
                "{} crosses the mud track: {:?} (dropped {dropped:?}, kept {kept:?})",
                s.id,
                s.points
            );
        }
    }
    assert!(kept.iter().any(|k| k.reason == "crosses"), "{kept:?}");
    let mut shuffled = g.clone();
    shuffled.stretches.reverse();
    assert_eq!(drop_sidepaths(&shuffled, &widths, CELL_M), want);
}

#[test]
fn a_road_piece_between_two_decks_shorter_than_two_junction_pieces_is_not_joined() {
    // a 20 m piece of road between two bridges; the track beside it meets a joining road at x = 10
    let deck = |id: i64, nodes: &[i64], points: &[(f64, f64)]| {
        way(id, nodes, points, Family::Asf3, true, true, false)
    };
    let features = vec![
        plain(1, &[1, 2], &[(-150.0, 0.0), (-100.0, 0.0)], Family::Asf3),
        deck(5, &[2, 3], &[(-100.0, 0.0), (0.0, 0.0)]),
        plain(6, &[3, 4], &[(0.0, 0.0), (20.0, 0.0)], Family::Asf3),
        deck(7, &[4, 14], &[(20.0, 0.0), (120.0, 0.0)]),
        plain(8, &[14, 15], &[(120.0, 0.0), (170.0, 0.0)], Family::Asf3),
        plain(2, &[10, 11], &[(-30.0, 5.0), (10.0, 5.0)], Family::Grav),
        plain(3, &[11, 12], &[(10.0, 5.0), (60.0, 5.0)], Family::Grav),
        plain(4, &[11, 13], &[(10.0, 5.0), (10.0, 60.0)], Family::Grav),
    ];
    let g = build_graph(&features);
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert!(components(&out) <= components(&g));
    assert!(
        kept.iter().any(|k| k.reason == "would_disconnect"),
        "{kept:?} {dropped:?}"
    );
    // no new node on the short piece, and the joining road still ends where it did
    assert!(
        out.stretches
            .iter()
            .all(|s| !s.id.starts_with("6:") || !s.id.contains("/side")),
        "{:?}",
        stretch_ids(&out)
    );
    let side = out.stretches.iter().find(|s| s.id == "4:11-13").unwrap();
    assert_eq!((side.start, side.end), (11, 13));
}

#[test]
fn a_joint_ten_metres_off_the_road_moved_out_from_a_deck_end_is_made() {
    // the joint at x = 0 is 10 m from the road; the nearest point is the end of a bridge, so the joint moves out to
    // 12.5 m from it: a connector of 16 m, within twice the reach
    let features = vec![
        plain(1, &[1, 2], &[(-50.0, 0.0), (0.0, 0.0)], Family::Asf3),
        way(
            5,
            &[2, 3],
            &[(0.0, 0.0), (100.0, 0.0)],
            Family::Asf3,
            true,
            true,
            false,
        ),
        plain(6, &[3, 4], &[(100.0, 0.0), (150.0, 0.0)], Family::Asf3),
        plain(2, &[10, 11], &[(-30.0, 10.0), (0.0, 10.0)], Family::Grav),
        plain(3, &[11, 12], &[(0.0, 10.0), (60.0, 10.0)], Family::Grav),
        plain(4, &[11, 13], &[(0.0, 10.0), (0.0, 60.0)], Family::Grav),
    ];
    let g = build_graph(&features);
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert!(components(&out) <= components(&g));
    assert!(kept.is_empty(), "{kept:?}");
    let joined: Vec<_> = dropped.iter().flat_map(|d| d.connectors.iter()).collect();
    assert_eq!(joined.len(), 1, "{dropped:?}");
    assert!(
        (joined[0].1.0 + 12.5).abs() < 1e-6 && joined[0].1.1.abs() < 1e-6,
        "{joined:?}"
    );
}

#[test]
fn the_end_of_a_part_inside_a_stretch_is_joined_straight_down_where_the_shallow_merge_would_cross_a_third_road()
 {
    // the track comes down to the road at x = 40; a short mud track lies across the shallow merge ahead of the end of
    // the run, but not across the straight connector down to the road
    let mud = [(44.0, 0.5), (44.0, 4.5)];
    let side = plain(
        2,
        &[5, 7, 6],
        &[(0.0, 30.0), (40.0, 5.0), (150.0, 5.0)],
        Family::Grav,
    );
    let g = build_graph(&[
        main_road(Family::Asf3),
        side,
        plain(3, &[9, 10], &mud, Family::Mud),
    ]);
    let (out, dropped, kept) = drop_sidepaths(&g, &widths, CELL_M);
    assert!(kept.is_empty(), "{kept:?}");
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    for s in out.stretches.iter().filter(|s| !s.id.starts_with("3:")) {
        for w in s.points.windows(2) {
            assert!(
                !segments_cross(w[0], w[1], mud[0], mud[1]),
                "{} crosses the mud track: {:?}",
                s.id,
                s.points
            );
        }
    }
    // the rest of the track ends on the road below the end of the run, before the mud track
    let rest = out
        .stretches
        .iter()
        .find(|s| s.id.starts_with("2:"))
        .unwrap();
    assert!(
        rest.points.iter().any(|p| p.1.abs() < 1e-6 && p.0 < 30.0),
        "{:?}",
        rest.points
    );
}

// ---- bridge approaches ----

fn deck(id: i64, nodes: &[i64], points: &[(f64, f64)]) -> Feature {
    way(id, nodes, points, Family::Asf3, true, true, false)
}

fn asf(id: i64, nodes: &[i64], points: &[(f64, f64)]) -> Feature {
    plain(id, nodes, points, Family::Asf3)
}

fn grav(id: i64, nodes: &[i64], points: &[(f64, f64)]) -> Feature {
    plain(id, nodes, points, Family::Grav)
}

#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a missing stretch fails the test"
)]
fn by_id<'a>(g: &'a RoadGraph, id: &str) -> &'a naves_core::roadnet::Stretch {
    g.stretches.iter().find(|s| s.id == id).unwrap()
}

/// A deck of `len` metres from the origin east to node 2, the road going on east for `rest` metres to node 3 (a
/// side road leaves node 3 so that it is a junction), and a side road leaving node 2 north.
fn continuing(len: f64, rest: f64) -> RoadGraph {
    build_graph(&[
        deck(1, &[1, 2], &[(0.0, 0.0), (len, 0.0)]),
        asf(2, &[2, 3], &[(len, 0.0), (len + rest, 0.0)]),
        asf(
            5,
            &[3, 7, 8],
            &[(len + rest, 0.0), (len + rest, -30.0), (len + rest, -80.0)],
        ),
        asf(6, &[3, 9], &[(len + rest, 0.0), (len + rest + 100.0, 0.0)]),
        grav(3, &[2, 4], &[(len, 0.0), (len, 50.0)]),
    ])
}

/// A deck of `len` metres from the origin east to node 2 where a road passes across (nodes 5 and 6).
fn t_at_the_end(len: f64) -> RoadGraph {
    build_graph(&[
        deck(1, &[1, 2], &[(0.0, 0.0), (len, 0.0)]),
        asf(2, &[5, 2, 6], &[(len, -50.0), (len, 0.0), (len, 50.0)]),
    ])
}

fn length(points: &[(f64, f64)]) -> f64 {
    points
        .iter()
        .zip(points.iter().skip(1))
        .map(|(a, b)| (b.0 - a.0).hypot(b.1 - a.1))
        .sum()
}

fn sorted_stretches(g: &RoadGraph) -> Vec<naves_core::roadnet::Stretch> {
    let mut v = g.stretches.clone();
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
}

#[test]
fn a_side_road_on_the_end_of_a_bridge_is_joined_eight_metres_beyond() {
    let g = continuing(40.0, 100.0);
    let (out, approaches, none) = make_approaches(&g);
    assert_eq!(none, Vec::new());
    assert_eq!(approaches.len(), 1);
    let a = &approaches[0];
    assert_eq!(
        (a.bridge.as_str(), a.end, a.kind),
        ("1:1-2", 1, ApproachKind::SideRoadsMoved)
    );
    assert_eq!(a.moved, vec!["3:2-4".to_string()]);
    assert!((a.at.0 - 48.0).abs() < 1e-9 && a.at.1.abs() < 1e-9);
    assert_eq!(by_id(&out, "1:1-2"), by_id(&g, "1:1-2"));
    // the end node has two stretches now: the deck and the first 8 m of the road that goes on
    let at_end = out
        .stretches
        .iter()
        .filter(|s| s.start == 2 || s.end == 2)
        .count();
    assert_eq!(at_end, 2);
    let approach = by_id(&out, "2:2-3/approach0");
    assert!((length(&approach.points) - 8.0).abs() < 1e-9);
    assert!(!approach.bridge);
    let side = by_id(&out, "3:2-4");
    let new_node = a_new_node(&g, &out);
    assert!(side.start == new_node || side.end == new_node);
    let mut ends = vec![side.points[0], side.points[side.points.len() - 1]];
    ends.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(ends, vec![(40.0, 50.0), (48.0, 0.0)]);
    assert_components_kept(&g, &out);
}

/// The one node of `out` that `g` does not have.
#[expect(
    clippy::indexing_slicing,
    reason = "a test helper: the list is checked to be filled"
)]
fn a_new_node(g: &RoadGraph, out: &RoadGraph) -> i64 {
    let old: std::collections::BTreeSet<i64> =
        g.stretches.iter().flat_map(|s| [s.start, s.end]).collect();
    let new: Vec<i64> = out
        .stretches
        .iter()
        .flat_map(|s| [s.start, s.end])
        .filter(|n| !old.contains(n))
        .collect();
    assert!(new.iter().all(|n| *n == new[0]) && !new.is_empty());
    new[0]
}

#[test]
fn a_road_that_goes_on_only_five_metres_before_the_next_junction_leaves_no_room() {
    let g = continuing(40.0, 5.0);
    let (out, approaches, none) = make_approaches(&g);
    assert_eq!(approaches, Vec::new());
    assert_eq!(none.len(), 1);
    assert_eq!(
        (none[0].bridge.as_str(), none[0].end, none[0].reason),
        ("1:1-2", 1, "no_room")
    );
    assert_eq!(none[0].at, (40.0, 0.0));
    assert_eq!(out, g);
}

#[test]
fn a_road_that_goes_on_for_less_than_a_junction_piece_beyond_the_approach_leaves_no_room() {
    let g = continuing(40.0, 15.0);
    let (out, approaches, none) = make_approaches(&g);
    assert_eq!((approaches.len(), none.len()), (0, 1));
    assert_eq!(out, g);
}

#[test]
fn a_road_that_goes_on_exactly_the_approach_joins_the_side_road_at_its_far_node() {
    let g = continuing(40.0, 8.0);
    let (out, approaches, none) = make_approaches(&g);
    assert_eq!(none, Vec::new());
    assert_eq!(approaches.len(), 1);
    let side = by_id(&out, "3:2-4");
    assert!(side.start == 3 || side.end == 3);
    assert!(side.points.contains(&(48.0, 0.0)));
    assert_eq!(out.stretches.len(), g.stretches.len());
    assert_components_kept(&g, &out);
}

#[test]
fn a_deck_that_ends_in_a_t_is_shortened_by_eight_metres() {
    let g = t_at_the_end(40.0);
    let (out, approaches, none) = make_approaches(&g);
    assert_eq!(none, Vec::new());
    assert_eq!(approaches.len(), 1);
    assert_eq!(approaches[0].kind, ApproachKind::BridgeShortened);
    assert_eq!(
        (approaches[0].bridge.as_str(), approaches[0].end),
        ("1:1-2", 1)
    );
    let deck = by_id(&out, "1:1-2");
    assert!(deck.bridge);
    assert!((length(&deck.points) - 32.0).abs() < 1e-9);
    let piece = by_id(&out, "1:1-2/approach0");
    assert!(!piece.bridge && piece.bridge_kind.is_none());
    assert_eq!((piece.family, piece.drivable), (deck.family, deck.drivable));
    assert!((length(&piece.points) - 8.0).abs() < 1e-9);
    assert!(piece.start == 2 || piece.end == 2);
    assert_eq!(out.stretches.len(), g.stretches.len() + 1);
    assert_components_kept(&g, &out);
}

#[test]
fn a_deck_of_twenty_metres_that_ends_in_a_t_is_too_short_to_shorten() {
    let g = t_at_the_end(20.0);
    let (out, approaches, none) = make_approaches(&g);
    assert_eq!(approaches, Vec::new());
    assert_eq!(none.len(), 1);
    assert_eq!(none[0].reason, "bridge_too_short");
    assert_eq!(out, g);
}

#[test]
fn a_deck_with_a_road_under_its_last_five_metres_is_not_shortened() {
    let mut features = vec![
        deck(1, &[1, 2], &[(0.0, 0.0), (40.0, 0.0)]),
        asf(2, &[5, 2, 6], &[(40.0, -50.0), (40.0, 0.0), (40.0, 50.0)]),
        asf(3, &[11, 12], &[(35.0, -30.0), (35.0, 30.0)]),
    ];
    let g = build_graph(&features);
    let (out, approaches, none) = make_approaches(&g);
    assert_eq!(approaches, Vec::new());
    assert_eq!(
        none.iter().map(|n| n.reason).collect::<Vec<_>>(),
        vec!["road_below"]
    );
    assert_eq!(out, g);
    // the same road further back does not stop it
    features[2] = asf(3, &[11, 12], &[(20.0, -30.0), (20.0, 30.0)]);
    let (_, approaches, none) = make_approaches(&build_graph(&features));
    assert_eq!((approaches.len(), none.len()), (1, 0));
}

#[test]
fn a_bridge_end_with_only_two_stretches_is_untouched_and_not_reported() {
    let g = build_graph(&[
        asf(1, &[1, 2], &[(-50.0, 0.0), (0.0, 0.0)]),
        deck(2, &[2, 3], &[(0.0, 0.0), (40.0, 0.0)]),
        asf(3, &[3, 4], &[(40.0, 0.0), (90.0, 0.0)]),
    ]);
    let (out, approaches, none) = make_approaches(&g);
    assert_eq!((approaches, none), (Vec::new(), Vec::new()));
    assert_eq!(out, g);
}

#[test]
fn both_ends_of_one_bridge_are_handled_independently() {
    let g = build_graph(&[
        deck(1, &[1, 2], &[(0.0, 0.0), (60.0, 0.0)]),
        asf(2, &[5, 1, 6], &[(0.0, -50.0), (0.0, 0.0), (0.0, 50.0)]),
        asf(3, &[7, 2, 8], &[(60.0, -50.0), (60.0, 0.0), (60.0, 50.0)]),
    ]);
    let (out, approaches, none) = make_approaches(&g);
    assert_eq!(none, Vec::new());
    assert_eq!(
        approaches.iter().map(|a| a.end).collect::<Vec<_>>(),
        vec![0, 1]
    );
    let deck = by_id(&out, "1:1-2");
    assert!((length(&deck.points) - 44.0).abs() < 1e-9);
    assert_eq!(out.stretches.len(), g.stretches.len() + 2);
    assert_components_kept(&g, &out);
}

/// A deck of 40 m from node 1 east to node 2, a road passing across at node 1 (a T there) and, at node 2, the road
/// going on east for 100 m and a side road leaving node 2 north.
fn t_at_one_end_and_a_road_on_at_the_other() -> RoadGraph {
    build_graph(&[
        deck(1, &[1, 2], &[(0.0, 0.0), (40.0, 0.0)]),
        asf(2, &[5, 1, 6], &[(0.0, -50.0), (0.0, 0.0), (0.0, 50.0)]),
        asf(4, &[2, 3], &[(40.0, 0.0), (140.0, 0.0)]),
        grav(5, &[2, 7], &[(40.0, 0.0), (40.0, 50.0)]),
    ])
}

#[test]
fn one_end_of_a_deck_is_shortened_while_the_other_gets_its_side_road_moved() {
    let g = t_at_one_end_and_a_road_on_at_the_other();
    let (out, approaches, none) = make_approaches(&g);
    assert_eq!(none, Vec::new());
    assert_eq!(
        approaches
            .iter()
            .map(|a| (a.bridge.as_str(), a.end, a.kind))
            .collect::<Vec<_>>(),
        vec![
            ("1:1-2", 0, ApproachKind::BridgeShortened),
            ("1:1-2", 1, ApproachKind::SideRoadsMoved),
        ]
    );
    // the T end: the last 8 m before node 1 are a plain stretch of the deck's family, the deck keeps 32 m
    let piece = by_id(&out, "1:1-2/approach0");
    assert!(!piece.bridge && piece.bridge_kind.is_none() && piece.layer == 0);
    assert!((length(&piece.points) - 8.0).abs() < 1e-9);
    assert!(piece.start == 1 || piece.end == 1);
    let deck = by_id(&out, "1:1-2");
    assert!(deck.bridge);
    assert!((length(&deck.points) - 32.0).abs() < 1e-9);
    // the two share the cut node, and the deck still ends at the far end of the line
    let cut = if piece.start == 1 {
        piece.end
    } else {
        piece.start
    };
    assert!(cut == deck.start || cut == deck.end);
    assert!(deck.start == 2 || deck.end == 2);
    assert!(deck.points.contains(&(40.0, 0.0)));
    // the other end: the side road left node 2 and joins the road 8 m beyond the deck's far end
    let approach = by_id(&out, "4:2-3/approach0");
    assert!((length(&approach.points) - 8.0).abs() < 1e-9);
    let joint = if approach.start == 2 {
        approach.end
    } else {
        approach.start
    };
    assert_ne!(joint, 2);
    assert!(approach.points.contains(&(48.0, 0.0)));
    let side = by_id(&out, "5:2-7");
    assert!(side.start == joint || side.end == joint);
    assert!(side.points.contains(&(48.0, 0.0)));
    assert!(side.points.contains(&(40.0, 50.0)));
    assert_eq!(out.stretches.len(), g.stretches.len() + 2);
    assert_components_kept(&g, &out);
}

/// Three deck ends on node 2, 120 degrees apart: none of them continues another, so each would be a T.
fn three_decks_at_one_node() -> RoadGraph {
    build_graph(&[
        deck(1, &[1, 2], &[(0.0, 0.0), (40.0, 0.0)]),
        deck(2, &[2, 3], &[(40.0, 0.0), (15.0, 43.301_270_189_221_93)]),
        deck(3, &[2, 4], &[(40.0, 0.0), (10.0, -51.961_524_227_066_32)]),
    ])
}

#[test]
fn a_deck_that_ends_where_other_decks_end_is_not_shortened() {
    let g = three_decks_at_one_node();
    let (out, approaches, none) = make_approaches(&g);
    assert_eq!(approaches, Vec::new());
    assert_eq!(
        none.iter()
            .map(|n| (n.bridge.as_str(), n.end, n.reason))
            .collect::<Vec<_>>(),
        vec![
            ("1:1-2", 1, "no_room"),
            ("2:2-3", 0, "no_room"),
            ("3:2-4", 0, "no_room"),
        ]
    );
    assert!(none.iter().all(|n| n.at == (40.0, 0.0)));
    assert_eq!(out, g);
}

#[test]
fn a_side_road_that_would_cross_another_road_when_moved_stays_and_leaves_no_room() {
    let mut features = vec![
        deck(1, &[1, 2], &[(0.0, 0.0), (40.0, 0.0)]),
        asf(2, &[2, 3], &[(40.0, 0.0), (140.0, 0.0)]),
        grav(3, &[2, 4], &[(40.0, 0.0), (40.0, 50.0)]),
    ];
    features.push(asf(9, &[20, 21], &[(42.0, 20.0), (60.0, 20.0)]));
    let g = build_graph(&features);
    let (out, approaches, none) = make_approaches(&g);
    assert_eq!(approaches, Vec::new());
    assert_eq!(
        none.iter().map(|n| n.reason).collect::<Vec<_>>(),
        vec!["no_room"]
    );
    assert_eq!(out, g);
}

#[test]
fn a_side_road_that_is_a_deck_itself_stays_and_leaves_no_room() {
    // a second deck leaves the end node of the first, beside the road that goes on
    let g = build_graph(&[
        deck(1, &[1, 2], &[(0.0, 0.0), (40.0, 0.0)]),
        asf(2, &[2, 3], &[(40.0, 0.0), (140.0, 0.0)]),
        asf(5, &[3, 7], &[(140.0, 0.0), (140.0, -50.0)]),
        asf(6, &[3, 9], &[(140.0, 0.0), (240.0, 0.0)]),
        deck(3, &[2, 4], &[(40.0, 0.0), (40.0, 20.0)]),
    ]);
    let (out, approaches, none) = make_approaches(&g);
    assert_eq!(approaches, Vec::new());
    assert!(
        none.iter().all(|n| n.reason == "no_room") && !none.is_empty(),
        "{none:?}"
    );
    assert_eq!(out, g);
}

#[test]
fn a_road_that_goes_on_exactly_the_approach_to_a_deck_end_leaves_no_room() {
    // the far node of the 8 m road is the end of another deck (too short to be shortened itself)
    let g = build_graph(&[
        deck(1, &[1, 2], &[(0.0, 0.0), (40.0, 0.0)]),
        asf(2, &[2, 3], &[(40.0, 0.0), (48.0, 0.0)]),
        deck(7, &[3, 10], &[(48.0, 0.0), (48.0, 20.0)]),
        asf(6, &[3, 9], &[(48.0, 0.0), (148.0, 0.0)]),
        grav(3, &[2, 4], &[(40.0, 0.0), (40.0, 50.0)]),
    ]);
    let (out, approaches, none) = make_approaches(&g);
    assert_eq!(approaches, Vec::new());
    assert!(
        none.iter()
            .any(|n| n.bridge == "1:1-2" && n.reason == "no_room"),
        "{none:?}"
    );
    assert_eq!(out, g);
}

#[test]
fn a_side_road_whose_other_end_is_the_node_it_would_be_joined_to_stays_and_leaves_no_room() {
    // the road goes on exactly 8 m to node 3, and a second road runs from node 2 round to node 3 as well: moving it
    // onto node 3 would make a stretch from the node to itself
    let g = build_graph(&[
        deck(1, &[1, 2], &[(0.0, 0.0), (40.0, 0.0)]),
        asf(2, &[2, 3], &[(40.0, 0.0), (48.0, 0.0)]),
        asf(6, &[3, 9], &[(48.0, 0.0), (148.0, 0.0)]),
        grav(
            3,
            &[2, 4, 3],
            &[(40.0, 0.0), (40.0, 20.0), (48.0, 20.0), (48.0, 0.0)],
        ),
    ]);
    let (out, approaches, none) = make_approaches(&g);
    assert_eq!(approaches, Vec::new());
    assert_eq!(
        none.iter().map(|n| n.reason).collect::<Vec<_>>(),
        vec!["no_room"]
    );
    assert_eq!(out, g);
}

#[test]
fn the_approach_step_keeps_components_and_does_not_depend_on_the_order() {
    for g in [
        continuing(40.0, 100.0),
        continuing(40.0, 5.0),
        t_at_the_end(40.0),
        t_at_the_end(20.0),
        t_at_one_end_and_a_road_on_at_the_other(),
        three_decks_at_one_node(),
    ] {
        let mut reversed = g.clone();
        reversed.stretches.reverse();
        let a = make_approaches(&g);
        let b = make_approaches(&reversed);
        assert_eq!(sorted_stretches(&a.0), sorted_stretches(&b.0));
        assert_eq!((&a.1, &a.2), (&b.1, &b.2));
        assert!(components(&a.0) <= components(&g));
    }
}

#[test]
fn clean_makes_the_approaches_last_and_reports_them() {
    let g = t_at_the_end(40.0);
    let (out, report) = clean(&g, &widths, 8.0);
    assert_eq!(report.approaches.len(), 1);
    assert_eq!(out.stretches.len(), g.stretches.len() + 1);
    let g = t_at_the_end(20.0);
    let (_, report) = clean(&g, &widths, 8.0);
    assert_eq!(report.no_approach.len(), 1);
}

// ---- the paved and unpaved metres of a stretch stay right through the clean-up ----

/// Every stretch of `g` with all of its metres paved.
fn paved(mut g: RoadGraph) -> RoadGraph {
    for s in &mut g.stretches {
        s.paved_m = length(&s.points);
        s.unpaved_m = 0.0;
    }
    g
}

/// Every stretch of `g` has paved metres equal to its length, and no unpaved ones.
fn assert_all_paved(g: &RoadGraph) {
    assert_ne!(g.stretches.len(), 0);
    for s in &g.stretches {
        let l = length(&s.points);
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
fn removing_specks_keeps_the_metres_of_the_stretch() {
    let g = paved(build_graph(&[plain(
        1,
        &[1, 11, 12, 2],
        &[(0.0, 0.0), (10.0, 0.0), (10.1, 0.3), (50.0, 0.0)],
        Family::Asf3,
    )]));
    let (out, report) = clean(&g, &widths, 8.0);
    assert_eq!(report.specks.len(), 1);
    assert_all_paved(&out);
}

#[test]
fn merging_junctions_keeps_the_metres_of_the_roads_it_moves() {
    let g = paved(two_ts(4.0));
    let (out, merged, _) = merge_junctions(&g);
    assert_eq!(merged.len(), 1);
    assert_all_paved(&out);
}

#[test]
fn dropping_a_sidepath_keeps_the_metres_of_the_roads_that_stay() {
    let g = paved(build_graph(&track_with_a_side_road()));
    let (out, dropped, _) = drop_sidepaths(&g, &widths, CELL_M);
    assert_ne!(dropped.len(), 0);
    assert_all_paved(&out);
}

#[test]
fn a_bridge_approach_keeps_the_metres_of_what_it_cuts() {
    for g in [continuing(40.0, 100.0), t_at_the_end(40.0)] {
        let g = paved(g);
        let (out, approaches, _) = make_approaches(&g);
        assert_ne!(approaches.len(), 0);
        assert_all_paved(&out);
    }
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The frame of the network clean-up: the components of a graph and the steps, which are still empty.

use naves_core::catalog::Family;
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::netclean::{CleanReport, clean, components};
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
            sidepath_hint: false,
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

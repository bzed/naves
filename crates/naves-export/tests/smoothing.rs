// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The smoothing stage of `fitted_graph`: it runs before the bridges are fitted, and a switch turns it off.

use naves_core::bridgekind::{Lead, choose, fit_bridges};
use naves_core::catalog::Family;
use naves_core::catalog::testing::synthetic_catalog;
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::heightmap::Heightmap;
use naves_core::netclean;
use naves_core::roadnet::{build_graph, collapse_small_rings, drop_duplicates};
use naves_core::smooth::SMOOTH_ENABLED;
use naves_export::generate::{Stages, fitted_graph, fitted_graph_with};

fn way(id: i64, nodes: &[i64], points: &[(f64, f64)], bridge: bool) -> Feature {
    Feature {
        id: format!("osm/way/{id}/0"),
        way_id: id,
        source: Source::Osm,
        kind: FeatureKind::Road {
            family: Family::Asf3,
            drivable: true,
            bridge,
            tunnel: false,
            roundabout: false,
            layer: i8::from(bridge),
            use_sidepath: false,
            paving: None,
            held: false,
            plan: false,
        },
        points: points.to_vec(),
        nodes: nodes.to_vec(),
    }
}

fn flat() -> Heightmap {
    Heightmap {
        size: 250,
        cell_m: 4.0,
        h: vec![100.0; 250 * 250],
    }
}

/// A road that turns by 60 degrees `d` m before a 30 m bridge on a straight road. At 10 m the bridge's part
/// grows into the corner and the road beyond needs a lead on the unsmoothed line.
fn corner_at(d: f64) -> Vec<Feature> {
    vec![
        way(
            1,
            &[1, 2, 3],
            &[
                (500.0 - d - 100.0, 500.0 - 173.2),
                (500.0 - d, 500.0),
                (500.0, 500.0),
            ],
            false,
        ),
        way(2, &[3, 4], &[(500.0, 500.0), (530.0, 500.0)], true),
        way(3, &[4, 5], &[(530.0, 500.0), (900.0, 500.0)], false),
    ]
}

fn lead_is_laid(fits: &[naves_core::bridgekind::BridgeFit]) -> bool {
    fits.iter()
        .any(|f| matches!(f.lead.0, Lead::Laid { .. }) || matches!(f.lead.1, Lead::Laid { .. }))
}

#[test]
fn smoothing_runs_before_the_bridges_are_fitted() {
    let catalog = synthetic_catalog();
    // a corner 40 m before the bridge: outside the 25 m of the bridge approach that the widening holds
    let features = corner_at(40.0);
    let base = flat();
    let ground = |p: (f64, f64)| f64::from(base.sample(p.0, p.1).height);
    let off = fitted_graph_with(
        &features,
        &catalog,
        &base,
        Stages {
            reroute: false,
            smooth: false,
            lots: false,
        },
        &|_| false,
        &[],
        &[],
    );
    let fitted = fitted_graph_with(
        &features,
        &catalog,
        &base,
        Stages {
            reroute: false,
            smooth: true,
            lots: false,
        },
        &|_| false,
        &[],
        &[],
    );
    let report = fitted.smooth.as_ref().expect("smoothing ran");
    assert!(
        report.max_move_m > 0.0 || report.max_outward_m > 0.0,
        "{report:?}"
    );
    assert_ne!(fitted.built, off.built, "the corner was not smoothed");
    // the bridges are fitted on the smoothed graph
    let (graph, bridges) = fit_bridges(&fitted.built, &|s| choose(&catalog, s), &ground);
    assert_eq!(fitted.graph, graph);
    assert_eq!(fitted.bridges, bridges);
    assert_eq!(fitted.bridges.len(), 1);
}

#[test]
fn a_corner_in_the_approach_of_a_bridge_is_held_and_needs_its_lead() {
    // 10 m before the bridge the widening is held (WIDEN_BRIDGE_HOLD_M), the bend cannot be widened and goes back
    // to the line as drawn, so the road beyond still leaves 60 degrees off the deck's heading and gets a lead
    let catalog = synthetic_catalog();
    let features = corner_at(10.0);
    let base = flat();
    let fitted = fitted_graph_with(
        &features,
        &catalog,
        &base,
        Stages {
            reroute: false,
            smooth: true,
            lots: false,
        },
        &|_| false,
        &[],
        &[],
    );
    assert!(lead_is_laid(&fitted.bridges), "{:?}", fitted.bridges);
    let report = fitted.smooth.as_ref().expect("smoothing ran");
    assert!(
        report.zones.iter().any(|z| z.plate_candidate),
        "{:?}",
        report.zones
    );
}

#[test]
fn the_switch_off_leaves_the_graph_as_it_was_and_says_so() {
    let catalog = synthetic_catalog();
    let features = corner_at(10.0);
    let base = flat();
    let off = fitted_graph_with(
        &features,
        &catalog,
        &base,
        Stages {
            reroute: false,
            smooth: false,
            lots: false,
        },
        &|_| false,
        &[],
        &[],
    );
    assert!(off.smooth.is_none());
    // the graph is the cleaned graph itself: nothing of the stage is in it
    let (flat_graph, _) =
        collapse_small_rings(&build_graph(&features), &|f| catalog.tightest_arc(f));
    let (deduplicated, _) = drop_duplicates(&flat_graph);
    let (cleaned, _) = netclean::clean(&deduplicated, &|f| f.width_m(), f64::from(base.cell_m));
    assert_eq!(off.built, cleaned);
    // and the stage on does change it (this corner is one the stage widens), so the pin above means something
    let on = fitted_graph_with(
        &features,
        &catalog,
        &base,
        Stages {
            reroute: false,
            smooth: true,
            lots: false,
        },
        &|_| false,
        &[],
        &[],
    );
    assert_ne!(on.built, cleaned);
    // the graph is what fit_bridges makes of the cleaned graph
    let ground = |p: (f64, f64)| f64::from(base.sample(p.0, p.1).height);
    let (graph, bridges) = fit_bridges(&off.built, &|s| choose(&catalog, s), &ground);
    assert_eq!(off.graph, graph);
    assert_eq!(off.bridges, bridges);
    // and `fitted_graph` follows the constant
    let default = fitted_graph(&features, &catalog, &base, &|_| false, &[], &[]);
    assert_eq!(default.smooth.is_some(), SMOOTH_ENABLED);
    if !SMOOTH_ENABLED {
        // the build's graph is bitwise the cleaned graph
        assert_eq!(default.built, cleaned);
    }
}

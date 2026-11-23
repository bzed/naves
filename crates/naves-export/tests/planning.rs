// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Road planning in `fitted_graph`: it runs after the clean-up and before the smoothing, a switch turns it
//! off, and the report of the stage reaches `build/report.json`.

#![expect(
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test helper code; failing loudly is the point, and the polylines have at least two points"
)]

use std::cell::RefCell;

use naves_core::catalog::Family;
use naves_core::catalog::testing::synthetic_catalog;
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::heightmap::{Heightmap, MapGeo};
use naves_core::project::Project;
use naves_core::reroute::{
    AnchorKind, Candidate, KeptReason, PlannedZone, REROUTE_ENABLED, RerouteReport, ZoneOutcome,
};
use naves_core::roadnet::RoadGraph;
use naves_export::build::planning_json;
use naves_export::generate::{Stages, fitted_graph, fitted_graph_with, plan_roads_with};

fn way(id: i64, nodes: &[i64], points: &[(f64, f64)]) -> Feature {
    Feature {
        id: format!("osm/way/{id}/0"),
        way_id: id,
        source: Source::Osm,
        kind: FeatureKind::Road {
            family: Family::City,
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

/// 2048 m of flat ground at 4 m cells: the map the planning fixtures are laid on.
fn flat() -> Heightmap {
    Heightmap {
        size: 513,
        cell_m: 4.0,
        h: vec![100.0; 513 * 513],
    }
}

/// A right-angled city corner at (500, 500), with a through road at each of its junctions: the fixture of
/// `naves-core/tests/reroute.rs`; the smoothing tests use `corner_at` in `tests/smoothing.rs`.
fn town_corner() -> Vec<Feature> {
    vec![
        way(
            1,
            &[1, 2, 3],
            &[(400.0, 500.0), (500.0, 500.0), (500.0, 600.0)],
        ),
        way(
            2,
            &[10, 1, 11],
            &[(400.0, 400.0), (400.0, 500.0), (400.0, 600.0)],
        ),
        way(
            3,
            &[12, 3, 13],
            &[(400.0, 600.0), (500.0, 600.0), (600.0, 600.0)],
        ),
    ]
}

/// The stretch of the corner itself, whose line the stage plans.
const CORNER: &str = "1:1-3";

fn points_of(graph: &RoadGraph, id: &str) -> Vec<(f64, f64)> {
    graph
        .stretches
        .iter()
        .find(|s| s.id == id)
        .map_or_else(|| panic!("no stretch {id}"), |s| s.points.clone())
}

/// Proper crossings of two polylines: a shared end or a touching point is none of them.
fn crossings(a: &[(f64, f64)], b: &[(f64, f64)]) -> usize {
    let side = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| {
        (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
    };
    let mut n = 0;
    for w in a.windows(2) {
        for v in b.windows(2) {
            let (d1, d2) = (side(w[0], w[1], v[0]), side(w[0], w[1], v[1]));
            let (d3, d4) = (side(v[0], v[1], w[0]), side(v[0], v[1], w[1]));
            if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn the_stages_off_leave_the_graph_of_today() {
    let catalog = synthetic_catalog();
    let base = flat();
    let features = town_corner();
    let plain = fitted_graph_with(
        &features,
        &catalog,
        &base,
        Stages {
            reroute: false,
            smooth: false,
        },
        &|_| false,
        &[],
    );
    assert!(plain.reroute.is_none());
    assert!(plain.smooth.is_none());
    assert_eq!(plain.reroute_time, std::time::Duration::ZERO);
    // the graph is the cleaned graph, before any stage: what the build made before the planning existed
    let (flat_graph, _) = naves_core::roadnet::collapse_small_rings(
        &naves_core::roadnet::build_graph(&features),
        &|f| catalog.tightest_arc(f),
    );
    let (deduplicated, _) = naves_core::roadnet::drop_duplicates(&flat_graph);
    let (cleaned, _) =
        naves_core::netclean::clean(&deduplicated, &|f| f.width_m(), f64::from(base.cell_m));
    assert_eq!(plain.built, cleaned);
}

#[test]
fn a_tight_corner_is_planned_and_only_its_chain_moves() {
    let catalog = synthetic_catalog();
    let base = flat();
    let features = town_corner();
    let off = fitted_graph_with(
        &features,
        &catalog,
        &base,
        Stages {
            reroute: false,
            smooth: false,
        },
        &|_| false,
        &[],
    );
    let on = fitted_graph_with(
        &features,
        &catalog,
        &base,
        Stages {
            reroute: true,
            smooth: false,
        },
        &|_| false,
        &[],
    );
    let report = on.reroute.as_ref().expect("the planning ran");
    assert_eq!(report.zones.len(), 1, "{:?}", report.zones);
    assert!(
        matches!(report.zones[0].outcome, ZoneOutcome::Planned),
        "{:?}",
        report.zones[0]
    );
    assert_eq!(report.zones[0].stretches, vec![CORNER.to_string()]);
    assert!(on.reroute_time > std::time::Duration::ZERO);
    // the corner's line moved, everything else is bit for bit what it was
    assert_ne!(points_of(&on.built, CORNER), points_of(&off.built, CORNER));
    assert_eq!(on.built.stretches.len(), off.built.stretches.len());
    for (after, before) in on.built.stretches.iter().zip(&off.built.stretches) {
        assert_eq!(after.id, before.id);
        assert_eq!(after.start, before.start);
        assert_eq!(after.end, before.end);
        if after.id != CORNER {
            assert_eq!(after.points, before.points, "{} moved", after.id);
        }
    }
}

#[test]
fn a_stream_beside_the_corner_is_not_crossed_by_a_planned_route() {
    let catalog = synthetic_catalog();
    let base = flat();
    let features = town_corner();
    // a short stream inside the corner, on the arc a route without the stream takes
    let stream = [(497.0, 502.0), (499.0, 508.0)];
    let free = fitted_graph_with(
        &features,
        &catalog,
        &base,
        Stages {
            reroute: true,
            smooth: false,
        },
        &|_| false,
        &[],
    );
    assert!(
        crossings(&points_of(&free.built, CORNER), &stream) >= 1,
        "the fixture is not discriminating: the free route misses the stream"
    );
    let held = fitted_graph_with(
        &features,
        &catalog,
        &base,
        Stages {
            reroute: true,
            smooth: false,
        },
        &|_| false,
        &[stream.to_vec()],
    );
    let report = held.reroute.as_ref().expect("the planning ran");
    assert_eq!(report.zones.len(), 1, "{:?}", report.zones);
    assert!(
        matches!(report.zones[0].outcome, ZoneOutcome::Planned),
        "{:?}",
        report.zones[0]
    );
    assert_eq!(
        crossings(&points_of(&held.built, CORNER), &stream),
        0,
        "a route crosses a stream the drawn line does not"
    );
}

#[test]
fn the_default_follows_the_switch() {
    let catalog = synthetic_catalog();
    let base = flat();
    let features = town_corner();
    let default = fitted_graph(&features, &catalog, &base, &|_| false, &[]);
    assert_eq!(default.reroute.is_some(), REROUTE_ENABLED);
}

#[test]
fn the_report_of_the_stage_reaches_the_planning_section() {
    assert_eq!(planning_json(None)["enabled"], false);
    let zone = |id: &str, at: (f64, f64), outcome: ZoneOutcome, relax: usize| PlannedZone {
        id: id.to_string(),
        at,
        stretches: vec!["7:1-2".to_string()],
        legs: 1,
        anchors: (AnchorKind::Line, AnchorKind::Line),
        junction: None,
        partner: None,
        outcome,
        radius_before_m: 4.0,
        radius_wanted_m: 10.0,
        radius_after_m: 10.5,
        length_before_m: 40.0,
        length_after_m: 42.0,
        max_stray_m: 1.5,
        drawn_grade: 0.05,
        drawn_grade_window: 0.09,
        planned_grade: 0.14,
        grade_limit: 0.14,
        relax_steps: relax,
        climb_m: 1.0,
        earthwork_m2: 2.5,
        cut_max_m: 0.2,
        fill_max_m: 0.3,
        states: 42,
        weight: Some(1.0),
        budget_exhausted: false,
        original_feasible: true,
        candidate: None,
    };
    let mut planned = zone("7:1-2#0", (1.0, 2.0), ZoneOutcome::Planned, 0);
    // the bend leaves a junction and ends inside the chain: both ends are named, the partner is not
    planned.anchors = (AnchorKind::Junction(5), AnchorKind::Line);
    planned.junction = Some(5);
    let mut kept = zone(
        "7:1-2#1",
        (3.0, 4.0),
        ZoneOutcome::Kept {
            reason: KeptReason::Held,
            detail: "anchors_close".to_string(),
        },
        0,
    );
    kept.candidate = Some(Candidate::Plate);
    // this one is the other half of the same bend across the junction
    kept.anchors = (AnchorKind::Line, AnchorKind::BridgeHold);
    kept.partner = Some("7:1-2#0".to_string());
    let report = RerouteReport {
        zones: vec![planned, kept],
        kept_zones: vec![("7:1-2#1".to_string(), "anchors_close".to_string())],
        tight_before_m: 120.0,
        tight_after_m: 40.0,
        max_stray_m: 1.5,
        steeper_than_drawn: 1,
        reverted: 0,
        states: 42,
        ..RerouteReport::default()
    };
    let json = planning_json(Some(&report));
    assert_eq!(json["enabled"], true);
    assert_eq!(json["zones"].as_array().unwrap().len(), 2);
    assert_eq!(json["tight_before_m"], 120.0);
    assert_eq!(json["tight_after_m"], 40.0);
    assert_eq!(json["steeper_than_drawn"], 1);
    assert_eq!(json["reverted"], 0);
    let first = &json["zones"][0];
    assert_eq!(first["outcome"], "planned");
    assert!(first["reason"].is_null());
    assert!(first["candidate"].is_null());
    assert!(first["detail"].is_null());
    assert_eq!(first["anchors"], serde_json::json!(["junction", "line"]));
    assert_eq!(first["junction"], 5);
    assert!(first["partner"].is_null());
    assert_eq!(first["drawn_grade"], 0.05);
    assert_eq!(first["planned_grade"], 0.14);
    assert_eq!(first["grade_limit"], 0.14);
    assert_eq!(first["relax_steps"], 0);
    assert_eq!(first["earthwork_m2"], 2.5);
    let second = &json["zones"][1];
    assert_eq!(second["outcome"], "kept");
    assert_eq!(second["reason"], "held");
    assert_eq!(second["detail"], "anchors_close");
    assert_eq!(
        second["anchors"],
        serde_json::json!(["line", "bridge_hold"])
    );
    assert!(second["junction"].is_null());
    assert_eq!(second["partner"], "7:1-2#0");
    assert_eq!(second["candidate"], "plate");
    assert_eq!(
        Candidate::Plate.id(),
        "plate",
        "the candidate of a held zone"
    );
}

#[test]
fn the_stage_prints_its_progress_line_with_its_time() {
    let catalog = synthetic_catalog();
    let base = flat();
    let features = town_corner();
    let frame = MapGeo::new(&Project::new("Planning", 47.55, 12.975, 1000.0)).unwrap();
    let lines = RefCell::new(Vec::new());
    plan_roads_with(
        &features,
        &catalog,
        &base,
        &[],
        &[],
        &frame,
        &|l| {
            lines.borrow_mut().push(l.to_string());
        },
        Stages {
            reroute: true,
            smooth: false,
        },
    )
    .unwrap();
    let lines = lines.into_inner();
    // the line is one sentence, like the smoothing's: `Planning the tight curves (<s> s)`, with the
    // stage's own time in it, and there is no smoothing line with the smoothing off
    let planned: Vec<&String> = lines
        .iter()
        .filter(|l| l.starts_with("Planning the tight curves (") && l.ends_with(" s)"))
        .collect();
    assert_eq!(planned.len(), 1, "{lines:#?}");
    let seconds = planned[0]
        .strip_prefix("Planning the tight curves (")
        .and_then(|l| l.strip_suffix(" s)"))
        .map_or_else(
            || panic!("the line has no time: {}", planned[0]),
            str::parse::<f64>,
        );
    assert!(seconds.expect("one decimal") >= 0.0);
    assert!(
        !lines
            .iter()
            .any(|l| l.starts_with("Smoothing the road lines")),
        "a stage that is off prints no line: {lines:#?}"
    );
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dips: where the planned road stands high above the original ground, and what is done about it.

#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::catalog::Family;
use naves_core::dips::{BRIDGE_DIP_M, BRIDGE_STRAIGHT_M, DIP_M, Dip, DipAction, decide, find_dips};
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::heightmap::{Heightmap, MapGeo};
use naves_core::osm::LandPolygon;
use naves_core::profile::{Profiles, RoadStrip};
use naves_core::project::Project;
use naves_core::roadfit::Pose;
use naves_core::roadnet::{RoadGraph, build_graph};
use naves_core::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad, Slot};
use naves_core::surface::SurfaceClass;

/// Flat at 100 m on a map of 400 m with a triangular hollow across it.
fn hollow(centre: f64, width: f64, depth: f64) -> Heightmap {
    let (size, cell) = (41_u32, 10.0_f64);
    let mut h = Vec::new();
    for _row in 0..size {
        for col in 0..size {
            let x = f64::from(col) * cell;
            let dip = depth * (1.0 - (x - centre).abs() / (width / 2.0)).max(0.0);
            h.push((100.0 - dip) as f32);
        }
    }
    Heightmap {
        size,
        cell_m: cell as f32,
        h,
    }
}

fn feature(id: i64, points: &[(f64, f64)], nodes: &[i64], family: Family) -> Feature {
    Feature {
        id: format!("osm/way/{id}/0"),
        way_id: id,
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

/// A road along y = 200 from x = 0 to x = 400, a dead end at both ends.
fn straight_road(family: Family) -> RoadGraph {
    build_graph(&[feature(1, &[(0.0, 200.0), (400.0, 200.0)], &[1, 2], family)])
}

fn piece(stretch: &str, index: u32, from: (f64, f64), to: (f64, f64)) -> PlacedRoad {
    let pose = |p: (f64, f64)| Pose {
        x: p.0,
        y: p.1,
        heading: 90.0_f64.to_radians(),
    };
    PlacedRoad {
        slot: Slot {
            stretch: stretch.to_string(),
            index,
        },
        kind: PlacedKind::Piece,
        family: Family::Asf3,
        drivable: true,
        model: "m".to_string(),
        origin: pose(from),
        start: pose(from),
        end: pose(to),
        width: 7.0,
        scale: 1.0,
    }
}

/// The road of stretch 0 of `graph` as pieces of 25 m, every joint at `height`.
fn flat_plan(graph: &RoadGraph, height: f64) -> (PlacedNetwork, Profiles) {
    let stretch = &graph.stretches[0];
    let (a, b) = (stretch.points[0], *stretch.points.last().unwrap());
    let n = ((b.0 - a.0).hypot(b.1 - a.1) / 25.0).round() as u32;
    let at = |i: u32| {
        let t = f64::from(i) / f64::from(n);
        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
    };
    let roads: Vec<PlacedRoad> = (0..n)
        .map(|i| piece(&stretch.id, i, at(i), at(i + 1)))
        .collect();
    let strip = RoadStrip {
        slots: (0..roads.len()).collect(),
        joints: (0..=n).map(|i| (at(i).0, at(i).1, height)).collect(),
        bridge: Vec::new(),
        junction: Vec::new(),
        side_slope: Vec::new(),
        half_width: 3.5,
        rank: 3,
    };
    let net = PlacedNetwork {
        chains: vec![(0..roads.len()).collect()],
        roads,
        ..PlacedNetwork::default()
    };
    (
        net,
        Profiles {
            strips: vec![strip],
            excesses: Vec::new(),
        },
    )
}

fn frame() -> MapGeo {
    MapGeo::new(&Project::new("D", 47.55, 12.975, 1000.0)).unwrap()
}

fn dips_with(
    base: &Heightmap,
    graph: &RoadGraph,
    water: &[Vec<(f64, f64)>],
    lakes: &[LandPolygon],
) -> Vec<Dip> {
    let (net, profiles) = flat_plan(graph, 100.0);
    find_dips(graph, &net, &profiles, base, water, lakes, &frame())
}

fn dips_on(base: &Heightmap, graph: &RoadGraph) -> Vec<Dip> {
    dips_with(base, graph, &[], &[])
}

fn dip(stretch: usize, from_m: f64, to_m: f64, over_water: bool) -> Dip {
    Dip {
        stretch,
        from_m,
        to_m,
        depth_m: 8.0,
        over_water,
        at: (f64::midpoint(from_m, to_m), 200.0),
    }
}

#[test]
fn a_road_over_a_hollow_has_a_dip_as_long_as_the_hollow() {
    // flat at 100 with a 40 m wide, 8 m deep hollow across the road at x = 200: the road is more than 3 m
    // above the ground where the hollow is deeper than 3 m, about 25 m
    let d = dips_on(&hollow(200.0, 40.0, 8.0), &straight_road(Family::Asf3));
    assert_eq!(d.len(), 1, "{d:?}");
    assert!(
        (d[0].depth_m - 8.0).abs() < 1.0 && !d[0].over_water,
        "{d:?}"
    );
    let length = d[0].to_m - d[0].from_m;
    assert!(length > 20.0 && length < 45.0, "{length}");
    assert!((f64::midpoint(d[0].from_m, d[0].to_m) - 200.0).abs() < 3.0);
    assert!((d[0].at.0 - 200.0).abs() < 3.0 && (d[0].at.1 - 200.0).abs() < 1e-6);
    assert_eq!(d[0].stretch, 0);
}

#[test]
fn a_hollow_of_two_metres_is_not_a_dip() {
    let d = dips_on(&hollow(200.0, 40.0, 2.0), &straight_road(Family::Asf3));
    assert!(d.is_empty(), "{d:?}");
    assert!((DIP_M - 3.0).abs() < 1e-12);
}

#[test]
fn two_hollows_make_two_dips_in_the_order_along_the_stretch() {
    let mut base = hollow(100.0, 40.0, 8.0);
    let second = hollow(300.0, 40.0, 8.0);
    for (a, b) in base.h.iter_mut().zip(&second.h) {
        *a = a.min(*b);
    }
    let d = dips_on(&base, &straight_road(Family::Asf3));
    assert_eq!(d.len(), 2, "{d:?}");
    assert!(d[0].to_m < d[1].from_m);
}

#[test]
fn a_dip_over_unknown_ground_ends_where_the_ground_is_not_a_number() {
    let mut base = hollow(200.0, 40.0, 8.0);
    // the ground is unknown from x = 200 on: no sample there is part of a run
    for row in 0..41 {
        for col in 20..41 {
            base.h[row * 41 + col] = f32::NAN;
        }
    }
    let d = dips_on(&base, &straight_road(Family::Asf3));
    assert!(
        d.iter().all(|d| d.to_m < 215.0 && d.depth_m.is_finite()),
        "{d:?}"
    );
}

#[test]
fn a_dip_with_room_gets_a_bridge_of_whole_models() {
    let graph = straight_road(Family::Asf3);
    let (action, span) = decide(&dip(0, 180.0, 220.0, false), &graph, Some(25.0));
    assert_eq!(action, DipAction::Bridge);
    let span = span.unwrap();
    assert_eq!(span.stretch, 0);
    // 40 m need two models
    assert!(((span.to_m - span.from_m) / 25.0 - 2.0).abs() < 1e-9);
    assert!((f64::midpoint(span.from_m, span.to_m) - 200.0).abs() < 1e-9);
}

#[test]
fn a_long_dip_gets_a_chain() {
    // 80 m with a 25 m model: four models
    let graph = straight_road(Family::Asf3);
    let (action, span) = decide(&dip(0, 160.0, 240.0, false), &graph, Some(25.0));
    assert_eq!(action, DipAction::Bridge);
    let span = span.unwrap();
    assert!(((span.to_m - span.from_m) / 25.0 - 4.0).abs() < 1e-9);
    assert!((f64::midpoint(span.from_m, span.to_m) - 200.0).abs() < 1e-9);
}

#[test]
fn a_run_shorter_than_half_a_model_gets_no_bridge() {
    let graph = straight_road(Family::Asf3);
    assert_eq!(
        decide(&dip(0, 195.0, 205.0, false), &graph, Some(25.0)).0,
        DipAction::Fill
    );
    // exactly half a model is long enough
    assert_eq!(
        decide(&dip(0, 187.5, 200.0, false), &graph, Some(25.0)).0,
        DipAction::Bridge
    );
}

#[test]
fn a_short_dry_dip_is_filled_and_a_short_wet_one_is_reported() {
    let graph = straight_road(Family::Asf3);
    assert_eq!(
        decide(&dip(0, 195.0, 205.0, false), &graph, Some(25.0)),
        (DipAction::Fill, None)
    );
    assert_eq!(
        decide(&dip(0, 195.0, 205.0, true), &graph, Some(25.0)),
        (DipAction::Report, None)
    );
}

/// Three stretches that meet at node 1 at x = 0: the road east, one north and one south.
fn graph_starting_at_a_junction() -> RoadGraph {
    build_graph(&[
        feature(1, &[(0.0, 200.0), (400.0, 200.0)], &[1, 2], Family::Asf3),
        feature(2, &[(0.0, 200.0), (0.0, 300.0)], &[1, 3], Family::Asf3),
        feature(3, &[(0.0, 200.0), (0.0, 100.0)], &[1, 4], Family::Asf3),
    ])
}

fn index_of(graph: &RoadGraph, way: &str) -> usize {
    graph
        .stretches
        .iter()
        .position(|s| s.id.starts_with(way))
        .unwrap()
}

#[test]
fn a_dip_at_a_junction_or_at_the_end_of_a_dead_end_gets_no_bridge() {
    // run from 2 m to 30 m of a stretch that starts at a three-way crossing: two models would reach it
    let graph = graph_starting_at_a_junction();
    let east = index_of(&graph, "1:");
    assert_eq!(
        decide(&dip(east, 2.0, 30.0, false), &graph, Some(25.0)).0,
        DipAction::Fill
    );
    // the last 20 m of a dead end: a model centred on the run reaches beyond the end
    let dead_end = straight_road(Family::Asf3);
    assert_eq!(
        decide(&dip(0, 380.0, 400.0, true), &dead_end, Some(25.0)).0,
        DipAction::Report
    );
    // a span that stays 1 m clear of the crossing is fine
    assert_eq!(
        decide(&dip(east, 14.0, 40.0, false), &graph, Some(25.0)).0,
        DipAction::Bridge
    );
}

#[test]
fn a_span_may_reach_the_end_of_a_stretch_that_continues_into_another() {
    // two stretches of different families meet at node 2: nothing but this road is there
    let graph = build_graph(&[
        feature(1, &[(0.0, 200.0), (200.0, 200.0)], &[1, 2], Family::Asf3),
        feature(2, &[(200.0, 200.0), (400.0, 200.0)], &[2, 3], Family::Grav),
    ]);
    let first = index_of(&graph, "1:");
    assert_eq!(
        decide(&dip(first, 175.0, 200.0, false), &graph, Some(25.0)).0,
        DipAction::Bridge
    );
}

#[test]
fn a_dip_in_a_bend_gets_no_bridge() {
    // an arc of radius 30 m through 90 degrees: over 25 m it leaves the chord by more than 2 m
    let radius = 30.0;
    let points: Vec<(f64, f64)> = (0..=18)
        .map(|i| {
            let a = f64::from(i) * 5.0_f64.to_radians();
            (100.0 + radius * a.sin(), 100.0 + radius * (1.0 - a.cos()))
        })
        .collect();
    let graph = build_graph(&[feature(1, &points, &[1, 2], Family::Asf3)]);
    let length = radius * 90.0_f64.to_radians();
    let at = length / 2.0;
    let found = dip(0, at - 10.0, at + 10.0, false);
    assert_eq!(decide(&found, &graph, Some(25.0)).0, DipAction::Fill);
    assert!((BRIDGE_STRAIGHT_M - 2.0).abs() < 1e-12);
    // the same run on a straight road is bridged
    let (action, _) = decide(&found, &straight_road(Family::Asf3), Some(25.0));
    assert_eq!(action, DipAction::Bridge);
}

#[test]
fn a_dip_is_bridged_only_from_the_bridge_depth() {
    let graph = straight_road(Family::Asf3);
    let deep = |depth_m: f64, over_water: bool| Dip {
        depth_m,
        ..dip(0, 180.0, 220.0, over_water)
    };
    let model = Some(25.0);
    // the same dip, with room for a bridge, 0.1 m too shallow: filled when dry, reported over water
    assert_eq!(
        decide(&deep(BRIDGE_DIP_M - 0.1, false), &graph, model),
        (DipAction::Fill, None)
    );
    assert_eq!(
        decide(&deep(BRIDGE_DIP_M - 0.1, true), &graph, model),
        (DipAction::Report, None)
    );
    // and at the depth it gets its bridge, wet or dry
    for wet in [false, true] {
        let (action, span) = decide(&deep(BRIDGE_DIP_M, wet), &graph, model);
        assert_eq!((action, span.is_some()), (DipAction::Bridge, true), "{wet}");
    }
    // a depth that is not a number gets none
    for depth in [f64::NAN, f64::INFINITY] {
        assert_eq!(
            decide(&deep(depth, false), &graph, model),
            (DipAction::Fill, None),
            "{depth}"
        );
    }
    assert!((BRIDGE_DIP_M - 8.0).abs() < 1e-12 && DIP_M < BRIDGE_DIP_M);
}

#[test]
fn no_model_means_fill_or_report() {
    let graph = straight_road(Family::Asf3);
    assert_eq!(
        decide(&dip(0, 180.0, 220.0, false), &graph, None),
        (DipAction::Fill, None)
    );
    assert_eq!(
        decide(&dip(0, 180.0, 220.0, true), &graph, None),
        (DipAction::Report, None)
    );
    // a model without a usable length is no model
    for length in [0.0, -5.0, 1e-9, 0.99, f64::NAN, f64::INFINITY] {
        assert_eq!(
            decide(&dip(0, 180.0, 220.0, false), &graph, Some(length)).0,
            DipAction::Fill,
            "{length}"
        );
    }
}

#[test]
fn a_dip_on_a_stretch_that_is_not_there_is_not_bridged() {
    let graph = straight_road(Family::Asf3);
    assert_eq!(
        decide(&dip(7, 180.0, 220.0, false), &graph, Some(25.0)).0,
        DipAction::Fill
    );
}

#[test]
fn a_stream_under_the_run_makes_it_wet_and_one_beside_it_does_not() {
    let base = hollow(200.0, 40.0, 8.0);
    let graph = straight_road(Family::Asf3);
    let across = vec![vec![(200.0, 150.0), (200.0, 250.0)]];
    let wet = dips_with(&base, &graph, &across, &[]);
    assert!(wet[0].over_water, "{wet:?}");
    // a stream beside the road, 5 m away, never crossing it
    let beside = vec![vec![(150.0, 205.0), (250.0, 205.0)]];
    assert!(!dips_with(&base, &graph, &beside, &[])[0].over_water);
    // a stream that crosses the road far from the run
    let elsewhere = vec![vec![(50.0, 150.0), (50.0, 250.0)]];
    assert!(!dips_with(&base, &graph, &elsewhere, &[])[0].over_water);
    // a stream far away on both axes, and a line without points
    let far = vec![vec![(9000.0, 9000.0), (9100.0, 9200.0)], vec![]];
    assert!(!dips_with(&base, &graph, &far, &[])[0].over_water);
    // a stream that ends before the road
    let short = vec![vec![(200.0, 150.0), (200.0, 195.0)]];
    assert!(!dips_with(&base, &graph, &short, &[])[0].over_water);
}

fn lake_around(x: f64, y: f64, priority: u8) -> LandPolygon {
    let frame = frame();
    let corner = |dx: f64, dy: f64| frame.to_geo(x + dx, y + dy).unwrap();
    LandPolygon {
        class: SurfaceClass::Soil,
        priority,
        outer: vec![
            corner(-30.0, -30.0),
            corner(30.0, -30.0),
            corner(30.0, 30.0),
            corner(-30.0, 30.0),
            corner(-30.0, -30.0),
        ],
        holes: Vec::new(),
    }
}

#[test]
fn a_lake_under_the_middle_of_the_run_makes_it_wet() {
    let base = hollow(200.0, 40.0, 8.0);
    let graph = straight_road(Family::Asf3);
    let wet = dips_with(&base, &graph, &[], &[lake_around(200.0, 200.0, 3)]);
    assert!(wet[0].over_water);
    // an area of another class is no water
    let dry = dips_with(&base, &graph, &[], &[lake_around(200.0, 200.0, 1)]);
    assert!(!dry[0].over_water);
    // a lake somewhere else
    let away = dips_with(&base, &graph, &[], &[lake_around(50.0, 200.0, 3)]);
    assert!(!away[0].over_water);
}

#[test]
fn a_bridge_that_was_not_placed_is_a_road_with_dips() {
    // the stretch is flagged as a bridge but nothing of it was placed: it is road, and it has its dips
    let mut graph = straight_road(Family::Asf3);
    graph.stretches[0].bridge = true;
    let base = hollow(200.0, 40.0, 8.0);
    let (net, profiles) = flat_plan(&graph, 100.0);
    let found = find_dips(&graph, &net, &profiles, &base, &[], &[], &frame());
    assert_eq!(found.len(), 1, "{found:?}");
    // placed as a bridge, it has none
    let mut placed = net.clone();
    for road in &mut placed.roads {
        road.kind = PlacedKind::Bridge;
    }
    assert_eq!(
        find_dips(&graph, &placed, &profiles, &base, &[], &[], &frame()).len(),
        0
    );
    // a tunnel never has dips
    graph.stretches[0].bridge = false;
    graph.stretches[0].tunnel = true;
    assert_eq!(
        find_dips(&graph, &net, &profiles, &base, &[], &[], &frame()).len(),
        0
    );
}

#[test]
fn a_road_far_from_its_stretch_is_not_measured() {
    // the pieces lie 20 m north of the stretch (a junction piece would): no sample has a road under it
    let graph = straight_road(Family::Asf3);
    let (mut net, mut profiles) = flat_plan(&graph, 100.0);
    for road in &mut net.roads {
        road.start.y += 20.0;
        road.end.y += 20.0;
    }
    for joint in &mut profiles.strips[0].joints {
        joint.1 += 20.0;
    }
    let found = find_dips(
        &graph,
        &net,
        &profiles,
        &hollow(200.0, 40.0, 8.0),
        &[],
        &[],
        &frame(),
    );
    assert!(found.is_empty(), "{found:?}");
}

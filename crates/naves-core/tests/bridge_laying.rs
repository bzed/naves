// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Laying what an OpenStreetMap bridge is built from: panel grids, and models that are centred or lie the way
//! the stretch runs.

#![expect(
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "test helper code; failing loudly is the point"
)]

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use naves_core::bridgekind::{BridgeFit, BridgeKind, choose, fit_bridges, panel_grid};
use naves_core::catalog::testing::synthetic_catalog;
use naves_core::catalog::{Family, PieceCatalog, Slab};
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::objects::road_objects;
use naves_core::roadnet::build_graph;
use naves_core::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad, place_network};

fn feature(way: i64, family: Family, bridge: bool, nodes: &[i64], pts: &[(f64, f64)]) -> Feature {
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
        },
        points: pts.to_vec(),
        nodes: nodes.to_vec(),
    }
}

/// A road along y = 500 with a bridge of `span` metres from x = 400.
fn bridged(family: Family, span: f64) -> Vec<Feature> {
    vec![
        feature(1, family, false, &[1, 2], &[(100.0, 500.0), (400.0, 500.0)]),
        feature(
            2,
            family,
            true,
            &[2, 3],
            &[(400.0, 500.0), (400.0 + span, 500.0)],
        ),
        feature(
            3,
            family,
            false,
            &[3, 4],
            &[(400.0 + span, 500.0), (900.0, 500.0)],
        ),
    ]
}

fn fitted(
    features: &[Feature],
    ground: &dyn Fn((f64, f64)) -> f64,
) -> (PlacedNetwork, Vec<BridgeFit>, PieceCatalog) {
    let catalog = synthetic_catalog();
    let (graph, fits) = fit_bridges(
        &build_graph(features),
        &|site| choose(&catalog, site),
        ground,
    );
    let net = place_network(&graph, &catalog).unwrap();
    (net, fits, catalog)
}

fn flat(_: (f64, f64)) -> f64 {
    0.0
}

fn slab(catalog: &PieceCatalog) -> Slab {
    catalog
        .panels()
        .iter()
        .min_by(|a, b| a.model.cmp(&b.model))
        .unwrap()
        .clone()
}

fn bridges(net: &PlacedNetwork) -> Vec<&PlacedRoad> {
    net.roads
        .iter()
        .filter(|r| r.kind == PlacedKind::Bridge)
        .collect()
}

fn wrap(a: f64) -> f64 {
    (a + PI).rem_euclid(TAU) - PI
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn a_nine_metre_bridge_on_gravel_is_three_panels_three_metres_apart() {
    let (net, fits, catalog) = fitted(&bridged(Family::Grav, 9.0), &flat);
    let slab = slab(&catalog);
    let (n, k, s) = panel_grid(9.0, 6.0, &slab);
    assert_eq!((n, k), (3, 1));
    assert!(close(s, 1.0, 1e-9));
    assert!(
        matches!(fits[0].kind, BridgeKind::Panels { .. }),
        "{fits:?}"
    );
    let panels: Vec<_> = road_objects(&net)
        .into_iter()
        .filter(|o| o.model == slab.model)
        .collect();
    assert_eq!(panels.len(), 3, "{panels:?}");
    let mut xs: Vec<f64> = panels.iter().map(|o| o.x).collect();
    xs.sort_by(f64::total_cmp);
    for (i, x) in xs.iter().enumerate() {
        assert!(close(*x, 401.5 + 3.0 * i as f64, 1e-6), "{xs:?}");
    }
    for o in &panels {
        // the line runs east (heading 90 degrees); the panel's long side lies across the road
        assert!(close(wrap(o.yaw - PI), 0.0, 1e-6), "{}", o.yaw);
        assert!(close(o.y, 500.0, 1e-6) && close(o.scale, 1.0, 1e-9));
    }
    // no road piece lies under the panels
    for r in &net.roads {
        if r.kind == PlacedKind::Piece {
            let x = f64::midpoint(r.start.x, r.end.x);
            assert!(!(400.5..408.5).contains(&x), "{r:?}");
        }
    }
    // the stretch is a real bridge: its roads are bridge roads covering exactly the grid
    let b = bridges(&net);
    assert_eq!(b.len(), 3);
    assert!(close(b[0].start.x, 400.0, 1e-6) && close(b[2].end.x, 409.0, 1e-6));
    for pair in b.windows(2) {
        assert!(close(pair[0].end.x, pair[1].start.x, 1e-6));
    }
}

#[test]
fn a_grid_two_abreast_lies_either_side_of_the_line() {
    let (net, fits, catalog) = fitted(&bridged(Family::Asf1, 8.85), &flat);
    let slab = slab(&catalog);
    let (n, k, s) = panel_grid(8.85, 12.0, &slab);
    assert_eq!((n, k), (3, 2));
    match fits[0].kind {
        BridgeKind::Panels {
            count,
            abreast,
            scale,
        } => {
            assert_eq!((count, abreast), (n, k));
            assert!(close(scale, s, 1e-6));
        }
        _ => panic!("{fits:?}"),
    }
    let panels: Vec<_> = road_objects(&net)
        .into_iter()
        .filter(|o| o.model == slab.model)
        .collect();
    assert_eq!(panels.len(), n * k);
    let step = slab.along * s;
    let mut xs: Vec<f64> = panels.iter().map(|o| o.x).collect();
    xs.sort_by(f64::total_cmp);
    xs.dedup_by(|a, b| close(*a, *b, 1e-6));
    assert_eq!(xs.len(), n);
    for (i, x) in xs.iter().enumerate() {
        let expect = 400.0 + step * (i as f64 + 0.5);
        assert!(close(*x, expect, 1e-6), "{xs:?}");
    }
    let half = slab.across * s / 2.0;
    for o in &panels {
        assert!(close(o.scale, s, 1e-9));
        assert!(
            close((o.y - 500.0).abs(), half, 1e-6),
            "{} is not {half} from the line",
            o.y
        );
    }
    // every road of the grid is one step wide as a grid
    for r in bridges(&net) {
        assert_eq!(r.abreast, 2);
        assert!(close(r.width, 2.0 * slab.across * s, 1e-6));
    }
}

#[test]
fn a_panel_grid_follows_a_bend_on_the_headings_of_the_line() {
    // a bridge of two segments: 6 m east, then 6 m north-east
    let features = vec![
        feature(
            1,
            Family::Grav,
            false,
            &[1, 2],
            &[(100.0, 500.0), (400.0, 500.0)],
        ),
        feature(
            2,
            Family::Grav,
            true,
            &[2, 5, 3],
            &[(400.0, 500.0), (406.0, 500.0), (410.0, 504.0)],
        ),
        feature(
            3,
            Family::Grav,
            false,
            &[3, 4],
            &[(410.0, 504.0), (700.0, 504.0)],
        ),
    ];
    let (net, _, catalog) = fitted(&features, &flat);
    let slab = slab(&catalog);
    let panels: Vec<_> = road_objects(&net)
        .into_iter()
        .filter(|o| o.model == slab.model)
        .collect();
    assert!(panels.len() >= 3, "{panels:?}");
    let yaws: Vec<f64> = panels.iter().map(|o| o.yaw).collect();
    assert!(
        yaws.iter().any(|y| close(wrap(y - PI), 0.0, 1e-3)),
        "{yaws:?}"
    );
    assert!(
        yaws.iter()
            .any(|y| close(wrap(y - (FRAC_PI_2 + PI / 4.0)), 0.0, 1e-3)),
        "{yaws:?}"
    );
}

#[test]
fn no_object_of_a_bridge_is_scaled_unless_it_is_a_panel() {
    for (family, span) in [
        (Family::Grav, 9.0),
        (Family::Asf1, 8.85),
        (Family::Asf1, 60.0),
        (Family::Grav, 22.0),
        (Family::Asf3, 14.1),
    ] {
        let (net, _, catalog) = fitted(&bridged(family, span), &flat);
        let panel = slab(&catalog).model;
        for r in bridges(&net) {
            assert!(
                r.model == panel || close(r.scale, 1.0, 1e-12),
                "{family:?} {span}: {r:?}"
            );
        }
        for o in road_objects(&net) {
            assert!(
                o.model == panel || close(o.scale, 1.0, 1e-12),
                "{family:?} {span}: {o:?}"
            );
        }
    }
}

#[test]
fn a_model_shorter_than_its_bridge_is_centred_on_it() {
    // a 25 m bridge over a valley: the posts of bridge_wood_25 do not reach it, so bridge_wood2 (19.7 m)
    let valley = |p: (f64, f64)| {
        if (405.0..420.0).contains(&p.0) {
            -4.0
        } else {
            0.0
        }
    };
    for reversed in [false, true] {
        let mut features = bridged(Family::Grav, 25.0);
        if reversed {
            // stored from the east end, walked from the west
            features[0].nodes = vec![8, 9];
            features[1].nodes = vec![9, 1];
            features[2].nodes = vec![1, 7];
        }
        let (net, fits, _) = fitted(&features, &valley);
        match &fits[0].kind {
            BridgeKind::Model { model, count } => {
                assert!(model.ends_with("bridge_wood2.p3d"), "{model}");
                assert_eq!(*count, 1);
            }
            other => panic!("{other:?}"),
        }
        assert!(fits[0].grown.0 < 0.0 && fits[0].grown.1 < 0.0);
        let b = bridges(&net);
        assert_eq!(b.len(), 1);
        let (lo, hi) = (b[0].start.x.min(b[0].end.x), b[0].start.x.max(b[0].end.x));
        assert!(
            close(lo, 402.65, 1e-3) && close(hi, 422.35, 1e-3),
            "{lo} {hi}"
        );
        assert!(close(b[0].scale, 1.0, 1e-12));
    }
}

#[test]
fn bridge_ways_that_follow_each_other_are_one_bridge_and_abut() {
    // two bridge ways that follow each other are one bridge (a fork is in the next test)
    let features = vec![
        feature(
            1,
            Family::Asf1,
            false,
            &[1, 2],
            &[(100.0, 500.0), (400.0, 500.0)],
        ),
        feature(
            2,
            Family::Asf1,
            true,
            &[2, 3],
            &[(400.0, 500.0), (417.0, 500.0)],
        ),
        feature(
            3,
            Family::Asf1,
            true,
            &[3, 4],
            &[(417.0, 500.0), (434.0, 500.0)],
        ),
        feature(
            4,
            Family::Asf1,
            false,
            &[4, 5],
            &[(434.0, 500.0), (800.0, 500.0)],
        ),
    ];
    let (net, fits, _) = fitted(&features, &flat);
    assert_eq!(fits.len(), 1, "joined: {fits:?}");
    let b = bridges(&net);
    assert_ne!(b.len(), 0);
    for pair in b.windows(2) {
        assert!(close(pair[0].end.x, pair[1].start.x, 0.01), "{pair:?}");
    }
}

#[test]
fn a_model_lies_from_the_start_of_its_stretch_whichever_way_it_is_walked() {
    use naves_core::roadplace::bridge_for;
    // the bridge's stored start is the lower node: at the east end where the node ids run the other way
    for nodes in [[2, 3], [3, 2], [9, 1]] {
        let catalog = synthetic_catalog();
        let mut features = bridged(Family::Grav, 22.0);
        if nodes == [9, 1] {
            // the road is walked from the west, the bridge is stored from the east
            features[0].nodes = vec![8, 9];
            features[1].nodes = vec![9, 1];
            features[2].nodes = vec![1, 7];
        }
        if nodes == [3, 2] {
            // the same road drawn the other way round
            for f in &mut features {
                f.points.reverse();
                f.nodes.reverse();
            }
        }
        let (graph, fits) = fit_bridges(
            &build_graph(&features),
            &|site| choose(&catalog, site),
            &flat,
        );
        let net = place_network(&graph, &catalog).unwrap();
        let stretch = graph
            .stretches
            .iter()
            .find(|s| s.id == fits[0].stretch)
            .unwrap();
        let first = *stretch.points.first().unwrap();
        let last = *stretch.points.last().unwrap();
        let laid = bridges(&net);
        assert_eq!(laid.len(), 1);
        let model = bridge_for(&catalog, Family::Grav)
            .into_iter()
            .find(|m| m.model == laid[0].model)
            .unwrap();
        // the world position of the model's first port
        let (sin, cos) = laid[0].origin.heading.sin_cos();
        let (east, north) = (model.start.x, model.start.z);
        let at = (
            laid[0].origin.x + east * cos + north * sin,
            laid[0].origin.y - east * sin + north * cos,
        );
        let away = |p: (f64, f64)| (at.0 - p.0).hypot(at.1 - p.1);
        assert!(
            away(first) < away(last),
            "{nodes:?}: {at:?} {first:?} {last:?}"
        );
    }
}

#[test]
fn a_panel_grid_is_one_level_deck_that_covers_exactly_the_grid() {
    use naves_core::heightmap::Heightmap;
    use naves_core::junctions::find_junctions;
    use naves_core::profile::solve_clearing;
    use naves_core::sideslope::side_slopes;

    let features = bridged(Family::Grav, 9.0);
    let (net, _, _) = fitted(&features, &flat);
    let catalog = synthetic_catalog();
    let (graph, _) = fit_bridges(
        &build_graph(&features),
        &|site| choose(&catalog, site),
        &flat,
    );
    // a gentle slope up to the east, 2 m per 100 m
    let (size, cell) = (101_u32, 10.0_f32);
    let mut h = Vec::new();
    for _row in 0..size {
        for col in 0..size {
            h.push(f64::from(col) as f32 * cell * 0.02);
        }
    }
    let hm = Heightmap {
        size,
        cell_m: cell,
        h,
    };
    let junctions = find_junctions(&net, &graph, &hm);
    let thickness =
        |s: &naves_core::roadnet::Stretch| naves_core::bridgekind::deck_thickness(&catalog, s);
    let (profiles, _) = solve_clearing(
        &net,
        &hm,
        &junctions,
        &side_slopes(&net, &hm),
        &graph,
        &thickness,
    )
    .unwrap();
    let mut deck = Vec::new();
    for strip in &profiles.strips {
        for (i, flag) in strip.bridge.iter().enumerate() {
            if *flag {
                deck.push((strip.joints[i], strip.joints[i + 1]));
            }
        }
    }
    assert_eq!(deck.len(), 3, "{deck:?}");
    assert!(close(deck[0].0.0, 400.0, 1e-6) && close(deck[2].1.0, 409.0, 1e-6));
    // one level deck
    let level = deck[0].0.2;
    assert!(
        deck.iter()
            .all(|d| close(d.0.2, level, 1e-6) && close(d.1.2, level, 1e-6))
    );
}

#[test]
fn a_model_keeps_to_the_end_where_another_bridge_meets_it() {
    // a 25 m bridge over a valley gets the 19.7 m deck; at the node where two short bridges meet it, the deck
    // reaches the node and the gap is at the other end
    let valley = |p: (f64, f64)| {
        if (405.0..420.0).contains(&p.0) {
            -4.0
        } else {
            0.0
        }
    };
    // (the fork node's id: the stored end of the long bridge when it is the higher one, else its start)
    for fork in [3_i64, 1] {
        let (long, a, b) = if fork == 3 {
            ([2, 3], [3, 4], [3, 5])
        } else {
            ([1, 2], [1, 4], [1, 5])
        };
        let (long_points, a_points, b_points);
        if fork == 3 {
            long_points = [(400.0, 500.0), (425.0, 500.0)];
            a_points = [(425.0, 500.0), (428.0, 500.0)];
            b_points = [(425.0, 500.0), (428.0, 503.0)];
        } else {
            // the fork at the west end
            long_points = [(400.0, 500.0), (425.0, 500.0)];
            a_points = [(400.0, 500.0), (397.0, 500.0)];
            b_points = [(400.0, 500.0), (397.0, 503.0)];
        }
        // the road that gives the long bridge room at its other end
        let (rest, rest_points) = if fork == 3 {
            ([2, 10], [(400.0, 500.0), (100.0, 500.0)])
        } else {
            ([2, 10], [(425.0, 500.0), (700.0, 500.0)])
        };
        let features = vec![
            feature(1, Family::Grav, true, &long, &long_points),
            feature(5, Family::Grav, false, &rest, &rest_points),
            feature(2, Family::Grav, true, &a, &a_points),
            feature(3, Family::Grav, true, &b, &b_points),
        ];
        let (net, fits, _) = fitted(&features, &valley);
        let long_fit = fits.iter().find(|f| f.stretch.starts_with("1:")).unwrap();
        assert!(
            matches!(&long_fit.kind, BridgeKind::Model { model, .. } if model.ends_with("bridge_wood2.p3d")),
            "{fork}: {long_fit:?}"
        );
        // the fit, the report and the placement say the same: the deck lies 5.3 m from the bridge's start where
        // the fork is at its end, and at its start where the fork is
        let expected = if fork == 3 { 5.3 } else { 0.0 };
        assert!(
            close(long_fit.part_start, expected, 1e-6),
            "{fork}: {long_fit:?}"
        );
        assert!(
            close(long_fit.grown.0, -expected, 1e-6),
            "{fork}: {long_fit:?}"
        );
        let wood: Vec<&PlacedRoad> = bridges(&net)
            .into_iter()
            .filter(|r| r.model.contains("wood2"))
            .collect();
        assert_eq!(wood.len(), 1, "{fork}");
        let (lo, hi) = (
            wood[0].start.x.min(wood[0].end.x),
            wood[0].start.x.max(wood[0].end.x),
        );
        if fork == 3 {
            assert!(
                close(hi, 425.0, 0.05) && close(lo, 405.3, 0.05),
                "{lo} {hi}"
            );
        } else {
            assert!(
                close(lo, 400.0, 0.05) && close(hi, 419.7, 0.05),
                "{lo} {hi}"
            );
        }
    }
}

/// The pieces of the chain right before and right after the bridge roads of `net`.
fn neighbours(net: &PlacedNetwork) -> (&PlacedRoad, &PlacedRoad, &PlacedRoad, &PlacedRoad) {
    let first = net
        .roads
        .iter()
        .position(|r| r.kind == PlacedKind::Bridge)
        .unwrap();
    let last = net
        .roads
        .iter()
        .rposition(|r| r.kind == PlacedKind::Bridge)
        .unwrap();
    (
        net.roads.get(first - 1).unwrap(),
        net.roads.get(first).unwrap(),
        net.roads.get(last).unwrap(),
        net.roads.get(last + 1).unwrap(),
    )
}

/// A road east along y = 500, a 9 m gravel bridge leaving it at `deg` degrees, and the road going on at that
/// heading: the bridge's end points, in map metres.
fn turned_bridge(deg: f64) -> (Vec<Feature>, [(f64, f64); 2]) {
    let (dx, dy) = (deg.to_radians().cos(), deg.to_radians().sin());
    let bridge_end = (400.0 + 9.0 * dx, 500.0 + 9.0 * dy);
    let features = vec![
        feature(
            1,
            Family::Grav,
            false,
            &[1, 2],
            &[(100.0, 500.0), (400.0, 500.0)],
        ),
        feature(
            2,
            Family::Grav,
            true,
            &[2, 3],
            &[(400.0, 500.0), bridge_end],
        ),
        feature(
            3,
            Family::Grav,
            false,
            &[3, 4],
            &[bridge_end, (400.0 + 300.0 * dx, 500.0 + 300.0 * dy)],
        ),
    ];
    (features, [(400.0, 500.0), bridge_end])
}

fn gap_between(a: &naves_core::roadfit::Pose, b: &naves_core::roadfit::Pose) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn kink_between(a: &naves_core::roadfit::Pose, b: &naves_core::roadfit::Pose) -> f64 {
    wrap(a.heading - b.heading).abs().to_degrees()
}

#[test]
fn a_grid_joins_the_road_before_and_after_it_whatever_the_turn_at_its_ends() {
    use naves_core::roadfit::{GAP_CLOSED_M, KINK_TOLERATED_DEG};
    for deg in [10.0, 30.0, 66.0] {
        let (features, _) = turned_bridge(deg);
        let (net, _, catalog) = fitted(&features, &flat);
        let panel = slab(&catalog).model;
        let (before, first, last, after) = neighbours(&net);
        assert_eq!(first.model, panel);
        for (a, b, what) in [
            (before.end, first.start, "before"),
            (last.end, after.start, "after"),
        ] {
            assert!(
                gap_between(&a, &b) <= GAP_CLOSED_M,
                "{deg} degrees, {what}: gap {}",
                gap_between(&a, &b)
            );
            assert!(
                kink_between(&a, &b) <= KINK_TOLERATED_DEG + 1e-3,
                "{deg} degrees, {what}: kink {}",
                kink_between(&a, &b)
            );
        }
        assert!(net.gaps.is_empty(), "{deg} degrees: {:?}", net.gaps);
    }
}

/// An 8 m road, then a bridge leaving it at 30 degrees, then the road going on at that heading: the road before
/// the bridge has no room for a lead.
fn bridge_after_a_stub() -> (Vec<Feature>, (f64, f64)) {
    let (dx, dy) = (30.0_f64.to_radians().cos(), 30.0_f64.to_radians().sin());
    let end = (400.0 + 9.0 * dx, 500.0 + 9.0 * dy);
    let features = vec![
        feature(
            1,
            Family::Grav,
            false,
            &[1, 2],
            &[(392.0, 500.0), (400.0, 500.0)],
        ),
        feature(2, Family::Grav, true, &[2, 3], &[(400.0, 500.0), end]),
        feature(
            3,
            Family::Grav,
            false,
            &[3, 4],
            &[end, (400.0 + 300.0 * dx, 500.0 + 300.0 * dy)],
        ),
    ];
    (features, end)
}

#[test]
fn a_neighbour_with_no_room_for_a_lead_is_left_as_it_was_and_still_reaches_the_grid() {
    use naves_core::roadfit::GAP_CLOSED_M;
    let (features, _) = bridge_after_a_stub();
    let catalog = synthetic_catalog();
    let built = build_graph(&features);
    let (graph, _) = fit_bridges(&built, &|site| choose(&catalog, site), &flat);
    let find = |g: &naves_core::roadnet::RoadGraph| {
        g.stretches
            .iter()
            .find(|s| s.id.starts_with("1:"))
            .unwrap()
            .points
            .clone()
    };
    assert_eq!(find(&graph), find(&built), "nothing is moved");
    let net = place_network(&graph, &catalog).unwrap();
    // it arrives where the grid starts, on its own heading
    let (before, first, _, _) = neighbours(&net);
    assert!(
        gap_between(&before.end, &first.start) <= GAP_CLOSED_M,
        "{before:?}"
    );
    // the turn that is left is a kink finding at the bridge, not a gap
    assert!(net.gaps.is_empty(), "{:?}", net.gaps);
    assert!(
        net.kinks
            .iter()
            .any(|k| (k.at.0 - 400.0).hypot(k.at.1 - 500.0) < 1.0 && k.heading_error_deg > 5.0),
        "{:?}",
        net.kinks
    );
}

#[test]
fn a_heading_miss_alone_at_a_bridge_is_a_kink_and_no_gap() {
    // two grids meet at a fork of three bridges at 150 degrees: the pose after the first is not on the second
    let arm = |deg: f64| {
        (
            400.0 + 9.0 * deg.to_radians().cos(),
            500.0 + 9.0 * deg.to_radians().sin(),
        )
    };
    let features = vec![
        feature(1, Family::Grav, true, &[1, 2], &[(400.0, 500.0), arm(0.0)]),
        feature(
            2,
            Family::Grav,
            true,
            &[1, 3],
            &[(400.0, 500.0), arm(150.0)],
        ),
        feature(
            3,
            Family::Grav,
            true,
            &[1, 4],
            &[(400.0, 500.0), arm(270.0)],
        ),
    ];
    let (net, _, _) = fitted(&features, &flat);
    assert!(
        net.kinks.iter().any(|k| k.heading_error_deg > 5.0),
        "a heading-only miss is a finding: {:?}",
        net.kinks
    );
    // a heading miss is not a position gap: `improves` and `road_gap` count positions only
    assert!(net.gaps.iter().all(|g| g.gap_m > 0.5), "{:?}", net.gaps);
}

#[test]
fn a_junction_port_away_from_a_bridge_reports_no_kink() {
    // the same road network without a bridge: a fit to a junction port that ends turned is not a finding
    let features = vec![
        feature(
            1,
            Family::Asf3,
            false,
            &[1, 2, 3],
            &[(100.0, 500.0), (500.0, 500.0), (900.0, 500.0)],
        ),
        feature(
            2,
            Family::Asf3,
            false,
            &[2, 4],
            &[(500.0, 500.0), (560.0, 900.0)],
        ),
        feature(
            3,
            Family::Asf3,
            false,
            &[3, 5],
            &[(900.0, 500.0), (905.0, 900.0)],
        ),
    ];
    let net = place_network(&build_graph(&features), &synthetic_catalog()).unwrap();
    assert!(net.kinks.is_empty(), "{:?}", net.kinks);
}

#[test]
fn the_steps_of_a_bent_grid_run_from_chord_end_to_chord_end() {
    // a bridge of two segments: 9 m east, then 9 m north-east
    let features = vec![
        feature(
            1,
            Family::Grav,
            false,
            &[1, 2],
            &[(100.0, 500.0), (400.0, 500.0)],
        ),
        feature(
            2,
            Family::Grav,
            true,
            &[2, 5, 3],
            &[(400.0, 500.0), (409.0, 500.0), (415.36, 506.36)],
        ),
        feature(
            3,
            Family::Grav,
            false,
            &[3, 4],
            &[(415.36, 506.36), (700.0, 506.36 + 284.64)],
        ),
    ];
    let (net, _, _) = fitted(&features, &flat);
    let steps: Vec<&PlacedRoad> = bridges(&net);
    assert!(steps.len() >= 4, "{}", steps.len());
    for step in &steps {
        let chord = (step.end.x - step.start.x).atan2(step.end.y - step.start.y);
        assert!(
            close(wrap(chord - step.start.heading), 0.0, 1e-9),
            "{step:?}"
        );
        assert!(close(wrap(chord - step.end.heading), 0.0, 1e-9), "{step:?}");
        let mid = (
            f64::midpoint(step.start.x, step.end.x),
            f64::midpoint(step.start.y, step.end.y),
        );
        assert!(close(step.origin.x, mid.0, 1e-9) && close(step.origin.y, mid.1, 1e-9));
    }
}

#[test]
fn the_lead_changes_the_neighbour_only_within_what_it_replaces() {
    use naves_core::bridgekind::Lead;
    let (features, _) = turned_bridge(30.0);
    let catalog = synthetic_catalog();
    let built = build_graph(&features);
    let (graph, fits) = fit_bridges(&built, &|site| choose(&catalog, site), &flat);
    let Lead::Laid {
        straight,
        replaced,
        turn_deg,
    } = fits[0].lead.0
    else {
        panic!("{:?}", fits[0].lead);
    };
    assert!(close(turn_deg, 30.0, 1e-6) && straight >= 4.0 && replaced >= straight + 4.0);
    let before = |g: &naves_core::roadnet::RoadGraph| {
        g.stretches
            .iter()
            .find(|s| s.id.starts_with("1:"))
            .unwrap()
            .points
            .clone()
    };
    let (old, new) = (before(&built), before(&graph));
    // the node is where it was, the point before it is on the deck's axis, and the road beyond `replaced`
    // metres from the node is the road it was
    assert_eq!(old.last(), new.last());
    let node = *new.last().unwrap();
    let axis = new[new.len() - 2];
    // the deck leaves the node at 30 degrees, so its axis extended backwards points south-west
    let (sin, cos) = 30.0_f64.to_radians().sin_cos();
    assert!(
        close(axis.0, node.0 - straight * cos, 1e-6)
            && close(axis.1, node.1 - straight * sin, 1e-6)
    );
    assert_eq!(old.first(), new.first());
    assert!(
        new.len() == 3
            || new
                .iter()
                .rev()
                .skip(2)
                .all(|p| old.contains(p) || p.0 <= node.0 - replaced + 1e-6)
    );
}

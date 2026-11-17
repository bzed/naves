// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A bridge over a road is built and placed so that the road passes through its opening, on synthetic models
//! (see `naves_core::catalog::testing`); no game data is used.

#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::cast_precision_loss,
    reason = "test helper code; failing loudly is the point"
)]

use std::collections::BTreeMap;

use naves_core::bridgefit::{
    Chain, Fit, Need, PASS_CLEARANCE_M, PASS_MARGIN_M, SIN_MIN, below_interval, fit_over,
};
use naves_core::bridgekind::{
    BridgeFit, BridgeKind, Fallback, GROW_TURN_MAX_DEG, choose, crossing_thickness, fit_bridges,
};
use naves_core::catalog::testing::{Block, bridge_of_blocks, synthetic_catalog, synthetic_models};
use naves_core::catalog::{Family, PartShape, PieceCatalog, SAKHAL_BRIDGES_PREFIX};
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::profile::BRIDGE_CLEARANCE_M;
use naves_core::roadnet::{BridgeOverRoad, RoadGraph, build_graph, over_crossings};

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn flat(_: (f64, f64)) -> f64 {
    0.0
}

fn feature(
    way: i64,
    family: Family,
    bridge: bool,
    layer: i8,
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
            layer,
        },
        points: points.to_vec(),
        nodes: nodes.to_vec(),
    }
}

/// A road along y = 500 with a bridge of `span` metres from x = 400 and 300 m of the same road on each side, and
/// a road of `below` crossing under the bridge at `station` metres from its start, `angle` radians from the
/// bridge's line (a right angle is square), long enough to run past it both ways.
fn over_road(family: Family, span: f64, below: Family, station: f64, angle: f64) -> Vec<Feature> {
    let at = (400.0 + station, 500.0);
    let (dx, dy) = (angle.cos(), angle.sin());
    // the crossing road's line reaches 300 m both ways from the crossing
    let a = (at.0 - 300.0 * dx, at.1 - 300.0 * dy);
    let b = (at.0 + 300.0 * dx, at.1 + 300.0 * dy);
    vec![
        feature(
            1,
            family,
            false,
            0,
            &[1, 2],
            &[(100.0, 500.0), (400.0, 500.0)],
        ),
        feature(
            2,
            family,
            true,
            1,
            &[2, 3],
            &[(400.0, 500.0), (400.0 + span, 500.0)],
        ),
        feature(
            3,
            family,
            false,
            0,
            &[3, 4],
            &[(400.0 + span, 500.0), (400.0 + span + 300.0, 500.0)],
        ),
        feature(4, below, false, 0, &[5, 6], &[a, b]),
    ]
}

/// The same, with `near` metres of road before and after the bridge, each ending at a junction (a crossing road
/// 50 m to either side), so that the road beside the bridge is only that long.
fn over_road_near(
    family: Family,
    span: f64,
    below: Family,
    station: f64,
    angle: f64,
    near: (f64, f64),
) -> Vec<Feature> {
    let mut features = over_road(family, span, below, station, angle);
    let (start, end) = (400.0 - near.0, 400.0 + span + near.1);
    features[0] = feature(
        1,
        family,
        false,
        0,
        &[1, 2],
        &[(start, 500.0), (400.0, 500.0)],
    );
    features[2] = feature(
        3,
        family,
        false,
        0,
        &[3, 4],
        &[(400.0 + span, 500.0), (end, 500.0)],
    );
    for (way, node, x) in [(10, 1, start), (11, 4, end)] {
        features.push(feature(
            way,
            family,
            false,
            0,
            &[node, way],
            &[(x, 500.0), (x, 550.0)],
        ));
        features.push(feature(
            way + 10,
            family,
            false,
            0,
            &[node, way + 10],
            &[(x, 500.0), (x, 450.0)],
        ));
    }
    features
}

/// The fit of the bridge among `features` with the synthetic catalog (or `catalog`).
fn fits(features: &[Feature], catalog: &PieceCatalog) -> (RoadGraph, Vec<BridgeFit>) {
    fit_bridges(&build_graph(features), &|site| choose(catalog, site), &flat)
}

/// The catalog with a model of the Sakhal folder replaced.
fn catalog_with_sakhal(name: &str, model: naves_formats::odol::OdolModel) -> PieceCatalog {
    let mut models: BTreeMap<String, naves_formats::odol::OdolModel> = synthetic_models();
    models.insert(format!("{SAKHAL_BRIDGES_PREFIX}{name}.p3d"), model);
    PieceCatalog::from_models(&models).unwrap()
}

fn model_name(kind: &BridgeKind) -> String {
    match kind {
        BridgeKind::Model { model, .. } => model.rsplit('\\').next().unwrap().to_string(),
        other => format!("{other}"),
    }
}

#[test]
fn a_track_square_to_a_truss_slides_the_truss_within_its_room() {
    // an Asf1 bridge of 20 m takes the 25 m truss (blocks 3.4 m long at its ends, open between 3.4 and 21.6), which
    // reaches 5 m past the bridge: 2.5 m at each end where nothing is under it. A 6 m track crosses at 17 m:
    // its interval (3 m and a metre of margin each side, nothing for the skew) is 13..21 on the bridge, so
    // the truss must start at most 0.6 m before the bridge; 2.5 m before it, its far block would stand on the
    // track
    let catalog = synthetic_catalog();
    let features = over_road(
        Family::Asf1,
        20.0,
        Family::Grav,
        17.0,
        std::f64::consts::FRAC_PI_2,
    );
    let (_, fits) = fits(&features, &catalog);
    let fit = &fits[0];
    assert_eq!(model_name(&fit.kind), "bridge_metal_25.p3d", "{fit:?}");
    assert!(near(fit.part_start, -0.6, 1e-6), "{}", fit.part_start);
    assert!(near(fit.slid, 1.9, 1e-6), "{}", fit.slid);
    assert!(
        near(fit.grown.0, 0.6, 1e-6) && near(fit.grown.1, 4.4, 1e-6),
        "{:?}",
        fit.grown
    );
    assert_eq!(fit.below.len(), 1);
    assert!(fit.below[0].missing.abs() < 1e-9, "{:?}", fit.below);
    assert!(
        near(fit.below[0].interval.0, 13.0, 1e-6) && near(fit.below[0].interval.1, 21.0, 1e-6),
        "{:?}",
        fit.below
    );
    assert_eq!(fit.fallback.len(), 0);
}

#[test]
fn the_placement_lays_the_slid_part_where_the_fit_says() {
    // the one place a part lies is the fit's: the placed truss starts 0.6 m before the bridge, not 2.5 m
    use naves_core::roadplace::{PlacedKind, place_network};
    let catalog = synthetic_catalog();
    let features = over_road(
        Family::Asf1,
        20.0,
        Family::Grav,
        17.0,
        std::f64::consts::FRAC_PI_2,
    );
    let (graph, fits) = fits(&features, &catalog);
    let net = place_network(&graph, &catalog).unwrap();
    let deck: Vec<_> = net
        .roads
        .iter()
        .filter(|r| r.kind == PlacedKind::Bridge)
        .collect();
    assert_eq!(deck.len(), 1);
    assert!(
        near(deck[0].start.x, 400.0 + fits[0].part_start, 1e-3),
        "{}",
        deck[0].start.x
    );
    assert!(
        near(deck[0].end.x, 400.0 + fits[0].part_start + 25.0, 1e-3),
        "{}",
        deck[0].end.x
    );
}

#[test]
fn a_road_that_passes_where_the_table_puts_the_part_leaves_it_there() {
    // the same bridge, the track at 8 m: 4..12 is in the opening of the truss as the table lays it
    let catalog = synthetic_catalog();
    let features = over_road(
        Family::Asf1,
        20.0,
        Family::Grav,
        8.0,
        std::f64::consts::FRAC_PI_2,
    );
    let (_, fits) = fits(&features, &catalog);
    assert!(fits[0].slid.abs() < 1e-9, "{}", fits[0].slid);
    assert!(near(fits[0].part_start, -2.5, 1e-6));
    assert!(fits[0].below[0].missing.abs() < 1e-9);
}

#[test]
fn a_bridge_without_a_road_below_is_laid_as_before() {
    let catalog = synthetic_catalog();
    let mut features = over_road(
        Family::Asf1,
        20.0,
        Family::Grav,
        17.0,
        std::f64::consts::FRAC_PI_2,
    );
    features.pop();
    let (_, fits) = fits(&features, &catalog);
    assert!(near(fits[0].part_start, -2.5, 1e-6));
    assert!(fits[0].below.is_empty() && fits[0].slid.abs() < 1e-9);
}

#[test]
fn a_wide_road_at_an_angle_that_the_truss_cannot_span_blocks_where_the_grid_cannot_grow_either() {
    // a 25 m truss on a 25 m bridge has no room to slide. A 10 m road crossing at 45 degrees needs, on each side of
    // the crossing: the skew over the width of the end blocks (8 m), the footprint (10 / sin 45 degrees / 2 =
    // 7.07 m) and a metre of margin: 16.07 m, 32 m in all, and the opening is 18.2 m. The panel grid is
    // lengthened as far as the road beside the bridge allows (3 m of road at each end, 2 m of it free), and
    // what is still missing is the finding
    let catalog = synthetic_catalog();
    let features = over_road_near(
        Family::Asf1,
        25.0,
        Family::City,
        12.5,
        std::f64::consts::FRAC_PI_4,
        (3.0, 3.0),
    );
    let (_, fits) = fits(&features, &catalog);
    let fit = &fits[0];
    let below = &fit.below[0];
    assert!(below.missing > 1.0, "{fit:?}");
    assert!(near(fit.grown.0, GROW_KEEP_FREE, 1e-6) && near(fit.grown.1, GROW_KEEP_FREE, 1e-6));
    let BridgeKind::Panels {
        count,
        abreast,
        scale,
    } = fit.kind
    else {
        panic!("{fit:?}");
    };
    let slab = catalog
        .panels()
        .iter()
        .min_by(|a, b| a.model.cmp(&b.model))
        .unwrap();
    let shape = PartShape::panels(slab, count, abreast, scale);
    let reach = below_interval(10.0, std::f64::consts::FRAC_PI_4, &shape);
    let total = reach.total();
    assert!(near(shape.length, 25.0 + 2.0 * GROW_KEEP_FREE, 1e-6));
    assert!(
        near(below.interval.1 - below.interval.0, 2.0 * total, 1e-9),
        "{below:?}"
    );
    // only what the grown grid still lacks is missing
    assert!(
        near(below.missing, 2.0 * total - shape.length, 0.01),
        "{below:?} {total}"
    );
    assert!(
        fit.fallback
            .iter()
            .any(|f| matches!(f, Fallback::ModelBlocksRoad { .. })),
        "{fit:?}"
    );
    assert!(
        fit.fallback
            .iter()
            .any(|f| matches!(f, Fallback::GridLengthened { .. })),
        "{fit:?}"
    );
}

/// What a bridge may grow into a road 3 m long between it and a junction: the road less `GROW_KEEP_M`.
const GROW_KEEP_FREE: f64 = 2.0;

#[test]
fn a_road_that_a_truss_cannot_span_gets_a_panel_row_where_the_row_can() {
    // 60 m of bridge is three trusses end to end, each with its blocks at both ends; a 10 m road at 45 degrees
    // wants 32 m of opening and a truss has 18.2 m. The panel row of the bridge's own 60 m has it (the
    // crossing is in the middle), and nothing grows
    let catalog = synthetic_catalog();
    let features = over_road(
        Family::Asf1,
        60.0,
        Family::City,
        30.0,
        std::f64::consts::FRAC_PI_4,
    );
    let (graph, fits) = fits(&features, &catalog);
    let fit = &fits[0];
    assert!(matches!(fit.kind, BridgeKind::Panels { .. }), "{fit:?}");
    assert!(fit.below[0].missing.abs() < 1e-9, "{fit:?}");
    assert_eq!(fit.grown, (0.0, 0.0));
    assert!(
        fit.fallback
            .iter()
            .any(|f| matches!(f, Fallback::ModelBlocksRoad { .. })),
        "{fit:?}"
    );
    // the interval lies in the bridge's own grid, so the grid is not lengthened: the bridge is as long as
    // OpenStreetMap has it
    let bridge = graph.stretches.iter().find(|s| s.bridge).unwrap();
    let length: f64 = bridge
        .points
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum();
    assert!(near(length, 60.0, 1e-6), "{length}");
}

#[test]
fn a_road_nearly_along_the_bridge_gives_a_bounded_interval_and_blocks() {
    let catalog = synthetic_catalog();
    // 0.01 rad between the lines: 1 / sin is 100, not infinity
    let features = over_road(Family::Asf1, 20.0, Family::Grav, 10.0, 0.01);
    let (_, fits) = fits(&features, &catalog);
    let below = &fits[0].below[0];
    assert!(below.interval.0.is_finite() && below.interval.1.is_finite());
    assert!(below.interval.1 - below.interval.0 < 4000.0, "{below:?}");
    assert!(
        below.missing.is_finite() && below.missing > 1.0,
        "{below:?}"
    );
    // the geometry in isolation: the angle's sine is bounded below, whatever the angle
    let catalog_models = catalog.bridges();
    let shape = catalog_models[0].shape();
    for angle in [
        0.0,
        -0.0,
        1e-12,
        0.01,
        f64::NAN,
        f64::INFINITY,
        std::f64::consts::PI,
    ] {
        let reach = below_interval(6.0, angle, &shape);
        assert!(
            reach.footprint.is_finite() && reach.footprint > 0.0,
            "{angle}: {reach:?}"
        );
        assert!(
            reach.footprint
                <= 3.0
                    + PASS_MARGIN_M
                    + shape.deck_half_width.max(8.1) / SIN_MIN
                    + 6.0 / (2.0 * SIN_MIN)
                    + 1.0
        );
    }
    // a right angle has no skew; a quarter turn has as much skew as the part's half width
    let square = below_interval(6.0, std::f64::consts::FRAC_PI_2, &shape);
    assert!(near(square.centre, 0.0, 1e-9), "{square:?}");
    assert!(
        near(square.footprint, 3.0 + PASS_MARGIN_M, 1e-9),
        "{square:?}"
    );
    assert!(near(PASS_CLEARANCE_M, 3.0, 0.0) && near(BRIDGE_CLEARANCE_M, 4.5, 0.0));
}

#[test]
fn bridge_concrete3_is_never_laid_over_a_road_and_the_next_kind_takes_the_bridge() {
    // Kehlsteinstrasse near (7995, 7320): a bridge_concrete3 with its cross walls every 3 m. The real model has
    // walls 1.26 m thick with gaps of 1.74 m, so something stands in nearly every metre and no road passes
    let mut walls = vec![Block {
        a0: 0.0,
        a1: 21.1,
        c0: -3.8,
        c1: 3.8,
        top: 0.0,
        depth: 0.6,
    }];
    for k in 0..7 {
        let a0 = 0.5 + 3.0 * f64::from(k);
        walls.push(Block {
            a0,
            a1: a0 + 1.26,
            c0: -3.8,
            c1: 3.8,
            top: 0.6,
            depth: 4.8,
        });
    }
    let catalog = catalog_with_sakhal(
        "bridge_concrete3",
        bridge_of_blocks(21.1, 7.6, 2.16, false, &walls),
    );
    // an 18 m bridge on Asf3: the 21.1 m deck is the shortest model it fills to two thirds
    let mut features = over_road(
        Family::Asf3,
        18.0,
        Family::Mud,
        9.0,
        std::f64::consts::FRAC_PI_2,
    );
    let road = features.pop().unwrap();
    let (_, alone) = fits(&features, &catalog);
    assert_eq!(
        model_name(&alone[0].kind),
        "bridge_concrete3.p3d",
        "{:?}",
        alone[0]
    );
    features.push(road);
    let (_, over) = fits(&features, &catalog);
    let fit = &over[0];
    let BridgeKind::Panels { .. } = fit.kind else {
        panic!("{fit:?}");
    };
    assert!(fit.below[0].missing.abs() < 1e-9, "{fit:?}");
    assert!(
        fit.fallback
            .iter()
            .any(|f| matches!(f, Fallback::ModelBlocksRoad { .. })),
        "{fit:?}"
    );
}

#[test]
fn fit_over_finds_the_nearest_start_and_the_one_with_the_least_missing() {
    let catalog = synthetic_catalog();
    let metal = catalog
        .bridges()
        .iter()
        .find(|b| b.model.ends_with("bridge_metal_25.p3d"))
        .unwrap();
    let shape = metal.shape();
    let chain = Chain {
        shape: &shape,
        count: 1,
    };
    let need = |station: f64| Need {
        station,
        reach: below_interval(6.0, std::f64::consts::FRAC_PI_2, &shape),
    };
    // a road at 17 m (interval 13..21) on a bridge the truss starts at 0: the opening is 3.4..21.6 and holds it
    let fit = fit_over(&chain, (-5.0, 5.0), 0.0, &[need(17.0)], &|_| true);
    assert_eq!(fit, Fit::Fits { start: 0.0 });
    // at 19 m the interval is 15..23 and the opening ends at 21.6: the part moves on by 1.4 m
    let fit = fit_over(&chain, (-5.0, 5.0), 0.0, &[need(19.0)], &|_| true);
    let Fit::Fits { start } = fit else {
        panic!("{fit:?}")
    };
    assert!(near(start, 1.4, 1e-4), "{start}");
    // with no room to move, the missing metres are the part of the interval outside the opening
    let fit = fit_over(&chain, (0.0, 0.0), 0.0, &[need(19.0)], &|_| true);
    assert!(matches!(fit, Fit::Blocked { .. }));
    assert!(near(fit.missing(), 1.4, 1e-4), "{fit:?}");
    // a start that is not valid is not used, and the nearest valid one is
    let fit = fit_over(&chain, (-5.0, 5.0), 0.0, &[need(19.0)], &|s| s >= 2.0);
    assert!(near(fit.start(), 2.0, 1e-9), "{fit:?}");
    assert!(fit.missing() < 1e-9);
}

#[test]
fn the_deck_thickness_over_a_road_is_what_hangs_over_its_interval() {
    // a stone arch 25 m long, 3.2 m under the deck at its crown and about 4.3 m deep a little from its middle,
    // on an Asf2 bridge; a 5 m road square at its middle has 3.5 m each side: 9..16 on the part
    let catalog = synthetic_catalog();
    let features = over_road(
        Family::Asf2,
        25.0,
        Family::Mud,
        12.5,
        std::f64::consts::FRAC_PI_2,
    );
    let (graph, fits) = fits(&features, &catalog);
    assert_eq!(
        model_name(&fits[0].kind),
        "bridge_stone_25.p3d",
        "{:?}",
        fits[0]
    );
    let crossings = over_crossings(&graph);
    assert_eq!(crossings.len(), 1);
    let c = &crossings[0];
    let (upper, lower) = (&graph.stretches[c.upper], &graph.stretches[c.lower]);
    let t = crossing_thickness(
        &catalog,
        &BridgeOverRoad {
            upper,
            lower,
            crossing: c,
        },
    )
    .unwrap();
    let arch = catalog
        .bridges()
        .iter()
        .find(|b| b.model.ends_with("bridge_stone_25.p3d"))
        .unwrap();
    // 4.5 m under the deepest point over the centre line (the metre either side of the crossing) and 3 m over the
    // footprint (3.5 m either side): the larger of the first and the second less 1.5 m
    let centre = arch.thickness(12.5 - 1.0, 12.5 + 1.0);
    let footprint = arch.thickness(12.5 - 3.5, 12.5 + 3.5);
    let expected = centre.max(footprint - 1.5);
    assert!(near(t, expected, 1e-9), "{t} against {expected}");
    assert!(
        footprint - 1.5 < centre,
        "the centre line decides here: {centre} {footprint}"
    );
    // not below the crown, and not the abutments' depth, the deepest point of the part
    assert!(t >= 3.2 && t < arch.reach_down / 2.0, "{t}");
    // a road near the flank is deeper under the deck
    let flank = arch.thickness(9.0, 11.0);
    assert!(flank > t, "{flank} {t}");
}

#[test]
fn a_grown_model_reads_its_interval_from_where_the_part_starts() {
    // the 25 m truss on a 20 m bridge, slid so that it starts 0.6 m before the bridge: the stretch is grown by that
    // much, and a crossing 17 m from the bridge's start is 17.6 m along the stretch, with the part starting at its
    // first end; the thickness there is the deck's 1.7 m
    let catalog = synthetic_catalog();
    let features = over_road(
        Family::Asf1,
        20.0,
        Family::Grav,
        17.0,
        std::f64::consts::FRAC_PI_2,
    );
    let (graph, _) = fits(&features, &catalog);
    let c = over_crossings(&graph);
    assert_eq!(c.len(), 1);
    assert!(near(c[0].upper_s, 17.6, 1e-6), "{}", c[0].upper_s);
    let t = crossing_thickness(
        &catalog,
        &BridgeOverRoad {
            upper: &graph.stretches[c[0].upper],
            lower: &graph.stretches[c[0].lower],
            crossing: &c[0],
        },
    )
    .unwrap();
    assert!(near(t, 1.7, 1e-6), "{t}");
}

#[test]
fn a_model_is_slid_only_within_the_turn_limit() {
    // the neighbour beyond the bridge's end bends by 3 degrees after 2 m; the model may grow there, but sliding
    // it further would add the bend of the road beyond (a degree for every 1.5 degrees), and with the first 2.5 m
    // already used the turn is 3 degrees: another 4 m of growth there would go past 5
    let catalog = synthetic_catalog();
    let mut features = over_road(
        Family::Asf1,
        20.0,
        Family::Grav,
        17.0,
        std::f64::consts::FRAC_PI_2,
    );
    // 20 m after the bridge's end: 3 degrees at 22.5 m, i.e. 2.5 m after the first grown metres
    let bend = 3.0_f64.to_radians();
    let end = (420.0, 500.0);
    let turned = (end.0 + 22.0, 500.0);
    let after = (turned.0 + 300.0 * bend.cos(), 500.0 - 300.0 * bend.sin());
    features[2] = feature(3, Family::Asf1, false, 0, &[3, 4], &[end, turned, after]);
    let (_, fits) = fits(&features, &catalog);
    let fit = &fits[0];
    let turn = fit.own_turn_deg + fit.turn_deg.0 + fit.turn_deg.1;
    assert!(turn <= GROW_TURN_MAX_DEG + 1e-9, "{turn}");
}

#[test]
fn a_filled_bridge_and_a_bridge_without_parts_are_unchanged() {
    let catalog = synthetic_catalog();
    // 2 m of bridge with nothing below is plain road
    let features = over_road(
        Family::Asf1,
        2.0,
        Family::Grav,
        1.0,
        std::f64::consts::FRAC_PI_2,
    );
    let mut features = features;
    features.pop();
    let (_, fits) = fits(&features, &catalog);
    assert!(matches!(fits[0].kind, BridgeKind::Fill));
    assert_eq!(fits[0].below.len(), 0);
}

#[test]
fn a_chain_knows_its_length_and_what_hangs_only_where_it_lies() {
    let catalog = synthetic_catalog();
    let metal = catalog
        .bridges()
        .iter()
        .find(|b| b.model.ends_with("bridge_metal_25.p3d"))
        .unwrap();
    let shape = metal.shape();
    let chain = Chain {
        shape: &shape,
        count: 3,
    };
    assert!(near(chain.length(), 3.0 * shape.length, 1e-9));
    // nothing of the chain lies 100 m past its end or before its start
    assert_eq!(chain.thickness(100.0, 110.0), None);
    assert_eq!(chain.thickness(-20.0, -10.0), None);
    // the runs of the copies stay apart where a block stands between them
    let runs = chain.passable(10.0, 3.0);
    assert_eq!(runs.len(), 3, "{runs:?}");
    // a fit that is blocked says where the part lies and how much is missing
    let fit = Fit::Blocked {
        start: 1.0,
        missing: 2.0,
    };
    assert!(near(fit.start(), 1.0, 0.0) && near(fit.missing(), 2.0, 0.0));
}

/// The length of the bridge stretch of `graph` along its line.
fn bridge_length(graph: &RoadGraph) -> f64 {
    let bridge = graph.stretches.iter().find(|s| s.bridge).unwrap();
    bridge
        .points
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum()
}

#[test]
fn a_short_bridge_over_a_wide_road_gets_a_grid_that_covers_the_road() {
    // 4 m of bridge over a 10 m road with long neighbours, square and at 45 degrees: the grid grows at each end by
    // what that end lacks, covers exactly its new length, and nothing is missing
    let catalog = synthetic_catalog();
    for angle in [std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_4] {
        let features = over_road(Family::Asf1, 4.0, Family::City, 2.0, angle);
        let (graph, fits) = fits(&features, &catalog);
        let fit = &fits[0];
        let BridgeKind::Panels { count, scale, .. } = fit.kind else {
            panic!("{fit:?}");
        };
        let below = &fit.below[0];
        assert!(below.missing.abs() < 1e-6, "{angle}: {fit:?}");
        assert!(fit.grown.0 > 1.0 && fit.grown.1 > 1.0, "{angle}: {fit:?}");
        // the road is in the middle: both ends lack the same
        assert!(near(fit.grown.0, fit.grown.1, 1e-6), "{:?}", fit.grown);
        let length = bridge_length(&graph);
        assert!(
            near(length, 4.0 + fit.grown.0 + fit.grown.1, 1e-6),
            "{length}"
        );
        assert!(near(fit.part_length, length, 1e-6));
        // the panels cover the grown bridge exactly
        let slab = catalog
            .panels()
            .iter()
            .min_by(|a, b| a.model.cmp(&b.model))
            .unwrap();
        assert!(near(slab.along * scale * count as f64, length, 1e-6));
        assert!(near(fit.part_start, -fit.grown.0, 1e-9));
        assert!(
            fit.fallback
                .iter()
                .any(|f| matches!(f, Fallback::GridLengthened { .. })),
            "{fit:?}"
        );
        // the interval lies in the grown grid
        assert!(below.interval.0 + fit.grown.0 > -1e-6);
        assert!(below.interval.1 + fit.grown.0 < length + 1e-6);
    }
}

#[test]
fn a_grid_stops_a_metre_before_a_junction_and_only_what_is_left_is_missing() {
    // the road after the bridge is 6 m long and ends at a junction: the grid may take 5 m of it, which brings the
    // road's footprint under the deck; the cutting's walls still reach past the end, and that is the finding.
    // The other end has room and grows by what it lacks
    let catalog = synthetic_catalog();
    let features = over_road_near(
        Family::Asf1,
        4.0,
        Family::City,
        2.0,
        std::f64::consts::FRAC_PI_2,
        (300.0, 6.0),
    );
    let (_, fits) = fits(&features, &catalog);
    let fit = &fits[0];
    assert!(near(fit.grown.1, 5.0, 1e-6), "{:?}", fit.grown);
    assert!(fit.grown.0 > fit.grown.1 + 1.0, "{:?}", fit.grown);
    assert!(fit.below[0].missing > 0.1, "{fit:?}");
    assert!(fit.below[0].missing < 4.0, "{fit:?}");
    // with only 3 m of road after the bridge the footprint would still be outside the deck: that end does not
    // grow, and the whole of what it lacks is missing
    let features = over_road_near(
        Family::Asf1,
        4.0,
        Family::City,
        2.0,
        std::f64::consts::FRAC_PI_2,
        (300.0, 3.0),
    );
    let (_, fits) = fits_of(&features, &catalog);
    assert!(near(fits[0].grown.1, 0.0, 1e-9), "{:?}", fits[0].grown);
    assert!(fits[0].grown.0 > 1.0);
}

#[test]
fn a_grid_follows_the_bend_of_the_road_it_grows_into() {
    // 4 m of bridge east along y = 500; the road after it runs east for 2 m and turns 30 degrees to the north. A
    // 10 m road crosses at 2 m. The grid grows 8 m into that road, round the bend: its steps run chord to chord
    // along the line, so its end is on the line and its last step has turned
    use naves_core::roadplace::{PlacedKind, place_network};
    let catalog = synthetic_catalog();
    let mut features = over_road(
        Family::Asf1,
        4.0,
        Family::City,
        2.0,
        std::f64::consts::FRAC_PI_2,
    );
    let turn = 30.0_f64.to_radians();
    features[2] = feature(
        3,
        Family::Asf1,
        false,
        0,
        &[3, 30, 4],
        &[
            (404.0, 500.0),
            (406.0, 500.0),
            (406.0 + 300.0 * turn.cos(), 500.0 + 300.0 * turn.sin()),
        ],
    );
    let (graph, fits) = fits(&features, &catalog);
    let fit = &fits[0];
    assert!(fit.grown.1 > 3.0, "{fit:?}");
    assert!(fit.below[0].missing.abs() < 1e-6, "{fit:?}");
    let bridge = graph.stretches.iter().find(|s| s.bridge).unwrap();
    assert!(bridge.points.len() >= 3, "{:?}", bridge.points);
    let net = place_network(&graph, &catalog).unwrap();
    let deck: Vec<_> = net
        .roads
        .iter()
        .filter(|r| r.kind == PlacedKind::Bridge)
        .collect();
    let (first, last) = (deck[0], deck[deck.len() - 1]);
    let (a, b) = (bridge.points[0], bridge.points[bridge.points.len() - 1]);
    assert!(near(first.start.x, a.0, 1e-6) && near(first.start.y, a.1, 1e-6));
    assert!(near(last.end.x, b.0, 1e-6) && near(last.end.y, b.1, 1e-6));
    let turned = (last.end.heading - first.start.heading).abs().to_degrees();
    assert!(turned > 5.0, "{turned}");
}

#[test]
fn a_grid_with_no_road_below_is_not_lengthened() {
    let catalog = synthetic_catalog();
    let mut features = over_road(
        Family::Asf1,
        4.0,
        Family::City,
        2.0,
        std::f64::consts::FRAC_PI_2,
    );
    features.pop();
    let (graph, fits) = fits(&features, &catalog);
    assert_eq!(fits[0].grown, (0.0, 0.0));
    assert!(near(bridge_length(&graph), 4.0, 1e-9));
    assert_eq!(fits[0].below.len(), 0);
}

/// The fit with `ground` as the original ground.
fn fits_on(
    features: &[Feature],
    catalog: &PieceCatalog,
    ground: &dyn Fn((f64, f64)) -> f64,
) -> Vec<BridgeFit> {
    fit_bridges(
        &build_graph(features),
        &|site| choose(catalog, site),
        ground,
    )
    .1
}

#[test]
fn a_road_that_turns_back_at_the_bridges_end_is_no_road_below_and_nothing_grows() {
    // 6 m of bridge, and at its end node the road turns back by 172 degrees along the bridge: the two meet at the
    // node, nothing crosses under the deck
    let catalog = synthetic_catalog();
    let features = vec![
        feature(
            1,
            Family::Asf1,
            false,
            0,
            &[1, 2],
            &[(100.0, 500.0), (400.0, 500.0)],
        ),
        feature(
            2,
            Family::Asf1,
            true,
            1,
            &[2, 3],
            &[(400.0, 500.0), (406.0, 500.0)],
        ),
        feature(
            3,
            Family::Asf1,
            false,
            0,
            &[3, 4],
            &[(406.0, 500.0), (307.0, 486.1)],
        ),
    ];
    let (graph, fits) = fits(&features, &catalog);
    assert_eq!(fits[0].below.len(), 0, "{:?}", fits[0]);
    assert_eq!(fits[0].grown, (0.0, 0.0));
    assert!(near(bridge_length(&graph), 6.0, 1e-6));
}

#[test]
fn a_grid_is_not_lengthened_for_a_road_nearly_along_it() {
    // 0.01 rad between the lines wants 600 m of deck: no grid helps, so none grows (it was 618 m long)
    let catalog = synthetic_catalog();
    let features = over_road(Family::Asf1, 4.0, Family::Grav, 2.0, 0.01);
    let (graph, fits) = fits(&features, &catalog);
    assert_eq!(fits[0].grown, (0.0, 0.0), "{:?}", fits[0]);
    assert!(near(bridge_length(&graph), 4.0, 1e-6));
    assert!(fits[0].below[0].missing > 100.0, "{:?}", fits[0].below);
}

#[test]
fn an_end_that_cannot_be_helped_does_not_grow_and_the_other_end_does() {
    // 4 m of bridge, a 10 m road at 0.7 rad 3.9 m from the start: the far end would need 18.6 m, more than 15 m
    // and with the footprint still outside the deck, so it does not grow; the start needs 14.8 m and does
    let catalog = synthetic_catalog();
    let features = over_road(Family::Asf1, 4.0, Family::City, 3.9, 0.7);
    let (_, fits) = fits(&features, &catalog);
    let fit = &fits[0];
    assert!(near(fit.grown.1, 0.0, 1e-9), "{:?}", fit.grown);
    assert!(near(fit.grown.0, -fit.below[0].interval.0, 1e-3), "{fit:?}");
    assert!(fit.below[0].missing > 10.0, "{:?}", fit.below);
}

#[test]
fn an_end_grows_by_exactly_what_it_lacks_and_at_most_fifteen_metres() {
    let catalog = synthetic_catalog();
    // square: what each end lacks is what the interval reaches past the bridge
    let features = over_road(
        Family::Asf1,
        4.0,
        Family::City,
        3.5,
        std::f64::consts::FRAC_PI_2,
    );
    let (_, fits) = fits(&features, &catalog);
    let (fit, below) = (&fits[0], &fits[0].below[0]);
    assert!(near(fit.grown.0, -below.interval.0, 1e-3), "{fit:?}");
    assert!(near(fit.grown.1, below.interval.1 - 4.0, 1e-3), "{fit:?}");
    assert!(below.missing.abs() < 1e-6);
    // at 0.8 rad the far end lacks more than 15 m: it grows by 15 m, the footprint is in, the walls are not
    let features = over_road(Family::Asf1, 4.0, Family::City, 3.9, 0.8);
    let (_, fits) = fits_of(&features, &catalog);
    assert!(near(fits[0].grown.1, 15.0, 1e-9), "{:?}", fits[0].grown);
    assert!(fits[0].below[0].missing > 0.1, "{:?}", fits[0].below);
}

#[test]
fn a_bridge_of_three_metres_or_less_over_a_road_is_a_panel_grid_and_without_one_it_is_plain_road() {
    let catalog = synthetic_catalog();
    let features = over_road(
        Family::Asf1,
        2.5,
        Family::Grav,
        1.25,
        std::f64::consts::FRAC_PI_2,
    );
    let (graph, fits) = fits(&features, &catalog);
    assert!(
        matches!(fits[0].kind, BridgeKind::Panels { .. }),
        "{:?}",
        fits[0]
    );
    assert!(fits[0].below[0].missing.abs() < 1e-6);
    assert!(fits[0].grown.0 > 1.0, "{:?}", fits[0].grown);
    assert!(graph.stretches.iter().any(|s| s.bridge));
    // the road is found in the fitted graph, so the height solve keeps its clearance
    assert_eq!(over_crossings(&graph).len(), 1);
    let mut alone = features;
    alone.pop();
    let (_, fits) = fits_of(&alone, &catalog);
    assert!(matches!(fits[0].kind, BridgeKind::Fill));
}

fn fits_of(features: &[Feature], catalog: &PieceCatalog) -> (RoadGraph, Vec<BridgeFit>) {
    fits(features, catalog)
}

#[test]
fn a_model_shorter_than_its_bridge_is_slid_over_the_road_below() {
    // 25 m of bridge on a gravel track; where the ground under the posts of bridge_wood_25 is 10 m down the plain
    // deck of bridge_wood2 (19.7 m) is laid, centred (2.65 m from the start). A 5 m road square at 18 m needs
    // 11.8..24.2 m (its footprint and the cutting's walls): the deck must start at 4.5 m
    let catalog = synthetic_catalog();
    let features = over_road(
        Family::Mud,
        25.0,
        Family::Mud,
        18.0,
        std::f64::consts::FRAC_PI_2,
    );
    let valley = |p: (f64, f64)| {
        if (404.0..421.0).contains(&p.0) {
            -10.0
        } else {
            0.0
        }
    };
    let fits = fits_on(&features, &catalog, &valley);
    let fit = &fits[0];
    assert_eq!(model_name(&fit.kind), "bridge_wood2.p3d", "{fit:?}");
    assert!(near(fit.part_start, 4.5, 2e-3), "{}", fit.part_start);
    assert!(near(fit.slid, 4.5 - 2.65, 2e-3), "{}", fit.slid);
    assert!(fit.below[0].missing.abs() < 1e-6, "{fit:?}");
    assert!(near(fit.grown.0, -fit.part_start, 1e-9));
    // the same where the part keeps to a node of bridges at its start: it cannot move
    let at_start = over_road(
        Family::Mud,
        25.0,
        Family::Mud,
        8.0,
        std::f64::consts::FRAC_PI_2,
    );
    let fits = fits_on(&at_start, &catalog, &valley);
    // 1.8..14.2 m needs the deck to start at most 1.8 m in: the other way
    assert!(near(fits[0].part_start, 1.8, 2e-3), "{:?}", fits[0]);
    assert!(near(fits[0].slid, 1.8 - 2.65, 2e-3), "{:?}", fits[0]);
}

#[test]
fn a_model_on_posts_is_tried_at_every_start_against_its_posts() {
    // bridge_wood_25 on a 25 m bridge, a road below square at 12 m: its rows of posts leave no gap for it; the
    // starts that are tried are checked against the posts (valid ones only) and the bridge ends as a grid or as the
    // model with what is missing
    let catalog = synthetic_catalog();
    let features = over_road(
        Family::Mud,
        25.0,
        Family::Mud,
        12.0,
        std::f64::consts::FRAC_PI_2,
    );
    let (_, fits) = fits(&features, &catalog);
    assert!(fits[0].below[0].missing.is_finite());
    assert_eq!(fits[0].below.len(), 1);
}

#[test]
fn a_blocked_model_stays_where_there_is_no_panel_to_replace_it() {
    use naves_core::catalog::PANELS_PREFIX;
    let mut models = synthetic_models();
    models.retain(|path, _| !path.starts_with(PANELS_PREFIX));
    let catalog = PieceCatalog::from_models(&models).unwrap();
    let features = over_road(
        Family::Asf1,
        25.0,
        Family::City,
        12.5,
        std::f64::consts::FRAC_PI_4,
    );
    let (_, fits) = fits(&features, &catalog);
    let fit = &fits[0];
    assert_eq!(model_name(&fit.kind), "bridge_metal_25.p3d", "{fit:?}");
    assert!(fit.below[0].missing > 1.0, "{fit:?}");
}

#[test]
fn the_report_texts_of_a_lengthened_grid_and_a_blocked_model() {
    let grid = Fallback::GridLengthened { by: (2.5, 0.0) };
    assert_eq!(
        grid.to_string(),
        "the panel grid was lengthened by 2.5 m at the start and 0.0 m at the end for the road below"
    );
    let model = Fallback::ModelBlocksRoad {
        model: "dz\\structures\\roads\\bridges\\bridge_metal_25.p3d".to_string(),
    };
    assert_eq!(
        model.to_string(),
        "bridge_metal_25 would stand on the road below the bridge"
    );
}

#[test]
fn a_fit_with_no_start_to_try_keeps_the_part_where_it_was_asked_for() {
    let catalog = synthetic_catalog();
    let metal = catalog
        .bridges()
        .iter()
        .find(|b| b.model.ends_with("bridge_metal_25.p3d"))
        .unwrap();
    let shape = metal.shape();
    let chain = Chain {
        shape: &shape,
        count: 1,
    };
    let need = Need {
        station: 10.0,
        reach: below_interval(6.0, std::f64::consts::FRAC_PI_2, &shape),
    };
    let fit = fit_over(&chain, (f64::NAN, f64::NAN), f64::NAN, &[need], &|_| true);
    assert!(matches!(fit, Fit::Blocked { .. }), "{fit:?}");
    assert!(fit.start().is_nan());
}

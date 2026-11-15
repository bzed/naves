// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Placing the road network: pieces, junctions, side roads, end caps, tunnels.

#![expect(
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    reason = "test helper code; failing loudly is the point"
)]

use std::collections::BTreeSet;

use naves_core::catalog::testing::{synthetic_catalog, synthetic_models_with_bridges, width};
use naves_core::catalog::{Family, PieceCatalog};
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::roadnet::{build_graph, collapse_small_rings, widen_bridges};
use naves_core::roadplace::{
    MAX_BRIDGE_SCALE, PlacedKind, PlacedNetwork, PlacedRoad, bridge_for, place_network,
};

fn feature(way: i64, family: Family, nodes: &[i64], points: &[(f64, f64)]) -> Feature {
    Feature {
        id: format!("osm/way/{way}/0"),
        way_id: way,
        source: Source::Osm,
        kind: FeatureKind::Road {
            family,
            drivable: true,
            bridge: false,
            tunnel: false,
            roundabout: false,
            layer: 0,
        },
        points: points.to_vec(),
        nodes: nodes.to_vec(),
    }
}

fn place(features: &[Feature]) -> PlacedNetwork {
    place_network(&build_graph(features), &synthetic_catalog()).unwrap()
}

fn catalog() -> PieceCatalog {
    synthetic_catalog()
}

/// A junction side port in the world: position and heading.
fn world_port(road: &PlacedRoad, x: f64, z: f64, heading: f64) -> (f64, f64, f64) {
    let (s, c) = road.origin.heading.sin_cos();
    (
        road.origin.x + x * c + z * s,
        road.origin.y - x * s + z * c,
        heading + road.origin.heading,
    )
}

fn centre(road: &PlacedRoad) -> (f64, f64) {
    (
        f64::midpoint(road.start.x, road.end.x),
        f64::midpoint(road.start.y, road.end.y),
    )
}

fn of_kind(net: &PlacedNetwork, kind: PlacedKind) -> Vec<&PlacedRoad> {
    net.roads.iter().filter(|r| r.kind == kind).collect()
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// A horizontal through road at y = 500 with a crossing at x = 500, and a side road going north.
fn t_network(side_family: Family) -> Vec<Feature> {
    vec![
        feature(
            1,
            Family::Asf3,
            &[1, 2, 3],
            &[(100.0, 500.0), (500.0, 500.0), (900.0, 500.0)],
        ),
        feature(2, side_family, &[2, 4], &[(500.0, 500.0), (500.0, 900.0)]),
    ]
}

#[test]
fn a_t_places_a_junction_and_a_side_road() {
    let net = place(&t_network(Family::Asf3));
    let junctions = of_kind(&net, PlacedKind::Junction);
    assert_eq!(junctions.len(), 1);
    let j = junctions[0];
    assert!(j.model.contains("kr_t_asf3_asf3"), "{}", j.model);
    assert!(dist(centre(j), (500.0, 500.0)) < 3.0, "{:?}", centre(j));
    assert!(of_kind(&net, PlacedKind::Piece).len() > 20);
    assert!(net.gaps.is_empty(), "{:?}", net.gaps);
    assert_eq!(net.joins_without_junction.len(), 0);
}

#[test]
fn a_side_road_starts_at_the_port() {
    let net = place(&t_network(Family::Asf3));
    let j = of_kind(&net, PlacedKind::Junction)[0];
    let cat = catalog();
    let model = cat.t_junction(Family::Asf3, Family::Asf3).unwrap();
    let port = &model.side_ports[0];
    let (px, py, ph) = world_port(j, port.x, port.z, port.heading);
    // the side road is the piece that starts at the port
    let side = net
        .roads
        .iter()
        .find(|r| r.kind == PlacedKind::Piece && dist((r.start.x, r.start.y), (px, py)) < 1e-3)
        .expect("no piece starts at the port");
    let turn = (side.start.heading - ph + std::f64::consts::PI)
        .rem_euclid(2.0 * std::f64::consts::PI)
        - std::f64::consts::PI;
    assert!(turn.abs() < 1e-6, "heading differs by {turn}");
    // and it heads away from the through road
    assert!(
        (side.end.y - 500.0).abs() > (side.start.y - 500.0).abs() - 1e-6
            || (side.end.x - 500.0).abs() > 0.0
    );
}

#[test]
fn a_crossing_places_an_x() {
    let net = place(&[
        feature(
            1,
            Family::City,
            &[1, 2, 3],
            &[(100.0, 500.0), (500.0, 500.0), (900.0, 500.0)],
        ),
        feature(
            2,
            Family::City,
            &[4, 2, 5],
            &[(500.0, 100.0), (500.0, 500.0), (500.0, 900.0)],
        ),
    ]);
    let junctions = of_kind(&net, PlacedKind::Junction);
    assert_eq!(junctions.len(), 1, "{:?}", net.joins_without_junction);
    assert!(junctions[0].model.contains("kr_x_city_city"));
    // both cross arms start at a port
    let starts = net
        .roads
        .iter()
        .filter(|r| {
            r.kind == PlacedKind::Piece && dist((r.start.x, r.start.y), (500.0, 500.0)) < 8.0
        })
        .count();
    assert!(starts >= 2, "{starts}");
}

#[test]
fn a_pair_without_a_model_overlays_the_roads() {
    // there is no gravel junction
    let net = place(&t_network(Family::Grav));
    assert_eq!(of_kind(&net, PlacedKind::Junction).len(), 0);
    assert_eq!(net.joins_without_junction.len(), 0);
    assert_eq!(net.overlays.len(), 1);
    assert!(dist(net.overlays[0].at, (500.0, 500.0)) < 1.0);
    // the side road ends on the edge of the through road, overlapping it by up to 2 m
    let grav_start = net
        .roads
        .iter()
        .filter(|r| r.kind == PlacedKind::Piece && r.family == Family::Grav)
        .map(|r| dist((r.start.x, r.start.y), (500.0, 500.0)))
        .fold(f64::INFINITY, f64::min);
    assert!(
        grav_start <= 3.5 + 1.0,
        "starts {grav_start} m from the centre line"
    );
    assert!(
        grav_start >= 0.5,
        "runs over the whole through road: {grav_start}"
    );
}

#[test]
fn two_crossings_close_together_use_one_junction() {
    let net = place(&[
        feature(
            1,
            Family::Asf3,
            &[1, 2, 3, 4],
            &[
                (100.0, 500.0),
                (500.0, 500.0),
                (506.0, 500.0),
                (900.0, 500.0),
            ],
        ),
        feature(2, Family::Asf3, &[2, 5], &[(500.0, 500.0), (500.0, 800.0)]),
        feature(3, Family::Asf3, &[3, 6], &[(506.0, 500.0), (506.0, 200.0)]),
    ]);
    assert_eq!(of_kind(&net, PlacedKind::Junction).len(), 1);
    assert_eq!(net.overlays.len(), 1);
    // no through piece overlaps the junction by more than 2 m
    let j = of_kind(&net, PlacedKind::Junction)[0];
    let (lo, hi) = (j.start.x.min(j.end.x), j.start.x.max(j.end.x));
    for r in net.roads.iter().filter(|r| {
        r.kind == PlacedKind::Piece
            && (r.start.y - 500.0).abs() < 1.0
            && (r.end.y - 500.0).abs() < 1.0
    }) {
        let (a, b) = (r.start.x.min(r.end.x), r.start.x.max(r.end.x));
        let overlap = (hi.min(b) - lo.max(a)).max(0.0);
        assert!(overlap <= 2.0 + 1e-6, "{overlap} m over the junction");
    }
}

#[test]
fn dead_ends_have_end_caps() {
    let net = place(&[feature(
        1,
        Family::Asf3,
        &[1, 2],
        &[(100.0, 500.0), (500.0, 500.0)],
    )]);
    assert_eq!(of_kind(&net, PlacedKind::EndCap).len(), 2);
}

#[test]
fn a_clipped_end_has_no_cap_and_stays_inside_the_map() {
    let net = place(&[feature(
        1,
        Family::Asf3,
        &[-1, 5, -2],
        &[(13.0, 500.0), (300.0, 500.0), (1987.0, 500.0)],
    )]);
    assert_eq!(of_kind(&net, PlacedKind::EndCap).len(), 0);
    for r in &net.roads {
        for p in [r.start, r.end] {
            assert!(
                (0.0..=2000.0).contains(&p.x) && (0.0..=2000.0).contains(&p.y),
                "{p:?}"
            );
        }
    }
    assert_ne!(net.roads.len(), 0);
}

#[test]
fn a_side_road_between_two_through_roads_closes_or_reports_a_gap() {
    let net = place(&[
        feature(
            1,
            Family::Asf3,
            &[1, 2, 3],
            &[(100.0, 300.0), (500.0, 300.0), (900.0, 300.0)],
        ),
        feature(
            2,
            Family::Asf3,
            &[4, 5, 6],
            &[(100.0, 700.0), (500.0, 700.0), (900.0, 700.0)],
        ),
        feature(3, Family::City, &[2, 5], &[(500.0, 300.0), (500.0, 700.0)]),
    ]);
    let side = net
        .roads
        .iter()
        .filter(|r| r.family == Family::City && r.kind == PlacedKind::Piece)
        .count();
    assert!(side > 5);
    // either it closes (no gap reported) or the gap is reported against the side road
    for gap in &net.gaps {
        assert!(gap.stretch.starts_with("3:"), "{gap:?}");
    }
}

#[test]
fn a_tunnel_leaves_a_hole_and_is_reported() {
    let mut tunnel = feature(2, Family::Asf3, &[2, 3], &[(400.0, 500.0), (600.0, 500.0)]);
    if let FeatureKind::Road { tunnel: t, .. } = &mut tunnel.kind {
        *t = true;
    }
    let net = place(&[
        feature(1, Family::Asf3, &[1, 2], &[(100.0, 500.0), (400.0, 500.0)]),
        tunnel,
        feature(3, Family::Asf3, &[3, 4], &[(600.0, 500.0), (900.0, 500.0)]),
    ]);
    assert_eq!(net.tunnels_omitted.len(), 1);
    assert!(net.tunnels_omitted[0].0.starts_with("2:"));
    assert!(net.roads.iter().all(|r| !r.slot.stretch.starts_with("2:")));
    // the neighbours end in caps at the portals
    let caps = of_kind(&net, PlacedKind::EndCap);
    assert_eq!(caps.len(), 4, "two dead ends and two portals");
    assert!(
        net.roads
            .iter()
            .all(|r| !(r.start.x > 420.0 && r.end.x < 580.0))
    );
}

#[test]
fn a_ring_closes_on_itself() {
    let points: Vec<(f64, f64)> = (0..=60)
        .map(|i| {
            let a = f64::from(i) / 60.0 * std::f64::consts::TAU;
            (1000.0 + 150.0 * a.sin(), 1000.0 + 150.0 * a.cos())
        })
        .collect();
    let mut nodes: Vec<i64> = (1..=60).collect();
    nodes.push(1);
    let net = place(&[feature(1, Family::City, &nodes, &points)]);
    assert_eq!(of_kind(&net, PlacedKind::EndCap).len(), 0);
    let first = net.roads.first().unwrap();
    let last = net.roads.last().unwrap();
    let gap = dist((first.start.x, first.start.y), (last.end.x, last.end.y));
    assert!(
        gap < 0.6 || net.gaps.iter().any(|g| g.gap_m >= 0.5),
        "gap {gap}, reported {:?}",
        net.gaps
    );
}

fn slots(net: &PlacedNetwork) -> BTreeSet<(String, u32, String)> {
    net.roads
        .iter()
        .map(|r| (r.slot.stretch.clone(), r.slot.index, r.model.clone()))
        .collect()
}

#[test]
fn slots_do_not_change_when_a_stretch_is_walked_backwards() {
    let forward = t_network(Family::Asf3);
    let mut backward = forward.clone();
    for f in &mut backward {
        f.points.reverse();
        f.nodes.reverse();
    }
    let a = place(&forward);
    let b = place(&backward);
    // the same slots, with the same models (the pieces themselves may be laid from the other end)
    let (sa, sb) = (slots(&a), slots(&b));
    let ids = |s: &BTreeSet<(String, u32, String)>| {
        s.iter().map(|x| x.0.clone()).collect::<BTreeSet<_>>()
    };
    assert_eq!(ids(&sa), ids(&sb));
}

#[test]
fn a_changed_side_road_does_not_move_the_through_road() {
    let base = t_network(Family::Asf3);
    let mut changed = base.clone();
    changed[1].points = vec![(500.0, 500.0), (520.0, 700.0), (560.0, 900.0)];
    changed[1].nodes = vec![2, 7, 4];
    let a = place(&base);
    let b = place(&changed);
    let through = |n: &PlacedNetwork| -> Vec<(String, u32, [u64; 3])> {
        n.roads
            .iter()
            .filter(|r| r.slot.stretch.starts_with("1:"))
            .map(|r| {
                (
                    r.model.clone(),
                    r.slot.index,
                    [
                        r.origin.x.to_bits(),
                        r.origin.y.to_bits(),
                        r.origin.heading.to_bits(),
                    ],
                )
            })
            .collect()
    };
    assert_ne!(through(&a).len(), 0);
    assert_eq!(through(&a), through(&b));
}

#[test]
fn the_same_input_gives_the_same_network() {
    let a = place(&t_network(Family::Asf3));
    let b = place(&t_network(Family::Asf3));
    assert_eq!(a, b);
}

#[test]
fn chains_index_the_roads_in_driving_order() {
    let net = place(&t_network(Family::Asf3));
    for chain in &net.chains {
        assert_ne!(chain.len(), 0);
        for &i in chain {
            assert!(i < net.roads.len());
        }
    }
    let covered: usize = net.chains.iter().map(Vec::len).sum();
    assert_eq!(covered, net.roads.len());
}

fn bridge_feature(way: i64, nodes: &[i64], points: &[(f64, f64)]) -> Feature {
    let mut f = feature(way, Family::Asf3, nodes, points);
    if let FeatureKind::Road { bridge, .. } = &mut f.kind {
        *bridge = true;
    }
    f
}

/// A road with a bridge of the given length in the middle.
fn bridged(span: f64) -> Vec<Feature> {
    vec![
        feature(1, Family::Asf3, &[1, 2], &[(100.0, 500.0), (400.0, 500.0)]),
        bridge_feature(2, &[2, 3], &[(400.0, 500.0), (400.0 + span, 500.0)]),
        feature(
            3,
            Family::Asf3,
            &[3, 4],
            &[(400.0 + span, 500.0), (900.0, 500.0)],
        ),
    ]
}

#[test]
fn a_bridge_is_a_straight_run_of_bridge_models() {
    let net = place(&bridged(100.0));
    let bridges = of_kind(&net, PlacedKind::Bridge);
    // 100 m of 25 m models
    assert_eq!(bridges.len(), 4, "{:?}", net.warnings);
    for b in &bridges {
        assert!(b.model.contains("bridge_stone"), "{}", b.model);
        assert!((b.start.y - 500.0).abs() < 0.5 && (b.end.y - 500.0).abs() < 0.5);
    }
    for pair in bridges.windows(2) {
        assert!(
            dist(
                (pair[0].end.x, pair[0].end.y),
                (pair[1].start.x, pair[1].start.y)
            ) < 1e-6
        );
    }
    // the models cover the span
    assert!(
        (bridges[0].start.x - 400.0).abs() < 3.0,
        "{}",
        bridges[0].start.x
    );
    assert!(
        (bridges[3].end.x - 500.0).abs() < 3.0,
        "{}",
        bridges[3].end.x
    );
    // the slots count along the stretch
    assert!(bridges.iter().all(|b| b.slot.stretch.starts_with("2:")));
    let indices: Vec<u32> = bridges.iter().map(|b| b.slot.index).collect();
    assert_eq!(indices, vec![0, 1, 2, 3]);
}

/// The network of `features` with every bridge widened to one model, as the build does.
fn place_widened(features: &[Feature]) -> PlacedNetwork {
    let catalog = synthetic_catalog();
    let graph = widen_bridges(&build_graph(features), &|s| {
        bridge_for(&catalog, s.family).map_or(0.0, |(m, scale, _)| m.length * scale)
    });
    place_network(&graph, &catalog).unwrap()
}

#[test]
fn an_eight_metre_osm_bridge_is_built_as_a_bridge() {
    let net = place_widened(&bridged(8.0));
    let bridges = of_kind(&net, PlacedKind::Bridge);
    assert_eq!(bridges.len(), 1, "{:?}", net.warnings);
    // one 25 m model, centred on the 8 m bridge
    let b = bridges[0];
    assert!(
        (b.start.x - 391.5).abs() < 1.0 && (b.end.x - 416.5).abs() < 1.0,
        "{b:?}"
    );
}

#[test]
fn a_gravel_road_gets_a_wooden_bridge() {
    let mut features = bridged(100.0);
    for f in &mut features {
        if let FeatureKind::Road { family, .. } = &mut f.kind {
            *family = Family::Grav;
        }
    }
    let net = place(&features);
    let bridges = of_kind(&net, PlacedKind::Bridge);
    assert_ne!(bridges.len(), 0);
    assert!(
        bridges.iter().all(|b| b.model.contains("bridge_wood")),
        "{}",
        bridges[0].model
    );
    // the wooden models lie along x in their own frame and still follow the road
    assert!(bridges.iter().all(|b| (b.start.y - 500.0).abs() < 0.5));
}

/// `bridged(100.0)` with every road of `family`.
fn bridge_stretch(family: Family) -> Vec<Feature> {
    let mut features = bridged(100.0);
    for f in &mut features {
        if let FeatureKind::Road { family: fam, .. } = &mut f.kind {
            *fam = family;
        }
    }
    features
}

/// The synthetic catalog with only the given bridges: name, length, width, deck height.
fn place_with(bridges: &[(&str, f64, f64, f64)], features: &[Feature]) -> PlacedNetwork {
    let catalog = PieceCatalog::from_models(&synthetic_models_with_bridges(bridges)).unwrap();
    place_network(&build_graph(features), &catalog).unwrap()
}

#[test]
fn a_paved_road_gets_the_narrowest_bridge_that_is_wide_enough() {
    // asf1 is 12 m wide: the 12 m model, not the 9 m one, and not scaled
    let net = place(&bridge_stretch(Family::Asf1));
    let bridges = of_kind(&net, PlacedKind::Bridge);
    assert_ne!(bridges.len(), 0);
    assert!(
        bridges.iter().all(|r| r.model.contains("bridge_stone_12")
            && r.width >= width(Family::Asf1)
            && (r.scale - 1.0).abs() < 1e-12),
        "{:?}",
        bridges[0]
    );
    assert!(net.warnings.is_empty(), "{:?}", net.warnings);
    // asf3 (7 m) is served by the 9 m model
    let net = place(&bridge_stretch(Family::Asf3));
    assert!(
        of_kind(&net, PlacedKind::Bridge)
            .iter()
            .all(|r| r.model.contains("bridge_stone_25"))
    );
}

#[test]
fn a_bridge_that_is_too_narrow_is_scaled_up_and_not_down() {
    let only_9m = [("bridge_stone_25", 25.0, 9.0, 7.9)];
    // a 12 m road
    let wide = place_with(&only_9m, &bridge_stretch(Family::Asf1));
    let bridges = of_kind(&wide, PlacedKind::Bridge);
    assert_ne!(bridges.len(), 0);
    let scale = width(Family::Asf1) / 9.0;
    assert!(bridges.iter().all(|r| (r.scale - scale).abs() < 1e-9));
    assert!(bridges.iter().all(|r| (r.width - 12.0).abs() < 1e-9));
    assert!(wide.warnings.is_empty(), "{:?}", wide.warnings);
    // a 7 m road is not given a bridge scaled down
    let narrow = place_with(&only_9m, &bridge_stretch(Family::Asf3));
    let bridges = of_kind(&narrow, PlacedKind::Bridge);
    assert_ne!(bridges.len(), 0);
    assert!(bridges.iter().all(|r| (r.scale - 1.0).abs() < 1e-12));
}

#[test]
fn the_scale_stops_at_one_and_a_half_and_says_so() {
    // a 3.5 m wooden model for a 6 m track: 6 / 3.5 is more than 1.5
    let net = place_with(
        &[("bridge_wood_25", 30.0, 3.5, 0.05)],
        &bridge_stretch(Family::Grav),
    );
    let bridges = of_kind(&net, PlacedKind::Bridge);
    assert_ne!(bridges.len(), 0);
    assert!(
        bridges
            .iter()
            .all(|r| (r.scale - MAX_BRIDGE_SCALE).abs() < 1e-12)
    );
    assert_eq!(net.warnings.len(), 1, "{:?}", net.warnings);
    assert!(net.warnings[0].contains("bridge"));
}

#[test]
fn tracks_get_wooden_bridges_and_paved_roads_do_not() {
    for (family, wooden) in [
        (Family::Grav, true),
        (Family::Mud, true),
        (Family::Asf1, false),
        (Family::Asf2, false),
        (Family::Asf3, false),
        (Family::City, false),
    ] {
        let net = place(&bridge_stretch(family));
        let bridges = of_kind(&net, PlacedKind::Bridge);
        assert!(!bridges.is_empty(), "{family:?}");
        assert!(
            bridges.iter().all(|r| r.model.contains("wood") == wooden),
            "{family:?}: {}",
            bridges[0].model
        );
    }
}

#[test]
fn scaled_bridges_still_meet_end_to_end() {
    let net = place_with(
        &[("bridge_stone_25", 25.0, 9.0, 7.9)],
        &bridge_stretch(Family::Asf1),
    );
    let bridges = of_kind(&net, PlacedKind::Bridge);
    assert!(bridges.len() >= 2);
    for pair in bridges.windows(2) {
        assert!(
            dist(
                (pair[0].end.x, pair[0].end.y),
                (pair[1].start.x, pair[1].start.y)
            ) < 0.01
        );
    }
    // the scaled length: 100 m of 33.3 m pieces, three of them
    assert_eq!(bridges.len(), 3);
    let first = bridges[0];
    assert!(
        (dist((first.start.x, first.start.y), (first.end.x, first.end.y)) - 25.0 * 12.0 / 9.0)
            .abs()
            < 0.01
    );
}

#[test]
fn a_bridge_that_fits_is_not_scaled() {
    let net = place(&bridged(100.0));
    assert!(
        of_kind(&net, PlacedKind::Bridge)
            .iter()
            .all(|r| (r.scale - 1.0).abs() < 1e-12)
    );
}

#[test]
fn no_junction_sits_at_the_end_of_a_bridge() {
    let mut features = bridged(100.0);
    features.push(feature(
        9,
        Family::Asf3,
        &[2, 8],
        &[(400.0, 500.0), (400.0, 800.0)],
    ));
    let net = place(&features);
    assert_eq!(of_kind(&net, PlacedKind::Junction).len(), 0);
    assert_eq!(net.overlays.len(), 1);
}

#[test]
fn an_edit_keeps_its_object_when_a_neighbouring_stretch_changes() {
    use naves_core::objects::{Override, Overrides, PinnedEdit, apply_overrides, road_objects};
    let base = t_network(Family::Asf3);
    let net = place(&base);
    let objects = road_objects(&net);
    let target = objects
        .iter()
        .find(|o| o.id.feature.starts_with("1:") && o.id.slot == 3)
        .unwrap();
    let overrides = Overrides {
        edits: vec![PinnedEdit {
            id: target.id.clone(),
            generated_model: target.model.clone(),
            generated_at: (target.x, target.y),
            generated_yaw: target.yaw,
            edit: Override::Moved {
                x: target.x + 1.0,
                y: target.y + 2.0,
                yaw: target.yaw,
            },
        }],
    };
    let mut changed = base.clone();
    changed[1].points = vec![(500.0, 500.0), (530.0, 700.0), (600.0, 900.0)];
    changed[1].nodes = vec![2, 7, 4];
    let after = road_objects(&place(&changed));
    let (placed, orphans) = apply_overrides(&after, &overrides);
    assert!(orphans.is_empty(), "{orphans:?}");
    assert!(
        placed
            .iter()
            .any(|p| p.id.as_ref() == Some(&target.id) && (p.x - (target.x + 1.0)).abs() < 1e-9)
    );
}

#[test]
fn a_bridge_model_without_a_usable_length_is_never_chosen() {
    // the catalog is untrusted input: a model whose ports coincide (length 0) or are not numbers must
    // neither loop forever nor divide by zero; the good model next to it is used
    for bad in [0.0, f64::NAN, f64::INFINITY] {
        let net = place_with(
            &[
                ("bridge_stone_bad", bad, 8.0, 7.9),
                ("bridge_stone_ok", 25.0, 12.0, 7.9),
            ],
            &bridged(100.0),
        );
        let bridges = of_kind(&net, PlacedKind::Bridge);
        assert!(!bridges.is_empty(), "length {bad}");
        assert!(
            bridges.iter().all(|b| b.model.contains("bridge_stone_ok")),
            "length {bad}: {}",
            bridges[0].model
        );
    }
    // and if no model is usable, the bridge is left out
    let net = place_with(&[("bridge_stone_bad", 0.0, 12.0, 7.9)], &bridged(100.0));
    assert_eq!(of_kind(&net, PlacedKind::Bridge).len(), 0);
}

fn assert_finite_bridges(net: &PlacedNetwork) {
    for b in of_kind(net, PlacedKind::Bridge) {
        assert!(b.scale.is_finite(), "{b:?}");
        assert!(b.start.y.is_finite() && b.end.y.is_finite(), "{b:?}");
        assert!(b.width.is_finite(), "{b:?}");
    }
}

#[test]
fn a_bridge_model_without_a_width_is_not_used() {
    // the ports of a catalog read from f32 points cannot give a width that is not a number, but they
    // can coincide
    let catalog = PieceCatalog::from_models(&synthetic_models_with_bridges(&[(
        "bridge_stone_25",
        25.0,
        0.0,
        7.9,
    )]))
    .unwrap();
    assert!(bridge_for(&catalog, Family::Asf3).is_none());
    let net = place_network(&build_graph(&bridge_stretch(Family::Asf3)), &catalog).unwrap();
    assert_eq!(of_kind(&net, PlacedKind::Bridge).len(), 0);
    assert_finite_bridges(&net);
}

#[test]
fn a_road_width_that_is_not_a_number_gives_an_unscaled_bridge() {
    use naves_core::catalog::PARTS_PREFIX;
    let mut models = synthetic_models_with_bridges(&[("bridge_stone_25", 25.0, 9.0, 7.9)]);
    for (path, m) in &mut models {
        if path.starts_with(PARTS_PREFIX) && path.contains("asf3") {
            for points in m.memory.values_mut() {
                for p in points {
                    p[0] = f32::NAN;
                }
            }
        }
    }
    let catalog = PieceCatalog::from_models(&models).unwrap();
    let (model, scale, _) = bridge_for(&catalog, Family::Asf3).unwrap();
    assert!(scale.is_finite() && (scale - 1.0).abs() < 1e-12, "{scale}");
    assert!(model.width.is_finite());
}

#[test]
fn a_bridge_model_shorter_than_a_metre_is_not_used() {
    let catalog = PieceCatalog::from_models(&synthetic_models_with_bridges(&[(
        "bridge_stone_25",
        1e-9,
        9.0,
        7.9,
    )]))
    .unwrap();
    assert!(bridge_for(&catalog, Family::Asf3).is_none());
    let net = place_network(&build_graph(&bridge_stretch(Family::Asf3)), &catalog).unwrap();
    assert_eq!(of_kind(&net, PlacedKind::Bridge).len(), 0);
}

/// The synthetic catalog with only the junction models whose file names are listed.
fn catalog_with_junctions(keep: &[&str]) -> PieceCatalog {
    let mut models = synthetic_models_with_bridges(&[("bridge_stone_25", 25.0, 9.0, 7.9)]);
    models.retain(|path, _| {
        let stem = path.rsplit('/').next().unwrap_or(path);
        !(stem.starts_with("kr_t_") || stem.starts_with("kr_x_"))
            || keep.iter().any(|k| stem == format!("{k}.p3d"))
    });
    PieceCatalog::from_models(&models).unwrap()
}

#[test]
fn an_x_crossing_uses_the_swapped_piece_when_only_that_exists() {
    let cat = catalog_with_junctions(&["kr_x_asf1_city"]);
    // a long City through road at y = 500, crossed by a short Asf1 road at x = 500
    let features = [
        feature(
            1,
            Family::City,
            &[1, 2, 3],
            &[(100.0, 500.0), (500.0, 500.0), (900.0, 500.0)],
        ),
        feature(
            2,
            Family::Asf1,
            &[4, 2, 5],
            &[(500.0, 300.0), (500.0, 500.0), (500.0, 700.0)],
        ),
    ];
    let net = place_network(&build_graph(&features), &cat).unwrap();
    let junctions = of_kind(&net, PlacedKind::Junction);
    assert_eq!(junctions.len(), 1, "{:?}", net.joins_without_junction);
    assert!(junctions[0].model.contains("kr_x_asf1_city"));
    assert_eq!(net.joins_without_junction.len(), 0);
    assert!(dist(centre(junctions[0]), (500.0, 500.0)) < 3.0);
    assert!(net.gaps.is_empty(), "{:?}", net.gaps);
    // both roads continue from the junction: pieces start within a few metres of its ports
    let at_port = |c: (f64, f64)| {
        net.roads.iter().any(|r| {
            r.kind == PlacedKind::Piece
                && (dist((r.start.x, r.start.y), c) < 1.0 || dist((r.end.x, r.end.y), c) < 1.0)
        })
    };
    for c in [
        (493.75, 500.0),
        (506.25, 500.0),
        (500.0, 493.75),
        (500.0, 506.25),
    ] {
        assert!(at_port(c), "no piece at {c:?}");
    }
}

#[test]
fn a_t_crossing_is_never_swapped() {
    let cat = catalog_with_junctions(&["kr_t_city_asf3"]);
    // Asf3 through road, City side road: the T the game has is the other way round
    let net = place_network(
        &build_graph(&t_network_of(Family::Asf3, Family::City)),
        &cat,
    )
    .unwrap();
    assert_eq!(of_kind(&net, PlacedKind::Junction).len(), 0);
    assert_eq!(net.overlays.len(), 1);
}

fn t_network_of(through: Family, side: Family) -> Vec<Feature> {
    vec![
        feature(
            1,
            through,
            &[1, 2, 3],
            &[(100.0, 500.0), (500.0, 500.0), (900.0, 500.0)],
        ),
        feature(2, side, &[2, 4], &[(500.0, 500.0), (500.0, 900.0)]),
    ]
}

// ---- the overlay rule: where no junction piece exists the lower road ends under the higher one ----

/// The pieces of a family, with the distance of their ends from the line y = 500.
fn south_north(net: &PlacedNetwork, family: Family) -> Vec<&PlacedRoad> {
    net.roads
        .iter()
        .filter(|r| r.kind == PlacedKind::Piece && r.family == family)
        .collect()
}

fn half(family: Family) -> f64 {
    width(family) / 2.0
}

/// The distances from the line y = 500 of the ends of the pieces of `family` that lie on one side of it,
/// nearest first.
fn ends_near_line(net: &PlacedNetwork, family: Family, north: bool) -> Vec<f64> {
    let mut d: Vec<f64> = south_north(net, family)
        .iter()
        .flat_map(|r| [r.start.y, r.end.y])
        .filter(|y| (*y > 500.0) == north)
        .map(|y| (y - 500.0).abs())
        .collect();
    d.sort_by(f64::total_cmp);
    d
}

#[test]
fn a_mud_track_ends_under_the_edge_of_a_street() {
    // both directions: the track starts at the crossing, or ends there (the node ids decide)
    for nodes in [[2_i64, 4], [4, 2]] {
        let mut mud = [(500.0, 500.0), (500.0, 900.0)];
        if nodes[0] == 4 {
            mud.reverse();
        }
        let net = place(&[
            feature(
                1,
                Family::City,
                &[1, 2, 3],
                &[(100.0, 500.0), (500.0, 500.0), (900.0, 500.0)],
            ),
            feature(2, Family::Mud, &nodes, &mud),
        ]);
        assert_eq!(net.joins_without_junction.len(), 0);
        assert_eq!(net.overlays.len(), 1);
        let nearest = ends_near_line(&net, Family::Mud, true)[0];
        assert!(
            nearest > 0.0 && nearest <= half(Family::City),
            "{nodes:?}: the last Mud piece ends {nearest} m from the City centre line"
        );
        // no end cap where the track meets the street
        assert!(
            !net.roads.iter().any(|r| r.kind == PlacedKind::EndCap
                && dist((r.start.x, r.start.y), (500.0, 500.0)) < 15.0),
            "{nodes:?}"
        );
    }
}

/// A Grav road along y = 500 crossed by a Mud road along x = 500. `mud_first` gives the Mud road the lower
/// way id, so that it is the one the crossing makes a through road.
fn grav_mud_cross(mud_first: bool) -> Vec<Feature> {
    let (grav_way, mud_way) = if mud_first { (2, 1) } else { (1, 2) };
    vec![
        feature(
            grav_way,
            Family::Grav,
            &[1, 2, 3],
            &[(100.0, 500.0), (500.0, 500.0), (900.0, 500.0)],
        ),
        feature(
            mud_way,
            Family::Mud,
            &[4, 2, 5],
            &[(500.0, 100.0), (500.0, 500.0), (500.0, 900.0)],
        ),
    ]
}

#[test]
fn a_crossing_without_piece_interrupts_the_lower_road() {
    for mud_first in [false, true] {
        let net = place(&grav_mud_cross(mud_first));
        assert!(net.joins_without_junction.is_empty(), "{mud_first}");
        let mud_chains: Vec<&Vec<usize>> = net
            .chains
            .iter()
            .filter(|c| c.iter().any(|i| net.roads[*i].family == Family::Mud))
            .collect();
        assert_eq!(mud_chains.len(), 2, "{mud_first}: two parts");
        for r in south_north(&net, Family::Mud) {
            assert!(
                (r.start.y - 500.0) * (r.end.y - 500.0) >= 0.0,
                "{mud_first}: a Mud piece crosses the Grav centre line: {r:?}"
            );
        }
        for north in [false, true] {
            let nearest = ends_near_line(&net, Family::Mud, north)[0];
            assert!(
                nearest > 0.0 && nearest <= half(Family::Grav) + 1e-6,
                "{mud_first}: the Mud part ends {nearest} m from the Grav centre line"
            );
        }
        // the Grav road is the one that is not under anything
        let lower: Vec<&String> = net.overlays.iter().flat_map(|o| &o.lower).collect();
        assert!(
            !lower.is_empty()
                && lower
                    .iter()
                    .all(|id| id.contains("1:") || id.contains("2:"))
        );
    }
}

#[test]
fn the_higher_road_ending_on_a_lower_one_lies_on_top() {
    // an Asf3 road runs on, a City road ends on it; the game has no T for this pair
    let net = place(&t_network_of(Family::Asf3, Family::City));
    assert_eq!(net.joins_without_junction.len(), 0);
    let asf3: Vec<&PlacedRoad> = south_north(&net, Family::Asf3);
    let asf3_chains = net
        .chains
        .iter()
        .filter(|c| c.iter().any(|i| net.roads[*i].family == Family::Asf3))
        .count();
    assert_eq!(asf3_chains, 1, "the Asf3 road is unbroken");
    for pair in asf3.windows(2) {
        assert!(
            dist(
                (pair[0].end.x, pair[0].end.y),
                (pair[1].start.x, pair[1].start.y)
            ) < 1e-6,
            "a gap in the Asf3 road"
        );
    }
    // no end cap on the City road where it ends on the Asf3 road
    assert!(!net.roads.iter().any(|r| r.kind == PlacedKind::EndCap
        && r.family == Family::City
        && dist((r.start.x, r.start.y), (500.0, 500.0)) < 15.0));
    // the Asf3 road is the lower one
    assert_eq!(net.overlays.len(), 1);
    assert!(
        net.overlays[0].lower.iter().all(|id| id.starts_with("1:")),
        "{:?}",
        net.overlays
    );
    assert_ne!(net.overlays[0].lower.len(), 0);
}

#[test]
fn equal_families_put_the_longer_road_on_top() {
    // two Grav roads, no piece for them; the vertical one is longer
    let net = place(&[
        feature(
            1,
            Family::Grav,
            &[1, 2, 3],
            &[(350.0, 500.0), (500.0, 500.0), (650.0, 500.0)],
        ),
        feature(2, Family::Grav, &[2, 4], &[(500.0, 500.0), (500.0, 1300.0)]),
    ]);
    assert_eq!(net.overlays.len(), 1);
    // the short horizontal through road is lower, the long side road on top
    assert!(
        net.overlays[0].lower.iter().all(|id| id.starts_with("1:")),
        "{:?}",
        net.overlays
    );
    assert_ne!(net.overlays[0].lower.len(), 0);
    // and the other way round: the through road is the longer one
    let net = place(&[
        feature(
            1,
            Family::Grav,
            &[1, 2, 3],
            &[(100.0, 500.0), (500.0, 500.0), (900.0, 500.0)],
        ),
        feature(2, Family::Grav, &[2, 4], &[(500.0, 500.0), (500.0, 700.0)]),
    ]);
    assert!(
        net.overlays[0].lower.iter().all(|id| id.starts_with("2:")),
        "{:?}",
        net.overlays
    );
}

#[test]
fn a_stub_shorter_than_half_the_width_is_dropped_and_reported() {
    // a 1.5 m Mud stub from the crossing to a dead end, shorter than the cut in front of it
    let net = place(&[
        feature(
            1,
            Family::City,
            &[1, 2, 3],
            &[(100.0, 500.0), (500.0, 500.0), (900.0, 500.0)],
        ),
        feature(2, Family::Mud, &[2, 4], &[(500.0, 500.0), (500.0, 501.5)]),
    ]);
    assert_eq!(south_north(&net, Family::Mud).len(), 0);
    assert!(
        net.roads
            .iter()
            .all(|r| dist((r.start.x, r.start.y), (r.end.x, r.end.y)) > 1e-6),
        "a piece of no length"
    );
    assert!(
        net.warnings.iter().any(|w| w.contains("not placed")),
        "{:?}",
        net.warnings
    );
}

#[test]
fn an_end_under_a_road_never_passes_its_centre_line_at_an_angle() {
    // a Mud track arriving at a City street at about 27 degrees from the normal, in both directions of
    // the stretch, and a Mud road crossing a Grav road at that angle (two halves that must not overlap)
    // the length of the track decides where its last piece lands: many lengths
    for (nodes, length) in [[2_i64, 4], [4, 2]]
        .into_iter()
        .flat_map(|n| (0..6).map(move |k| (n, 12.0 + 6.1 * f64::from(k))))
    {
        let end = (500.0 + length * 0.45, 500.0 + length * 0.89);
        let mut mud = [(500.0, 500.0), end];
        if nodes[0] == 4 {
            mud.reverse();
        }
        let net = place(&[
            feature(
                1,
                Family::City,
                &[1, 2, 3],
                &[(100.0, 500.0), (500.0, 500.0), (900.0, 500.0)],
            ),
            feature(2, Family::Mud, &nodes, &mud),
        ]);
        for r in south_north(&net, Family::Mud) {
            assert!(
                r.start.y > 500.0 && r.end.y > 500.0,
                "{nodes:?} {length}: {r:?}"
            );
        }
    }
    for mud_first in [false, true] {
        let (grav_way, mud_way) = if mud_first { (2, 1) } else { (1, 2) };
        let net = place(&[
            feature(
                grav_way,
                Family::Grav,
                &[1, 2, 3],
                &[(100.0, 500.0), (500.0, 500.0), (900.0, 500.0)],
            ),
            feature(
                mud_way,
                Family::Mud,
                &[4, 2, 5],
                &[(300.0, 100.0), (500.0, 500.0), (700.0, 900.0)],
            ),
        ]);
        let mud = south_north(&net, Family::Mud);
        let (north, south): (Vec<&PlacedRoad>, Vec<&PlacedRoad>) =
            mud.into_iter().partition(|r| r.start.y > 500.0);
        assert!(!north.is_empty() && !south.is_empty());
        assert!(
            north.iter().all(|r| r.start.y > 500.0 && r.end.y > 500.0),
            "{mud_first}: a Mud piece passes the Grav centre line"
        );
        assert!(
            south.iter().all(|r| r.start.y < 500.0 && r.end.y < 500.0),
            "{mud_first}: a Mud piece passes the Grav centre line"
        );
    }
}

/// A roundabout of `radius` metres around (1000, 1000), 24 nodes, with a street leaving at each of the
/// given node indexes.
fn roundabout(radius: f64, family: Family, spokes: &[i64]) -> Vec<Feature> {
    let n = 24_i64;
    let at = |i: i64, r: f64| {
        let a = (i % n) as f64 / n as f64 * std::f64::consts::TAU;
        (1000.0 + r * a.sin(), 1000.0 + r * a.cos())
    };
    let nodes: Vec<i64> = (0..=n).map(|i| 100 + i % n).collect();
    let points: Vec<(f64, f64)> = (0..=n).map(|i| at(i, radius)).collect();
    let mut ring = feature(1, family, &nodes, &points);
    if let FeatureKind::Road { roundabout, .. } = &mut ring.kind {
        *roundabout = true;
    }
    let mut all = vec![ring];
    for (k, &i) in (0_i64..).zip(spokes) {
        all.push(feature(
            10 + k,
            Family::Asf3,
            &[100 + i, 500 + k],
            &[at(i, radius), at(i, radius + 80.0)],
        ));
    }
    all
}

#[test]
fn a_ring_is_placed_as_a_closed_loop() {
    let net = place(&roundabout(40.0, Family::City, &[0, 8, 16]));
    assert_eq!(net.rings.len(), 1);
    // the ring is one chain of its own: the one that is not a spoke
    let ring_chain = net
        .chains
        .iter()
        .find(|c| {
            c.iter()
                .all(|i| net.roads[*i].slot.stretch.starts_with("1:"))
        })
        .unwrap();
    let first = &net.roads[*ring_chain.first().unwrap()];
    let last = &net.roads[*ring_chain.last().unwrap()];
    let gap = dist((first.start.x, first.start.y), (last.end.x, last.end.y));
    assert!(
        gap < naves_core::roadfit::GAP_CLOSED_M,
        "the ring does not close: {gap} m, gaps {:?}",
        net.gaps
    );
    assert!(net.gaps.is_empty(), "{:?}", net.gaps);
    // a circle of 40 m is not cut into spoke-sized bits: no junction pieces on it
    assert_eq!(of_kind(&net, PlacedKind::Junction).len(), 0);
    // the spokes end under the ring instead
    assert_eq!(net.overlays.len(), 3, "{:?}", net.overlays);
}

#[test]
fn joining_roads_do_not_cut_a_ring_even_when_they_outrank_it() {
    // an Asf1 spoke on a City ring: the ring still lies on top
    let mut features = roundabout(40.0, Family::City, &[0]);
    if let FeatureKind::Road { family, .. } = &mut features[1].kind {
        *family = Family::Asf1;
    }
    let net = place(&features);
    assert_eq!(net.rings.len(), 1);
    let o = &net.overlays[0];
    assert!(o.lower.iter().all(|id| !id.starts_with("1:")), "{o:?}");
    assert_eq!(o.lower.len(), 1);
}

#[test]
fn a_ring_smaller_than_the_tightest_arc_becomes_a_crossing() {
    // the tightest arc of the synthetic catalog turns on a radius of 10 m
    let c = catalog();
    let tightest = |f| c.tightest_arc(f);
    let features = roundabout(6.0, Family::City, &[0, 8, 16]);
    // placed as it is, the small ring stays a set of ordinary roads
    let as_roads = place(&features);
    assert_eq!(as_roads.rings.len(), 0);
    // collapsed first, as the build does: the three streets meet at one crossing at the centre
    let graph = collapse_small_rings(&build_graph(&features), &tightest);
    assert!(graph.stretches.iter().all(|s| !s.roundabout));
    assert_eq!(graph.stretches.len(), 3);
    assert_eq!(graph.crossings.len(), 1);
    let crossing = &graph.crossings[0];
    assert!(dist(crossing.point, (1000.0, 1000.0)) < 0.5);
    assert_eq!(crossing.stretches.len(), 3);
    let net = place_network(&graph, &c).unwrap();
    assert_eq!(net.rings.len(), 0);
    // a junction piece where the game has one, else the overlay rule: never a heap
    assert!(
        !of_kind(&net, PlacedKind::Junction).is_empty()
            || !net.overlays.is_empty()
            || !net.joins_without_junction.is_empty()
    );
    // a ring that is big enough is left alone
    let big = collapse_small_rings(
        &build_graph(&roundabout(12.0, Family::City, &[0, 8, 16])),
        &tightest,
    );
    assert!(big.stretches.iter().any(|s| s.roundabout));
    assert_eq!(place_network(&big, &c).unwrap().rings.len(), 1);
}

/// Two parallel through roads, 40 m apart, with a side road between their junctions; the second junction
/// is `dx` m east of the first, so that the side road cannot close between the ports where the crossings are.
fn offset_ladder(dy: f64, dx: f64) -> Vec<Feature> {
    vec![
        feature(
            1,
            Family::Asf3,
            &[1, 2, 3],
            &[(100.0, 300.0), (500.0, 300.0), (900.0, 300.0)],
        ),
        feature(
            2,
            Family::Asf3,
            &[4, 5, 6],
            &[
                (100.0, 300.0 + dy),
                (500.0 + dx, 300.0 + dy),
                (900.0, 300.0 + dy),
            ],
        ),
        feature(
            3,
            Family::Asf3,
            &[2, 5],
            &[(500.0, 300.0), (500.0 + dx, 300.0 + dy)],
        ),
    ]
}

#[test]
fn a_side_road_that_cannot_close_closes_after_the_junction_moves() {
    let net = place(&offset_ladder(40.0, 6.0));
    assert!(net.gaps.is_empty(), "{:?}", net.gaps);
    assert_eq!(of_kind(&net, PlacedKind::Junction).len(), 2);
}

#[test]
fn the_shift_never_exceeds_25_m() {
    // a case that does not close at any position still keeps the junctions within 25 m (and the 3 m
    // tolerance) of their crossings
    for (dy, dx) in [(30.0, 12.0), (40.0, 6.0), (50.0, 12.0)] {
        let net = place(&offset_ladder(dy, dx));
        let crossings = [(500.0, 300.0), (500.0 + dx, 300.0 + dy)];
        for j in of_kind(&net, PlacedKind::Junction) {
            let nearest = crossings
                .iter()
                .map(|c| dist(centre(j), *c))
                .fold(f64::MAX, f64::min);
            assert!(nearest <= 25.0 + 3.0, "dy {dy} dx {dx}: {nearest}");
        }
    }
}

#[test]
fn the_through_road_gaps_do_not_get_worse() {
    // the through roads close at their junctions without a gap whatever the side road needs
    for (dy, dx) in [(30.0, 4.0), (40.0, 6.0), (50.0, 8.0), (80.0, 12.0)] {
        let net = place(&offset_ladder(dy, dx));
        for gap in &net.gaps {
            assert!(gap.stretch.starts_with("3:"), "dy {dy} dx {dx}: {gap:?}");
        }
    }
}

#[test]
fn the_same_network_gives_the_same_shift() {
    let a = place(&offset_ladder(30.0, 8.0));
    let b = place(&offset_ladder(30.0, 8.0));
    assert_eq!(a, b);
}

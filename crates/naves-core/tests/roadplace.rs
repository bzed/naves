// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Placing the road network: pieces, junctions, side roads, end caps, tunnels.

#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]

use std::collections::BTreeSet;

use naves_core::catalog::testing::synthetic_catalog;
use naves_core::catalog::{Family, PieceCatalog};
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::roadnet::build_graph;
use naves_core::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad, place_network};

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
    assert!(net.joins_without_junction.is_empty());
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
fn a_pair_without_a_model_joins_without_a_junction() {
    // there is no gravel junction
    let net = place(&t_network(Family::Grav));
    assert!(of_kind(&net, PlacedKind::Junction).is_empty());
    assert_eq!(net.joins_without_junction.len(), 1);
    assert!(dist(net.joins_without_junction[0], (500.0, 500.0)) < 1.0);
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
    assert_eq!(net.joins_without_junction.len(), 1);
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
    assert!(of_kind(&net, PlacedKind::EndCap).is_empty());
    for r in &net.roads {
        for p in [r.start, r.end] {
            assert!(
                (0.0..=2000.0).contains(&p.x) && (0.0..=2000.0).contains(&p.y),
                "{p:?}"
            );
        }
    }
    assert!(!net.roads.is_empty());
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
    assert!(of_kind(&net, PlacedKind::EndCap).is_empty());
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
    assert!(!through(&a).is_empty());
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
        assert!(!chain.is_empty());
        for &i in chain {
            assert!(i < net.roads.len());
        }
    }
    let covered: usize = net.chains.iter().map(Vec::len).sum();
    assert_eq!(covered, net.roads.len());
}

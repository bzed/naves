// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The lot areas: kinds, placement, area, inside tests.

#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test helper code; failing loudly is the point"
)]

use std::collections::BTreeMap;

use naves_core::heightmap::MapGeo;
use naves_core::lot::{LotKind, Paving, Placement, areas, contains, edge_distance};
use naves_core::osm::{LotPolygon, OsmLand};
use naves_core::project::Project;

fn frame() -> MapGeo {
    let mut project = Project::new("Lots", 47.601, 12.985, 2000.0);
    project.scale.horizontal = 1.0; // a map of 2000 m
    MapGeo::new(&project).unwrap()
}

/// A ring of map points as (lat, lon), closed.
fn ring(frame: &MapGeo, points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut ring: Vec<(f64, f64)> = points
        .iter()
        .map(|&(x, y)| frame.to_geo(x, y).unwrap())
        .collect();
    ring.push(ring[0]);
    ring
}

fn square(frame: &MapGeo, x: f64, y: f64, side: f64) -> Vec<(f64, f64)> {
    ring(
        frame,
        &[(x, y), (x + side, y), (x + side, y + side), (x, y + side)],
    )
}

fn tags(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn lot(id: i64, pairs: &[(&str, &str)], outer: Vec<(f64, f64)>) -> LotPolygon {
    LotPolygon {
        id,
        relation: false,
        part: 0,
        tags: tags(pairs),
        outer,
        holes: Vec::new(),
    }
}

fn land(lots: Vec<LotPolygon>) -> OsmLand {
    OsmLand {
        lots,
        ..OsmLand::default()
    }
}

fn one(pairs: &[(&str, &str)]) -> naves_core::lot::LotArea {
    let f = frame();
    let (found, skipped) = areas(
        &land(vec![lot(7, pairs, square(&f, 1000.0, 1000.0, 40.0))]),
        &f,
    );
    assert_eq!(skipped, 0);
    found.into_iter().next().unwrap()
}

#[test]
fn kinds_come_from_the_tags() {
    for (pairs, kind) in [
        (("amenity", "parking"), LotKind::CarPark),
        (("amenity", "tourist_bus_parking"), LotKind::BusParking),
        (("highway", "rest_area"), LotKind::RestArea),
        (("tourism", "camp_site"), LotKind::CampSite),
        (("tourism", "caravan_site"), LotKind::CaravanSite),
    ] {
        assert_eq!(one(&[pairs]).kind, kind, "{pairs:?}");
    }
}

#[test]
fn placement_comes_from_the_tags() {
    let parking = ("amenity", "parking");
    for value in ["street_side", "lane", "layby"] {
        assert_eq!(
            one(&[parking, ("parking", value)]).placement,
            Placement::StreetSide,
            "{value}"
        );
    }
    for pairs in [
        [parking, ("parking", "underground")],
        [parking, ("parking", "multi-storey")],
        [parking, ("parking", "rooftop")],
        [parking, ("building", "yes")],
        [parking, ("location", "underground")],
        [parking, ("layer", "-1")],
        [parking, ("layer", "1")],
    ] {
        assert_eq!(one(&pairs).placement, Placement::NotAtGround, "{pairs:?}");
    }
    for pairs in [
        vec![parking],
        vec![parking, ("parking", "surface")],
        vec![parking, ("layer", "0")],
        vec![parking, ("building", "no")],
    ] {
        assert_eq!(one(&pairs).placement, Placement::Ground, "{pairs:?}");
    }
}

#[test]
fn a_square_has_its_area_and_a_hole_takes_its_share() {
    let f = frame();
    let mut p = lot(
        7,
        &[("amenity", "parking")],
        square(&f, 1000.0, 1000.0, 40.0),
    );
    let (found, _) = areas(&land(vec![p.clone()]), &f);
    assert!(
        (found[0].area_m2 - 1600.0).abs() <= 1.0,
        "{}",
        found[0].area_m2
    );
    p.holes.push(square(&f, 1015.0, 1015.0, 10.0));
    let (found, _) = areas(&land(vec![p]), &f);
    let a = &found[0];
    assert!((a.area_m2 - 1500.0).abs() <= 1.0, "{}", a.area_m2);
    assert!(!contains(a, (1020.0, 1020.0)), "the hole's centre");
    assert!(contains(a, (1005.0, 1020.0)), "5 m inside the edge");
    assert!(!contains(a, (990.0, 1020.0)), "outside");
}

#[test]
fn a_bow_tie_is_counted_from_samples() {
    let f = frame();
    // two right triangles of 10 x 10 m that meet in (1000, 1000), wound the opposite way round: the shoelace
    // area is 0, the sampled even-odd area is 100
    let outer = ring(
        &f,
        &[
            (1000.0, 1000.0),
            (1010.0, 1000.0),
            (1000.0, 1010.0),
            (1000.0, 1000.0),
            (1000.0, 990.0),
            (990.0, 1000.0),
        ],
    );
    let (found, skipped) = areas(&land(vec![lot(7, &[("amenity", "parking")], outer)]), &f);
    assert_eq!(skipped, 0);
    let a = &found[0];
    assert!((a.area_m2 - 100.0).abs() <= 3.0, "{}", a.area_m2);
    assert!(contains(a, (1003.0, 1003.0)));
    assert!(contains(a, (997.0, 997.0)));
    assert!(!contains(a, (1003.0, 997.0)));
}

#[test]
fn a_ring_that_winds_twice_is_empty_by_even_odd() {
    let f = frame();
    // a 40 m square traced twice: non-zero winding says inside, even-odd says outside, area 0
    let corners = [
        (1000.0, 1000.0),
        (1040.0, 1000.0),
        (1040.0, 1040.0),
        (1000.0, 1040.0),
    ];
    let twice: Vec<(f64, f64)> = corners.iter().chain(corners.iter()).copied().collect();
    let outer = ring(&f, &twice);
    let (found, skipped) = areas(&land(vec![lot(7, &[("amenity", "parking")], outer)]), &f);
    assert_eq!(skipped, 0);
    assert!(found[0].area_m2.abs() < 1e-9, "{}", found[0].area_m2);
    assert!(!contains(&found[0], (1020.0, 1020.0)));
}

/// The sample centres of a 1 m grid inside `area` by `contains`, over the map.
fn contained_samples(area: &naves_core::lot::LotArea) -> f64 {
    let mut n = 0.0;
    for i in 980..1080 {
        for j in 980..1080 {
            if contains(area, (f64::from(i) + 0.5, f64::from(j) + 0.5)) {
                n += 1.0;
            }
        }
    }
    n
}

#[test]
fn the_area_agrees_with_contains_for_a_hole_that_sticks_out() {
    let f = frame();
    let mut p = lot(
        7,
        &[("amenity", "parking")],
        square(&f, 1000.0, 1000.0, 40.0),
    );
    // 20 x 10 m, of which 10 x 10 m lie inside the lot
    p.holes.push(ring(
        &f,
        &[
            (1030.0, 1015.0),
            (1050.0, 1015.0),
            (1050.0, 1025.0),
            (1030.0, 1025.0),
        ],
    ));
    let (found, skipped) = areas(&land(vec![p]), &f);
    assert_eq!(skipped, 0);
    let a = &found[0];
    assert!((a.area_m2 - 1500.0).abs() <= 2.0, "{}", a.area_m2);
    assert!((a.area_m2 - contained_samples(a)).abs() <= 2.0);
}

#[test]
fn the_area_agrees_with_contains_for_overlapping_holes() {
    let f = frame();
    let mut p = lot(
        7,
        &[("amenity", "parking")],
        square(&f, 1000.0, 1000.0, 40.0),
    );
    for x in [1005.0, 1010.0] {
        p.holes.push(ring(
            &f,
            &[
                (x, 1005.0),
                (x + 10.0, 1005.0),
                (x + 10.0, 1015.0),
                (x, 1015.0),
            ],
        ));
    }
    let (found, _) = areas(&land(vec![p]), &f);
    let a = &found[0];
    // the union of the holes is 15 x 10 m
    assert!((a.area_m2 - 1450.0).abs() <= 2.0, "{}", a.area_m2);
    assert!((a.area_m2 - contained_samples(a)).abs() <= 2.0);
}

#[test]
fn a_lot_larger_than_the_map_counts_the_part_on_the_map() {
    let f = frame();
    let outer = ring(
        &f,
        &[
            (-60_000.0, -60_000.0),
            (60_000.0, -60_000.0),
            (60_000.0, 60_000.0),
            (-60_000.0, 60_000.0),
        ],
    );
    let (found, skipped) = areas(&land(vec![lot(7, &[("amenity", "parking")], outer)]), &f);
    assert_eq!(skipped, 0);
    assert!(
        (found[0].area_m2 - 4_000_000.0).abs() <= 2.0,
        "{}",
        found[0].area_m2
    );
}

#[test]
fn absurd_coordinates_end_with_a_result() {
    let f = frame();
    let huge = vec![
        (1e16, 1e16),
        (1e16, 1e16 + 50.0),
        (1e16 + 50.0, 1e16 + 50.0),
        (1e16 + 50.0, 1e16),
        (1e16, 1e16),
    ];
    let (found, skipped) = areas(&land(vec![lot(7, &[("amenity", "parking")], huge)]), &f);
    assert_eq!(found.len() + skipped, 1);
    assert!(
        found
            .iter()
            .all(|a| a.area_m2.is_finite() && a.area_m2 >= 0.0)
    );
}

#[test]
fn a_degenerate_hole_is_dropped_and_the_lot_stays() {
    let f = frame();
    let mut p = lot(
        7,
        &[("amenity", "parking")],
        square(&f, 1000.0, 1000.0, 40.0),
    );
    let a = f.to_geo(1010.0, 1010.0).unwrap();
    let b = f.to_geo(1020.0, 1010.0).unwrap();
    p.holes.push(vec![a, b, a, b, a]);
    let (found, skipped) = areas(&land(vec![p]), &f);
    assert_eq!(skipped, 0, "skipped counts lots");
    assert_eq!(found.len(), 1);
    assert!(
        (found[0].area_m2 - 1600.0).abs() <= 1.0,
        "{}",
        found[0].area_m2
    );
    assert_eq!(found[0].holes.len(), 0);
    assert_eq!(found[0].holes_dropped, 1);
}

#[test]
fn a_hole_with_a_point_that_is_not_finite_skips_the_lot() {
    let f = frame();
    let mut p = lot(
        7,
        &[("amenity", "parking")],
        square(&f, 1000.0, 1000.0, 40.0),
    );
    let a = f.to_geo(1010.0, 1010.0).unwrap();
    p.holes.push(vec![a, (f64::NAN, 12.985), a]);
    let good = lot(
        8,
        &[("amenity", "parking")],
        square(&f, 1100.0, 1000.0, 40.0),
    );
    let (found, skipped) = areas(&land(vec![p, good]), &f);
    assert_eq!(skipped, 1);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "osm/way/8");
}

#[test]
fn a_ring_that_cannot_be_used_is_skipped_and_counted() {
    let f = frame();
    let tags = [("amenity", "parking")];
    let mut nan = square(&f, 1000.0, 1000.0, 40.0);
    nan[1] = (f64::NAN, 12.985);
    let two = {
        let (a, b) = (
            f.to_geo(1000.0, 1000.0).unwrap(),
            f.to_geo(1010.0, 1000.0).unwrap(),
        );
        vec![a, b, a, b, a]
    };
    let good = lot(3, &tags, square(&f, 1000.0, 1000.0, 40.0));
    let (found, skipped) = areas(
        &land(vec![lot(1, &tags, nan), lot(2, &tags, two), good]),
        &f,
    );
    assert_eq!(skipped, 2);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "osm/way/3");
}

#[test]
fn the_edge_distance_of_the_centre_is_half_the_side() {
    let a = one(&[("amenity", "parking")]);
    assert!((edge_distance(&a, (1020.0, 1020.0)) - 20.0).abs() < 1e-6);
    assert!((edge_distance(&a, (1000.0, 1010.0))).abs() < 1e-6);
    assert!((edge_distance(&a, (990.0, 1010.0)) - 10.0).abs() < 1e-6);
}

#[test]
fn the_surface_tag_is_read() {
    let parking = ("amenity", "parking");
    assert_eq!(
        one(&[parking, ("surface", "asphalt")]).surface_tag,
        Some(Paving::Paved)
    );
    assert_eq!(
        one(&[parking, ("surface", "grass")]).surface_tag,
        Some(Paving::Unpaved)
    );
    assert_eq!(one(&[parking, ("surface", "foo")]).surface_tag, None);
    assert_eq!(one(&[parking]).surface_tag, None);
}

#[test]
fn ids_and_order_do_not_depend_on_the_input_order() {
    let f = frame();
    let tags = [("amenity", "parking")];
    let way = |id, x| lot(id, &tags, square(&f, x, 1000.0, 20.0));
    let part = |id, part, x| LotPolygon {
        relation: true,
        part,
        ..lot(id, &tags, square(&f, x, 1200.0, 20.0))
    };
    let lots = vec![
        way(7, 1000.0),
        way(8, 1100.0),
        part(9, 0, 1000.0),
        part(9, 1, 1100.0),
    ];
    let (forward, _) = areas(&land(lots.clone()), &f);
    let mut reversed = lots;
    reversed.reverse();
    let (backward, _) = areas(&land(reversed), &f);
    assert_eq!(forward, backward);
    let ids: Vec<&str> = forward.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "osm/way/7",
            "osm/way/8",
            "osm/relation/9/0",
            "osm/relation/9/1"
        ]
    );
}

#[test]
fn a_lot_on_a_map_is_not_changed_by_its_corner_order() {
    // winding does not matter
    let f = frame();
    let mut outer = square(&f, 1000.0, 1000.0, 40.0);
    let (a, _) = areas(
        &land(vec![lot(1, &[("amenity", "parking")], outer.clone())]),
        &f,
    );
    outer.reverse();
    let (b, _) = areas(&land(vec![lot(1, &[("amenity", "parking")], outer)]), &f);
    assert!((a[0].area_m2 - b[0].area_m2).abs() < 1e-9);
}

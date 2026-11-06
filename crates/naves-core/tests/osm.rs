// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Behaviour of the OpenStreetMap land polygon reader.

use std::collections::BTreeMap;
use std::path::PathBuf;

use naves_core::landcover::GeoWindow;
use naves_core::osm::{LandPolygon, RawWay, assemble, read_land, read_land_from_bytes};
use naves_core::surface::SurfaceClass;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/osm/koenigssee.osm.pbf")
}

fn koenigssee_window() -> GeoWindow {
    GeoWindow {
        south: 47.54,
        west: 12.96,
        north: 47.56,
        east: 12.99,
    }
}

fn way(nodes: &[i64]) -> RawWay {
    RawWay {
        nodes: nodes.to_vec(),
        inner: false,
    }
}

fn inner(nodes: &[i64]) -> RawWay {
    RawWay {
        nodes: nodes.to_vec(),
        inner: true,
    }
}

/// Nodes 1 to 4 are a square of 10 degrees, 5 to 8 a small square inside it, 9 and 10 sit on the outer edge.
fn nodes() -> BTreeMap<i64, (f64, f64)> {
    BTreeMap::from([
        (1, (0.0, 0.0)),
        (2, (0.0, 10.0)),
        (3, (10.0, 10.0)),
        (4, (10.0, 0.0)),
        (5, (4.0, 4.0)),
        (6, (4.0, 6.0)),
        (7, (6.0, 6.0)),
        (8, (6.0, 4.0)),
        (9, (0.0, 5.0)),
        (10, (5.0, 0.0)),
    ])
}

#[test]
fn way_rings_close() {
    let rings = assemble(&[way(&[1, 2, 3, 4, 1])], &nodes());
    assert_eq!(rings.skipped, 0);
    assert_eq!(rings.polygons.len(), 1);
    let (outer, holes) = &rings.polygons[0];
    assert_eq!(outer.len(), 5);
    assert_eq!(outer.first(), outer.last());
    assert!(holes.is_empty());
}

#[test]
fn split_outer_ring_is_joined() {
    // three ways, one of them reversed, given out of order: 1-9-2-3 | 4-3 | 4-10-1
    let ways = [way(&[4, 10, 1]), way(&[1, 9, 2, 3]), way(&[4, 3])];
    let rings = assemble(&ways, &nodes());
    assert_eq!(rings.skipped, 0);
    assert_eq!(rings.polygons.len(), 1);
    let (outer, _) = &rings.polygons[0];
    assert_eq!(
        outer.len(),
        7,
        "six corners and the closing point: {outer:?}"
    );
    assert_eq!(outer.first(), outer.last());
}

#[test]
fn inner_ring_becomes_a_hole() {
    let ways = [way(&[1, 2, 3, 4, 1]), inner(&[5, 6, 7, 8, 5])];
    let rings = assemble(&ways, &nodes());
    assert_eq!(rings.skipped, 0);
    assert_eq!(rings.polygons.len(), 1);
    let (outer, holes) = &rings.polygons[0];
    assert_eq!(outer.len(), 5);
    assert_eq!(holes.len(), 1);
    assert_eq!(holes[0].len(), 5);
}

#[test]
fn an_inner_ring_outside_every_outer_ring_is_skipped_and_counted() {
    let far: BTreeMap<_, _> = nodes()
        .into_iter()
        .chain([(21, (50.0, 50.0)), (22, (50.0, 51.0)), (23, (51.0, 51.0))])
        .collect();
    let ways = [way(&[1, 2, 3, 4, 1]), inner(&[21, 22, 23, 21])];
    let rings = assemble(&ways, &far);
    assert_eq!(rings.polygons.len(), 1);
    assert!(rings.polygons[0].1.is_empty());
    assert_eq!(rings.skipped, 1);
}

#[test]
fn unclosed_ring_is_skipped_and_counted() {
    // what an extract that cuts a lake leaves behind: a way that never comes back to its start
    let rings = assemble(&[way(&[1, 2, 3])], &nodes());
    assert!(rings.polygons.is_empty());
    assert_eq!(rings.skipped, 1);
}

#[test]
fn missing_node_is_skipped_and_counted() {
    let mut known = nodes();
    known.remove(&3);
    let rings = assemble(&[way(&[1, 2, 3, 4, 1])], &known);
    assert!(rings.polygons.is_empty());
    assert_eq!(rings.skipped, 1);
}

#[test]
fn a_ring_of_fewer_than_three_corners_is_skipped_and_counted() {
    let rings = assemble(&[way(&[1, 2, 1])], &nodes());
    assert!(rings.polygons.is_empty());
    assert_eq!(rings.skipped, 1);
}

/// Even-odd test of a (lat, lon) point against a polygon with holes.
fn contains(polygon: &LandPolygon, lat: f64, lon: f64) -> bool {
    let crossings = |ring: &[(f64, f64)]| {
        let mut inside = false;
        for pair in ring.windows(2) {
            let [(lat_a, lon_a), (lat_b, lon_b)] = *pair else {
                continue;
            };
            if (lat_a > lat) != (lat_b > lat) {
                let at = lon_a + (lat - lat_a) / (lat_b - lat_a) * (lon_b - lon_a);
                if lon < at {
                    inside = !inside;
                }
            }
        }
        inside
    };
    crossings(&polygon.outer) && !polygon.holes.iter().any(|h| crossings(h))
}

#[test]
fn fixture_has_the_koenigssee() {
    let land = read_land(&[fixture()], koenigssee_window()).unwrap();
    let lake: Vec<_> = land
        .polygons
        .iter()
        .filter(|p| contains(p, 47.5534, 12.9765))
        .collect();
    assert_eq!(lake.len(), 1, "polygons at the lake: {}", lake.len());
    assert_eq!(lake[0].class, SurfaceClass::Soil);
    assert_eq!(lake[0].priority, 3, "water");
}

#[test]
fn irrelevant_tags_are_ignored() {
    // the fixture also holds natural=wood and landuse=forest areas: they are not read
    let land = read_land(&[fixture()], koenigssee_window()).unwrap();
    assert!(!land.polygons.is_empty());
    assert!(
        land.polygons
            .iter()
            .all(|p| matches!(p.class, SurfaceClass::Soil | SurfaceClass::Rock)),
        "only water, rock and built-up areas are read"
    );
}

#[test]
fn elements_outside_the_window_are_dropped() {
    let far = GeoWindow {
        south: 10.0,
        west: 10.0,
        north: 10.1,
        east: 10.1,
    };
    let land = read_land(&[fixture()], far).unwrap();
    assert!(land.polygons.is_empty());
}

#[test]
fn result_does_not_depend_on_order() {
    let a = read_land(&[fixture()], koenigssee_window()).unwrap();
    let b = read_land(&[fixture()], koenigssee_window()).unwrap();
    assert_eq!(a, b);
    let bytes = std::fs::read(fixture()).unwrap();
    assert_eq!(
        read_land_from_bytes(&bytes, koenigssee_window()).unwrap(),
        a
    );
}

#[test]
fn a_missing_file_names_itself() {
    let err = read_land(
        &[PathBuf::from("/nonexistent/x.osm.pbf")],
        koenigssee_window(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("x.osm.pbf"), "{err}");
}

#[test]
fn damaged_files_are_errors_not_panics() {
    let good = std::fs::read(fixture()).unwrap();
    assert!(read_land_from_bytes(b"not a pbf", koenigssee_window()).is_err());
    assert!(read_land_from_bytes(&good[..good.len() / 2], koenigssee_window()).is_err());
    // flip bytes all over the file; every outcome is Ok or Err
    for step in [7_usize, 13, 101] {
        let mut bytes = good.clone();
        for i in (0..bytes.len()).step_by(step) {
            bytes[i] ^= 0xa5;
        }
        let _ = read_land_from_bytes(&bytes, koenigssee_window());
    }
}

#[test]
#[ignore = "needs a large OSM extract: set NAVES_BIG_OSM to a .osm.pbf"]
fn a_regional_extract_is_read() {
    let path = PathBuf::from(std::env::var("NAVES_BIG_OSM").unwrap());
    let land = read_land(&[path], koenigssee_window()).unwrap();
    assert!(land.polygons.iter().any(|p| contains(p, 47.5534, 12.9765)));
    eprintln!(
        "{} polygons, {} skipped rings",
        land.polygons.len(),
        land.skipped_rings
    );
}

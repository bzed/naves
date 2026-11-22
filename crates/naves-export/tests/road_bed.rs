// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The road bed is painted under the pieces that were placed.

use naves_core::catalog::Family;
use naves_core::roadfit::Pose;
use naves_core::roadnet::{RoadGraph, Stretch};
use naves_core::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad, Slot};
use naves_core::surface::{SurfaceClass, SurfaceGrid};
use naves_export::generate::paint_roads;

fn grass() -> SurfaceGrid {
    SurfaceGrid::from_fn(100, 5.0, |_, _| SurfaceClass::Grass)
}

fn stretch(id: &str, a: (f64, f64), b: (f64, f64), bridge: bool, tunnel: bool) -> Stretch {
    Stretch {
        id: id.to_string(),
        family: Family::Asf3,
        drivable: true,
        bridge,
        tunnel,
        roundabout: false,
        layer: 0,
        points: vec![a, b],
        start: 1,
        end: 2,
        bridge_kind: None,
        bridge_offset: 0.0,
        use_sidepath: false,
        paved_m: 0.0,
        unpaved_m: 0.0,
    }
}

fn piece(stretch: &str, kind: PlacedKind, a: (f64, f64), b: (f64, f64)) -> PlacedRoad {
    let pose = |p: (f64, f64)| Pose {
        x: p.0,
        y: p.1,
        heading: 0.0,
    };
    PlacedRoad {
        slot: Slot {
            stretch: stretch.to_string(),
            index: 0,
        },
        kind,
        family: Family::Asf3,
        drivable: true,
        model: "m".to_string(),
        origin: pose(a),
        start: pose(a),
        end: pose(b),
        width: 7.0,
        scale: 1.0,
        abreast: 1,
    }
}

fn graph(stretches: Vec<Stretch>) -> RoadGraph {
    RoadGraph {
        stretches,
        crossings: Vec::new(),
    }
}

fn network(roads: Vec<PlacedRoad>) -> PlacedNetwork {
    PlacedNetwork {
        roads,
        ..PlacedNetwork::default()
    }
}

#[test]
fn the_bed_lies_under_the_placed_pieces_not_under_the_osm_line() {
    // the OSM line runs along y = 100; the fit put the piece 20 m north of it
    let mut surfaces = grass();
    paint_roads(
        &mut surfaces,
        &graph(vec![stretch(
            "1:1-2",
            (50.0, 100.0),
            (150.0, 100.0),
            false,
            false,
        )]),
        &network(vec![piece(
            "1:1-2",
            PlacedKind::Piece,
            (50.0, 120.0),
            (150.0, 120.0),
        )]),
        None,
    );
    assert_eq!(surfaces.at(100.0, 120.0), SurfaceClass::Soil);
    assert_eq!(surfaces.at(100.0, 123.0), SurfaceClass::Soil);
    assert_eq!(surfaces.at(100.0, 100.0), SurfaceClass::Grass);
}

#[test]
fn a_side_road_arm_at_a_right_angle_has_its_bed() {
    let mut surfaces = grass();
    paint_roads(
        &mut surfaces,
        &graph(vec![stretch(
            "1:1-2",
            (50.0, 100.0),
            (150.0, 100.0),
            false,
            false,
        )]),
        &network(vec![piece(
            "1:1-2",
            PlacedKind::Piece,
            (100.0, 100.0),
            (100.0, 150.0),
        )]),
        None,
    );
    assert_eq!(surfaces.at(100.0, 140.0), SurfaceClass::Soil);
}

#[test]
fn a_short_bridge_placed_as_road_pieces_gets_a_bed_but_a_bridge_model_does_not() {
    let mut surfaces = grass();
    paint_roads(
        &mut surfaces,
        &graph(vec![
            stretch("1:1-2", (50.0, 100.0), (150.0, 100.0), true, false),
            stretch("2:3-4", (50.0, 300.0), (150.0, 300.0), true, false),
        ]),
        &network(vec![
            piece("1:1-2", PlacedKind::Piece, (50.0, 100.0), (150.0, 100.0)),
            piece("2:3-4", PlacedKind::Bridge, (50.0, 300.0), (150.0, 300.0)),
        ]),
        None,
    );
    assert_eq!(surfaces.at(100.0, 100.0), SurfaceClass::Soil);
    assert_eq!(surfaces.at(100.0, 300.0), SurfaceClass::Grass);
}

#[test]
fn what_nothing_was_placed_for_is_painted_along_the_osm_line() {
    let mut surfaces = grass();
    paint_roads(
        &mut surfaces,
        &graph(vec![
            stretch("1:1-2", (50.0, 100.0), (150.0, 100.0), false, false),
            stretch("2:3-4", (50.0, 300.0), (150.0, 300.0), false, true),
            stretch("3:5-6", (50.0, 400.0), (150.0, 400.0), true, false),
        ]),
        &network(Vec::new()),
        None,
    );
    assert_eq!(surfaces.at(100.0, 100.0), SurfaceClass::Soil);
    // a tunnel and a bridge that was not placed get nothing
    assert_eq!(surfaces.at(100.0, 300.0), SurfaceClass::Grass);
    assert_eq!(surfaces.at(100.0, 400.0), SurfaceClass::Grass);
}

// ---- lots and the paths inside them ----

use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::lot::{LotArea, LotKind, Placement};
use naves_export::generate::{paint_lots, paint_paths_with};

fn area(id: &str, kind: LotKind, outer: Vec<(f64, f64)>) -> LotArea {
    let (x0, y0) = outer.first().copied().unwrap_or_default();
    let (x1, y1) = outer.get(2).copied().unwrap_or_default();
    LotArea {
        id: id.to_string(),
        kind,
        placement: Placement::Ground,
        outer,
        holes: Vec::new(),
        area_m2: ((x1 - x0) * (y1 - y0)).abs(),
        surface_tag: None,
        holes_dropped: 0,
    }
}

fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<(f64, f64)> {
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]
}

fn path(id: i64, points: &[(f64, f64)], street: bool) -> Feature {
    Feature {
        id: format!("osm/way/{id}/0"),
        way_id: id,
        source: Source::Osm,
        kind: FeatureKind::Path {
            width_m: 3.0,
            street,
        },
        points: points.to_vec(),
        nodes: (0..100).take(points.len()).map(|i| id * 100 + i).collect(),
    }
}

#[test]
fn a_car_park_inside_a_camp_site_shows_on_top_of_it() {
    let camp = area(
        "osm/way/1",
        LotKind::CampSite,
        square(100.0, 100.0, 300.0, 300.0),
    );
    let park = area(
        "osm/way/2",
        LotKind::CarPark,
        square(150.0, 150.0, 200.0, 200.0),
    );
    let decided = vec![
        ("osm/way/1".to_string(), SurfaceClass::Grass),
        ("osm/way/2".to_string(), SurfaceClass::Tarmac),
    ];
    // the order of the lists does not matter: the larger lot is painted first
    for areas in [[camp.clone(), park.clone()], [park, camp]] {
        let mut surfaces = SurfaceGrid::from_fn(100, 5.0, |_, _| SurfaceClass::Forest);
        paint_lots(&mut surfaces, &areas, &decided);
        assert_eq!(surfaces.at(175.0, 175.0), SurfaceClass::Tarmac, "car park");
        assert_eq!(surfaces.at(250.0, 250.0), SurfaceClass::Grass, "camp site");
        assert_eq!(surfaces.at(50.0, 50.0), SurfaceClass::Forest, "outside");
    }
}

#[test]
fn a_lot_without_a_decision_is_not_painted() {
    let park = area(
        "osm/way/2",
        LotKind::CarPark,
        square(150.0, 150.0, 200.0, 200.0),
    );
    let mut surfaces = grass();
    paint_lots(&mut surfaces, &[park], &[]);
    assert_eq!(surfaces.at(175.0, 175.0), SurfaceClass::Grass);
    // a decision for a lot that is not in the list is no error
    let mut surfaces = grass();
    paint_lots(
        &mut surfaces,
        &[],
        &[("osm/way/7".to_string(), SurfaceClass::Tarmac)],
    );
    assert_eq!(surfaces.at(175.0, 175.0), SurfaceClass::Grass);
}

#[test]
fn lots_with_a_hole_leave_it_alone() {
    let mut park = area(
        "osm/way/2",
        LotKind::CarPark,
        square(100.0, 100.0, 200.0, 200.0),
    );
    park.holes.push(square(140.0, 140.0, 160.0, 160.0));
    let mut surfaces = grass();
    paint_lots(
        &mut surfaces,
        &[park],
        &[("osm/way/2".to_string(), SurfaceClass::Tarmac)],
    );
    assert_eq!(surfaces.at(120.0, 120.0), SurfaceClass::Tarmac);
    assert_eq!(surfaces.at(150.0, 150.0), SurfaceClass::Grass, "the hole");
}

#[test]
fn a_parking_aisle_in_a_lot_is_not_painted_but_a_footway_through_it_is() {
    let park = area(
        "osm/way/2",
        LotKind::CarPark,
        square(100.0, 100.0, 300.0, 300.0),
    );
    let decided = vec![("osm/way/2".to_string(), SurfaceClass::Tarmac)];
    let aisle = path(10, &[(120.0, 150.0), (280.0, 150.0)], true);
    let footway = path(11, &[(120.0, 250.0), (280.0, 250.0)], false);
    // a driveway that runs outside the lot is painted as before
    let driveway = path(12, &[(120.0, 400.0), (280.0, 400.0)], true);
    let mut surfaces = grass();
    paint_lots(&mut surfaces, std::slice::from_ref(&park), &decided);
    paint_paths_with(&mut surfaces, &[aisle, footway, driveway], &[park]);
    assert_eq!(surfaces.at(200.0, 150.0), SurfaceClass::Tarmac, "aisle");
    assert_eq!(surfaces.at(200.0, 250.0), SurfaceClass::Soil, "footway");
    assert_eq!(surfaces.at(200.0, 400.0), SurfaceClass::Soil, "driveway");
}

#[test]
fn paths_are_painted_as_before_without_lots() {
    let aisle = path(10, &[(120.0, 150.0), (280.0, 150.0)], true);
    let mut surfaces = grass();
    paint_paths_with(&mut surfaces, &[aisle], &[]);
    assert_eq!(surfaces.at(200.0, 150.0), SurfaceClass::Soil);
}

#[test]
fn a_street_path_is_skipped_only_if_the_whole_line_lies_in_one_lot() {
    // an L-shaped lot: x 100..300, y 100..150 and x 100..150, y 100..300; the inner corner is open ground
    let l_shape = LotArea {
        outer: vec![
            (100.0, 100.0),
            (300.0, 100.0),
            (300.0, 150.0),
            (150.0, 150.0),
            (150.0, 300.0),
            (100.0, 300.0),
            (100.0, 100.0),
        ],
        ..area("osm/way/1", LotKind::CarPark, square(0.0, 0.0, 1.0, 1.0))
    };
    // the ends are inside the lot, the line cuts across the open inner corner
    let cutting = path(10, &[(280.0, 120.0), (120.0, 280.0)], true);
    // a line inside the lot all the way
    let inside = path(11, &[(110.0, 110.0), (290.0, 110.0)], true);
    let mut surfaces = grass();
    paint_paths_with(&mut surfaces, &[cutting, inside], &[l_shape]);
    assert_eq!(surfaces.at(198.0, 202.0), SurfaceClass::Soil, "the corner");
    assert_eq!(surfaces.at(200.0, 110.0), SurfaceClass::Grass, "inside");
}

#[test]
fn a_street_path_between_two_lots_is_painted() {
    let one = area(
        "osm/way/1",
        LotKind::CarPark,
        square(100.0, 100.0, 150.0, 150.0),
    );
    let two = area(
        "osm/way/2",
        LotKind::CarPark,
        square(250.0, 100.0, 300.0, 150.0),
    );
    let across = path(10, &[(125.0, 125.0), (275.0, 125.0)], true);
    let mut surfaces = grass();
    paint_paths_with(&mut surfaces, &[across], &[one, two]);
    assert_eq!(surfaces.at(200.0, 125.0), SurfaceClass::Soil);
}

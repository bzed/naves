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
        points: vec![a, b],
        start: 1,
        end: 2,
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

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A bridge over a road keeps its clearance through the placement, and a bridge model whose deck thickness
//! cannot be read is said so.

use naves_core::catalog::testing::synthetic_models;
use naves_core::catalog::{Family, PieceCatalog};
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::heightmap::Heightmap;
use naves_core::roadnet::build_graph;
use naves_export::generate::place_once;

fn way(id: i64, nodes: &[i64], points: &[(f64, f64)], bridge: bool) -> Feature {
    Feature {
        id: format!("osm/way/{id}/0"),
        way_id: id,
        source: Source::Osm,
        kind: FeatureKind::Road {
            family: Family::Asf3,
            drivable: true,
            bridge,
            tunnel: false,
            roundabout: false,
            layer: i8::from(bridge),
        },
        points: points.to_vec(),
        nodes: nodes.to_vec(),
    }
}

fn flat() -> Heightmap {
    Heightmap {
        size: 250,
        cell_m: 4.0,
        h: vec![100.0; 250 * 250],
    }
}

fn graph() -> naves_core::roadnet::RoadGraph {
    build_graph(&[
        way(1, &[1, 2], &[(50.0, 500.0), (550.0, 500.0)], false),
        way(3, &[3, 4], &[(300.0, 100.0), (300.0, 487.5)], false),
        way(4, &[4, 5], &[(300.0, 487.5), (300.0, 512.5)], true),
        way(5, &[5, 6], &[(300.0, 512.5), (300.0, 900.0)], false),
    ])
}

#[test]
fn a_bridge_over_a_road_clears_it_through_the_placement() {
    let catalog = PieceCatalog::from_models(&synthetic_models()).unwrap();
    let placed = place_once(&graph(), &catalog, &flat()).unwrap();
    assert_eq!(placed.profiles.clearances.len(), 1);
    assert!(
        placed.profiles.clearances[0].reached >= 4.45,
        "{:?}",
        placed.profiles.clearances
    );
    assert!(
        placed
            .net
            .warnings
            .iter()
            .all(|w| !w.contains("Geometry LOD")),
        "{:?}",
        placed.net.warnings
    );
}

#[test]
fn an_unreadable_deck_thickness_is_borrowed_and_named_in_a_warning() {
    let mut models = synthetic_models();
    // the stone bridges cannot be read, the wooden one can
    for (path, m) in &mut models {
        if path.contains("stone") {
            m.geometry = None;
        }
    }
    let catalog = PieceCatalog::from_models(&models).unwrap();
    let placed = place_once(&graph(), &catalog, &flat()).unwrap();
    assert!(
        placed
            .net
            .warnings
            .iter()
            .any(|w| w.contains("bridge_stone") && w.contains("thickest deck of all bridge models")),
        "{:?}",
        placed.net.warnings
    );
    assert!(
        placed.profiles.clearances[0].reached >= 4.45,
        "{:?}",
        placed.profiles.clearances
    );
}

#[test]
fn with_no_readable_deck_the_clearance_is_not_checked_and_the_build_says_so() {
    let mut models = synthetic_models();
    for m in models.values_mut() {
        m.geometry = None;
    }
    let catalog = PieceCatalog::from_models(&models).unwrap();
    let placed = place_once(&graph(), &catalog, &flat()).unwrap();
    assert!(
        placed
            .net
            .warnings
            .iter()
            .any(|w| w.contains("not checked")),
        "{:?}",
        placed.net.warnings
    );
    assert_eq!(placed.profiles.clearances.len(), 0);
}

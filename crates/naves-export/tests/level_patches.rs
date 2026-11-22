// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The level patches under junctions without a junction piece.

use std::collections::BTreeSet;

use naves_core::catalog::testing::synthetic_models;
use naves_core::catalog::{Family, PieceCatalog};
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::heightmap::Heightmap;
use naves_core::junctions::{Junction, JunctionKind, Junctions, RingPlane};
use naves_core::lot::{Form, LotSurface, Surface};
use naves_core::netclean::CutAccess;
use naves_core::roadnet::build_graph;
use naves_export::generate::{Placed, area_patches, level_patches, place_once, place_once_with};

fn junction(kind: JunctionKind, at: (f64, f64), height: f64) -> Junction {
    Junction {
        at,
        kind,
        ground: height,
        height,
        rank: 4,
        patch_m: 5.0,
        stretches: BTreeSet::new(),
        lower: BTreeSet::new(),
        gradient: (0.0, 0.0),
        tilt_axis: None,
        tilt_cap: 0.0,
    }
}

#[test]
fn a_join_an_overlay_and_a_chain_end_on_a_road_get_a_patch_one_cell_wider_and_a_piece_gets_none() {
    let junctions = Junctions {
        points: vec![
            junction(JunctionKind::Join, (10.0, 20.0), 101.0),
            junction(JunctionKind::Piece, (30.0, 40.0), 102.0),
            junction(JunctionKind::EndOnRoad, (50.0, 60.0), 103.0),
            junction(JunctionKind::Overlay, (70.0, 80.0), 104.0),
        ],
        ..Junctions::default()
    };
    let patches = level_patches(&junctions, 3.125);
    assert_eq!(patches.len(), 3);
    assert_eq!(patches[0].at, (10.0, 20.0));
    assert!((patches[0].radius - 8.125).abs() < 1e-9);
    assert!((patches[0].height - 101.0).abs() < 1e-9);
    assert_eq!(patches[0].rank, 4);
    assert_eq!(patches[1].at, (50.0, 60.0));
    // an overlay is levelled like a join
    assert_eq!(patches[2].at, (70.0, 80.0));
    assert!((patches[2].height - 104.0).abs() < 1e-9);
}

#[test]
fn a_ring_gets_one_tilted_patch_and_the_junctions_on_it_none() {
    let ring_ids: BTreeSet<String> = ["1:100-112".to_string()].into();
    let mut on_ring = junction(JunctionKind::Overlay, (400.0, 430.0), 124.0);
    on_ring.stretches = ring_ids.clone();
    let junctions = Junctions {
        points: vec![on_ring, junction(JunctionKind::Join, (10.0, 20.0), 101.0)],
        rings: vec![RingPlane {
            at: (400.0, 400.0),
            radius: 33.5,
            inner_radius: 26.5,
            height: 123.2,
            gradient: (0.04, 0.02),
            rank: 3,
            stretches: ring_ids,
        }],
        ..Junctions::default()
    };
    let patches = level_patches(&junctions, 3.125);
    assert_eq!(patches.len(), 2);
    assert_eq!(patches[0].at, (10.0, 20.0));
    assert!((patches[0].gradient.0).abs() < 1e-12 && (patches[0].gradient.1).abs() < 1e-12);
    assert_eq!(patches[1].at, (400.0, 400.0));
    assert!((patches[1].radius - 36.625).abs() < 1e-9);
    // the band only: the inside of the ring is not levelled
    assert!((patches[1].inner_radius - 23.375).abs() < 1e-9);
    assert!((patches[1].height - 123.2).abs() < 1e-9);
    assert!((patches[1].gradient.0 - 0.04).abs() < 1e-12);
    assert!((patches[1].gradient.1 - 0.02).abs() < 1e-12);
    assert_eq!(patches[1].rank, 3);
}

#[test]
fn a_tilted_junction_patch_shapes_a_plane() {
    use naves_core::conform::conform;
    use naves_core::heightmap::Heightmap;
    // a 10 % slope to the east; an end-on-road junction tilted with it
    let size = 64_u32;
    let cell = 3.125_f32;
    let ground = |x: f64| 100.0 + 0.10 * x;
    let base = Heightmap {
        size,
        cell_m: cell,
        h: (0..size * size)
            .map(|i| {
                #[expect(clippy::cast_possible_truncation, reason = "a test height")]
                let h = ground(f64::from(i % size) * f64::from(cell)) as f32;
                h
            })
            .collect(),
    };
    let mut j = junction(JunctionKind::EndOnRoad, (100.0, 100.0), ground(100.0));
    j.gradient = (0.10, 0.0);
    let junctions = Junctions {
        points: vec![j],
        ..Junctions::default()
    };
    let patches = level_patches(&junctions, f64::from(cell));
    assert_eq!(patches.len(), 1);
    assert!((patches[0].gradient.0 - 0.10).abs() < 1e-12);
    let out = conform(&base, &[], &patches).unwrap();
    let radius = patches[0].radius;
    let mut checked = 0;
    for row in 0..size {
        for col in 0..size {
            let (x, y) = (
                f64::from(col) * f64::from(cell),
                f64::from(row) * f64::from(cell),
            );
            if (x - 100.0).hypot(y - 100.0) <= radius {
                let i = usize::try_from(row * size + col).unwrap();
                let (got, want) = (f64::from(out.heightmap.h[i]), ground(x));
                assert!((got - want).abs() < 0.02, "({x}, {y}): {got} vs {want}");
                checked += 1;
            }
        }
    }
    assert!(checked > 20);
}

// ----- lots -----

fn way(id: i64, nodes: &[i64], points: &[(f64, f64)]) -> Feature {
    Feature {
        id: format!("osm/way/{id}/0"),
        way_id: id,
        source: Source::Osm,
        kind: FeatureKind::Road {
            family: Family::City,
            drivable: true,
            bridge: false,
            tunnel: false,
            roundabout: false,
            layer: 0,
            use_sidepath: false,
            paving: None,
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

/// The bridge-over-a-road graph of `bridge_clearance.rs`.
fn bridge_graph() -> naves_core::roadnet::RoadGraph {
    build_graph(&[
        way(1, &[1, 2], &[(50.0, 500.0), (550.0, 500.0)]),
        way(3, &[3, 4], &[(300.0, 100.0), (300.0, 487.5)]),
        way(4, &[4, 5], &[(300.0, 487.5), (300.0, 512.5)]),
        way(5, &[5, 6], &[(300.0, 512.5), (300.0, 900.0)]),
    ])
}

/// The heights, tilts and side slopes bit for bit, and the shapes they belong to.
fn bits(placed: &Placed) -> Vec<u64> {
    let mut out = Vec::new();
    for strip in &placed.profiles.strips {
        out.push(u64::try_from(strip.joints.len()).unwrap_or(u64::MAX));
        for joint in &strip.joints {
            out.push(joint.0.to_bits());
            out.push(joint.1.to_bits());
            out.push(joint.2.to_bits());
        }
    }
    out.push(u64::try_from(placed.junctions.points.len()).unwrap_or(u64::MAX));
    for junction in &placed.junctions.points {
        out.push(junction.at.0.to_bits());
        out.push(junction.at.1.to_bits());
        out.push(junction.height.to_bits());
        out.push(junction.gradient.0.to_bits());
        out.push(junction.gradient.1.to_bits());
    }
    for slope in &placed.side {
        out.push(slope.to_bits());
    }
    out
}

/// A lot's surface as a level plane at `height` through `at`.
fn lot_surface(outer: Vec<(f64, f64)>, at: (f64, f64), height: f64) -> LotSurface {
    LotSurface {
        group: 0,
        members: vec!["osm/way/1".to_string()],
        polygons: vec![(outer, Vec::new())],
        surface: Surface {
            at,
            coeffs: [height, 0.0, 0.0, 0.0, 0.0, 0.0],
            form: Form::Plane,
            lambda: 1.0,
        },
    }
}

#[test]
fn a_lot_gives_one_area_patch_of_rank_zero() {
    let lot = lot_surface(
        vec![
            (100.0, 100.0),
            (200.0, 100.0),
            (200.0, 200.0),
            (100.0, 200.0),
            (100.0, 100.0),
        ],
        (150.0, 150.0),
        101.0,
    );
    let patches = area_patches(std::slice::from_ref(&lot));
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].rank, 0);
    assert_eq!(patches[0].surface, lot);
}

#[test]
fn placing_without_lots_is_placing_bit_for_bit() {
    let catalog = PieceCatalog::from_models(&synthetic_models()).unwrap();
    let graph = bridge_graph();
    let base = flat();
    let plain = place_once(&graph, &catalog, &base).unwrap();
    let with = place_once_with(&graph, &catalog, &base, &[], &[]).unwrap();
    assert_eq!(bits(&plain), bits(&with));
    assert_eq!(plain.junctions, with.junctions);
    assert_eq!(plain.profiles, with.profiles);
}

#[test]
fn an_entry_of_a_lot_without_a_surface_changes_nothing() {
    let catalog = PieceCatalog::from_models(&synthetic_models()).unwrap();
    let graph = bridge_graph();
    let base = flat();
    let stretch = graph.stretches[0].id.clone();
    let entry = CutAccess {
        stretch: stretch.clone(),
        lot: "osm/way/9".to_string(),
        cut_m: 4.0,
        entry: (550.0, 500.0),
        node: 900,
    };
    let plain = place_once(&graph, &catalog, &base).unwrap();
    // the lot is not smoothed (it has no surface), so the entry makes no junction
    let with = place_once_with(&graph, &catalog, &base, &[], &[entry]).unwrap();
    assert_eq!(bits(&plain), bits(&with));
    assert!(
        with.junctions
            .points
            .iter()
            .all(|j| j.kind != JunctionKind::LotEntry)
    );
}

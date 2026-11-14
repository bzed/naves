// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tests for the catalog of vanilla road pieces. The models are synthetic (see
//! `naves_core::catalog::testing`); no game data is used.

#![expect(
    clippy::panic,
    reason = "test helper code; failing loudly is the point"
)]

use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI};

use naves_core::catalog::testing::{synthetic_catalog, synthetic_models};
use naves_core::catalog::{CatalogError, Family, PieceCatalog, PieceKind};
use naves_formats::odol::OdolModel;

const PARTS: &str = "dz\\structures\\roads\\parts\\";

fn model(name: &str) -> String {
    format!("{PARTS}{name}.p3d")
}

fn empty_model(points: &[(&str, [f32; 3])]) -> OdolModel {
    OdolModel {
        version: 56,
        bbox_min: [0.0; 3],
        bbox_max: [0.0; 3],
        memory: points
            .iter()
            .map(|(k, p)| ((*k).to_string(), vec![*p]))
            .collect(),
        properties: BTreeMap::new(),
    }
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn piece<'a>(
    c: &'a PieceCatalog,
    family: Family,
    name: &str,
) -> &'a naves_core::catalog::RoadPiece {
    let wanted = model(name);
    c.pieces(family)
        .iter()
        .find(|p| p.model == wanted)
        .unwrap_or_else(|| panic!("no piece {wanted}"))
}

#[test]
fn arc_angles_come_from_the_models() {
    let c = synthetic_catalog();
    let arc = piece(&c, Family::Asf3, "asf3_10 25");
    assert_eq!(arc.kind, PieceKind::Arc);
    assert!(near(arc.heading_change().to_degrees(), 10.0, 1e-4));
    assert!(near(arc.chord(), 4.358, 0.001));
    assert!(near(arc.width, 7.0, 1e-6));

    // the angle is read from the points, not from the "10" in the name
    let mut models = synthetic_models();
    let key = model("asf3_10 25");
    let turned = models["dz\\structures\\roads\\parts\\asf3_30 25.p3d"].clone();
    models.insert(key, turned);
    let c = PieceCatalog::from_models(&models).unwrap();
    let arc = piece(&c, Family::Asf3, "asf3_10 25");
    assert!(near(arc.heading_change().to_degrees(), 30.0, 1e-4));
}

#[test]
fn used_backwards_a_piece_turns_the_other_way() {
    let c = synthetic_catalog();
    let arc = piece(&c, Family::Asf3, "asf3_10 25");
    let back = arc.reversed();
    assert!(near(back.heading_change(), -arc.heading_change(), 1e-9));
    assert!(near(back.start.x, arc.end.x, 1e-12) && near(back.start.z, arc.end.z, 1e-12));
    assert!(near(back.end.x, arc.start.x, 1e-12) && near(back.end.z, arc.start.z, 1e-12));
    // both headings turn by pi, wrapped into (-pi, pi]
    let turned = (arc.end.heading + PI + PI).rem_euclid(2.0 * PI) - PI;
    assert!(near(back.start.heading.sin(), turned.sin(), 1e-9));
    assert!(near(back.start.heading.cos(), turned.cos(), 1e-9));
    assert_eq!(back.model, arc.model);
    assert!(near(back.chord(), arc.chord(), 1e-9));
}

#[test]
fn straights_and_end_caps_are_told_apart() {
    let c = synthetic_catalog();
    assert_eq!(piece(&c, Family::Mud, "mud_25").kind, PieceKind::Straight);
    assert!(near(piece(&c, Family::Mud, "mud_25").chord(), 25.0, 1e-6));
    assert!(near(piece(&c, Family::Mud, "mud_6").chord(), 6.25, 1e-6));
    assert_eq!(
        piece(&c, Family::City, "city_6konec").kind,
        PieceKind::EndCap
    );
    // a straight does not turn
    assert!(piece(&c, Family::Grav, "grav_12").heading_change().abs() < 1e-9);
}

#[test]
fn junction_ports_are_found() {
    let c = synthetic_catalog();
    let t = c.t_junction(Family::Asf3, Family::Asf3).unwrap();
    assert_eq!(t.model, model("kr_t_asf3_asf3"));
    assert_eq!(t.through, Family::Asf3);
    assert_eq!(t.side, Family::Asf3);
    assert_eq!(t.side_ports.len(), 1);
    let port = &t.side_ports[0];
    // on the left of the through road, pointing away from it (west)
    assert!(port.x < t.start.x);
    assert!(near(port.heading, -FRAC_PI_2, 1e-9));
    assert!(near(port.z, 0.0, 1e-9));

    let x = c.x_junction(Family::City, Family::City).unwrap();
    assert_eq!(x.side_ports.len(), 2);
    let headings: Vec<f64> = x.side_ports.iter().map(|p| p.heading).collect();
    assert!(headings.iter().any(|h| near(*h, -FRAC_PI_2, 1e-9)));
    assert!(headings.iter().any(|h| near(*h, FRAC_PI_2, 1e-9)));
}

#[test]
fn a_t_is_found_by_its_families() {
    let c = synthetic_catalog();
    assert!(c.t_junction(Family::Asf3, Family::Mud).is_some());
    // the list is not symmetric
    assert!(c.t_junction(Family::Asf2, Family::Asf3).is_some());
    assert!(c.t_junction(Family::Mud, Family::Asf3).is_none());
    assert!(c.t_junction(Family::Grav, Family::Grav).is_none());
}

#[test]
fn an_x_for_a_pair_that_does_not_exist_is_none() {
    let c = synthetic_catalog();
    assert!(c.x_junction(Family::Grav, Family::Grav).is_none());
    assert!(c.x_junction(Family::Asf3, Family::Asf1).is_none());
    assert!(c.x_junction(Family::Asf1, Family::Asf3).is_some());
}

#[test]
fn a_family_without_straights_names_the_missing_model() {
    let mut models = synthetic_models();
    models.retain(|k, _| !k.contains("grav_"));
    let e = PieceCatalog::from_models(&models).unwrap_err();
    assert!(matches!(e, CatalogError::NoStraight { .. }), "{e}");
    assert!(e.to_string().contains("grav_25"), "{e}");
}

#[test]
fn a_piece_without_its_points_names_the_model_and_the_point() {
    let mut models = synthetic_models();
    models
        .get_mut(&model("asf1_12"))
        .unwrap()
        .memory
        .remove("lb");
    let e = PieceCatalog::from_models(&models).unwrap_err();
    let text = e.to_string();
    assert!(text.contains("asf1_12") && text.contains("lb"), "{text}");
}

#[test]
fn models_that_are_not_road_pieces_are_left_out() {
    let before = synthetic_catalog();
    let mut models = synthetic_models();
    let spare = models[&model("asf3_25")].clone();
    for name in [
        "asf3_6_crosswalk",
        "runway_main",
        "taxiway_25",
        "centerline_25",
        "path_dirt_10",
        "asf1enoch_25",
        "kr_x_city_city_asf3",
        "decal_manhole",
    ] {
        models.insert(model(name), spare.clone());
    }
    let after = PieceCatalog::from_models(&models).unwrap();
    assert_eq!(after, before);
}

#[test]
fn bridges_keep_their_direction_and_deck_height() {
    let c = synthetic_catalog();
    assert!(c.bridges().len() >= 2);
    let stone = c
        .bridges()
        .iter()
        .find(|b| b.model.contains("stone"))
        .unwrap();
    assert!(near(stone.length, 25.0, 1e-4));
    assert!(near(stone.deck_height, 7.9, 1e-4));
    assert!(near(stone.start.heading, 0.0, 1e-9));
    // a bridge that lies along the x axis runs east
    let wood = c
        .bridges()
        .iter()
        .find(|b| b.model.contains("wood"))
        .unwrap();
    assert!(near(wood.start.heading, FRAC_PI_2, 1e-6));
    assert!(near(wood.width, 5.0, 1e-4));
}

#[test]
fn a_bridge_without_a_memory_lod_is_left_out() {
    let mut models = synthetic_models();
    models.insert(
        "dz\\structures\\roads\\bridges\\bridge_wood_50.p3d".to_string(),
        empty_model(&[]),
    );
    let before = synthetic_catalog();
    assert_eq!(PieceCatalog::from_models(&models).unwrap(), before);
}

#[test]
fn the_synthetic_catalog_is_complete() {
    let c = synthetic_catalog();
    for family in Family::ALL {
        assert!(c.pieces(family).len() >= 14, "{family:?}");
    }
    let junctions = Family::ALL
        .iter()
        .flat_map(|a| Family::ALL.iter().map(move |b| (*a, *b)))
        .filter(|(a, b)| c.t_junction(*a, *b).is_some() || c.x_junction(*a, *b).is_some())
        .count();
    assert!(junctions >= 11);
}

#[test]
fn the_catalog_survives_a_json_round_trip() {
    let c = synthetic_catalog();
    let json = serde_json::to_string(&c).unwrap();
    let back: PieceCatalog = serde_json::from_str(&json).unwrap();
    assert_eq!(back, c);
}

#[test]
fn the_same_models_give_the_same_catalog() {
    let a = serde_json::to_string(&synthetic_catalog()).unwrap();
    let b = serde_json::to_string(&synthetic_catalog()).unwrap();
    assert_eq!(a, b);
}

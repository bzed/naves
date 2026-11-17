// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What hangs below a bridge part, the panels and the pipe, measured on synthetic models (see
//! `naves_core::catalog::testing`); no game data is used.

#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]

use std::collections::BTreeMap;

use naves_core::catalog::Family;
use naves_core::catalog::testing::{
    Block, arch_bridge, bridge_of_blocks, bridge_with_end_blocks, panel_model, post_bridge,
    slab_bridge, synthetic_catalog, synthetic_models,
};
use naves_core::catalog::{
    BRIDGES_PREFIX, BridgeModel, MAX_DECK_THICKNESS_M, PartShape, PieceCatalog,
    SAKHAL_BRIDGES_PREFIX, Slab,
};
use naves_core::roadplace::{BRIDGE_NARROWER_M, bridge_for};
use naves_formats::odol::{OdolError, OdolModel};

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// The catalog of the synthetic models with `name` (below the road bridges' folder) set to `model`.
fn catalog_with(name: &str, model: OdolModel) -> PieceCatalog {
    let mut models = synthetic_models();
    models.insert(format!("{BRIDGES_PREFIX}{name}.p3d"), model);
    PieceCatalog::from_models(&models).unwrap()
}

fn bridge<'a>(c: &'a PieceCatalog, name: &str) -> &'a BridgeModel {
    c.bridges()
        .iter()
        .find(|b| b.model.ends_with(&format!("\\{name}.p3d")))
        .unwrap()
}

#[test]
fn a_deck_with_end_blocks_is_solid_at_both_ends_and_open_between() {
    let c = catalog_with(
        "bridge_metal_25",
        bridge_with_end_blocks(25.0, 12.0, 8.0, 1.7, (3.4, 8.0, 20.5)),
    );
    let b = bridge(&c, "bridge_metal_25");
    assert_eq!(b.underside.depth.len(), 25);
    assert_eq!(b.solid.len(), 2, "{:?}", b.solid);
    let (first, last) = (&b.solid[0], &b.solid[1]);
    assert!(
        near(first.from, 0.0, 1e-9) && near(first.to, 4.0, 1e-9),
        "{first:?}"
    );
    assert!(
        near(last.from, 21.0, 1e-9) && near(last.to, 25.0, 1e-9),
        "{last:?}"
    );
    assert!(near(first.depth, 20.5, 1e-4) && near(first.half_width, 8.0, 1e-4));
    assert!(first.wall && last.wall);
    assert_eq!(b.retains, (true, true));
    let (from, to) = b.opening();
    assert!(near(from, 4.0, 1e-9) && near(to, 21.0, 1e-9), "{from} {to}");
    assert!(
        near(b.thickness(from, to), 1.7, 1e-4),
        "{}",
        b.thickness(from, to)
    );
    assert!(near(b.deck_thickness, 1.7, 1e-4));
    // an interval that touches a block gets the block
    assert!(near(b.thickness(2.0, 6.0), 20.5, 1e-4));
    assert!(near(b.reach_down, 20.5, 1e-4));
    assert!(!b.posts());
}

#[test]
fn an_arch_deepens_from_the_crown_to_the_springings() {
    let c = catalog_with("bridge_stone_25", arch_bridge(25.0, 9.0, 7.9, 3.2, 16.9));
    let b = bridge(&c, "bridge_stone_25");
    let d = &b.underside.depth;
    // deepest at the ports, shallowest at mid-span, growing outwards
    assert!(d[0] > 16.0 && d[24] > 16.0, "{d:?}");
    assert!(d[12] < 3.5, "{d:?}");
    for i in 0..12 {
        assert!(d[i] >= d[i + 1] - 1e-9, "{i}: {d:?}");
        assert!(d[24 - i] >= d[23 - i] - 1e-9, "{i}: {d:?}");
    }
    assert!(b.thickness(12.0, 13.0) < b.thickness(8.0, 17.0));
    assert_eq!(b.retains, (true, true));
    let (from, to) = b.opening();
    assert!(from > 5.0 && to < 20.0 && to > from, "{from} {to}");
    assert!(b.deck_thickness <= MAX_DECK_THICKNESS_M + 1e-9);
    assert!(near(b.deck_thickness, b.thickness(from, to), 1e-12));
}

#[test]
fn posts_hold_nothing_back() {
    let c = catalog_with(
        "bridge_wood_25",
        post_bridge(
            31.6,
            5.0,
            0.05,
            &[3.0, 7.0, 11.0, 15.0, 19.0, 24.0, 27.5],
            2.6,
        ),
    );
    let b = bridge(&c, "bridge_wood_25");
    assert_eq!(b.solid.len(), 0);
    assert_eq!(b.retains, (false, false));
    assert!(near(b.reach_down, 2.6, 1e-4));
    assert!(near(b.deck_thickness, 2.6, 1e-4));
    // between the rows of posts the deck hangs nothing (the rails are above it)
    assert!(
        near(b.underside.depth[1], 0.0, 1e-9),
        "{:?}",
        b.underside.depth
    );
    assert!(b.posts());
    assert_eq!(b.opening(), (0.0, b.length));
}

#[test]
fn two_pylons_beside_the_road_are_solid_but_hold_nothing_back() {
    // like bridge_metal_25_1: a 12 m deck, two pylons 1.4 m wide 6.7 to 8.1 m off the axis at its second port
    let pylon = |c0: f64, c1: f64| Block {
        a0: 23.25,
        a1: 26.6,
        c0,
        c1,
        top: 1.7,
        depth: 20.5,
    };
    let model = bridge_of_blocks(
        25.0,
        12.0,
        8.0,
        false,
        &[
            Block {
                a0: -1.25,
                a1: 25.0,
                c0: -6.0,
                c1: 6.0,
                top: 0.0,
                depth: 1.7,
            },
            pylon(-8.1, -6.7),
            pylon(6.7, 8.1),
        ],
    );
    let c = catalog_with("bridge_metal_25", model);
    let b = bridge(&c, "bridge_metal_25");
    assert_eq!(b.solid.len(), 1, "{:?}", b.solid);
    assert!(near(b.solid[0].to, 25.0, 1e-9));
    assert!(b.solid[0].half_width >= 6.0);
    assert!(!b.solid[0].wall);
    assert_eq!(b.retains, (false, false));
}

#[test]
fn a_model_without_faces_is_open_over_its_whole_length_and_says_so() {
    let mut model = slab_bridge(25.0, 9.0, 7.9, 1.2);
    model.geometry_faces = Vec::new();
    let c = catalog_with("bridge_stone_25", model.clone());
    let b = bridge(&c, "bridge_stone_25");
    assert_eq!(b.solid.len(), 0);
    assert_eq!(b.retains, (false, false));
    assert_eq!(b.opening(), (0.0, 25.0));
    assert!(
        b.underside.depth.iter().all(|d| near(*d, 1.2, 1e-4)),
        "{:?}",
        b.underside.depth
    );
    assert!(
        c.warnings()
            .iter()
            .any(|w| w.contains("bridge_stone_25") && w.contains("faces")),
        "{:?}",
        c.warnings()
    );
    // faces that the reader could not use: the same, and the warning names why
    model.geometry_faces_error = Some(OdolError::FaceCorners { at: 7, count: 5 });
    let c = catalog_with("bridge_stone_25", model);
    assert_eq!(bridge(&c, "bridge_stone_25").solid.len(), 0);
    assert!(
        c.warnings()
            .iter()
            .any(|w| w.contains("bridge_stone_25") && w.contains("5 corners")),
        "{:?}",
        c.warnings()
    );
}

#[test]
fn the_synthetic_catalog_has_no_warnings() {
    assert_eq!(synthetic_catalog().warnings(), &[] as &[String]);
}

#[test]
fn a_model_with_absurd_coordinates_is_left_out_with_a_warning() {
    for bad in [f32::NAN, 1e30, f32::INFINITY] {
        let mut model = slab_bridge(25.0, 9.0, 7.9, 1.2);
        if let Some(g) = model.geometry.as_mut() {
            g[3][1] = bad;
        }
        let c = catalog_with("bridge_stone_25", model);
        assert!(
            !c.bridges()
                .iter()
                .any(|b| b.model.contains("bridge_stone_25")),
            "{bad}"
        );
        assert!(
            c.warnings().iter().any(|w| w.contains("bridge_stone_25")),
            "{bad}"
        );
    }
    // ports at one point, or a part far longer than any bridge
    for length in [0.0, 1e6] {
        let c = catalog_with("bridge_stone_25", slab_bridge(length, 9.0, 7.9, 1.2));
        assert!(
            !c.bridges()
                .iter()
                .any(|b| b.model.contains("bridge_stone_25")),
            "{length}"
        );
        assert!(
            c.warnings().iter().any(|w| w.contains("bridge_stone_25")),
            "{length}"
        );
    }
}

#[test]
fn a_panel_and_the_pipe_are_sized_from_their_lods() {
    let c = synthetic_catalog();
    assert_eq!(c.panels().len(), 2);
    let p = &c.panels()[0];
    assert!(p.model.ends_with("panel_concrete_1.p3d"), "{}", p.model);
    assert!(
        near(p.along, 3.0, 1e-4) && near(p.across, 6.0, 1e-4),
        "{p:?}"
    );
    assert!(p.along_x);
    assert!(near(p.thickness, 0.24, 1e-4), "{p:?}");
    // the surface is the roadway's
    assert!(near(p.top, 0.14, 1e-4), "{p:?}");
    let pipe = c.pipe().unwrap();
    assert!(
        near(pipe.length, 3.04, 1e-4) && near(pipe.diameter, 2.46, 1e-4),
        "{pipe:?}"
    );
    assert!(!pipe.axis_x);
    assert!(near(pipe.bottom, -1.23, 1e-4));
    // the other panels are no panels of the catalog
    let mut models = synthetic_models();
    for name in [
        "panel_concrete_dam",
        "panel_concrete_1_legacy",
        "panel_concrete_1_lc",
    ] {
        models.insert(
            format!("dz\\structures\\roads\\panels\\{name}.p3d"),
            panel_model(3.0, 6.0, 0.24),
        );
    }
    assert_eq!(
        PieceCatalog::from_models(&models).unwrap().panels().len(),
        2
    );
}

#[test]
fn a_panel_row_is_a_plain_slab() {
    let slab = Slab {
        model: "panel".into(),
        along: 3.0,
        across: 6.0,
        along_x: true,
        top: 0.14,
        thickness: 0.24,
    };
    let row = PartShape::panels(&slab, 3, 1.5);
    assert!(near(row.length, 13.5, 1e-9));
    assert_eq!(row.underside.depth.len(), 14);
    assert!(row.underside.depth.iter().all(|d| near(*d, 0.36, 1e-9)));
    assert!(row.underside.half_width.iter().all(|d| near(*d, 4.5, 1e-9)));
    assert_eq!(row.solid.len(), 0);
    assert_eq!(row.retains, (false, false));
    assert_eq!(row.opening(), (0.0, 13.5));
    assert!(near(row.thickness(2.0, 5.0), 0.36, 1e-9));
    // a scale that is not a number counts as 1, and a very long row has a bounded underside
    assert!(near(
        PartShape::panels(&slab, 2, f64::NAN).length,
        6.0,
        1e-9
    ));
    let long = PartShape::panels(&slab, 1_000_000, 1.0);
    assert!(long.underside.depth.len() <= 200);
    assert!(near(long.thickness(1e5, 2e5), 0.24, 1e-9));
    assert_eq!(long.opening(), (0.0, long.length));
}

#[test]
fn without_the_sakhal_parts_there_is_no_bridge_wood2_and_a_warning() {
    let mut models = synthetic_models();
    models.retain(|path, _| !path.starts_with(SAKHAL_BRIDGES_PREFIX));
    let c = PieceCatalog::from_models(&models).unwrap();
    assert!(!c.bridges().iter().any(|b| b.model.contains("bridge_wood2")));
    assert!(
        c.warnings().iter().any(|w| w.contains("bridge_wood2")),
        "{:?}",
        c.warnings()
    );
    // the ladder goes on with what is there
    assert_ne!(bridge_for(&c, Family::Grav).len(), 0);
}

#[test]
fn a_bridge_may_be_a_metre_narrower_than_its_road_and_is_never_scaled() {
    // a gravel road is 6 m: 0.9 m narrower fits, 1.1 m does not
    for (width, fits) in [(5.1, true), (4.9, false), (9.0, true)] {
        let mut models = synthetic_models();
        models.retain(|path, _| !path.contains("bridge_wood"));
        models.insert(
            format!("{BRIDGES_PREFIX}bridge_wood_25.p3d"),
            slab_bridge(31.6, width, 0.05, 0.5),
        );
        let c = PieceCatalog::from_models(&models).unwrap();
        let list = bridge_for(&c, Family::Grav);
        assert_eq!(!list.is_empty(), fits, "{width}: {list:?}");
        assert!(
            list.iter()
                .all(|b| b.width + BRIDGE_NARROWER_M + 1e-9 >= 6.0)
        );
    }
}

#[test]
fn every_fitting_model_is_offered_shortest_first() {
    let c = synthetic_catalog();
    let names = |family| -> Vec<String> {
        bridge_for(&c, family)
            .iter()
            .map(|b| b.model.rsplit('\\').next().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        names(Family::Grav),
        ["bridge_wood2.p3d", "bridge_wood_25.p3d"]
    );
    assert_eq!(
        names(Family::Mud),
        ["bridge_wood2.p3d", "bridge_wood_25.p3d"]
    );
    assert_eq!(
        names(Family::Asf3),
        ["bridge_concrete3.p3d", "bridge_stone_25.p3d"]
    );
    assert_eq!(names(Family::Asf2), ["bridge_stone_25.p3d"]);
    assert_eq!(names(Family::Asf1), ["bridge_metal_25.p3d"]);
    assert_eq!(names(Family::City), ["bridge_metal_25.p3d"]);
}

#[test]
fn the_shape_of_a_model_is_its_own_fields() {
    let c = synthetic_catalog();
    let b = bridge(&c, "bridge_metal_25");
    let s = b.shape();
    assert_eq!(s.underside, b.underside);
    assert_eq!(s.solid, b.solid);
    assert_eq!(s.opening(), b.opening());
    assert!(near(s.thickness(0.0, 25.0), b.thickness(0.0, 25.0), 1e-12));
}

#[test]
fn the_catalog_with_parts_survives_a_json_round_trip() {
    let c = synthetic_catalog();
    let back: PieceCatalog = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
    assert_eq!(back, c);
    let _: BTreeMap<String, OdolModel> = synthetic_models();
}

#[test]
fn ports_a_float_rounding_apart_from_a_whole_length_give_no_sliver_slice() {
    // the game's metal bridge is 25.000002 m between its ports: no 26th slice of a few micrometres that would
    // hide the block at its second port
    let c = catalog_with(
        "bridge_metal_25",
        bridge_with_end_blocks(25.000_02, 12.0, 8.0, 1.7, (3.4, 8.0, 20.5)),
    );
    let b = bridge(&c, "bridge_metal_25");
    assert_eq!(b.underside.depth.len(), 25);
    assert_eq!(b.retains, (true, true));
}

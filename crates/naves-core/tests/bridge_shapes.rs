// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What hangs below a bridge part, the panels and the pipe, measured on synthetic models (see
//! `naves_core::catalog::testing`); no game data is used.

#![expect(
    clippy::unwrap_used,
    clippy::many_single_char_names,
    reason = "test helper code; failing loudly is the point"
)]

use std::collections::BTreeMap;

use naves_core::catalog::Family;
use naves_core::catalog::testing::{
    Block, arch_bridge, bridge_of_blocks, bridge_with_end_blocks, panel_model, post_bridge,
    slab_bridge, synthetic_catalog, synthetic_models,
};
use naves_core::catalog::{
    BRIDGES_PREFIX, BridgeModel, Ceiling, MAX_DECK_THICKNESS_M, PartShape, PieceCatalog,
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
    // the solid parts end where the blocks end, not on whole slices (they were 0..4 and 21..25 before)
    assert!(
        near(first.from, 0.0, 1e-4) && near(first.to, 3.4, 1e-4),
        "{first:?}"
    );
    assert!(
        near(last.from, 21.6, 1e-4) && near(last.to, 25.0, 1e-4),
        "{last:?}"
    );
    assert!(near(first.depth, 20.5, 1e-4) && near(first.half_width, 8.0, 1e-4));
    assert!(first.wall && last.wall);
    assert_eq!(b.retains, (true, true));
    let (from, to) = b.opening();
    assert!(near(from, 3.4, 1e-4) && near(to, 21.6, 1e-4), "{from} {to}");
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
    assert!(b.deck_thickness < MAX_DECK_THICKNESS_M + 1.0);
    assert!(near(b.deck_thickness, b.thickness(from, to), 1e-12));
}

#[test]
fn the_deck_of_an_arch_is_not_deeper_than_its_opening_can_be() {
    // the opening ends inside a slice whose flank hangs deeper than 4 m: the slice counts for the thickness over
    // the opening, but nothing in the opening is deeper than 4 m (deeper geometry is solid)
    let c = catalog_with("bridge_stone_25", arch_bridge(25.0, 9.0, 7.9, 3.5, 16.9));
    let b = bridge(&c, "bridge_stone_25");
    let (from, to) = b.opening();
    assert!(b.thickness(from, to) > MAX_DECK_THICKNESS_M);
    assert!(
        near(b.deck_thickness, MAX_DECK_THICKNESS_M, 1e-12),
        "{}",
        b.deck_thickness
    );
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
    let row = PartShape::panels(&slab, 3, 1, 1.5);
    assert!(near(row.length, 13.5, 1e-9));
    assert_eq!(row.underside.depth.len(), 14);
    assert!(row.underside.depth.iter().all(|d| near(*d, 0.36, 1e-9)));
    assert!(row.underside.half_width.iter().all(|d| near(*d, 4.5, 1e-9)));
    assert_eq!(row.solid.len(), 0);
    assert_eq!(row.retains, (false, false));
    assert_eq!(row.opening(), (0.0, 13.5));
    // three abreast: the grid is three times as wide, and no abreast is one
    let grid = PartShape::panels(&slab, 3, 3, 1.5);
    assert!(
        grid.underside
            .half_width
            .iter()
            .all(|d| near(*d, 13.5, 1e-9))
    );
    assert!(near(grid.length, 13.5, 1e-9));
    let none = PartShape::panels(&slab, 3, 0, 1.5);
    assert!(
        none.underside
            .half_width
            .iter()
            .all(|d| near(*d, 4.5, 1e-9))
    );
    assert!(near(row.thickness(2.0, 5.0), 0.36, 1e-9));
    // a scale that is not a number counts as 1, and a very long row has a bounded underside
    assert!(near(
        PartShape::panels(&slab, 2, 1, f64::NAN).length,
        6.0,
        1e-9
    ));
    let long = PartShape::panels(&slab, 1_000_000, 1, 1.0);
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

fn pylon_model() -> OdolModel {
    // like bridge_metal_25_1: a 12 m deck 1.7 m thick, two pylons 1.4 m wide 6.7 to 8.1 m off the axis
    let pylon = |c0: f64, c1: f64| Block {
        a0: 23.25,
        a1: 26.6,
        c0,
        c1,
        top: 1.7,
        depth: 20.5,
    };
    bridge_of_blocks(
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
    )
}

#[test]
fn between_two_pylons_the_deck_hangs_and_only_the_pylons_stand() {
    let c = catalog_with("bridge_metal_25", pylon_model());
    let b = bridge(&c, "bridge_metal_25");
    let u = &b.underside;
    for i in [23, 24] {
        assert!(near(u.hang[i], 1.7, 1e-4), "{i}: {:?}", u.hang);
        assert_eq!(u.stand[i].len(), 2, "{i}: {:?}", u.stand[i]);
        let (left, right) = (u.stand[i][0], u.stand[i][1]);
        assert!(
            near(left.0, -8.1, 1e-4) && near(left.1, -6.7, 1e-4),
            "{left:?}"
        );
        assert!(
            near(right.0, 6.7, 1e-4) && near(right.1, 8.1, 1e-4),
            "{right:?}"
        );
    }
    assert_eq!(u.stand[10].len(), 0);
    let s = b.shape();
    assert_eq!(s.ceiling(24.0, 0.0), Ceiling::Hang(1.7_f64.min(u.hang[24])));
    assert!(matches!(s.ceiling(24.0, 7.4), Ceiling::Foot(d) if near(d, 20.5, 1e-4)));
    assert!(matches!(s.ceiling(10.0, -7.4), Ceiling::Hang(d) if near(d, 1.7, 1e-4)));
    // outside the part: the nearest slice
    assert!(matches!(s.ceiling(40.0, 7.4), Ceiling::Foot(_)));
    assert!(matches!(s.ceiling(f64::NAN, 0.0), Ceiling::Hang(_)));
    // the solid part starts where the pylons do
    assert!(near(b.solid[0].from, 23.25, 1e-4), "{:?}", b.solid);
    let stands: Vec<_> = s.stands().collect();
    assert!(
        stands
            .iter()
            .all(|p| p.0 >= 23.0 - 1e-9 && p.1 <= 25.0 + 1e-9),
        "{stands:?}"
    );
    assert!(
        stands
            .iter()
            .all(|p| p.2.abs() >= 6.7 - 1e-4 && p.3.abs() >= 6.7 - 1e-4)
    );
    assert_eq!(stands.len(), 4);
    // the pylons stand beside the deck, yet a road below that clears the deck does not pass them: what stands
    // anywhere across blocks the slice
    let deep = s.passable(25.0, 4.5);
    assert_eq!(deep.len(), 1, "{deep:?}");
    assert!(
        near(deep[0].0, 0.0, 1e-4) && near(deep[0].1, 23.25, 1e-4),
        "{deep:?}"
    );
}

#[test]
fn an_arch_hangs_over_its_flanks_and_stands_on_its_abutments() {
    let c = catalog_with("bridge_stone_25", arch_bridge(25.0, 9.0, 7.9, 3.2, 16.9));
    let b = bridge(&c, "bridge_stone_25");
    let u = &b.underside;
    let n = u.depth.len();
    for i in 0..n {
        let abutment = i < 2 || i >= n - 2;
        assert_eq!(!u.stand[i].is_empty(), abutment, "{i}: {:?}", u.stand[i]);
        if !abutment {
            assert!(near(u.hang[i], u.depth[i], 1e-9), "{i}");
        }
    }
    let s = b.shape();
    let crown = s.passable(8.5, 4.5);
    assert_eq!(crown.len(), 1, "{crown:?}");
    let wider = s.passable(12.0, 4.5);
    assert_eq!(wider.len(), 1, "{wider:?}");
    let (c0, c1) = crown[0];
    let (w0, w1) = wider[0];
    assert!(
        w0 < c0 && w1 > c1 && c0 < 12.5 && c1 > 12.5,
        "{crown:?} {wider:?}"
    );
    // never through the abutments, however deep the road below
    for (from, to) in s.passable(100.0, 4.5) {
        assert!(from >= 2.0 - 1e-9 && to <= 23.0 + 1e-9, "{from} {to}");
    }
}

#[test]
fn a_road_below_never_passes_through_the_blocks() {
    let c = catalog_with(
        "bridge_metal_25",
        bridge_with_end_blocks(25.0, 12.0, 8.0, 1.7, (3.4, 8.0, 20.5)),
    );
    let s = bridge(&c, "bridge_metal_25").shape();
    let deep = s.passable(25.0, 4.5);
    assert_eq!(deep.len(), 1, "{deep:?}");
    assert!(
        near(deep[0].0, 3.4, 1e-4) && near(deep[0].1, 21.6, 1e-4),
        "{deep:?}"
    );
    // a road 5 m below the deck does not clear 4.5 m under a 1.7 m deck
    assert_eq!(s.passable(5.0, 4.5).len(), 0);
    assert_eq!(s.passable(6.2, 4.5).len(), 1);
}

#[test]
fn posts_stand_and_the_deck_between_them_hangs_nothing() {
    let rows = [3.0, 7.0, 11.0, 15.0, 19.0, 24.0, 27.5];
    let c = catalog_with("bridge_wood_25", post_bridge(31.6, 5.0, 0.05, &rows, 2.6));
    let b = bridge(&c, "bridge_wood_25");
    let u = &b.underside;
    for i in [3, 7, 11, 15, 19, 24, 27] {
        assert_ne!(u.stand[i].len(), 0, "{i}");
    }
    assert_eq!(u.stand[1].len(), 0);
    assert!(near(u.hang[1], 0.0, 1e-9));
    assert!(b.posts());
    assert_eq!(b.shape().stands().count(), rows.len());
    // a road below never passes a row of posts, beside the deck or under it
    for (from, to) in b.shape().passable(20.0, 4.5) {
        assert!(
            rows.iter().all(|r| *r + 0.4 <= from || *r >= to),
            "{from} {to}"
        );
    }
}

#[test]
fn a_plain_slab_hangs_everywhere_and_nothing_stands() {
    let c = catalog_with("bridge_metal_25", slab_bridge(19.7, 12.0, 4.0, 0.9));
    let b = bridge(&c, "bridge_metal_25");
    assert!(b.underside.stand.iter().all(Vec::is_empty));
    assert!(b.underside.hang.iter().all(|h| near(*h, 0.9, 1e-4)));
    assert!(!b.posts());
    assert!(matches!(b.shape().ceiling(5.0, 0.0), Ceiling::Hang(h) if near(h, 0.9, 1e-4)));
}

#[test]
fn cross_walls_every_three_metres_pass_no_road() {
    let mut blocks = vec![Block {
        a0: 0.0,
        a1: 21.1,
        c0: -3.8,
        c1: 3.8,
        top: 0.0,
        depth: 0.64,
    }];
    let mut at = 2.39;
    while at < 20.0 {
        blocks.push(Block {
            a0: at,
            a1: at + 1.26,
            c0: -8.0,
            c1: 8.8,
            top: 0.64,
            depth: 4.82,
        });
        at += 2.98;
    }
    let c = catalog_with(
        "bridge_metal_25",
        bridge_of_blocks(21.1, 7.6, 2.16, false, &blocks),
    );
    let s = bridge(&c, "bridge_metal_25").shape();
    for depth in [4.5, 6.0, 10.0, 50.0] {
        for (from, to) in s.passable(depth, 4.5) {
            // the gaps between the walls are 1.72 m
            assert!(to - from < 4.0, "{depth}: {from} {to}");
        }
    }
    assert!(matches!(s.ceiling(2.39 + 0.6, 0.0), Ceiling::Foot(d) if near(d, 4.82, 1e-4)));
}

#[test]
fn a_panel_row_hangs_its_slab_and_stands_nowhere() {
    let slab = Slab {
        model: "panel".into(),
        along: 3.0,
        across: 6.0,
        along_x: true,
        top: 0.14,
        thickness: 0.24,
    };
    let row = PartShape::panels(&slab, 4, 1, 1.0);
    assert!(row.underside.hang.iter().all(|h| near(*h, 0.24, 1e-9)));
    assert!(row.underside.stand.iter().all(Vec::is_empty));
    assert_eq!(row.stands().count(), 0);
    assert_eq!(row.passable(5.0, 4.5), vec![(0.0, 12.0)]);
}

/// `m` turned by `angle` about the vertical axis (model points and geometry alike).
fn turned(m: &OdolModel, angle: f64) -> OdolModel {
    let (s, c) = angle.sin_cos();
    let turn = |p: [f32; 3]| {
        let (x, z) = (f64::from(p[0]), f64::from(p[2]));
        [narrow(x * c - z * s), p[1], narrow(x * s + z * c)]
    };
    let mut out = m.clone();
    for points in out.memory.values_mut() {
        for p in points {
            *p = turn(*p);
        }
    }
    if let Some(g) = out.geometry.as_mut() {
        for p in g {
            *p = turn(*p);
        }
    }
    out
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "test geometry is stored as f32"
)]
fn narrow(v: f64) -> f32 {
    v as f32
}

/// `m` with its ports swapped: it is used from the other end.
fn swapped(m: &OdolModel) -> OdolModel {
    let mut out = m.clone();
    let get = |k: &str| m.memory.get(k).cloned().unwrap();
    for (to, from) in [("lb", "pe"), ("pb", "le"), ("le", "pb"), ("pe", "lb")] {
        out.memory.insert(to.to_string(), get(from));
    }
    out
}

#[test]
fn ports_along_a_diagonal_or_swapped_give_the_same_shape() {
    let model = bridge_with_end_blocks(25.0, 12.0, 8.0, 1.7, (3.4, 8.0, 20.5));
    let plain = bridge(
        &catalog_with("bridge_metal_25", model.clone()),
        "bridge_metal_25",
    )
    .clone();
    for other in [turned(&model, 0.6), turned(&model, 2.5), swapped(&model)] {
        let b = bridge(&catalog_with("bridge_metal_25", other), "bridge_metal_25").clone();
        assert_eq!(b.underside.depth.len(), plain.underside.depth.len());
        for (x, y) in b.underside.depth.iter().zip(&plain.underside.depth) {
            assert!(
                near(*x, *y, 1e-3),
                "{:?} {:?}",
                b.underside.depth,
                plain.underside.depth
            );
        }
        assert_eq!(b.retains, plain.retains);
        let (o, p) = (b.opening(), plain.opening());
        assert!(near(o.0, p.0, 1e-3) && near(o.1, p.1, 1e-3), "{o:?} {p:?}");
    }
    // the pylon model used from its other end has its pylons at the start
    let back = bridge(
        &catalog_with("bridge_metal_25", swapped(&pylon_model())),
        "bridge_metal_25",
    )
    .clone();
    assert!(
        near(back.solid[0].from, 0.0, 1e-4) && near(back.solid[0].to, 1.75, 1e-4),
        "{:?}",
        back.solid
    );
    // and the pylons are mirrored across the axis: still the two intervals
    assert_eq!(back.underside.stand[0].len(), 2);
    // `across` is positive to the left of the first port's way to the second: one pylon to the east of a deck
    // that runs north stands to the right (negative), and to the left when the deck is used from its far end
    let one = one_pylon_model();
    let right = |m: OdolModel, slice: usize| {
        bridge(&catalog_with("bridge_metal_25", m), "bridge_metal_25")
            .underside
            .stand[slice]
            .clone()
    };
    for m in [one.clone(), turned(&one, 0.6), turned(&one, 2.5)] {
        let feet = right(m, 24);
        assert_eq!(feet.len(), 1, "{feet:?}");
        assert!(
            near(feet[0].0, -8.1, 1e-3) && near(feet[0].1, -6.7, 1e-3),
            "{feet:?}"
        );
    }
    let feet = right(swapped(&one), 0);
    assert_eq!(feet.len(), 1, "{feet:?}");
    assert!(
        near(feet[0].0, 6.7, 1e-3) && near(feet[0].1, 8.1, 1e-3),
        "{feet:?}"
    );
}

/// A 12 m deck running north with one pylon to its east (model x 6.7 to 8.1) at its second port.
fn one_pylon_model() -> OdolModel {
    bridge_of_blocks(
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
            Block {
                a0: 23.25,
                a1: 26.6,
                c0: 6.7,
                c1: 8.1,
                top: 1.7,
                depth: 20.5,
            },
        ],
    )
}

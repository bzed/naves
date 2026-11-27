// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ground under a bridge: a ceiling under the deck, cut open to both sides, never raised, and the roads
//! that stay where they are.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test helper code; failing loudly is the point and the numbers are tiny"
)]

use naves_core::bridgeground::{BridgeSpan, bridge_spans};
use naves_core::catalog::testing::synthetic_catalog;
use naves_core::catalog::{PartShape, Underside};
use naves_core::conform::{
    BRIDGE_AIR_M, CUT_SLOPE_MAX, Holder, conform, conform_bridges, spans_above_underside,
};
use naves_core::heightmap::Heightmap;
use naves_core::profile::RoadStrip;
use naves_core::roadnet::{RoadGraph, Stretch};

const CELL: f32 = 2.0;
const SIZE: u32 = 101;

fn terrain(f: impl Fn(f64, f64) -> f64) -> Heightmap {
    let mut h = Vec::new();
    for row in 0..SIZE {
        for col in 0..SIZE {
            h.push(f(
                f64::from(col) * f64::from(CELL),
                f64::from(row) * f64::from(CELL),
            ) as f32);
        }
    }
    Heightmap {
        size: SIZE,
        cell_m: CELL,
        h,
    }
}

fn flat(height: f64) -> Heightmap {
    terrain(|_, _| height)
}

fn at(hm: &Heightmap, x: f64, y: f64) -> f64 {
    f64::from(hm.at(
        (x / f64::from(CELL)).round() as u32,
        (y / f64::from(CELL)).round() as u32,
    ))
}

/// A part with a level underside of `depth` and `half_width`, from `first` to `second`.
fn level(
    first: (f64, f64, f64),
    second: (f64, f64, f64),
    depth: f64,
    half_width: f64,
) -> BridgeSpan {
    let length = (second.0 - first.0).hypot(second.1 - first.1);
    BridgeSpan {
        stretch: "deck".to_string(),
        first,
        second,
        shape: PartShape::level(length, depth, half_width),
        own_strips: Vec::new(),
    }
}

fn strip(joints: &[(f64, f64, f64)], bridge: &[bool], half_width: f64) -> RoadStrip {
    RoadStrip {
        slots: Vec::new(),
        joints: joints.to_vec(),
        bridge: bridge.to_vec(),
        junction: Vec::new(),
        side_slope: Vec::new(),
        half_width,
        rank: 3,
    }
}

/// A level road along x = 100 at height 100 with a deck from y = 50 to y = `to`.
fn road_over(to: f64) -> RoadStrip {
    strip(
        &[
            (100.0, 0.0, 100.0),
            (100.0, 50.0, 100.0),
            (100.0, to, 100.0),
            (100.0, 190.0, 100.0),
        ],
        &[false, true, false],
        3.5,
    )
}

/// A level road across, at `height`, from x = 20 to x = 180, at `y`.
fn road_across(y: f64, height: f64, half_width: f64) -> RoadStrip {
    strip(&[(20.0, y, height), (180.0, y, height)], &[], half_width)
}

/// The vertices under the deck (along 4 m inside it, across its half width), as `(x, y)`.
fn under(from: f64, to: f64) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let mut y = from;
    while y <= to {
        let mut x = 94.0;
        while x <= 106.0 {
            if (x - 100.0_f64).abs() <= 3.5 {
                out.push((x, y));
            }
            x += f64::from(CELL);
        }
        y += f64::from(CELL);
    }
    out
}

#[test]
fn flat_ground_at_deck_level_is_cut_under_a_panel_row_open_to_both_sides() {
    let base = flat(100.0);
    let span = level((100.0, 50.0, 100.0), (100.0, 90.0, 100.0), 3.0, 3.5);
    let out = conform_bridges(&base, &[road_over(90.0)], &[], &[span]).unwrap();
    let ceiling = 100.0 - 3.0 - BRIDGE_AIR_M;
    for (x, y) in under(56.0, 84.0) {
        assert!(
            at(&out.heightmap, x, y) <= ceiling + 1e-3,
            "({x}, {y}): {}",
            at(&out.heightmap, x, y)
        );
    }
    // the vertices at the ports are the road's: its core holds there
    for y in [50.0, 90.0] {
        assert!((at(&out.heightmap, 100.0, y) - 100.0).abs() < 1e-3, "{y}");
    }
    // the cut spreads sideways with the cutting slope, on both sides, and ends where the ground meets it
    for side in [-1.0, 1.0] {
        // two metres beyond the last vertex under the deck's band (3.5 m and 1.5 cells)
        let beside = at(&out.heightmap, 100.0 + side * 8.0, 70.0);
        assert!(beside < 99.5, "{side}: {beside}");
        assert!(beside <= ceiling + CUT_SLOPE_MAX * 2.0 + 1e-3, "{beside}");
        assert!((at(&out.heightmap, 100.0 + side * 40.0, 70.0) - 100.0).abs() < 1e-6);
    }
}

#[test]
fn a_truss_with_end_blocks_is_cut_to_the_hang_ceiling_also_under_the_blocks() {
    // 20 m long: the deck hangs 1.7 m, an abutment block stands 2 m deep at each end, foot 5 m below the deck
    let n = 20;
    let mut stand = vec![Vec::new(); n];
    for i in [0, 1, 2, 3, 16, 17, 18, 19] {
        stand[i] = vec![(-3.0, 3.0)];
    }
    let mut depth = vec![1.7; n];
    for i in [0, 1, 2, 3, 16, 17, 18, 19] {
        depth[i] = 5.0;
    }
    let shape = PartShape {
        length: 20.0,
        underside: Underside {
            depth,
            half_width: vec![3.0; n],
            hang: vec![1.7; n],
            stand,
        },
        solid: Vec::new(),
        retains: (true, true),
        reach_down: 5.0,
        deck_half_width: 3.0,
    };
    let span = BridgeSpan {
        stretch: "truss".to_string(),
        first: (100.0, 50.0, 100.0),
        second: (100.0, 70.0, 100.0),
        shape,
        own_strips: Vec::new(),
    };
    let out = conform_bridges(&flat(100.0), &[road_over(70.0)], &[], &[span]).unwrap();
    // under the blocks the bridge's cut stops at the hang ceiling (the deck's underside less the air) like
    // everywhere under the deck: never at the foot, 5 m below the deck, which only bounds other roads' cutting
    let ceiling = 100.0 - 1.7 - BRIDGE_AIR_M;
    for y in [54.0, 66.0] {
        let under = at(&out.heightmap, 100.0, y);
        assert!((under - ceiling).abs() < 0.05, "{y}: {under}");
    }
    // the ground beside the blocks is not lower than beside the open span
    for x in [106.0, 108.0, 112.0, 88.0, 92.0, 94.0] {
        let by_block = at(&out.heightmap, x, 54.0);
        let by_span = at(&out.heightmap, x, 60.0);
        assert!(by_block >= by_span - 0.05, "{x}: {by_block} < {by_span}");
    }
    // within a cell of a port the ground is the road's side, not shaped for the deck
    for y in [50.0, 52.0, 68.0, 70.0] {
        assert!((at(&out.heightmap, 100.0, y) - 100.0).abs() < 1e-3, "{y}");
    }
    // between them at the deck's underside less the air
    let middle = at(&out.heightmap, 100.0, 60.0);
    assert!((middle - ceiling).abs() < 0.05, "{middle}");
    // nowhere under or beside the part is the ground lower than the lowest of the original ground and the ceiling
    let lowest = out
        .heightmap
        .h
        .iter()
        .copied()
        .fold(f32::INFINITY, f32::min);
    assert!(f64::from(lowest) >= ceiling - 0.05, "{lowest}");
}

#[test]
fn cross_walls_every_three_metres_are_cut_to_the_hang_ceiling_and_no_lower() {
    // like bridge_concrete3: a thin deck, walls 4.8 m deep in every third slice across the whole width
    let n = 40;
    let mut stand = vec![Vec::new(); n];
    let mut depth = vec![0.64; n];
    for i in (0..n).filter(|i| i % 3 == 0) {
        stand[i] = vec![(-3.0, 3.0)];
        depth[i] = 4.8;
    }
    let shape = PartShape {
        length: 40.0,
        underside: Underside {
            depth,
            half_width: vec![3.0; n],
            hang: vec![0.64; n],
            stand,
        },
        solid: Vec::new(),
        retains: (false, false),
        reach_down: 4.8,
        deck_half_width: 3.0,
    };
    let span = BridgeSpan {
        stretch: "walls".to_string(),
        first: (100.0, 50.0, 100.0),
        second: (100.0, 90.0, 100.0),
        shape,
        own_strips: Vec::new(),
    };
    let out = conform_bridges(&flat(100.0), &[road_over(90.0)], &[], &[span]).unwrap();
    let ceiling = 100.0 - 0.64 - BRIDGE_AIR_M;
    for y in (56..=84).step_by(2) {
        let under = at(&out.heightmap, 100.0, f64::from(y));
        assert!((under - ceiling).abs() < 0.05, "{y}: {under}");
    }
    let lowest = out
        .heightmap
        .h
        .iter()
        .copied()
        .fold(f32::INFINITY, f32::min);
    assert!(f64::from(lowest) >= ceiling - 0.05, "{lowest}");
}

#[test]
fn ground_that_is_lower_than_the_ceiling_stays_and_nothing_is_raised() {
    let base = terrain(|x, _| if x < 100.0 { 90.0 } else { 98.0 });
    let span = level((100.0, 50.0, 100.0), (100.0, 90.0, 100.0), 0.25, 3.5);
    let out = conform_bridges(&base, &[road_over(90.0)], &[], &[span]).unwrap();
    for (x, y) in under(56.0, 84.0) {
        assert_eq!(
            at(&out.heightmap, x, y).to_bits(),
            at(&base, x, y).to_bits(),
            "({x}, {y})"
        );
    }
}

#[test]
fn a_road_below_keeps_its_core_and_its_cutting_spreads_under_the_deck() {
    let base = flat(100.0);
    let span = level((100.0, 50.0, 100.0), (100.0, 90.0, 100.0), 0.25, 3.5);
    let mut strips = vec![road_over(90.0), road_across(70.0, 90.0, 3.0)];
    let out = conform_bridges(&base, &strips, &[], std::slice::from_ref(&span)).unwrap();
    // the road keeps its height under the deck: the bridge's ceiling never raises it
    for y in [66.0, 68.0, 70.0, 72.0, 74.0] {
        assert!((at(&out.heightmap, 100.0, y) - 90.0).abs() < 1e-3, "{y}");
    }
    // its cutting reaches under the deck: 4 m from its core's edge the ground is no higher than its slope
    // allows (before, nothing under a span was shaped by it)
    assert!(
        at(&out.heightmap, 100.0, 80.0) < 95.0,
        "{}",
        at(&out.heightmap, 100.0, 80.0)
    );
    assert!(out.buried.is_empty(), "{:?}", out.buried);
    strips.clear();
}

#[test]
fn another_roads_core_under_the_deck_is_not_lowered_and_what_stays_is_buried() {
    let base = flat(100.0);
    let span = level((100.0, 50.0, 100.0), (100.0, 90.0, 100.0), 0.25, 3.5);
    // a road at the deck's own level runs along under it
    let other = strip(&[(100.0, 40.0, 100.0), (100.0, 110.0, 100.0)], &[], 1.5);
    let out = conform_bridges(&base, &[road_over(90.0), other], &[], &[span]).unwrap();
    assert_eq!(
        at(&out.heightmap, 100.0, 70.0).to_bits(),
        100.0_f64.to_bits()
    );
    assert_eq!(out.buried.len(), 1, "{:?}", out.buried);
    let buried = &out.buried[0];
    assert_eq!(buried.stretch, "deck");
    // above the underside (99.75), not the ceiling with its metre of air
    assert!((buried.above_m - 0.25).abs() < 0.01, "{buried:?}");
    assert!(
        buried.vertices >= 3 && (50.0..=90.0).contains(&buried.at.1),
        "{buried:?}"
    );
}

/// A wooden model: a 0.6 m deck with a post at each end 2 m deep, 20 m long.
fn on_posts() -> PartShape {
    let n = 20;
    let mut stand = vec![Vec::new(); n];
    for (i, feet) in stand.iter_mut().enumerate() {
        if i == 5 || i == 14 {
            *feet = vec![(-2.5, -1.5), (1.5, 2.5)];
        }
    }
    PartShape {
        length: 20.0,
        underside: Underside {
            depth: vec![2.0; n],
            half_width: vec![3.0; n],
            hang: vec![0.6; n],
            stand,
        },
        solid: Vec::new(),
        retains: (false, false),
        reach_down: 2.0,
        deck_half_width: 3.0,
    }
}

fn wood() -> BridgeSpan {
    BridgeSpan {
        stretch: "wood".to_string(),
        first: (100.0, 50.0, 100.0),
        second: (100.0, 70.0, 100.0),
        shape: on_posts(),
        own_strips: Vec::new(),
    }
}

#[test]
fn posts_over_a_dip_do_not_reach_the_ground_and_posts_on_it_do() {
    // the ground 6 m below the deck: the posts (2 m long) float by 6 - 2 - 0.5
    let dip = conform_bridges(&flat(94.0), &[road_over(70.0)], &[], &[wood()]).unwrap();
    assert_eq!(dip.floating.len(), 1, "{:?}", dip.floating);
    let f = &dip.floating[0];
    assert_eq!(f.stretch, "wood");
    assert!((f.by_m - 3.5).abs() < 1e-3, "{f:?}");
    assert!((50.0..=70.0).contains(&f.at.1), "{f:?}");
    // ground 1.8 m below reaches, within the tolerance
    let near = conform_bridges(&flat(98.2), &[road_over(70.0)], &[], &[wood()]).unwrap();
    assert!(near.floating.is_empty(), "{:?}", near.floating);
    // ground at the deck's level is cut to the foot: the posts stand on it
    let level = conform_bridges(&flat(100.0), &[road_over(70.0)], &[], &[wood()]).unwrap();
    assert!(level.floating.is_empty(), "{:?}", level.floating);
    // the cut stops at the deck's ceiling (0.6 m of deck and a metre of air), the posts' foot is 2 m down
    for y in [56.0, 64.0] {
        let under = at(&level.heightmap, 98.0, y);
        assert!((under - 98.4).abs() < 0.05, "{y}: {under}");
    }
    let lowest = level
        .heightmap
        .h
        .iter()
        .copied()
        .fold(f32::INFINITY, f32::min);
    assert!(f64::from(lowest) >= 98.4 - 0.05, "{lowest}");
}

#[test]
fn a_deck_narrower_than_a_cell_and_a_bridge_at_the_edge_change_only_existing_vertices() {
    let base = flat(100.0);
    let narrow = level((100.0, 50.0, 100.0), (100.0, 90.0, 100.0), 0.3, 0.2);
    let edge = level((0.0, 20.0, 100.0), (0.0, 60.0, 100.0), 0.3, 3.0);
    let corner = level((190.0, 190.0, 100.0), (210.0, 210.0, 100.0), 0.3, 3.0);
    let strip = road_over(90.0);
    let out = conform_bridges(&base, &[strip], &[], &[narrow, edge, corner]).unwrap();
    assert_eq!(out.heightmap.h.len(), base.h.len());
    assert_eq!(out.heightmap.size, base.size);
    assert!(out.heightmap.h.iter().zip(&base.h).all(|(n, o)| n <= o));
    // the narrow deck still gets its ceiling at the vertex it passes
    assert!(at(&out.heightmap, 100.0, 70.0) <= 100.0 - 0.3 - BRIDGE_AIR_M + 1e-3);
    // a part with no length or no numbers is nothing
    let broken = [
        level((100.0, 50.0, 100.0), (100.0, 50.0, 100.0), 1.0, 3.0),
        level((f64::NAN, 50.0, 100.0), (100.0, 90.0, f64::NAN), 1.0, 3.0),
    ];
    let same = conform_bridges(&base, &[], &[], &broken).unwrap();
    assert_eq!(same.heightmap, base);
}

#[test]
fn a_thick_deck_is_cut_less_deep_near_a_port_that_holds_nothing_back() {
    let span = level((100.0, 50.0, 100.0), (100.0, 90.0, 100.0), 3.0, 3.5);
    let open = conform_bridges(
        &flat(100.0),
        &[road_over(90.0)],
        &[],
        std::slice::from_ref(&span),
    )
    .unwrap();
    // the first cell of the end is not shaped (the terrain cannot show a ramp that short)
    assert!((at(&open.heightmap, 100.0, 52.0) - 100.0).abs() < 1e-3);
    // four metres in, the ceiling is still on its ramp up to the road (4 m of air and deck over 1:1)
    let near = at(&open.heightmap, 100.0, 54.0);
    assert!(near > 95.9 && near < 97.1, "{near}");
    assert!(at(&open.heightmap, 100.0, 70.0) <= 96.0 + 1e-3);
    // a wall that holds the ground back has no ramp: the ceiling is the same all along
    let mut walled = span;
    walled.shape.retains = (true, true);
    let held = conform_bridges(&flat(100.0), &[road_over(90.0)], &[], &[walled]).unwrap();
    assert!(at(&held.heightmap, 100.0, 54.0) <= 96.0 + 1e-3);
    assert!(at(&held.heightmap, 100.0, 86.0) <= 96.0 + 1e-3);
}

#[test]
fn parts_that_meet_in_the_middle_of_a_bridge_have_no_ramp_there() {
    let a = level((100.0, 50.0, 100.0), (100.0, 70.0, 100.0), 3.0, 3.5);
    let b = level((100.0, 70.0, 100.0), (100.0, 90.0, 100.0), 3.0, 3.5);
    let out = conform_bridges(&flat(100.0), &[road_over(90.0)], &[], &[a, b]).unwrap();
    assert!(
        at(&out.heightmap, 100.0, 70.0) <= 96.0 + 1e-3,
        "{}",
        at(&out.heightmap, 100.0, 70.0)
    );
}

#[test]
fn a_part_laid_the_other_way_round_has_the_same_ceiling() {
    // the same part from its other end: first and second swapped, the deck sloping the other way
    let base = flat(100.0);
    let forward = level((100.0, 50.0, 100.0), (100.0, 90.0, 102.0), 0.3, 3.5);
    let back = level((100.0, 90.0, 102.0), (100.0, 50.0, 100.0), 0.3, 3.5);
    let road = strip(
        &[
            (100.0, 0.0, 100.0),
            (100.0, 50.0, 100.0),
            (100.0, 90.0, 102.0),
            (100.0, 190.0, 102.0),
        ],
        &[false, true, false],
        3.5,
    );
    let a = conform_bridges(&base, std::slice::from_ref(&road), &[], &[forward]).unwrap();
    let b = conform_bridges(&base, std::slice::from_ref(&road), &[], &[back]).unwrap();
    assert_eq!(a.heightmap, b.heightmap);
    // and the deck's height along it follows the slope: at the middle 101 less the underside and the air
    // (the ceiling is the lowest within half a cell along: a metre back where the deck rises)
    assert!((at(&a.heightmap, 100.0, 70.0) - (101.0 - 0.3 - BRIDGE_AIR_M)).abs() < 0.06);
}

#[test]
fn the_shaping_is_deterministic_and_leaves_the_base_alone() {
    let base = terrain(|x, y| 100.0 + 0.05 * x - 0.03 * y);
    let copy = base.clone();
    let span = level((100.0, 50.0, 104.0), (100.0, 90.0, 104.0), 1.0, 3.5);
    let road = strip(
        &[
            (100.0, 0.0, 104.0),
            (100.0, 50.0, 104.0),
            (100.0, 90.0, 104.0),
            (100.0, 190.0, 104.0),
        ],
        &[false, true, false],
        3.5,
    );
    let a = conform_bridges(
        &base,
        std::slice::from_ref(&road),
        &[],
        std::slice::from_ref(&span),
    )
    .unwrap();
    let b = conform_bridges(
        &base,
        std::slice::from_ref(&road),
        &[],
        std::slice::from_ref(&span),
    )
    .unwrap();
    assert_eq!(a, b);
    assert_eq!(base, copy);
}

#[test]
fn without_bridges_conform_is_what_it_was() {
    let base = terrain(|x, y| 100.0 + 0.05 * x - 0.03 * y);
    let road = road_across(70.0, 95.0, 3.0);
    assert_eq!(
        conform(&base, std::slice::from_ref(&road), &[]).unwrap(),
        conform_bridges(&base, std::slice::from_ref(&road), &[], &[]).unwrap()
    );
}

#[test]
fn ground_above_the_underside_is_counted_before_and_not_after() {
    let base = flat(100.0);
    let spans = [level((100.0, 50.0, 100.0), (100.0, 90.0, 100.0), 3.0, 3.5)];
    let before = spans_above_underside(&base, &spans);
    assert_eq!(before.len(), 1, "{before:?}");
    assert_eq!(before[0].stretch, "deck");
    assert!((before[0].above_m - 3.0).abs() < 1e-3, "{before:?}");
    assert!(before[0].vertices >= 10);
    let out = conform_bridges(&base, &[road_over(90.0)], &[], &spans).unwrap();
    // two vertices at a port are the road's and one end's ramp may leave ground above the underside there
    let after = spans_above_underside(&out.heightmap, &spans);
    assert!(after.iter().all(|a| a.above_m < 1.5), "{after:?}");
    // ground that was lower than the underside is none
    assert!(matches!(
        spans_above_underside(&flat(90.0), &spans).as_slice(),
        []
    ));
}

#[test]
fn the_placed_parts_of_a_bridge_are_its_spans_with_the_heights_of_the_joints() {
    use naves_core::catalog::Family;
    use naves_core::roadfit::Pose;
    use naves_core::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad, Slot};

    let catalog = synthetic_catalog();
    let slab = catalog
        .panels()
        .iter()
        .min_by(|a, b| a.model.cmp(&b.model))
        .unwrap()
        .clone();
    let model = catalog.bridges().first().unwrap().clone();
    let pose = |x: f64, y: f64| Pose {
        x,
        y,
        heading: std::f64::consts::FRAC_PI_2,
    };
    let panel = |x0: f64| PlacedRoad {
        slot: Slot {
            stretch: "7:1-2".to_string(),
            index: 0,
        },
        kind: PlacedKind::Bridge,
        family: Family::Grav,
        drivable: true,
        model: slab.model.clone(),
        origin: pose(x0 + 1.5, 500.0),
        start: pose(x0, 500.0),
        end: pose(x0 + 3.0, 500.0),
        width: 6.0,
        scale: 1.5,
        abreast: 1,
    };
    // a model with its origin at (700, 500) turned to the east; it lies against the stretch's direction in
    // the second road: start and end swapped
    let theta = std::f64::consts::FRAC_PI_2;
    let (s, c) = theta.sin_cos();
    let world = |x: f64, z: f64| (700.0 + x * c + z * s, 500.0 - x * s + z * c);
    let (p0, p1) = (
        world(model.start.x, model.start.z),
        world(model.end.x, model.end.z),
    );
    let laid = |start: (f64, f64), end: (f64, f64)| PlacedRoad {
        slot: Slot {
            stretch: "8:3-4".to_string(),
            index: 0,
        },
        kind: PlacedKind::Bridge,
        family: Family::Asf2,
        drivable: true,
        model: model.model.clone(),
        origin: Pose {
            x: 700.0,
            y: 500.0,
            heading: theta,
        },
        start: pose(start.0, start.1),
        end: pose(end.0, end.1),
        width: model.width,
        scale: 1.0,
        abreast: 1,
    };
    let net = PlacedNetwork {
        roads: vec![panel(400.0), panel(403.0), laid(p0, p1), laid(p1, p0)],
        ..PlacedNetwork::default()
    };
    let strips = vec![
        strip_of(
            &[0, 1],
            &[
                (400.0, 500.0, 10.0),
                (403.0, 500.0, 10.5),
                (406.0, 500.0, 11.0),
            ],
        ),
        strip_of(&[2], &[(p0.0, p0.1, 20.0), (p1.0, p1.1, 21.0)]),
        strip_of(&[3], &[(p1.0, p1.1, 21.0), (p0.0, p0.1, 20.0)]),
    ];
    let spans = bridge_spans(&net, &strips, &catalog, &RoadGraph::default());
    assert_eq!(spans.len(), 4, "{spans:?}");
    assert_eq!(spans[0].stretch, "7:1-2");
    assert_eq!(
        (spans[0].first, spans[0].second),
        ((400.0, 500.0, 10.0), (403.0, 500.0, 10.5))
    );
    // a panel is as thick as one at its scale and as wide as the step
    assert!((spans[0].shape.underside.hang[0] - slab.thickness * 1.5).abs() < 1e-9);
    assert!((spans[0].half_width_at(1.0) - 3.0).abs() < 1e-9);
    // the model lies from its first port: at the stretch's start, or at its end where it is turned round
    assert_eq!(spans[2].first.2.to_bits(), 20.0_f64.to_bits());
    assert_eq!(spans[3].first.2.to_bits(), 20.0_f64.to_bits());
    assert!((spans[2].first.0 - p0.0).abs() < 1e-9 && (spans[3].first.0 - p0.0).abs() < 1e-9);
    assert!((spans[3].second.0 - p1.0).abs() < 1e-9);
    // a model that is not in the catalog is left out, a strip with no deck has none
    let mut other = net.clone();
    other.roads[2].model = "no/such/model.p3d".to_string();
    assert_eq!(
        bridge_spans(&other, &strips, &catalog, &RoadGraph::default()).len(),
        3
    );
    let idle = [strip(&[(0.0, 0.0, 0.0), (1.0, 0.0, 0.0)], &[], 1.0)];
    assert!(matches!(
        bridge_spans(&net, &idle, &catalog, &RoadGraph::default()).as_slice(),
        []
    ));
}

fn strip_of(slots: &[usize], joints: &[(f64, f64, f64)]) -> RoadStrip {
    RoadStrip {
        slots: slots.to_vec(),
        joints: joints.to_vec(),
        bridge: vec![true; slots.len()],
        junction: Vec::new(),
        side_slope: Vec::new(),
        half_width: 3.0,
        rank: 3,
    }
}

#[test]
fn the_road_of_a_short_bridge_does_not_hold_the_ground_under_its_own_deck() {
    // a 10 m deck on a grid of 2 m; the approach has short pieces before it, whose cores reach under the deck
    let base = flat(100.0);
    let span = level((100.0, 100.0, 100.0), (100.0, 110.0, 100.0), 0.25, 3.5);
    let road = strip(
        &[
            (100.0, 0.0, 100.0),
            (100.0, 90.0, 100.0),
            (100.0, 96.0, 100.0),
            (100.0, 100.0, 100.0),
            (100.0, 110.0, 100.0),
            (100.0, 114.0, 100.0),
            (100.0, 120.0, 100.0),
            (100.0, 190.0, 100.0),
        ],
        &[false, false, false, true, false, false, false],
        3.5,
    );
    let out = conform_bridges(
        &base,
        std::slice::from_ref(&road),
        &[],
        std::slice::from_ref(&span),
    )
    .unwrap();
    // the vertex in the middle of the deck is cut to the ceiling, the ports stay the road's
    assert!(at(&out.heightmap, 100.0, 105.0_f64.round()) <= 100.0 - 0.25 - BRIDGE_AIR_M + 1e-3);
    for y in [100.0, 110.0] {
        assert!((at(&out.heightmap, 100.0, y) - 100.0).abs() < 1e-3, "{y}");
    }
    assert!(matches!(out.buried.as_slice(), []), "{:?}", out.buried);
    // a core of another road under the deck is not touched and is reported
    let other = strip(&[(100.0, 90.0, 100.0), (100.0, 120.0, 100.0)], &[], 1.5);
    let out = conform_bridges(&base, &[road, other], &[], &[span]).unwrap();
    assert!((at(&out.heightmap, 100.0, 106.0) - 100.0).abs() < 1e-3);
    assert_eq!(out.buried.len(), 1, "{:?}", out.buried);
}

#[test]
fn ground_that_another_road_holds_between_the_underside_and_the_ceiling_is_not_buried() {
    let base = flat(100.0);
    let span = level((100.0, 50.0, 100.0), (100.0, 90.0, 100.0), 0.25, 3.5);
    let road = |height: f64| strip(&[(100.0, 40.0, height), (100.0, 110.0, height)], &[], 1.5);
    // the underside is at 99.75 and the ceiling at 98.75: a road at 99.0 holds ground in the air margin
    let held = conform_bridges(
        &base,
        &[road_over(90.0), road(99.0)],
        &[],
        std::slice::from_ref(&span),
    )
    .unwrap();
    assert!(matches!(held.buried.as_slice(), []), "{:?}", held.buried);
    assert!((at(&held.heightmap, 100.0, 70.0) - 99.0).abs() < 1e-3);
    // one at 99.9 is in the deck
    let hit = conform_bridges(
        &base,
        &[road_over(90.0), road(99.9)],
        &[],
        std::slice::from_ref(&span),
    )
    .unwrap();
    assert_eq!(hit.buried.len(), 1, "{:?}", hit.buried);
    assert!(
        (hit.buried[0].above_m - 0.15).abs() < 0.01,
        "{:?}",
        hit.buried
    );
    assert_eq!(hit.buried[0].holder, Holder::Strip(1));
    // the count of spans with ground above the underside is the same measure
    let above = spans_above_underside(&hit.heightmap, std::slice::from_ref(&span));
    assert_eq!(above.len(), 1, "{above:?}");
    assert!((above[0].above_m - 0.15).abs() < 0.01, "{above:?}");
    let none = spans_above_underside(&held.heightmap, std::slice::from_ref(&span));
    assert!(matches!(none.as_slice(), []), "{none:?}");
}

#[test]
fn the_strips_of_the_bridges_own_chain_hold_nothing_under_its_deck() {
    let base = flat(100.0);
    let mut span = level((100.0, 50.0, 100.0), (100.0, 90.0, 100.0), 0.25, 3.5);
    // a strip of the road that does not touch the ports (a piece before the junction at the end of the chain)
    let chain = strip(&[(100.0, 40.0, 100.0), (100.0, 110.0, 100.0)], &[], 1.5);
    let foreign = conform_bridges(
        &base,
        &[road_over(90.0), chain.clone()],
        &[],
        std::slice::from_ref(&span),
    )
    .unwrap();
    assert_eq!(foreign.buried.len(), 1, "{:?}", foreign.buried);
    span.own_strips = vec![1];
    let own = conform_bridges(&base, &[road_over(90.0), chain], &[], &[span]).unwrap();
    assert!(matches!(own.buried.as_slice(), []), "{:?}", own.buried);
    assert!(at(&own.heightmap, 100.0, 70.0) <= 98.75 + 1e-3);
}

/// A deck of `length` m from (60, 60) in the direction of `degrees` at 100 m over flat ground at 100 m, with the
/// road on its line at 100 m, and the lowest terrain under the last `along` metres of the approach.
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: failing loudly is the point"
)]
fn dip_under_the_approach(degrees: f64, origin: (f64, f64)) -> f64 {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let point = |t: f64| (origin.0 + t * cos, origin.1 + t * sin);
    let (a, b) = (point(0.0), point(40.0));
    let span = level((a.0, a.1, 100.0), (b.0, b.1, 100.0), 3.0, 3.5);
    let road = strip(
        &[
            (point(-50.0).0, point(-50.0).1, 100.0),
            (a.0, a.1, 100.0),
            (b.0, b.1, 100.0),
            (point(90.0).0, point(90.0).1, 100.0),
        ],
        &[false, true, false],
        3.5,
    );
    let out = conform_bridges(&flat(100.0), &[road], &[], &[span]).unwrap();
    let mut lowest: f64 = 100.0;
    for k in 0..=24 {
        for c in -6..=6 {
            let t = -f64::from(k) / 4.0;
            let (nx, ny) = (-sin, cos);
            let p = (
                point(t).0 + f64::from(c) * 0.5 * nx,
                point(t).1 + f64::from(c) * 0.5 * ny,
            );
            lowest = lowest.min(f64::from(out.heightmap.sample(p.0, p.1).height));
        }
    }
    lowest
}

#[test]
fn a_deck_that_runs_diagonally_to_the_grid_does_not_dip_the_road_before_it() {
    for degrees in [0.0, 30.0, 45.0, 17.0] {
        // the phase of the grid under the port line matters: sweep it
        for dx in [0.0, 0.5, 1.0, 1.5] {
            for dy in [0.0, 0.5, 1.0, 1.5] {
                let lowest = dip_under_the_approach(degrees, (60.0 + dx, 60.0 + dy));
                assert!(
                    lowest >= 99.99,
                    "{degrees} degrees at ({dx}, {dy}): {lowest}"
                );
            }
        }
    }
}

#[test]
fn a_wide_road_beside_a_deck_end_does_not_hold_the_ground_under_it_but_a_road_below_does() {
    let base = flat(100.0);
    let span = level((100.0, 50.0, 100.0), (100.0, 90.0, 100.0), 0.25, 3.5);
    // a road 5 m half width at the deck's level 2 m past its end: its ring reaches 6 m under the deck
    let beside = strip(&[(20.0, 92.0, 100.0), (180.0, 92.0, 100.0)], &[], 5.0);
    let out = conform_bridges(
        &base,
        &[road_over(90.0), beside],
        &[],
        std::slice::from_ref(&span),
    )
    .unwrap();
    assert!(
        at(&out.heightmap, 100.0, 86.0) <= 98.75 + 1e-3,
        "{}",
        at(&out.heightmap, 100.0, 86.0)
    );
    // in the end guard (the last 2 m of the deck) this road's core (|88 - 92| < 5) still claims: its pieces need the ground
    assert!(
        (at(&out.heightmap, 100.0, 88.0) - 100.0).abs() < 1e-3,
        "{}",
        at(&out.heightmap, 100.0, 88.0)
    );
    // a road below, its ring included, keeps what it claims
    let below = strip(&[(20.0, 70.0, 90.0), (180.0, 70.0, 90.0)], &[], 3.0);
    let out = conform_bridges(&base, &[road_over(90.0), below], &[], &[span]).unwrap();
    for y in [64.0, 66.0, 70.0, 74.0, 76.0] {
        assert!((at(&out.heightmap, 100.0, y) - 90.0).abs() < 1e-3, "{y}");
    }
}

#[test]
fn beyond_the_guard_the_ground_does_not_depend_on_the_spacing_of_the_approachs_joints() {
    // a deck 5 m above ground that a road at 100 m comes down to: the road's last piece is 3 m, 6.25 m or 12 m
    let base = flat(95.0);
    let span = level((100.0, 50.0, 100.0), (100.0, 90.0, 100.0), 0.25, 3.5);
    let result = |last: f64| {
        let road = strip(
            &[
                (100.0, 0.0, 100.0),
                (100.0, 50.0 - last, 100.0),
                (100.0, 50.0, 100.0),
                (100.0, 90.0, 100.0),
                (100.0, 190.0, 100.0),
            ],
            &[false, false, true, false],
            3.5,
        );
        conform_bridges(&base, &[road], &[], std::slice::from_ref(&span)).unwrap()
    };
    let first = result(3.0);
    for last in [6.25, 12.0] {
        let other = result(last);
        for y in (56..=84).step_by(2) {
            for x in [94.0, 98.0, 100.0, 102.0, 106.0] {
                assert_eq!(
                    at(&first.heightmap, x, f64::from(y)).to_bits(),
                    at(&other.heightmap, x, f64::from(y)).to_bits(),
                    "{x},{y} with a last piece of {last}"
                );
            }
        }
    }
    // beyond the guard nothing is raised
    for y in (56..=84).step_by(2) {
        assert!(
            (at(&first.heightmap, 100.0, f64::from(y)) - 95.0).abs() < 1e-3,
            "{y}"
        );
    }
}

#[test]
fn ground_in_the_end_guard_and_under_a_short_deck_is_measured_against_the_deck_surface() {
    // a 4 m deck at 98 m over ground at 100 m: all of it is within a cell of an end
    let base = flat(100.0);
    let span = level((100.0, 60.0, 98.0), (100.0, 64.0, 98.0), 0.5, 3.5);
    let road = strip(
        &[
            (100.0, 0.0, 98.0),
            (100.0, 60.0, 98.0),
            (100.0, 64.0, 98.0),
            (100.0, 190.0, 98.0),
        ],
        &[false, true, false],
        3.5,
    );
    let out = conform_bridges(
        &base,
        std::slice::from_ref(&road),
        &[],
        std::slice::from_ref(&span),
    )
    .unwrap();
    // the ground standing in the guard is cut to the deck: nothing is left to report
    assert!(matches!(out.buried.as_slice(), []), "{:?}", out.buried);
    // the analysis of the unshaped ground counts it: 2 m above the surface
    let above = spans_above_underside(&base, std::slice::from_ref(&span));
    assert_eq!(above.len(), 1, "{above:?}");
    assert!((above[0].above_m - 2.0).abs() < 0.05, "{above:?}");
    assert!(matches!(
        spans_above_underside(&out.heightmap, std::slice::from_ref(&span)).as_slice(),
        []
    ));
    // ground below the deck's surface there is none
    assert!(matches!(
        spans_above_underside(&flat(97.0), std::slice::from_ref(&span)).as_slice(),
        []
    ));
}

#[test]
fn a_deck_height_that_is_not_a_number_gives_no_finding() {
    let base = flat(100.0);
    let mut span = level((100.0, 60.0, 98.0), (100.0, 64.0, 98.0), 0.5, 3.5);
    span.first.2 = f64::NAN;
    let road = strip(&[(100.0, 0.0, 100.0), (100.0, 190.0, 100.0)], &[], 3.5);
    let out = conform_bridges(&base, &[road], &[], &[span.clone()]).unwrap();
    assert!(matches!(out.buried.as_slice(), []), "{:?}", out.buried);
    assert!(matches!(
        spans_above_underside(&out.heightmap, &[span]).as_slice(),
        []
    ));
}

#[test]
fn a_road_that_runs_only_beside_a_deck_keeps_its_shoulder() {
    let base = flat(100.0);
    let span = level((100.0, 50.0, 100.0), (100.0, 90.0, 100.0), 0.25, 3.5);
    // a road 3 m half width 8 m beside the deck's axis: its ring reaches the vertex at x = 104, which is beside the
    // deck (3.5 m half width), not under it
    let beside = strip(&[(108.0, 20.0, 100.0), (108.0, 120.0, 100.0)], &[], 3.0);
    let out = conform_bridges(&base, &[road_over(90.0), beside], &[], &[span]).unwrap();
    for y in [62.0, 70.0, 78.0] {
        assert!(
            (at(&out.heightmap, 104.0, y) - 100.0).abs() < 1e-3,
            "{y}: {}",
            at(&out.heightmap, 104.0, y)
        );
    }
    // under the deck the cut stays
    assert!(at(&out.heightmap, 100.0, 70.0) <= 98.75 + 1e-3);
}

#[test]
fn a_guard_survives_a_neighbouring_span_that_is_cut_there() {
    let base = flat(100.0);
    // two steps that fold back: the second ends free at (104, 76); (104, 78) is in its end guard and in the first
    // step's band, which has no guard there
    let first = level((100.0, 50.0, 100.0), (100.0, 80.0, 100.0), 0.25, 3.5);
    let second = level((100.0, 80.0, 100.0), (104.0, 76.0, 100.0), 0.25, 3.5);
    let out = conform_bridges(&base, &[], &[], &[first, second]).unwrap();
    assert!(
        (at(&out.heightmap, 104.0, 78.0) - 100.0).abs() < 1e-3,
        "{}",
        at(&out.heightmap, 104.0, 78.0)
    );
}

#[test]
fn a_guard_survives_where_the_spans_of_a_fork_overlap() {
    let base = flat(100.0);
    // two grids from one node; the short one ends free at (106, 56), its end guard overlaps the long one's band
    let long = level((100.0, 50.0, 100.0), (100.0, 90.0, 100.0), 0.25, 3.5);
    let short = level((100.0, 50.0, 100.0), (106.0, 56.0, 100.0), 0.25, 3.5);
    let out = conform_bridges(&base, &[], &[], &[long, short]).unwrap();
    assert!(
        (at(&out.heightmap, 104.0, 56.0) - 100.0).abs() < 1e-3,
        "{}",
        at(&out.heightmap, 104.0, 56.0)
    );
    // on the long deck, beside the short one's free end but outside its width: the short span's guard does not stop
    // the long span's cut
    for (x, y) in [(100.0, 58.0), (102.0, 58.0)] {
        assert!(at(&out.heightmap, x, y) <= 98.75 + 1e-3, "{x},{y}");
    }
}

#[test]
fn only_a_free_end_has_a_guard_not_the_joint_of_two_steps() {
    let base = flat(100.0);
    let steps = [
        level((100.0, 50.0, 100.0), (100.0, 62.0, 100.0), 0.25, 3.5),
        level((100.0, 62.0, 100.0), (100.0, 78.0, 100.0), 0.25, 3.5),
        level((100.0, 78.0, 100.0), (100.0, 90.0, 100.0), 0.25, 3.5),
    ];
    let out = conform_bridges(&base, &[], &[], &steps).unwrap();
    // the middle step and both sides of both joints are cut; the free ends are not
    for y in [58.0, 60.0, 62.0, 64.0, 70.0, 76.0, 78.0, 80.0] {
        for x in [98.0, 100.0, 102.0] {
            assert!(at(&out.heightmap, x, y) <= 98.75 + 1e-3, "{x},{y}");
        }
    }
    for y in [50.0, 52.0, 88.0, 90.0] {
        assert!((at(&out.heightmap, 100.0, y) - 100.0).abs() < 1e-3, "{y}");
    }
}

#[test]
fn ground_standing_on_a_decks_end_is_cut_to_the_deck_and_never_below() {
    // a bank 2 m above the deck
    let base = flat(102.0);
    let span = level((100.0, 50.0, 100.0), (100.0, 90.0, 100.0), 0.25, 3.5);
    let out = conform_bridges(&base, &[], &[], std::slice::from_ref(&span)).unwrap();
    for y in [50.0, 52.0, 88.0, 90.0] {
        for x in [98.0, 100.0, 102.0] {
            assert!(
                (at(&out.heightmap, x, y) - 100.0).abs() < 1e-3,
                "{x},{y}: {}",
                at(&out.heightmap, x, y)
            );
        }
    }
    assert!(out.buried.is_empty(), "{:?}", out.buried);
    // ground lower than the deck's surface stays where it is
    let out = conform_bridges(&flat(99.4), &[], &[], &[span]).unwrap();
    for y in [50.0, 52.0, 88.0, 90.0] {
        assert!((at(&out.heightmap, 100.0, y) - 99.4).abs() < 1e-3, "{y}");
    }
}

#[test]
fn another_roads_core_in_the_guard_is_left_and_reported() {
    let base = flat(99.0);
    let span = level((100.0, 50.0, 100.0), (100.0, 90.0, 100.0), 0.25, 3.5);
    // the core covers y = 88 and 90 (cells of 2 m)
    let other = road_across(89.0, 101.5, 1.0);
    let out = conform_bridges(&base, &[other], &[], std::slice::from_ref(&span)).unwrap();
    assert!(
        (at(&out.heightmap, 100.0, 88.0) - 101.5).abs() < 1e-3,
        "{}",
        at(&out.heightmap, 100.0, 88.0)
    );
    assert_eq!(out.buried.len(), 1, "{:?}", out.buried);
    assert_eq!(out.buried[0].holder, Holder::Strip(0));
    // inside the guard: measured from the surface, and the metric agrees with the finding
    assert!(out.buried[0].guarded, "{:?}", out.buried[0]);
    let metric = spans_above_underside(&out.heightmap, std::slice::from_ref(&span));
    assert_eq!(metric.len(), 1, "{metric:?}");
    assert!(
        (metric[0].above_m - out.buried[0].above_m).abs() < 1e-3
            && metric[0].vertices == out.buried[0].vertices,
        "{metric:?} {:?}",
        out.buried[0]
    );
}

#[test]
fn another_roads_core_cuts_ground_standing_above_it_in_the_guard_and_is_left_there() {
    // a bank 2 m above the deck, a road in the guard at 101.5: its core is cut down to the road, not to the deck
    let base = flat(102.0);
    let span = level((100.0, 50.0, 100.0), (100.0, 90.0, 100.0), 0.25, 3.5);
    let other = road_across(88.0, 101.5, 1.0);
    let out = conform_bridges(&base, &[other], &[], std::slice::from_ref(&span)).unwrap();
    assert!(
        (at(&out.heightmap, 100.0, 88.0) - 101.5).abs() < 1e-3,
        "{}",
        at(&out.heightmap, 100.0, 88.0)
    );
    assert!(!out.buried.is_empty(), "{:?}", out.buried);
    assert!(
        out.buried
            .iter()
            .all(|b| b.guarded && b.holder == Holder::Strip(0)),
        "{:?}",
        out.buried
    );
}

#[test]
fn another_roads_shoulder_ring_in_the_guard_is_cut_to_the_deck_while_its_core_is_not() {
    // a bank 2 m above the deck; a road at 101.5 across y = 94 with a core of 3 m half width: the vertices at
    // y = 88 and 90 are in its ring (4 to 6 m away; y = 88 is the ring's last vertex), not in its core, and in the
    // deck's guard: they take at most the deck's surface, so nothing is held and `buried` stays empty (no holder
    // to assert). The road's own width keeps its height.
    let base = flat(102.0);
    let span = level((100.0, 50.0, 100.0), (100.0, 90.0, 100.0), 0.25, 3.5);
    let other = road_across(94.0, 101.5, 3.0);
    let out = conform_bridges(&base, &[other], &[], std::slice::from_ref(&span)).unwrap();
    for y in [88.0, 90.0] {
        assert!(
            at(&out.heightmap, 100.0, y) <= 100.05,
            "{y}: {}",
            at(&out.heightmap, 100.0, y)
        );
    }
    assert!(
        (at(&out.heightmap, 100.0, 94.0) - 101.5).abs() < 0.05,
        "the core of the road: {}",
        at(&out.heightmap, 100.0, 94.0)
    );
    assert!(out.buried.is_empty(), "{:?}", out.buried);
}

#[test]
fn the_first_step_of_a_grid_starts_where_the_placed_panel_starts_not_at_the_junction_before_it() {
    use naves_core::catalog::Family;
    use naves_core::roadfit::Pose;
    use naves_core::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad, Slot};

    let catalog = synthetic_catalog();
    let slab = catalog
        .panels()
        .iter()
        .min_by(|a, b| a.model.cmp(&b.model))
        .unwrap()
        .clone();
    let pose = |x: f64| Pose {
        x,
        y: 500.0,
        heading: std::f64::consts::FRAC_PI_2,
    };
    // a grid lengthened over the end of the road before it: the panel starts at 398, but the strip's joint before it
    // is the end of the junction piece at 399.5, inside the deck
    let panel = |x0: f64, x1: f64, index: u32| PlacedRoad {
        slot: Slot {
            stretch: "7:1-2".to_string(),
            index,
        },
        kind: PlacedKind::Bridge,
        family: Family::Grav,
        drivable: true,
        model: slab.model.clone(),
        origin: pose(f64::midpoint(x0, x1)),
        start: pose(x0),
        end: pose(x1),
        width: 6.0,
        scale: 1.0,
        abreast: 1,
    };
    let net = PlacedNetwork {
        roads: vec![panel(398.0, 401.0, 0), panel(401.0, 404.0, 1)],
        ..PlacedNetwork::default()
    };
    let strips = vec![strip_of(
        &[0, 1],
        &[
            (399.5, 500.0, 10.0),
            (401.0, 500.0, 10.0),
            (404.0, 500.0, 10.0),
        ],
    )];
    let spans = bridge_spans(&net, &strips, &catalog, &RoadGraph::default());
    assert_eq!(spans.len(), 2, "{spans:?}");
    assert_eq!(
        (spans[0].first, spans[0].second),
        ((398.0, 500.0, 10.0), (401.0, 500.0, 10.0))
    );
    assert!((spans[0].length() - 3.0).abs() < 1e-9);
    assert_eq!(spans[1].first, (401.0, 500.0, 10.0));
}

#[test]
fn a_panel_steps_shape_is_as_long_as_the_step_not_as_the_strips_old_joints() {
    use naves_core::catalog::Family;
    use naves_core::roadfit::Pose;
    use naves_core::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad, Slot};

    let catalog = synthetic_catalog();
    let slab = catalog
        .panels()
        .iter()
        .min_by(|a, b| a.model.cmp(&b.model))
        .unwrap()
        .clone();
    let pose = |x: f64| Pose {
        x,
        y: 500.0,
        heading: std::f64::consts::FRAC_PI_2,
    };
    let net = PlacedNetwork {
        roads: vec![PlacedRoad {
            slot: Slot {
                stretch: "7:1-2".to_string(),
                index: 0,
            },
            kind: PlacedKind::Bridge,
            family: Family::Grav,
            drivable: true,
            model: slab.model.clone(),
            origin: pose(399.5),
            start: pose(398.0),
            end: pose(401.0),
            width: 6.0,
            scale: 1.0,
            abreast: 1,
        }],
        ..PlacedNetwork::default()
    };
    // the strip's joint before the step is at the step's end (a junction piece that ends where the deck does)
    let strips = vec![strip_of(
        &[0],
        &[(401.0, 500.0, 10.0), (401.0, 500.0, 10.0)],
    )];
    let spans = bridge_spans(&net, &strips, &catalog, &RoadGraph::default());
    assert_eq!(spans.len(), 1, "{spans:?}");
    assert!((spans[0].length() - 3.0).abs() < 1e-9);
    assert!(
        (spans[0].shape.length - 3.0).abs() < 1e-9,
        "{:?}",
        spans[0].shape
    );
    assert!(
        !spans[0].shape.underside.depth.is_empty(),
        "{:?}",
        spans[0].shape
    );
}

fn stretch_of(id: &str, start: i64, end: i64, bridge: bool) -> Stretch {
    Stretch {
        id: id.to_string(),
        family: naves_core::catalog::Family::Grav,
        drivable: true,
        bridge,
        tunnel: false,
        roundabout: false,
        layer: 0,
        points: vec![(0.0, 0.0), (10.0, 0.0)],
        start,
        end,
        bridge_kind: None,
        bridge_offset: 0.0,
        use_sidepath: false,
        paved_m: 0.0,
        unpaved_m: 0.0,
        held: false,
        plan: false,
    }
}

#[test]
fn the_chain_of_a_bridge_follows_the_ends_of_the_stretches_not_the_node_ids_in_their_names() {
    use naves_core::catalog::Family;
    use naves_core::roadfit::Pose;
    use naves_core::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad, Slot};

    let catalog = synthetic_catalog();
    let slab = catalog
        .panels()
        .iter()
        .min_by(|a, b| a.model.cmp(&b.model))
        .unwrap()
        .clone();
    let pose = |x: f64| Pose {
        x,
        y: 500.0,
        heading: std::f64::consts::FRAC_PI_2,
    };
    let road = |stretch: &str, kind: PlacedKind, x: f64| PlacedRoad {
        slot: Slot {
            stretch: stretch.to_string(),
            index: 0,
        },
        kind,
        family: Family::Grav,
        drivable: true,
        model: slab.model.clone(),
        origin: pose(x + 1.5),
        start: pose(x),
        end: pose(x + 3.0),
        width: 6.0,
        scale: 1.5,
        abreast: 1,
    };
    let net = PlacedNetwork {
        roads: vec![
            road("7:1-2", PlacedKind::Bridge, 400.0),
            road("9:5-6", PlacedKind::Piece, 403.0),
        ],
        ..PlacedNetwork::default()
    };
    let strips = vec![
        strip_of(&[0], &[(400.0, 500.0, 10.0), (403.0, 500.0, 10.5)]),
        strip_of(&[1], &[(403.0, 500.0, 10.5), (406.0, 500.0, 11.0)]),
    ];
    // the road after the bridge was named when it ended at node 5; a merge moved its end: it starts at node 2
    let graph = RoadGraph {
        stretches: vec![
            stretch_of("7:1-2", 1, 2, true),
            stretch_of("9:5-6", 2, 6, false),
        ],
        crossings: Vec::new(),
    };
    let spans = bridge_spans(&net, &strips, &catalog, &graph);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].own_strips, vec![0, 1]);
}

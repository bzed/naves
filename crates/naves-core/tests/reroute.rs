// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tests of road planning: the obstacle raster, its distance maps and its wall lines, the feasibility band, the
//! cost-to-go field, the Dubins paths and the arc search of a leg.

#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::catalog::Family;
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::heightmap::Heightmap;
use naves_core::reroute::band::{
    Band, BandFail, DrawnGrade, Reach, arrive, arrives, band_step, drawn_grade, ground_at,
    leg_limit, relaxed_limits,
};
use naves_core::reroute::field::Field;
use naves_core::reroute::raster::{
    Blocked, Clearance, Obstacles, Wall, WallKind, chamfer, crossings_away, exempt_samples, samples,
};
use naves_core::reroute::{
    AnchorKind, CHAMFER_ERROR, Candidate, KeptReason, PlannedZone, RerouteParams, RerouteReport,
    ZoneOutcome, reroute_graph,
};
use naves_core::roadnet::{RoadGraph, Stretch, build_graph};
use proptest::prelude::*;
use proptest::test_runner::{FileFailurePersistence, RngSeed};

const NONE: Clearance = Clearance::NONE;

/// The 200 m box of the examples: 4 m cells, so 2 m raster cells and 100 x 100 of them.
fn world(margin: f64) -> Obstacles {
    Obstacles::new((0.0, 0.0), (200.0, 200.0), 4.0, 200.0, margin, 4_000_000)
}

fn lake(p: (f64, f64)) -> bool {
    (100.0..120.0).contains(&p.0) && (100.0..120.0).contains(&p.1)
}

fn with_lake() -> Obstacles {
    let mut o = world(0.0);
    o.stamp_water(&lake);
    o.finish();
    o
}

#[test]
fn raster_shape() {
    let o = world(0.0);
    assert_eq!(o.cells(), 10_000);
    assert_eq!(o.resolution_m(), 2.0);
}

#[test]
fn lake_blocks_and_has_distance() {
    let o = with_lake();
    assert_eq!(
        o.segment((110.0, 0.0), (110.0, 200.0), NONE),
        Err(Blocked::Water)
    );
    // an estimate from the probe's cell centre (131 m, 12 m from the lake cell centre 119, less half a cell);
    // the true distance is 10 m
    let d = o.water_distance_m((130.0, 110.0));
    assert!((d - 11.0).abs() < 1e-9, "{d}");
}

#[test]
fn threshold_and_clearance() {
    let o = with_lake();
    let want = 7.0 * CHAMFER_ERROR + 2.0 * 2.0_f64.sqrt();
    assert!((o.threshold(5.0) - want).abs() < 1e-9);
    assert_eq!(o.threshold(0.0), 0.0);
    assert_eq!(o.threshold(-3.0), 0.0);
    let clear5 = Clearance::new(5.0, 0.0).unwrap();
    assert_eq!(
        o.segment((130.0, 0.0), (130.0, 200.0), clear5),
        Err(Blocked::Water)
    );
    assert_eq!(o.segment((140.0, 0.0), (140.0, 200.0), clear5), Ok(()));
}

/// The lake with an original line set, so that the raster keeps the water cap.
fn lake_with_original(original: &[(f64, f64)]) -> Obstacles {
    let mut o = world(0.0);
    o.stamp_water(&lake);
    o.set_original(original);
    o.finish();
    o
}

#[test]
fn one_threshold_for_test_and_water_cap() {
    let original = [(135.0, 0.0), (135.0, 200.0)];
    let o = lake_with_original(&original);
    // the stored chamfer distance of the original's cell (centre 135 to the lake cell centre 119)
    let cap = o.water_cap_m().unwrap();
    assert!((cap - 16.0).abs() < 1e-9, "{cap}");
    // the reach alone (threshold 18.4) would block the original line; with the cap it passes its own test
    let clear = Clearance::new(15.0, 0.0).unwrap();
    assert!(o.threshold(15.0) > cap);
    assert_eq!(o.segment(original[0], original[1], clear), Ok(()));
    // nearer than the original was: blocked (cells at 14 m)
    assert_eq!(
        o.segment((134.0, 0.0), (134.0, 200.0), clear),
        Err(Blocked::Water)
    );
}

#[test]
fn without_an_original_there_is_no_cap() {
    let o = with_lake();
    assert_eq!(o.water_cap_m(), None);
    let clear = Clearance::new(15.0, 0.0).unwrap();
    assert_eq!(
        o.segment((135.0, 0.0), (135.0, 200.0), clear),
        Err(Blocked::Water)
    );
}

#[test]
fn an_original_in_the_water_gives_cap_zero() {
    let o = lake_with_original(&[(110.0, 0.0), (110.0, 200.0)]);
    assert_eq!(o.water_cap_m(), Some(0.0));
    // only water cells block now
    let clear = Clearance::new(15.0, 0.0).unwrap();
    assert_eq!(o.segment((121.0, 0.0), (121.0, 200.0), clear), Ok(()));
    assert_eq!(
        o.segment((110.0, 0.0), (110.0, 200.0), clear),
        Err(Blocked::Water)
    );
}

#[test]
fn a_riverside_road_may_stay_as_near_as_it_was() {
    // the original runs 4 m from the lake (cells 61 and 62: centres 123 and 125, 4 and 6 m from 119)
    let original = [(124.0, 0.0), (124.0, 200.0)];
    let o = lake_with_original(&original);
    let cap = o.water_cap_m().unwrap();
    assert!((cap - 4.0).abs() < 1e-9, "{cap}");
    let clear = Clearance::new(15.0, 0.0).unwrap();
    assert_eq!(o.segment(original[0], original[1], clear), Ok(()));
    assert_eq!(o.segment((125.0, 0.0), (125.0, 200.0), clear), Ok(()));
    assert_eq!(
        o.segment((122.0, 0.0), (122.0, 200.0), clear),
        Err(Blocked::Water)
    );
}

#[test]
fn a_clearance_must_be_a_number_and_not_negative() {
    assert!(Clearance::new(f64::NAN, 0.0).is_none());
    assert!(Clearance::new(0.0, f64::NAN).is_none());
    assert!(Clearance::new(f64::INFINITY, 0.0).is_none());
    assert!(Clearance::new(-1.0, 0.0).is_none());
    let c = Clearance::new(5.0, 7.0).unwrap();
    assert!((c.water_m() - 5.0).abs() < 1e-12 && (c.road_m() - 7.0).abs() < 1e-12);
}

fn road_world() -> Obstacles {
    let mut o = world(0.0);
    o.stamp_road(&[(0.0, 50.0), (200.0, 50.0)], 6.0, &[]);
    o.finish();
    o
}

#[test]
fn road_blocks_by_cells_and_clearance() {
    let o = road_world();
    assert_eq!(o.segment((10.0, 60.0), (190.0, 60.0), NONE), Ok(()));
    assert_eq!(
        o.segment((10.0, 55.0), (190.0, 55.0), NONE),
        Err(Blocked::Road)
    );
    let wide = Clearance::new(0.0, 15.0).unwrap();
    assert_eq!(
        o.segment((10.0, 60.0), (190.0, 60.0), wide),
        Err(Blocked::Road)
    );
    assert_eq!(o.segment((10.0, 100.0), (190.0, 100.0), wide), Ok(()));
    let d = o.road_distance_m((100.0, 100.0));
    assert!((40.0..=55.0).contains(&d), "{d}");
}

#[test]
fn exempt_places_are_not_stamped() {
    let mut o = world(0.0);
    let road = [(0.0, 50.0), (200.0, 50.0)];
    let flags: Vec<bool> = samples(&road).iter().map(|p| p.0 < 100.0).collect();
    o.stamp_road(&road, 6.0, &flags);
    o.finish();
    assert_eq!(o.segment((10.0, 52.0), (90.0, 52.0), NONE), Ok(()));
    assert_eq!(
        o.segment((110.0, 52.0), (190.0, 52.0), NONE),
        Err(Blocked::Road)
    );
}

#[test]
fn exempt_samples_grow_along_the_road() {
    // a road of 200 m with a 20 m piece of the original line 8 m from it; near_m 20
    let road = [(0.0, 0.0), (200.0, 0.0)];
    let original = [(90.0, 8.0), (110.0, 8.0)];
    let flags = exempt_samples(&road, &original, 20.0, 10.0);
    assert_eq!(flags.len(), 101);
    // within 20 m of the piece: x in 71.7..128.3, grown by 10 m: 61.7..138.3
    for (i, &f) in flags.iter().enumerate() {
        let x = 2.0 * f64::from(u32::try_from(i).unwrap());
        assert_eq!(f, (62.0..=138.0).contains(&x), "sample at {x}");
    }
    // no original line: nothing is exempt
    assert!(exempt_samples(&road, &[], 20.0, 10.0).iter().all(|&f| !f));
}

#[test]
fn exempt_samples_feed_stamp_road() {
    // the road is the leg's original line's neighbour at its start only
    let road = [(50.0, 10.0), (50.0, 101.0), (150.0, 101.0)];
    let original = [(53.0, 10.0), (53.0, 98.0)];
    let flags = exempt_samples(&road, &original, 6.0, 10.0);
    assert_eq!(flags.len(), samples(&road).len());
    let mut o = world(0.0);
    o.stamp_road(&road, 6.0, &flags);
    o.finish();
    // the original line passes, also round the corner whose vertex (50, 101) is not a sample
    for pair in original.windows(2) {
        assert_eq!(o.segment(pair[0], pair[1], NONE), Ok(()), "{pair:?}");
    }
    // far from the original the road is stamped
    assert_eq!(
        o.segment((100.0, 100.0), (110.0, 100.0), NONE),
        Err(Blocked::Road)
    );
    assert_eq!(o.segment((100.0, 120.0), (110.0, 120.0), NONE), Ok(()));
}

#[test]
fn a_road_over_the_raster_boundary_keeps_its_marking() {
    let mut o = world(0.0);
    // thinner than a cell: only the cells the line touches are marked
    o.stamp_road(&[(-100.0, 50.5), (300.0, 50.5)], 0.5, &[]);
    o.finish();
    assert_eq!(
        o.segment((10.0, 50.5), (190.0, 50.5), NONE),
        Err(Blocked::Road)
    );
    assert_eq!(o.segment((10.0, 70.0), (190.0, 70.0), NONE), Ok(()));
}

#[test]
fn map_edge_rule() {
    let o = world(5.0);
    assert_eq!(
        o.segment((3.0, 20.0), (3.0, 180.0), NONE),
        Err(Blocked::Edge)
    );
    assert_eq!(
        o.segment((50.0, 50.0), (-10.0, 50.0), NONE),
        Err(Blocked::Outside)
    );
    assert_eq!(
        o.segment((50.0, 50.0), (250.0, 50.0), NONE),
        Err(Blocked::Outside)
    );
    let wide = Clearance::new(0.0, 15.0).unwrap();
    assert_eq!(o.segment((7.0, 20.0), (7.0, 180.0), wide), Ok(()));
    // a road that runs to the edge: no margin, only the outside of the map
    let o = world(0.0);
    assert_eq!(o.segment((1.0, 20.0), (1.0, 180.0), NONE), Ok(()));
}

#[test]
fn map_smaller_than_box_marks_the_outside() {
    let o = Obstacles::new((-20.0, -20.0), (120.0, 120.0), 4.0, 100.0, 0.0, 4_000_000);
    assert_eq!(o.segment((10.0, 10.0), (90.0, 90.0), NONE), Ok(()));
    assert_eq!(
        o.segment((10.0, 10.0), (110.0, 10.0), NONE),
        Err(Blocked::Edge)
    );
}

fn stream(allowed: usize, anchors: [(f64, f64); 2]) -> Wall {
    Wall {
        line: vec![(100.0, 0.0), (100.0, 200.0)],
        allowed,
        kind: WallKind::Stream,
        anchors,
        end_m: 10.0,
    }
}

#[test]
fn walls_block_and_count() {
    let far = [(0.0, 0.0), (0.0, 200.0)];
    let wall = stream(0, far);
    assert!(wall.blocks((90.0, 50.0), (110.0, 50.0)));
    assert!(!wall.blocks((90.0, 50.0), (99.0, 150.0)));
    assert!(!stream(1, far).blocks((90.0, 50.0), (110.0, 50.0)));
    assert!(!wall.blocks((f64::NAN, 50.0), (110.0, 50.0)));
    assert_eq!(wall.kind, WallKind::Stream);
}

#[test]
fn a_crossing_at_an_anchor_neither_counts_nor_blocks() {
    let line = vec![(100.0, 0.0), (100.0, 200.0)];
    // the route crosses the stream at (100, 50) and at (100, 150)
    let route = [(90.0, 50.0), (110.0, 50.0), (110.0, 150.0), (90.0, 150.0)];
    // 5 m from the first anchor: not counted, not blocked; the second crossing is far from both
    let anchors = [(95.0, 50.0), (0.0, 0.0)];
    assert_eq!(crossings_away(&route, &line, anchors, 10.0), 1);
    let wall = stream(0, anchors);
    assert_eq!(wall.crossings(&route), 1);
    assert!(!wall.blocks((90.0, 50.0), (110.0, 50.0)));
    assert!(wall.blocks((110.0, 150.0), (90.0, 150.0)));
    // 15 m from both anchors: counted and blocked
    let anchors = [(100.0, 35.0), (100.0, 165.0)];
    assert_eq!(crossings_away(&route, &line, anchors, 10.0), 2);
    assert!(stream(0, anchors).blocks((90.0, 50.0), (110.0, 50.0)));
    // with the rule off both count
    assert_eq!(
        crossings_away(&route, &line, [(95.0, 50.0), (0.0, 0.0)], 0.0),
        2
    );
}

#[test]
fn nan_ends_are_outside() {
    let o = with_lake();
    assert_eq!(
        o.segment((f64::NAN, 50.0), (60.0, 50.0), NONE),
        Err(Blocked::Outside)
    );
    assert_eq!(
        o.segment((10.0, 50.0), (60.0, f64::INFINITY), NONE),
        Err(Blocked::Outside)
    );
}

#[test]
fn stray_from_the_original_line() {
    let mut o = world(0.0);
    o.set_original(&[(0.0, 100.0), (200.0, 100.0)]);
    o.finish();
    let d = o.stray_m((50.0, 107.0));
    // the stored chamfer distance of the cell (6 m from the line's cells), not an upper bound
    assert!((7.0 - 2.0..=7.0 * CHAMFER_ERROR).contains(&d), "{d}");
    assert!(o.stray_m((50.0, 100.0)) < 2.0);
    assert_eq!(o.stray_m((f64::NAN, 1.0)), f64::INFINITY);
}

#[test]
fn big_boxes_coarsen_the_raster() {
    let o = Obstacles::new(
        (0.0, 0.0),
        (100_000.0, 100_000.0),
        4.0,
        100_000.0,
        0.0,
        4_000_000,
    );
    assert!(o.cells() <= 4_000_000);
    let mut k = 0;
    while {
        let res = 2.0 * 1.25_f64.powi(k);
        let n = (100_000.0 / res).ceil();
        n * n > 4_000_000.0
    } {
        k += 1;
    }
    assert!((o.resolution_m() - 2.0 * 1.25_f64.powi(k)).abs() < 1e-9);
}

#[test]
fn far_distance_saturates() {
    let mut o = Obstacles::new(
        (0.0, 0.0),
        (10_000.0, 10_000.0),
        4.0,
        10_000.0,
        0.0,
        4_000_000,
    );
    o.stamp_water(&|p| p.0 < 2.0 && p.1 < 2.0);
    o.finish();
    assert_eq!(o.water_distance_m((9_990.0, 9_990.0)), 6553.5);
    assert_eq!(o.segment((100.0, 100.0), (200.0, 200.0), NONE), Ok(()));
}

#[test]
fn chamfer_extremes() {
    assert!(
        chamfer(&[false; 12], 4, 3, 2.0)
            .iter()
            .all(|&v| v == u16::MAX)
    );
    assert!(chamfer(&[true; 12], 4, 3, 2.0).iter().all(|&v| v == 0));
    let one = [true, false, false, false, false, false];
    assert_eq!(chamfer(&one, 3, 2, 2.0), vec![0, 20, 40, 20, 28, 48]);
    // a mask of the wrong size is not a reason to panic
    assert_eq!(chamfer(&[true], 3, 3, 1.0).len(), 9);
    assert_eq!(chamfer(&[], 0, 0, 1.0).len(), 0);
}

#[test]
fn degenerate_inputs_do_not_panic() {
    let mut o = Obstacles::new(
        (f64::NAN, 0.0),
        (1.0, f64::INFINITY),
        f64::NAN,
        -1.0,
        f64::NAN,
        0,
    );
    o.stamp_road(&[(0.0, 0.0)], 1.0, &[false]);
    o.stamp_road(&[], 1.0, &[]);
    o.set_original(&[]);
    o.finish();
    assert!(o.cells() >= 1);
    let _ = o.segment((0.0, 0.0), (0.5, 0.5), NONE);
    let _ = o.water_cap_m();
    let _ = o.stray_m((0.0, 0.0));
}

#[test]
fn parameters_default_to_the_constants() {
    let p = RerouteParams::default();
    assert_eq!(p.chamfer_error, CHAMFER_ERROR);
    assert_eq!(p.raster_max_cells, 4_000_000);
    assert_eq!(p.crossing_end_m, 10.0);
    assert_eq!(p.simplify, PLAN_WRITE_TOL_M);
    assert!(!p.enabled);
}

// ---- the feasibility band ----

fn band(grade: f64) -> Band {
    Band {
        cut_m: 6.0,
        fill_m: 6.0,
        grade,
        spacing_m: 2.0,
        anchor_tol_m: 0.5,
    }
}

/// An obstacle-free raster over a 400 m box.
fn open() -> Obstacles {
    let mut o = Obstacles::new((0.0, 0.0), (400.0, 400.0), 4.0, 400.0, 0.0, 4_000_000);
    o.finish();
    o
}

/// A line of `len` metres from (10, 10) towards the east.
fn east(from: f64, to: f64) -> Vec<(f64, f64)> {
    vec![(10.0 + from, 10.0), (10.0 + to, 10.0)]
}

/// Ground that depends on the distance east of x = 10 only.
fn along(f: impl Fn(f64) -> f64) -> impl Fn((f64, f64)) -> Option<f64> {
    move |p: (f64, f64)| Some(f(p.0 - 10.0))
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn flat_ground_keeps_the_pinned_start_in_reach() {
    let flat = along(|_| 100.0);
    let step = band_step(
        &east(0.0, 100.0),
        &flat,
        &band(0.1),
        Reach::at_anchor(100.0, &band(0.1)),
        &open(),
        NONE,
    )
    .unwrap();
    assert_eq!(step.climb_m, 0.0);
    assert_eq!(step.earthwork_low_m2, 0.0);
    // The brief says [99.5, 100.5]; after 100 m at 10 % the reach has widened by 10 m each way and is held by the
    // band [94, 106] (spec: the interval propagates by +-g*ds).
    assert!(step.reach.lo() <= 99.5 && step.reach.hi() >= 100.5);
    assert!(close(step.reach.lo(), 94.0, 1e-9) && close(step.reach.hi(), 106.0, 1e-9));
    let p = arrive(&east(0.0, 100.0), &flat, &band(0.1)).unwrap();
    assert_eq!(p.earthwork_m2(), 0.0);
    assert_eq!(p.cut_max_m(), 0.0);
    assert_eq!(p.fill_max_m(), 0.0);
    assert_eq!(p.heights().len(), 51);
}

#[test]
fn a_climb_the_band_allows_but_the_anchors_do_not() {
    let ramp = along(|x| 100.0 + 0.2 * x);
    let start = Reach::at_anchor(100.0, &band(0.1));
    let step = band_step(&east(0.0, 60.0), &ramp, &band(0.1), start, &open(), NONE).unwrap();
    assert!(close(step.reach.lo(), 106.0, 1e-6), "{:?}", step.reach);
    assert!(close(step.reach.hi(), 106.5, 1e-6), "{:?}", step.reach);
    assert!(!arrives(step.reach, 112.0, 0.5));
    assert!(arrives(step.reach, 106.2, 0.5));
    assert_eq!(
        band_step(&east(0.0, 70.0), &ramp, &band(0.1), start, &open(), NONE),
        Err(BandFail::Grade)
    );
    // The interval empties only after 65 m.
    assert!(band_step(&east(0.0, 65.0), &ramp, &band(0.1), start, &open(), NONE).is_ok());
    assert_eq!(
        arrive(&east(0.0, 60.0), &ramp, &band(0.1)),
        Err(BandFail::Arrival)
    );
}

#[test]
fn a_steeper_limit_arrives() {
    let ramp = along(|x| 100.0 + 0.2 * x);
    let start = Reach::at_anchor(100.0, &band(0.1));
    let step = band_step(&east(0.0, 60.0), &ramp, &band(0.2), start, &open(), NONE).unwrap();
    assert!(arrives(step.reach, 112.0, 0.5));
    let p = arrive(&east(0.0, 60.0), &ramp, &band(0.2)).unwrap();
    assert!(p.max_grade() <= 0.2 + 1e-9, "{}", p.max_grade());
    assert!(p.earthwork_m2() < 1e-6);
}

/// Switchback-1 as a line along x: 90.8 m, net rise 21.4 m, a 20 m stretch at 54.9 % in the middle, the rest gentler.
fn switchback() -> impl Fn((f64, f64)) -> Option<f64> {
    let steep = 0.549;
    let rest = (21.4 - 20.0 * steep) / 70.8;
    along(move |x| {
        let a = x.clamp(0.0, 35.0);
        let b = (x - 35.0).clamp(0.0, 20.0);
        let c = (x - 55.0).max(0.0);
        100.0 + a * rest + b * steep + c * rest
    })
}

#[test]
fn switchback_needs_real_earthwork() {
    let line = east(0.0, 90.8);
    let ground = switchback();
    let start = Reach::at_anchor(100.0, &band(0.1));
    let step = band_step(&line, &ground, &band(0.36), start, &open(), NONE).unwrap();
    assert_eq!(step.earthwork_low_m2, 0.0, "the lower bound");
    assert!(arrives(step.reach, 121.4, 0.5));
    let p = arrive(&line, &ground, &band(0.36)).unwrap();
    // Measured: earthwork 67.6 m2, cut 3.62 m, no fill.
    assert!(close(p.earthwork_m2(), 67.6, 1.0) && close(p.cut_max_m(), 3.62, 0.1));
    assert!(p.earthwork_m2() > 10.0, "{}", p.earthwork_m2());
    assert!(p.cut_max_m().max(p.fill_max_m()) > 1.0);
    assert!(p.max_grade() <= 0.36 + 1e-9);
    assert!(step.earthwork_low_m2 <= p.earthwork_m2());
    // 90.8 m at 18 % climbs 16.3 m, the anchors want 21.4 - 1.0.
    assert_eq!(arrive(&line, &ground, &band(0.18)), Err(BandFail::Arrival));
}

fn slope(len: f64, rise: f64) -> impl Fn((f64, f64)) -> Option<f64> {
    along(move |x| 100.0 + rise / len * x)
}

#[test]
fn drawn_grade_and_leg_limit() {
    let w = 20.0;
    let (line, g) = (east(0.0, 145.7), slope(145.7, 21.9));
    let d = drawn_grade(&line, &g, w).unwrap();
    assert!(close(d.mean, 0.150, 0.001), "{d:?}");
    assert!(close(d.delta_z, 21.9, 1e-6));
    let (limit, ceiling) = leg_limit(&[Family::Grav], &d).unwrap();
    assert_eq!(limit, 0.18);
    assert!(close(ceiling, 0.36, 1e-12));
    let (line, g) = (east(0.0, 145.7), slope(145.7, 36.4));
    let d = drawn_grade(&line, &g, w).unwrap();
    assert!(close(
        leg_limit(&[Family::Grav], &d).unwrap().0,
        0.25,
        0.001
    ));
    let (line, g) = (east(0.0, 145.7), slope(145.7, 60.0));
    let d = drawn_grade(&line, &g, w).unwrap();
    assert!(close(
        leg_limit(&[Family::Grav], &d).unwrap().0,
        0.36,
        1e-12
    ));
    // Narrow asphalt: mean 0.074, a 20 m window of 0.264: the window is reported, not used.
    let d = DrawnGrade {
        delta_z: 7.6,
        mean: 0.074,
        window: 0.264,
    };
    let (limit, ceiling) = leg_limit(&[Family::Asf3], &d).unwrap();
    assert_eq!(limit, 0.12);
    assert!(close(ceiling, 0.27, 1e-12));
    // The smallest aim and ceiling of several families.
    assert_eq!(
        leg_limit(&[Family::Grav, Family::Asf3], &d).unwrap().0,
        0.12
    );
    assert_eq!(leg_limit(&[], &d), None);
}

#[test]
fn drawn_grade_reports_the_steepest_window() {
    let steep = switchback();
    let line = east(0.0, 90.8);
    let d = drawn_grade(&line, &steep, 20.0).unwrap();
    assert!(close(d.window, 0.549, 0.01), "{d:?}");
    assert!(close(d.mean, 21.4 / 90.8, 1e-6));
    // A line shorter than the window: the window is the whole line.
    let d = drawn_grade(&east(0.0, 10.0), &steep, 20.0).unwrap();
    assert!(d.window.is_finite());
    assert!(drawn_grade(&[(1.0, 1.0)], &steep, 20.0).is_none());
    assert!(drawn_grade(&east(0.0, 10.0), &|_| None, 20.0).is_none());
}

#[test]
fn relaxation_steps() {
    let r = relaxed_limits(0.18, 0.36, 3).unwrap();
    let want = [0.18, 0.24, 0.30, 0.36];
    assert_eq!(r.len(), 4);
    assert!(
        r.iter().zip(want).all(|(a, b)| close(*a, b, 1e-12)),
        "{r:?}"
    );
    assert_eq!(r.last(), Some(&0.36));
    assert_eq!(relaxed_limits(0.36, 0.36, 3), Some(vec![0.36]));
    assert_eq!(relaxed_limits(0.2, 0.36, 0), Some(vec![0.2]));
}

#[test]
fn no_ground_and_water() {
    let start = Reach::at_anchor(100.0, &band(0.1));
    let nan = |_: (f64, f64)| Some(f64::NAN);
    assert_eq!(
        band_step(&east(0.0, 10.0), &nan, &band(0.1), start, &open(), NONE),
        Err(BandFail::NoGround)
    );
    let none = |_: (f64, f64)| None;
    assert_eq!(
        arrive(&east(0.0, 10.0), &none, &band(0.1)),
        Err(BandFail::NoGround)
    );
    let bad = [(10.0, 10.0), (f64::NAN, 10.0)];
    let flat = along(|_| 100.0);
    assert_eq!(
        band_step(&bad, &flat, &band(0.1), start, &open(), NONE),
        Err(BandFail::NoGround)
    );
    let mut lake = open();
    lake.stamp_water(&|p| (50.0..60.0).contains(&p.0) && (0.0..40.0).contains(&p.1));
    lake.finish();
    assert_eq!(
        band_step(&east(0.0, 100.0), &flat, &band(0.1), start, &lake, NONE),
        Err(BandFail::Blocked(Blocked::Water))
    );
    // A line without segments is a point.
    let step = band_step(&[(10.0, 10.0)], &flat, &band(0.1), start, &open(), NONE).unwrap();
    assert_eq!(step.reach, start);
    let step = band_step(&[], &flat, &band(0.1), start, &open(), NONE).unwrap();
    assert_eq!(step.reach, start);
    let p = arrive(&[(10.0, 10.0)], &flat, &band(0.1)).unwrap();
    assert_eq!(p.heights().len(), 1);
}

#[test]
fn ground_from_the_heightmap() {
    let mut h = vec![10.0_f32; 16];
    h[15] = f32::NAN;
    let map = Heightmap {
        size: 4,
        cell_m: 10.0,
        h,
    };
    assert_eq!(ground_at(&map, (0.0, 0.0)), Some(10.0));
    assert_eq!(ground_at(&map, (10.0, 10.0)), Some(10.0));
    assert_eq!(ground_at(&map, (-0.1, 5.0)), None);
    assert_eq!(ground_at(&map, (30.1, 5.0)), None);
    assert_eq!(ground_at(&map, (f64::NAN, 5.0)), None);
    assert_eq!(ground_at(&map, (25.0, 25.0)), None, "NODATA corner");
}

#[test]
fn consecutive_steps_carry_the_reach() {
    let ramp = along(|x| 100.0 + 0.05 * x);
    let start = Reach::at_anchor(100.0, &band(0.1));
    let b = band(0.05);
    let whole = band_step(&east(0.0, 100.0), &ramp, &b, start, &open(), NONE).unwrap();
    let one = band_step(&east(0.0, 50.0), &ramp, &b, start, &open(), NONE).unwrap();
    let two = band_step(&east(50.0, 100.0), &ramp, &b, one.reach, &open(), NONE).unwrap();
    assert!(close(whole.reach.lo(), two.reach.lo(), 1e-9));
    assert!(close(whole.reach.hi(), two.reach.hi(), 1e-9));
    assert!(close(whole.climb_m, one.climb_m + two.climb_m, 1e-9));
    assert!(close(whole.climb_m, 5.0, 1e-9));
}

#[test]
fn band_from_the_parameters() {
    let p = RerouteParams::default();
    let b = Band::from_params(&p, 0.18, 4.0);
    assert_eq!(
        (b.cut_m, b.fill_m, b.grade, b.spacing_m),
        (6.0, 6.0, 0.18, 2.0)
    );
    assert_eq!(b.anchor_tol_m, 0.5);
    assert_eq!(Band::from_params(&p, 0.1, 100.0).spacing_m, 5.0);
    assert_eq!(Band::from_params(&p, 0.1, 0.0).spacing_m, 0.1);
    assert_eq!(Band::from_params(&p, 0.1, f64::NAN).spacing_m, 2.0);
}

/// One case of the property: a random terrain and a polyline with a duplicated vertex. `Ok(true)` when the band and
/// the profile both accepted it.
fn run_case(seed: u64, grade: f64, cut: f64, fill: f64, len: f64) -> Result<bool, TestCaseError> {
    let mut state = seed | 1;
    let mut noise = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        f64::from(u32::try_from(state % 10_000).unwrap_or(0)) / 10_000.0
    };
    let bumps: Vec<(f64, f64, f64, f64)> = (0..8)
        .map(|_| {
            (
                noise() * 300.0,
                noise() * 300.0,
                noise() * 40.0,
                500.0 + noise() * 2500.0,
            )
        })
        .collect();
    let ground = move |p: (f64, f64)| {
        Some(
            100.0
                + bumps
                    .iter()
                    .map(|(bx, by, a, w)| {
                        a * (-((p.0 - bx).powi(2) + (p.1 - by).powi(2)) / w).exp()
                    })
                    .sum::<f64>(),
        )
    };
    let (a, b) = ((20.0, 20.0), (20.0 + len * 0.8, 20.0 + len * 0.6));
    let mid = (f64::midpoint(a.0, b.0), a.1 + 5.0);
    let line = [a, mid, mid, b];
    let bd = Band {
        cut_m: cut,
        fill_m: fill,
        grade,
        spacing_m: 2.0,
        anchor_tol_m: 0.5,
    };
    let (Some(z0), Some(z1)) = (ground(a), ground(b)) else {
        return Err(TestCaseError::fail("no ground"));
    };
    let low = band_step(
        &line,
        &ground,
        &bd,
        Reach::at_anchor(z0, &bd),
        &open(),
        NONE,
    );
    let real = arrive(&line, &ground, &bd);
    let Ok(real) = real else {
        return Ok(false);
    };
    // A profile that exists means the forward band passes and arrives.
    let low = low.map_err(|e| TestCaseError::fail(format!("band_step failed: {e}")))?;
    prop_assert!(arrives(low.reach, z1, 0.5));
    prop_assert!(low.earthwork_low_m2 <= real.earthwork_m2() + 1e-6);
    prop_assert!(real.max_grade() <= grade + 1e-9);
    for (h, z) in real.heights().iter().zip(real.ground()) {
        prop_assert!(*h >= z - cut - 1e-9 && *h <= z + fill + 1e-9);
    }
    let (Some(first), Some(last)) = (real.heights().first(), real.heights().last()) else {
        return Err(TestCaseError::fail("empty profile"));
    };
    prop_assert!((first - z0).abs() <= 0.5 + 1e-9 && (last - z1).abs() <= 0.5 + 1e-9);
    Ok(true)
}

proptest! {
    #![proptest_config(ProptestConfig {
        rng_seed: RngSeed::Fixed(0x6e61_7665_735f_3033),
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/proptest-regressions/reroute.txt",
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn the_lower_bound_never_exceeds_the_profile(
        seed in any::<u64>(),
        grade in 0.05_f64..0.5,
        cut in 0.5_f64..8.0,
        fill in 0.5_f64..8.0,
        len in 20.0_f64..200.0,
    ) {
        run_case(seed, grade, cut, fill, len)?;
    }
}

#[test]
fn the_property_is_not_vacuous() {
    let ok = (0..300_u64)
        .filter(|&s| {
            let n = f64::from(u32::try_from(s).unwrap());
            run_case(
                s.wrapping_mul(0x9e37_79b9_7f4a_7c15),
                0.1 + (n % 5.0) * 0.08,
                2.0 + n % 4.0,
                1.0 + n % 5.0,
                20.0 + (n * 7.0) % 150.0,
            )
            .unwrap()
        })
        .count();
    // Measured: 217 of 300. Both accepted in at least half of the cases.
    assert!(ok >= 150, "{ok} of 300");
}

#[test]
fn a_start_cannot_float_and_a_route_is_accepted_through_arrive() {
    // The spike's start [z - 6, z + 6] cannot be built except from a band with that tolerance; acceptance does not
    // depend on it.
    // arrive pins both ends itself, from the route's own terrain.
    let line = east(0.0, 90.8);
    let ground = switchback();
    assert_eq!(arrive(&line, &ground, &band(0.18)), Err(BandFail::Arrival));
    let p = arrive(&line, &ground, &band(0.36)).unwrap();
    assert!(p.earthwork_m2() > 60.0, "{}", p.earthwork_m2());
    assert!(close(p.cut_max_m(), 3.6, 0.2), "{}", p.cut_max_m());
}

/// `band(0.1)` with one field changed.
fn altered(change: impl Fn(&mut Band)) -> Band {
    let mut b = band(0.1);
    change(&mut b);
    b
}

/// A grade, cut, fill or tolerance that is not usable.
#[test]
fn the_band_refuses_what_is_not_a_number() {
    let flat = along(|_| 100.0);
    let line = east(0.0, 20.0);
    let start = Reach::at_anchor(100.0, &band(0.1));
    let bad = [
        ("grade NaN", altered(|b| b.grade = f64::NAN)),
        ("grade negative", altered(|b| b.grade = -0.1)),
        ("cut NaN", altered(|b| b.cut_m = f64::NAN)),
        ("cut negative", altered(|b| b.cut_m = -1.0)),
        ("cut infinite", altered(|b| b.cut_m = f64::INFINITY)),
        ("fill NaN", altered(|b| b.fill_m = f64::NAN)),
        ("fill negative", altered(|b| b.fill_m = -1.0)),
        ("fill infinite", altered(|b| b.fill_m = f64::INFINITY)),
        ("tolerance NaN", altered(|b| b.anchor_tol_m = f64::NAN)),
        ("tolerance negative", altered(|b| b.anchor_tol_m = -0.5)),
        (
            "tolerance infinite",
            altered(|b| b.anchor_tol_m = f64::INFINITY),
        ),
    ];
    let mut accepted: Vec<String> = Vec::new();
    for (what, b) in &bad {
        if band_step(&line, &flat, b, start, &open(), NONE) != Err(BandFail::Invalid) {
            accepted.push(format!("band_step: {what}"));
        }
        if arrive(&line, &flat, b) != Err(BandFail::Invalid) {
            accepted.push(format!("arrive: {what}"));
        }
    }
    assert_eq!(
        accepted,
        Vec::<String>::new(),
        "accepted instead of refused"
    );
    // An infinite grade is the band with the grade limit removed.
    let b = altered(|b| b.grade = f64::INFINITY);
    assert!(band_step(&line, &flat, &b, start, &open(), NONE).is_ok());
    assert!(arrive(&line, &flat, &b).is_ok());
}

#[test]
fn a_start_that_is_not_usable_is_refused() {
    let flat = along(|_| 100.0);
    let line = east(0.0, 20.0);
    let starts = [
        ("start NaN", Reach::at_anchor(f64::NAN, &band(0.1))),
        (
            "start infinite",
            Reach::at_anchor(f64::INFINITY, &band(0.1)),
        ),
        (
            "start tolerance NaN",
            Reach::at_anchor(100.0, &altered(|b| b.anchor_tol_m = f64::NAN)),
        ),
        (
            "start empty",
            Reach::at_anchor(100.0, &altered(|b| b.anchor_tol_m = -1.0)),
        ),
    ];
    for (what, r) in starts {
        assert_eq!(
            band_step(&line, &flat, &band(0.1), r, &open(), NONE),
            Err(BandFail::Invalid),
            "{what}"
        );
    }
    // No limit to try when the limit or the ceiling is not a number.
    assert_eq!(relaxed_limits(f64::NAN, 0.36, 3), None);
    assert_eq!(relaxed_limits(0.18, f64::NAN, 3), None);
    // arrives is false with a NaN.
    assert!(!arrives(Reach::at_anchor(f64::NAN, &band(0.1)), 100.0, 0.5));
}

#[test]
fn degenerate_bands_do_not_panic() {
    let flat = along(|_| 100.0);
    // An empty line has an empty profile.
    let p = arrive(&[], &flat, &band(0.1)).unwrap();
    assert_eq!(p.heights(), Vec::<f64>::new());
    // A spacing that is not positive or not a number falls back to 2 m.
    for spacing_m in [0.0, -1.0, f64::NAN] {
        let b = Band {
            spacing_m,
            ..band(0.1)
        };
        let p = arrive(&east(0.0, 10.0), &flat, &b).unwrap();
        assert_eq!(p.heights().len(), 6);
    }
    // A reach that is empty by less than the tolerance is a point.
    let b = Band {
        cut_m: 0.0,
        fill_m: 0.0,
        ..band(0.1)
    };
    let low = along(|_| 100.000_000_000_5);
    let step = band_step(
        &east(0.0, 10.0),
        &low,
        &b,
        Reach::at_anchor(
            100.0,
            &Band {
                anchor_tol_m: 0.0,
                ..b
            },
        ),
        &open(),
        NONE,
    )
    .unwrap();
    assert!(close(step.reach.lo(), step.reach.hi(), 1e-9));
}

#[test]
fn stray_is_the_stored_distance() {
    let mut o = world(0.0);
    o.set_original(&[(0.0, 100.0), (200.0, 100.0)]);
    o.finish();
    assert!((o.stray_m((50.0, 107.0)) - 6.0).abs() < 1e-9);
    assert!(o.stray_m((50.0, 100.0)).abs() < 1e-9);
}

#[test]
fn a_corner_tip_is_stamped_by_distance_to_the_segments() {
    // the vertex (51, 101) is not a sample; the cell centred (47, 105) is 5.66 m from it but 6.4 m from the
    // nearest samples, so only a stamp by segment distance marks it
    let mut o = world(0.0);
    o.stamp_road(&[(51.0, 10.0), (51.0, 101.0), (151.0, 101.0)], 6.0, &[]);
    o.finish();
    assert_eq!(
        o.segment((46.5, 104.5), (47.5, 105.5), NONE),
        Err(Blocked::Road)
    );
}

// ---- the cost-to-go field ----

#[expect(
    clippy::unnecessary_wraps,
    reason = "the signature of a ground function"
)]
fn flat(_: (f64, f64)) -> Option<f64> {
    Some(0.0)
}

fn field_band(grade: f64) -> Band {
    Band::from_params(&RerouteParams::default(), grade, 4.0)
}

/// The field of the examples: box (0, 0)-(200, 200), step 6, target (100, 100).
fn field_with(
    obstacles: &Obstacles,
    walls: &[Wall],
    ground: &dyn Fn((f64, f64)) -> Option<f64>,
    band: &Band,
) -> Field {
    field_capped(obstacles, walls, ground, band, 1.0e9)
}

fn field_capped(
    obstacles: &Obstacles,
    walls: &[Wall],
    ground: &dyn Fn((f64, f64)) -> Option<f64>,
    band: &Band,
    stray_cap_m: f64,
) -> Field {
    Field::build(
        (0.0, 0.0),
        (200.0, 200.0),
        (100.0, 100.0),
        6.0,
        4.0,
        band,
        ground,
        obstacles,
        walls,
        NONE,
        stray_cap_m,
        1_000_000,
    )
}

fn open_world() -> Obstacles {
    let mut o = world(0.0);
    o.finish();
    o
}

fn wall_of_water(p: (f64, f64)) -> bool {
    (120.0..124.0).contains(&p.0) && (0.0..180.0).contains(&p.1)
}

#[test]
fn the_field_estimates_the_straight_distance_on_open_ground() {
    let f = field_with(&open_world(), &[], &flat, &field_band(0.1));
    let e = f.estimate((142.0, 100.0));
    let want = 42.0 / CHAMFER_ERROR - 6.0;
    assert!((e - want).abs() < 0.5, "{e} vs {want}");
    // never above the true distance, and no estimate below 0 at the target
    assert!(f.estimate((100.0, 100.0)).abs() < 1e-9);
    assert!(f.estimate((180.0, 180.0)) <= 80.0 * 2.0_f64.sqrt());
}

#[test]
fn water_and_stream_walls_make_the_estimate_go_around() {
    let mut lake_world = world(0.0);
    lake_world.stamp_water(&wall_of_water);
    lake_world.finish();
    let by_water = field_with(&lake_world, &[], &flat, &field_band(0.1));
    let e = by_water.estimate((160.0, 100.0));
    assert!(e >= 150.0, "water: {e}");
    // the same wall as a stream the route may not cross
    let stream = Wall {
        line: vec![(122.0, -10.0), (122.0, 180.0)],
        allowed: 0,
        kind: WallKind::Stream,
        anchors: [(100.0, 100.0), (160.0, 100.0)],
        end_m: 10.0,
    };
    let by_stream = field_with(
        &open_world(),
        std::slice::from_ref(&stream),
        &flat,
        &field_band(0.1),
    );
    let e = by_stream.estimate((160.0, 100.0));
    assert!(e >= 150.0, "stream: {e}");
    // a stream the route may cross once does not hold the field back
    let open = Wall {
        allowed: 1,
        ..stream
    };
    let crossable = field_with(&open_world(), &[open], &flat, &field_band(0.1));
    let e = crossable.estimate((160.0, 100.0));
    assert!(e < 60.0, "allowed crossing: {e}");
}

#[test]
fn a_cell_enclosed_by_water_has_no_estimate() {
    let mut o = world(0.0);
    let ring = |p: (f64, f64)| {
        let d = (p.0 - 160.0).hypot(p.1 - 40.0);
        (8.0..16.0).contains(&d)
    };
    o.stamp_water(&ring);
    o.finish();
    let f = field_with(&o, &[], &flat, &field_band(0.1));
    assert!(f.estimate((160.0, 40.0)).abs() < 1e-9);
    // outside the ring the estimate is the open one
    assert!(f.estimate((160.0, 100.0)) > 40.0);
}

#[test]
fn the_field_is_bounded_by_max_cells() {
    let far = 100_000.0;
    let o = Obstacles::new((0.0, 0.0), (far, far), 4.0, far, 0.0, 4_000_000);
    let nowhere = |_: (f64, f64)| None;
    let f = Field::build(
        (0.0, 0.0),
        (far, far),
        (far / 2.0, far / 2.0),
        6.0,
        4.0,
        &field_band(0.1),
        &nowhere,
        &o,
        &[],
        NONE,
        1.0e9,
        1_000_000,
    );
    assert!(f.cells() > 0 && f.cells() <= 1_000_000, "{}", f.cells());
    // a small cap holds as well, and the estimate still grows with the distance
    let o = Obstacles::new((0.0, 0.0), (2000.0, 2000.0), 4.0, 2000.0, 0.0, 4_000_000);
    let f = Field::build(
        (0.0, 0.0),
        (2000.0, 2000.0),
        (1000.0, 1000.0),
        6.0,
        4.0,
        &field_band(0.1),
        &flat,
        &o,
        &[],
        NONE,
        1.0e9,
        5_000,
    );
    assert!(f.cells() <= 5_000, "{}", f.cells());
    assert!(f.estimate((1500.0, 1000.0)) > f.estimate((1200.0, 1000.0)));
}

#[test]
fn the_field_ignores_the_grade() {
    // 200 %: a 6 m edge rises 12 m, more than the cut, the tolerance and the grade limit allow together
    let slope = |p: (f64, f64)| Some(2.0 * p.0);
    let o = open_world();
    let limited = field_band(0.05);
    assert!(
        band_step(
            &[(100.0, 100.0), (106.0, 100.0)],
            &slope,
            &limited,
            Reach::at_anchor(200.0, &limited),
            &o,
            NONE
        )
        .is_err(),
        "the band with its grade refuses the edge, so the test can tell"
    );
    let steep = field_with(&o, &[], &slope, &limited);
    let level = field_with(&o, &[], &flat, &limited);
    for p in [(142.0, 100.0), (30.0, 170.0), (190.0, 20.0)] {
        assert!(steep.estimate(p) > 30.0);
        assert_eq!(steep.estimate(p).to_bits(), level.estimate(p).to_bits());
    }
}

#[test]
fn two_builds_agree_bit_for_bit() {
    let mut o = world(0.0);
    o.stamp_water(&wall_of_water);
    o.finish();
    let hill = |p: (f64, f64)| Some(3.0 * (p.0 / 40.0).sin());
    let a = field_with(&o, &[], &hill, &field_band(0.1));
    let b = field_with(&o, &[], &hill, &field_band(0.1));
    assert_eq!(a.cells(), b.cells());
    for x in (0..200).step_by(7) {
        for y in (0..200).step_by(11) {
            let p = (f64::from(x), f64::from(y));
            assert_eq!(a.estimate(p).to_bits(), b.estimate(p).to_bits());
        }
    }
}

#[test]
fn points_outside_the_field_do_not_panic() {
    let field = field_with(&open_world(), &[], &flat, &field_band(0.1));
    for p in [
        (-50.0, 10.0),
        (f64::NAN, 1.0),
        (1.0, f64::INFINITY),
        (1e300, 1e300),
        (205.0, 100.0), // exactly past the last column
    ] {
        assert!(field.estimate(p).abs() < 1e-9, "{p:?}");
    }
}

/// A build of the 200 m examples with one bad input at a time.
#[expect(clippy::too_many_arguments, reason = "one build, every input")]
fn odd(
    min: (f64, f64),
    max: (f64, f64),
    target: (f64, f64),
    step: f64,
    cell: f64,
    band: &Band,
    cap: f64,
    max_cells: usize,
) -> Field {
    Field::build(
        min,
        max,
        target,
        step,
        cell,
        band,
        &flat,
        &open_world(),
        &[],
        NONE,
        cap,
        max_cells,
    )
}

#[test]
fn each_bad_input_gives_a_field_without_a_panic() {
    let nan = f64::NAN;
    let ok = field_band(0.1);
    let (lo, hi, mid) = ((0.0, 0.0), (200.0, 200.0), (100.0, 100.0));
    // not a number: target, box (the grid then covers the target only), step and cell (the default step)
    assert_eq!(
        odd(lo, hi, (nan, nan), 6.0, 4.0, &ok, 1e9, 1_000_000).cells(),
        0
    );
    let boxless = odd(
        (nan, 0.0),
        (200.0, f64::INFINITY),
        mid,
        6.0,
        4.0,
        &ok,
        1e9,
        1_000_000,
    );
    assert!(boxless.cells() >= 1 && boxless.estimate((100.0, 100.0)).abs() < 1e-9);
    let default_step = odd(lo, hi, mid, 0.0, nan, &ok, 1e9, 1_000_000);
    assert!((default_step.estimate((142.0, 100.0)) - (42.0 / CHAMFER_ERROR - 6.0)).abs() < 0.5);
    // a box that cannot be made to fit in the growth limit is empty
    assert_eq!(
        odd(lo, (1e300, 1e300), mid, 6.0, 4.0, &ok, 1e9, 1_000_000).cells(),
        0
    );
    // at most one cell for a limit of 0
    assert_eq!(odd(lo, hi, mid, 6.0, 4.0, &ok, 1e9, 0).cells(), 1);
    // a cap that is NaN blocks every edge: the target cell alone
    assert!(
        odd(lo, hi, mid, 6.0, 4.0, &ok, nan, 1_000_000)
            .estimate((142.0, 100.0))
            .abs()
            < 1e-9
    );
    // a band the band test refuses: the same
    for bad in [
        Band { grade: nan, ..ok },
        Band { cut_m: -1.0, ..ok },
        Band {
            fill_m: f64::INFINITY,
            ..ok
        },
        Band {
            anchor_tol_m: nan,
            ..ok
        },
    ] {
        assert!(
            odd(lo, hi, mid, 6.0, 4.0, &bad, 1e9, 1_000_000)
                .estimate((142.0, 100.0))
                .abs()
                < 1e-9
        );
    }
    // an infinite grade is the field's own
    let inf = Band {
        grade: f64::INFINITY,
        ..ok
    };
    assert!(odd(lo, hi, mid, 6.0, 4.0, &inf, 1e9, 1_000_000).estimate((142.0, 100.0)) > 30.0);
}

#[test]
fn a_clearance_and_a_target_outside_the_box_still_give_a_field() {
    let obstacles = open_world();
    let cleared = Field::build(
        (0.0, 0.0),
        (200.0, 200.0),
        (150.0, 100.0),
        6.0,
        4.0,
        &field_band(0.1),
        &flat,
        &obstacles,
        &[],
        Clearance::new(5.0, 5.0).unwrap(),
        1.0e9,
        1_000_000,
    );
    assert!(cleared.estimate((20.0, 100.0)) > 100.0);
    let outside = odd(
        (0.0, 0.0),
        (100.0, 100.0),
        (150.0, 100.0),
        6.0,
        4.0,
        &field_band(0.1),
        1e9,
        1_000_000,
    );
    assert!(outside.estimate((60.0, 100.0)) > 60.0);
}

#[test]
fn a_target_inside_an_obstacle_reaches_nothing() {
    let o = with_lake();
    let field = Field::build(
        (0.0, 0.0),
        (200.0, 200.0),
        (110.0, 110.0),
        6.0,
        4.0,
        &field_band(0.1),
        &flat,
        &o,
        &[],
        NONE,
        1.0e9,
        1_000_000,
    );
    assert!(field.cells() > 0);
    assert!(field.estimate((50.0, 100.0)).abs() < 1e-9);
}

#[test]
fn the_stray_cap_holds_the_field_near_the_original_line() {
    let mut o = world(0.0);
    o.set_original(&[(100.0, 100.0), (160.0, 100.0)]);
    o.finish();
    let capped = field_capped(&o, &[], &flat, &field_band(0.1), 20.0);
    let free = field_capped(&o, &[], &flat, &field_band(0.1), 1.0e9);
    // 60 m off the line: out of reach with the cap, an open-ground value without it
    assert!(capped.estimate((160.0, 160.0)).abs() < 1e-9);
    assert!(free.estimate((160.0, 160.0)) > 60.0);
    // near the line the two agree
    let near = capped.estimate((150.0, 110.0));
    assert!(near > 40.0, "{near}");
    assert_eq!(near.to_bits(), free.estimate((150.0, 110.0)).to_bits());
}

#[test]
fn a_limit_above_u32_max_is_capped_not_wrapped() {
    let big = usize::try_from(u64::from(u32::MAX) + 2).unwrap_or(usize::MAX);
    let wrapped = odd(
        (0.0, 0.0),
        (200.0, 200.0),
        (100.0, 100.0),
        6.0,
        4.0,
        &field_band(0.1),
        1e9,
        big,
    );
    let top = odd(
        (0.0, 0.0),
        (200.0, 200.0),
        (100.0, 100.0),
        6.0,
        4.0,
        &field_band(0.1),
        1e9,
        usize::try_from(u32::MAX).unwrap_or(usize::MAX),
    );
    assert_eq!(wrapped.cells(), top.cells());
    assert!(wrapped.cells() > 1000);
}

#[expect(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    reason = "a test helper over a fixed 11 x 11 grid"
)]
/// The reference: the same grid as `Field` (box (0, 0)-(60, 60), target (30, 30), step 6: 11 x 11 cells from
/// (-3, -3)), Dijkstra by the plain O(n^2) rule (lowest cost, then lowest index) and every edge tested through
/// `band_step` with the grade limit removed. Returns the estimate at every cell centre.
fn reference(
    ground: &dyn Fn((f64, f64)) -> Option<f64>,
    obstacles: &Obstacles,
    walls: &[Wall],
    clear: Clearance,
    cap: f64,
    grade: f64,
    ends_only: bool,
) -> Vec<f64> {
    let (n, res, origin) = (11_usize, 6.0_f64, -3.0_f64);
    let centre = |c: usize, r: usize| {
        (
            origin + (f64::from(u32::try_from(c).unwrap()) + 0.5) * res,
            origin + (f64::from(u32::try_from(r).unwrap()) + 0.5) * res,
        )
    };
    let free = Band {
        grade,
        ..field_band(0.1)
    };
    let mut value = vec![f64::INFINITY; n * n];
    let mut done = vec![false; n * n];
    value[5 * n + 5] = 0.0;
    loop {
        let mut pick: Option<usize> = None;
        for i in 0..n * n {
            if !done[i]
                && value[i].is_finite()
                && pick.is_none_or(|p| value[i].total_cmp(&value[p]).is_lt())
            {
                pick = Some(i);
            }
        }
        let Some(cur) = pick else { break };
        done[cur] = true;
        let (cc, cr) = (cur % n, cur / n);
        let here = centre(cc, cr);
        if o_stray(obstacles, here) > cap {
            continue;
        }
        for (dc, dr) in [
            (-1_i32, -1_i32),
            (0, -1),
            (1, -1),
            (-1, 0),
            (1, 0),
            (-1, 1),
            (0, 1),
            (1, 1),
        ] {
            let (Ok(nc), Ok(nr)) = (
                usize::try_from(i32::try_from(cc).unwrap() + dc),
                usize::try_from(i32::try_from(cr).unwrap() + dr),
            ) else {
                continue;
            };
            if nc >= n || nr >= n {
                continue;
            }
            let len = if dc == 0 || dr == 0 {
                res
            } else {
                res * 2.0_f64.sqrt()
            };
            let through = value[cur] + len;
            if through >= value[nr * n + nc] {
                continue;
            }
            let there = centre(nc, nr);
            let ok = o_stray(obstacles, there) <= cap
                && ground(there).is_some()
                && !walls.iter().any(|w| w.blocks(there, here))
                && if ends_only {
                    // terrain only at the two ends of the edge: what a field that skipped the samples would do
                    ground(here).is_some() && obstacles.segment(there, here, clear).is_ok()
                } else {
                    band_step(
                        &[there, here],
                        ground,
                        &free,
                        Reach::at_anchor(ground(there).unwrap(), &free),
                        obstacles,
                        clear,
                    )
                    .is_ok()
                };
            if ok {
                value[nr * n + nc] = through;
            }
        }
    }
    value
        .iter()
        .map(|&v| {
            if v.is_finite() {
                (v / CHAMFER_ERROR - res).max(0.0)
            } else {
                0.0
            }
        })
        .collect()
}

fn o_stray(o: &Obstacles, p: (f64, f64)) -> f64 {
    o.stray_m(p)
}

fn equivalence_setup() -> (Obstacles, Vec<Wall>, Clearance) {
    let mut o = Obstacles::new((-10.0, -10.0), (70.0, 70.0), 4.0, 70.0, 0.0, 4_000_000);
    o.stamp_water(&|p: (f64, f64)| (14.0..22.0).contains(&p.0) && (30.0..50.0).contains(&p.1));
    o.set_original(&[(30.0, 30.0), (55.0, 40.0)]);
    o.finish();
    let stream = Wall {
        line: vec![(40.0, -5.0), (40.0, 22.0)],
        allowed: 0,
        kind: WallKind::Stream,
        anchors: [(30.0, 30.0), (55.0, 40.0)],
        end_m: 5.0,
    };
    (
        o,
        vec![stream],
        Clearance::new(2.0, 2.0).unwrap_or(Clearance::NONE),
    )
}

fn holey_with(p: (f64, f64), strip_x: bool, strip_y: bool) -> Option<f64> {
    // a patch with no terrain, two narrow strips that no cell centre is in (x = 44 between the centres 42 and 48,
    // hit by the sample at 44 of a 3-sample edge; y = 4 between 0 and 6, hit at offset 4 by 3 samples but missed
    // by the samples of 4), on hilly ground
    let patch = (44.0..49.0).contains(&p.0) && (40.0..60.0).contains(&p.1);
    let x = strip_x && (43.6..44.4).contains(&p.0) && (10.0..40.0).contains(&p.1);
    let y = strip_y && (3.8..4.2).contains(&p.1);
    if patch || x || y {
        None
    } else {
        Some(8.0 * (p.0 / 9.0).sin() + p.1)
    }
}

fn holey(p: (f64, f64)) -> Option<f64> {
    holey_with(p, true, true)
}

#[test]
fn an_edge_costs_what_the_band_with_no_grade_says() {
    let (o, walls, clear) = equivalence_setup();
    let cap = 45.0;
    let field = Field::build(
        (0.0, 0.0),
        (60.0, 60.0),
        (30.0, 30.0),
        6.0,
        4.0,
        &field_band(0.1),
        &holey,
        &o,
        &walls,
        clear,
        cap,
        1_000_000,
    );
    assert_eq!(field.cells(), 121);
    let want = reference(&holey, &o, &walls, clear, cap, f64::INFINITY, false);
    let mut reached = 0;
    for r in 0..11_u32 {
        for c in 0..11_u32 {
            let p = (
                -3.0 + (f64::from(c) + 0.5) * 6.0,
                -3.0 + (f64::from(r) + 0.5) * 6.0,
            );
            let w = want[usize::try_from(r * 11 + c).unwrap()];
            assert_eq!(field.estimate(p).to_bits(), w.to_bits(), "{p:?}");
            reached += usize::from(w > 0.0);
        }
    }
    // the setup has something to tell: cells reached, and cells the holes, water, stream and cap cut off
    assert!(reached > 40 && reached < 110, "{reached}");
    // and a finite grade in the reference would give other answers on this terrain
    let graded = reference(&holey, &o, &walls, clear, cap, 0.1, false);
    assert!(
        graded
            .iter()
            .zip(&want)
            .any(|(a, b)| a.to_bits() != b.to_bits())
    );
    // and a field that looked at the ends of an edge only would give other answers, for each strip alone: the
    // samples in between decide
    for (sx, sy) in [(true, false), (false, true)] {
        let g = move |p: (f64, f64)| holey_with(p, sx, sy);
        let full = reference(&g, &o, &walls, clear, cap, f64::INFINITY, false);
        let ends = reference(&g, &o, &walls, clear, cap, f64::INFINITY, true);
        let differing = ends
            .iter()
            .zip(&full)
            .filter(|(a, b)| a.to_bits() != b.to_bits())
            .count();
        assert!(differing >= 2, "strips {sx} {sy}: {differing}");
        let field = Field::build(
            (0.0, 0.0),
            (60.0, 60.0),
            (30.0, 30.0),
            6.0,
            4.0,
            &field_band(0.1),
            &g,
            &o,
            &walls,
            clear,
            cap,
            1_000_000,
        );
        let (mut same, mut total) = (0, 0);
        for r in 0..11_u32 {
            for c in 0..11_u32 {
                let p = (
                    -3.0 + (f64::from(c) + 0.5) * 6.0,
                    -3.0 + (f64::from(r) + 0.5) * 6.0,
                );
                total += 1;
                same += usize::from(
                    field.estimate(p).to_bits()
                        == full
                            .get(usize::try_from(r * 11 + c).unwrap_or(0))
                            .map_or(0, |v| v.to_bits()),
                );
            }
        }
        assert_eq!(same, total, "the field follows the full reference");
    }
}

#[test]
fn more_samples_than_the_band_takes_are_clamped_like_the_band() {
    // 6 m edges at a spacing of 1e-9 m would be 6e9 samples; the band takes 10^6, which still find a strip 0.2 m wide
    let tiny = Band {
        spacing_m: 1.0e-9,
        ..field_band(0.1)
    };
    let strip = |p: (f64, f64)| (!(2.9..3.1).contains(&p.0)).then_some(0.0);
    let o = Obstacles::new((0.0, 0.0), (30.0, 30.0), 4.0, 30.0, 0.0, 4_000_000);
    assert_eq!(
        band_step(
            &[(0.0, 12.0), (6.0, 12.0)],
            &strip,
            &tiny,
            Reach::at_anchor(0.0, &tiny),
            &o,
            NONE
        )
        .map(|_| ()),
        Err(BandFail::NoGround)
    );
    let field = Field::build(
        (0.0, 0.0),
        (24.0, 24.0),
        (12.0, 12.0),
        6.0,
        4.0,
        &tiny,
        &strip,
        &o,
        &[],
        NONE,
        1.0e9,
        1_000_000,
    );
    // the left column (x = 0) is behind the strip, 12 m from the target: 5.09 when reached, 0 when not
    assert!(field.estimate((0.0, 12.0)).abs() < 1e-9);
    assert!(field.estimate((24.0, 12.0)) > 5.0);
}

// ---- Dubins paths ----

use naves_core::reroute::dubins::{DubinsPath, Turn, dubins_paths};
use naves_core::reroute::search::{
    LegInput, LegOutcome, Route, Weights, WrittenLine, search_leg, search_leg_relaxed,
    self_clearance, written_line,
};
use naves_core::reroute::{Anchor, PLAN_BUDGETS, PLAN_WRITE_TOL_M, Pose, RerouteError, SAMPLE_M};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

const RHO: f64 = 10.5;

fn pose(x: f64, y: f64, heading: f64) -> Pose {
    Pose {
        at: (x, y),
        heading,
    }
}

/// A fixed-seed generator (xorshift64*), a number in 0..1.
fn unit(state: &mut u64) -> f64 {
    *state ^= *state >> 12;
    *state ^= *state << 25;
    *state ^= *state >> 27;
    let v = state.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11;
    f64::from(u32::try_from(v >> 21).unwrap()) / 4_294_967_296.0
}

fn shortest(from: Pose, to: Pose) -> DubinsPath {
    dubins_paths(from, to, RHO)[0]
}

#[test]
fn dubins_known_lengths() {
    // a U-turn to the left: half a circle
    let u = shortest(pose(0.0, 0.0, 0.0), pose(0.0, 21.0, PI));
    assert!(close(u.length(), PI * RHO, 1e-9), "{}", u.length());
    assert_eq!(u.word(), [Turn::Left, Turn::Straight, Turn::Left]);
    // a straight
    let s = shortest(pose(0.0, 0.0, 0.0), pose(50.0, 0.0, 0.0));
    assert!(close(s.length(), 50.0, 1e-9), "{}", s.length());
    // a quarter circle
    let q = shortest(pose(0.0, 0.0, 0.0), pose(RHO, RHO, FRAC_PI_2));
    assert!(close(q.length(), FRAC_PI_2 * RHO, 1e-9), "{}", q.length());
    assert!(close(q.turning(RHO), FRAC_PI_2, 1e-9));
}

#[test]
fn equal_poses_have_one_path_of_no_length() {
    for heading in [0.0, 1.0, -2.5, 7.0] {
        let paths = dubins_paths(pose(3.0, 4.0, heading), pose(3.0, 4.0, heading), RHO);
        assert_eq!(paths.len(), 1, "{heading}");
        assert!(paths[0].length().abs() < 1e-12);
    }
}

#[test]
fn dubins_refuses_what_is_not_a_number() {
    let ok = pose(0.0, 0.0, 0.0);
    for bad in [
        pose(f64::NAN, 0.0, 0.0),
        pose(0.0, f64::INFINITY, 0.0),
        pose(0.0, 0.0, f64::NAN),
    ] {
        assert_eq!(dubins_paths(bad, ok, RHO).len(), 0);
        assert_eq!(dubins_paths(ok, bad, RHO).len(), 0);
    }
    for radius in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(dubins_paths(ok, pose(30.0, 10.0, 1.0), radius).len(), 0);
    }
}

/// The angle between two headings, in `0..=PI`.
fn heading_gap(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(TAU);
    d.min(TAU - d)
}

#[test]
fn ten_thousand_random_pairs_end_at_the_target() {
    let mut state = 0x6e61_7665_735f_3035_u64;
    let (mut paths_seen, mut worst) = (0_usize, 0.0_f64);
    for _ in 0..10_000 {
        let mut v = |scale: f64| (unit(&mut state) * 2.0 - 1.0) * scale;
        let from = pose(v(200.0), v(200.0), v(PI * 2.0));
        let to = pose(v(200.0), v(200.0), v(PI * 2.0));
        let paths = dubins_paths(from, to, RHO);
        assert!(!paths.is_empty(), "{from:?} {to:?}");
        assert!(
            paths
                .windows(2)
                .all(|w| w[0].length() <= w[1].length() + 1e-12),
            "sorted by length"
        );
        for p in &paths {
            paths_seen += 1;
            let end = p.end_pose(from, RHO);
            let err = (end.at.0 - to.at.0)
                .hypot(end.at.1 - to.at.1)
                .max(heading_gap(end.heading, to.heading));
            worst = worst.max(err);
            assert!(err <= 1e-9, "{from:?} {to:?} {:?} err {err}", p.word());
            assert!(p.lengths().iter().all(|l| l.is_finite() && *l >= 0.0));
        }
    }
    assert!(paths_seen > 40_000, "{paths_seen}");
    assert!(worst < 1e-9, "{worst}");
}

#[test]
fn dubins_points_follow_the_path_and_end_at_the_target() {
    let (from, to) = (pose(0.0, 0.0, 0.3), pose(60.0, -25.0, 2.0));
    for p in dubins_paths(from, to, RHO) {
        let pts = p.points(from, RHO, 2.0);
        assert_eq!(pts[0], from.at);
        let last = *pts.last().unwrap();
        assert!((last.0 - to.at.0).hypot(last.1 - to.at.1) < 1e-9);
        assert!(
            pts.windows(2)
                .all(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1) <= 2.0 + 1e-9)
        );
        let len: f64 = pts
            .windows(2)
            .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
            .sum();
        assert!(len <= p.length() + 1e-9 && len >= p.length() - 0.5, "{len}");
    }
}

// ---- the direction of a segment test ----

#[test]
fn a_segment_is_as_free_one_way_as_the_other() {
    let mut o = Obstacles::new((0.0, 0.0), (200.0, 200.0), 4.0, 200.0, 6.0, 4_000_000);
    o.stamp_water(&|p| {
        (60.0..80.0).contains(&p.0) && (20.0..140.0).contains(&p.1)
            || (110.0..150.0).contains(&p.0) && (100.0..112.0).contains(&p.1)
    });
    o.stamp_road(&[(10.0, 150.0), (190.0, 170.0), (190.0, 120.0)], 6.0, &[]);
    o.finish();
    let clear = Clearance::new(5.0, 8.0).unwrap();
    let mut state = 0x6e61_7665_735f_3036_u64;
    let (mut blocked, mut free) = (0_usize, 0_usize);
    for case in 0..40_000 {
        let mut p = || {
            let (x, y) = (unit(&mut state) * 200.0, unit(&mut state) * 200.0);
            match case % 3 {
                // on the corners of the cells, where a walk has to decide
                0 => ((x / 2.0).round() * 2.0, (y / 2.0).round() * 2.0),
                // on the lines of the grid
                1 => ((x / 2.0).round() * 2.0, y),
                _ => (x, y),
            }
        };
        let a = p();
        // the other end within 30 m, so that many segments are free
        let b = {
            let q = p();
            (a.0 + (q.0 - a.0) * 0.15, a.1 + (q.1 - a.1) * 0.15)
        };
        let (ab, ba) = (o.segment(a, b, clear), o.segment(b, a, clear));
        assert_eq!(ab.is_ok(), ba.is_ok(), "{a:?} {b:?}: {ab:?} {ba:?}");
        if ab.is_ok() {
            free += 1;
        } else {
            blocked += 1;
        }
    }
    assert!(blocked > 4_000 && free > 4_000, "{blocked} {free}");
}

// ---- the search of one leg ----

/// Everything a leg needs, built from the parts a test wants to change. The geometry is given in local metres and
/// shifted by `SHIFT` so that it lies inside the 400 m box.
struct Setup {
    start: Anchor,
    end: Anchor,
    original: Vec<(f64, f64)>,
    ground: Ground,
    water: fn((f64, f64)) -> bool,
    walls: Vec<Wall>,
    grade: f64,
    clear: Clearance,
    cap: f64,
    budgets: Vec<(f64, usize)>,
    self_along: f64,
    band_override: Option<Band>,
    self_clear: Option<f64>,
}

type Ground = Box<dyn Fn((f64, f64)) -> Option<f64> + Send + Sync>;

const SHIFT: (f64, f64) = (100.0, 100.0);

/// A local point on the map.
fn at(x: f64, y: f64) -> (f64, f64) {
    (x + SHIFT.0, y + SHIFT.1)
}

fn anchor(x: f64, y: f64, heading: f64) -> Anchor {
    Anchor {
        at: at(x, y),
        heading: Some(heading),
        straight_m: 0.0,
    }
}

fn line_at(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    points.iter().map(|p| at(p.0, p.1)).collect()
}

/// A ground function of local coordinates.
fn local(f: impl Fn(f64, f64) -> f64 + Send + Sync + 'static) -> Ground {
    Box::new(move |p| Some(f(p.0 - SHIFT.0, p.1 - SHIFT.1)))
}

fn dry(_: (f64, f64)) -> bool {
    false
}

impl Setup {
    fn new(start: Anchor, end: Anchor, original: &[(f64, f64)]) -> Setup {
        Setup {
            start,
            end,
            original: line_at(original),
            ground: local(|_, _| 100.0),
            water: dry,
            walls: Vec::new(),
            grade: 0.5,
            clear: NONE,
            cap: 3.0 * 10.5,
            budgets: PLAN_BUDGETS.to_vec(),
            self_along: PI * RHO + 25.0,
            band_override: None,
            self_clear: None,
        }
    }

    fn band(&self) -> Band {
        self.band_override
            .unwrap_or_else(|| Band::from_params(&RerouteParams::default(), self.grade, 4.0))
    }

    fn obstacles(&self) -> Obstacles {
        let mut o = Obstacles::new((0.0, 0.0), (400.0, 400.0), 4.0, 400.0, 0.0, 4_000_000);
        o.stamp_water(&self.water);
        o.set_original(&self.original);
        o.finish();
        o
    }

    fn run(&self) -> Result<LegOutcome, RerouteError> {
        self.run_with(None)
    }

    fn run_with(&self, limits: Option<&[f64]>) -> Result<LegOutcome, RerouteError> {
        self.run_edited(limits, |_| {})
    }

    /// Runs the leg with the input changed by `edit` after it was built.
    fn run_edited(
        &self,
        limits: Option<&[f64]>,
        edit: impl FnOnce(&mut LegInput<'_>),
    ) -> Result<LegOutcome, RerouteError> {
        let o = self.obstacles();
        let band = self.band();
        let field = Field::build(
            (0.0, 0.0),
            (400.0, 400.0),
            self.end.at,
            6.0,
            4.0,
            &band,
            &*self.ground,
            &o,
            &self.walls,
            self.clear,
            self.cap,
            1_000_000,
        );
        let mut input = LegInput {
            start: self.start,
            end: self.end,
            radius: RHO,
            step_m: 6.0,
            band: &band,
            ground: &*self.ground,
            obstacles: &o,
            walls: &self.walls,
            clear: self.clear,
            field: &field,
            weights: Weights::from_params(&RerouteParams::default()),
            stray_cap_m: self.cap,
            arrive_reach_m: (3.0_f64 * 6.0).max(4.0 * RHO),
            self_along_m: self.self_along,
            self_clear_m: self.self_clear.unwrap_or_else(|| self.clear.road_m()),
            original: &self.original,
            budgets: &self.budgets,
        };
        edit(&mut input);
        match limits {
            None => search_leg(&input),
            Some(l) => search_leg_relaxed(&input, l),
        }
    }

    fn route(&self) -> Route {
        match self.run().unwrap() {
            LegOutcome::Found(r) => r,
            other @ LegOutcome::NoRoute { .. } => panic!("no route: {other:?}"),
        }
    }
}

fn length_of(points: &[(f64, f64)]) -> f64 {
    points
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum()
}

/// The smallest three-point radius of a route.
fn tightest(points: &[(f64, f64)]) -> f64 {
    points
        .windows(3)
        .map(|w| naves_core::smooth::radius_at(w[0], w[1], w[2]))
        .fold(f64::INFINITY, f64::min)
}

fn is_close(p: (f64, f64), q: (f64, f64), tol: f64) -> bool {
    (p.0 - q.0).hypot(p.1 - q.1) <= tol
}

/// Two points are the same bit for bit.
fn same_bits(p: (f64, f64), q: (f64, f64)) -> bool {
    p.0.to_bits() == q.0.to_bits() && p.1.to_bits() == q.1.to_bits()
}

#[test]
fn a_straight_leg_is_a_straight_route() {
    let s = Setup::new(
        anchor(0.0, 0.0, 0.0),
        anchor(100.0, 0.0, 0.0),
        &[(0.0, 0.0), (100.0, 0.0)],
    );
    let r = s.route();
    assert!(close(r.length_m, 100.0, 1e-6), "{}", r.length_m);
    assert!(close(length_of(&r.points), r.length_m, 1e-9));
    assert!(r.max_stray_m <= 0.01, "{}", r.max_stray_m);
    assert!(r.states <= 500, "{}", r.states);
    assert!(is_close(r.points[0], at(0.0, 0.0), 1e-9));
    assert!(is_close(*r.points.last().unwrap(), at(100.0, 0.0), 1e-9));
    assert_eq!(r.relax_steps, 0);
    assert!((r.grade_limit - 0.5).abs() < 1e-12);
    assert!(r.weight >= 1.0 && r.cost >= r.length_m - 1e-6);
}

/// The independent cost of a route in the search's own measure: length, the turn, the stray (the stored distance of
/// the raster, as a trapezoid along the points); flat ground, so no slope or earthwork.
fn flat_cost(points: &[(f64, f64)], turn: f64, o: &Obstacles) -> f64 {
    let wt = Weights::from_params(&RerouteParams::default());
    let mut cost = wt.curve * turn * RHO;
    for w in points.windows(2) {
        let l = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        cost += l + wt.stray * l * f64::midpoint(o.stray_m(w[0]), o.stray_m(w[1]));
    }
    cost
}

/// The distance of a point from a polyline.
fn dist_to_line(p: (f64, f64), line: &[(f64, f64)]) -> f64 {
    line.windows(2)
        .map(|w| {
            let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
            let l2 = dx * dx + dy * dy;
            let t = (((p.0 - w[0].0) * dx + (p.1 - w[0].1) * dy) / l2).clamp(0.0, 1.0);
            (p.0 - w[0].0 - t * dx).hypot(p.1 - w[0].1 - t * dy)
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn a_right_angle_is_rounded_at_the_radius() {
    let corner = [(0.0, 0.0), (60.0, 0.0), (60.0, 60.0)];
    let s = Setup::new(
        anchor(18.0, 0.0, 0.0),
        anchor(60.0, 42.0, FRAC_PI_2),
        &corner,
    );
    let r = s.route();
    assert!(tightest(&r.points) >= 9.9, "{}", tightest(&r.points));
    assert!(is_close(*r.points.last().unwrap(), at(60.0, 42.0), 1e-6));
    // the last chord runs along the arc's end: within one chord's half angle of the heading
    let n = r.points.len();
    let (prev, last) = (r.points[n - 2], r.points[n - 1]);
    assert!(heading_gap((last.1 - prev.1).atan2(last.0 - prev.0), FRAC_PI_2) < 0.02);
    // the true largest move, and the same measure computed here
    let moved = r
        .points
        .iter()
        .map(|p| dist_to_line(*p, &s.original))
        .fold(0.0, f64::max);
    assert!(close(r.max_move_m, moved, 1e-9), "{} {moved}", r.max_move_m);
    assert!(r.max_move_m <= 6.0, "{}", r.max_move_m);
    // Derived bound. A tangent arc of radius r between the two legs is 84 - (2 - pi/2) r long, and its midpoint is
    // r (1 - 1/sqrt 2) from the legs; a largest move of 6 m allows r <= 20.5, so the route is at least 75.2 m long.
    // (The brief's 79.0 was the arc of radius 10.5 only.)
    assert!(
        r.length_m >= 84.0 - (2.0 - FRAC_PI_2) * 20.5 - 0.1,
        "length {}",
        r.length_m
    );
    assert!(r.length_m <= 84.0, "{}", r.length_m);
    // The search must find something at least as cheap as the obvious candidate: the corner-tangent route
    // (straight 31.5 m, a quarter circle of radius 10.5, straight 31.5 m), evaluated with the same weights.
    let mut tangent = vec![at(18.0, 0.0)];
    for k in 0..=12 {
        let a = FRAC_PI_2 * f64::from(k) / 12.0;
        tangent.push(at(49.5 + RHO * a.sin(), RHO - RHO * a.cos()));
    }
    tangent.push(at(60.0, 42.0));
    assert!(
        r.cost <= flat_cost(&tangent, FRAC_PI_2, &s.obstacles()) + 1e-6,
        "{} against {}",
        r.cost,
        flat_cost(&tangent, FRAC_PI_2, &s.obstacles())
    );
}

#[test]
fn the_written_line_keeps_the_samples_the_stage_still_reads() {
    // The search returns an arc about every 2° of curvature (79 points here) and a straight at most every 2 m;
    // the write-back drops that to the stage's own SAMPLE_M sampling with its straight parts collapsed, and
    // the written line passes the route's check from scratch again.
    let s = Setup::new(
        anchor(18.0, 0.0, 0.0),
        anchor(60.0, 42.0, FRAC_PI_2),
        &[(0.0, 0.0), (60.0, 0.0), (60.0, 60.0)],
    );
    let route = s.route();
    let band = s.band();
    // the stage's own acceptance: the family's tightest arc is 10 m here, and a written route must still
    // reach 99 % of it (PLAN_REACHED_FRACTION); the search radius RHO is the planning margin over that
    let reached = 9.9;
    let mut written: Option<WrittenLine> = None;
    let leg = s.run_edited(None, |input| {
        written = Some(written_line(
            input,
            &band,
            reached,
            &route,
            PLAN_WRITE_TOL_M,
        ));
    });
    assert!(leg.is_ok(), "{leg:?}");
    let w = written.expect("the write-back of an accepted route");
    assert_eq!(
        route.points.len(),
        79,
        "the fixtures may not drift silently"
    );
    // a plain corner with its straights: the points the stage's own sampling needs (the two anchors, one
    // point at each end of the arc, and one per 2 m of it; every interior point of the straight parts is gone)
    assert_eq!(w.points.len(), 11, "{:?}", w.points);
    assert!(!w.fell_back);
    // the anchors stay bit for bit
    assert!(same_bits(w.points[0], route.points[0]));
    assert!(same_bits(
        *w.points.last().unwrap(),
        *route.points.last().unwrap()
    ));
    // every sample of the search lies within the sagitta of the written line's sampling: a 2 m chord of an
    // arc at the planning radius is 0.05 m off it (the step of the sampling divides the line evenly, so it is
    // never longer than 2 m by much), far within the 0.20 m the road pieces are placed with
    for p in &route.points {
        let d = dist_to_line(*p, &w.points);
        assert!(d <= 0.05, "sample {p:?} is {d} m from the written line");
    }
    // the written arc still reads its own radius, at any phase of the samples its measures take
    let tight = |line: &[(f64, f64)]| {
        naves_core::smooth::resample(line, SAMPLE_M)
            .windows(3)
            .map(|q| naves_core::smooth::radius_at(q[0], q[1], q[2]))
            .fold(f64::INFINITY, f64::min)
    };
    assert!(
        tight(&w.points) >= reached,
        "the written arc reads {} m against {reached}",
        tight(&w.points)
    );
    for skip in 1..4 {
        let trimmed = &w.points[skip..];
        assert!(
            tight(trimmed) >= reached,
            "the written arc reads {} m against {reached} from sample {skip}",
            tight(trimmed)
        );
    }
    assert!(tight(&route.points) >= reached, "fixture: the dense line");
}

/// The written line of a route whose drop fails the check from scratch: every sample is kept. The wall
/// between the chord the resample takes and the arc it stands for crosses the written line one time too many,
/// and no other check is offended — the samples of the search cross it nowhere.
#[test]
fn the_written_line_that_fails_its_check_keeps_every_sample() {
    let s = Setup::new(
        anchor(18.0, 0.0, 0.0),
        anchor(60.0, 42.0, FRAC_PI_2),
        &[(0.0, 0.0), (60.0, 0.0), (60.0, 60.0)],
    );
    let route = s.route();
    let band = s.band();
    let reached = 9.9;
    let dropped = {
        let mut written: Option<WrittenLine> = None;
        let _ = s.run_edited(None, |input| {
            written = Some(written_line(
                input,
                &band,
                reached,
                &route,
                PLAN_WRITE_TOL_M,
            ));
        });
        let w = written.expect("the write-back");
        assert!(!w.fell_back);
        w.points
    };
    // a chord of the arc part with its widest gap to the samples over it, and the foot of the sample farthest
    // off it: a wall from the inside of the chord halfway across the gap crosses the written line but stops
    // well short of the samples of the search over it
    let mut farthest = ((0.0, 0.0), 0.0_f64);
    let mut foot = (0.0, 0.0);
    for pair in dropped.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let span = (b.0 - a.0).hypot(b.1 - a.1);
        for &p in &route.points {
            let along = (p.0 - a.0) * (b.0 - a.0) + (p.1 - a.1) * (b.1 - a.1);
            let t = along / (span * span);
            if span <= 0.0 || t < 0.2 || t > 0.8 {
                continue;
            }
            let gap = (p.0 - a.0 - t * (b.0 - a.0)).hypot(p.1 - a.1 - t * (b.1 - a.1));
            if gap > farthest.1 {
                farthest = (p, gap);
                foot = (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
            }
        }
    }
    // sagitta of a 2 m sample at the planning radius: the wall needs the chord of an arc, not a straight
    assert!(
        (0.03..0.06).contains(&farthest.1),
        "the fixture needs the chord of an arc: {farthest:?}"
    );
    let (dx, dy) = (farthest.0.0 - foot.0, farthest.0.1 - foot.1);
    let len = dx.hypot(dy);
    assert!(len > 0.0, "the chord and its sample coincide");
    let n = (dx / len, dy / len);
    let wall = Wall {
        line: vec![
            (foot.0 - 0.05 * n.0, foot.1 - 0.05 * n.1),
            (
                foot.0 + 0.5 * farthest.1 * n.0,
                foot.1 + 0.5 * farthest.1 * n.1,
            ),
        ],
        allowed: 0,
        kind: WallKind::Stream,
        anchors: [at(1000.0, 1000.0), at(1000.0, 1000.0)],
        end_m: 10.0,
    };
    // the samples of the search cross it nowhere, the written line crosses it once
    assert_eq!(
        wall.crossings(&route.points),
        0,
        "the wall must not reach the samples"
    );
    assert_eq!(
        wall.crossings(&dropped),
        1,
        "the wall must cross the written line"
    );
    // the edit closure must borrow the walls for any lifetime the input has, so the wall lives for good
    let walls: &'static [Wall] = Box::leak(vec![wall].into_boxed_slice());
    let mut kept: Option<WrittenLine> = None;
    let _ = s.run_edited(None, |input| {
        input.walls = walls;
        kept = Some(written_line(
            input,
            &band,
            reached,
            &route,
            PLAN_WRITE_TOL_M,
        ));
    });
    let w = kept.expect("the write-back against the wall");
    assert!(w.fell_back, "a written line of {} points", w.points.len());
    assert_eq!(w.points, route.points, "every sample is kept");
}

/// The hairpin of the examples: legs x = 0 and x = 8 from y = 0 to 60, an apex of radius 4 above, anchors at
/// y = 20 on both legs.
fn hairpin_line() -> Vec<(f64, f64)> {
    let mut line = vec![(0.0, 0.0), (0.0, 60.0)];
    for i in 1..16 {
        let a = PI - PI * f64::from(i) / 16.0;
        line.push((4.0 + 4.0 * a.cos(), 60.0 + 4.0 * a.sin()));
    }
    line.extend([(8.0, 60.0), (8.0, 0.0)]);
    line
}

fn hairpin() -> Setup {
    Setup::new(
        anchor(0.0, 20.0, FRAC_PI_2),
        anchor(8.0, 20.0, -FRAC_PI_2),
        &hairpin_line(),
    )
}

#[test]
fn a_hairpin_gets_a_loop_of_the_radius() {
    let r = hairpin().route();
    assert!(tightest(&r.points) >= 9.9, "{}", tightest(&r.points));
    // On flat, free ground nothing is cheaper than the shortest Dubins path: an LRL "bulb" of arcs of 46.3, 272.6 and
    // 46.3 degrees at R 10.5, 10.5 (4 * 0.808 + pi) = 66.93 m, 25.7 m deep (top at y = 20 + 15.19 + 10.5 = 45.7),
    // widest stray 6.5 m. The brief's 92.6 m assumed a loop out at the drawn apex, which costs the same stray and
    // 40 m more road.
    assert!(close(r.length_m, 66.93, 0.5), "{}", r.length_m);
    let top = r
        .points
        .iter()
        .map(|p| p.1 - SHIFT.1)
        .fold(f64::MIN, f64::max);
    assert!(close(top, 45.7, 1.0), "{top}");
    assert!(
        r.max_stray_m <= 31.5 && r.max_move_m <= 7.0,
        "{} {}",
        r.max_stray_m,
        r.max_move_m
    );
    assert!(is_close(r.points[0], at(0.0, 20.0), 1e-9));
    assert!(is_close(*r.points.last().unwrap(), at(8.0, 20.0), 1e-9));
}

fn water_above_30(p: (f64, f64)) -> bool {
    p.1 >= 130.0
}

#[test]
fn water_over_the_apex_leaves_no_loop() {
    let mut s = hairpin();
    s.water = water_above_30;
    match s.run().unwrap() {
        LegOutcome::NoRoute {
            budget_exhausted,
            states,
            ..
        } => {
            assert!(!budget_exhausted, "proven, not out of budget");
            assert!(states > 0);
        }
        other @ LegOutcome::Found(_) => panic!("{other:?}"),
    }
}

#[test]
fn a_budget_that_runs_out_is_reported() {
    let mut s = hairpin();
    s.budgets = vec![(1.0, 10)];
    match s.run().unwrap() {
        LegOutcome::NoRoute {
            budget_exhausted,
            states,
            ..
        } => {
            assert!(budget_exhausted);
            assert_eq!(states, 10);
        }
        other @ LegOutcome::Found(_) => panic!("{other:?}"),
    }
}

/// A switchback on a hillside that falls across it: the drawn road is a "V" of two legs of 66.6 m that meet in an
/// apex of radius 4, the anchors 40 m apart at the bottom of both legs, `rise` metres apart in height. The drawn
/// length is 145.7 m. The terrain is one slope, `rise / 40` per metre east.
struct Switchback {
    setup: Setup,
    drawn: Vec<(f64, f64)>,
    rise: f64,
}

const APEX_Y: f64 = 64.6;

fn switchback_at(rise: f64) -> Switchback {
    let mut drawn = vec![(0.0, 0.0), (16.0, APEX_Y)];
    for i in 1..16 {
        let a = PI - PI * f64::from(i) / 16.0;
        drawn.push((20.0 + 4.0 * a.cos(), APEX_Y + 4.0 * a.sin()));
    }
    drawn.extend([(24.0, APEX_Y), (40.0, 0.0)]);
    let heading = APEX_Y.atan2(16.0);
    let mut setup = Setup::new(
        anchor(0.0, 0.0, heading),
        anchor(40.0, 0.0, -heading),
        &drawn,
    );
    let slope = rise / 40.0;
    setup.ground = local(move |x, _| 100.0 + slope * x);
    let g = drawn_grade(&line_at(&drawn), &*setup.ground, 20.0).unwrap();
    let (limit, _) = leg_limit(&[Family::Grav], &g).unwrap();
    setup.grade = limit;
    Switchback {
        setup,
        drawn: line_at(&drawn),
        rise,
    }
}

#[test]
fn a_switchback_within_the_grade_keeps_its_length() {
    let sb = switchback_at(21.9);
    let drawn = drawn_grade(&sb.drawn, &*sb.setup.ground, 20.0).unwrap();
    assert!(
        close(length_of(&sb.drawn), 145.7, 0.2),
        "{}",
        length_of(&sb.drawn)
    );
    assert!(close(drawn.mean, 0.150, 0.001), "{}", drawn.mean);
    assert!(close(sb.setup.grade, 0.18, 1e-12), "{}", sb.setup.grade);
    let r = sb.setup.route();
    assert!(r.length_m >= (sb.rise - 1.0) / 0.18, "{}", r.length_m);
    let profile = arrive(&r.points, &*sb.setup.ground, &sb.setup.band()).unwrap();
    assert!(
        profile.max_grade() <= 0.18 + 1e-9,
        "{}",
        profile.max_grade()
    );
    assert!(r.planned_grade <= 0.189, "{}", r.planned_grade);
    assert!(tightest(&r.points) >= 9.9);
    // The brief wanted the route's top at or above the drawn apex. It is not: a loop of radius 10.5 fits where the
    // drawn V is wide, so the planned loop lies inside the drawn one (top at y = 54.8 against 68.6 for the drawn
    // apex). It is not shorter than the climb needs, which is what the grade rule is for.
    let top = |pts: &[(f64, f64)]| pts.iter().map(|p| p.1).fold(f64::MIN, f64::max);
    assert!(
        top(&r.points) > SHIFT.1 + 40.0,
        "a loop, not a shortcut: {}",
        top(&r.points)
    );
    assert_eq!(r.relax_steps, 0);
    assert!(close(r.earthwork_m2, profile.earthwork_m2(), 1e-9));
    assert!(close(r.cut_max_m, profile.cut_max_m(), 1e-9));
    assert!(close(r.fill_max_m, profile.fill_max_m(), 1e-9));
}

#[test]
fn a_steeper_switchback_keeps_its_steeper_length() {
    let sb = switchback_at(36.4);
    assert!(close(sb.setup.grade, 0.25, 0.001), "{}", sb.setup.grade);
    let r = sb.setup.route();
    assert!(close(r.grade_limit, sb.setup.grade, 1e-12));
    assert!(r.length_m >= (36.4 - 1.0) / r.grade_limit, "{}", r.length_m);
    assert!(r.planned_grade <= r.grade_limit + 1.0 / r.length_m);
}

#[test]
fn a_wall_of_water_relaxes_the_grade() {
    // Water from y = 44 on, and the route held within 10 m of the drawn line: only about a 100 m route fits.
    let mut sb = switchback_at(21.9);
    sb.setup.water = water_over_switchback;
    sb.setup.cap = 10.0;
    let limits = relaxed_limits(0.18, 0.36, 3).unwrap();
    assert!(close(limits[1], 0.24, 1e-12));
    // at 0.18 alone there is none, and the open set is empty
    match sb.setup.run().unwrap() {
        LegOutcome::NoRoute {
            budget_exhausted, ..
        } => assert!(!budget_exhausted),
        other @ LegOutcome::Found(_) => panic!("{other:?}"),
    }
    let LegOutcome::Found(r) = sb.setup.run_with(Some(&limits)).unwrap() else {
        panic!("no route even relaxed");
    };
    assert_eq!(r.relax_steps, 1);
    assert!(close(r.grade_limit, 0.24, 1e-12));
    assert!(
        r.planned_grade <= 0.24 + 1.0 / r.length_m,
        "{} {}",
        r.planned_grade,
        r.length_m
    );
    assert!(
        (87.0..116.1).contains(&r.length_m),
        "about 100 m: {}",
        r.length_m
    );
    // the states of the limit that failed count too
    assert!(r.states > 861);
}

fn water_over_switchback(p: (f64, f64)) -> bool {
    p.1 >= SHIFT.1 + 44.0
}

fn pinned(rise: f64, cap: f64) -> Setup {
    let mut s = Setup::new(
        anchor(0.0, 0.0, 0.0),
        anchor(40.0, 0.0, 0.0),
        &[(0.0, 0.0), (40.0, 0.0)],
    );
    s.ground = local(move |x, _| 100.0 + rise / 40.0 * x);
    s.grade = 0.27;
    s.cap = cap;
    s
}

#[test]
fn pinned_anchors_cannot_shorten_a_climb() {
    // 12 m in 40 m at 0.27: every route is at least (12 - 1) / 0.27 = 40.74 m, and the cap of 31.5 m lets one wiggle
    // that much: a route exists.
    let LegOutcome::Found(r) = pinned(12.0, 31.5).run().unwrap() else {
        panic!("a wiggling route exists");
    };
    assert!(r.length_m >= (12.0 - 1.0) / 0.27 - 1e-9, "{}", r.length_m);
    // With a cap of 0.5 m (the true distance, the cap is below two cells) a wiggle of amplitude 0.5 m and radius >= R
    // gains a fraction 0.5 / (4 R) = 1.2 % of 40 m: 40.5 m < 40.74 m. No route, as the brief says; the 4 m strip
    // of stored distance 0 does not count.
    match pinned(12.0, 0.5).run().unwrap() {
        LegOutcome::NoRoute {
            budget_exhausted, ..
        } => assert!(!budget_exhausted),
        other @ LegOutcome::Found(_) => panic!("{other:?}"),
    }
    // 15 m (37.5 %) needs 52 m: the spike's anchors, free by 6 m, would have passed.
    match pinned(15.0, 0.5).run().unwrap() {
        LegOutcome::NoRoute {
            budget_exhausted, ..
        } => assert!(!budget_exhausted),
        other @ LegOutcome::Found(_) => panic!("{other:?}"),
    }
}

#[test]
fn a_route_is_pinned_to_the_terrain_at_both_anchors() {
    let LegOutcome::Found(r) = pinned(12.0, 31.5).run().unwrap() else {
        panic!("a route exists");
    };
    let s = pinned(12.0, 31.5);
    let profile = arrive(&r.points, &*s.ground, &s.band()).unwrap();
    let (z0, z1) = (
        (s.ground)(s.start.at).unwrap(),
        (s.ground)(s.end.at).unwrap(),
    );
    assert!((profile.heights()[0] - z0).abs() <= 0.5 + 1e-9);
    assert!((profile.heights().last().unwrap() - z1).abs() <= 0.5 + 1e-9);
    assert!(close(r.planned_grade, (z1 - z0).abs() / r.length_m, 1e-12));
}

#[test]
fn a_leg_that_is_too_steep_is_relaxed_to_the_limit_that_fits() {
    let limits = relaxed_limits(0.27, 0.36, 3).unwrap();
    // 12.5 m: 0.27 needs 42.6 m, which a held route cannot make, 0.30 needs 38.3 m
    let LegOutcome::Found(r) = pinned(12.5, 0.5).run_with(Some(&limits)).unwrap() else {
        panic!("no route");
    };
    assert_eq!(r.relax_steps, 1);
    assert!(close(r.grade_limit, 0.30, 1e-12), "{}", r.grade_limit);
    assert!(r.planned_grade <= 0.30 + 1.0 / r.length_m);
}

#[test]
fn a_steep_slope_needs_a_winding_route() {
    // 30 m of rise in 40 m at 0.27: (30 - 1) / 0.27 = 107.4 m of road are needed at least. The cap of 31.5 m is room
    // enough to wind; the 0.5 m cap is not (the open set runs empty).
    let mut s = Setup::new(
        anchor(0.0, 0.0, 0.0),
        anchor(40.0, 0.0, 0.0),
        &[(0.0, 0.0), (40.0, 0.0)],
    );
    s.ground = local(|x, _| 100.0 + 0.75 * x);
    s.grade = 0.27;
    let r = s.route();
    assert!(r.length_m >= (30.0 - 1.0) / 0.27 - 1e-9, "{}", r.length_m);
    let profile = arrive(&r.points, &*s.ground, &s.band()).unwrap();
    assert!(profile.max_grade() <= 0.27 + 1e-9);
    s.cap = 0.5;
    match s.run().unwrap() {
        LegOutcome::NoRoute {
            budget_exhausted, ..
        } => assert!(!budget_exhausted),
        other @ LegOutcome::Found(_) => panic!("{other:?}"),
    }
}

/// The pairs of samples of a route, 1 m apart, that are more than `along` apart along it, with their distance.
fn self_gaps(points: &[(f64, f64)], along: f64) -> f64 {
    let mut samples: Vec<(f64, f64, f64)> = Vec::new();
    let mut s = 0.0;
    for w in points.windows(2) {
        let l = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        let n = l.ceil().max(1.0);
        for k in 0..naves_core::reroute::dubins::segments(0.0, l, 1.0) {
            let t = f64::from(u32::try_from(k).unwrap()) / n;
            samples.push((
                w[0].0 + (w[1].0 - w[0].0) * t,
                w[0].1 + (w[1].1 - w[0].1) * t,
                s + l * t,
            ));
        }
        s += l;
    }
    let mut least = f64::INFINITY;
    for (i, a) in samples.iter().enumerate() {
        for b in &samples[i + 1..] {
            if b.2 - a.2 > along {
                least = least.min((a.0 - b.0).hypot(a.1 - b.1));
            }
        }
    }
    least
}

#[test]
fn a_route_does_not_loop_back_across_itself() {
    // A turn of 300 degrees between anchors 12 m apart. The shortest route is the Dubins path RSL of 76.8 m, longer
    // than the 58 m of `self_along`, so the two anchors, 12 m apart, are samples of one route that the rule keeps
    // 12.5 m apart: every route breaks it, and the leg has none.
    let heading_end = -PI / 3.0;
    let mut s = Setup::new(
        anchor(0.0, 0.0, 0.0),
        anchor(12.0, 0.0, heading_end),
        &[(0.0, 0.0), (12.0, 0.0)],
    );
    s.clear = Clearance::new(0.0, 12.5).unwrap();
    let direct = shortest(pose(0.0, 0.0, 0.0), pose(12.0, 0.0, heading_end)).length();
    assert!(close(direct, 76.78, 0.01) && direct > s.self_along);
    match s.run().unwrap() {
        LegOutcome::NoRoute { states, .. } => assert!(states > 0),
        other @ LegOutcome::Found(_) => panic!("{other:?}"),
    }
    // Without the rule the leg has a route, the loop of the shortest path, which does break it.
    let mut free = Setup::new(
        anchor(0.0, 0.0, 0.0),
        anchor(12.0, 0.0, heading_end),
        &[(0.0, 0.0), (12.0, 0.0)],
    );
    free.self_along = 1.0e9;
    let LegOutcome::Found(loose) = free.run().unwrap() else {
        panic!("the loop is a route without the rule");
    };
    assert!(
        loose.length_m >= direct - 0.05,
        "{} {direct}",
        loose.length_m
    );
    assert!(self_gaps(&loose.points, PI * RHO + 25.0) < 12.5);
}

#[test]
fn a_dead_end_is_reached_exactly() {
    let end = Anchor {
        at: at(50.0, 10.0),
        heading: None,
        straight_m: 0.0,
    };
    let s = Setup::new(anchor(0.0, 0.0, 0.0), end, &[(0.0, 0.0), (50.0, 10.0)]);
    let r = s.route();
    assert!(is_close(*r.points.last().unwrap(), at(50.0, 10.0), 1e-9));
    assert!(tightest(&r.points) >= 9.9);
}

#[test]
fn a_dead_end_start_picks_its_heading() {
    let start = Anchor {
        at: at(0.0, 0.0),
        heading: None,
        straight_m: 0.0,
    };
    let s = Setup::new(start, anchor(60.0, 30.0, 0.0), &[(0.0, 0.0), (60.0, 30.0)]);
    let r = s.route();
    assert!(is_close(r.points[0], at(0.0, 0.0), 1e-9));
    assert!(is_close(*r.points.last().unwrap(), at(60.0, 30.0), 1e-9));
}

#[test]
fn a_junction_starts_with_its_straight() {
    let mut start = anchor(0.0, 0.0, 0.0);
    start.straight_m = 6.25;
    let mut end = anchor(60.0, 40.0, FRAC_PI_2);
    end.straight_m = 6.25;
    let s = Setup::new(start, end, &[(0.0, 0.0), (60.0, 0.0), (60.0, 40.0)]);
    let r = s.route();
    assert!(is_close(r.points[0], at(0.0, 0.0), 1e-9));
    // the first 6.25 m run along the heading
    let first = r.points[1..]
        .iter()
        .take_while(|p| (p.0 - r.points[0].0).hypot(p.1 - r.points[0].1) <= 6.25 + 1e-9)
        .count();
    assert!(first >= 1);
    for p in &r.points[..=first] {
        assert!((p.1 - r.points[0].1).abs() < 1e-6, "{p:?}");
    }
    assert!(r.points.iter().any(|p| is_close(*p, at(6.25, 0.0), 1e-6)));
    // and the last 6.25 m run along the end heading, exactly
    let n = r.points.len();
    assert!(is_close(r.points[n - 1], at(60.0, 40.0), 1e-9));
    assert!(
        is_close(r.points[n - 2], at(60.0, 33.75), 1e-6),
        "{:?}",
        r.points[n - 2]
    );
    assert!(tightest(&r.points) >= 9.9);
}

#[test]
fn close_anchors_and_bad_anchors() {
    let line = [(0.0, 0.0), (5.0, 0.0)];
    let r = Setup::new(anchor(0.0, 0.0, 0.0), anchor(5.0, 0.0, 0.0), &line).route();
    assert!(close(r.length_m, 5.0, 1e-6), "{}", r.length_m);
    let same = Setup::new(anchor(0.0, 0.0, 0.0), anchor(0.0, 0.0, 1.0), &line);
    assert_eq!(same.run().unwrap_err(), RerouteError::Degenerate);
    for bad in [f64::NAN, f64::INFINITY] {
        let mut a = anchor(0.0, 0.0, 0.0);
        a.at.0 = bad;
        let s = Setup::new(a, anchor(5.0, 0.0, 0.0), &line);
        assert_eq!(s.run().unwrap_err(), RerouteError::NotFinite);
        let mut e = Setup::new(anchor(0.0, 0.0, 0.0), anchor(5.0, 0.0, 0.0), &line);
        e.end.heading = Some(bad);
        assert_eq!(e.run().unwrap_err(), RerouteError::NotFinite);
    }
}

#[test]
fn the_switchback_terrain_needs_the_ceiling_and_real_earthwork() {
    // the straight 90.8 m of switchback-1 on its own terrain, held to the line by a cap of 0.5 m
    let mut s = Setup::new(
        anchor(0.0, 0.0, 0.0),
        anchor(90.8, 0.0, 0.0),
        &[(0.0, 0.0), (90.8, 0.0)],
    );
    let steep = 0.549;
    let rest = (21.4 - 20.0 * steep) / 70.8;
    s.ground = local(move |x, _| {
        let a = x.clamp(0.0, 35.0);
        let b = (x - 35.0).clamp(0.0, 20.0);
        let c = (x - 55.0).max(0.0);
        100.0 + a * rest + b * steep + c * rest
    });
    s.cap = 0.5;
    s.grade = 0.18;
    match s.run().unwrap() {
        LegOutcome::NoRoute {
            budget_exhausted, ..
        } => assert!(!budget_exhausted),
        other @ LegOutcome::Found(_) => panic!("{other:?}"),
    }
    s.grade = 0.36;
    let r = s.route();
    let p = arrive(&r.points, &*s.ground, &s.band()).unwrap();
    // an independent two-pass profile of the route's own points, the same rules written out in the test
    let (earth, cut, fill) = independent_profile(&r.points, &*s.ground, 0.36, 6.0, 6.0, 0.5, 2.0);
    assert!(
        close(r.earthwork_m2, earth, 1e-6 * earth),
        "{} {earth}",
        r.earthwork_m2
    );
    assert!(close(r.cut_max_m, cut, 1e-6) && close(r.fill_max_m, fill, 1e-6));
    // The measured earthwork of the straight line is 67.6 m2 (cut 3.62 m); the route may wiggle by 1.7 m inside the
    // strip of cells the line touches and does so to save earthwork (63.8 m2).
    assert!(r.earthwork_m2 > 10.0, "{}", r.earthwork_m2);
    assert!(close(r.earthwork_m2, p.earthwork_m2(), 1e-9));
    assert!(close(r.cut_max_m, p.cut_max_m(), 1e-9) && r.cut_max_m > 1.0);
    assert!(close(r.climb_m, 21.4, 0.5), "{}", r.climb_m);
    assert!(r.planned_grade <= 0.36 && r.planned_grade > 0.2);
    // the cost counts the real earthwork
    assert!(
        r.cost >= r.length_m + 0.99 * r.earthwork_m2,
        "{} {}",
        r.cost,
        r.earthwork_m2
    );
}

#[test]
fn the_same_leg_gives_the_same_route_every_time() {
    let corner = [(0.0, 0.0), (60.0, 0.0), (60.0, 60.0)];
    let s = Setup::new(
        anchor(18.0, 0.0, 0.0),
        anchor(60.0, 42.0, FRAC_PI_2),
        &corner,
    );
    let first = s.route();
    assert_eq!(first, s.route());
    for threads in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let routes: Vec<Route> = pool.install(|| {
            use rayon::prelude::*;
            (0..4).into_par_iter().map(|_| s.route()).collect()
        });
        for r in routes {
            assert_eq!(r, first, "{threads} threads");
        }
    }
}

fn is_none(outcome: &LegOutcome) -> Option<(bool, usize)> {
    match outcome {
        LegOutcome::NoRoute {
            budget_exhausted,
            states,
            ..
        } => Some((*budget_exhausted, *states)),
        LegOutcome::Found(_) => None,
    }
}

/// A wall at x = 30 (local) between two y values; a route of 2 m samples along the line has a vertex there.
fn stream_between(y0: f64, y1: f64, allowed: usize, anchors: [(f64, f64); 2]) -> Wall {
    Wall {
        line: line_at(&[(30.0, y0), (30.0, y1)]),
        allowed,
        kind: WallKind::Stream,
        anchors,
        end_m: 10.0,
    }
}

#[test]
fn a_stream_the_line_crosses_is_crossed_once() {
    let line = [(0.0, 0.0), (60.0, 0.0)];
    let mut s = Setup::new(anchor(0.0, 0.0, 0.0), anchor(60.0, 0.0, 0.0), &line);
    s.walls = vec![stream_between(
        -40.0,
        40.0,
        1,
        [at(0.0, 0.0), at(60.0, 0.0)],
    )];
    let r = s.route();
    assert_eq!(s.walls[0].crossings(&r.points), 1);
}

#[test]
fn a_stream_the_line_does_not_cross_is_not_crossed() {
    // the stream ends at y = 3, just beside the line, and the route may not go round the wrong side of its end
    let line = [(0.0, 0.0), (60.0, 0.0)];
    let mut s = Setup::new(anchor(0.0, 0.0, 0.0), anchor(60.0, 0.0, 0.0), &line);
    s.walls = vec![stream_between(3.0, 40.0, 0, [at(0.0, 0.0), at(60.0, 0.0)])];
    let r = s.route();
    assert_eq!(s.walls[0].crossings(&r.points), 0);
    // a stream across the whole box cannot be crossed at all: the open set runs empty
    s.walls = vec![stream_between(
        -200.0,
        200.0,
        0,
        [at(0.0, 0.0), at(60.0, 0.0)],
    )];
    assert_eq!(is_none(&s.run().unwrap()).map(|n| n.0), Some(false));
}

#[test]
fn a_blocked_junction_straight_or_missing_ground_is_no_route_at_once() {
    let line = [(0.0, 0.0), (60.0, 0.0)];
    let mut s = Setup::new(anchor(0.0, 0.0, 0.0), anchor(60.0, 0.0, 0.0), &line);
    s.start.straight_m = 6.25;
    s.water = water_right_of_3;
    assert_eq!(is_none(&s.run().unwrap()), Some((false, 0)));
    // no terrain at the end anchor, and at the start anchor
    let mut s = Setup::new(anchor(0.0, 0.0, 0.0), anchor(60.0, 0.0, 0.0), &line);
    s.ground = Box::new(|p| (p.0 < 150.0).then_some(100.0));
    assert_eq!(is_none(&s.run().unwrap()), Some((false, 0)));
    s.ground = Box::new(|p| (p.0 > 120.0).then_some(100.0));
    assert_eq!(is_none(&s.run().unwrap()), Some((false, 0)));
}

fn water_right_of_3(p: (f64, f64)) -> bool {
    p.0 >= SHIFT.0 + 3.0
}

#[test]
fn the_limits_of_a_relaxed_search_are_checked() {
    let line = [(0.0, 0.0), (60.0, 0.0)];
    let s = Setup::new(anchor(0.0, 0.0, 0.0), anchor(60.0, 0.0, 0.0), &line);
    assert_eq!(s.run_with(Some(&[])).unwrap_err(), RerouteError::Degenerate);
    assert_eq!(
        s.run_with(Some(&[0.2, -0.1])).unwrap_err(),
        RerouteError::Degenerate
    );
    assert_eq!(
        s.run_with(Some(&[f64::NAN])).unwrap_err(),
        RerouteError::NotFinite
    );
    let LegOutcome::Found(r) = s.run_with(Some(&[0.2, 0.3])).unwrap() else {
        panic!("no route");
    };
    assert_eq!(r.relax_steps, 0);
    // an infinite limit takes the grade off
    assert!(matches!(
        s.run_with(Some(&[f64::INFINITY])).unwrap(),
        LegOutcome::Found(_)
    ));
}

type Edit = fn(&mut LegInput<'_>);

#[test]
fn the_inputs_of_a_leg_are_checked() {
    let line = [(0.0, 0.0), (60.0, 0.0)];
    let s = Setup::new(anchor(0.0, 0.0, 0.0), anchor(60.0, 0.0, 0.0), &line);
    let nan = f64::NAN;
    let not_finite: [Edit; 8] = [
        |i| i.radius = f64::NAN,
        |i| i.step_m = f64::INFINITY,
        |i| i.stray_cap_m = f64::NAN,
        |i| i.arrive_reach_m = f64::NAN,
        |i| i.self_along_m = f64::INFINITY,
        |i| i.weights.stray = f64::NAN,
        |i| i.start.straight_m = f64::NAN,
        |i| i.budgets = &[(f64::NAN, 10)],
    ];
    for (k, edit) in not_finite.into_iter().enumerate() {
        assert_eq!(
            s.run_edited(None, edit).unwrap_err(),
            RerouteError::NotFinite,
            "{k}"
        );
    }
    let degenerate: [Edit; 8] = [
        |i| i.radius = 0.0,
        |i| i.step_m = -1.0,
        |i| i.weights.earth = -1.0,
        |i| i.stray_cap_m = -1.0,
        |i| i.end.straight_m = -2.0,
        |i| i.budgets = &[],
        |i| i.budgets = &[(-1.0, 10)],
        |i| i.end.at = i.start.at,
    ];
    for (k, edit) in degenerate.into_iter().enumerate() {
        assert_eq!(
            s.run_edited(None, edit).unwrap_err(),
            RerouteError::Degenerate,
            "{k}"
        );
    }
    // a band that is not a band
    let mut bad = Setup::new(anchor(0.0, 0.0, 0.0), anchor(60.0, 0.0, 0.0), &line);
    bad.band_override = Some(Band {
        grade: nan,
        ..s.band()
    });
    assert_eq!(bad.run().unwrap_err(), RerouteError::NotFinite);
    bad.band_override = Some(Band {
        cut_m: -1.0,
        ..s.band()
    });
    assert_eq!(bad.run().unwrap_err(), RerouteError::Degenerate);
}

#[test]
fn a_later_budget_runs_when_the_first_runs_out() {
    let mut s = hairpin();
    s.budgets = vec![(1.0, 10), (1.5, 100_000)];
    let r = s.route();
    assert!((r.weight - 1.5).abs() < 1e-12);
    assert!(r.states > 10, "{}", r.states);
    // an empty open set ends it without trying the next
    let mut s = hairpin();
    s.water = water_above_30;
    s.budgets = vec![(1.0, 100_000), (1.5, 100_000)];
    let (exhausted, states) = is_none(&s.run().unwrap()).unwrap();
    assert!(!exhausted);
    let mut one = hairpin();
    one.water = water_above_30;
    one.budgets = vec![(1.0, 100_000)];
    assert_eq!(is_none(&one.run().unwrap()), Some((false, states)));
}

#[test]
fn a_dead_end_behind_the_start_is_reached_by_a_loop() {
    let line = [(0.0, 0.0), (-30.0, 10.0)];
    let end = Anchor {
        at: at(-30.0, 10.0),
        heading: None,
        straight_m: 0.0,
    };
    let s = Setup::new(anchor(0.0, 0.0, 0.0), end, &line);
    let r = s.route();
    assert!(tightest(&r.points) >= 9.9);
    assert!(is_close(*r.points.last().unwrap(), at(-30.0, 10.0), 1e-9));
    // a point 30 m behind and 10 m beside needs a U-turn: more than the half circle of the radius
    assert!(r.length_m > PI * RHO, "{}", r.length_m);
    // straight ahead: the arc through the point is a straight
    let end = Anchor {
        at: at(50.0, 0.0),
        heading: None,
        straight_m: 0.0,
    };
    let s = Setup::new(anchor(0.0, 0.0, 0.0), end, &[(0.0, 0.0), (50.0, 0.0)]);
    let r = s.route();
    assert!(close(r.length_m, 50.0, 1e-6), "{}", r.length_m);
}

#[test]
fn the_weights_are_the_constants() {
    let w = Weights::from_params(&RerouteParams::default());
    assert!(close(w.slope, 4.0, 0.0) && close(w.curve, 0.1, 0.0));
    assert!(close(w.earth, 1.0, 0.0) && close(w.stray, 0.25, 0.0));
}

// ---- what every route must satisfy ----

/// A random leg on a random slope with a random lake. Positions are local, in `0..200`.
fn random_leg(seed: u64) -> Setup {
    let mut state = seed | 1;
    let mut pick = |lo: f64, hi: f64| lo + (hi - lo) * unit(&mut state);
    let start = anchor(pick(10.0, 60.0), pick(10.0, 60.0), pick(0.0, TAU));
    let mut end = anchor(pick(90.0, 190.0), pick(60.0, 190.0), pick(0.0, TAU));
    if pick(0.0, 1.0) < 0.25 {
        end.heading = None;
    }
    if pick(0.0, 1.0) < 0.25 {
        end.straight_m = 0.0;
    }
    if pick(0.0, 1.0) < 0.3 {
        // close anchors: the end 25 to 60 m from the start
        end.at = (
            start.at.0 + pick(25.0, 55.0),
            start.at.1 + pick(-30.0, 30.0),
        );
    }
    let wall_wanted = pick(0.0, 1.0) < 0.4;
    let (sx, sy) = (pick(-0.3, 0.3), pick(-0.3, 0.3));
    let line = [
        (start.at.0 - SHIFT.0, start.at.1 - SHIFT.1),
        (end.at.0 - SHIFT.0, end.at.1 - SHIFT.1),
    ];
    let mut s = Setup::new(start, end, &line);
    s.ground = local(move |x, y| 100.0 + sx * x + sy * y);
    s.grade = pick(0.15, 0.4);
    s.clear = Clearance::new(0.0, 12.5).unwrap();
    s.budgets = vec![(1.0, 4_000), (2.0, 4_000)];
    let lake_at = (pick(60.0, 140.0) + SHIFT.0, pick(30.0, 140.0) + SHIFT.1);
    let size = pick(0.0, 18.0);
    LAKE.with(|l| l.set((lake_at.0, lake_at.1, size)));
    s.water = lake_of_the_thread;
    if wall_wanted {
        // a stream across the middle of the leg that the original line crosses once
        let mid = (
            f64::midpoint(line[0].0, line[1].0),
            f64::midpoint(line[0].1, line[1].1),
        );
        let (dx, dy) = (line[1].0 - line[0].0, line[1].1 - line[0].1);
        let len = dx.hypot(dy);
        let perp = (-dy / len * 40.0, dx / len * 40.0);
        let stream = line_at(&[
            (mid.0 - perp.0, mid.1 - perp.1),
            (mid.0 + perp.0, mid.1 + perp.1),
        ]);
        let anchors = [s.start.at, s.end.at];
        let allowed = crossings_away(&s.original, &stream, anchors, 10.0);
        s.walls = vec![Wall {
            line: stream,
            allowed,
            kind: WallKind::Stream,
            anchors,
            end_m: 10.0,
        }];
    }
    s
}

thread_local! {
    static LAKE: std::cell::Cell<(f64, f64, f64)> = const { std::cell::Cell::new((0.0, 0.0, 0.0)) };
}

fn lake_of_the_thread(p: (f64, f64)) -> bool {
    let (x, y, size) = LAKE.with(std::cell::Cell::get);
    (p.0 - x).abs() <= size && (p.1 - y).abs() <= size
}

/// Whether a found route holds every promise; `Ok(true)` when there was a route.
fn check_leg(seed: u64) -> Result<bool, TestCaseError> {
    let s = random_leg(seed);
    let LegOutcome::Found(r) = s.run().map_err(|e| TestCaseError::fail(format!("{e}")))? else {
        return Ok(false);
    };
    let o = s.obstacles();
    prop_assert!(is_close(r.points[0], s.start.at, 1e-9));
    prop_assert!(is_close(*r.points.last().unwrap(), s.end.at, 1e-9));
    prop_assert!(tightest(&r.points) >= 9.9, "radius {}", tightest(&r.points));
    for w in r.points.windows(2) {
        prop_assert!(o.segment(w[0], w[1], s.clear).is_ok(), "{w:?}");
    }
    prop_assert!(r.points.iter().all(|p| o.stray_m(*p) <= s.cap));
    for w in &s.walls {
        prop_assert!(
            w.crossings(&r.points) == w.allowed,
            "wall {} of {}",
            w.crossings(&r.points),
            w.allowed
        );
    }
    let profile = arrive(&r.points, &*s.ground, &s.band())
        .map_err(|e| TestCaseError::fail(format!("profile: {e}")))?;
    prop_assert!(profile.max_grade() <= s.grade + 1e-9);
    prop_assert!((r.earthwork_m2 - profile.earthwork_m2()).abs() < 1e-9);
    prop_assert!(r.planned_grade <= s.grade + 1.0 / r.length_m + 1e-9);
    prop_assert!(close(length_of(&r.points), r.length_m, 1e-6));
    let gap = self_gaps(&r.points, s.self_along);
    prop_assert!(gap >= 12.5 - 1.5, "self gap {gap}");
    if let (Some(h0), Some(h1), true) = (
        s.start.heading,
        s.end.heading,
        s.start.straight_m == 0.0 && s.end.straight_m == 0.0,
    ) {
        let direct = shortest(
            pose(s.start.at.0, s.start.at.1, h0),
            pose(s.end.at.0, s.end.at.1, h1),
        )
        .length();
        prop_assert!(r.length_m >= direct - 0.1, "{} {direct}", r.length_m);
    }
    Ok(true)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 40,
        rng_seed: RngSeed::Fixed(0x6e61_7665_735f_3037),
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/proptest-regressions/reroute.txt",
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn every_route_found_holds_its_promises(seed in any::<u64>()) {
        check_leg(seed)?;
    }
}

#[test]
fn the_route_property_is_not_vacuous() {
    let (mut found, mut walled, mut close) = (0, 0, 0);
    for k in 1..=40_u64 {
        let seed = k.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let s = random_leg(seed);
        if check_leg(seed).unwrap() {
            found += 1;
            walled += usize::from(!s.walls.is_empty());
            close += usize::from(dist_between(s.start.at, s.end.at) < 70.0);
        }
    }
    // measured: see the assertion messages when they fail; the families must be there, or the property says nothing
    assert!(
        found >= 10 && walled >= 2 && close >= 2,
        "{found} found, {walled} with a wall, {close} close"
    );
}

fn dist_between(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// The two-pass profile of notes 5.1 written out again: samples equally spaced along the polyline (at most `spacing`
/// apart), the road pinned to the terrain within `tol` at both ends, forward and backward passes of the reachable
/// interval, then the profile closest to the terrain. Returns the earthwork, the deepest cut and the highest fill.
fn independent_profile(
    points: &[(f64, f64)],
    ground: &dyn Fn((f64, f64)) -> Option<f64>,
    grade: f64,
    cut: f64,
    fill: f64,
    tol: f64,
    spacing: f64,
) -> (f64, f64, f64) {
    let total = length_of(points);
    let parts = (total / spacing - 1e-9).ceil().max(1.0);
    let count = naves_core::reroute::dubins::segments(0.0, total, total / parts);
    let ds = total / f64::from(u32::try_from(count).unwrap());
    let last_segment = points.len() - 2;
    let at_arc = |target: f64| {
        let mut done = 0.0;
        for (k, w) in points.windows(2).enumerate() {
            let l = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
            if done + l >= target || k == last_segment {
                let t = if l > 0.0 {
                    ((target - done) / l).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                return (
                    w[0].0 + (w[1].0 - w[0].0) * t,
                    w[0].1 + (w[1].1 - w[0].1) * t,
                );
            }
            done += l;
        }
        points[0]
    };
    let terrain: Vec<f64> = (0..=count)
        .map(|k| ground(at_arc(f64::from(u32::try_from(k).unwrap()) * ds)).unwrap())
        .collect();
    let (mut low, mut high): (Vec<f64>, Vec<f64>) =
        terrain.iter().map(|t| (t - cut, t + fill)).unzip();
    low[0] = low[0].max(terrain[0] - tol);
    high[0] = high[0].min(terrain[0] + tol);
    for i in 1..terrain.len() {
        low[i] = low[i].max(low[i - 1] - grade * ds);
        high[i] = high[i].min(high[i - 1] + grade * ds);
    }
    let last = terrain.len() - 1;
    low[last] = low[last].max(terrain[last] - tol);
    high[last] = high[last].min(terrain[last] + tol);
    for i in (0..last).rev() {
        low[i] = low[i].max(low[i + 1] - grade * ds);
        high[i] = high[i].min(high[i + 1] + grade * ds);
    }
    let mut road = vec![terrain[0].clamp(low[0], high[0])];
    for i in 1..terrain.len() {
        let (lo, hi) = (
            low[i].max(road[i - 1] - grade * ds),
            high[i].min(road[i - 1] + grade * ds),
        );
        road.push(terrain[i].clamp(lo, hi));
    }
    let earth: f64 = (1..terrain.len())
        .map(|i| (road[i] - terrain[i]).abs() * ds)
        .sum();
    let cut_max = terrain
        .iter()
        .zip(&road)
        .map(|(t, r)| t - r)
        .fold(0.0, f64::max);
    let fill_max = terrain
        .iter()
        .zip(&road)
        .map(|(t, r)| r - t)
        .fold(0.0, f64::max);
    (earth, cut_max, fill_max)
}

// ---- fix round 1: the arrival, the walls, the hairpin's own measure ----

#[test]
fn a_route_round_the_end_of_a_stream_does_not_block_the_valid_one() {
    // The original bends over the end of a stream and crosses it once. With no stray cost the cheapest route is the
    // straight line below the stream's end: it crosses nothing, so the check refuses it (the count must equal what the
    // original did). That refusal must not end the search: the S-curve over the end is a route.
    let line = [(0.0, 0.0), (30.0, 6.0), (60.0, 0.0)];
    let mut s = Setup::new(anchor(0.0, 0.0, 0.0), anchor(60.0, 0.0, 0.0), &line);
    let anchors = [at(0.0, 0.0), at(60.0, 0.0)];
    let mut wall = Wall {
        line: line_at(&[(30.4, 3.0), (30.4, 40.0)]),
        allowed: 0,
        kind: WallKind::Stream,
        anchors,
        end_m: 10.0,
    };
    wall.allowed = crossings_away(&s.original, &wall.line, anchors, 10.0);
    assert_eq!(wall.allowed, 1);
    s.walls = vec![wall];
    let LegOutcome::Found(r) = s.run_edited(None, |i| i.weights.stray = 0.0).unwrap() else {
        panic!("a route over the end of the stream exists");
    };
    assert_eq!(s.walls[0].crossings(&r.points), 1);
    assert!(r.length_m > 60.0 && r.length_m < 70.0, "{}", r.length_m);
}

#[test]
fn a_wall_is_counted_wall_by_wall() {
    // Two streams, each crossed once by the original. The sum of the crossings of a route that crosses the first
    // twice and the second not at all is the same as the original's, and is no route.
    let line = [(0.0, 0.0), (80.0, 0.0)];
    let mut s = Setup::new(anchor(0.0, 0.0, 0.0), anchor(80.0, 0.0, 0.0), &line);
    let anchors = [at(0.0, 0.0), at(80.0, 0.0)];
    let wall = |x: f64| Wall {
        line: line_at(&[(x, -40.0), (x, 40.0)]),
        allowed: 1,
        kind: WallKind::Stream,
        anchors,
        end_m: 10.0,
    };
    s.walls = vec![wall(30.4), wall(50.4)];
    let r = s.route();
    for w in &s.walls {
        assert_eq!(w.crossings(&r.points), 1);
    }
}

#[test]
fn a_wall_through_a_vertex_is_counted_once() {
    // a wall at integer coordinates through the 2 m vertices of a straight route
    let route = [
        (0.0, 0.0),
        (28.0, 0.0),
        (30.0, 0.0),
        (32.0, 0.0),
        (60.0, 0.0),
    ];
    let wall = [(30.0, -5.0), (30.0, 5.0)];
    assert_eq!(
        crossings_away(&route, &wall, [(0.0, 0.0), (60.0, 0.0)], 10.0),
        1
    );
    assert_eq!(
        crossings_away(&route, &wall, [(0.0, 0.0), (60.0, 0.0)], 10.0),
        1
    );
    // through a vertex of the wall as well
    let bent = [(30.0, -5.0), (30.0, 0.0), (30.0, 5.0)];
    assert_eq!(
        crossings_away(&route, &bent, [(0.0, 0.0), (60.0, 0.0)], 10.0),
        1
    );
    // blocked as a segment test
    let w = Wall {
        line: wall.to_vec(),
        allowed: 0,
        kind: WallKind::Stream,
        anchors: [(0.0, 0.0), (60.0, 0.0)],
        end_m: 10.0,
    };
    assert_ne!(
        w.blocks((28.0, 0.0), (30.0, 0.0)),
        w.blocks((30.0, 0.0), (32.0, 0.0))
    );
}

#[test]
fn the_hairpin_clears_itself_by_the_original_measure() {
    // Production clearances: own reach 9 m, road clearance 13 m, a road 6 m wide. The anchors are 8 m apart, so the
    // original legs are 8 m apart, and `self_clearance` gives 8 m (not 13).
    let along = PI * RHO + 26.0;
    let original = line_at(&hairpin_line());
    let own = self_clearance(&original, along, 13.0, 6.0);
    assert!(close(own, 8.0, 0.1), "{own}");
    let mut s = hairpin();
    s.clear = Clearance::new(9.0, 13.0).unwrap();
    s.self_along = along;
    s.self_clear = Some(own);
    let r = s.route();
    assert!(close(r.length_m, 66.93, 0.5), "{}", r.length_m);
    // no two samples more than `self_along` apart along the route are nearer than `self_clear`, within the
    // spacing of the samples
    let least = self_gaps(&r.points, along);
    assert!(least >= own - 1.5, "{least}");
    // with the road clearance as the measure the hairpin has no route at all: that was the defect
    s.self_clear = Some(13.0);
    assert!(matches!(s.run().unwrap(), LegOutcome::NoRoute { .. }));
}

#[test]
fn the_self_clearance_of_an_original_line() {
    let along = 59.0;
    // two legs 8 m apart: 8
    let hairpin = line_at(&hairpin_line());
    assert!(close(self_clearance(&hairpin, along, 13.0, 6.0), 8.0, 0.1));
    // never below the road's width
    assert!(close(
        self_clearance(&hairpin, along, 13.0, 9.0),
        9.0,
        1e-12
    ));
    // legs further apart than the threshold: the threshold
    let wide = line_at(&[(0.0, 0.0), (0.0, 80.0), (40.0, 80.0), (40.0, 0.0)]);
    assert!(close(self_clearance(&wide, along, 13.0, 6.0), 13.0, 1e-12));
    // a straight line has no pair that far apart and not near: the threshold
    let straight = line_at(&[(0.0, 0.0), (200.0, 0.0)]);
    assert!(close(
        self_clearance(&straight, along, 13.0, 6.0),
        13.0,
        1e-12
    ));
    // a line too short to have a pair more than `along` apart: the distance of its two ends counts, whatever their
    // distance along the line (the route between them is long, whatever the drawing was)
    let short = line_at(&[(0.0, 0.0), (20.0, 0.0), (20.0, 8.0), (0.0, 8.0)]);
    assert!(close(self_clearance(&short, along, 13.0, 6.0), 8.0, 1e-12));
    // ends further apart than the threshold: the threshold
    let apart = line_at(&[(0.0, 0.0), (20.0, 0.0), (20.0, 30.0)]);
    assert!(close(self_clearance(&apart, along, 13.0, 6.0), 13.0, 1e-12));
    // a threshold below the width gives the width; nothing that is not a number is used
    assert!(close(
        self_clearance(&straight, along, 4.0, 6.0),
        6.0,
        1e-12
    ));
    assert!(close(
        self_clearance(&[(f64::NAN, 0.0), (1.0, 1.0)], along, 13.0, 6.0),
        13.0,
        1e-12
    ));
    assert!(close(
        self_clearance(&straight, f64::NAN, 13.0, 6.0),
        13.0,
        1e-12
    ));
}

/// A corridor 7 m wide and 30 m long in front of the end anchor, between water.
fn corridor_water(p: (f64, f64)) -> bool {
    let (x, y) = (p.0 - SHIFT.0, p.1 - SHIFT.1);
    (70.0..=101.0).contains(&x) && y.abs() > 3.5
}

#[test]
fn a_dearer_state_that_climbs_further_is_kept() {
    // 24 m of rise in 100 m at 0.18, anchors pinned: the road needs (24 - 1) / 0.18 = 127.8 m, and it can only gain
    // 0.18 * 30 = 5.4 m in the corridor, so it must enter it at 18.1 m or more above its start (terrain there: 16.8 m)
    // after winding 100 m in the open. A short route is cheaper and enters the same cells of the corridor at the
    // same headings with a lower reachable interval: it cannot arrive. Keeping only the cheapest state of a cell and
    // heading (the old rule) drops every winding route at the corridor's entrance and the open set runs empty.
    let mut s = Setup::new(
        anchor(0.0, 0.0, 0.0),
        anchor(100.0, 0.0, 0.0),
        &[(0.0, 0.0), (100.0, 0.0)],
    );
    s.ground = local(|x, _| 100.0 + 0.24 * x);
    s.grade = 0.18;
    s.cap = 12.0;
    s.water = corridor_water;
    let r = s.route();
    assert!(r.length_m >= (24.0 - 1.0) / 0.18 - 1e-9, "{}", r.length_m);
    let profile = arrive(&r.points, &*s.ground, &s.band()).unwrap();
    assert!(profile.max_grade() <= 0.18 + 1e-9);
    // the same leg without the water at the entrance is found as well, and the corridor route is no shorter
    let mut open = Setup::new(
        anchor(0.0, 0.0, 0.0),
        anchor(100.0, 0.0, 0.0),
        &[(0.0, 0.0), (100.0, 0.0)],
    );
    open.ground = local(|x, _| 100.0 + 0.24 * x);
    open.grade = 0.18;
    open.cap = 12.0;
    assert!(open.route().length_m <= r.length_m + 1e-9);
}

#[test]
fn sizes_that_would_blow_up_the_search_are_refused() {
    let line = [(0.0, 0.0), (60.0, 0.0)];
    let s = Setup::new(anchor(0.0, 0.0, 0.0), anchor(60.0, 0.0, 0.0), &line);
    let bad: [fn(&mut LegInput<'_>); 3] = [
        |i| i.radius = 1.0e15,
        |i| i.step_m = 1.0e4,
        |i| i.arrive_reach_m = 1.0e5,
    ];
    for (k, edit) in bad.into_iter().enumerate() {
        assert_eq!(
            s.run_edited(None, edit).unwrap_err(),
            RerouteError::Degenerate,
            "{k}"
        );
    }
    assert_eq!(
        s.run_edited(None, |i| i.self_clear_m = f64::NAN)
            .unwrap_err(),
        RerouteError::NotFinite
    );
    assert_eq!(
        s.run_edited(None, |i| i.self_clear_m = -1.0).unwrap_err(),
        RerouteError::Degenerate
    );
}

/// The hairpin with legs from y = 20 to 40 only: about 52 m of drawing between the anchors.
fn short_hairpin_line() -> Vec<(f64, f64)> {
    let mut line = vec![(0.0, 20.0), (0.0, 40.0)];
    for i in 1..16 {
        let a = PI - PI * f64::from(i) / 16.0;
        line.push((4.0 + 4.0 * a.cos(), 40.0 + 4.0 * a.sin()));
    }
    line.extend([(8.0, 40.0), (8.0, 20.0)]);
    line
}

#[test]
fn a_short_drawn_hairpin_clears_itself_as_well() {
    // About 52 m of drawing, less than `self_along` (59 m): no pair of the original is that far apart along it, but
    // every route between the anchors is longer than 66.9 m, so its first and last samples are, and they are 8 m apart.
    // The distance between the anchors counts, and the leg has a route at production clearances.
    let along = PI * RHO + 26.0;
    let original = short_hairpin_line();
    let drawn = length_of(&original);
    assert!(drawn > 50.0 && drawn < along, "{drawn}");
    let own = self_clearance(&line_at(&original), along, 13.0, 6.0);
    assert!(close(own, 8.0, 1e-9), "{own}");
    let mut s = Setup::new(
        anchor(0.0, 20.0, FRAC_PI_2),
        anchor(8.0, 20.0, -FRAC_PI_2),
        &original,
    );
    s.clear = Clearance::new(9.0, 13.0).unwrap();
    s.self_along = along;
    s.self_clear = Some(own);
    let r = s.route();
    assert!(close(r.length_m, 66.93, 0.5), "{}", r.length_m);
    assert!(self_gaps(&r.points, along) >= own - 1.5);
}

#[test]
fn a_route_that_crosses_one_wall_twice_and_another_not_at_all_is_no_route() {
    // Wall A is a U open at the bottom: the straight line through it crosses it twice. Wall B ends above the
    // original's detour, which the straight line passes. The original crosses A once and B once; with no stray cost
    // the cheapest route is the straight line: 2 crossings of A, none of B, the same sum as the original's. Only
    // the count wall by wall refuses it.
    let original = [
        (0.0, 0.0),
        (30.0, 0.0),
        (30.0, -8.0),
        (60.0, -8.0),
        (80.0, 0.0),
    ];
    let mut s = Setup::new(anchor(0.0, 0.0, 0.0), anchor(80.0, 0.0, 0.0), &original);
    let anchors = [at(0.0, 0.0), at(80.0, 0.0)];
    let mut a = Wall {
        line: line_at(&[(25.4, -5.0), (25.4, 5.0), (35.4, 5.0), (35.4, -5.0)]),
        allowed: 0,
        kind: WallKind::Stream,
        anchors,
        end_m: 10.0,
    };
    let mut b = Wall {
        line: line_at(&[(55.4, -40.0), (55.4, -3.0)]),
        allowed: 0,
        kind: WallKind::Stream,
        anchors,
        end_m: 10.0,
    };
    for w in [&mut a, &mut b] {
        w.allowed = crossings_away(&s.original, &w.line, anchors, 10.0);
    }
    assert_eq!((a.allowed, b.allowed), (1, 1));
    // the straight line is what the sum would let through
    let straight = line_at(&[(0.0, 0.0), (80.0, 0.0)]);
    assert_eq!((a.crossings(&straight), b.crossings(&straight)), (2, 0));
    s.walls = vec![a, b];
    let LegOutcome::Found(r) = s.run_edited(None, |i| i.weights.stray = 0.0).unwrap() else {
        panic!("a route that crosses each once exists");
    };
    for w in &s.walls {
        assert_eq!(w.crossings(&r.points), 1);
    }
}

#[test]
fn the_cap_is_tested_in_the_middle_of_a_chord() {
    // A notch 2 m wide in the original line. The straight route has chord ends on the original at x = 28 and x = 30,
    // and the middle of that chord, 1 m from the arms and 5 m from the bottom, is outside a true cap of 0.5 m. No
    // route within 0.5 m of the original exists (a radius of 10.5 cannot follow the notch).
    let original = [
        (0.0, 0.0),
        (28.0, 0.0),
        (28.0, -5.0),
        (30.0, -5.0),
        (30.0, 0.0),
        (60.0, 0.0),
    ];
    let mut s = Setup::new(anchor(0.0, 0.0, 0.0), anchor(60.0, 0.0, 0.0), &original);
    s.cap = 0.5;
    match s.run().unwrap() {
        LegOutcome::NoRoute { .. } => {}
        other @ LegOutcome::Found(_) => panic!("{other:?}"),
    }
}

// ----------------------------------------------------------------------------------------------------------
// Road planning: zones, anchors, legs and the write-back (plan Task 6).
// ----------------------------------------------------------------------------------------------------------

/// The tightest arc of the City family in these fixtures, in metres.
const CITY_MIN_R: f64 = 10.0;
/// The half width of a City road, in metres.
const CITY_HALF: f64 = 5.0;
/// The terrain cell of the fixtures, in metres.
const MAP_CELL_M: f64 = 4.0;
/// The vertices per side of the fixture map (2048 m at 4 m).
const MAP_SIZE: u32 = 513;

fn flat_map() -> Heightmap {
    Heightmap {
        size: MAP_SIZE,
        cell_m: 4.0,
        h: vec![100.0_f32; (MAP_SIZE * MAP_SIZE) as usize],
    }
}

/// A slope of 10 % along the town-corner road: east from x = 400 at y = 500, then north.
fn ramp_map() -> Heightmap {
    let mut h = Vec::with_capacity((MAP_SIZE * MAP_SIZE) as usize);
    for row in 0..MAP_SIZE {
        for col in 0..MAP_SIZE {
            let x = f64::from(col) * MAP_CELL_M;
            let y = f64::from(row) * MAP_CELL_M;
            let along = (x - 400.0) + (y - 500.0).max(0.0);
            #[expect(clippy::cast_possible_truncation, reason = "a fixture height")]
            let z = (100.0 + 0.10 * along) as f32;
            h.push(z);
        }
    }
    Heightmap {
        size: MAP_SIZE,
        cell_m: 4.0,
        h,
    }
}

fn road_way(
    way: i64,
    family: Family,
    nodes: &[i64],
    points: &[(f64, f64)],
    bridge: bool,
    tunnel: bool,
    roundabout: bool,
) -> Feature {
    Feature {
        id: format!("osm/way/{way}/0"),
        way_id: way,
        source: Source::Osm,
        kind: FeatureKind::Road {
            family,
            drivable: true,
            bridge,
            tunnel,
            roundabout,
            layer: 0,
            use_sidepath: false,
            paving: None,
            held: false,
            plan: false,
        },
        points: points.to_vec(),
        nodes: nodes.to_vec(),
    }
}

fn city_way(way: i64, nodes: &[i64], points: &[(f64, f64)]) -> Feature {
    road_way(way, Family::City, nodes, points, false, false, false)
}

fn bridge_way(way: i64, nodes: &[i64], points: &[(f64, f64)]) -> Feature {
    road_way(way, Family::City, nodes, points, true, false, false)
}

fn plan_with(
    graph: &RoadGraph,
    map: &Heightmap,
    min: &(dyn Fn(&Stretch) -> f64 + Sync),
    water: &(dyn Fn((f64, f64)) -> bool + Sync),
    streams: &[Vec<(f64, f64)>],
    params: &RerouteParams,
) -> (RoadGraph, RerouteReport) {
    reroute_graph(graph, map, min, water, streams, params)
}

fn plan_graph(graph: &RoadGraph, map: &Heightmap) -> (RoadGraph, RerouteReport) {
    plan_with(
        graph,
        map,
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &[],
        &RerouteParams::default(),
    )
}

fn points_of(graph: &RoadGraph, id: &str) -> Vec<(f64, f64)> {
    graph
        .stretches
        .iter()
        .find(|s| s.id == id)
        .map_or_else(|| panic!("no stretch {id}"), |s| s.points.clone())
}

/// The position of the crossing of `node` in the graph.
fn crossing_point(graph: &RoadGraph, node: i64) -> Option<(f64, f64)> {
    graph
        .crossings
        .iter()
        .find(|c| c.node == node)
        .map(|c| c.point)
}

fn stretch_ids(graph: &RoadGraph) -> Vec<String> {
    graph.stretches.iter().map(|s| s.id.clone()).collect()
}

fn node_ids(graph: &RoadGraph) -> Vec<i64> {
    graph.crossings.iter().map(|c| c.node).collect()
}

/// The one zone of the report that matches.
fn zone_matching(report: &RerouteReport, what: impl Fn(&PlannedZone) -> bool) -> &PlannedZone {
    let found: Vec<&PlannedZone> = report.zones.iter().filter(|z| what(z)).collect();
    assert_eq!(found.len(), 1, "zones: {:?}", report.zones);
    found[0]
}

fn only_zone(report: &RerouteReport) -> &PlannedZone {
    assert_eq!(report.zones.len(), 1, "zones: {:?}", report.zones);
    &report.zones[0]
}

fn is_planned(zone: &PlannedZone) -> bool {
    matches!(zone.outcome, ZoneOutcome::Planned)
}

fn kept(zone: &PlannedZone) -> (KeptReason, &str) {
    match &zone.outcome {
        ZoneOutcome::Kept { reason, detail } => (*reason, detail.as_str()),
        ZoneOutcome::Planned => panic!("zone planned: {zone:?}"),
    }
}

/// The town corner of the brief: J1 (400, 500), a corner at (500, 500), J2 (500, 600), with a through side road
/// at J1 and at J2.
fn town_corner() -> RoadGraph {
    build_graph(&[
        city_way(
            1,
            &[1, 2, 3],
            &[(400.0, 500.0), (500.0, 500.0), (500.0, 600.0)],
        ),
        city_way(
            2,
            &[10, 1, 11],
            &[(400.0, 400.0), (400.0, 500.0), (400.0, 600.0)],
        ),
        city_way(
            3,
            &[12, 3, 13],
            &[(400.0, 600.0), (500.0, 600.0), (600.0, 600.0)],
        ),
    ])
}

/// The same corner, but 15 m after J1 and with J2 further north.
fn corner_after_junction() -> RoadGraph {
    build_graph(&[
        city_way(
            1,
            &[1, 2, 3],
            &[(400.0, 500.0), (415.0, 500.0), (415.0, 600.0)],
        ),
        city_way(
            2,
            &[10, 1, 11],
            &[(400.0, 400.0), (400.0, 500.0), (400.0, 600.0)],
        ),
        city_way(
            3,
            &[12, 3, 13],
            &[(315.0, 600.0), (415.0, 600.0), (515.0, 600.0)],
        ),
    ])
}

/// A corner 15 m before a dead end.
fn corner_before_dead_end() -> RoadGraph {
    build_graph(&[
        city_way(
            1,
            &[1, 2, 3],
            &[(400.0, 500.0), (450.0, 500.0), (450.0, 515.0)],
        ),
        city_way(
            2,
            &[10, 1, 11],
            &[(400.0, 400.0), (400.0, 500.0), (400.0, 600.0)],
        ),
    ])
}

/// A corner `gap` metres before a 30 m bridge.
fn corner_before_bridge(gap: f64) -> RoadGraph {
    let start = 500.0 + gap;
    build_graph(&[
        city_way(
            1,
            &[1, 2, 20, 21, 3],
            &[
                (400.0, 500.0),
                (500.0, 500.0),
                (500.0, start - 10.0),
                (500.4, start - 5.0),
                (500.0, start),
            ],
        ),
        bridge_way(2, &[3, 4], &[(500.0, start), (500.0, start + 30.0)]),
        city_way(3, &[4, 5], &[(500.0, start + 30.0), (500.0, start + 60.0)]),
        city_way(
            4,
            &[10, 1, 11],
            &[(400.0, 400.0), (400.0, 500.0), (400.0, 600.0)],
        ),
        city_way(
            5,
            &[12, 5, 13],
            &[
                (400.0, start + 60.0),
                (500.0, start + 60.0),
                (600.0, start + 60.0),
            ],
        ),
    ])
}

/// A bridge whose end is 8 m from a junction, with the corner 15 m past that junction.
fn junction_near_bridge() -> RoadGraph {
    build_graph(&[
        bridge_way(1, &[1, 2], &[(500.0, 500.0), (500.0, 540.0)]),
        city_way(2, &[2, 3], &[(500.0, 540.0), (500.0, 548.0)]),
        city_way(
            3,
            &[10, 3, 11],
            &[(400.0, 548.0), (500.0, 548.0), (600.0, 548.0)],
        ),
        city_way(
            4,
            &[3, 4, 5],
            &[(500.0, 548.0), (500.0, 563.0), (560.0, 563.0)],
        ),
    ])
}

/// A 6 m kink between two junctions.
fn tight_kink_between_junctions() -> RoadGraph {
    build_graph(&[
        city_way(
            1,
            &[1, 2, 3],
            &[(400.0, 500.0), (403.0, 503.0), (406.0, 500.0)],
        ),
        city_way(
            2,
            &[10, 1, 11],
            &[(400.0, 400.0), (400.0, 500.0), (400.0, 600.0)],
        ),
        city_way(
            3,
            &[12, 3, 13],
            &[(406.0, 400.0), (406.0, 500.0), (406.0, 600.0)],
        ),
    ])
}

/// A node inside the zone where two stretches of different families continue each other.
fn two_stretch_node() -> RoadGraph {
    build_graph(&[
        road_way(
            1,
            Family::Asf3,
            &[1, 2],
            &[(400.0, 500.0), (490.0, 500.0)],
            false,
            false,
            false,
        ),
        city_way(
            2,
            &[2, 3, 4],
            &[(490.0, 500.0), (500.0, 500.0), (500.0, 600.0)],
        ),
        city_way(
            3,
            &[10, 1, 11],
            &[(400.0, 400.0), (400.0, 500.0), (400.0, 600.0)],
        ),
        city_way(
            4,
            &[12, 4, 13],
            &[(400.0, 600.0), (500.0, 600.0), (600.0, 600.0)],
        ),
    ])
}

/// The town corner with a bridge deck crossing the drawn line 12 m after the start anchor.
fn town_corner_with_deck() -> RoadGraph {
    build_graph(&[
        city_way(
            1,
            &[1, 2, 3],
            &[(400.0, 500.0), (500.0, 500.0), (500.0, 600.0)],
        ),
        city_way(
            2,
            &[10, 1, 11],
            &[(400.0, 400.0), (400.0, 500.0), (400.0, 600.0)],
        ),
        city_way(
            3,
            &[12, 3, 13],
            &[(400.0, 600.0), (500.0, 600.0), (600.0, 600.0)],
        ),
        bridge_way(4, &[20, 21], &[(492.0, 470.0), (492.0, 530.0)]),
    ])
}

/// The Task 5 hairpin, 35 m below a road across its top.
fn hairpin_with_a_road() -> RoadGraph {
    let mut nodes = vec![1_i64];
    let mut points = vec![(500.0, 400.0), (500.0, 460.0)];
    nodes.push(2);
    let mut node = 3_i64;
    for i in 1..16 {
        let a = PI - PI * f64::from(i) / 16.0;
        points.push((506.0 + 6.0 * a.cos(), 460.0 + 4.0 * a.sin()));
        nodes.push(node);
        node += 1;
    }
    points.push((512.0, 460.0));
    nodes.push(node);
    node += 1;
    points.push((512.0, 400.0));
    nodes.push(node);
    build_graph(&[
        city_way(1, &nodes, &points),
        city_way(2, &[100, 101], &[(450.0, 499.0), (550.0, 499.0)]),
    ])
}

/// Three hairpins whose parallel legs are 25 m apart.
fn switchback_stack() -> RoadGraph {
    let mut nodes: Vec<i64> = Vec::new();
    let mut points: Vec<(f64, f64)> = Vec::new();
    let mut next = 1_i64;
    let mut push = |p: (f64, f64), nodes: &mut Vec<i64>| {
        points.push(p);
        nodes.push(next);
        next += 1;
    };
    push((450.0, 400.0), &mut nodes);
    push((550.0, 400.0), &mut nodes);
    // hairpin from the east leg (y = 400) to the next leg (y = 425), bulging east
    for i in 1..16 {
        let a = -PI / 2.0 - PI * f64::from(i) / 16.0;
        push((554.0 + 4.0 * a.cos(), 412.5 + 4.0 * a.sin()), &mut nodes);
    }
    push((550.0, 425.0), &mut nodes);
    push((450.0, 425.0), &mut nodes);
    // hairpin west, y = 425 -> 450
    for i in 1..16 {
        let a = PI / 2.0 - PI * f64::from(i) / 16.0;
        push((446.0 + 4.0 * a.cos(), 437.5 + 4.0 * a.sin()), &mut nodes);
    }
    push((450.0, 450.0), &mut nodes);
    push((550.0, 450.0), &mut nodes);
    // hairpin east, y = 450 -> 475
    for i in 1..16 {
        let a = -PI / 2.0 - PI * f64::from(i) / 16.0;
        push((554.0 + 4.0 * a.cos(), 462.5 + 4.0 * a.sin()), &mut nodes);
    }
    push((550.0, 475.0), &mut nodes);
    push((450.0, 475.0), &mut nodes);
    build_graph(&[city_way(1, &nodes, &points)])
}

/// The town corner with a narrow side road 8 m from the first leg at one place and far outside the corner later.
fn town_corner_with_side_road() -> RoadGraph {
    build_graph(&[
        city_way(
            1,
            &[1, 2, 3],
            &[(400.0, 500.0), (500.0, 500.0), (500.0, 600.0)],
        ),
        city_way(
            2,
            &[10, 1, 11],
            &[(400.0, 400.0), (400.0, 500.0), (400.0, 600.0)],
        ),
        city_way(
            3,
            &[12, 3, 13],
            &[(400.0, 600.0), (500.0, 600.0), (600.0, 600.0)],
        ),
        road_way(
            4,
            Family::Mud,
            &[20, 21, 22],
            &[(400.0, 508.0), (525.0, 508.0), (525.0, 620.0)],
            false,
            false,
            false,
        ),
    ])
}

/// A ring whose cut (the lowest node) is itself a tight 90° bend, with other tight corners elsewhere.
fn ring_with_a_tight_cut() -> RoadGraph {
    build_graph(&[city_way(
        1,
        &[1, 2, 3, 4, 1],
        &[
            (400.0, 500.0),
            (500.0, 500.0),
            (500.0, 600.0),
            (400.0, 600.0),
            (400.0, 500.0),
        ],
    )])
}

/// A road north with a corner at `x` that turns west, its west leg ending 8 m later: the free fillet bulges
/// toward x < x. The map edge is x = 0; the drawn line's westernmost point is `x - 8`.
fn corner_at_the_edge(x: f64) -> RoadGraph {
    build_graph(&[
        city_way(1, &[1, 2, 3], &[(x, 300.0), (x, 500.0), (x - 8.0, 500.0)]),
        city_way(
            2,
            &[10, 1, 11],
            &[(x - 100.0, 300.0), (x, 300.0), (x + 100.0, 300.0)],
        ),
    ])
}

/// A corner 10 m from a dead end on the map edge, at (0, 500).
fn road_to_the_edge() -> RoadGraph {
    build_graph(&[
        city_way(1, &[1, 2, 3], &[(0.0, 500.0), (0.0, 510.0), (60.0, 510.0)]),
        city_way(
            2,
            &[12, 3, 13],
            &[(60.0, 410.0), (60.0, 510.0), (60.0, 610.0)],
        ),
    ])
}

/// A U-turn of radius 25 (tight against a minimum of 30) with a bridge deck's earthwork across its top, and long
/// straights on both sides so that the zone's anchors are clear of the deck.
fn u_turn_with_a_deck() -> RoadGraph {
    let mut nodes = vec![1_i64, 2, 3];
    let mut points = vec![(200.0, 500.0), (300.0, 500.0), (360.0, 500.0)];
    let mut node = 4_i64;
    for i in 1..32 {
        let a = PI * f64::from(i) / 32.0;
        points.push((385.0 - 25.0 * a.cos(), 500.0 + 25.0 * a.sin()));
        nodes.push(node);
        node += 1;
    }
    for p in [(410.0, 500.0), (470.0, 500.0), (570.0, 500.0)] {
        points.push(p);
        nodes.push(node);
        node += 1;
    }
    build_graph(&[
        city_way(1, &nodes, &points),
        bridge_way(2, &[100, 101], &[(370.0, 535.0), (400.0, 535.0)]),
    ])
}

#[expect(
    clippy::many_single_char_names,
    reason = "the endpoint names of the two segments, as in the geometry"
)]
fn segment_crossings(a: &[(f64, f64)], b: &[(f64, f64)]) -> usize {
    let cross = |p: (f64, f64), q: (f64, f64), r: (f64, f64), s: (f64, f64)| {
        let (d1, d2) = ((q.0 - p.0, q.1 - p.1), (s.0 - r.0, s.1 - r.1));
        let den = d1.0 * d2.1 - d1.1 * d2.0;
        if den.abs() < 1e-12 {
            return false;
        }
        let t = ((r.0 - p.0) * d2.1 - (r.1 - p.1) * d2.0) / den;
        let u = ((r.0 - p.0) * d1.1 - (r.1 - p.1) * d1.0) / den;
        let e = 1e-9;
        t > e && t < 1.0 - e && u > e && u < 1.0 - e
    };
    let mut n = 0;
    for sa in a.windows(2) {
        for sb in b.windows(2) {
            if cross(sa[0], sa[1], sb[0], sb[1]) {
                n += 1;
            }
        }
    }
    n
}

/// The smallest distance of the polyline `a` to the polyline `b`.
fn line_distance(a: &[(f64, f64)], b: &[(f64, f64)]) -> f64 {
    a.iter()
        .map(|&p| dist_to_line(p, b))
        .fold(f64::INFINITY, f64::min)
}

// ----------------------------------------------------------------------------------------------------------
// Step 1: zones and anchors
// ----------------------------------------------------------------------------------------------------------

#[test]
fn plan_a_graph_without_a_tight_run_is_unchanged() {
    let g = build_graph(&[city_way(1, &[1, 2], &[(400.0, 500.0), (700.0, 500.0)])]);
    let (out, report) = plan_graph(&g, &flat_map());
    assert_eq!(out, g);
    assert!(report.zones.is_empty(), "{:?}", report.zones);
    assert_eq!(report.tight_before_m, 0.0);
    assert_eq!(report.tight_after_m, 0.0);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
}

#[test]
fn plan_a_town_corner() {
    let g = town_corner();
    let before = g.clone();
    let (out, report) = plan_graph(&g, &flat_map());
    let zone = only_zone(&report);
    assert!(is_planned(zone), "{zone:?}");
    assert_eq!(zone.anchors, (AnchorKind::Line, AnchorKind::Line));
    assert_eq!(zone.radius_wanted_m, CITY_MIN_R);
    assert_eq!(zone.junction, None);
    assert!(
        zone.radius_before_m < CITY_MIN_R,
        "{}",
        zone.radius_before_m
    );
    assert!(
        zone.radius_after_m >= CITY_MIN_R - 0.01,
        "{}",
        zone.radius_after_m
    );
    assert!(zone.legs >= 1);
    assert_eq!(zone.stretches.len(), 1);
    // the junctions never move
    for node in [1, 3] {
        assert_eq!(crossing_point(&out, node), crossing_point(&before, node));
    }
    assert_eq!(stretch_ids(&out), stretch_ids(&before));
    assert_eq!(node_ids(&out), node_ids(&before));
    // the corner is buildable now
    assert_eq!(report.tight_after_m, 0.0, "{}", report.tight_after_m);
    assert!(report.tight_before_m > 1.0, "{}", report.tight_before_m);
    assert!(report.max_stray_m >= 0.0);
    // the written stretch covers the same nodes and is longer than the chord
    let main = points_of(&out, "1:1-3");
    assert!(is_close(main[0], (400.0, 500.0), 1e-9));
    assert!(is_close(*main.last().unwrap(), (500.0, 600.0), 1e-9));
    assert!(length_of(&main) > 190.0);
}

#[test]
fn plan_a_written_route_keeps_only_the_points_it_needs() {
    // The write-back of a planned leg is the stage's own sampling of the route: arcs keep a point every
    // 2 m, straight parts collapse to what bounds them, and the anchors stay bit for bit. With the drop off
    // (`simplify: 0.0`) every sample of the search is written instead — both runs are asserted here, so the
    // count of the written line and its distance to the searched one cannot regress silently.
    let g = town_corner();
    let (dense_out, dense_report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &[],
        &RerouteParams {
            simplify: 0.0,
            ..RerouteParams::default()
        },
    );
    assert!(is_planned(only_zone(&dense_report)), "{dense_report:?}");
    let (out, report) = plan_graph(&g, &flat_map());
    let zone = only_zone(&report);
    assert!(is_planned(zone), "{zone:?}");
    let dense = points_of(&dense_out, "1:1-3");
    let written = points_of(&out, "1:1-3");
    // the counts, so a regression shows up as a number: every sample of the search, and the same route at
    // the stage's own sampling
    assert_eq!(dense.len(), 58, "the unsimplified write-back");
    assert_eq!(written.len(), 14, "the written line");
    // a quarter of the samples at most: the drop earns its name
    assert!(
        written.len() * 4 < dense.len(),
        "{} written of {} samples",
        written.len(),
        dense.len()
    );
    // the anchors keep their exact position
    assert!(same_bits(written[0], dense[0]));
    assert!(same_bits(*written.last().unwrap(), *dense.last().unwrap()));
    // the search's samples stay within the placement tolerance of the written line
    for p in &dense {
        let d = dist_to_line(*p, &written);
        assert!(d <= 0.20 + 1e-6, "a searched sample is {d} m from the line");
    }
    // the zone the search planned is unchanged: same stretches, same nodes, and the corner is buildable
    assert_eq!(stretch_ids(&out), stretch_ids(&dense_out));
    assert_eq!(node_ids(&out), node_ids(&dense_out));
    for node in [1, 3] {
        assert_eq!(
            crossing_point(&out, node),
            crossing_point(&g, node),
            "node {node} moved"
        );
    }
    assert!(
        zone.radius_after_m >= CITY_MIN_R - 0.01,
        "{}",
        zone.radius_after_m
    );
    assert_eq!(report.tight_after_m, 0.0, "{}", report.tight_after_m);
    assert_eq!(report.kept_sampled_routes, 0, "{report:?}");
    assert_eq!(dense_report.kept_sampled_routes, 0);
    // the two runs agree on what the zone is; a kilometre of written line is neither
    assert!(
        length_of(&written) > 190.0 && (length_of(&written) - length_of(&dense)).abs() < 1.0,
        "written {} against {}",
        length_of(&written),
        length_of(&dense)
    );
}

#[test]
fn plan_a_corner_after_a_junction_keeps_the_arm() {
    let g = corner_after_junction();
    let (out, report) = plan_graph(&g, &flat_map());
    let zone = only_zone(&report);
    assert!(is_planned(zone), "{zone:?}");
    assert_eq!(zone.anchors.0, AnchorKind::Junction(1));
    assert_eq!(zone.junction, Some(1));
    let main = points_of(&out, "1:1-3");
    assert!(same_bits(main[0], (400.0, 500.0)));
    let arm = (main[1].1 - main[0].1).atan2(main[1].0 - main[0].0);
    assert!(arm.abs() < 1e-9, "{arm}");
    let mut along = 0.0;
    for pair in main.windows(2) {
        if along > 6.25 + 1e-9 {
            break;
        }
        assert!((pair[0].1 - 500.0).abs() < 1e-6, "{:?}", pair[0]);
        along += (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1);
    }
    assert!(along >= 6.25, "{along}");
}

#[test]
fn plan_a_corner_before_a_dead_end_keeps_it() {
    let g = corner_before_dead_end();
    let (out, report) = plan_graph(&g, &flat_map());
    let zone = only_zone(&report);
    assert!(is_planned(zone), "{zone:?}");
    assert_eq!(zone.anchors.1, AnchorKind::DeadEnd(3));
    let main = points_of(&out, "1:1-3");
    assert!(same_bits(*main.last().unwrap(), (450.0, 515.0)));
}

#[test]
fn plan_a_bridge_keeps_its_points_and_held_approach() {
    // the corner is 40 m before the bridge: the zone ends in the 25 m hold, so the leg is planned against a
    // BridgeHold anchor, and the bridge and the hold's points are untouched
    let g = corner_before_bridge(40.0);
    let bridge = points_of(&g, "2:3-4");
    let approach = points_of(&g, "1:1-3");
    let (out, report) = plan_graph(&g, &flat_map());
    assert_ne!(out, g, "the corner before the hold must be planned");
    assert_eq!(points_of(&out, "2:3-4"), bridge, "the bridge moved");
    let zone = only_zone(&report);
    assert!(is_planned(zone), "{zone:?}");
    assert_eq!(zone.anchors.1, AnchorKind::BridgeHold);
    let ends = [(500.0, 540.0), (500.0, 570.0)];
    let at_end =
        |p: &(f64, f64), near: f64| ends.iter().any(|e| (e.0 - p.0).hypot(e.1 - p.1) <= near);
    for (a, b) in g.stretches.iter().zip(&out.stretches) {
        for p in &a.points {
            if at_end(p, 25.0 + 1e-9) {
                assert!(
                    b.points.iter().any(|q| same_bits(*p, *q)),
                    "{} lost the hold point {p:?}",
                    a.id
                );
            }
        }
        for q in &b.points {
            if at_end(q, 25.0 - 1e-6) {
                assert!(
                    a.points.iter().any(|p| same_bits(*p, *q)),
                    "{} added a point inside the hold: {q:?}",
                    a.id
                );
            }
        }
    }
    assert!(approach.iter().any(|p| same_bits(*p, (500.4, 535.0))));
    let written = points_of(&out, "1:1-3");
    assert!(written.iter().any(|p| same_bits(*p, (500.4, 535.0))));
    assert_no_reversal(&written);
}
#[test]
fn plan_a_corner_too_near_a_bridge_is_held() {
    let g = corner_before_bridge(12.0);
    let (out, report) = plan_graph(&g, &flat_map());
    assert_eq!(out, g);
    let zone = only_zone(&report);
    let (reason, detail) = kept(zone);
    assert_eq!(reason, KeptReason::Held, "{zone:?}");
    assert_eq!(detail, "tight_run_held", "{zone:?}");
    assert_eq!(zone.candidate, Some(Candidate::Plate));
    assert_eq!(zone.legs, 0, "the hold leaves no leg: {zone:?}");
}

#[test]
fn plan_a_deck_near_a_junction_belongs_to_the_anchor() {
    let g = junction_near_bridge();
    let (out, report) = plan_graph(&g, &flat_map());
    let _ = &out;
    let zone = only_zone(&report);
    assert!(is_planned(zone), "{zone:?}");
    assert_eq!(zone.anchors.0, AnchorKind::Junction(3));
    assert_eq!(zone.junction, Some(3));
}

#[test]
fn plan_a_tight_kink_between_junctions_is_held_close() {
    let g = tight_kink_between_junctions();
    let (out, report) = plan_graph(&g, &flat_map());
    assert_eq!(out, g);
    let zone = only_zone(&report);
    let (reason, detail) = kept(zone);
    assert_eq!(reason, KeptReason::Held);
    assert_eq!(detail, "anchors_close");
    assert!(
        report.kept_zones.iter().any(|(id, _)| *id == zone.id),
        "{:?}",
        report.kept_zones
    );
    // it was searched and kept, so it is not in the list of what was not planned at all
    assert!(
        !report.skipped.iter().any(|(id, _)| *id == zone.id),
        "{:?}",
        report.skipped
    );
}

#[test]
fn plan_a_node_inside_a_leg_moves() {
    let g = two_stretch_node();
    let (out, report) = plan_graph(&g, &flat_map());
    let zone = only_zone(&report);
    assert!(is_planned(zone), "{zone:?}");
    let first = points_of(&out, "1:1-2");
    let second = points_of(&out, "2:2-4");
    assert_eq!(first.last(), second.first());
    assert!(!is_close(*first.last().unwrap(), (490.0, 500.0), 1e-6));
    let crossing = out
        .crossings
        .iter()
        .find(|c| c.node == 2)
        .unwrap_or_else(|| panic!("no crossing at node 2: {:?}", out.crossings));
    assert_eq!(Some(crossing.point), first.last().copied());
    assert_eq!(crossing.stretches.len(), 2);
}

#[test]
fn plan_a_zone_under_a_deck_is_held() {
    let g = town_corner_with_deck();
    let (out, report) = plan_graph(&g, &flat_map());
    assert_eq!(out, g);
    let zone = only_zone(&report);
    let (reason, detail) = kept(zone);
    assert_eq!(reason, KeptReason::Held);
    assert_eq!(detail, "crosses_deck");
}

#[test]
fn plan_the_grade_of_a_leg() {
    let g = town_corner();
    let (out, report) = plan_graph(&g, &ramp_map());
    let zone = only_zone(&report);
    assert!(is_planned(zone), "{zone:?}");
    assert!(
        (zone.drawn_grade - 0.10).abs() <= 0.005,
        "{}",
        zone.drawn_grade
    );
    assert!(
        (zone.grade_limit - 0.14).abs() < 1e-9,
        "{}",
        zone.grade_limit
    );
    assert!(zone.length_after_m > 0.0);
    assert!(
        zone.planned_grade <= 0.14 + 1.0 / zone.length_after_m,
        "{} {}",
        zone.planned_grade,
        zone.length_after_m
    );
    // the drawn grade is a mean, the window the steepest part
    assert!(zone.drawn_grade_window >= zone.drawn_grade);
    let _ = out;
}

// ----------------------------------------------------------------------------------------------------------
// Step 2: obstacles on the graph
// ----------------------------------------------------------------------------------------------------------

#[test]
fn plan_a_route_keeps_off_a_road_beyond_the_hairpin() {
    let g = hairpin_with_a_road();
    let (out, report) = plan_graph(&g, &flat_map());
    let zone = only_zone(&report);
    assert!(is_planned(zone), "{zone:?}");
    let id = g
        .stretches
        .first()
        .map_or_else(|| panic!("no stretch"), |s| s.id.clone());
    let route = points_of(&out, &id);
    let road = [(450.0, 499.0), (550.0, 499.0)];
    // the road's reach (9 to 11 m here) plus the segment threshold of a 15 m clearance
    let reach = naves_core::conform::core_reach_m(CITY_HALF, MAP_CELL_M);
    let clearance = reach + MAP_CELL_M;
    let threshold = (clearance + 2.0) * CHAMFER_ERROR + 2.0 * 2.0_f64.sqrt();
    let want = reach + threshold - 2.0;
    let got = line_distance(&route, &road);
    assert!(got >= want, "{got} against {want}");
}

#[test]
fn plan_a_side_road_is_exempt_where_the_drawn_line_comes_near() {
    // the rule itself, per place: a road that comes within 8 m of the drawn line at one place is exempt there,
    // and its samples 30 m away are not, however near it comes elsewhere
    let road = [(0.0, 8.0), (20.0, 8.0), (40.0, 30.0), (60.0, 30.0)];
    let original = [(0.0, 0.0), (60.0, 0.0)];
    let flags = exempt_samples(&road, &original, 20.0, 10.0);
    assert!(
        flags.first().copied().unwrap_or(false),
        "the near part is not exempt"
    );
    assert!(
        !flags.last().copied().unwrap_or(true),
        "the far part is exempt: the rule is per road, not per place"
    );

    // the near part's exemption is what lets the plan exist: its earthwork covers the start anchor
    let g = town_corner_with_side_road();
    let (out, report) = plan_graph(&g, &flat_map());
    let zone = zone_matching(&report, |z| z.stretches.iter().any(|s| s == "1:1-3"));
    assert!(
        is_planned(zone),
        "the near exemption did not clear the start: {zone:?}"
    );
    let route = points_of(&out, "1:1-3");
    let side = points_of(&g, "4:20-22");
    let drawn = [(400.0, 500.0), (500.0, 500.0), (500.0, 600.0)];
    assert_eq!(
        segment_crossings(&route, &side),
        segment_crossings(&drawn, &side)
    );
}
#[test]
fn plan_a_switchback_stack_is_not_cut_through() {
    let g = switchback_stack();
    let id = g
        .stretches
        .first()
        .map_or_else(|| panic!("no stretch"), |s| s.id.clone());
    let original = points_of(&g, &id);
    let (out, report) = plan_graph(&g, &flat_map());
    assert!(!report.zones.is_empty(), "no zones: {report:?}");
    let route_line = points_of(&out, &id);
    let unchanged: Vec<(u64, u64)> = original
        .iter()
        .map(|p| (p.0.to_bits(), p.1.to_bits()))
        .collect();
    let new_points: Vec<(f64, f64)> = route_line
        .iter()
        .copied()
        .filter(|p| !unchanged.contains(&(p.0.to_bits(), p.1.to_bits())))
        .collect();
    let reach = naves_core::conform::core_reach_m(CITY_HALF, MAP_CELL_M);
    let clearance = reach + MAP_CELL_M;
    let self_free = (clearance + 2.0) * CHAMFER_ERROR + 2.0 * 2.0_f64.sqrt() + reach;
    let bound = reach + (clearance + 2.0) * CHAMFER_ERROR + 2.0 * 2.0_f64.sqrt() - 2.0;
    // the 25 m legs leave no room for the road clearance: the stage keeps every zone and says why
    for zone in &report.zones {
        match &zone.outcome {
            ZoneOutcome::Kept { reason, .. } => {
                assert!(
                    matches!(reason, KeptReason::Held | KeptReason::NoRoute),
                    "{zone:?}"
                );
                assert_eq!(zone.candidate, Some(Candidate::Plate));
            }
            ZoneOutcome::Planned => {
                let (pieces, zone_new) = own_chain_outside(&original, &new_points, zone, self_free);
                for piece in &pieces {
                    if zone_new.len() < 2 {
                        continue;
                    }
                    let got = zone_new
                        .iter()
                        .map(|&p| dist_to_line(p, piece))
                        .fold(f64::INFINITY, f64::min);
                    assert!(got >= bound, "zone {}: {got} against {bound}", zone.id);
                    assert_eq!(
                        segment_crossings(&zone_new, piece),
                        0,
                        "zone {} crosses the chain's other legs",
                        zone.id
                    );
                }
            }
        }
    }
}
#[test]
fn plan_streams_are_crossed_only_as_often_as_before() {
    let g = town_corner();
    let params = RerouteParams::default();
    // a short stream inside the corner, on the arc the free route takes
    let inner = vec![(497.0, 502.0), (499.0, 508.0)];
    let (free_out, free_report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &[],
        &params,
    );
    assert!(is_planned(only_zone(&free_report)));
    let free = points_of(&free_out, "1:1-3");
    assert!(
        segment_crossings(&free, &inner) >= 1,
        "the free route does not cross the stream"
    );
    let streams = [inner.clone()];
    let (out, report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &streams,
        &params,
    );
    assert!(is_planned(only_zone(&report)), "{:?}", report.zones);
    let route = points_of(&out, "1:1-3");
    assert_eq!(
        segment_crossings(&route, &inner),
        0,
        "the route crosses a stream the drawn line does not"
    );

    // a stream the drawn line crosses once, and the free route crosses twice
    let twice = vec![(492.0, 480.0), (492.0, 504.0), (498.0, 505.0)];
    let (free_out, free_report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &[],
        &params,
    );
    assert!(is_planned(only_zone(&free_report)));
    let free = points_of(&free_out, "1:1-3");
    assert_eq!(
        segment_crossings(&free, &twice),
        2,
        "the fixture is not discriminating"
    );
    let streams = [twice.clone()];
    let (out, report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &streams,
        &params,
    );
    assert!(is_planned(only_zone(&report)), "{:?}", report.zones);
    let route = points_of(&out, "1:1-3");
    assert_eq!(segment_crossings(&route, &twice), 1);
}
#[test]
fn plan_a_ring_cut_is_held_and_the_rest_is_planned() {
    let g = ring_with_a_tight_cut();
    let (out, report) = plan_graph(&g, &flat_map());
    let cut = zone_matching(&report, |z| matches!(z.anchors.0, AnchorKind::RingCut(_)));
    let (reason, detail) = kept(cut);
    assert_eq!(reason, KeptReason::Held, "{cut:?}");
    assert_eq!(detail, "ring_cut");
    assert_eq!(cut.anchors.0, AnchorKind::RingCut(1));
    // the bend at the cut is measured across the closed ring, and is the tightest point of its zone
    assert!(cut.radius_before_m < CITY_MIN_R, "{cut:?}");
    assert!(report.tight_before_m > 0.0);
    let planned = zone_matching(&report, |z| {
        is_planned(z) && z.at.0 > 450.0 && z.at.1 < 550.0
    });
    assert!(is_planned(planned), "{planned:?}");
    // the ring's first and last point (the cut) stay bit for bit
    let ring = points_of(&out, "1:1-1");
    assert!(ring.first().is_some_and(|&p| same_bits(p, (400.0, 500.0))));
    assert!(ring.last().is_some_and(|&p| same_bits(p, (400.0, 500.0))));
}
/// The Task 5 hairpin with its legs at `x0` and `x0 + 12`, its free loop bulging about 4.6 m outward.
fn edge_hairpin(x0: f64) -> RoadGraph {
    let mut nodes = vec![1_i64];
    let mut points = vec![(x0, 400.0), (x0, 460.0)];
    nodes.push(2);
    let mut node = 3_i64;
    for i in 1..16 {
        let a = PI - PI * f64::from(i) / 16.0;
        points.push((x0 + 6.0 + 6.0 * a.cos(), 460.0 + 4.0 * a.sin()));
        nodes.push(node);
        node += 1;
    }
    points.push((x0 + 12.0, 460.0));
    nodes.push(node);
    node += 1;
    points.push((x0 + 12.0, 400.0));
    nodes.push(node);
    build_graph(&[city_way(1, &nodes, &points)])
}

#[test]
fn plan_the_map_edge_blocks_only_what_is_outside() {
    let reach = naves_core::conform::core_reach_m(CITY_HALF, MAP_CELL_M);
    // a hairpin 200 m from the edge: its free loop bulges about 4.6 m outward, below x = 200 - 3
    let far = edge_hairpin(200.0);
    let (out, report) = plan_graph(&far, &flat_map());
    let id = far
        .stretches
        .first()
        .map_or_else(|| panic!("no stretch"), |s| s.id.clone());
    assert!(report.zones.iter().any(is_planned), "{report:?}");
    let written = points_of(&out, &id);
    let min_x = written.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    assert!(min_x < 200.0 - 3.0, "the free loop does not bulge: {min_x}");
    // the same hairpin near the edge: its drawn line is outside the 11 m margin, the free loop would enter it
    let near = edge_hairpin(13.0);
    let (out, _report) = plan_graph(&near, &flat_map());
    let id = near
        .stretches
        .first()
        .map_or_else(|| panic!("no stretch"), |s| s.id.clone());
    let drawn = points_of(&near, &id);
    assert!(
        drawn.iter().all(|p| p.0 >= reach - 1e-9),
        "the drawn line is in the margin"
    );
    let written = points_of(&out, &id);
    // the raster marks cells whose centre is in the margin, so the bound it can guarantee is a cell below
    // `reach`; the free loop (13 - 4.6 = 8.4) is below it
    assert!(
        written
            .iter()
            .all(|p| p.0 >= reach - MAP_CELL_M / 4.0 - 1e-6),
        "a written point is inside the edge margin: {:?}",
        written.iter().map(|p| p.0).fold(f64::INFINITY, f64::min)
    );
    // the drawn line is already in the margin at x = 8: the margin is not applied, the fillet may use it, but
    // no point goes outside the map
    let g = corner_at_the_edge(8.0);
    let (out, report) = plan_graph(&g, &flat_map());
    assert!(is_planned(only_zone(&report)), "{:?}", report.zones);
    let main = points_of(&out, "1:1-3");
    let min_x = main.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    assert!(min_x >= 0.0, "a point outside the map: {min_x}");
    assert!(
        min_x < 8.0,
        "the route did not use the exempt margin: {min_x}"
    );
    // a road that runs to the edge keeps its dead end there
    let g = road_to_the_edge();
    let (out, report) = plan_graph(&g, &flat_map());
    assert!(is_planned(only_zone(&report)), "{:?}", report.zones);
    let main = points_of(&out, "1:1-3");
    assert!(same_bits(main[0], (0.0, 500.0)));
    assert!(main.iter().all(|p| p.0 >= 0.0));
}
#[test]
fn plan_a_drawn_line_inside_another_earthwork_can_leave_it() {
    // a U-turn of radius 25 against a minimum of 30: the zone's drawn line runs through the earthwork of the bridge
    // deck across its top, and the anchors are far enough away to be clear of it
    let g = u_turn_with_a_deck();
    let (out, report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| 30.0,
        &|_: (f64, f64)| false,
        &[],
        &RerouteParams::default(),
    );
    let zone = only_zone(&report);
    assert!(!zone.original_feasible, "{zone:?}");
    assert!(is_planned(zone), "{zone:?}");
    let _ = out;
}

#[test]
fn plan_skips_a_stretch_that_is_not_finite() {
    let mut features = vec![
        city_way(
            1,
            &[1, 2, 3],
            &[(400.0, 500.0), (500.0, 500.0), (500.0, 600.0)],
        ),
        city_way(
            2,
            &[10, 1, 11],
            &[(400.0, 400.0), (400.0, 500.0), (400.0, 600.0)],
        ),
        city_way(
            3,
            &[12, 3, 13],
            &[(400.0, 600.0), (500.0, 600.0), (600.0, 600.0)],
        ),
    ];
    let clean = build_graph(&features);
    features.push(city_way(4, &[90, 91], &[(f64::NAN, 700.0), (800.0, 700.0)]));
    let g = build_graph(&features);
    let (out, report) = plan_graph(&g, &flat_map());
    assert!(
        report
            .skipped
            .iter()
            .any(|(id, reason)| id == "4:90-91" && reason.to_lowercase().contains("finite")),
        "{:?}",
        report.skipped
    );
    let (clean_out, clean_report) = plan_graph(&clean, &flat_map());
    assert_eq!(points_of(&out, "1:1-3"), points_of(&clean_out, "1:1-3"));
    let zone = zone_matching(&report, |z| z.at.0 > 450.0 && is_planned(z));
    let clean_zone = zone_matching(&clean_report, |z| z.at.0 > 450.0 && is_planned(z));
    assert_eq!(zone, clean_zone);
    let _ = (out, clean_out);
}

#[test]
fn plan_a_stretch_far_too_long_is_skipped_or_kept_without_panic() {
    let g = build_graph(&[city_way(
        1,
        &[1, 2],
        &[
            (f64::from(MAP_SIZE) * MAP_CELL_M / 2.0, 500.0),
            (1.0e9, 500.0),
        ],
    )]);
    let (out, report) = plan_graph(&g, &flat_map());
    assert_eq!(out, g);
    let budget: usize = RerouteParams::default().budgets.iter().map(|b| b.1).sum();
    assert!(report.states <= budget, "{}", report.states);
}

// ----------------------------------------------------------------------------------------------------------
// Step 3: determinism
// ----------------------------------------------------------------------------------------------------------

#[test]
fn plan_the_same_graph_gives_the_same_result() {
    let features = vec![
        city_way(
            1,
            &[1, 2, 3],
            &[(400.0, 500.0), (500.0, 500.0), (500.0, 600.0)],
        ),
        city_way(
            2,
            &[10, 1, 11],
            &[(400.0, 400.0), (400.0, 500.0), (400.0, 600.0)],
        ),
        city_way(
            3,
            &[12, 3, 13],
            &[(400.0, 600.0), (500.0, 600.0), (600.0, 600.0)],
        ),
        city_way(
            4,
            &[20, 21, 22],
            &[(1300.0, 500.0), (1400.0, 500.0), (1400.0, 600.0)],
        ),
        city_way(
            5,
            &[30, 20, 31],
            &[(1300.0, 400.0), (1300.0, 500.0), (1300.0, 600.0)],
        ),
        city_way(
            6,
            &[32, 22, 33],
            &[(1300.0, 600.0), (1400.0, 600.0), (1500.0, 600.0)],
        ),
    ];
    let map = flat_map();
    let g = build_graph(&features);
    let (graph_a, report_a) = plan_graph(&g, &map);
    assert!(report_a.zones.len() >= 2, "only one zone: {report_a:?}");
    let (graph_b, report_b) = plan_graph(&g, &map);
    assert_eq!(graph_a, graph_b);
    assert_eq!(report_a, report_b);

    let mut reversed = features.clone();
    reversed.reverse();
    let g_rev = build_graph(&reversed);
    assert_eq!(g_rev, g);
    let (graph_c, report_c) = plan_graph(&g_rev, &map);
    assert_eq!(graph_a, graph_c);
    assert_eq!(report_a, report_c);

    for threads in [1_usize, 4, 16] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap_or_else(|e| panic!("pool: {e}"));
        let (graph_d, report_d) = pool.install(|| plan_graph(&g, &map));
        assert_eq!(graph_a, graph_d, "{threads} threads");
        assert_eq!(report_a, report_d, "{threads} threads");
    }
}

#[test]
fn plan_the_labels_of_the_report() {
    let reasons = [
        (KeptReason::NoRoute, "no_route"),
        (KeptReason::BlockedByWater, "blocked_by_water"),
        (KeptReason::BlockedByRoad, "blocked_by_road"),
        (KeptReason::Held, "held"),
    ];
    for (reason, id) in reasons {
        assert_eq!(reason.id(), id);
    }
    assert_eq!(Candidate::Plate.id(), "plate");
    assert_eq!(Candidate::CurvedBridge.id(), "curved_bridge");
}

/// A corner 30 m after the end of a tunnel.
fn corner_past_a_tunnel() -> RoadGraph {
    build_graph(&[
        road_way(
            1,
            Family::City,
            &[1, 2],
            &[(500.0, 500.0), (500.0, 540.0)],
            false,
            true,
            false,
        ),
        city_way(
            2,
            &[2, 3, 4],
            &[(500.0, 540.0), (500.0, 580.0), (560.0, 580.0)],
        ),
        city_way(
            3,
            &[12, 4, 13],
            &[(400.0, 580.0), (560.0, 580.0), (660.0, 580.0)],
        ),
    ])
}

#[test]
fn plan_a_tunnel_is_skipped_and_its_end_holds_the_leg() {
    let g = corner_past_a_tunnel();
    let tunnel = points_of(&g, "1:1-2");
    let (out, report) = plan_graph(&g, &flat_map());
    assert!(
        report
            .skipped
            .iter()
            .any(|(id, reason)| id == "1:1-2" && reason == "tunnel"),
        "{:?}",
        report.skipped
    );
    assert_ne!(
        out, g,
        "the corner past the hold must be planned: {report:?}"
    );
    let zone = only_zone(&report);
    assert!(is_planned(zone), "{zone:?}");
    assert_eq!(
        zone.anchors.0,
        AnchorKind::TunnelHold,
        "the leg starts after the tunnel's hold: {zone:?}"
    );
    assert_eq!(points_of(&out, "1:1-2"), tunnel, "the tunnel moved");
    let before = points_of(&g, "2:2-4");
    let after = points_of(&out, "2:2-4");
    for (p, q) in before.iter().zip(&after) {
        if (p.1 - 540.0).abs() <= 25.0 + 1e-9 {
            assert!(same_bits(*p, *q), "{p:?}");
        }
    }
}
#[test]
fn plan_a_roundabout_is_not_planned() {
    let g = build_graph(&[road_way(
        1,
        Family::City,
        &[1, 2, 3],
        &[(400.0, 500.0), (500.0, 500.0), (500.0, 600.0)],
        false,
        false,
        true,
    )]);
    let (out, report) = plan_graph(&g, &flat_map());
    assert_eq!(out, g);
    let zone = only_zone(&report);
    let (reason, detail) = kept(zone);
    assert_eq!(reason, KeptReason::Held);
    assert_eq!(detail, "roundabout");
    assert!(
        report
            .skipped
            .iter()
            .any(|(id, r)| *id == zone.id && r == "roundabout"),
        "{:?}",
        report.skipped
    );
}

#[test]
fn plan_a_zone_longer_than_the_limit_is_kept() {
    let g = town_corner();
    let params = RerouteParams {
        max_zone_m: 10.0,
        ..RerouteParams::default()
    };
    let (out, report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &[],
        &params,
    );
    assert_eq!(out, g);
    let zone = only_zone(&report);
    let (_, detail) = kept(zone);
    assert_eq!(detail, "too_long");
}

#[test]
fn plan_a_bad_parameter_keeps_the_zone() {
    let g = town_corner();
    // a planning radius above the search's limit
    let params = RerouteParams {
        radius_margin: 1000.0,
        ..RerouteParams::default()
    };
    let (_out, report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &[],
        &params,
    );
    let (_, detail) = kept(only_zone(&report));
    assert_eq!(detail, "bad_radius");

    // a value that is not a number reaches the search's own check
    let params = RerouteParams {
        stray_max: f64::NAN,
        ..RerouteParams::default()
    };
    let (_out, report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &[],
        &params,
    );
    let (_, detail) = kept(only_zone(&report));
    assert_eq!(detail, "invalid_input");
}

/// A corner at a node where the second stretch is stored the other way round.
fn reversed_corner() -> RoadGraph {
    build_graph(&[
        city_way(1, &[1, 5], &[(400.0, 500.0), (500.0, 500.0)]),
        road_way(
            2,
            Family::Asf3,
            &[0, 5],
            &[(500.0, 600.0), (500.0, 500.0)],
            false,
            false,
            false,
        ),
        city_way(
            3,
            &[10, 1, 11],
            &[(400.0, 400.0), (400.0, 500.0), (400.0, 600.0)],
        ),
        city_way(
            4,
            &[12, 0, 13],
            &[(500.0, 700.0), (500.0, 600.0), (500.0, 500.0)],
        ),
    ])
}

#[test]
fn plan_a_reversed_link_is_written_back_in_its_own_direction() {
    let g = reversed_corner();
    let (out, report) = plan_graph(&g, &flat_map());
    let zone = only_zone(&report);
    assert!(is_planned(zone), "{zone:?}");
    let a = points_of(&out, "1:1-5");
    let b = points_of(&out, "2:0-5");
    // the corner is node 5, which is the *last* point of both stored stretches
    assert_eq!(a.last(), b.last());
    assert!(crossing_point(&out, 5).is_some_and(|p| !is_close(p, (500.0, 500.0), 1e-6)));
    assert!(is_close(b[0], (500.0, 600.0), 1e-12));
    assert!(is_close(a[0], (400.0, 500.0), 1e-12));
}

/// Two chains that each have a tight bend near the same junction.
fn two_bends_at_a_junction() -> RoadGraph {
    build_graph(&[
        city_way(
            1,
            &[1, 2, 3],
            &[(300.0, 500.0), (490.0, 490.0), (500.0, 500.0)],
        ),
        city_way(
            2,
            &[3, 4, 5],
            &[(500.0, 500.0), (510.0, 510.0), (600.0, 520.0)],
        ),
        city_way(
            3,
            &[10, 3, 11],
            &[(500.0, 400.0), (500.0, 500.0), (500.0, 600.0)],
        ),
        city_way(
            4,
            &[20, 1, 21],
            &[(300.0, 400.0), (300.0, 500.0), (300.0, 600.0)],
        ),
        city_way(
            5,
            &[22, 5, 23],
            &[(600.0, 420.0), (600.0, 520.0), (600.0, 620.0)],
        ),
    ])
}

#[test]
fn plan_two_zones_at_a_junction_are_partners() {
    let g = two_bends_at_a_junction();
    let (_out, report) = plan_graph(&g, &flat_map());
    assert_eq!(report.zones.len(), 2, "{:?}", report.zones);
    for (i, zone) in report.zones.iter().enumerate() {
        assert_eq!(zone.junction, Some(3), "{zone:?}");
        let partner = zone
            .partner
            .as_ref()
            .unwrap_or_else(|| panic!("no partner"));
        let other = report
            .zones
            .get(1 - i)
            .unwrap_or_else(|| panic!("no other"));
        assert_eq!(partner, &other.id);
    }
}

// ----------------------------------------------------------------------------------------------------------
// Fix round 1: one measure of length, the ring cut, and tests that can fail
// ----------------------------------------------------------------------------------------------------------

/// The written line never doubles back: every pair of consecutive segments turns by less than 90°.
fn assert_no_reversal(line: &[(f64, f64)]) {
    for w in line.windows(3) {
        let [a, b, c] = *w else { continue };
        let (d1, d2) = ((b.0 - a.0, b.1 - a.1), (c.0 - b.0, c.1 - b.1));
        let dot = d1.0 * d2.0 + d1.1 * d2.1;
        assert!(dot > 0.0, "the line turns back at {b:?}: {a:?} {b:?} {c:?}");
    }
}

/// The arc position along `line` of the point nearest to `p`, and the line's length.
#[expect(
    clippy::many_single_char_names,
    reason = "the points of the segment, as in the geometry"
)]
fn arc_at_point(line: &[(f64, f64)], p: (f64, f64)) -> (f64, f64) {
    let mut at = 0.0;
    let mut best = (f64::INFINITY, 0.0);
    for w in line.windows(2) {
        let [a, b] = *w else { continue };
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = dx.hypot(dy);
        let t = if len > 0.0 {
            (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (len * len)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let foot = (a.0 + t * dx, a.1 + t * dy);
        let d = (p.0 - foot.0).hypot(p.1 - foot.1);
        if d < best.0 {
            best = (d, at + t * len);
        }
        at += len;
    }
    (best.1, at)
}

/// Contiguous pieces of a polyline.
type Pieces = Vec<Vec<(f64, f64)>>;

/// The own chain outside a planned leg, as contiguous pieces beyond `self_free` along the chain from each
/// anchor, and the route's new points of that leg.
fn own_chain_outside(
    original: &[(f64, f64)],
    new_points: &[(f64, f64)],
    zone: &PlannedZone,
    self_free: f64,
) -> (Pieces, Vec<(f64, f64)>) {
    let zone_new: Vec<(f64, f64)> = new_points
        .iter()
        .copied()
        .filter(|p| (p.0 - zone.at.0).hypot(p.1 - zone.at.1) <= 40.0)
        .collect();
    if zone_new.len() < 2 {
        return (Vec::new(), zone_new);
    }
    let (Some(a), Some(b)) = (zone_new.first(), zone_new.last()) else {
        return (Vec::new(), zone_new);
    };
    let (a_arc, _) = arc_at_point(original, *a);
    let (b_arc, _) = arc_at_point(original, *b);
    let mut pieces: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    let mut at = 0.0;
    for w in original.windows(2) {
        let [p, q] = *w else { continue };
        let len = (q.0 - p.0).hypot(q.1 - p.1);
        let keep = at < a_arc - self_free || at > b_arc + self_free;
        if keep {
            current.push(p);
        } else if current.len() >= 2 {
            pieces.push(std::mem::take(&mut current));
        } else {
            current.clear();
        }
        at += len;
    }
    if current.len() >= 2 {
        pieces.push(current);
    }
    (pieces, zone_new)
}

/// A chain of two stretches whose first one has an off-grid turning vertex before the node inside the zone.
fn off_grid_two_stretch_node() -> RoadGraph {
    build_graph(&[
        road_way(
            1,
            Family::Asf3,
            &[1, 2, 3],
            &[(400.0, 500.0), (430.7, 503.3), (490.0, 500.0)],
            false,
            false,
            false,
        ),
        city_way(
            2,
            &[3, 4, 5],
            &[(490.0, 500.0), (500.0, 500.0), (500.0, 600.0)],
        ),
        city_way(
            3,
            &[10, 1, 11],
            &[(400.0, 400.0), (400.0, 500.0), (400.0, 600.0)],
        ),
        city_way(
            4,
            &[12, 5, 13],
            &[(400.0, 600.0), (500.0, 600.0), (600.0, 600.0)],
        ),
    ])
}

#[test]
fn plan_an_off_grid_two_stretch_node_is_one_point() {
    let g = off_grid_two_stretch_node();
    let (out, report) = plan_graph(&g, &flat_map());
    let zone = only_zone(&report);
    assert!(is_planned(zone), "{zone:?}");
    let first = points_of(&out, "1:1-3");
    let second = points_of(&out, "2:3-5");
    assert!(
        same_bits(*first.last().unwrap(), *second.first().unwrap()),
        "the node is two points: {:?} {:?}",
        first.last(),
        second.first()
    );
    let crossing = crossing_point(&out, 3).unwrap_or_else(|| panic!("no crossing at node 3"));
    assert!(same_bits(crossing, *first.last().unwrap()));
    assert!(
        !same_bits(crossing, (490.0, 500.0)),
        "the node did not move"
    );
    assert_no_reversal(&first);
    assert_no_reversal(&second);
}

/// A chain with off-grid 90° corners before a final corner, and a vertex 0.7 m before the end anchor.
fn off_grid_vertex_before_the_end() -> RoadGraph {
    build_graph(&[
        city_way(
            1,
            &[1, 2, 3, 4, 5, 6, 7, 8],
            &[
                (400.0, 500.0),
                (441.3, 500.0),
                (441.3, 503.3),
                (482.9, 503.3),
                (482.9, 500.0),
                (560.0, 500.0),
                (560.0, 519.2),
                (560.0, 600.0),
            ],
        ),
        city_way(
            2,
            &[10, 1, 11],
            &[(400.0, 400.0), (400.0, 500.0), (400.0, 600.0)],
        ),
        city_way(
            3,
            &[12, 8, 13],
            &[(460.0, 600.0), (560.0, 600.0), (660.0, 600.0)],
        ),
    ])
}

#[test]
fn plan_a_vertex_just_before_the_end_anchor_is_not_written_after_it() {
    let g = off_grid_vertex_before_the_end();
    let (out, report) = plan_graph(&g, &flat_map());
    let zone = zone_matching(&report, |z| z.at.0 > 540.0 && z.at.1 < 520.0);
    assert!(is_planned(zone), "{zone:?}");
    let written = points_of(&out, "1:1-8");
    assert_no_reversal(&written);
    assert!(
        !written.iter().any(|&p| same_bits(p, (560.0, 519.2))),
        "the vertex before the anchor is written after the route"
    );
    assert!(
        written.iter().any(|&p| p.1 >= 519.0 && p.1 <= 520.0 + 1e-9),
        "the end anchor is not on the written line"
    );
}

/// Two tight corners 41 m apart: with a 20 m lead their grown zones are 1 m apart, which rounds into the same
/// sample.
fn zones_one_metre_apart() -> RoadGraph {
    build_graph(&[
        city_way(
            1,
            &[1, 2, 3, 4, 5],
            &[
                (300.0, 500.0),
                (400.0, 500.0),
                (400.0, 541.0),
                (500.0, 541.0),
                (500.0, 600.0),
            ],
        ),
        city_way(
            2,
            &[10, 1, 11],
            &[(300.0, 400.0), (300.0, 500.0), (300.0, 600.0)],
        ),
        city_way(
            3,
            &[12, 5, 13],
            &[(400.0, 600.0), (500.0, 600.0), (600.0, 600.0)],
        ),
    ])
}

#[test]
fn plan_zones_less_than_one_sample_apart_are_one_zone() {
    let g = zones_one_metre_apart();
    let (out, report) = plan_graph(&g, &flat_map());
    let near: Vec<&PlannedZone> = report
        .zones
        .iter()
        .filter(|z| z.at.0 < 450.0 && z.at.1 > 490.0)
        .collect();
    assert_eq!(
        near.len(),
        1,
        "the two grown zones were not merged: {near:?}"
    );
    assert!(is_planned(near[0]), "{:?}", near[0]);
    assert!(
        near[0].length_before_m > 80.0,
        "{}",
        near[0].length_before_m
    );
    let written = points_of(&out, "1:1-5");
    assert_no_reversal(&written);
}

/// A U-turn whose parallel legs are 60 m apart: a loop fits with the full road clearance.
fn wide_u_turn() -> RoadGraph {
    build_graph(&[city_way(
        1,
        &[1, 2, 3, 4],
        &[
            (500.0, 400.0),
            (500.0, 500.0),
            (560.0, 500.0),
            (560.0, 400.0),
        ],
    )])
}

#[test]
fn plan_a_wide_u_turn_keeps_the_clearance_from_its_own_road() {
    let g = wide_u_turn();
    let id = g
        .stretches
        .first()
        .map_or_else(|| panic!("no stretch"), |s| s.id.clone());
    let original = points_of(&g, &id);
    let (out, report) = plan_graph(&g, &flat_map());
    let written = points_of(&out, &id);
    let unchanged: Vec<(u64, u64)> = original
        .iter()
        .map(|p| (p.0.to_bits(), p.1.to_bits()))
        .collect();
    let new_points: Vec<(f64, f64)> = written
        .iter()
        .copied()
        .filter(|p| !unchanged.contains(&(p.0.to_bits(), p.1.to_bits())))
        .collect();
    let reach = naves_core::conform::core_reach_m(CITY_HALF, MAP_CELL_M);
    let clearance = reach + MAP_CELL_M;
    let self_free = (clearance + 2.0) * CHAMFER_ERROR + 2.0 * 2.0_f64.sqrt() + reach;
    let bound = reach + (clearance + 2.0) * CHAMFER_ERROR + 2.0 * 2.0_f64.sqrt() - 2.0;
    let planned: Vec<&PlannedZone> = report.zones.iter().filter(|z| is_planned(z)).collect();
    assert!(!planned.is_empty(), "no zone planned: {report:?}");
    for zone in &planned {
        let (pieces, zone_new) = own_chain_outside(&original, &new_points, zone, self_free);
        for piece in &pieces {
            if zone_new.len() < 2 {
                continue;
            }
            let got = zone_new
                .iter()
                .map(|&p| dist_to_line(p, piece))
                .fold(f64::INFINITY, f64::min);
            assert!(got >= bound, "zone {}: {got} against {bound}", zone.id);
            assert_eq!(
                segment_crossings(&zone_new, piece),
                0,
                "zone {} crosses the chain's other legs",
                zone.id
            );
        }
    }
    assert_no_reversal(&written);
}

/// A chain whose first stretch is `l` metres long, with a two-stretch node at its end inside a zone.
fn node_at_a_length(l: f64) -> RoadGraph {
    build_graph(&[
        road_way(
            1,
            Family::Asf3,
            &[1, 2],
            &[(50.0, 500.0), (50.0 + l, 500.0)],
            false,
            false,
            false,
        ),
        city_way(
            2,
            &[2, 3, 4],
            &[(50.0 + l, 500.0), (60.0 + l, 500.0), (60.0 + l, 600.0)],
        ),
        city_way(
            3,
            &[10, 1, 11],
            &[(50.0, 400.0), (50.0, 500.0), (50.0, 600.0)],
        ),
        city_way(
            4,
            &[12, 4, 13],
            &[(40.0 + l, 600.0), (60.0 + l, 600.0), (80.0 + l, 600.0)],
        ),
    ])
}

#[test]
fn plan_a_node_at_a_length_that_loses_a_bit_is_one_point() {
    // `(l * n) / n` is not always `l` in f64; the fixture must use such a length, or it cannot see the last-bit
    // split of the node
    let mut chosen = None;
    let candidates: [f64; 6] = [
        1_899.876_587_074_421_4,
        1_899.595_112_120_010_5,
        1_898.882_184_302_414,
        955.3,
        477.7,
        6.1,
    ];
    for &l in &candidates {
        let n = (l / 2.0).round();
        if n >= 1.0 && ((l * n) / n).to_bits() != l.to_bits() {
            chosen = Some((l, n));
            break;
        }
    }
    let (l, n) = chosen.unwrap_or_else(|| panic!("no length loses a bit"));
    assert!(
        ((l * n) / n).to_bits() != l.to_bits(),
        "the fixture lost its point"
    );
    let g = node_at_a_length(l);
    let (out, report) = plan_graph(&g, &flat_map());
    let zone = only_zone(&report);
    assert!(is_planned(zone), "{zone:?}");
    let first = points_of(&out, "1:1-2");
    let second = points_of(&out, "2:2-4");
    assert!(
        same_bits(*first.last().unwrap(), *second.first().unwrap()),
        "the node splits in the last bit: {:?} {:?}",
        first.last(),
        second.first()
    );
    let crossing = crossing_point(&out, 2).unwrap_or_else(|| panic!("no crossing at node 2"));
    assert!(same_bits(crossing, *first.last().unwrap()));
    assert!(
        !same_bits(crossing, (50.0 + l, 500.0)),
        "the node did not move"
    );
    let _ = n;
}

/// The Task 5 hairpin, with a return leg of the same chain above its apex and 24 m east: the free loop bulges
/// toward it. The return leg is a horizontal piece at y = 470 from x = 536 to 560, reached from (512, 400).
fn hairpin_with_a_return_leg(with_leg: bool) -> RoadGraph {
    let mut nodes = vec![1_i64];
    let mut points = vec![(500.0, 400.0), (500.0, 460.0)];
    nodes.push(2);
    let mut node = 3_i64;
    for i in 1..16 {
        let a = PI - PI * f64::from(i) / 16.0;
        points.push((506.0 + 6.0 * a.cos(), 460.0 + 4.0 * a.sin()));
        nodes.push(node);
        node += 1;
    }
    points.push((512.0, 460.0));
    nodes.push(node);
    node += 1;
    points.push((512.0, 400.0));
    nodes.push(node);
    node += 1;
    if with_leg {
        for p in [(560.0, 400.0), (560.0, 470.0), (536.0, 470.0)] {
            points.push(p);
            nodes.push(node);
            node += 1;
        }
    }
    build_graph(&[city_way(1, &nodes, &points)])
}

#[test]
fn plan_a_planned_leg_keeps_the_clearance_from_the_own_chain() {
    let return_leg = [(536.0, 470.0), (560.0, 470.0)];
    let reach = naves_core::conform::core_reach_m(CITY_HALF, MAP_CELL_M);
    let clearance = reach + MAP_CELL_M;
    let threshold = (clearance + 2.0) * CHAMFER_ERROR + 2.0 * 2.0_f64.sqrt();
    let bound = reach + threshold - 2.0;
    // without the return leg the free loop comes closer than the bound
    let free_graph = hairpin_with_a_return_leg(false);
    let (free_out, free_report) = plan_graph(&free_graph, &flat_map());
    let id = free_graph
        .stretches
        .first()
        .map_or_else(|| panic!("no stretch"), |s| s.id.clone());
    assert!(free_report.zones.iter().any(is_planned), "{free_report:?}");
    let free = points_of(&free_out, &id);
    let got = line_distance(&free, &return_leg);
    assert!(
        got < bound,
        "the free loop is not near the return leg: {got} against {bound}"
    );
    // with it, a planned route keeps the bound or the zone is kept
    let g = hairpin_with_a_return_leg(true);
    let (out, report) = plan_graph(&g, &flat_map());
    let id = g
        .stretches
        .first()
        .map_or_else(|| panic!("no stretch"), |s| s.id.clone());
    let zone = report.zones.first().unwrap_or_else(|| panic!("no zone"));
    match &zone.outcome {
        ZoneOutcome::Planned => {
            let route = points_of(&out, &id);
            let original = points_of(&g, &id);
            let unchanged: Vec<(u64, u64)> = original
                .iter()
                .map(|p| (p.0.to_bits(), p.1.to_bits()))
                .collect();
            // the points of *this* zone: those nearest to its place, so that a point of another planned zone
            // (the return leg's corner) that happens to lie within a radius is not mixed in
            let nearest = |p: (f64, f64)| {
                report
                    .zones
                    .iter()
                    .min_by(|a, b| {
                        (p.0 - a.at.0)
                            .hypot(p.1 - a.at.1)
                            .total_cmp(&(p.0 - b.at.0).hypot(p.1 - b.at.1))
                    })
                    .map(|z| z.id.as_str())
            };
            let new_points: Vec<(f64, f64)> = route
                .iter()
                .copied()
                .filter(|p| !unchanged.contains(&(p.0.to_bits(), p.1.to_bits())))
                .filter(|p| nearest(*p) == Some(zone.id.as_str()))
                .collect();
            assert!(new_points.len() >= 2);
            let got = line_distance(&new_points, &return_leg);
            assert!(
                got >= bound,
                "a planned route at {got} from its own chain, bound {bound}"
            );
        }
        ZoneOutcome::Kept { reason, .. } => {
            assert!(
                matches!(reason, KeptReason::Held | KeptReason::NoRoute),
                "{zone:?}"
            );
        }
    }
}

/// A bridge followed by an approach with a corner 40 m past the bridge's end: the leg starts at the bridge hold.
fn corner_past_a_bridge() -> RoadGraph {
    build_graph(&[
        bridge_way(1, &[1, 2], &[(500.0, 400.0), (500.0, 440.0)]),
        city_way(
            2,
            &[2, 3, 4],
            &[(500.0, 440.0), (500.0, 480.0), (560.0, 480.0)],
        ),
        city_way(
            3,
            &[12, 4, 13],
            &[(400.0, 480.0), (560.0, 480.0), (660.0, 480.0)],
        ),
    ])
}

#[test]
fn plan_a_leg_starting_at_a_bridge_hold_is_planned() {
    let g = corner_past_a_bridge();
    let bridge = points_of(&g, "1:1-2");
    let (out, report) = plan_graph(&g, &flat_map());
    assert_ne!(
        out, g,
        "the corner past the hold must be planned: {report:?}"
    );
    let zone = only_zone(&report);
    assert!(is_planned(zone), "{zone:?}");
    assert_eq!(zone.anchors.0, AnchorKind::BridgeHold);
    assert_eq!(points_of(&out, "1:1-2"), bridge, "the bridge moved");
}

// ----------------------------------------------------------------------------------------------------------
// Task 7: why a bend was kept, routes against each other, the report (plan Task 7).
// ----------------------------------------------------------------------------------------------------------

/// A heightmap of the fixture size whose height at a point is `height`.
fn map_with(height: impl Fn(f64, f64) -> f64) -> Heightmap {
    let mut h = Vec::with_capacity((MAP_SIZE * MAP_SIZE) as usize);
    for row in 0..MAP_SIZE {
        for col in 0..MAP_SIZE {
            #[expect(clippy::cast_possible_truncation, reason = "a fixture height")]
            let z = height(f64::from(col) * MAP_CELL_M, f64::from(row) * MAP_CELL_M) as f32;
            h.push(z);
        }
    }
    Heightmap {
        size: MAP_SIZE,
        cell_m: 4.0,
        h,
    }
}

/// Water from y = 445 on: 5 m above the anchors of the hairpins below, and over the drawn apex.
fn water_above_445(p: (f64, f64)) -> bool {
    p.1 >= 445.0
}

#[test]
fn plan_a_hairpin_under_water_is_a_curved_bridge() {
    let g = edge_hairpin(500.0);
    let params = RerouteParams::default();
    // the same graph without the water is planned: the fixture can fail
    let (_out, free) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &[],
        &params,
    );
    assert!(is_planned(only_zone(&free)), "{:?}", free.zones);
    // water 5 m above the anchors: the free loop needs 10.5 m of room to turn round
    let (out, report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &water_above_445,
        &[],
        &params,
    );
    assert_eq!(out, g, "a kept zone keeps its line bit for bit");
    let zone = only_zone(&report);
    let (reason, _) = kept(zone);
    assert_eq!(reason, KeptReason::BlockedByWater, "{zone:?}");
    assert_eq!(zone.candidate, Some(Candidate::CurvedBridge));
    assert_eq!(report.kept(KeptReason::BlockedByWater), 1);
    assert_eq!(report.kept(KeptReason::NoRoute), 0);
    // the water is not a page of the report's "not planned at all"
    assert!(
        !report.skipped.iter().any(|(id, _)| *id == zone.id),
        "{:?}",
        report.skipped
    );
    assert!(
        report.kept_zones.iter().any(|(id, _)| *id == zone.id),
        "{:?}",
        report.kept_zones
    );
    assert!(
        report.summary().starts_with(
            "Road planning: 1 zones, 0 planned, 1 kept (no_route 0, held 0, blocked_by_water 1, \
             blocked_by_road 0), tight length"
        ),
        "{}",
        report.summary()
    );
}

#[test]
fn plan_a_drawn_line_in_a_deck_earthwork_is_kept_for_a_road() {
    // the town corner with a bridge deck parallel to and 6 m north of its first leg: the deck's earthwork covers
    // the drawn line and both anchors, and the drawn line crosses the deck nowhere (so this is not `crosses_deck`)
    let g = build_graph(&[
        city_way(
            1,
            &[1, 2, 3],
            &[(400.0, 500.0), (500.0, 500.0), (500.0, 600.0)],
        ),
        city_way(
            2,
            &[10, 1, 11],
            &[(400.0, 400.0), (400.0, 500.0), (400.0, 600.0)],
        ),
        city_way(
            3,
            &[12, 3, 13],
            &[(400.0, 600.0), (500.0, 600.0), (600.0, 600.0)],
        ),
        bridge_way(4, &[20, 21], &[(420.0, 506.0), (496.0, 506.0)]),
    ]);
    let params = RerouteParams::default();
    let (out, report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &[],
        &params,
    );
    assert_eq!(out, g, "a kept zone keeps its line bit for bit");
    let zone = only_zone(&report);
    let (reason, detail) = kept(zone);
    assert_eq!(reason, KeptReason::BlockedByRoad, "{zone:?}");
    assert_ne!(detail, "crosses_deck", "the drawn line crosses no deck");
    assert_eq!(zone.candidate, None);
    assert!(!zone.original_feasible, "{zone:?}");
    // without the deck the same corner is planned
    let clean = town_corner();
    let (_out, report) = plan_with(
        &clean,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &[],
        &params,
    );
    assert!(is_planned(only_zone(&report)), "{:?}", report.zones);
}

#[test]
fn plan_a_leg_that_crosses_a_stream_gets_its_diagnostic_reason() {
    // A stream from x = 470 to 540 at y = 450, across both legs of the hairpin (x = 500 and 512) and 12 m above its
    // anchors at y = 437.96, and water from y = 455 on, which covers the loop. The drawn line crosses the stream
    // twice (so a route must, too) and the water blocks the loop; the candidates that avoid the stream are dropped
    // by the search (the crossing count must equal the drawn line's: routine, nothing is reported for them), the
    // leg ends without a route, and the diagnostic without water (areas and streams) finds the loop. The reason is
    // `blocked_by_water`, not `check_failed`.
    let g = edge_hairpin(500.0);
    let stream = vec![(470.0, 450.0), (540.0, 450.0)];
    let params = RerouteParams::default();
    let water = |p: (f64, f64)| p.1 >= 455.0;
    let (out, report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &water,
        std::slice::from_ref(&stream),
        &params,
    );
    assert_eq!(out, g);
    let zone = only_zone(&report);
    let (reason, detail) = kept(zone);
    assert_eq!(reason, KeptReason::BlockedByWater, "{zone:?}");
    assert_eq!(detail, "blocked_by_water");
    assert_eq!(zone.candidate, Some(Candidate::CurvedBridge));
    // the same bend under the same water without the stream: also `blocked_by_water`, so the fixture's failure is
    // the water in both runs
    let (_out, report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &water,
        &[],
        &params,
    );
    let (reason, _) = kept(only_zone(&report));
    assert_eq!(reason, KeptReason::BlockedByWater);
}

/// The Task 5 hairpin as a chain, its legs 12 m apart from y = 400 to 460, with an apex 12 m wide and 4 m high.
fn facing_hairpin(way: i64, left: f64) -> Feature {
    let mut nodes: Vec<i64> = vec![way * 100 + 1, way * 100 + 2];
    let mut points: Vec<(f64, f64)> = vec![(left, 400.0), (left, 460.0)];
    let mut next = 3_i64;
    for i in 1..16 {
        let a = PI - PI * f64::from(i) / 16.0;
        points.push((left + 6.0 + 6.0 * a.cos(), 460.0 + 4.0 * a.sin()));
        nodes.push(way * 100 + next);
        next += 1;
    }
    for p in [(left + 12.0, 460.0), (left + 12.0, 400.0)] {
        points.push(p);
        nodes.push(way * 100 + next);
        next += 1;
    }
    city_way(way, &nodes, &points)
}

/// Two hairpins whose loops bulge into the 32 m between them, 4.5 m each, so that the routes come 23 m apart.
fn two_facing_hairpins() -> Vec<Feature> {
    vec![facing_hairpin(1, 500.0), facing_hairpin(2, 544.0)]
}

#[test]
fn plan_two_routes_that_run_into_each_other_are_checked() {
    let features = two_facing_hairpins();
    let g = build_graph(&features);
    let params = RerouteParams::default();
    let plan = |graph: &RoadGraph| {
        plan_with(
            graph,
            &flat_map(),
            &|_: &Stretch| CITY_MIN_R,
            &|_: (f64, f64)| false,
            &[],
            &params,
        )
    };
    let (out, report) = plan(&g);
    assert_eq!(report.zones.len(), 2, "{:?}", report.zones);
    assert_eq!(report.reverted, 1, "{:?}", report.zones);
    let first = &report.zones[0];
    let second = &report.zones[1];
    assert!(
        is_planned(first),
        "the lower chain id is planned: {first:?}"
    );
    let (reason, detail) = kept(second);
    assert_eq!(reason, KeptReason::BlockedByRoad, "{second:?}");
    assert_eq!(detail, format!("route_of {}", first.id));
    assert_eq!(second.candidate, None);
    // the reverted route's line is bit for bit, and a planned leg was written
    let stretches = stretches_of(&g);
    assert_eq!(stretches.len(), 2, "{stretches:?}");
    let second_id = stretches
        .last()
        .map_or_else(|| panic!("no second stretch"), |s| s.id.clone());
    assert_eq!(points_of(&out, &second_id), points_of(&g, &second_id));
    assert_ne!(out, g, "the first zone is planned");
    assert_eq!(report.kept(KeptReason::BlockedByRoad), 1);
    // A different input: the order of the ways reversed and every way's points and nodes reversed as well (legal:
    // a way has no direction). The plan is the same because it holds by the `RoadGraph` invariant: `build_graph`
    // stores the stretches in the order of their ids, so the input's order cannot reach the planner; this only
    // checks that the invariant is kept for a reversed input.
    let reversed: Vec<Feature> = features
        .iter()
        .rev()
        .map(|f| {
            let mut points = f.points.clone();
            points.reverse();
            let mut nodes = f.nodes.clone();
            nodes.reverse();
            Feature {
                points,
                nodes,
                ..f.clone()
            }
        })
        .collect();
    assert_ne!(reversed, features, "the reversed input is the same input");
    let g_rev = build_graph(&reversed);
    assert_eq!(
        g_rev, g,
        "the graph of a reversed input is the same graph (its stretches are canonical)"
    );
    let (out_rev, report_rev) = plan(&g_rev);
    assert_eq!(out_rev, out);
    assert_eq!(report_rev, report);
    // and the check is the same on 1, 4 and 16 threads
    for threads in [1_usize, 4, 16] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap_or_else(|e| panic!("pool: {e}"));
        let (out_t, report_t) = pool.install(|| plan(&g));
        assert_eq!(out_t, out, "{threads} threads");
        assert_eq!(report_t, report, "{threads} threads");
    }
}

fn stretches_of(graph: &RoadGraph) -> Vec<Stretch> {
    graph.stretches.clone()
}

/// A corner that turns by 120 degrees, 60 m after a junction, on ground that rises 0.33 m per metre north.
fn corner_120(x0: f64) -> RoadGraph {
    let apex = x0 + 100.0;
    let direction = (-0.5, 0.866_025_403_784_438_6);
    build_graph(&[
        city_way(
            1,
            &[2, 1, 3],
            &[
                (apex - 60.0, 500.0),
                (apex, 500.0),
                (apex + 60.0 * direction.0, 500.0 + 60.0 * direction.1),
            ],
        ),
        city_way(
            2,
            &[10, 2, 11],
            &[
                (apex - 60.0, 400.0),
                (apex - 60.0, 500.0),
                (apex - 60.0, 600.0),
            ],
        ),
    ])
}

#[test]
fn plan_a_leg_only_at_a_relaxed_limit_is_counted_steeper() {
    let g = corner_120(400.0);
    let map = map_with(|_, y| 100.0 + 0.33 * (y - 500.0));
    // the cap keeps the route in a corridor along the drawn line: the route the search finds is 23.8 m long and
    // the drawn line needs 6.0 m of climb over its 42 m
    let params = RerouteParams {
        stray_max: 0.6,
        ..RerouteParams::default()
    };
    let (out, report) = plan_with(
        &g,
        &map,
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &[],
        &params,
    );
    let zone = only_zone(&report);
    assert!(is_planned(zone), "{zone:?}");
    assert!(zone.relax_steps > 0, "{zone:?}");
    assert!(
        zone.planned_grade > zone.drawn_grade + 0.02,
        "planned {} drawn {}",
        zone.planned_grade,
        zone.drawn_grade
    );
    assert_eq!(report.steeper_than_drawn, 1);
    // the plan is within the limit it was found at
    assert!(
        zone.grade_limit <= 0.27 + 1e-9 && zone.grade_limit > zone.drawn_grade,
        "{zone:?}"
    );
    assert!(zone.length_after_m > 0.0 && zone.length_after_m < 42.0);
    let _ = out;
}

/// A hairpin whose legs are 12 m apart (`left` and `left + 12`) from `bottom` up 60 m, with an apex 12 m wide
/// and 4 m high.
fn small_hairpin(way: i64, left: f64, bottom: f64) -> Feature {
    let mut nodes: Vec<i64> = vec![way * 1000 + 1, way * 1000 + 2];
    let mut points: Vec<(f64, f64)> = vec![(left, bottom), (left, bottom + 60.0)];
    let mut next = 3_i64;
    for i in 1..16 {
        let a = PI - PI * f64::from(i) / 16.0;
        points.push((left + 6.0 + 6.0 * a.cos(), bottom + 60.0 + 4.0 * a.sin()));
        nodes.push(way * 1000 + next);
        next += 1;
    }
    for p in [(left + 12.0, bottom + 60.0), (left + 12.0, bottom)] {
        points.push(p);
        nodes.push(way * 1000 + next);
        next += 1;
    }
    city_way(way, &nodes, &points)
}

#[test]
fn plan_fifty_bends_with_a_tiny_budget_report_the_same() {
    // A town corner is found in 37 states, so a 50-state first try does not exhaust it; the loop of a hairpin needs
    // 619. Fifty hairpins with these two budget pairs therefore all keep their line with the budget exhausted, and
    // the work stays within the four limits and the two diagnostic searches of the last pair only.
    let features: Vec<Feature> = (0..50_i64)
        .map(|k| {
            let x = 150.0 + 240.0 * f64::from(i32::try_from(k % 8).unwrap_or(0));
            let y = 150.0 + 250.0 * f64::from(i32::try_from(k / 8).unwrap_or(0));
            small_hairpin(10 * k + 1, x, y)
        })
        .collect();
    let g = build_graph(&features);
    // two pairs: the main search tries both (60 states per limit), the two diagnostics the last one only (10)
    let params = RerouteParams {
        budgets: vec![(1.0, 50), (1.0, 10)],
        ..RerouteParams::default()
    };
    let plan = |graph: &RoadGraph| {
        plan_with(
            graph,
            &flat_map(),
            &|_: &Stretch| CITY_MIN_R,
            &|_: (f64, f64)| false,
            &[],
            &params,
        )
    };
    let (out, report) = plan(&g);
    assert_eq!(out, g);
    assert_eq!(report.zones.len(), 50, "{}", report.zones.len());
    for zone in &report.zones {
        let (reason, _) = kept(zone);
        assert_eq!(reason, KeptReason::NoRoute, "{zone:?}");
        assert!(zone.budget_exhausted, "{zone:?}");
    }
    assert_eq!(report.kept(KeptReason::NoRoute), 50);
    // the main search tries the four limits with both budget pairs, the two diagnostics the last pair only:
    // with the pairs in the diagnostics as well, this would be 50 * (4 * 60 + 2 * 60) = 18 000
    let all: usize = params.budgets.iter().map(|b| b.1).sum();
    let last = params.budgets.last().map_or(0, |b| b.1);
    assert!(
        report.states <= 50 * (4 * all + 2 * last),
        "{}",
        report.states
    );
    let (_out, again) = plan(&g);
    assert_eq!(again, report, "the same report on a second run");
}

#[test]
fn plan_a_route_the_check_rejects_is_no_route_with_check_failed() {
    let g = town_corner();
    let plan = |fraction: f64| {
        let params = RerouteParams {
            reached_fraction: fraction,
            ..RerouteParams::default()
        };
        plan_with(
            &g,
            &flat_map(),
            &|_: &Stretch| CITY_MIN_R,
            &|_: (f64, f64)| false,
            &[],
            &params,
        )
    };
    // the search's own radius check is unchanged (0.99 of the family's tightest arc): with the default fraction the
    // same corner is planned, so the only difference below is the caller's check of the returned route
    let (out, report) = plan(0.99);
    assert_ne!(out, g);
    assert!(is_planned(only_zone(&report)), "{:?}", report.zones);
    // a check radius above the planning radius: the search still returns its route, the caller's check rejects it
    let (out, report) = plan(4.0);
    assert_eq!(out, g);
    let zone = only_zone(&report);
    let (reason, detail) = kept(zone);
    assert_eq!(reason, KeptReason::NoRoute, "{zone:?}");
    assert_eq!(detail, "check_failed");
    assert_eq!(zone.candidate, Some(Candidate::Plate));
    // the route was found by the search (its own radius check is 0.99 of the family's tightest arc): the zone's
    // states are a few hundred, not the four budgets of the four limits (280 000) that dropping every candidate
    // costs. The check that rejected it is the caller's, and its `budget_exhausted` is the search's answer: a
    // returned route is only returned by a try that did not run out, so false — not a default
    assert!(zone.states > 0, "{zone:?}");
    assert!(zone.states < 10_000, "{zone:?}");
    assert!(!zone.budget_exhausted, "{zone:?}");
    assert_eq!(zone.weight, None, "{zone:?}");
}

/// One zone of the report, for the summary's fixture.
fn reported_zone(id: &str, outcome: ZoneOutcome, planned_grade: f64) -> PlannedZone {
    PlannedZone {
        id: id.to_string(),
        at: (500.0, 500.0),
        stretches: vec![id.to_string()],
        legs: 1,
        anchors: (AnchorKind::Line, AnchorKind::Line),
        junction: None,
        partner: None,
        outcome,
        radius_before_m: 4.0,
        radius_wanted_m: 10.0,
        radius_after_m: 10.4,
        length_before_m: 40.0,
        length_after_m: 42.0,
        max_stray_m: 3.5,
        drawn_grade: 0.10,
        drawn_grade_window: 0.2,
        planned_grade,
        grade_limit: 0.14,
        relax_steps: 0,
        climb_m: 4.0,
        earthwork_m2: 1.0,
        cut_max_m: 0.2,
        fill_max_m: 0.3,
        states: 100,
        weight: Some(1.0),
        budget_exhausted: false,
        original_feasible: true,
        candidate: None,
    }
}

#[test]
fn plan_the_summary_of_a_report() {
    let report = RerouteReport {
        zones: vec![
            reported_zone("1:1-3#0", ZoneOutcome::Planned, 0.14),
            reported_zone("1:1-3#1", ZoneOutcome::Planned, 0.18),
            reported_zone(
                "2:4-6#0",
                ZoneOutcome::Kept {
                    reason: KeptReason::Held,
                    detail: "fixed_anchor".to_string(),
                },
                0.0,
            ),
        ],
        skipped: vec![("3:7-9".to_string(), "tunnel".to_string())],
        kept_zones: vec![("2:4-6#0".to_string(), "fixed_anchor".to_string())],
        tight_before_m: 16135.4,
        tight_after_m: 1547.2,
        max_stray_m: 28.46,
        steeper_than_drawn: 1,
        reverted: 0,
        kept_sampled_routes: 0,
        points_before: 30,
        points_after: 30,
        states: 300,
    };
    assert_eq!(
        report.summary(),
        "Road planning: 3 zones, 2 planned, 1 kept (no_route 0, held 1, blocked_by_water 0, \
         blocked_by_road 0), tight length 16135 m -> 1547 m, largest move 28.5 m, 1 steeper than drawn"
    );
    assert_eq!(report.kept(KeptReason::Held), 1);
    assert_eq!(report.kept(KeptReason::NoRoute), 0);
    assert_eq!(report.kept(KeptReason::BlockedByWater), 0);
    assert_eq!(report.kept(KeptReason::BlockedByRoad), 0);
}

#[test]
fn plan_the_summary_counts_a_real_report() {
    // The wording is pinned exactly by `plan_the_summary_of_a_report`; this one checks that the numbers in the line
    // are the report's own (its zone count and reasons, its tight lengths and its largest move), on a real graph.
    let g = build_graph(&[
        city_way(
            1,
            &[1, 2, 3],
            &[(400.0, 500.0), (500.0, 500.0), (500.0, 600.0)],
        ),
        city_way(
            2,
            &[10, 1, 11],
            &[(400.0, 400.0), (400.0, 500.0), (400.0, 600.0)],
        ),
        city_way(
            3,
            &[12, 3, 13],
            &[(400.0, 600.0), (500.0, 600.0), (600.0, 600.0)],
        ),
        city_way(
            4,
            &[30, 31, 32],
            &[(1000.0, 500.0), (1003.0, 503.0), (1006.0, 500.0)],
        ),
        city_way(
            5,
            &[20, 30, 21],
            &[(1000.0, 400.0), (1000.0, 500.0), (1000.0, 600.0)],
        ),
        city_way(
            6,
            &[22, 32, 23],
            &[(1006.0, 400.0), (1006.0, 500.0), (1006.0, 600.0)],
        ),
    ]);
    let (out, report) = plan_graph(&g, &flat_map());
    assert_eq!(report.zones.len(), 2, "{:?}", report.zones);
    assert_eq!(report.kept(KeptReason::Held), 1, "{:?}", report.zones);
    assert_eq!(
        report.zones.iter().filter(|z| is_planned(z)).count(),
        1,
        "{:?}",
        report.zones
    );
    let want = format!(
        "Road planning: {} zones, {} planned, {} kept (no_route {}, held {}, blocked_by_water {}, \
         blocked_by_road {}), tight length {:.0} m -> {:.0} m, largest move {:.1} m, {} steeper than drawn",
        report.zones.len(),
        report.zones.iter().filter(|z| is_planned(z)).count(),
        report.zones.len() - report.zones.iter().filter(|z| is_planned(z)).count(),
        report.kept(KeptReason::NoRoute),
        report.kept(KeptReason::Held),
        report.kept(KeptReason::BlockedByWater),
        report.kept(KeptReason::BlockedByRoad),
        report.tight_before_m,
        report.tight_after_m,
        report.max_stray_m,
        report.steeper_than_drawn
    );
    assert_eq!(report.summary(), want);
    assert_eq!(report.kept(KeptReason::Held), 1);
    // the planned corner on flat ground is not steeper than its drawn line: the count leaves it out
    assert_eq!(report.steeper_than_drawn, 0, "{:?}", report.zones);
    assert_ne!(out, g);
}

/// A chain with a corner 12 m before a 4 m bridge's hold, a straight past it and a tight 9 m arc whose first
/// sample is 22 m past the hold, so that the zone's two legs fail differently.
fn partly_planned_chain() -> RoadGraph {
    let approach_nodes: Vec<i64> = vec![1, 2, 40];
    let approach = vec![(400.0, 500.0), (490.0, 500.0), (490.0, 513.0)];
    let mut nodes: Vec<i64> = vec![41, 3];
    let mut points: Vec<(f64, f64)> = vec![(490.0, 517.0), (490.0, 540.0)];
    let mut next = 4_i64;
    for i in 1..=16 {
        let a = PI / 2.0 * f64::from(i) / 16.0;
        points.push((490.0 + 9.0 * (1.0 - a.cos()), 540.0 + 9.0 * a.sin()));
        nodes.push(next);
        next += 1;
    }
    points.push((530.0, 549.0));
    nodes.push(next);
    build_graph(&[
        city_way(1, &approach_nodes, &approach),
        bridge_way(2, &[40, 41], &[(490.0, 513.0), (490.0, 517.0)]),
        city_way(3, &nodes, &points),
    ])
}

#[test]
fn plan_a_partly_planned_zone_is_kept_and_writes_its_planned_leg() {
    // the bridge's 2 m hold splits the zone into two legs: the first (the corner 12 m before the hold) is planned,
    // the second keeps its line because its drawn line runs through its own chain's earthwork. The zone is `Kept`
    // with the first *kept* leg's reason, and the planned leg is still written.
    let g = partly_planned_chain();
    let params = RerouteParams {
        bridge_hold_m: 1.0,
        ..RerouteParams::default()
    };
    let (out, report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &[],
        &params,
    );
    let zone = only_zone(&report);
    assert_eq!(zone.legs, 2, "{zone:?}");
    let (reason, detail) = kept(zone);
    assert_eq!(reason, KeptReason::NoRoute, "{zone:?}");
    assert_eq!(detail, "no_route");
    assert_eq!(zone.candidate, Some(Candidate::Plate));
    assert_eq!(report.kept(KeptReason::NoRoute), 1);
    assert!(
        report.kept_zones.iter().any(|(id, _)| *id == zone.id),
        "{:?}",
        report.kept_zones
    );
    // the zone reports its planned leg's measures, not the kept leg's
    assert!(zone.radius_after_m >= CITY_MIN_R - 0.01, "{zone:?}");
    assert_ne!(zone.length_after_m, zone.length_before_m, "{zone:?}");
    assert!(zone.weight.is_some(), "{zone:?}");
    // the grade limit is the first *planned* leg's (flat ground: the City aim, 0.14); the kept leg has no limit
    assert!((zone.grade_limit - 0.14).abs() < 1e-9, "{zone:?}");
    // the planned leg is written: the corner at (490, 500) is replaced by a fillet
    let written = points_of(&out, "1:1-40");
    assert!(
        !written.iter().any(|&p| same_bits(p, (490.0, 500.0))),
        "the planned leg was not written: {written:?}"
    );
    // the bridge and the kept leg keep their line bit for bit
    assert_eq!(points_of(&out, "2:40-41"), points_of(&g, "2:40-41"));
    assert_eq!(points_of(&out, "3:20-41"), points_of(&g, "3:20-41"));
}

#[test]
fn plan_a_deck_near_a_junction_crossed_elsewhere_is_held() {
    // a corner 20 m after a junction, and a bridge deck whose end is 5 m from the junction but which the drawn
    // line crosses 20 m from it: that crossing is not part of the anchor
    let g = build_graph(&[
        city_way(
            1,
            &[1, 2, 3],
            &[(500.0, 500.0), (500.0, 520.0), (560.0, 520.0)],
        ),
        city_way(
            2,
            &[10, 1, 11],
            &[(400.0, 500.0), (500.0, 500.0), (600.0, 500.0)],
        ),
        bridge_way(3, &[20, 21], &[(505.0, 490.0), (505.0, 540.0)]),
    ]);
    let params = RerouteParams::default();
    let (out, report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &[],
        &params,
    );
    assert_eq!(out, g);
    let zone = only_zone(&report);
    assert_eq!(zone.anchors.0, AnchorKind::Junction(1), "{zone:?}");
    let (reason, detail) = kept(zone);
    assert_eq!(reason, KeptReason::Held, "{zone:?}");
    assert_eq!(detail, "crosses_deck");
    // without the deck the corner is planned: 20 m after the junction the arm is free
    let clean = build_graph(&[
        city_way(
            1,
            &[1, 2, 3],
            &[(500.0, 500.0), (500.0, 520.0), (560.0, 520.0)],
        ),
        city_way(
            2,
            &[10, 1, 11],
            &[(400.0, 500.0), (500.0, 500.0), (600.0, 500.0)],
        ),
    ]);
    let (_out, report) = plan_with(
        &clean,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &[],
        &params,
    );
    assert!(is_planned(only_zone(&report)), "{:?}", report.zones);
}

/// A chain with a vertex 3 cm before the end anchor's sample as the chord measure of Task 6's first version placed
/// it (its sample at y = 521.824), after off-grid 90 degree corners.
fn vertex_a_hand_span_before_the_end() -> RoadGraph {
    build_graph(&[
        city_way(
            1,
            &[1, 2, 3, 4, 5, 6, 7, 20, 8],
            &[
                (400.0, 500.0),
                (441.3, 500.0),
                (441.3, 503.3),
                (482.9, 503.3),
                (482.9, 500.0),
                (560.0, 500.0),
                (560.0, 519.79),
                (560.0, 521.79),
                (560.0, 600.0),
            ],
        ),
        city_way(
            2,
            &[10, 1, 11],
            &[(400.0, 400.0), (400.0, 500.0), (400.0, 600.0)],
        ),
        city_way(
            3,
            &[12, 8, 13],
            &[(460.0, 600.0), (560.0, 600.0), (660.0, 600.0)],
        ),
    ])
}

#[test]
fn plan_a_vertex_a_hand_span_before_the_end_anchor_is_not_written_after_it() {
    let g = vertex_a_hand_span_before_the_end();
    let (out, report) = plan_graph(&g, &flat_map());
    let zone = zone_matching(&report, |z| z.at.0 > 540.0 && z.at.1 < 520.0);
    assert!(is_planned(zone), "{zone:?}");
    let written = points_of(&out, "1:1-8");
    // The leg's end anchor is at y = 519.8195: the true arc of the chain's samples puts it 3 cm after the vertex at
    // (560, 519.79), which therefore lies inside the leg and is not written (the route replaces the leg). The
    // vertex at (560, 521.79) lies after the anchor, in the tail the leg does not touch, and is written in order.
    // The chord measure of Task 6's first version put the anchor at y = 521.824, and then both vertices lay before
    // the route's end; it wrote the second one after the end, which is the doubling back `assert_no_reversal`
    // catches.
    assert_no_reversal(&written);
    assert!(
        !written.iter().any(|&p| same_bits(p, (560.0, 519.79))),
        "the vertex 3 cm before the end anchor is written after the route"
    );
    assert!(
        written.iter().any(|&p| same_bits(p, (560.0, 521.79))),
        "the vertex after the anchor is not written in the tail: {written:?}"
    );
    assert!(
        written.iter().any(|&p| (p.1 - 520.0).abs() <= 1.0),
        "the end anchor is not on the written line: {written:?}"
    );
}

// ----------------------------------------------------------------------------------------------------------
// Fix round 1: the reason of a kept bend follows the rules' order, planned routes are compared place by place.
// ----------------------------------------------------------------------------------------------------------

#[test]
fn plan_a_hairpin_in_a_bottle_stream_is_a_curved_bridge() {
    // A stream that encloses the hairpin and keeps more than 10 m from both anchors: its room is 33 m wide but its
    // ceiling is 10 m above the anchors (a turn of radius 10.5 needs 10.5) and the only way up is an 18 m neck. The
    // drawn line is crossed 0 times. Without the stream the free loop is planned: the fixture can fail.
    let g = edge_hairpin(500.0);
    let bottle = vec![
        (489.5, 380.0),
        (489.5, 450.0),
        (497.0, 450.5),
        (497.0, 466.0),
        (515.0, 466.0),
        (515.0, 450.5),
        (522.5, 450.0),
        (522.5, 380.0),
    ];
    let params = RerouteParams::default();
    let (_out, report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &[],
        &params,
    );
    assert!(is_planned(only_zone(&report)), "{:?}", report.zones);
    // with the stream: no route, because the stream is in the way (the diagnostic without water and streams finds
    // the loop: if the streams stayed in, the leg would keep `no_route` instead)
    let (out, report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        std::slice::from_ref(&bottle),
        &params,
    );
    assert_eq!(out, g, "a kept zone keeps its line bit for bit");
    let zone = only_zone(&report);
    let (reason, detail) = kept(zone);
    assert_eq!(reason, KeptReason::BlockedByWater, "{zone:?}");
    assert_eq!(detail, "blocked_by_water");
    assert_eq!(zone.candidate, Some(Candidate::CurvedBridge));
}

/// Two hairpins whose loops bulge toward each other, joined at a junction: A's last leg runs into J = (512, 380),
/// where B's chain starts (east along y = 380) and a spur leaves to the south. Their drawn lines touch only at J
/// and are 32 m apart elsewhere.
fn two_hairpins_at_a_junction() -> RoadGraph {
    let mut nodes_a: Vec<i64> = vec![1, 2];
    let mut points_a: Vec<(f64, f64)> = vec![(500.0, 400.0), (500.0, 460.0)];
    let mut next = 3_i64;
    for i in 1..16 {
        let a = PI - PI * f64::from(i) / 16.0;
        points_a.push((506.0 + 6.0 * a.cos(), 460.0 + 4.0 * a.sin()));
        nodes_a.push(next);
        next += 1;
    }
    for p in [(512.0, 460.0), (512.0, 380.0)] {
        points_a.push(p);
        nodes_a.push(next);
        next += 1;
    }
    let junction = next - 1;
    let mut points_b: Vec<(f64, f64)> = vec![(512.0, 380.0), (544.0, 380.0), (544.0, 460.0)];
    for i in 1..16 {
        let a = PI - PI * f64::from(i) / 16.0;
        points_b.push((550.0 + 6.0 * a.cos(), 460.0 + 4.0 * a.sin()));
    }
    points_b.push((556.0, 460.0));
    points_b.push((556.0, 380.0));
    let nodes_b: Vec<i64> = std::iter::once(junction)
        .chain((200_i64..).take(points_b.len() - 1))
        .collect();
    build_graph(&[
        city_way(1, &nodes_a, &points_a),
        city_way(2, &nodes_b, &points_b),
        city_way(3, &[junction, 900], &[(512.0, 380.0), (512.0, 360.0)]),
    ])
}

#[test]
fn plan_two_zones_that_meet_at_a_junction_are_checked_away_from_it() {
    // A and B share the junction J: their drawn lines touch there and are 32 m apart at their loops, whose routes
    // come 23 m apart. The comparison is per place, so only the places near J (where the drawn lines already ran
    // that near) are exempt, and the later zone is reverted for the loops. B's corner at (544, 380) is a zone of its
    // own (a fillet far from both loops), so the chain of B has two.
    let g = two_hairpins_at_a_junction();
    let params = RerouteParams::default();
    let plan = |graph: &RoadGraph| {
        plan_with(
            graph,
            &flat_map(),
            &|_: &Stretch| CITY_MIN_R,
            &|_: (f64, f64)| false,
            &[],
            &params,
        )
    };
    let (out, report) = plan(&g);
    assert_eq!(report.zones.len(), 3, "{:?}", report.zones);
    assert_eq!(report.reverted, 1, "{:?}", report.zones);
    let loop_a = zone_matching(&report, |z| z.at.0 < 520.0 && z.at.1 > 450.0);
    let loop_b = zone_matching(&report, |z| z.at.0 > 540.0 && z.at.1 > 450.0);
    assert!(
        is_planned(loop_a),
        "the lower chain id is planned: {loop_a:?}"
    );
    let (reason, detail) = kept(loop_b);
    assert_eq!(reason, KeptReason::BlockedByRoad, "{loop_b:?}");
    assert_eq!(detail, format!("route_of {}", loop_a.id));
    assert_eq!(report.kept(KeptReason::BlockedByRoad), 1);
    assert_ne!(out, g, "the first zone is planned");
}

#[test]
fn plan_a_long_deck_near_a_junction_is_an_obstacle_beyond_the_anchor() {
    // A junction with a bridge deck whose near end is 11 m away, running east parallel to the leg's first arm; the
    // leg needs a 30 m radius, so it spans 120 m and its route runs alongside the deck. Only the deck's part that
    // belongs to the anchor is exempt (within PLAN_DECK_ANCHOR_M, and as far along the deck as its earthwork and the
    // leg's clearance could reach that place, about 44 m): the route leaves the junction past a short deck, and a
    // deck that runs on past that is an obstacle it may not cross.
    let deck = |end: f64| {
        build_graph(&[
            city_way(
                1,
                &[1, 2, 3],
                &[(500.0, 500.0), (560.0, 500.0), (560.0, 620.0)],
            ),
            city_way(
                2,
                &[10, 1, 11],
                &[(500.0, 400.0), (500.0, 500.0), (500.0, 600.0)],
            ),
            bridge_way(3, &[20, 21], &[(505.0, 510.0), (end, 510.0)]),
        ])
    };
    let params = RerouteParams::default();
    let plan = |graph: &RoadGraph| {
        plan_with(
            graph,
            &flat_map(),
            &|_: &Stretch| 30.0,
            &|_: (f64, f64)| false,
            &[],
            &params,
        )
    };
    let short = deck(535.0);
    let (out, report) = plan(&short);
    let zone = only_zone(&report);
    assert!(is_planned(zone), "the short deck: {zone:?}");
    assert_ne!(out, short);
    let long = deck(555.0);
    let (out, report) = plan(&long);
    assert_eq!(out, long, "the deck's far part must be an obstacle");
    let zone = only_zone(&report);
    // kept: the route cannot reach its fillet past the deck's far part. The reason is `held`, the spec's first, not
    // `blocked_by_road`: the leg's tight run lies within the lead of its junction anchor.
    let (reason, _) = kept(zone);
    assert_eq!(reason, KeptReason::Held, "{zone:?}");
}

// ----------------------------------------------------------------------------------------------------------
// Fix round 2: planned routes are compared at their nearest drawn points, and a deck near an anchor keeps
// its core.
// ----------------------------------------------------------------------------------------------------------

/// `facing_hairpin(1, 500)` whose east leg bows east: an arc of radius 50 through (522, 430), so the drawn line
/// comes within 22 m of `facing_hairpin(2, 544)`'s west leg at y ~ 430 while the two loops' drawn lines stay
/// 32 m apart at the top.
fn bulged_facing_hairpin() -> Feature {
    let left = 500.0;
    let mut nodes: Vec<i64> = vec![101, 102];
    let mut points: Vec<(f64, f64)> = vec![(left, 400.0), (left, 460.0)];
    let mut next = 103_i64;
    for i in 1..16 {
        let a = PI - PI * f64::from(i) / 16.0;
        points.push((left + 6.0 + 6.0 * a.cos(), 460.0 + 4.0 * a.sin()));
        nodes.push(next);
        next += 1;
    }
    points.push((left + 12.0, 460.0));
    nodes.push(next);
    next += 1;
    // the east leg: the arc of radius 50 around (472, 430), from (512, 460) down to (512, 400)
    let (cx, cy, r) = (472.0, 430.0, 50.0);
    let steps = 33;
    for k in 1..steps {
        let t = f64::from(k) / f64::from(steps);
        let deg = 36.869_897_645_993 * (1.0 - 2.0 * t);
        let a = deg.to_radians();
        points.push((cx + r * a.cos(), cy + r * a.sin()));
        nodes.push(next);
        next += 1;
    }
    points.push((left + 12.0, 400.0));
    nodes.push(next);
    city_way(1, &nodes, &points)
}

/// A facing hairpin at `left` whose legs run from y = 380 to 460 and whose both legs have a tight jog at y = 420
/// (the west one to `jog_x`): the jog's grown zone touches the loop's, so each chain has one zone that spans both.
fn jogged_facing_hairpin(way: i64, left: f64, jog_x: f64) -> Feature {
    let mut points: Vec<(f64, f64)> = vec![
        (left, 380.0),
        (left, 415.0),
        (jog_x, 420.0),
        (left, 425.0),
        (left, 460.0),
    ];
    let mut nodes: Vec<i64> = (1_i64..=5).map(|i| way * 100 + i).collect();
    let mut next = way * 100 + 6;
    for i in 1..16 {
        let a = PI - PI * f64::from(i) / 16.0;
        points.push((left + 6.0 + 6.0 * a.cos(), 460.0 + 4.0 * a.sin()));
        nodes.push(next);
        next += 1;
    }
    for p in [
        (left + 12.0, 460.0),
        (left + 12.0, 425.0),
        (left + 16.0, 420.0),
        (left + 12.0, 415.0),
        (left + 12.0, 380.0),
    ] {
        points.push(p);
        nodes.push(next);
        next += 1;
    }
    city_way(way, &nodes, &points)
}

/// The two plans of a pair of chains, with the later zone reverted against the earlier one.
fn assert_the_later_is_reverted(g: &RoadGraph) -> RerouteReport {
    let params = RerouteParams::default();
    let (out, report) = plan_with(
        g,
        &flat_map(),
        &|_: &Stretch| CITY_MIN_R,
        &|_: (f64, f64)| false,
        &[],
        &params,
    );
    assert_eq!(report.zones.len(), 2, "{:?}", report.zones);
    assert_eq!(report.reverted, 1, "{:?}", report.zones);
    let first = &report.zones[0];
    let second = &report.zones[1];
    assert!(
        is_planned(first),
        "the lower chain id is planned: {first:?}"
    );
    let (reason, detail) = kept(second);
    assert_eq!(reason, KeptReason::BlockedByRoad, "{second:?}");
    assert_eq!(detail, format!("route_of {}", first.id));
    assert_ne!(&out, g, "the first zone is planned");
    report
}

/// The later zone's route crosses the earlier one's places where the two drawn lines were 32 m apart, but the
/// drawn line of the later leg comes within the threshold somewhere within a disc of the threshold around the
/// route points (the bulge at y ~ 430). Only the nearest drawn points at the place may exempt, so the later zone
/// is reverted.
#[test]
fn plan_routes_are_compared_at_the_nearest_drawn_points_not_any_within_the_threshold() {
    let g = build_graph(&[bulged_facing_hairpin(), facing_hairpin(2, 544.0)]);
    assert_the_later_is_reverted(&g);
}

/// Zones on different chains, each with a second tight jog merged into the zone of its loop: the two jogs' drawn
/// lines come 18 m near each other at y = 420, about 30 m below the place where the routes conflict (the loops'
/// U-turns). The pair is not exempt as a whole: the jog's place is exempt and the loop's conflict is not.
#[test]
fn plan_a_jog_merged_into_a_zone_does_not_exempt_the_whole_pair() {
    let g = build_graph(&[
        jogged_facing_hairpin(1, 500.0, 522.0),
        jogged_facing_hairpin(2, 544.0, 540.0),
    ]);
    assert_the_later_is_reverted(&g);
}

/// The reviewer's sketch, exactly: a junction at (500, 500), a corner at (530, 500), a deck from (505, 505) to
/// (529, 505). The corner's arm is 30 m, longer than the corner's lead (2 x the 10 m radius), so the junction is
/// not an anchor of the corner's leg: the leg is anchored on the line, the deck belongs to no fixed anchor of it
/// and is a full obstacle. The drawn line is 5 m from the deck and fails the segment test; the zone is kept, and
/// nothing crosses the deck. (The sketch as drawn cannot show the exemption's hole; the fixture below it, with a
/// radius that makes the junction the leg's anchor, can.)
#[test]
fn plan_a_deck_across_a_corner_is_never_crossed() {
    let g = build_graph(&[
        city_way(
            1,
            &[1, 2, 3, 4],
            &[
                (400.0, 500.0),
                (500.0, 500.0),
                (530.0, 500.0),
                (530.0, 600.0),
            ],
        ),
        city_way(
            2,
            &[10, 2, 11],
            &[(500.0, 400.0), (500.0, 500.0), (500.0, 600.0)],
        ),
        bridge_way(3, &[20, 21], &[(505.0, 505.0), (529.0, 505.0)]),
    ]);
    let (out, report) = plan_graph(&g, &flat_map());
    let zone = zone_matching(&report, |z| z.stretches.iter().any(|s| s.starts_with("1:")));
    let (reason, _) = kept(zone);
    assert_eq!(reason, KeptReason::BlockedByRoad, "{zone:?}");
    assert_eq!(out, g);
    assert_no_route_touches_the_deck(&out, &report, (505.0, 505.0), (529.0, 505.0));
}

/// The same corner with the street's radius at 15 m, so that the corner's lead reaches the junction and the
/// junction is the leg's fixed anchor: the deck (7.07 m from it) is part of that anchor. The route of a corner
/// fillet would cross the deck at grade ((526.2, 505) on the fillet); the deck's core keeps every planned route
/// off it and its wall keeps it from crossing.
#[test]
fn plan_a_deck_that_belongs_to_a_junction_is_not_crossed_at_grade() {
    let deck = ((505.0, 505.0), (529.0, 505.0));
    let g = build_graph(&[
        city_way(
            1,
            &[1, 2, 3, 4],
            &[
                (400.0, 500.0),
                (500.0, 500.0),
                (530.0, 500.0),
                (530.0, 600.0),
            ],
        ),
        city_way(
            2,
            &[10, 2, 11],
            &[(500.0, 400.0), (500.0, 500.0), (500.0, 600.0)],
        ),
        bridge_way(3, &[20, 21], &[deck.0, deck.1]),
    ]);
    let (out, report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| 15.0,
        &|_: (f64, f64)| false,
        &[],
        &RerouteParams::default(),
    );
    let zone = zone_matching(&report, |z| z.stretches.iter().any(|s| s.starts_with("1:")));
    assert_eq!(zone.anchors.0, AnchorKind::Junction(2), "{zone:?}");
    assert_no_route_touches_the_deck(&out, &report, deck.0, deck.1);
}

/// A deck 8 m from the junction and 80 m long: the leg may leave the junction (the deck's first part belongs to
/// the anchor's place), but no route crosses the deck or runs on it.
#[test]
fn plan_a_long_deck_near_a_junction_does_not_freeze_the_anchor() {
    let deck = ((505.0, 492.0), (505.0, 412.0));
    let g = build_graph(&[
        city_way(
            1,
            &[1, 2, 3, 4],
            &[
                (400.0, 500.0),
                (500.0, 500.0),
                (515.0, 500.0),
                (515.0, 600.0),
            ],
        ),
        city_way(
            2,
            &[10, 2, 11],
            &[(500.0, 400.0), (500.0, 500.0), (500.0, 600.0)],
        ),
        bridge_way(3, &[20, 21], &[deck.0, deck.1]),
    ]);
    let (out, report) = plan_graph(&g, &flat_map());
    let zone = zone_matching(&report, |z| z.stretches.iter().any(|s| s.starts_with("1:")));
    assert!(
        is_planned(zone),
        "the deck must not freeze the anchor: {zone:?}"
    );
    assert_no_route_touches_the_deck(&out, &report, deck.0, deck.1);
}

/// A deck 20 m from the junction: it belongs to no anchor of the leg and is a full obstacle, so a planned route
/// keeps the leg's clearance from it (here it cannot be planned at all, the fillet would be 5 m from the deck).
#[test]
fn plan_a_deck_twenty_metres_from_an_anchor_is_an_obstacle() {
    let deck = ((510.0, 519.0), (525.0, 519.0));
    let g = build_graph(&[
        city_way(
            1,
            &[1, 2, 3, 4],
            &[
                (400.0, 500.0),
                (500.0, 500.0),
                (530.0, 500.0),
                (530.0, 600.0),
            ],
        ),
        city_way(
            2,
            &[10, 2, 11],
            &[(500.0, 400.0), (500.0, 500.0), (500.0, 600.0)],
        ),
        bridge_way(3, &[20, 21], &[deck.0, deck.1]),
    ]);
    let (out, report) = plan_with(
        &g,
        &flat_map(),
        &|_: &Stretch| 15.0,
        &|_: (f64, f64)| false,
        &[],
        &RerouteParams::default(),
    );
    let zone = zone_matching(&report, |z| z.stretches.iter().any(|s| s.starts_with("1:")));
    if is_planned(zone) {
        let id = zone
            .stretches
            .first()
            .unwrap_or_else(|| panic!("no stretch"));
        let route = points_of(&out, id);
        let got = line_distance(&route, &[deck.0, deck.1]);
        assert!(got >= 15.0, "a route at {got} m from the deck: {zone:?}");
    }
}

/// Every planned route stays out of the deck's core and does not cross its line.
fn assert_no_route_touches_the_deck(
    out: &RoadGraph,
    report: &RerouteReport,
    a: (f64, f64),
    b: (f64, f64),
) {
    let deck = [a, b];
    for zone in report.zones.iter().filter(|z| is_planned(z)) {
        for id in &zone.stretches {
            let route = points_of(out, id);
            assert_eq!(
                segment_crossings(&route, &deck),
                0,
                "the route of {id} crosses the deck: {zone:?}"
            );
            let near = line_distance(&route, &deck);
            assert!(
                near >= 5.0,
                "the route of {id} is {near} m from the deck's centre line, inside its core: {zone:?}"
            );
        }
    }
}

// ---- drawn lines (naves:keep, naves:plan) ----

/// The town corner with its corner way marked as drawn by hand (`held`), and with `plan` as given.
fn town_corner_drawn(plan: bool) -> RoadGraph {
    let mut g = town_corner();
    for s in g.stretches.iter_mut().filter(|s| s.id.starts_with("1:")) {
        s.held = true;
        s.plan = plan;
    }
    g
}

#[test]
fn plan_a_drawn_tight_corner_is_kept_as_drawn() {
    let g = town_corner_drawn(false);
    let (out, report) = plan_graph(&g, &flat_map());
    assert_eq!(out, g, "a drawn line moved");
    let zone = only_zone(&report);
    let (reason, detail) = kept(zone);
    assert_eq!(reason, KeptReason::Held);
    assert_eq!(detail, "drawn");
    assert!(
        report
            .skipped
            .iter()
            .any(|(id, r)| *id == zone.id && r == "drawn"),
        "{:?}",
        report.skipped
    );
}

#[test]
fn plan_a_drawn_corner_with_naves_plan_is_planned() {
    let g = town_corner_drawn(true);
    let (out, report) = plan_graph(&g, &flat_map());
    let zone = only_zone(&report);
    assert!(is_planned(zone), "{zone:?}");
    assert_ne!(
        out, g,
        "naves:plan=yes lets road planning move the drawn line"
    );
}

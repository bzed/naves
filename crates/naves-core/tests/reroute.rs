// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tests of the road-planning obstacle raster, its distance maps and its wall lines.

use naves_core::catalog::Family;
use naves_core::heightmap::Heightmap;
use naves_core::reroute::band::{
    Band, BandFail, DrawnGrade, Reach, arrive, arrives, band_step, drawn_grade, ground_at,
    leg_limit, relaxed_limits,
};
use naves_core::reroute::raster::{
    Blocked, Clearance, Obstacles, Wall, WallKind, chamfer, crossings_away, exempt_samples, samples,
};
use naves_core::reroute::{CHAMFER_ERROR, RerouteParams};
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
    // The spike's start [z - 6, z + 6] is not expressible: Reach has no public fields and at_anchor takes its tolerance
    // from the band, which refuses an infinite or negative one.
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

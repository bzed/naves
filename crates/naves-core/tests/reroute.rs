// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tests of the road-planning obstacle raster, its distance maps and its wall lines.

use naves_core::reroute::raster::{
    Blocked, Clearance, Obstacles, Wall, WallKind, chamfer, crossings_away, exempt_samples,
};
use naves_core::reroute::{CHAMFER_ERROR, RerouteParams};

const NONE: Clearance = Clearance {
    water_m: 0.0,
    road_m: 0.0,
};

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
    let d = o.water_distance_m((130.0, 110.0));
    assert!((9.0..=11.0).contains(&d), "{d}");
}

#[test]
fn threshold_and_clearance() {
    let o = with_lake();
    let want = 7.0 * CHAMFER_ERROR + 2.0 * 2.0_f64.sqrt();
    assert!((o.threshold(5.0) - want).abs() < 1e-9);
    assert_eq!(o.threshold(0.0), 0.0);
    assert_eq!(o.threshold(-3.0), 0.0);
    let clear5 = Clearance {
        water_m: 5.0,
        road_m: 0.0,
    };
    assert_eq!(
        o.segment((130.0, 0.0), (130.0, 200.0), clear5),
        Err(Blocked::Water)
    );
    assert_eq!(o.segment((140.0, 0.0), (140.0, 200.0), clear5), Ok(()));
}

#[test]
fn one_threshold_for_test_and_water_cap() {
    let o = with_lake();
    let original = [(135.0, 0.0), (135.0, 200.0)];
    let cap = o.water_cap_m(&original);
    assert!(cap > 0.0 && cap < 20.0, "{cap}");
    let clear = Clearance {
        water_m: cap,
        road_m: 0.0,
    };
    assert_eq!(o.segment(original[0], original[1], clear), Ok(()));
    // a little more and the original line itself would fail
    let more = Clearance {
        water_m: cap + 0.5,
        road_m: 0.0,
    };
    assert_eq!(
        o.segment(original[0], original[1], more),
        Err(Blocked::Water)
    );
    // a line in the water has no clearance at all
    assert_eq!(o.water_cap_m(&[(110.0, 0.0), (110.0, 200.0)]), 0.0);
}

fn road_world() -> Obstacles {
    let mut o = world(0.0);
    o.stamp_road(&[(0.0, 50.0), (200.0, 50.0)], 6.0, &|_| false);
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
    let wide = Clearance {
        water_m: 0.0,
        road_m: 15.0,
    };
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
    o.stamp_road(&[(0.0, 50.0), (200.0, 50.0)], 6.0, &|p| p.0 < 100.0);
    o.finish();
    assert_eq!(o.segment((10.0, 52.0), (90.0, 52.0), NONE), Ok(()));
    assert_eq!(
        o.segment((110.0, 52.0), (190.0, 52.0), NONE),
        Err(Blocked::Road)
    );
}

#[test]
fn exempt_samples_grow_along_the_road() {
    // a road of 200 m with its middle 20 m 8 m from the original line
    let road = [(0.0, 0.0), (200.0, 0.0)];
    let original = [(90.0, 8.0), (110.0, 8.0)];
    let flags = exempt_samples(&road, &original, 8.0, 10.0);
    assert_eq!(flags.len(), 101);
    for (i, &f) in flags.iter().enumerate() {
        let x = 2.0 * f64::from(u32::try_from(i).unwrap());
        assert_eq!(f, (80.0..=120.0).contains(&x), "sample at {x}");
    }
    // no original line: nothing is exempt
    assert!(exempt_samples(&road, &[], 8.0, 10.0).iter().all(|&f| !f));
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
        Err(Blocked::Edge)
    );
    assert_eq!(
        o.segment((50.0, 50.0), (250.0, 50.0), NONE),
        Err(Blocked::Edge)
    );
    let wide = Clearance {
        water_m: 0.0,
        road_m: 15.0,
    };
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

#[test]
fn walls_block_and_count() {
    let line = vec![(100.0, 0.0), (100.0, 200.0)];
    let wall = Wall {
        line: line.clone(),
        allowed: 0,
        kind: WallKind::Stream,
    };
    assert!(wall.blocks((90.0, 50.0), (110.0, 50.0)));
    assert!(!wall.blocks((90.0, 50.0), (99.0, 150.0)));
    let allowed = Wall {
        line: line.clone(),
        allowed: 1,
        kind: WallKind::Road,
    };
    assert!(!allowed.blocks((90.0, 50.0), (110.0, 50.0)));
    assert!(!wall.blocks((f64::NAN, 50.0), (110.0, 50.0)));

    // the route crosses the stream at (100, 50) and at (100, 195)
    let anchors = [(90.0, 50.0), (90.0, 195.0)];
    let two = [(90.0, 50.0), (110.0, 50.0), (110.0, 195.0), (90.0, 195.0)];
    assert_eq!(crossings_away(&two, &line, anchors, 10.0), 0);
    assert_eq!(crossings_away(&two, &line, anchors, 0.0), 2);
}

#[test]
fn nan_ends_are_an_edge() {
    let o = with_lake();
    assert_eq!(
        o.segment((f64::NAN, 50.0), (60.0, 50.0), NONE),
        Err(Blocked::Edge)
    );
    assert_eq!(
        o.segment((10.0, 50.0), (60.0, f64::INFINITY), NONE),
        Err(Blocked::Edge)
    );
}

#[test]
fn stray_from_the_original_line() {
    let mut o = world(0.0);
    o.set_original(&[(0.0, 100.0), (200.0, 100.0)]);
    o.finish();
    let d = o.stray_m((50.0, 107.0));
    assert!((7.0..=7.0 * CHAMFER_ERROR + 2.83).contains(&d), "{d}");
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
    o.stamp_road(&[(0.0, 0.0)], 1.0, &|_| false);
    o.stamp_road(&[], 1.0, &|_| false);
    o.set_original(&[]);
    o.finish();
    assert!(o.cells() >= 1);
    let _ = o.segment((0.0, 0.0), (0.5, 0.5), NONE);
    let _ = o.water_cap_m(&[]);
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

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Lot groups, exclusions and the smooth surface of a lot.

#![expect(
    clippy::indexing_slicing,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::catalog::Family;
use naves_core::heightmap::Heightmap;
use naves_core::limits::min_vertical_radius;
use naves_core::lot::{
    Assessed, Form, LOT_MIN_RADIUS_M, LotArea, LotKind, LotSurface, Outcome, Placement, assess,
    contains, local_relief,
};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed};

const CELL: f64 = 4.0;
const SIZE: u32 = 64; // vertices 0..=63, 252 m

fn index(v: u32) -> f64 {
    f64::from(v)
}

fn heightmap(f: impl Fn(f64, f64) -> f64) -> Heightmap {
    let mut h = Vec::new();
    for row in 0..SIZE {
        for col in 0..SIZE {
            #[expect(clippy::cast_possible_truncation, reason = "test heights are small")]
            h.push(f(index(col) * CELL, index(row) * CELL) as f32);
        }
    }
    Heightmap {
        size: SIZE,
        cell_m: 4.0,
        h,
    }
}

#[expect(clippy::cast_precision_loss, reason = "small indices")]
fn pos(col: i64, row: i64) -> (f64, f64) {
    (col as f64 * CELL, row as f64 * CELL)
}

fn closed(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut ring = points.to_vec();
    ring.push(points[0]);
    ring
}

fn rect_ring(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<(f64, f64)> {
    closed(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
}

fn lot_from(id: &str, outer: Vec<(f64, f64)>, holes: Vec<Vec<(f64, f64)>>, area: f64) -> LotArea {
    LotArea {
        id: id.to_string(),
        kind: LotKind::CarPark,
        placement: Placement::Ground,
        outer,
        holes,
        area_m2: area,
        surface_tag: None,
        holes_dropped: 0,
    }
}

fn rect(id: &str, x0: f64, y0: f64, w: f64, h: f64) -> LotArea {
    lot_from(id, rect_ring(x0, y0, x0 + w, y0 + h), Vec::new(), w * h)
}

fn dry(_: (f64, f64)) -> bool {
    false
}

fn one(area: LotArea, base: &Heightmap) -> (Assessed, Vec<LotSurface>) {
    let (assessed, surfaces) = assess(&[area], base, &dry);
    assert_eq!(assessed.len(), 1);
    (assessed[0].clone(), surfaces)
}

/// The terrain vertices inside a lot, as (col, row).
fn vertices_in(area: &LotArea) -> Vec<(u32, u32)> {
    let mut found = Vec::new();
    for row in 0..SIZE {
        for col in 0..SIZE {
            if contains(area, (index(col) * CELL, index(row) * CELL)) {
                found.push((col, row));
            }
        }
    }
    found
}

/// A fixed pseudo-random number in -1..1 per vertex.
fn noise(seed: u64, a: u64, b: u64) -> f64 {
    let mut x = seed
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(a.wrapping_mul(0xBF58_476D_1CE4_E5B9))
        .wrapping_add(b.wrapping_mul(0x94D0_49BB_1331_11EB));
    x ^= x >> 31;
    x = x.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    x ^= x >> 29;
    #[expect(clippy::cast_precision_loss, reason = "a noise value")]
    let unit = (x >> 11) as f64 / (1_u64 << 53) as f64;
    unit * 2.0 - 1.0
}

// ----- form -----

#[test]
fn flat_ground_gives_a_plane_without_moves() {
    let base = heightmap(|_, _| 100.0);
    let (a, surfaces) = one(rect("osm/way/1", 100.5, 100.5, 40.0, 40.0), &base);
    assert_eq!(a.outcome, Outcome::Smoothed { form: Form::Plane });
    assert_eq!(surfaces.len(), 1);
    assert_eq!(a.group, surfaces[0].group);
    assert!((surfaces[0].surface.height_at((120.0, 120.0)) - 100.0).abs() < 1e-6);
    assert!(a.move_max_m.abs() < 1e-9 && a.move_p95_m.abs() < 1e-9);
    assert!(a.relief_after_m.abs() < 1e-9);
    assert!((surfaces[0].surface.lambda - 1.0).abs() < 1e-12);
    assert!(!a.steep);
}

#[test]
fn a_rising_ground_with_bumps_gives_its_plane() {
    let base = heightmap(|x, y| {
        let (c, r) = (x / CELL, y / CELL);
        #[expect(
            clippy::cast_sign_loss,
            clippy::cast_possible_truncation,
            reason = "test"
        )]
        let n = noise(7, c as u64, r as u64) * 0.3;
        100.0 + 0.05 * x + n
    });
    let (a, surfaces) = one(rect("osm/way/1", 100.5, 100.5, 40.0, 40.0), &base);
    assert_eq!(a.outcome, Outcome::Smoothed { form: Form::Plane });
    let (gx, gy) = surfaces[0].surface.gradient_at((120.0, 120.0));
    assert!((gx - 0.05).abs() < 0.01 && gy.abs() < 0.01, "{gx} {gy}");
    assert!(a.relief_after_m < 1e-6);
    assert!(a.move_max_m < 0.35, "{}", a.move_max_m);
    assert!(a.move_max_m > 0.05, "the bumps do move: {}", a.move_max_m);
}

#[test]
fn a_curved_ground_gets_a_capped_quadratic() {
    let base = heightmap(|x, _| 100.0 + (x - 30.0) * (x - 30.0) / 600.0);
    let area = rect("osm/way/1", 50.5, 60.5, 59.0, 40.0);
    let (a, surfaces) = one(area.clone(), &base);
    assert_eq!(
        a.outcome,
        Outcome::Smoothed {
            form: Form::Quadratic
        }
    );
    let s = &surfaces[0].surface;
    assert!((s.coeffs[3] - 1.0 / 1200.0).abs() < 1e-9, "{:?}", s.coeffs);
    // along every row of vertices: second differences small, height monotonic
    let mut rows = 0;
    for row in 0..SIZE {
        let heights: Vec<f64> = vertices_in(&area)
            .into_iter()
            .filter(|&(_, r)| r == row)
            .map(|(c, r)| s.height_at((index(c) * CELL, index(r) * CELL)))
            .collect();
        if heights.len() < 3 {
            continue;
        }
        rows += 1;
        for w in heights.windows(3) {
            assert!((w[0] - 2.0 * w[1] + w[2]).abs() <= 16.0 / 600.0 + 1e-9);
        }
        let up = heights.windows(2).all(|w| w[1] >= w[0] - 1e-12);
        let down = heights.windows(2).all(|w| w[1] <= w[0] + 1e-12);
        assert!(up || down, "row {row} is not monotonic");
    }
    assert!(rows > 5);
}

/// Whether a vertex of the lot, with all eight neighbours in it, is strictly higher or lower than all of them.
fn has_interior_extremum(area: &LotArea, height: &dyn Fn((f64, f64)) -> f64) -> bool {
    let inside = |c: i64, r: i64| {
        (0..i64::from(SIZE)).contains(&c)
            && (0..i64::from(SIZE)).contains(&r)
            && contains(area, pos(c, r))
    };
    for (c, r) in vertices_in(area) {
        let (c, r) = (i64::from(c), i64::from(r));
        let neighbours: Vec<(i64, i64)> = (-1..=1)
            .flat_map(|dr| (-1..=1).map(move |dc| (dc, dr)))
            .filter(|&d| d != (0, 0))
            .map(|(dc, dr)| (c + dc, r + dr))
            .collect();
        if !neighbours.iter().all(|&(nc, nr)| inside(nc, nr)) {
            continue;
        }
        let here = height(pos(c, r));
        let diffs: Vec<f64> = neighbours
            .iter()
            .map(|&(nc, nr)| height(pos(nc, nr)) - here)
            .collect();
        if diffs.iter().all(|d| *d > 1e-9) || diffs.iter().all(|d| *d < -1e-9) {
            return true;
        }
    }
    false
}

#[test]
fn a_bowl_gets_a_plane_and_no_extremum() {
    let base = heightmap(|x, y| 100.0 + ((x - 128.0).powi(2) + (y - 128.0).powi(2)) / 400.0);
    let area = rect("osm/way/1", 98.5, 98.5, 60.0, 60.0);
    let (a, surfaces) = one(area.clone(), &base);
    assert_eq!(a.outcome, Outcome::Smoothed { form: Form::Plane });
    let s = &surfaces[0].surface;
    assert!(!has_interior_extremum(&area, &|p| s.height_at(p)));
}

#[test]
fn a_ridge_limits_the_move_by_scaling() {
    let base = heightmap(|x, _| 100.0 + 8.0 * (1.0 - (x - 70.0).abs() / 10.0).max(0.0));
    let area = rect("osm/way/1", 20.5, 20.5, 100.0, 100.0);
    let (a, surfaces) = one(area.clone(), &base);
    let s = &surfaces[0].surface;
    let largest = vertices_in(&area)
        .into_iter()
        .map(|(c, r)| {
            let p = (index(c) * CELL, index(r) * CELL);
            (s.height_at(p) - f64::from(base.at(c, r))).abs()
        })
        .fold(0.0, f64::max);
    assert!(largest > 3.0, "{largest}");
    assert!((s.lambda - 3.0 / largest).abs() < 1e-9, "{}", s.lambda);
    assert!((a.move_max_m - 3.0).abs() < 1e-6, "{}", a.move_max_m);
    assert!(s.lambda < 1.0);
    // the result's relief is at most (1 - lambda) of what it was, plus 0.12
    assert!(a.relief_after_m <= (1.0 - s.lambda) * a.relief_before_m + 0.12 + 1e-9);
    // the range after is the scaled one
    assert!(a.range_after.1 - a.range_after.0 < a.range_before.1 - a.range_before.0);
}

#[test]
fn a_steep_plane_is_smoothed_and_reported_steep() {
    let steep = heightmap(|x, _| 100.0 + 0.25 * x);
    let (a, _) = one(rect("osm/way/1", 100.5, 100.5, 40.0, 40.0), &steep);
    assert_eq!(a.outcome, Outcome::Smoothed { form: Form::Plane });
    assert!(a.steep);
    assert!((a.slope_p95 - 0.25).abs() < 1e-6);
    let gentle = heightmap(|x, _| 100.0 + 0.15 * x);
    let (b, _) = one(rect("osm/way/1", 100.5, 100.5, 40.0, 40.0), &gentle);
    assert_eq!(b.outcome, Outcome::Smoothed { form: Form::Plane });
    assert!(!b.steep);
}

#[test]
fn a_single_bump_on_a_narrow_lot_is_removed() {
    let base = heightmap(|x, y| {
        if (x - 108.0).abs() < 1e-9 && (y - 120.0).abs() < 1e-9 {
            103.0
        } else {
            100.0
        }
    });
    let (a, surfaces) = one(rect("osm/way/1", 100.5, 100.5, 10.0, 40.0), &base);
    assert_eq!(a.outcome, Outcome::Smoothed { form: Form::Plane });
    assert!(a.move_max_m > 2.4 && a.move_max_m < 3.0, "{}", a.move_max_m);
    assert!((surfaces[0].surface.lambda - 1.0).abs() < 1e-12);
}

#[test]
fn a_hole_does_not_pull_the_fit() {
    let hole = rect_ring(110.5, 110.5, 130.5, 130.5);
    let base = heightmap(|x, y| {
        if (110.5..130.5).contains(&x) && (110.5..130.5).contains(&y) {
            105.0
        } else {
            100.0
        }
    });
    let area = lot_from(
        "osm/way/1",
        rect_ring(100.5, 100.5, 140.5, 140.5),
        vec![hole],
        1200.0,
    );
    let (a, surfaces) = one(area, &base);
    assert_eq!(a.outcome, Outcome::Smoothed { form: Form::Plane });
    assert!((surfaces[0].surface.height_at((120.0, 120.0)) - 100.0).abs() < 1e-6);
}

// ----- outcomes -----

#[test]
fn small_street_side_water_and_underground_lots() {
    let base = heightmap(|_, _| 100.0);
    let (tiny, surfaces) = one(rect("osm/way/1", 101.5, 101.5, 6.0, 6.0), &base);
    assert_eq!(
        tiny.outcome,
        Outcome::SurfaceOnly {
            reason: "no_vertex"
        }
    );
    assert_eq!(surfaces.len(), 0);
    assert!(tiny.move_max_m.is_nan() && tiny.slope_p95.is_nan());

    let mut street = rect("osm/way/2", 100.5, 100.5, 40.0, 40.0);
    street.placement = Placement::StreetSide;
    let (a, surfaces) = one(street, &base);
    assert_eq!(
        a.outcome,
        Outcome::SurfaceOnly {
            reason: "street_side"
        }
    );
    assert_eq!(surfaces.len(), 0);

    let mut under = rect("osm/way/3", 100.5, 100.5, 40.0, 40.0);
    under.placement = Placement::NotAtGround;
    let (a, surfaces) = one(under, &base);
    assert_eq!(
        a.outcome,
        Outcome::Excluded {
            reason: "not_at_ground"
        }
    );
    assert_eq!(surfaces.len(), 0);

    let wet = |p: (f64, f64)| p.0 < 124.0;
    let (assessed, surfaces) = assess(&[rect("osm/way/4", 100.5, 100.5, 40.0, 40.0)], &base, &wet);
    assert_eq!(
        assessed[0].outcome,
        Outcome::Excluded { reason: "in_water" }
    );
    assert!((assessed[0].water_share - 0.6).abs() < 0.05);
    assert_eq!(surfaces.len(), 0);
    let damp = |p: (f64, f64)| p.0 < 116.0;
    let (assessed, surfaces) = assess(&[rect("osm/way/4", 100.5, 100.5, 40.0, 40.0)], &base, &damp);
    assert!(matches!(assessed[0].outcome, Outcome::Smoothed { .. }));
    assert!((assessed[0].water_share - 0.4).abs() < 0.05);
    assert_eq!(surfaces.len(), 1);
}

#[test]
fn a_lot_off_the_map_is_surface_only_and_a_lot_over_the_edge_uses_its_map_part() {
    // an area of 0 (off the map) must not divide by zero, in water or not
    let base = heightmap(|x, _| 100.0 + 0.05 * x);
    let off = lot_from(
        "osm/way/9",
        rect_ring(400.5, 100.5, 440.5, 140.5),
        vec![],
        0.0,
    );
    let all_wet = |_: (f64, f64)| true;
    let (assessed, surfaces) = assess(&[off], &base, &all_wet);
    assert_eq!(
        assessed[0].outcome,
        Outcome::SurfaceOnly {
            reason: "no_vertex"
        }
    );
    assert_eq!(surfaces.len(), 0);
    assert!(
        assessed[0].water_share.abs() < 1e-12,
        "{}",
        assessed[0].water_share
    );

    // 200 to 300 m, the map ends at 252 m: ground clamped past the edge would flatten the fit
    let (a, surfaces) = one(rect("osm/way/1", 200.5, 100.5, 100.0, 40.0), &base);
    assert_eq!(a.outcome, Outcome::Smoothed { form: Form::Plane });
    let (gx, _) = surfaces[0].surface.gradient_at((220.0, 120.0));
    assert!((gx - 0.05).abs() < 1e-6, "{gx}");
}

#[test]
fn the_curvature_cap_is_the_gentlest_roads_radius() {
    assert_eq!(
        LOT_MIN_RADIUS_M.to_bits(),
        min_vertical_radius(Family::Asf1).to_bits()
    );
}

#[test]
fn a_broken_heightmap_extent_does_not_panic() {
    for cell in [-4.0_f32, f32::NAN] {
        let base = Heightmap {
            size: SIZE,
            cell_m: cell,
            h: vec![100.0; (SIZE * SIZE) as usize],
        };
        let wet = |p: (f64, f64)| p.0 < 120.0;
        let (assessed, surfaces) =
            assess(&[rect("osm/way/1", 100.5, 100.5, 40.0, 40.0)], &base, &wet);
        assert_eq!(assessed.len(), 1, "cell {cell}");
        assert_eq!(
            assessed[0].outcome,
            Outcome::SurfaceOnly {
                reason: "no_vertex"
            }
        );
        assert_eq!(surfaces.len(), 0);
    }
}

#[test]
fn a_nan_height_is_skipped_and_a_one_row_lot_is_a_plane() {
    let base = heightmap(|x, y| {
        if (x - 120.0).abs() < 1e-9 && (y - 120.0).abs() < 1e-9 {
            f64::NAN
        } else {
            100.0
        }
    });
    let (a, surfaces) = one(rect("osm/way/1", 100.5, 100.5, 40.0, 40.0), &base);
    assert_eq!(a.outcome, Outcome::Smoothed { form: Form::Plane });
    assert!((surfaces[0].surface.height_at((120.0, 120.0)) - 100.0).abs() < 1e-6);
    for v in [
        a.move_p95_m,
        a.move_max_m,
        a.slope_p95,
        a.relief_before_m,
        a.relief_after_m,
        a.range_before.0,
        a.range_after.1,
    ] {
        assert!(v.is_finite(), "{a:?}");
    }

    // a strip on one row of vertices (y = 120): the ridge keeps the missing direction flat
    let rising = heightmap(|x, _| 100.0 + 0.05 * x);
    let (a, surfaces) = one(rect("osm/way/2", 100.5, 119.0, 40.0, 2.0), &rising);
    assert_eq!(a.outcome, Outcome::Smoothed { form: Form::Plane });
    let (gx, gy) = surfaces[0].surface.gradient_at((120.0, 120.0));
    assert!((gx - 0.05).abs() < 1e-6 && gy.abs() < 1e-6, "{gx} {gy}");
    assert!(a.move_max_m.is_finite() && a.relief_after_m.is_finite());
}

#[test]
fn the_capped_curvature_keeps_its_direction() {
    // curvature 1/150 across a diagonal, the minimum line outside the lot: capped to 1/600 along the same diagonal
    let sqrt2 = std::f64::consts::SQRT_2;
    let down = heightmap(|x, y| {
        let u = (x - y) / sqrt2 - 60.0;
        100.0 + u * u / 300.0
    });
    let (a, surfaces) = one(rect("osm/way/1", 100.5, 100.5, 60.0, 60.0), &down);
    assert_eq!(
        a.outcome,
        Outcome::Smoothed {
            form: Form::Quadratic
        }
    );
    let c = surfaces[0].surface.coeffs;
    assert!((c[3] - 1.0 / 2400.0).abs() < 1e-9, "{c:?}");
    assert!((c[4] + 1.0 / 1200.0).abs() < 1e-9, "{c:?}");
    assert!((c[5] - 1.0 / 2400.0).abs() < 1e-9, "{c:?}");
    let up = heightmap(|x, y| {
        let u = (x + y) / sqrt2 - 100.0;
        100.0 + u * u / 300.0
    });
    let (_, surfaces) = one(rect("osm/way/1", 100.5, 100.5, 60.0, 60.0), &up);
    let c = surfaces[0].surface.coeffs;
    assert!((c[3] - 1.0 / 2400.0).abs() < 1e-9, "{c:?}");
    assert!((c[4] - 1.0 / 1200.0).abs() < 1e-9, "{c:?}");
    assert!((c[5] - 1.0 / 2400.0).abs() < 1e-9, "{c:?}");
}

#[test]
fn a_hollow_just_outside_is_a_plane_and_one_far_outside_is_not() {
    // a bowl of exactly the capped curvature, so the fitted stationary point is the bowl's minimum; the lot is a
    // right triangle and the bowl's bottom is off both of its axes, so a sign error in either half of the
    // stationary-point formula throws the point to the far side of the centroid, well outside the lot
    let bowl = |sx: f64, sy: f64| {
        heightmap(move |x, y| 100.0 + ((x - sx).powi(2) + (y - sy).powi(2)) / 1200.0)
    };
    let area = lot_from(
        "osm/way/1",
        closed(&[(50.5, 50.5), (150.5, 50.5), (50.5, 150.5)]),
        Vec::new(),
        5000.0,
    );
    // 4.5 m beyond the corner (150.5, 50.5): within two cells of the lot
    let (near, _) = one(area.clone(), &bowl(154.5, 48.5));
    assert_eq!(near.outcome, Outcome::Smoothed { form: Form::Plane });
    // 12.6 m beyond it: not
    let (far, _) = one(area, &bowl(162.5, 45.5));
    assert_eq!(
        far.outcome,
        Outcome::Smoothed {
            form: Form::Quadratic
        }
    );
}

#[test]
fn a_summit_in_a_large_hole_still_counts_as_inside_the_lot() {
    let base = heightmap(|x, y| 100.0 + ((x - 128.0).powi(2) + (y - 128.0).powi(2)) / 400.0);
    let area = lot_from(
        "osm/way/1",
        rect_ring(98.5, 98.5, 158.5, 158.5),
        vec![rect_ring(113.5, 113.5, 143.5, 143.5)],
        2700.0,
    );
    let (a, _) = one(area, &base);
    assert_eq!(a.outcome, Outcome::Smoothed { form: Form::Plane });
}

// ----- groups -----

#[test]
fn lots_that_touch_form_one_group_with_one_surface() {
    let base = heightmap(|_, y| 100.0 + 0.05 * y);
    let a = rect("osm/way/1", 100.5, 100.5, 20.0, 20.0);
    let b = rect("osm/way/2", 120.5, 100.5, 20.0, 20.0);
    let (assessed, surfaces) = assess(&[a, b], &base, &dry);
    assert_eq!(surfaces.len(), 1);
    assert_eq!(assessed[0].group, assessed[1].group);
    assert_eq!(surfaces[0].members, ["osm/way/1", "osm/way/2"]);
    let s = &surfaces[0];
    assert!((s.surface.height_at((120.5, 110.0)) - 105.5).abs() < 1e-6);
    assert!(s.contains((110.0, 110.0)) && s.contains((130.0, 110.0)));
    assert!(!s.contains((150.0, 110.0)));
    assert!((s.target((110.0, 110.0), 105.0) - 105.5).abs() < 1e-6);
}

#[test]
fn the_gap_decides_one_group_or_two() {
    let base = heightmap(|_, y| 100.0 + 0.05 * y);
    let a = rect("osm/way/1", 100.5, 100.5, 20.0, 20.0);
    let near = rect("osm/way/2", 121.0, 100.5, 20.0, 20.0);
    let (assessed, surfaces) = assess(&[a.clone(), near], &base, &dry);
    assert_eq!(surfaces.len(), 1);
    assert_eq!(assessed[0].group, assessed[1].group);
    let far = rect("osm/way/2", 122.5, 100.5, 20.0, 20.0);
    let (assessed, surfaces) = assess(&[a, far], &base, &dry);
    assert_eq!(surfaces.len(), 2);
    assert_ne!(assessed[0].group, assessed[1].group);
}

#[test]
fn a_car_park_in_a_camp_site_shares_its_surface() {
    let base = heightmap(|x, y| 100.0 + 0.02 * x + 0.03 * y);
    let mut camp = rect("osm/way/10", 100.5, 100.5, 60.0, 60.0);
    camp.kind = LotKind::CampSite;
    let park = rect("osm/way/20", 120.5, 120.5, 20.0, 20.0);
    let (assessed, surfaces) = assess(&[park, camp], &base, &dry);
    assert_eq!(surfaces.len(), 1);
    assert_eq!(assessed.len(), 2);
    let ids: Vec<&str> = assessed.iter().map(|a| a.area.id.as_str()).collect();
    assert_eq!(ids, ["osm/way/10", "osm/way/20"]);
    assert_eq!(assessed[0].group, assessed[1].group);
    assert_eq!(surfaces[0].members, ["osm/way/10", "osm/way/20"]);
    assert!(surfaces[0].contains((105.0, 105.0)));
}

#[test]
fn a_duplicate_does_not_count_twice_and_the_order_does_not_matter() {
    let base = heightmap(|x, y| {
        let (c, r) = (x / CELL, y / CELL);
        #[expect(
            clippy::cast_sign_loss,
            clippy::cast_possible_truncation,
            reason = "test"
        )]
        let n = noise(3, c as u64, r as u64) * 0.6;
        100.0 + 0.04 * x + 0.01 * y + n
    });
    let way = rect("osm/way/5", 100.5, 100.5, 40.0, 40.0);
    let (_, single) = assess(std::slice::from_ref(&way), &base, &dry);
    let mut relation = way.clone();
    relation.id = "osm/relation/5/0".to_string();
    let (assessed, twice) = assess(&[way.clone(), relation.clone()], &base, &dry);
    assert_eq!(twice.len(), 1);
    assert_eq!(assessed[0].group, assessed[1].group);
    assert_eq!(twice[0].surface, single[0].surface, "vertices counted once");

    let other = rect("osm/way/6", 150.5, 100.5, 30.0, 30.0);
    let forward = assess(&[way.clone(), other.clone(), relation.clone()], &base, &dry);
    let reversed = assess(&[relation, other, way], &base, &dry);
    assert_eq!(forward, reversed);
}

// ----- relief -----

#[test]
fn relief_of_a_plane_is_zero_and_asks_only_for_listed_vertices() {
    let vertices: Vec<(i64, i64)> = (0..8).flat_map(|r| (0..8).map(move |c| (c, r))).collect();
    let listed = vertices.clone();
    let plane = move |c: i64, r: i64| {
        assert!(listed.contains(&(c, r)), "asked for ({c}, {r})");
        #[expect(clippy::cast_precision_loss, reason = "small")]
        Some(10.0 + 0.1 * c as f64 + 0.2 * r as f64)
    };
    let (p95, max) = local_relief(&plane, &vertices, 4.0);
    assert!(p95.abs() < 1e-9 && max.abs() < 1e-9);

    let spike = |c: i64, r: i64| Some(if (c, r) == (4, 4) { 11.0 } else { 10.0 });
    let (_, max) = local_relief(&spike, &vertices, 4.0);
    assert!(max > 0.5 && max <= 1.0, "{max}");
}

// ----- property -----

/// Runs of sign in a sequence of heights: the number of turning points, ignoring changes below 1e-9.
fn turning_points(heights: &[f64]) -> usize {
    let mut last: Option<f64> = None;
    let mut turns = 0;
    for w in heights.windows(2) {
        let d = w[1] - w[0];
        if d.abs() <= 1e-9 {
            continue;
        }
        if last.is_some_and(|l| l.signum() != d.signum()) {
            turns += 1;
        }
        last = Some(d);
    }
    turns
}

proptest! {
    #![proptest_config(Config {
        cases: 256,
        rng_seed: RngSeed::Fixed(0x4C_4F_54),
        ..Config::default()
    })]

    #[test]
    fn every_surface_is_smooth(
        cx in 80.0_f64..170.0,
        cy in 80.0_f64..170.0,
        size in 20.0_f64..120.0,
        corners in 3_usize..8,
        phase in 0.0_f64..std::f64::consts::TAU,
        squash in 0.4_f64..1.0,
        gx in -0.3_f64..0.3,
        gy in -0.3_f64..0.3,
        qxx in -0.01_f64..0.01,
        qxy in -0.01_f64..0.01,
        qyy in -0.01_f64..0.01,
        bumps in 0.0_f64..2.0,
        seed in 0_u64..1000,
    ) {
        // a convex polygon: points on an ellipse, in angle order
        let outer: Vec<(f64, f64)> = (0..corners)
            .map(|i| {
                #[expect(clippy::cast_precision_loss, reason = "small")]
                let angle = phase + std::f64::consts::TAU * i as f64 / corners as f64;
                (cx + size / 2.0 * angle.cos(), cy + squash * size / 2.0 * angle.sin())
            })
            .collect();
        let area = lot_from("osm/way/1", closed(&outer), Vec::new(), size * size / 4.0);
        let base = heightmap(|x, y| {
            let (dx, dy) = (x - 128.0, y - 128.0);
            #[expect(clippy::cast_sign_loss, clippy::cast_possible_truncation, reason = "test")]
            let n = noise(seed, (x / CELL) as u64, (y / CELL) as u64) * bumps;
            100.0 + gx * dx + gy * dy + qxx * dx * dx + qxy * dx * dy + qyy * dy * dy + n
        });
        let (assessed, surfaces) = assess(std::slice::from_ref(&area), &base, &dry);
        prop_assert_eq!(assessed.len(), 1);
        let a = &assessed[0];
        if !matches!(a.outcome, Outcome::Smoothed { .. }) {
            return Ok(());
        }
        prop_assert_eq!(surfaces.len(), 1);
        let s = &surfaces[0].surface;
        let h = |c: i64, r: i64| s.height_at(pos(c, r));
        let inside: std::collections::BTreeSet<(i64, i64)> = vertices_in(&area)
            .into_iter()
            .map(|(c, r)| (i64::from(c), i64::from(r)))
            .collect();
        // rows, columns, diagonals, anti-diagonals: monotonic or one turning point, bounded second difference
        for (dc, dr) in [(1_i64, 0_i64), (0, 1), (1, 1), (1, -1)] {
            let spacing = CELL * f64::from(i32::try_from(dc * dc + dr * dr).unwrap()).sqrt();
            let bound = spacing * spacing / 600.0 + 1e-9;
            for &(c0, r0) in &inside {
                if inside.contains(&(c0 - dc, r0 - dr)) {
                    continue; // not the start of a run
                }
                let mut line = vec![h(c0, r0)];
                let (mut c, mut r) = (c0 + dc, r0 + dr);
                while inside.contains(&(c, r)) {
                    line.push(h(c, r));
                    c += dc;
                    r += dr;
                }
                prop_assert!(turning_points(&line) <= 1, "line at {c0},{r0} direction {dc},{dr}");
                for w in line.windows(3) {
                    prop_assert!((w[0] - 2.0 * w[1] + w[2]).abs() <= bound, "second difference");
                }
            }
        }
        prop_assert!(!has_interior_extremum(&area, &|p| s.height_at(p)));
        if s.lambda >= 1.0 - 1e-12 {
            prop_assert!(a.relief_after_m <= 0.12 + 1e-9, "relief {}", a.relief_after_m);
        } else {
            prop_assert!(
                a.relief_after_m <= (1.0 - s.lambda) * a.relief_before_m + 0.12 + 1e-9,
                "relief {} before {} lambda {}", a.relief_after_m, a.relief_before_m, s.lambda
            );
        }
    }
}

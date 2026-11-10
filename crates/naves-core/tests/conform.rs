// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Shaping the terrain to fit the roads.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "test helper code; failing loudly is the point and the numbers are tiny"
)]

use naves_core::conform::{CUT_SLOPE, FILL_SLOPE, FILL_SLOPE_MAX, MAX_EARTHWORK_M, conform};
use naves_core::heightmap::Heightmap;
use naves_core::profile::RoadStrip;

fn terrain(size: u32, cell: f32, f: impl Fn(f64, f64) -> f64) -> Heightmap {
    let mut h = Vec::new();
    for row in 0..size {
        for col in 0..size {
            h.push(f(
                f64::from(col) * f64::from(cell),
                f64::from(row) * f64::from(cell),
            ) as f32);
        }
    }
    Heightmap {
        size,
        cell_m: cell,
        h,
    }
}

fn strip(joints: &[(f64, f64, f64)], half_width: f64, rank: u8) -> RoadStrip {
    RoadStrip {
        slots: Vec::new(),
        joints: joints.to_vec(),
        bridge: Vec::new(),
        junction: Vec::new(),
        half_width,
        rank,
    }
}

/// A straight road from `a` to `b` at the heights `ha` and `hb`, with a joint every 25 m.
fn road(a: (f64, f64), b: (f64, f64), ha: f64, hb: f64, half_width: f64, rank: u8) -> RoadStrip {
    let length = (b.0 - a.0).hypot(b.1 - a.1);
    let n = (length / 25.0).ceil() as usize;
    let joints: Vec<(f64, f64, f64)> = (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            (
                a.0 + t * (b.0 - a.0),
                a.1 + t * (b.1 - a.1),
                ha + t * (hb - ha),
            )
        })
        .collect();
    strip(&joints, half_width, rank)
}

fn at(hm: &Heightmap, col: u32, row: u32) -> f64 {
    f64::from(hm.at(col, row))
}

fn pos(hm: &Heightmap, col: u32, row: u32) -> (f64, f64) {
    (
        f64::from(col) * f64::from(hm.cell_m),
        f64::from(row) * f64::from(hm.cell_m),
    )
}

/// Distance from `p` to the segment `a`-`b` and the parameter of the closest point.
fn to_line(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy), t)
}

#[test]
fn the_road_core_is_at_the_profile() {
    let base = terrain(100, 6.25, |x, y| {
        100.0 + 8.0 * (x / 40.0).sin() * (y / 55.0).cos()
    });
    let r = road((300.0, 80.0), (300.0, 500.0), 105.0, 105.0, 3.5, 3);
    let out = conform(&base, std::slice::from_ref(&r)).unwrap();
    let reach = 3.5 + 1.5 * 6.25;
    let mut core = 0;
    for row in 0..100 {
        for col in 0..100 {
            let p = pos(&base, col, row);
            let (d, _) = to_line(p, (300.0, 80.0), (300.0, 500.0));
            if d <= reach {
                assert!((at(&out.heightmap, col, row) - 105.0).abs() < 1e-3, "{p:?}");
                core += 1;
            }
        }
    }
    assert!(core > 100);
}

#[test]
fn the_cross_section_is_flat_and_the_terrain_straight_between_joints() {
    let base = terrain(100, 6.25, |_, _| 100.0);
    let r = road((300.0, 80.0), (300.0, 500.0), 100.0, 130.0, 3.5, 3);
    let out = conform(&base, std::slice::from_ref(&r)).unwrap();
    for row in 20..70 {
        let y = f64::from(row) * 6.25;
        let expected = 100.0 + 30.0 * (y - 80.0) / 420.0;
        for col in 44..52 {
            let x = f64::from(col) * 6.25;
            if (x - 300.0).abs() <= 3.5 + 1.5 * 6.25 {
                assert!(
                    (at(&out.heightmap, col, row) - expected).abs() < 0.02,
                    "col {col} row {row}"
                );
            }
        }
    }
}

#[test]
fn every_triangle_under_the_road_has_its_corners_on_the_road() {
    for cell in [6.25_f32, 12.5] {
        for degrees in [0.0_f64, 17.0, 45.0] {
            let size = 120;
            let base = terrain(size, cell, |x, y| {
                50.0 + 9.0 * (x / 33.0).sin() + 7.0 * (y / 41.0).cos()
            });
            let centre = f64::from(size) * f64::from(cell) / 2.0;
            let (dx, dy) = (
                degrees.to_radians().sin() * 300.0,
                degrees.to_radians().cos() * 300.0,
            );
            let (from, to) = ((centre - dx, centre - dy), (centre + dx, centre + dy));
            let straight = road(from, to, 60.0, 60.0, 3.5, 3);
            let out = conform(&base, std::slice::from_ref(&straight)).unwrap();
            for row in 0..size - 1 {
                for col in 0..size - 1 {
                    let middle = (
                        pos(&base, col, row).0 + f64::from(cell) / 2.0,
                        pos(&base, col, row).1 + f64::from(cell) / 2.0,
                    );
                    let (distance, along) = to_line(middle, from, to);
                    // the cell is under the road when its centre is within the half width
                    if distance <= 3.5 && along > 0.0 && along < 1.0 {
                        for (dc, dr) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                            let height = at(&out.heightmap, col + dc, row + dr);
                            assert!(
                                (height - 60.0).abs() < 1e-3,
                                "cell {col},{row} corner {dc},{dr}: {height} (cell {cell}, {degrees} degrees)"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Maximum steepness between neighbouring vertices of a region.
fn steepest(hm: &Heightmap, f: impl Fn(u32, u32) -> bool) -> f64 {
    let mut worst = 0.0_f64;
    for row in 0..hm.size - 1 {
        for col in 0..hm.size - 1 {
            if f(col, row) {
                for (dc, dr) in [(1, 0), (0, 1)] {
                    let s = (at(hm, col + dc, row + dr) - at(hm, col, row)).abs()
                        / f64::from(hm.cell_m);
                    worst = worst.max(s);
                }
            }
        }
    }
    worst
}

#[test]
fn a_road_along_a_valley_side_gets_a_bench() {
    // a 25 degree slope falling to the east; the road runs along it at the height of the ground under it
    let slope = 25.0_f64.to_radians().tan();
    let base = terrain(160, 6.25, |x, _| 400.0 - slope * x);
    let r = road(
        (500.0, 100.0),
        (500.0, 900.0),
        400.0 - slope * 500.0,
        400.0 - slope * 500.0,
        3.5,
        3,
    );
    let out = conform(&base, std::slice::from_ref(&r)).unwrap();
    assert!(out.capped.is_empty(), "{:?}", out.capped);
    assert!(
        out.max_cut_m > 1.0 && out.max_fill_m > 1.0,
        "cut {} fill {}",
        out.max_cut_m,
        out.max_fill_m
    );
    // wider than the road, with daylight on both sides: unchanged ground at the foot and at the top
    let row = 80;
    let changed: Vec<u32> = (0..160)
        .filter(|&c| (at(&out.heightmap, c, row) - at(&base, c, row)).abs() > 1e-4)
        .collect();
    let (lo, hi) = (*changed.first().unwrap(), *changed.last().unwrap());
    assert!(f64::from(hi - lo) * 6.25 > 2.0 * 3.5 + 10.0);
    assert!(lo > 0 && hi < 159, "the earthworks run off the map");
    // the ground within the footprint is the road
    for col in 0..160 {
        if (f64::from(col) * 6.25 - 500.0).abs() <= 3.5 {
            assert!((at(&out.heightmap, col, row) - (400.0 - slope * 500.0)).abs() < 1e-3);
        }
    }
    // slopes next to the road stay inside the limits (the chamfer is 10 % off at worst)
    let limit = CUT_SLOPE.max(slope) * 1.1;
    assert!(
        steepest(&out.heightmap, |c, r| r > 20
            && r < 140
            && (f64::from(c) * 6.25 - 500.0).abs() < 100.0)
            <= limit
    );
}

#[test]
fn an_embankment_on_a_40_degree_side_reaches_the_ground() {
    let slope = 40.0_f64.to_radians().tan();
    let base = terrain(200, 6.25, |x, _| 500.0 - slope * x);
    // the road lies on the ground at its downhill edge: at the centre the level strip is 12.875 m wide
    // and the embankment would need 67 m, more than the 60 m cap
    let r = road(
        (600.0, 200.0),
        (600.0, 1000.0),
        500.0 - slope * 603.5,
        500.0 - slope * 603.5,
        3.5,
        3,
    );
    let out = conform(&base, std::slice::from_ref(&r)).unwrap();
    assert!(out.capped.is_empty(), "{} capped", out.capped.len());
    let downhill = steepest(&out.heightmap, |col, row| {
        col > 100 && (40..150).contains(&row)
    });
    assert!(downhill <= FILL_SLOPE_MAX + 0.05, "{downhill}");
}

#[test]
fn gentle_ground_keeps_the_gentle_slopes() {
    // a road 6 m above a 10 % slope: the embankment is 34 degrees, not steeper
    let base = terrain(200, 6.25, |x, _| 300.0 - 0.10 * x);
    let r = road((600.0, 200.0), (600.0, 1000.0), 246.0, 246.0, 3.5, 3);
    let out = conform(&base, std::slice::from_ref(&r)).unwrap();
    let steep = steepest(&out.heightmap, |_, row| (40..150).contains(&row));
    assert!(steep <= FILL_SLOPE + 0.05, "{steep}");
}

#[test]
fn level_ground_and_the_map_edge_are_shaped_as_before() {
    // natural slope zero, and a road whose core touches the border
    let base = terrain(64, 6.25, |_, _| 100.0);
    let r = road((3.0, 0.0), (3.0, 393.0), 103.0, 103.0, 3.5, 3);
    let out = conform(&base, std::slice::from_ref(&r)).unwrap();
    assert!(out.heightmap.h.iter().all(|h| h.is_finite()));
    let steep = steepest(&out.heightmap, |_, _| true);
    assert!(steep <= FILL_SLOPE + 0.05, "{steep}");
}

#[test]
fn a_road_across_a_valley_floor_gets_an_embankment() {
    // a V-shaped valley narrower than two cells of a 30 m DEM, crossed by a level road
    let base = terrain(120, 6.25, |_, y| {
        100.0 - 25.0 * (1.0 - ((y - 375.0).abs() / 15.0).min(1.0))
    });
    let r = road((100.0, 375.0), (650.0, 375.0), 100.0, 100.0, 3.5, 3);
    let out = conform(&base, std::slice::from_ref(&r)).unwrap();
    // the floor under the road is filled to the road, and the fill is wider than the road
    let col = 60;
    assert!((at(&out.heightmap, col, 60) - 100.0).abs() < 1e-3);
    assert!(out.max_fill_m > 20.0);
    assert!(at(&out.heightmap, col, 64) >= at(&base, col, 64));
}

#[test]
fn a_road_through_a_ridge_gets_a_cutting() {
    let base = terrain(120, 6.25, |_, y| {
        100.0 + 30.0 * (-((y - 375.0) / 25.0).powi(2)).exp()
    });
    let r = road((100.0, 375.0), (650.0, 375.0), 100.0, 100.0, 3.5, 3);
    let out = conform(&base, std::slice::from_ref(&r)).unwrap();
    assert!(out.max_cut_m > 25.0);
    assert!((at(&out.heightmap, 60, 60) - 100.0).abs() < 1e-3);
    assert!(out.capped.is_empty());
}

#[test]
fn nothing_changes_outside_the_earthworks() {
    let base = terrain(160, 6.25, |x, y| {
        200.0 + 30.0 * (x / 120.0).sin() + 20.0 * (y / 90.0).cos()
    });
    let r = road((300.0, 100.0), (700.0, 800.0), 190.0, 215.0, 3.5, 3);
    let out = conform(&base, std::slice::from_ref(&r)).unwrap();
    let a = (300.0, 100.0);
    let b = (700.0, 800.0);
    let mut changed = 0;
    for row in 0..160 {
        for col in 0..160 {
            let (d, _) = to_line(pos(&base, col, row), a, b);
            if d > MAX_EARTHWORK_M + 3.5 + 1.5 * 6.25 + 6.25 {
                assert_eq!(
                    out.heightmap.at(col, row).to_bits(),
                    base.at(col, row).to_bits(),
                    "{col},{row}"
                );
            } else if out.heightmap.at(col, row).to_bits() != base.at(col, row).to_bits() {
                changed += 1;
            }
        }
    }
    assert!(changed > 50);
}

#[test]
fn a_cliff_beyond_the_meeting_line_stays() {
    // gentle ground, and a 70 degree wall 100 m from the road
    let wall = 70.0_f64.to_radians().tan();
    let base = terrain(160, 6.25, |x, _| {
        100.0 + if x > 600.0 { wall * (x - 600.0) } else { 0.0 }
    });
    let r = road((400.0, 100.0), (400.0, 900.0), 100.0, 100.0, 3.5, 3);
    let out = conform(&base, std::slice::from_ref(&r)).unwrap();
    for row in 0..160 {
        for col in 0..160 {
            if f64::from(col) * 6.25 > 520.0 {
                assert_eq!(
                    out.heightmap.at(col, row).to_bits(),
                    base.at(col, row).to_bits(),
                    "{col},{row}"
                );
            }
        }
    }
}

#[test]
fn a_wall_next_to_the_road_stops_at_the_cap_and_is_reported() {
    let wall = 70.0_f64.to_radians().tan();
    let base = terrain(160, 6.25, |x, _| {
        100.0 + if x > 410.0 { wall * (x - 410.0) } else { 0.0 }
    });
    let r = road((400.0, 100.0), (400.0, 900.0), 100.0, 100.0, 3.5, 3);
    let out = conform(&base, std::slice::from_ref(&r)).unwrap();
    assert!(!out.capped.is_empty());
    for row in 0..160 {
        for col in 0..160 {
            if f64::from(col) * 6.25 > 400.0 + MAX_EARTHWORK_M + 3.5 + 1.5 * 6.25 + 12.5 {
                assert_eq!(
                    out.heightmap.at(col, row).to_bits(),
                    base.at(col, row).to_bits()
                );
            }
        }
    }
}

#[test]
fn parallel_roads_at_different_heights_report_a_conflict() {
    let base = terrain(120, 6.25, |_, _| 100.0);
    let a = road((300.0, 100.0), (300.0, 650.0), 100.0, 100.0, 3.5, 3);
    let b = road((312.0, 100.0), (312.0, 650.0), 110.0, 110.0, 3.5, 2);
    let out = conform(&base, &[a, b]).unwrap();
    assert!(!out.conflicts.is_empty());
    assert!(out.conflicts.iter().any(|c| c.height_difference_m > 5.0));
    let (lo, hi) = out.heightmap.min_max();
    assert!(lo >= 100.0 - 1e-3 && hi <= 110.0 + 1e-3, "{lo} {hi}");
    // the higher rank keeps its height where the two clash
    assert!((at(&out.heightmap, 48, 60) - 100.0).abs() < 1e-3);
}

#[test]
fn the_base_is_untouched() {
    let base = terrain(80, 6.25, |x, _| 100.0 + x / 10.0);
    let copy = base.clone();
    let r = road((200.0, 50.0), (200.0, 450.0), 120.0, 120.0, 3.5, 3);
    conform(&base, &[r]).unwrap();
    assert_eq!(base, copy);
}

#[test]
fn a_road_along_the_map_edge_does_not_panic() {
    let base = terrain(80, 6.25, |x, y| 100.0 + (x + y) / 20.0);
    for line in [
        ((0.0, 0.0), (0.0, 490.0)),
        ((0.0, 0.0), (490.0, 0.0)),
        ((-30.0, 100.0), (600.0, 100.0)),
        ((490.0, 490.0), (490.0, 0.0)),
    ] {
        let r = road(line.0, line.1, 110.0, 115.0, 3.5, 3);
        let out = conform(&base, &[r]).unwrap();
        assert_eq!(out.heightmap.size, 80);
    }
}

#[test]
fn no_roads_change_nothing() {
    let base = terrain(40, 6.25, |x, _| x);
    let out = conform(&base, &[]).unwrap();
    assert_eq!(out.heightmap, base);
    assert!(out.max_cut_m.abs() < f32::EPSILON && out.max_fill_m.abs() < f32::EPSILON);
}

#[test]
fn a_heightmap_of_the_wrong_size_is_an_error() {
    let base = Heightmap {
        size: 10,
        cell_m: 6.25,
        h: vec![0.0; 50],
    };
    assert!(conform(&base, &[]).is_err());
}

#[test]
fn conforming_is_deterministic() {
    let base = terrain(120, 6.25, |x, y| {
        100.0 + 20.0 * (x / 80.0).sin() * (y / 60.0).cos()
    });
    let rs = [
        road((200.0, 100.0), (500.0, 600.0), 100.0, 90.0, 3.5, 3),
        road((100.0, 400.0), (650.0, 300.0), 95.0, 95.0, 2.5, 1),
    ];
    let a = conform(&base, &rs).unwrap();
    let b = conform(&base, &rs).unwrap();
    assert_eq!(a, b);
}

#[test]
fn the_ground_under_a_bridge_keeps_its_height() {
    // a gorge 30 m deep, crossed by a level road on a bridge from y = 300 to y = 500
    let base = terrain(120, 6.25, |_, y| {
        100.0
            - if (300.0..500.0).contains(&y) {
                30.0
            } else {
                0.0
            }
    });
    let mut joints = vec![
        (300.0, 100.0, 100.0),
        (300.0, 300.0, 100.0),
        (300.0, 500.0, 100.0),
        (300.0, 700.0, 100.0),
    ];
    joints.dedup();
    let mut road = strip(&joints, 3.5, 3);
    road.bridge = vec![false, true, false];
    let out = conform(&base, std::slice::from_ref(&road)).unwrap();
    // in the middle of the span the ground is as it was
    for row in 63..69 {
        for col in 40..56 {
            assert_eq!(
                out.heightmap.at(col, row).to_bits(),
                base.at(col, row).to_bits(),
                "{col},{row}"
            );
        }
    }
    // beside the abutments the road still shapes the ground
    assert!((at(&out.heightmap, 48, 40) - 100.0).abs() < 1e-3);
    assert!((at(&out.heightmap, 48, 100) - 100.0).abs() < 1e-3);
}

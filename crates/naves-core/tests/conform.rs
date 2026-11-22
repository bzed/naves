// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Shaping the terrain to fit the roads.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "test helper code; failing loudly is the point and the numbers are tiny"
)]

use naves_core::bridgeground::BridgeSpan;
use naves_core::catalog::PartShape;
use naves_core::conform::{
    AreaPatch, BRIDGE_AIR_M, CUT_SLOPE, CUT_SLOPE_MAX, Conformed, FILL_SLOPE, FILL_SLOPE_MAX,
    LevelPatch, MAX_EARTHWORK_M, conform, conform_areas, conform_bridges,
};
use naves_core::heightmap::Heightmap;
use naves_core::lot::{
    Form, LOT_MAX_MOVE_M, LotArea, LotKind, LotSurface, Outcome, Placement, Polygon, Surface,
    assess, contains, edge_distance, local_relief,
};
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
        side_slope: Vec::new(),
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

/// The deck of a bridge along x = 300 from `y0` to `y1` at 100 m, as thick as a panel.
fn deck(y0: f64, y1: f64) -> BridgeSpan {
    BridgeSpan {
        stretch: "deck".to_string(),
        first: (300.0, y0, 100.0),
        second: (300.0, y1, 100.0),
        shape: PartShape::level(y1 - y0, 0.25, 3.5),
        own_strips: Vec::new(),
    }
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
    let out = conform(&base, std::slice::from_ref(&r), &[]).unwrap();
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
    let out = conform(&base, std::slice::from_ref(&r), &[]).unwrap();
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
            let out = conform(&base, std::slice::from_ref(&straight), &[]).unwrap();
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
    let out = conform(&base, std::slice::from_ref(&r), &[]).unwrap();
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
    let out = conform(&base, std::slice::from_ref(&r), &[]).unwrap();
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
    let out = conform(&base, std::slice::from_ref(&r), &[]).unwrap();
    let steep = steepest(&out.heightmap, |_, row| (40..150).contains(&row));
    assert!(steep <= FILL_SLOPE + 0.05, "{steep}");
}

#[test]
fn level_ground_and_the_map_edge_are_shaped_as_before() {
    // natural slope zero, and a road whose core touches the border
    let base = terrain(64, 6.25, |_, _| 100.0);
    let r = road((3.0, 0.0), (3.0, 393.0), 103.0, 103.0, 3.5, 3);
    let out = conform(&base, std::slice::from_ref(&r), &[]).unwrap();
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
    let out = conform(&base, std::slice::from_ref(&r), &[]).unwrap();
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
    let out = conform(&base, std::slice::from_ref(&r), &[]).unwrap();
    assert!(out.max_cut_m > 25.0);
    assert!((at(&out.heightmap, 60, 60) - 100.0).abs() < 1e-3);
    assert_eq!(out.capped.len(), 0);
}

#[test]
fn nothing_changes_outside_the_earthworks() {
    let base = terrain(160, 6.25, |x, y| {
        200.0 + 30.0 * (x / 120.0).sin() + 20.0 * (y / 90.0).cos()
    });
    let r = road((300.0, 100.0), (700.0, 800.0), 190.0, 215.0, 3.5, 3);
    let out = conform(&base, std::slice::from_ref(&r), &[]).unwrap();
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
    let out = conform(&base, std::slice::from_ref(&r), &[]).unwrap();
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
    let out = conform(&base, std::slice::from_ref(&r), &[]).unwrap();
    assert_ne!(out.capped.len(), 0);
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
    // the widths overlap: 2 m apart, 7 m wide (roads whose widths do not overlap each keep their ground)
    let b = road((302.0, 100.0), (302.0, 650.0), 110.0, 110.0, 3.5, 2);
    let out = conform(&base, &[a, b], &[]).unwrap();
    assert_ne!(out.conflicts.len(), 0);
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
    conform(&base, &[r], &[]).unwrap();
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
        let out = conform(&base, &[r], &[]).unwrap();
        assert_eq!(out.heightmap.size, 80);
    }
}

#[test]
fn no_roads_change_nothing() {
    let base = terrain(40, 6.25, |x, _| x);
    let out = conform(&base, &[], &[]).unwrap();
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
    assert!(conform(&base, &[], &[]).is_err());
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
    let a = conform(&base, &rs, &[]).unwrap();
    let b = conform(&base, &rs, &[]).unwrap();
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
    let out = conform_bridges(
        &base,
        std::slice::from_ref(&road),
        &[],
        &[deck(300.0, 500.0)],
    )
    .unwrap();
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

#[expect(
    clippy::unwrap_used,
    reason = "a test helper: failing loudly is the point"
)]
/// A level road at 100 m over a gorge of `depth` between y = 300 and y = 300 + `span`, on a bridge.
fn short_bridge(cell: f32, span: f64, depth: f64) {
    let size = (800.0 / f64::from(cell)) as u32;
    let base = terrain(size, cell, |_, y| {
        100.0
            - if y > 300.0 && y < 300.0 + span {
                depth
            } else {
                0.0
            }
    });
    let joints = vec![
        (300.0, 100.0, 100.0),
        (300.0, 300.0, 100.0),
        (300.0, 300.0 + span, 100.0),
        (300.0, 600.0, 100.0),
    ];
    let mut road = strip(&joints, 3.5, 3);
    road.bridge = vec![false, true, false];
    let out = conform_bridges(
        &base,
        std::slice::from_ref(&road),
        &[],
        &[deck(300.0, 300.0 + span)],
    )
    .unwrap();
    let c = |m: f64| (m / f64::from(cell)).round() as u32;
    // the ground under the span, around the centre line, is as it was
    for row in 0..size {
        let y = f64::from(row) * f64::from(cell);
        if y <= 300.0 || y >= 300.0 + span {
            continue;
        }
        let reach = 3.5 + 1.5 * f64::from(cell);
        for col in
            c(300.0 - reach + 0.5 * f64::from(cell))..=c(300.0 + reach - 0.5 * f64::from(cell))
        {
            assert_eq!(
                out.heightmap.at(col, row).to_bits(),
                base.at(col, row).to_bits(),
                "cell {cell}, span {span}: {col},{row}"
            );
        }
    }
    // the approaches are shaped up to the abutments
    for y in [250.0, 300.0 - f64::from(cell), 300.0] {
        assert!(
            (at(&out.heightmap, c(300.0), c(y)) - 100.0).abs() < 1e-3,
            "{y}"
        );
    }
    assert!((at(&out.heightmap, c(300.0), c(300.0 + span)) - 100.0).abs() < 1e-3);
}

#[test]
fn the_ground_under_a_short_bridge_keeps_its_height() {
    short_bridge(6.25, 25.0, 6.0);
}

#[test]
fn the_ground_under_a_longer_bridge_keeps_its_height() {
    short_bridge(6.25, 50.0, 6.0);
}

#[test]
fn the_ground_under_a_short_bridge_keeps_its_height_on_fine_cells() {
    short_bridge(2.5, 25.0, 6.0);
}

/// The terrain height at a map position, bilinear.
fn sample(hm: &Heightmap, x: f64, y: f64) -> f64 {
    f64::from(hm.sample(x, y).height)
}

/// A road along x from 20 m to 380 m at `y` and the height `h`, with the side slope `side` of the ground.
fn straight_strip(y: f64, h: f64, half: f64, rank: u8, side: f64) -> RoadStrip {
    let mut r = road((20.0, y), (380.0, y), h, h, half, rank);
    r.side_slope = vec![side; r.joints.len() - 1];
    r
}

/// The steepest slope between neighbouring vertices in the columns around `x`, north of the road at `y`
/// and beyond the level strip.
fn max_slope_beside(hm: &Heightmap, y: f64) -> f64 {
    let cell = f64::from(hm.cell_m);
    let col = (200.0 / cell) as u32;
    let first = ((y + 3.0 + 1.5 * cell) / cell).ceil() as u32;
    let mut worst = 0.0_f64;
    for row in first..first + 12 {
        let step = (at(hm, col, row + 1) - at(hm, col, row)).abs() / cell;
        worst = worst.max(step);
    }
    worst
}

#[test]
fn two_roads_side_by_side_at_different_heights_each_keep_their_ground() {
    // two 6 m wide roads, centre lines 12 m apart, at 100 and 105 m, on 3.125 m cells
    let base = terrain(128, 3.125, |_, _| 102.0);
    let strips = [
        straight_strip(194.0, 100.0, 3.0, 3, 0.0),
        straight_strip(206.0, 105.0, 3.0, 6, 0.0),
    ];
    let out = conform(&base, &strips, &[]).unwrap();
    for x in (60..340).step_by(5) {
        for (y, h) in [(194.0, 100.0), (206.0, 105.0)] {
            for dy in [-3.0, 0.0, 3.0] {
                let got = sample(&out.heightmap, f64::from(x), y + dy);
                // the vertex between the roads (y = 200) is the higher road's; at y = 197 the bilinear value
                // is 0.2 m above the lower road (0.125 of a 3.125 m cell towards a vertex 5 m higher), so
                // the limit is above that
                assert!(
                    (got - h).abs() < 0.3,
                    "x {x} y {} is {got}, not {h}",
                    y + dy
                );
            }
        }
    }
    assert!(out.conflicts.is_empty(), "{:?}", out.conflicts);
}

#[test]
fn roads_that_overlap_at_different_heights_are_still_a_conflict() {
    let base = terrain(128, 3.125, |_, _| 102.0);
    let strips = [
        straight_strip(200.0, 100.0, 3.0, 3, 0.0),
        straight_strip(202.0, 105.0, 3.0, 6, 0.0),
    ];
    assert_ne!(conform(&base, &strips, &[]).unwrap().conflicts.len(), 0);
}

#[test]
fn on_a_steep_hillside_the_cut_is_a_face_one_cell_wide() {
    // hillside rising 60 % to the north, road along x at the height of the ground under its centre
    let base = terrain(128, 3.125, |_, y| 100.0 + 0.6 * y);
    let strip = straight_strip(200.0, 220.0, 3.0, 3, 0.6);
    let out = conform(&base, &[strip], &[]).unwrap();
    // uphill: the original ground is back within two cells of the edge of the level strip
    let edge = 200.0 + 3.0 + 1.5 * 3.125;
    let y = edge + 2.0 * 3.125;
    let got = sample(&out.heightmap, 200.0, y);
    assert!((got - (100.0 + 0.6 * y)).abs() < 0.5, "{got}");
}

#[test]
fn on_gentle_ground_the_cut_slope_is_as_before() {
    let base = terrain(128, 3.125, |_, y| 100.0 + 0.1 * y);
    let strip = straight_strip(200.0, 115.0, 3.0, 3, 0.1);
    let out = conform(&base, &[strip], &[]).unwrap();
    assert!(max_slope_beside(&out.heightmap, 200.0) <= CUT_SLOPE_MAX + 0.05);
}

#[test]
fn a_patch_levels_the_ground_at_a_join() {
    let base = terrain(128, 3.125, |x, _| 100.0 + 0.3 * x);
    let patch = LevelPatch {
        at: (200.0, 200.0),
        radius: 8.0,
        height: 160.0,
        rank: 3,
        gradient: (0.0, 0.0),
        inner_radius: 0.0,
    };
    let out = conform(&base, &[], &[patch]).unwrap();
    for (dx, dy) in [(0.0, 0.0), (6.0, 0.0), (-6.0, 0.0), (0.0, 6.0)] {
        let got = sample(&out.heightmap, 200.0 + dx, 200.0 + dy);
        assert!((got - 160.0).abs() < 0.2, "{dx} {dy}: {got}");
    }
}

#[test]
fn a_patch_beside_a_road_at_another_height_takes_the_vertices_nearer_to_its_edge() {
    let base = terrain(128, 3.125, |_, _| 102.0);
    let road = straight_strip(200.0, 100.0, 3.0, 3, 0.0);
    // its edge is 20 m from the road's edge
    let patch = LevelPatch {
        at: (200.0, 230.0),
        radius: 7.0,
        height: 110.0,
        rank: 3,
        gradient: (0.0, 0.0),
        inner_radius: 0.0,
    };
    let out = conform(&base, &[road], &[patch]).unwrap();
    assert!(out.conflicts.is_empty(), "{:?}", out.conflicts);
    assert!((sample(&out.heightmap, 200.0, 200.0) - 100.0).abs() < 0.2);
    assert!((sample(&out.heightmap, 200.0, 230.0) - 110.0).abs() < 0.2);
}

#[test]
fn a_patch_over_a_road_at_another_height_leaves_the_road_its_width() {
    let base = terrain(128, 3.125, |_, _| 102.0);
    let road = straight_strip(200.0, 100.0, 3.0, 3, 0.0);
    let patch = LevelPatch {
        at: (200.0, 204.0),
        radius: 7.0,
        height: 100.8,
        rank: 3,
        gradient: (0.0, 0.0),
        inner_radius: 0.0,
    };
    let out = conform(&base, &[road], &[patch]).unwrap();
    // a patch is not a conflict with a road; the road keeps the vertices of its own width
    assert!(out.conflicts.is_empty(), "{:?}", out.conflicts);
    assert!((sample(&out.heightmap, 200.0, 200.0) - 100.0).abs() < 0.05);
    assert!((sample(&out.heightmap, 200.0, 206.25) - 100.8).abs() < 0.05);
}

#[test]
fn a_road_sloping_through_a_patch_keeps_its_own_heights_across_its_width() {
    let base = terrain(128, 3.125, |_, _| 102.0);
    // 8 % to the east; the patch is at the height of the road at x = 200
    let road = road((20.0, 200.0), (380.0, 200.0), 100.0, 128.8, 4.0, 3);
    let at_x = |x: f64| 100.0 + 0.08 * (x - 20.0);
    let patch = LevelPatch {
        at: (200.0, 200.0),
        radius: 9.125,
        height: at_x(200.0),
        rank: 3,
        gradient: (0.0, 0.0),
        inner_radius: 0.0,
    };
    let out = conform(&base, &[road], &[patch]).unwrap();
    assert!(out.conflicts.is_empty(), "{:?}", out.conflicts);
    for x in [193.75, 196.875, 200.0, 203.125, 206.25] {
        // the vertices of the road's own width: 3.125 m either side of the centre line
        for dy in [-3.125, 0.0, 3.125] {
            let got = sample(&out.heightmap, x, 200.0 + dy);
            assert!((got - at_x(x)).abs() < 0.05, "x {x} dy {dy}: {got}");
        }
    }
    // beside the road the patch levels the ground
    let got = sample(&out.heightmap, 206.25, 206.25);
    assert!((got - at_x(200.0)).abs() < 0.1, "{got}");
}

#[test]
fn a_patch_does_not_level_the_ground_under_a_bridge_span() {
    let base = terrain(120, 6.25, |_, y| {
        100.0
            - if (300.0..500.0).contains(&y) {
                30.0
            } else {
                0.0
            }
    });
    let mut road = strip(
        &[
            (300.0, 100.0, 100.0),
            (300.0, 300.0, 100.0),
            (300.0, 500.0, 100.0),
            (300.0, 700.0, 100.0),
        ],
        3.5,
        3,
    );
    road.bridge = vec![false, true, false];
    let patch = LevelPatch {
        at: (300.0, 400.0),
        radius: 8.0,
        height: 100.0,
        rank: 3,
        gradient: (0.0, 0.0),
        inner_radius: 0.0,
    };
    let out = conform_bridges(&base, &[road], &[patch], &[deck(300.0, 500.0)]).unwrap();
    assert!((sample(&out.heightmap, 300.0, 400.0) - 70.0).abs() < 1e-3);
}

#[test]
fn two_legs_of_one_road_at_different_heights_each_keep_their_ground() {
    // a road out along y = 200 and back along y = 212, 12 m apart, 5 m higher on the way back
    let base = terrain(128, 3.125, |_, _| 102.0);
    let road = strip(
        &[
            (60.0, 200.0, 100.0),
            (200.0, 200.0, 100.0),
            (200.0, 212.0, 105.0),
            (60.0, 212.0, 105.0),
        ],
        3.0,
        3,
    );
    let out = conform(&base, &[road], &[]).unwrap();
    assert!(out.conflicts.is_empty(), "{:?}", out.conflicts);
    for x in [80.0, 120.0, 160.0] {
        assert!((sample(&out.heightmap, x, 200.0) - 100.0).abs() < 0.2);
        assert!((sample(&out.heightmap, x, 212.0) - 105.0).abs() < 0.2);
    }
}

#[test]
fn a_patch_that_is_not_a_circle_on_the_map_claims_nothing() {
    let base = terrain(32, 3.125, |_, _| 102.0);
    for (at, radius, height) in [
        ((f64::NAN, 50.0), 8.0, 120.0),
        ((50.0, f64::NAN), 8.0, 120.0),
        ((50.0, 50.0), f64::NAN, 120.0),
        ((50.0, 50.0), f64::INFINITY, 120.0),
        ((50.0, 50.0), 0.0, 120.0),
        ((50.0, 50.0), -4.0, 120.0),
        ((50.0, 50.0), 8.0, f64::NAN),
        ((50.0, 50.0), 8.0, f64::INFINITY),
    ] {
        let patch = LevelPatch {
            at,
            radius,
            height,
            rank: 3,
            gradient: (0.0, 0.0),
            inner_radius: 0.0,
        };
        let out = conform(&base, &[], &[patch]).unwrap();
        assert_eq!(out.heightmap, base, "{at:?} {radius} {height}");
    }
}

#[test]
fn an_empty_map_with_a_patch_is_left_alone() {
    let base = Heightmap {
        size: 0,
        cell_m: 3.125,
        h: Vec::new(),
    };
    let patch = LevelPatch {
        at: (10.0, 10.0),
        radius: 8.0,
        height: 120.0,
        rank: 3,
        gradient: (0.0, 0.0),
        inner_radius: 0.0,
    };
    let out = conform(&base, &[], &[patch]).unwrap();
    assert_eq!(out.heightmap.h.len(), 0);
}

#[test]
fn a_vertex_both_roads_need_goes_to_the_road_whose_edge_is_nearer() {
    let base = terrain(128, 3.125, |_, _| 102.0);
    // edges 4 m apart; the vertex at y = 206.25 is 3.25 m from the edge of the road at 200 (rank 6) and 0.75 m
    // from the edge of the road at 210 (rank 3), and both want it
    let strips = [
        straight_strip(200.0, 100.0, 3.0, 6, 0.0),
        straight_strip(210.0, 105.0, 3.0, 3, 0.0),
    ];
    let out = conform(&base, &strips, &[]).unwrap();
    assert!(out.conflicts.is_empty(), "{:?}", out.conflicts);
    assert!((at(&out.heightmap, 64, 66) - 105.0).abs() < 1e-3);
    assert!((at(&out.heightmap, 64, 64) - 100.0).abs() < 1e-3);
}

#[test]
fn a_tie_between_the_edges_goes_to_the_higher_rank() {
    let base = terrain(128, 3.125, |_, _| 102.0);
    // the vertex at y = 200 is 6 m from both centre lines
    let strips = [
        straight_strip(194.0, 100.0, 3.0, 3, 0.0),
        straight_strip(206.0, 105.0, 3.0, 6, 0.0),
    ];
    let out = conform(&base, &strips, &[]).unwrap();
    assert!((at(&out.heightmap, 64, 64) - 105.0).abs() < 1e-3);
}

#[test]
fn a_vertex_inside_the_lower_roads_width_stays_with_it_in_the_higher_roads_ring() {
    let base = terrain(128, 3.125, |_, _| 102.0);
    // the higher road (rank 6, y = 200, half width 3) reaches 7.7 m; the lower (rank 3, y = 207, half width 4) has
    // the vertex at y = 203.125 inside its width (3.9 m from its centre), the higher road has it in its ring
    let strips = [
        straight_strip(200.0, 100.0, 3.0, 6, 0.0),
        straight_strip(207.0, 105.0, 4.0, 3, 0.0),
    ];
    let out = conform(&base, &strips, &[]).unwrap();
    assert!(out.conflicts.is_empty(), "{:?}", out.conflicts);
    assert!((at(&out.heightmap, 64, 65) - 105.0).abs() < 1e-3);
    assert!((at(&out.heightmap, 64, 64) - 100.0).abs() < 1e-3);
}

#[test]
fn equal_ranks_and_equal_edges_go_to_the_first_road() {
    let base = terrain(128, 3.125, |_, _| 102.0);
    let strips = [
        straight_strip(194.0, 100.0, 3.0, 3, 0.0),
        straight_strip(206.0, 105.0, 3.0, 3, 0.0),
    ];
    let out = conform(&base, &strips, &[]).unwrap();
    // the vertex at y = 200 is 6 m from both
    assert!((at(&out.heightmap, 64, 64) - 100.0).abs() < 1e-3);
}

#[test]
fn a_tilted_patch_follows_the_plane() {
    let base = terrain(128, 3.125, |_, _| 100.0);
    let patch = LevelPatch {
        at: (200.0, 200.0),
        radius: 30.0,
        height: 140.0,
        rank: 3,
        gradient: (0.06, -0.03),
        inner_radius: 0.0,
    };
    let out = conform(&base, &[], &[patch]).unwrap();
    let mut checked = 0;
    for row in 0..128 {
        for col in 0..128 {
            let (x, y) = (f64::from(col) * 3.125, f64::from(row) * 3.125);
            if (x - 200.0).hypot(y - 200.0) <= 22.0 {
                let want = 140.0 + 0.06 * (x - 200.0) - 0.03 * (y - 200.0);
                let got = at(&out.heightmap, col, row);
                assert!((got - want).abs() < 1e-3, "{x} {y}: {got} {want}");
                checked += 1;
            }
        }
    }
    assert!(checked > 100);
}

#[test]
fn an_annular_patch_leaves_the_inside_alone() {
    let base = terrain(256, 3.125, |x, _| 100.0 + 0.1 * x);
    let patch = LevelPatch {
        at: (400.0, 400.0),
        radius: 120.0,
        height: 140.0,
        rank: 3,
        gradient: (0.05, 0.0),
        inner_radius: 100.0,
    };
    let out = conform(&base, &[], &[patch]).unwrap();
    for row in 0..256 {
        for col in 0..256 {
            let (x, y) = (f64::from(col) * 3.125, f64::from(row) * 3.125);
            let d = (x - 400.0).hypot(y - 400.0);
            let got = at(&out.heightmap, col, row);
            if d < 90.0 {
                assert!((got - (100.0 + 0.1 * x)).abs() < 1e-3, "{x} {y}: {got}");
            } else if (104.0..=116.0).contains(&d) {
                let want = 140.0 + 0.05 * (x - 400.0);
                assert!((got - want).abs() < 0.3, "{x} {y}: {got} {want}");
            }
        }
    }
}

// ----- lots -----

/// A closed rectangle.
fn ring(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<(f64, f64)> {
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]
}

/// A lot area on the ground.
fn lot_area(id: &str, outer: Vec<(f64, f64)>, holes: Vec<Vec<(f64, f64)>>) -> LotArea {
    LotArea {
        id: id.to_string(),
        kind: LotKind::CarPark,
        placement: Placement::Ground,
        outer,
        holes,
        area_m2: 0.0,
        surface_tag: None,
        holes_dropped: 0,
    }
}

/// A lot's smooth surface as a plane through `at`, with the move guard `lambda`.
fn plane_patch(
    polygons: Vec<Polygon>,
    at: (f64, f64),
    height: f64,
    gradient: (f64, f64),
    lambda: f64,
) -> AreaPatch {
    AreaPatch {
        surface: LotSurface {
            group: 0,
            members: vec!["osm/way/1".to_string()],
            polygons,
            surface: Surface {
                at,
                coeffs: [height, gradient.0, gradient.1, 0.0, 0.0, 0.0],
                form: Form::Plane,
                lambda,
            },
        },
        rank: 0,
    }
}

/// The shaped height at a map position, bilinear.
fn shaped(out: &Conformed, x: f64, y: f64) -> f64 {
    f64::from(out.heightmap.sample(x, y).height)
}

/// The shaped height at a vertex.
fn shaped_at(out: &Conformed, col: i64, row: i64) -> Option<f64> {
    let col = u32::try_from(col).ok()?;
    let row = u32::try_from(row).ok()?;
    Some(f64::from(out.heightmap.at(col, row)))
}

#[test]
fn a_lot_is_shaped_to_its_plane_and_the_hill_falls_back_to_the_ground() {
    let base = terrain(64, 4.0, |_, _| 100.0);
    let lot = plane_patch(
        vec![(ring(100.0, 100.0, 180.0, 180.0), Vec::new())],
        (140.0, 140.0),
        101.0,
        (0.0, 0.0),
        1.0,
    );
    let out = conform_areas(&base, &[], &[], &[lot], &[]).unwrap();
    for (x, y) in [
        (104.0, 104.0),
        (140.0, 140.0),
        (176.0, 176.0),
        (104.0, 176.0),
    ] {
        let got = shaped(&out, x, y);
        assert!((got - 101.0).abs() < 1e-4, "inside ({x}, {y}): {got}");
    }
    // every vertex within 1.5 cells (6 m) beyond the edge
    for (x, y) in [
        (96.0, 140.0),
        (184.0, 140.0),
        (140.0, 96.0),
        (140.0, 184.0),
        (96.0, 96.0),
        (184.0, 184.0),
    ] {
        let got = shaped(&out, x, y);
        assert!((got - 101.0).abs() < 1e-4, "shoulder ({x}, {y}): {got}");
    }
    // beyond the shoulder the original ground, and never below it
    assert!((shaped(&out, 88.0, 140.0) - 100.0).abs() < 1e-6);
    assert!(
        out.heightmap
            .h
            .iter()
            .all(|h| f64::from(*h) >= 100.0 - 1e-6)
    );
}

#[test]
fn a_lot_gets_its_capped_quadratic_and_is_not_bumpy() {
    // the smooth quadratic of the base, with 12 m bumps of ±0.5 m that the surface must remove
    let base = terrain(64, 4.0, |x, y| {
        100.0
            + (x - 30.0) * (x - 30.0) / 600.0
            + 0.5
                * (std::f64::consts::TAU * x / 12.0).sin()
                * (std::f64::consts::TAU * y / 12.0).sin()
    });
    let lot = lot_area("osm/way/1", ring(50.5, 60.5, 109.5, 100.5), Vec::new());
    let (assessed, surfaces) = assess(std::slice::from_ref(&lot), &base, &|_| false);
    assert!(matches!(
        assessed[0].outcome,
        Outcome::Smoothed {
            form: Form::Quadratic
        }
    ));
    assert_eq!(surfaces.len(), 1);
    assert!((surfaces[0].surface.lambda - 1.0).abs() < 1e-12);
    let patch = AreaPatch {
        surface: surfaces[0].clone(),
        rank: 0,
    };
    let out = conform_areas(&base, &[], &[], &[patch], &[]).unwrap();
    let mut inside: Vec<(i64, i64)> = Vec::new();
    let mut claimed = 0;
    for row in 0..64_u32 {
        for col in 0..64_u32 {
            let p = pos(&base, col, row);
            let inside_lot = contains(&lot, p);
            // inside the lot and in its 1.5-cell shoulder (6 m) are claimed at the target
            if !inside_lot && edge_distance(&lot, p) > 6.0 {
                continue;
            }
            claimed += 1;
            if inside_lot {
                inside.push((i64::from(col), i64::from(row)));
            }
            let ground = f64::from(base.at(col, row));
            let want = surfaces[0].target(p, ground);
            let got = f64::from(out.heightmap.at(col, row));
            assert!((got - want).abs() < 1e-4, "{p:?}: {got} vs {want}");
        }
    }
    assert!(inside.len() > 20);
    assert!(claimed > inside.len(), "{claimed} {}", inside.len());
    // the bound means something: the original ground is bumpier than 0.12 m
    let ground_at = |c: i64, r: i64| {
        let col = u32::try_from(c).ok()?;
        let row = u32::try_from(r).ok()?;
        Some(f64::from(base.at(col, row)))
    };
    let before = local_relief(&ground_at, &inside, 4.0).1;
    assert!(before > 0.12, "the base is too smooth: {before}");
    let relief = local_relief(&|c, r| shaped_at(&out, c, r), &inside, 4.0).1;
    assert!(relief <= 0.12 + 1e-6, "local relief {relief}");
}

#[test]
fn a_road_strip_through_a_lot_takes_its_own_height() {
    let base = terrain(64, 4.0, |_, _| 100.0);
    let lot = plane_patch(
        vec![(ring(100.0, 100.0, 180.0, 180.0), Vec::new())],
        (140.0, 140.0),
        101.0,
        (0.0, 0.0),
        1.0,
    );
    let street = road((100.0, 140.0), (180.0, 140.0), 100.5, 100.5, 3.5, 2);
    let out = conform_areas(&base, &[street], &[], &[lot], &[]).unwrap();
    for x in [104.0, 120.0, 156.0, 176.0] {
        let got = shaped(&out, x, 140.0);
        assert!((got - 100.5).abs() < 1e-4, "{x}: {got}");
    }
}

#[test]
fn a_hole_keeps_its_own_ground_beyond_its_shoulder() {
    let base = terrain(64, 4.0, |_, _| 100.0);
    // six cells across: its centre vertex is 12 m from its edge, beyond the 1.5 cell shoulder
    let hole = ring(124.0, 124.0, 148.0, 148.0);
    let lot = plane_patch(
        vec![(ring(100.0, 100.0, 180.0, 180.0), vec![hole])],
        (140.0, 140.0),
        101.0,
        (0.0, 0.0),
        1.0,
    );
    let out = conform_areas(&base, &[], &[], &[lot], &[]).unwrap();
    assert!(
        (shaped(&out, 136.0, 136.0) - 100.0).abs() < 1e-6,
        "the hole"
    );
    // 2 m into the hole, in its shoulder, and the lot beside it are shaped
    assert!((shaped(&out, 126.0, 136.0) - 101.0).abs() < 1e-4);
    assert!((shaped(&out, 104.0, 104.0) - 101.0).abs() < 1e-4);
}

#[test]
fn a_vertex_under_a_bridge_span_is_not_claimed_by_a_lot() {
    let base = terrain(120, 6.25, |_, _| 100.0);
    let mut road = strip(
        &[
            (300.0, 100.0, 100.0),
            (300.0, 300.0, 100.0),
            (300.0, 500.0, 100.0),
            (300.0, 700.0, 100.0),
        ],
        3.5,
        3,
    );
    road.bridge = vec![false, true, false];
    let lot = plane_patch(
        vec![(ring(250.0, 350.0, 350.0, 450.0), Vec::new())],
        (300.0, 400.0),
        101.0,
        (0.0, 0.0),
        1.0,
    );
    let out = conform_areas(&base, &[road], &[], &[lot], &[deck(300.0, 500.0)]).unwrap();
    // under the deck the bridge's ceiling wins
    let ceiling = shaped(&out, 300.0, 400.0);
    assert!(
        (ceiling - (100.0 - 0.25 - BRIDGE_AIR_M)).abs() < 1e-3,
        "{ceiling}"
    );
    // beside the deck the lot's surface is shaped
    let beside = shaped(&out, 350.0, 400.0);
    assert!((beside - 101.0).abs() < 1e-4, "{beside}");
}

#[test]
fn the_two_lots_of_one_group_meet_on_their_shared_edge() {
    let base = terrain(64, 4.0, |_, _| 100.0);
    let gradient = (0.03, 0.01);
    let lot = plane_patch(
        vec![
            (ring(100.0, 100.0, 180.0, 180.0), Vec::new()),
            (ring(180.0, 100.0, 260.0, 180.0), Vec::new()),
        ],
        (180.0, 140.0),
        101.0,
        gradient,
        1.0,
    );
    let target = |p: (f64, f64)| 101.0 + gradient.0 * (p.0 - 180.0) + gradient.1 * (p.1 - 140.0);
    let out = conform_areas(&base, &[], &[], &[lot], &[]).unwrap();
    // the vertices on the shared edge, and those one cell either side of it, are on the one surface
    for y in [104.0, 120.0, 140.0, 160.0, 176.0] {
        let on = shaped(&out, 180.0, y);
        assert!((on - target((180.0, y))).abs() < 1e-4, "edge ({y}): {on}");
        let west = shaped(&out, 176.0, y);
        let east = shaped(&out, 184.0, y);
        assert!((west - target((176.0, y))).abs() < 1e-4, "west {y}: {west}");
        assert!((east - target((184.0, y))).abs() < 1e-4, "east {y}: {east}");
        // no jump over the shared edge beyond the surface's own rise
        assert!((west - on).abs() <= 4.0 * gradient.0.abs() + 1e-4);
        assert!((east - on).abs() <= 4.0 * gradient.0.abs() + 1e-4);
    }
}

/// The `tests/bridge_ground.rs` fixture: flat ground at 100 on a 2 m cell, a level road along x = 100 with a deck
/// from y = 50 to y = 90.
fn bridge_ground_fixture() -> (Heightmap, RoadStrip, BridgeSpan) {
    let base = terrain(101, 2.0, |_, _| 100.0);
    let mut road = strip(
        &[
            (100.0, 0.0, 100.0),
            (100.0, 50.0, 100.0),
            (100.0, 90.0, 100.0),
            (100.0, 190.0, 100.0),
        ],
        3.5,
        3,
    );
    road.bridge = vec![false, true, false];
    let span = BridgeSpan {
        stretch: "deck".to_string(),
        first: (100.0, 50.0, 100.0),
        second: (100.0, 90.0, 100.0),
        shape: PartShape::level(40.0, 3.0, 3.5),
        own_strips: Vec::new(),
    };
    (base, road, span)
}

/// A stable hash of the shaped heights (FNV-1a over the bits of every height).
fn height_hash(hm: &Heightmap) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for height in &hm.h {
        for byte in height.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

#[test]
fn conforming_without_areas_keeps_the_bridge_ground_fixture_as_before() {
    // the brief's bit-for-bit guard, against a fixed golden rather than against the wrapper of the new function.
    // `GOLDEN` was recorded on the no-areas path after the refactor; the pre-refactor code no longer exists as a
    // separate function, and the unchanged `bridge_ground` suite pins that path to it (see the report).
    const GOLDEN: u64 = 2_072_355_618_203_790_015;
    let (base, road, span) = bridge_ground_fixture();
    let out = conform_areas(
        &base,
        std::slice::from_ref(&road),
        &[],
        &[],
        std::slice::from_ref(&span),
    )
    .unwrap();
    assert_eq!(height_hash(&out.heightmap), GOLDEN);
    assert!(
        out.conflicts.is_empty(),
        "a conflict under the deck: {:?}",
        out.conflicts
    );
}

#[test]
fn no_claimed_vertex_moves_more_than_the_guard() {
    let base = terrain(64, 4.0, |_, _| 100.0);
    // 4.5 m above the ground: more than the guard allows, and the surface's lambda is 1
    let lot = plane_patch(
        vec![(ring(100.0, 100.0, 180.0, 180.0), Vec::new())],
        (140.0, 140.0),
        104.5,
        (0.0, 0.0),
        1.0,
    );
    let out = conform_areas(&base, &[], &[], &[lot], &[]).unwrap();
    assert!(out.heightmap.h.iter().all(|h| {
        let move_m = f64::from(*h) - 100.0;
        move_m.abs() <= LOT_MAX_MOVE_M + 1e-4
    }));
    // inside the lot the claim is clamped, not dropped
    let middle = shaped(&out, 140.0, 140.0);
    assert!((middle - (100.0 + LOT_MAX_MOVE_M)).abs() < 1e-4, "{middle}");
    // the shoulder would need 4.5 m: it is left to the fill slope from the claimed ground, not dropped to the
    // surface, and beyond the shoulder the ground is untouched
    let shoulder = shaped(&out, 96.0, 140.0);
    assert!(shoulder <= 100.0 + LOT_MAX_MOVE_M, "{shoulder}");
    assert!(
        shoulder < 103.5,
        "the shoulder took the lot's height: {shoulder}"
    );
    assert!(
        (shaped(&out, 92.0, 140.0) - 100.0).abs() < 1e-6,
        "beyond the shoulder"
    );
}

#[test]
fn a_street_beside_a_lot_that_is_a_little_above_it_leaves_no_step() {
    let base = terrain(64, 4.0, |_, _| 100.0);
    let lot = plane_patch(
        vec![(ring(100.0, 100.0, 180.0, 180.0), Vec::new())],
        (140.0, 140.0),
        100.5,
        (0.0, 0.0),
        1.0,
    );
    // a street whose centre line is two metres outside the lot's south edge; its width reaches 1.5 m into the lot
    let street = road((80.0, 98.0), (200.0, 98.0), 100.3, 100.3, 3.5, 2);
    let out = conform_areas(&base, &[street], &[], &[lot], &[]).unwrap();
    // inside the street's width and on the lot's edge beside it, the street's height (0.2 m is a compatible tie,
    // which the road's higher rank wins)
    for (x, y) in [(104.0, 96.0), (104.0, 100.0), (140.0, 96.0), (140.0, 100.0)] {
        let got = shaped(&out, x, y);
        assert!((got - 100.3).abs() < 1e-4, "({x}, {y}): {got}");
    }
    // deeper in the lot its surface
    let inside = shaped(&out, 140.0, 112.0);
    assert!((inside - 100.5).abs() < 1e-4, "{inside}");
    // no vertex jumps by more than the conflict tolerance from one cell to the next
    for y in [96.0, 100.0, 104.0, 108.0, 112.0] {
        let here = shaped(&out, 140.0, y);
        let next = shaped(&out, 140.0, y + 4.0);
        assert!((next - here).abs() <= 0.3, "y {y}: {here} -> {next}");
    }
    assert!(out.conflicts.is_empty(), "{:?}", out.conflicts);
}

#[test]
fn a_street_beside_a_lot_that_is_well_above_it_is_reported_at_the_edge() {
    let base = terrain(64, 4.0, |_, _| 100.0);
    let lot = plane_patch(
        vec![(ring(100.0, 100.0, 180.0, 180.0), Vec::new())],
        (140.0, 140.0),
        100.5,
        (0.0, 0.0),
        1.0,
    );
    // 0.8 m apart: more than the conflict tolerance, so the lot (the nearer edge) wins the ring and the ground
    // steps at the street's edge. Nothing pulls the lot or the street together; the step is reported.
    let street = road((80.0, 98.0), (200.0, 98.0), 99.7, 99.7, 3.5, 2);
    let out = conform_areas(&base, &[street], &[], std::slice::from_ref(&lot), &[]).unwrap();
    // inside the street's width it is the street's height; in the ring the lot's surface
    assert!((shaped(&out, 140.0, 96.0) - 99.7).abs() < 1e-4);
    assert!((shaped(&out, 140.0, 104.0) - 100.5).abs() < 1e-4);
    // the step is at least the conflict tolerance between neighbouring vertices of the edge
    assert!(shaped(&out, 140.0, 104.0) - shaped(&out, 140.0, 100.0) > 0.3);
    // and it is reported at the lot's edge beside the street
    assert!(
        out.conflicts.iter().any(|conflict| {
            conflict.height_difference_m > 0.3
                && (96.0..=108.0).contains(&conflict.at.1)
                && (96.0..=184.0).contains(&conflict.at.0)
        }),
        "{:?}",
        out.conflicts
    );
}

#[test]
fn lots_that_are_not_numbers_are_not_claimed() {
    let base = terrain(64, 4.0, |_, _| 100.0);
    let mut broken = plane_patch(
        vec![(ring(100.0, 100.0, 180.0, 180.0), Vec::new())],
        (140.0, 140.0),
        101.0,
        (0.0, 0.0),
        1.0,
    );
    broken.surface.polygons[0].0[1].0 = f64::NAN;
    // no ring at all, a ring of one point, and a surface whose height is not a number
    let empty = plane_patch(Vec::new(), (140.0, 140.0), 101.0, (0.0, 0.0), 1.0);
    let one_point = plane_patch(
        vec![(vec![(140.0, 140.0)], Vec::new())],
        (140.0, 140.0),
        101.0,
        (0.0, 0.0),
        1.0,
    );
    let not_a_height = plane_patch(
        vec![(ring(100.0, 100.0, 180.0, 180.0), Vec::new())],
        (140.0, 140.0),
        f64::NAN,
        (0.0, 0.0),
        1.0,
    );
    let out = conform_areas(
        &base,
        &[],
        &[],
        &[broken, empty, one_point, not_a_height],
        &[],
    )
    .unwrap();
    assert_eq!(out.heightmap.h, base.h);
}

#[test]
fn a_vertex_without_ground_under_a_lot_is_not_claimed() {
    let base = terrain(64, 4.0, |x, y| {
        if (x - 140.0).abs() < 1e-9 && (y - 140.0).abs() < 1e-9 {
            f64::NAN
        } else {
            100.0
        }
    });
    let lot = plane_patch(
        vec![(ring(100.0, 100.0, 180.0, 180.0), Vec::new())],
        (140.0, 140.0),
        101.0,
        (0.0, 0.0),
        1.0,
    );
    let out = conform_areas(&base, &[], &[], &[lot], &[]).unwrap();
    assert!(f64::from(out.heightmap.at(35, 35)).is_nan());
    assert!((shaped(&out, 104.0, 104.0) - 101.0).abs() < 1e-4);
}

#[test]
fn a_road_lying_on_a_cross_sloping_lot_is_no_conflict() {
    // an 8 % cross slope: the lot's target at the road's edge is 0.28 m above the level road, under the tolerance,
    // although it is 1.03 m at the outer edge of the road's ring. The cross slope of the ground must not count.
    let base = terrain(64, 4.0, |_, y| 100.0 + 0.08 * (y - 140.0));
    let lot = plane_patch(
        vec![(ring(100.0, 100.0, 180.0, 180.0), Vec::new())],
        (140.0, 140.0),
        100.0,
        (0.0, 0.08),
        1.0,
    );
    let on = road((100.0, 140.0), (180.0, 140.0), 100.0, 100.0, 3.5, 2);
    let out = conform_areas(&base, &[on], &[], std::slice::from_ref(&lot), &[]).unwrap();
    assert!(
        out.conflicts.is_empty(),
        "a road on the lot reports: {:?}",
        out.conflicts
    );
    // half a metre below the surface: the step at the road's edge is 0.78 m and is reported
    let low = road((100.0, 140.0), (180.0, 140.0), 99.5, 99.5, 3.5, 2);
    let out = conform_areas(&base, &[low], &[], std::slice::from_ref(&lot), &[]).unwrap();
    assert!(
        out.conflicts.iter().any(|conflict| {
            conflict.height_difference_m > 0.3 && (conflict.at.1 - 140.0).abs() >= 3.5 - 1e-6
        }),
        "{:?}",
        out.conflicts
    );
}

#[test]
fn a_road_that_parts_from_a_lot_in_two_places_gives_two_records() {
    // one group of two members, 300 m apart; a straight road at 100 over their surface at 100.5. The conflict is
    // counted by place: one record where each member is crossed, and one for the vertices within a member.
    let base = terrain(160, 4.0, |_, _| 100.0);
    let lot = plane_patch(
        vec![
            (ring(110.0, 250.0, 140.0, 350.0), Vec::new()),
            (ring(460.0, 250.0, 490.0, 350.0), Vec::new()),
        ],
        (300.0, 300.0),
        100.5,
        (0.0, 0.0),
        1.0,
    );
    let road = road((0.0, 310.0), (600.0, 310.0), 100.0, 100.0, 3.5, 2);
    let out = conform_areas(&base, &[road], &[], std::slice::from_ref(&lot), &[]).unwrap();
    assert_eq!(out.conflicts.len(), 2, "{:?}", out.conflicts);
    assert!(out.conflicts.iter().all(|c| c.height_difference_m > 0.3));
    let first = out
        .conflicts
        .iter()
        .filter(|c| (100.0..150.0).contains(&c.at.0))
        .count();
    let second = out
        .conflicts
        .iter()
        .filter(|c| (450.0..500.0).contains(&c.at.0))
        .count();
    assert_eq!((first, second), (1, 1), "{:?}", out.conflicts);
}

#[test]
fn a_road_lot_edge_point_without_ground_is_skipped() {
    // the ground is missing at a vertex between the road and the ring vertex: the edge point there has no height and
    // the pair is not compared, but the rest of the road and lot are (the test is that this does not panic)
    let base = terrain(64, 4.0, |x, y| {
        if (x - 140.0).abs() < 1e-9 && (y - 144.0).abs() < 1e-9 {
            f64::NAN
        } else {
            100.0
        }
    });
    let lot = plane_patch(
        vec![(ring(100.0, 100.0, 180.0, 180.0), Vec::new())],
        (140.0, 140.0),
        100.5,
        (0.0, 0.0),
        1.0,
    );
    let road = road((100.0, 140.0), (180.0, 140.0), 100.0, 100.0, 3.5, 2);
    let out = conform_areas(&base, &[road], &[], std::slice::from_ref(&lot), &[]).unwrap();
    assert!(!out.conflicts.is_empty(), "{:?}", out.conflicts);
    assert!(
        out.conflicts
            .iter()
            .all(|c| c.at.0.is_finite() && c.at.1.is_finite())
    );
}

#[test]
fn two_level_patches_at_different_heights_report_the_worst_place() {
    // two level patches that overlap with opposite tilts: their difference grows across the overlap, so the clash
    // is recorded at the vertex where it is worst (not at the first one)
    let base = terrain(64, 4.0, |_, _| 100.0);
    let patches = [
        LevelPatch {
            at: (140.0, 140.0),
            radius: 24.0,
            height: 100.0,
            gradient: (0.05, 0.0),
            inner_radius: 0.0,
            rank: 2,
        },
        LevelPatch {
            at: (140.0, 140.0),
            radius: 24.0,
            height: 100.0,
            gradient: (-0.05, 0.0),
            inner_radius: 0.0,
            rank: 3,
        },
    ];
    let out = conform(&base, &[], &patches).unwrap();
    assert_eq!(out.conflicts.len(), 1, "{:?}", out.conflicts);
    let worst = &out.conflicts[0];
    assert!(
        (worst.at.0 - 140.0).abs() >= 16.0,
        "not at the edge of the overlap: {worst:?}"
    );
    assert!(
        (worst.height_difference_m - 2.0 * 0.05 * (worst.at.0 - 140.0).abs()).abs() < 0.3,
        "{worst:?}"
    );
}

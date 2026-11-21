// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

#![no_main]

use libfuzzer_sys::fuzz_target;
use naves_core::reroute::raster::{
    Clearance, Obstacles, Wall, WallKind, chamfer, crossings_away, exempt_samples, samples,
};

/// Records of 17 bytes: x and y as little-endian `f64`, one kind byte: 0 road, 1 original line, 2 wall line,
/// 3 probe point, anything else a vertex of a water polygon.
fn decode(data: &[u8]) -> [Vec<(f64, f64)>; 5] {
    let mut lists: [Vec<(f64, f64)>; 5] = Default::default();
    for c in data.as_chunks::<17>().0 {
        let (Some(x), Some(y), Some(&kind)) = (c.get(0..8), c.get(8..16), c.get(16)) else {
            continue;
        };
        let (Ok(x), Ok(y)) = (x.try_into(), y.try_into()) else {
            continue;
        };
        let p = (f64::from_le_bytes(x), f64::from_le_bytes(y));
        if let Some(list) = lists.get_mut(usize::from(kind.min(4))) {
            list.push(p);
        }
    }
    lists
}

/// Even-odd point in polygon.
fn inside(poly: &[(f64, f64)], p: (f64, f64)) -> bool {
    let mut odd = false;
    for (i, a) in poly.iter().enumerate() {
        let Some(b) = poly.get((i + 1) % poly.len()) else {
            continue;
        };
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0 {
            odd = !odd;
        }
    }
    odd
}

fuzz_target!(|data: &[u8]| {
    let [road, original, wall, probes, polygon] = decode(data);
    let mut o = Obstacles::new((-50.0, -50.0), (250.0, 250.0), 4.0, 200.0, 3.0, 20_000);
    o.stamp_water(&|p| inside(&polygon, p));
    o.stamp_road(&road, 6.0, &|p| p.0 < 100.0);
    o.set_original(&original);
    o.finish();
    let clear = Clearance {
        water_m: 5.0,
        road_m: 12.0,
    };
    for pair in probes.windows(2) {
        if let [a, b] = *pair {
            let _ = o.segment(a, b, clear);
            let w = Wall {
                line: wall.clone(),
                allowed: 0,
                kind: WallKind::Stream,
            };
            let _ = w.blocks(a, b);
            let _ = o.stray_m(a);
            let _ = o.water_distance_m(b);
            let _ = o.road_distance_m(b);
        }
    }
    let _ = o.water_cap_m(&original);
    let _ = crossings_away(&road, &wall, [(0.0, 0.0), (1.0, 1.0)], 10.0);
    let _ = exempt_samples(&road, &original, 20.0, 10.0);
    let _ = samples(&wall);
    let mask: Vec<bool> = data.iter().map(|b| b & 1 != 0).collect();
    let cols = usize::from(data.first().copied().unwrap_or(1) % 32);
    let _ = chamfer(&mask, cols, mask.len() / cols.max(1), 2.0);
});

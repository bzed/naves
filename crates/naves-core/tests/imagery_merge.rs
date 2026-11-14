// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Behaviour of the merge of overlapping imagery: colour matching, who wins, feathering.

#![expect(
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::indexing_slicing,
    clippy::many_single_char_names,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::heightmap::MapGeo;
use naves_core::imagery::{Composite, ImageryRaster, MapRect, MergeReport, merge};
use naves_core::project::Project;

/// 3200 real metres, scale 0.5: a map of 1600 m with 128 land cells of 12.5 map metres (25 real metres).
fn project() -> Project {
    let mut p = Project::new("merge test", 47.0, 12.0, 3200.0);
    p.grid.terrain_size = 1024;
    p.grid.land_size = 128;
    p
}

const MAP: f64 = 1600.0;
/// One land cell in map metres.
const CELL: f64 = MAP / 128.0;

/// About 8 real metres.
const COARSE: f64 = 1e-4;
/// About 2 real metres.
const FINE: f64 = 2e-5;

/// Value noise in -1..=1 on a lattice of about 1.4 m.
fn noise(lat: f64, lon: f64, seed: u64) -> f32 {
    let (ix, iy) = ((lat * 8e4).floor() as i64, (lon * 8e4).floor() as i64);
    let mut h = (ix as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h % 2001) as f32 / 1000.0 - 1.0
}

/// A smooth scene colour, the same for everyone who looks at that spot.
fn scene(lat: f64, lon: f64) -> [f32; 3] {
    [
        100.0 + 15.0 * (lat * 600.0).sin() as f32,
        90.0 + 12.0 * (lon * 500.0).cos() as f32,
        80.0 + 10.0 * ((lat + lon) * 400.0).sin() as f32,
    ]
}

fn scaled(c: [f32; 3], gain: f32) -> [f32; 3] {
    [c[0] * gain, c[1] * gain, c[2] * gain]
}

fn plus(c: [f32; 3], d: f32) -> [f32; 3] {
    [c[0] + d, c[1] + d, c[2] + d]
}

fn byte(c: [f32; 3]) -> [u8; 3] {
    [
        c[0].round().clamp(0.0, 255.0) as u8,
        c[1].round().clamp(0.0, 255.0) as u8,
        c[2].round().clamp(0.0, 255.0) as u8,
    ]
}

/// The lat/lon box of a rectangle given in fractions of the map (x east, y north).
fn region(p: &Project, (x0, y0, x1, y1): (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    let geo = MapGeo::new(p).unwrap();
    let (mut west, mut south, mut east, mut north) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
        let (lat, lon) = geo.to_geo(x * MAP, y * MAP).unwrap();
        west = west.min(lon);
        east = east.max(lon);
        south = south.min(lat);
        north = north.max(lat);
    }
    (west, south, east, north)
}

/// A raster over a part of the map; `colour(lat, lon)` is `None` where it has no data.
fn raster(
    p: &Project,
    frac: (f64, f64, f64, f64),
    pixel_deg: f64,
    colour: impl Fn(f64, f64) -> Option<[f32; 3]>,
) -> ImageryRaster {
    let (west, south, east, north) = region(p, frac);
    ImageryRaster::from_fn(west, south, east, north, pixel_deg, |lat, lon| {
        colour(lat, lon).map(byte)
    })
}

const ALL: (f64, f64, f64, f64) = (-0.05, -0.05, 1.05, 1.05);

fn paint_at(c: &Composite, p: &Project, x: f64, y: f64) -> Option<[u8; 3]> {
    let rect = MapRect {
        west: x - 0.25,
        north: y + 0.25,
        step_x: 0.5,
        step_y: 0.5,
        width: 1,
        height: 1,
    };
    c.paint(p, &rect).unwrap()[0]
}

/// The colour at (x, y) of the map metres, 3 channels as f32.
fn paint_f(c: &Composite, p: &Project, x: f64, y: f64) -> Option<[f32; 3]> {
    paint_at(c, p, x, y).map(|b| [f32::from(b[0]), f32::from(b[1]), f32::from(b[2])])
}

fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() <= tol)
}

fn lat_lon(p: &Project, x: f64, y: f64) -> (f64, f64) {
    MapGeo::new(p).unwrap().to_geo(x, y).unwrap()
}

#[test]
fn one_source_passes_through() {
    let p = project();
    let r = raster(&p, ALL, COARSE, |lat, lon| Some(scene(lat, lon)));
    let (c, report) = merge(vec![r.clone()], &p).unwrap();
    assert_eq!(report.reference, Some(0));
    let sampler = r.sampler().unwrap();
    let rect = MapRect {
        west: 700.0,
        north: 900.0,
        step_x: 0.5,
        step_y: 0.5,
        width: 16,
        height: 16,
    };
    let out = c.paint(&p, &rect).unwrap();
    for row in 0..16_u32 {
        for col in 0..16_u32 {
            let (lat, lon) = lat_lon(
                &p,
                700.0 + f64::midpoint(f64::from(col), 0.5),
                900.0 - f64::midpoint(f64::from(row), 0.5),
            );
            let want = sampler.rgb_at(lat, lon).map(byte);
            assert_eq!(out[(row * 16 + col) as usize], want, "{row} {col}");
        }
    }
}

#[test]
fn whole_area_source_is_the_reference() {
    let p = project();
    let full = raster(&p, ALL, COARSE, |lat, lon| Some(scene(lat, lon)));
    let part = raster(&p, (0.2, 0.2, 0.6, 0.6), FINE, |lat, lon| {
        Some(scaled(plus(scene(lat, lon), 8.0 * noise(lat, lon, 1)), 1.4))
    });
    // the partial one is listed first: the full one is still the reference
    let (c, report) = merge(vec![part, full.clone()], &p).unwrap();
    assert_eq!(report.reference, Some(1));
    let sampler = full.sampler().unwrap();
    let (mut sum_merged, mut sum_full, mut n) = (0.0_f64, 0.0_f64, 0.0_f64);
    for i in 0..30 {
        for j in 0..30 {
            let (x, y) = (
                MAP * (0.25 + 0.3 * f64::from(i) / 30.0),
                MAP * (0.25 + 0.3 * f64::from(j) / 30.0),
            );
            let merged = paint_f(&c, &p, x, y).unwrap();
            let (lat, lon) = lat_lon(&p, x, y);
            let reference = sampler.rgb_at(lat, lon).unwrap();
            sum_merged += f64::from(merged[0] + merged[1] + merged[2]) / 3.0;
            sum_full += f64::from(reference[0] + reference[1] + reference[2]) / 3.0;
            n += 1.0;
        }
    }
    assert!(
        ((sum_merged - sum_full) / n).abs() < 2.0,
        "merged mean {} against the full image's {}",
        sum_merged / n,
        sum_full / n
    );
}

#[test]
fn exposure_step_disappears() {
    let p = project();
    // two flights with a different exposure, overlapping between 40 % and 60 % of the width
    let a = raster(&p, (-0.05, -0.05, 0.6, 1.05), FINE, |lat, lon| {
        Some(plus(scene(lat, lon), 6.0 * noise(lat, lon, 1)))
    });
    let b = raster(&p, (0.4, -0.05, 1.05, 1.05), FINE, |lat, lon| {
        Some(scaled(plus(scene(lat, lon), 6.0 * noise(lat, lon, 2)), 1.4))
    });
    let (c, report) = merge(vec![a, b], &p).unwrap();
    assert_eq!(report.reference, Some(0));
    let gain = report.sources[1].median_gain;
    assert!(
        (gain[0] - 1.0 / 1.4).abs() < 0.03,
        "the second flight is corrected by {gain:?}"
    );
    let y = MAP * 0.5;
    let mut worst: f32 = 0.0;
    let mut worst_ideal: f32 = 0.0;
    let mut prev = paint_f(&c, &p, MAP * 0.35, y).unwrap();
    let mut x = MAP * 0.35;
    while x < MAP * 0.65 {
        x += 1.0;
        let now = paint_f(&c, &p, x, y).unwrap();
        worst = worst.max((0..3).map(|i| (now[i] - prev[i]).abs()).fold(0.0, f32::max));
        let (lat0, lon0) = lat_lon(&p, x - 1.0, y);
        let (lat1, lon1) = lat_lon(&p, x, y);
        let (i0, i1) = (
            plus(scene(lat0, lon0), 6.0 * noise(lat0, lon0, 1)),
            plus(scene(lat1, lon1), 6.0 * noise(lat1, lon1, 1)),
        );
        worst_ideal = worst_ideal.max((0..3).map(|i| (i1[i] - i0[i]).abs()).fold(0.0, f32::max));
        prev = now;
    }
    assert!(
        worst <= worst_ideal + 3.0,
        "largest step {worst}, the scene's own {worst_ideal}"
    );
}

#[test]
fn a_much_brighter_flight_is_matched() {
    // aerial photographs against a satellite scene: 4 times as bright, which a gain of 0.25 undoes
    let p = project();
    let reference = raster(&p, ALL, COARSE, |lat, lon| {
        Some(scaled(scene(lat, lon), 0.25))
    });
    let bright = raster(&p, (0.2, 0.2, 0.8, 0.8), 3e-5, |lat, lon| {
        Some(scene(lat, lon))
    });
    let (c, report) = merge(vec![reference.clone(), bright], &p).unwrap();
    assert!(report.sources[1].median_gain[0] < 0.3, "{report:?}");
    let sampler = reference.sampler().unwrap();
    for i in 0..10 {
        for j in 0..10 {
            let (x, y) = (
                MAP * (0.35 + 0.3 * f64::from(i) / 9.0),
                MAP * (0.35 + 0.3 * f64::from(j) / 9.0),
            );
            let (lat, lon) = lat_lon(&p, x, y);
            let want = sampler.rgb_at(lat, lon).unwrap();
            let got = paint_f(&c, &p, x, y).unwrap();
            assert!(close(got, want, 4.0), "{x} {y}: {got:?} against {want:?}");
        }
    }
}

#[test]
fn neutral_ground_stays_neutral() {
    // Where the aerial photograph and the satellite scene relate differently by kind of ground (forest and rock),
    // a gain per colour channel from the typical ground would tint the rest: here the forest is 3.3, 2.2 and
    // 10 times as bright in the photograph and the rock 2.9 times in all channels. The rock must stay grey.
    let p = project();
    let in_rock = |lat: f64, lon: f64| {
        let (x, _) = MapGeo::new(&project()).unwrap().to_map(lat, lon).unwrap();
        x > MAP * 0.1 && x < MAP * 0.3
    };
    let reference = raster(&p, ALL, COARSE, |lat, lon| {
        Some(if in_rock(lat, lon) {
            [60.0, 60.0, 60.0]
        } else {
            [30.0, 45.0, 10.0]
        })
    });
    let photo = raster(&p, ALL, 3e-5, |lat, lon| {
        Some(if in_rock(lat, lon) {
            [170.0, 170.0, 170.0]
        } else {
            [100.0, 100.0, 100.0]
        })
    });
    let (c, _) = merge(vec![reference, photo], &p).unwrap();
    for i in 0..8 {
        let (x, y) = (
            MAP * (0.14 + 0.12 * f64::from(i) / 7.0),
            MAP * (0.3 + 0.05 * f64::from(i)),
        );
        let got = paint_f(&c, &p, x, y).unwrap();
        let spread = got[0].max(got[1]).max(got[2]) - got[0].min(got[1]).min(got[2]);
        assert!(spread <= 3.0, "the rock is tinted at ({x}, {y}): {got:?}");
    }
}

#[test]
fn falloff_inside_a_photo_is_evened_out() {
    let p = project();
    let reference = raster(&p, ALL, COARSE, |lat, lon| Some(scene(lat, lon)));
    let (west, south, east, north) = region(&p, (0.2, 0.2, 0.8, 0.8));
    let (clat, clon) = (f64::midpoint(south, north), f64::midpoint(west, east));
    let radius = (north - south) / 2.0;
    let photo = raster(&p, (0.2, 0.2, 0.8, 0.8), 3e-5, |lat, lon| {
        let d = ((lat - clat).powi(2) + ((lon - clon) * 0.68).powi(2)).sqrt() / radius;
        Some(scaled(scene(lat, lon), 1.0 + 0.3 * d.min(1.0) as f32))
    });
    let (c, _) = merge(vec![reference.clone(), photo], &p).unwrap();
    let sampler = reference.sampler().unwrap();
    for i in 0..12 {
        for j in 0..12 {
            let (x, y) = (
                MAP * (0.3 + 0.4 * f64::from(i) / 11.0),
                MAP * (0.3 + 0.4 * f64::from(j) / 11.0),
            );
            let (lat, lon) = lat_lon(&p, x, y);
            let want = sampler.rgb_at(lat, lon).unwrap();
            let got = paint_f(&c, &p, x, y).unwrap();
            assert!(close(got, want, 4.0), "{x} {y}: {got:?} against {want:?}");
        }
    }
}

fn std_dev(values: &[f32]) -> f32 {
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32).sqrt()
}

#[test]
fn detail_survives() {
    let p = project();
    let reference = raster(&p, ALL, COARSE, |lat, lon| Some(scene(lat, lon)));
    let sharp = raster(&p, (0.3, 0.3, 0.7, 0.7), FINE, |lat, lon| {
        Some(scaled(
            plus(scene(lat, lon), 25.0 * noise(lat, lon, 3)),
            1.4,
        ))
    });
    let own = sharp.sampler().unwrap();
    let (c, _) = merge(vec![reference, sharp.clone()], &p).unwrap();
    let (mut merged, mut ideal) = (Vec::new(), Vec::new());
    let y = MAP * 0.5;
    let mut x = MAP * 0.42;
    while x < MAP * 0.58 {
        merged.push(paint_f(&c, &p, x, y).unwrap()[0]);
        let (lat, lon) = lat_lon(&p, x, y);
        // what the sharp image shows, with the exposure of the reference
        ideal.push(own.rgb_at(lat, lon).unwrap()[0] / 1.4);
        x += 0.3;
    }
    let (sm, si) = (std_dev(&merged), std_dev(&ideal));
    assert!(
        (sm / si - 1.0).abs() < 0.05,
        "spread {sm} after the correction, {si} expected"
    );
}

#[test]
fn sharpest_wins() {
    let p = project();
    let reference = raster(&p, ALL, COARSE, |lat, lon| Some(scene(lat, lon)));
    let sharp = raster(&p, (0.3, 0.3, 0.7, 0.7), FINE, |lat, lon| {
        Some(plus(scene(lat, lon), 25.0 * noise(lat, lon, 3)))
    });
    let (c, _) = merge(vec![sharp, reference], &p).unwrap();
    let variation = |x0: f64| {
        let steps: Vec<f32> = (0..200)
            .map(|i| paint_f(&c, &p, x0 + f64::from(i) * 0.3, MAP * 0.5).unwrap()[0])
            .collect();
        steps.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / 199.0
    };
    let inside = variation(MAP * 0.48);
    let outside = variation(MAP * 0.1);
    assert!(
        inside > 10.0 * outside.max(0.2),
        "pixel to pixel change {inside} in the sharp image, {outside} in the coarse one"
    );
}

/// RMS difference between the merged image and the smooth scene in a band of the map.
fn band_rms(c: &Composite, p: &Project, x0: f64, x1: f64) -> f32 {
    let mut sum = 0.0_f32;
    let mut n = 0.0_f32;
    let mut y = MAP * 0.4;
    while y < MAP * 0.6 {
        let mut x = x0;
        while x < x1 {
            let (lat, lon) = lat_lon(p, x, y);
            let got = paint_f(c, p, x, y).unwrap()[0];
            sum += (got - scene(lat, lon)[0]).powi(2);
            n += 1.0;
            x += 0.7;
        }
        y += 1.3;
    }
    (sum / n).sqrt()
}

#[test]
fn edge_is_feathered() {
    let p = project();
    let reference = raster(&p, ALL, COARSE, |lat, lon| Some(scene(lat, lon)));
    // a sharp image whose western edge is at 30 % of the width; its noise has the scene's mean
    let sharp = raster(&p, (0.3, 0.2, 0.9, 0.8), FINE, |lat, lon| {
        Some(plus(scene(lat, lon), 25.0 * noise(lat, lon, 3)))
    });
    let (c, _) = merge(vec![reference, sharp], &p).unwrap();
    // bands of one cell width, going east from well outside the image to deep inside
    let start = MAP * 0.3 - 3.0 * CELL;
    let bands: Vec<f32> = (0..14)
        .map(|k| {
            let x0 = start + f64::from(k) * CELL;
            band_rms(&c, &p, x0, x0 + CELL)
        })
        .collect();
    let plateau = *bands.last().unwrap();
    assert!(plateau > 8.0, "the noise shows deep inside: {bands:?}");
    for pair in bands.windows(2) {
        assert!(
            pair[1] >= pair[0] - 0.12 * plateau,
            "the mix grows monotonically: {bands:?}"
        );
    }
    let ramp = bands
        .iter()
        .filter(|&&b| b > 0.06 * plateau && b < 0.94 * plateau)
        .count();
    assert!(
        (3..=6).contains(&ramp),
        "the transition takes about four cells, not {ramp}: {bands:?}"
    );
}

#[test]
fn equals_first_listed_on_top() {
    let p = project();
    let make = |seed: u64| {
        raster(&p, ALL, FINE, move |lat, lon| {
            Some(plus(scene(lat, lon), 20.0 * noise(lat, lon, seed)))
        })
    };
    let (a, b) = (make(1), make(2));
    let (sa, sb) = (a.sampler().unwrap(), b.sampler().unwrap());
    for (swap, listed) in [
        (false, vec![a.clone(), b.clone()]),
        (true, vec![b.clone(), a.clone()]),
    ] {
        let (c, _) = merge(listed, &p).unwrap();
        let top = if swap { &sb } else { &sa };
        for i in 0..20 {
            let (x, y) = (
                MAP * 0.4 + f64::from(i) * 1.7,
                MAP * 0.5 + f64::from(i) * 0.9,
            );
            let (lat, lon) = lat_lon(&p, x, y);
            let want = top.rgb_at(lat, lon).unwrap();
            let got = paint_f(&c, &p, x, y).unwrap();
            assert!(
                close(got, want, 3.0),
                "swap {swap}: {got:?} against {want:?}"
            );
        }
    }
}

#[test]
fn sharper_than_the_map_counts_as_equal() {
    // the map's own pixel is about 0.42 real metres: 0.2 m and 0.4 m are both sharper
    let p = project();
    let region_ = (0.35, 0.35, 0.65, 0.65);
    let make = |pixel_deg: f64, seed: u64| {
        raster(&p, region_, pixel_deg, move |lat, lon| {
            Some(plus(scene(lat, lon), 20.0 * noise(lat, lon, seed)))
        })
    };
    let (fine, sharp) = (make(3.6e-6, 7), make(1.8e-6, 8));
    // reduce the work: only look at the middle
    let (sf, ss) = (fine.sampler().unwrap(), sharp.sampler().unwrap());
    assert!(fine.pixel_m() < 0.45 && sharp.pixel_m() < fine.pixel_m());
    for (swap, listed) in [
        (false, vec![fine.clone(), sharp.clone()]),
        (true, vec![sharp.clone(), fine.clone()]),
    ] {
        let (c, _) = merge(listed, &p).unwrap();
        let top = if swap { &ss } else { &sf };
        for i in 0..20 {
            let (x, y) = (
                MAP * 0.48 + f64::from(i) * 0.9,
                MAP * 0.5 + f64::from(i) * 0.4,
            );
            let (lat, lon) = lat_lon(&p, x, y);
            let want = top.rgb_at(lat, lon).unwrap();
            let got = paint_f(&c, &p, x, y).unwrap();
            assert!(
                close(got, want, 3.0),
                "swap {swap}: {got:?} against {want:?}"
            );
        }
    }
}

fn block(lat: f64, lon: f64, p: &Project) -> bool {
    let (x0, y0) = (MAP * 0.5, MAP * 0.5);
    let (blat0, blon0) = lat_lon(p, x0, y0);
    let (blat1, blon1) = lat_lon(p, x0 + 6.0 * CELL, y0 + 6.0 * CELL);
    (blat0..blat1).contains(&lat) && (blon0..blon1).contains(&lon)
}

#[test]
fn changed_content_leaves_no_stamp() {
    let p = project();
    let reference = raster(&p, ALL, COARSE, |lat, lon| Some(scene(lat, lon)));
    let second = |with_block: bool| {
        raster(&p, (0.1, 0.1, 0.9, 0.9), 3e-5, |lat, lon| {
            let gain = if with_block && block(lat, lon, &p) {
                3.6
            } else {
                1.2
            };
            Some(scaled(scene(lat, lon), gain))
        })
    };
    let (with, report) = merge(vec![reference.clone(), second(true)], &p).unwrap();
    let (without, _) = merge(vec![reference, second(false)], &p).unwrap();
    assert!(report.sources[1].changed_share > 0.0, "{report:?}");
    // a ring 3 to 4 cells outside the block
    let (lo, hi) = (MAP * 0.5 - 3.5 * CELL, MAP * 0.5 + 9.5 * CELL);
    for i in 0..=20 {
        let t = lo + (hi - lo) * f64::from(i) / 20.0;
        for (x, y) in [(t, lo), (t, hi), (lo, t), (hi, t)] {
            let got = paint_f(&with, &p, x, y).unwrap();
            let want = paint_f(&without, &p, x, y).unwrap();
            assert!(
                close(got, want, 3.0),
                "({x}, {y}): {got:?} against {want:?}"
            );
        }
    }
}

/// A block of 5.3 cells that does not follow the grid of cells.
fn off_grid_block(lat: f64, lon: f64, p: &Project) -> bool {
    let (x0, y0) = (MAP * 0.5 + 0.4 * CELL, MAP * 0.5 + 0.6 * CELL);
    let (blat0, blon0) = lat_lon(p, x0, y0);
    let (blat1, blon1) = lat_lon(p, x0 + 5.3 * CELL, y0 + 5.3 * CELL);
    (blat0..blat1).contains(&lat) && (blon0..blon1).contains(&lon)
}

#[test]
fn a_small_real_change_leaves_no_halo() {
    // a field that is 1.5 times as bright in the second flight and not aligned to the cells: within the factor of
    // two that colour differences between sensors need, but small against its surroundings
    let p = project();
    let reference = raster(&p, ALL, COARSE, |lat, lon| Some(scene(lat, lon)));
    let second = |with_block: bool| {
        raster(&p, (0.1, 0.1, 0.9, 0.9), 3e-5, |lat, lon| {
            let gain = if with_block && off_grid_block(lat, lon, &p) {
                1.8
            } else {
                1.2
            };
            Some(scaled(scene(lat, lon), gain))
        })
    };
    let (with, _) = merge(vec![reference.clone(), second(true)], &p).unwrap();
    let (without, _) = merge(vec![reference, second(false)], &p).unwrap();
    // a ring one to two cells outside the block
    let (lo, hi) = (
        MAP * 0.5 + 0.4 * CELL - 1.5 * CELL,
        MAP * 0.5 + 0.4 * CELL + 5.3 * CELL + 1.5 * CELL,
    );
    let (lo_y, hi_y) = (lo + 0.2 * CELL, hi + 0.2 * CELL);
    for i in 0..=20 {
        let t = f64::from(i) / 20.0;
        let (tx, ty) = (lo + (hi - lo) * t, lo_y + (hi_y - lo_y) * t);
        for (x, y) in [(tx, lo_y), (tx, hi_y), (lo, ty), (hi, ty)] {
            let got = paint_f(&with, &p, x, y).unwrap();
            let want = paint_f(&without, &p, x, y).unwrap();
            assert!(
                close(got, want, 3.0),
                "({x}, {y}): {got:?} against {want:?}"
            );
        }
    }
}

#[test]
fn chain_of_flights_is_matched() {
    let p = project();
    let flat = |gain: f32| move |_: f64, _: f64| Some([100.0 * gain, 100.0 * gain, 100.0 * gain]);
    let a = raster(&p, (-0.05, -0.05, 0.4, 1.05), 3e-5, flat(1.0));
    let b = raster(&p, (0.3, -0.05, 0.7, 1.05), 3e-5, flat(1.2));
    let c3 = raster(&p, (0.6, -0.05, 1.05, 1.05), 3e-5, flat(1.5));
    let (c, report) = merge(vec![c3, b, a], &p).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let in_a = paint_f(&c, &p, MAP * 0.1, MAP * 0.5).unwrap();
    let in_c = paint_f(&c, &p, MAP * 0.9, MAP * 0.5).unwrap();
    assert!(close(in_a, in_c, 3.0), "{in_a:?} and {in_c:?}");
}

#[test]
fn no_overlap_warns_and_keeps_colours() {
    let p = project();
    let a = raster(&p, (0.0, 0.0, 0.4, 1.0), COARSE, |lat, lon| {
        Some(scene(lat, lon))
    });
    let b = raster(&p, (0.6, 0.0, 1.0, 1.0), COARSE, |lat, lon| {
        Some(scaled(scene(lat, lon), 1.4))
    });
    let label = b.label().to_string();
    let (c, report) = merge(vec![a, b.clone()], &p).unwrap();
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("overlaps no other image") && w.contains(&label)),
        "{:?}",
        report.warnings
    );
    assert_eq!(report.sources[1].median_gain, [1.0, 1.0, 1.0]);
    let (lat, lon) = lat_lon(&p, MAP * 0.8, MAP * 0.5);
    let want = b.sampler().unwrap().rgb_at(lat, lon).unwrap();
    let got = paint_f(&c, &p, MAP * 0.8, MAP * 0.5).unwrap();
    assert!(close(got, want, 1.0), "{got:?} against {want:?}");
}

#[test]
fn hole_in_a_flight_falls_through() {
    let p = project();
    let reference = raster(&p, ALL, COARSE, |lat, lon| Some(scene(lat, lon)));
    let (hlat, hlon) = lat_lon(&p, MAP * 0.5, MAP * 0.5);
    let sharp = raster(&p, (0.3, 0.3, 0.7, 0.7), FINE, |lat, lon| {
        let in_hole = (lat - hlat).abs() < 1.5e-4 && (lon - hlon).abs() < 2.2e-4;
        (!in_hole).then(|| plus(scene(lat, lon), 20.0 * noise(lat, lon, 4)))
    });
    let (c, _) = merge(vec![reference.clone(), sharp], &p).unwrap();
    let sampler = reference.sampler().unwrap();
    let got = paint_f(&c, &p, MAP * 0.5, MAP * 0.5).expect("the hole is not empty");
    let want = sampler.rgb_at(hlat, hlon).unwrap();
    assert!(close(got, want, 2.0), "{got:?} against {want:?}");
}

#[test]
fn dark_water_does_not_drive_gains() {
    let p = project();
    let lake = |lat: f64, lon: f64| {
        let (x, y) = {
            let geo = MapGeo::new(&project()).unwrap();
            geo.to_map(lat, lon).unwrap()
        };
        x < MAP * 0.75 && y > MAP * 0.2
    };
    // the lake is dark in both images but differently so; it is most of the overlap
    let a = raster(&p, ALL, 3e-5, |lat, lon| {
        Some(if lake(lat, lon) {
            [4.0, 5.0, 6.0]
        } else {
            scene(lat, lon)
        })
    });
    let b = raster(&p, ALL, 3e-5, |lat, lon| {
        Some(if lake(lat, lon) {
            [7.0, 3.0, 2.0]
        } else {
            scaled(scene(lat, lon), 1.2)
        })
    });
    let (_, report) = merge(vec![a, b], &p).unwrap();
    let gain = report.sources[1].median_gain;
    for g in gain {
        assert!((g - 1.0 / 1.2).abs() < 0.02, "gain {gain:?}");
    }
}

fn run(threads: usize) -> (Vec<Option<[u8; 3]>>, MergeReport) {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    pool.install(|| {
        let p = project();
        let a = raster(&p, (-0.05, -0.05, 0.6, 1.05), 4e-5, |lat, lon| {
            Some(plus(scene(lat, lon), 5.0 * noise(lat, lon, 1)))
        });
        let b = raster(&p, (0.4, -0.05, 1.05, 1.05), 3e-5, |lat, lon| {
            Some(scaled(plus(scene(lat, lon), 5.0 * noise(lat, lon, 2)), 1.3))
        });
        let (c, report) = merge(vec![a, b], &p).unwrap();
        let rect = MapRect {
            west: 600.0,
            north: 900.0,
            step_x: 1.0,
            step_y: 1.0,
            width: 400,
            height: 40,
        };
        (c.paint(&p, &rect).unwrap(), report)
    })
}

#[test]
fn two_runs_are_identical() {
    let (image1, report1) = run(1);
    let (image8, report8) = run(8);
    assert_eq!(report1, report8);
    assert_eq!(image1, image8);
}

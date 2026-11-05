// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Projection and heightmap generation from a (synthetic) DEM.

#![expect(
    clippy::cast_possible_truncation,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::dem::Dem;
use naves_core::heightmap::{HeightmapError, build_heightmap};
use naves_core::project::Project;
use naves_core::projection::LocalProjection;

const LAT: f64 = 47.585;
const LON: f64 = 12.965;

fn project() -> Project {
    let mut p = Project::new("t", LAT, LON, 2000.0);
    p.grid.terrain_size = 256;
    p.grid.land_size = 64;
    p.scale.height = 1.0;
    p
}

/// A DEM that covers the project area with a margin, sampled every 0.0005 degrees.
fn dem(f: impl Fn(f64, f64) -> f32) -> Dem {
    Dem::from_fn(LON - 0.05, LAT - 0.05, LON + 0.05, LAT + 0.05, 0.0005, f)
}

#[test]
fn projection_roundtrip() {
    let p = LocalProjection::for_center(LAT, LON).unwrap();
    let (e, n) = p.to_utm(LAT, LON).unwrap();
    let (la, lo) = p.to_geo(e, n).unwrap();
    assert!((la - LAT).abs() < 1e-7 && (lo - LON).abs() < 1e-7);
}

#[test]
fn utm_zone_follows_the_centre() {
    // 12.965 E is in zone 33 (12-18 E): easting is around 300 km west of the central meridian 15 E
    let (e, _) = LocalProjection::for_center(LAT, LON)
        .unwrap()
        .to_utm(LAT, LON)
        .unwrap();
    assert!(
        (e - 500_000.0).abs() > 100_000.0 && (e - 500_000.0) < 0.0,
        "{e}"
    );
    // the southern hemisphere has a false northing
    let (_, n) = LocalProjection::for_center(-33.9, 18.4)
        .unwrap()
        .to_utm(-33.9, 18.4)
        .unwrap();
    assert!(n > 5_000_000.0, "{n}");
}

#[test]
fn height_scale_applies() {
    let mut p = project();
    p.scale.height = 0.5;
    let hm = build_heightmap(&p, &dem(|_, _| 1000.0)).unwrap();
    assert_eq!(hm.h.len(), 256 * 256);
    assert!(hm.h.iter().all(|&h| (h - 500.0).abs() < 1e-3));
    assert_eq!(hm.min_max().0.to_bits(), hm.min_max().1.to_bits());
}

#[test]
fn horizontal_scale_stretches() {
    // DEM = easting in UTM metres relative to the centre: one metre of height per real metre east.
    let proj = LocalProjection::for_center(LAT, LON).unwrap();
    let (e0, _) = proj.to_utm(LAT, LON).unwrap();
    let hm = build_heightmap(
        &project(),
        &dem(move |lat, lon| (proj.to_utm(lat, lon).unwrap().0 - e0) as f32),
    )
    .unwrap();
    // map cell is 1000 m / 256 = 3.906 m, real cell = cell / 0.5 horizontal
    let real_cell = f64::from(hm.cell_m) / 0.5;
    let step = f64::from(hm.at(101, 50) - hm.at(100, 50));
    assert!((step - real_cell).abs() < 0.05, "{step} vs {real_cell}");
}

#[test]
fn cell_size_matches_project() {
    let p = project();
    let hm = build_heightmap(&p, &dem(|_, _| 1.0)).unwrap();
    assert_eq!(hm.size, 256);
    assert!((f64::from(hm.cell_m) - p.terrain_cell_m()).abs() < 1e-4);
}

#[test]
fn south_row_first() {
    let hm = build_heightmap(&project(), &dem(|lat, _| ((lat - LAT) * 10_000.0) as f32)).unwrap();
    assert!(hm.at(10, 0) < hm.at(10, 255));
    let (min, max) = hm.min_max();
    assert!(min < 0.0 && max > 0.0);
}

#[test]
fn missing_coverage_is_error() {
    // only the western half: the map centre is at LON, so everything east of it is missing
    let west_only = Dem::from_fn(LON - 0.05, LAT - 0.05, LON, LAT + 0.05, 0.0005, |_, _| 1.0);
    match build_heightmap(&project(), &west_only) {
        Err(HeightmapError::MissingDem { lat, lon }) => {
            assert!((lat - LAT).abs() < 0.05 && lon >= LON - 1e-6, "{lat},{lon}");
        }
        other => panic!("expected MissingDem, got {other:?}"),
    }
}

#[test]
fn nan_in_the_dem_is_an_error_not_a_hole() {
    let nan_dem = dem(|_, lon| if lon > LON { f32::NAN } else { 1.0 });
    assert!(matches!(
        build_heightmap(&project(), &nan_dem),
        Err(HeightmapError::MissingDem { .. })
    ));
}

#[test]
fn sample_interpolates_and_reports_the_gradient() {
    use naves_core::heightmap::Heightmap;
    // heights rise 2 m per 4 m eastwards and 1 m per 4 m northwards
    let h: Vec<f32> = (0..8_u8)
        .flat_map(|row| (0..8_u8).map(move |col| f32::from(col) * 2.0 + f32::from(row)))
        .collect();
    let hm = Heightmap {
        size: 8,
        cell_m: 4.0,
        h,
    };
    assert!((hm.sample(8.0, 4.0).height - hm.at(2, 1)).abs() < 1e-5);
    let mid = hm.sample(6.0, 2.0);
    assert!((mid.height - 3.5).abs() < 1e-5, "{mid:?}");
    assert!(
        (mid.dz_dx - 0.5).abs() < 1e-5 && (mid.dz_dy - 0.25).abs() < 1e-5,
        "{mid:?}"
    );
    // outside the grid the edge value is held
    assert!((hm.sample(-100.0, -100.0).height - hm.at(0, 0)).abs() < 1e-5);
    assert!((hm.sample(1e6, 1e6).height - hm.at(7, 7)).abs() < 1e-5);
}

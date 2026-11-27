// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Writing a fetched DEM as one-band Float32 GeoTIFF and reading it back with `naves_core::dem`: no network,
//! every file lives under a temporary directory.

use naves_core::dem::Dem;
use naves_core::heightmap::MapGeo;
use naves_core::landcover::GeoWindow;
use naves_core::project::Project;
use naves_export::dem_fetch::{MARGIN_M, TILE_PX, fetch_window, tile_range, world_px};
use naves_export::geotiff_write::{DemMosaic, write_dem};
use naves_export::imagery_fetch::FetchError;

/// A 3 x 2 mosaic of distinct heights, row 0 at the north.
fn mosaic() -> DemMosaic {
    DemMosaic {
        width: 3,
        height: 2,
        data: vec![100.0, 101.5, 102.25, -3.0, 0.5, 1234.75],
    }
}

/// Three degrees of longitude, two of latitude: one degree per pixel in x, one in y.
fn bounds() -> GeoWindow {
    GeoWindow {
        south: 50.0,
        west: 10.0,
        north: 52.0,
        east: 13.0,
    }
}

#[test]
fn written_dem_reads_back_at_pixel_centres() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dem").join("fetched.tif");
    write_dem(&path, &mosaic(), bounds()).unwrap();

    let dem = Dem::open(std::slice::from_ref(&path)).unwrap();
    let b = bounds();
    let (sx, sy) = ((b.east - b.west) / 3.0, (b.north - b.south) / 2.0);
    // first pixel: row 0, column 0; last pixel: row 1, column 2
    let first = dem.height(b.north - 0.5 * sy, b.west + 0.5 * sx).unwrap();
    let last = dem.height(b.south + 0.5 * sy, b.west + 2.5 * sx).unwrap();
    assert!((first - 100.0).abs() < 1e-4, "first pixel was {first}");
    assert!((last - 1234.75).abs() < 1e-4, "last pixel was {last}");
}

#[test]
fn mosaic_of_the_wrong_size_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fetched.tif");

    let short = DemMosaic {
        data: vec![1.0; 5],
        ..mosaic()
    };
    assert!(matches!(
        write_dem(&path, &short, bounds()),
        Err(FetchError::Other(_))
    ));

    let empty = DemMosaic {
        width: 0,
        height: 0,
        data: Vec::new(),
    };
    assert!(matches!(
        write_dem(&path, &empty, bounds()),
        Err(FetchError::Other(_))
    ));
    assert!(!path.exists(), "a refused mosaic must not leave a file");
}

#[test]
fn bounds_that_are_not_a_box_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fetched.tif");

    let flipped = GeoWindow {
        east: 10.0,
        ..bounds()
    };
    assert!(matches!(
        write_dem(&path, &mosaic(), flipped),
        Err(FetchError::Other(_))
    ));

    let infinite = GeoWindow {
        north: f64::INFINITY,
        ..bounds()
    };
    assert!(matches!(
        write_dem(&path, &mosaic(), infinite),
        Err(FetchError::Other(_))
    ));
    assert!(!path.exists(), "refused bounds must not leave a file");
}

/// The example project of the spike: Berchtesgaden, 16 km of real ground, half a map metre per metre.
fn example_project() -> Project {
    let mut p = Project::new("T", 47.571_769, 12.946_451, 16384.0);
    p.scale.horizontal = 0.5;
    p
}

/// Latitude and longitude of the north-west corner of tile `(x, y)` at zoom `z` (Web Mercator).
fn tile_corner(z: u32, x: u32, y: u32) -> (f64, f64) {
    let n = f64::from(1u32 << z);
    let lon = f64::from(x) / n * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * f64::from(y) / n))
        .sinh()
        .atan()
        .to_degrees();
    (lat, lon)
}

#[test]
fn fetch_window_is_the_map_window_with_a_real_margin() {
    let p = example_project();
    let fetched = fetch_window(&p).unwrap();
    let exact = MapGeo::new(&p).unwrap().window(500.0).unwrap();
    assert_eq!(fetched.south.to_bits(), exact.south.to_bits());
    assert_eq!(fetched.west.to_bits(), exact.west.to_bits());
    assert_eq!(fetched.north.to_bits(), exact.north.to_bits());
    assert_eq!(fetched.east.to_bits(), exact.east.to_bits());

    // the margin is MARGIN_M real metres (MARGIN_M * scale.horizontal map metres) on every side
    let bare = MapGeo::new(&p).unwrap().window(0.0).unwrap();
    let real_m_per_degree = 111_132.0;
    let expected_deg = MARGIN_M / real_m_per_degree;
    let north_margin = fetched.north - bare.north;
    assert!(
        (north_margin - expected_deg).abs() < 0.05 * expected_deg,
        "north margin {north_margin} deg, expected about {expected_deg}"
    );
    assert!(fetched.north > bare.north && fetched.south < bare.south);
    assert!(fetched.west < bare.west && fetched.east > bare.east);
}

#[test]
fn world_pixel_of_a_tile_corner_is_the_tile_origin() {
    let (lat, lon) = tile_corner(16, 35130, 22901);
    let (x, y) = world_px(lat, lon, 16);
    let tile = f64::from(TILE_PX);
    assert!((x - 35130.0 * tile).abs() < 1e-6, "x was {x}");
    assert!((y - 22901.0 * tile).abs() < 1e-6, "y was {y}");
}

#[test]
fn a_point_inside_the_koenigssee_fixture_tile_lands_in_it() {
    // centre of the fixture tile z16 35130/22901 (tests/fixtures/dem/mapterhorn_z16_35130_22901.webp)
    let (north, west) = tile_corner(16, 35130, 22901);
    let (south, east) = tile_corner(16, 35131, 22902);
    let (lat, lon) = (f64::midpoint(north, south), f64::midpoint(west, east));
    let (x, y) = world_px(lat, lon, 16);
    let tile = f64::from(TILE_PX);
    assert!(((x / tile).floor() - 35130.0).abs() < 1e-9, "column of {x}");
    assert!(((y / tile).floor() - 22901.0).abs() < 1e-9, "row of {y}");
}

#[test]
fn tile_range_grows_the_corner_tiles_by_one_and_clamps() {
    // a window inside one tile at z16: the tile and its eight neighbours
    let (north, west) = tile_corner(16, 35130, 22901);
    let (south, east) = tile_corner(16, 35131, 22902);
    let inside = GeoWindow {
        south: south + 1e-4,
        west: west + 1e-4,
        north: north - 1e-4,
        east: east - 1e-4,
    };
    assert_eq!(tile_range(&inside, 16), (35129, 35131, 22900, 22902));

    // the antimeridian at z2 (4 tiles per side): clamped to 0..=3
    let east_edge = GeoWindow {
        south: -1.0,
        west: 179.0,
        north: 1.0,
        east: 179.9,
    };
    assert_eq!(tile_range(&east_edge, 2), (2, 3, 0, 3));
    let west_edge = GeoWindow {
        south: -1.0,
        west: -179.9,
        north: 1.0,
        east: -179.0,
    };
    assert_eq!(tile_range(&west_edge, 2), (0, 1, 0, 3));

    // at the poles: latitudes are clamped to the Web Mercator limit, so the rows stay in 0..=2^z-1
    let north_pole = GeoWindow {
        south: 84.0,
        west: 0.0,
        north: 90.0,
        east: 1.0,
    };
    assert_eq!(tile_range(&north_pole, 4).2, 0);
    assert_eq!(tile_range(&north_pole, 4).3, 1);
    let south_pole = GeoWindow {
        south: -90.0,
        west: 0.0,
        north: -84.0,
        east: 1.0,
    };
    assert_eq!(tile_range(&south_pole, 4).2, 14);
    assert_eq!(tile_range(&south_pole, 4).3, 15);
}

#[test]
fn the_example_map_needs_about_two_thousand_tiles_at_z16() {
    let window = fetch_window(&example_project()).unwrap();
    let (x0, x1, y0, y1) = tile_range(&window, 16);
    // tile_range grows by one tile per side: the tiles without that halo are inside
    let unhaloed = (x1 - x0 - 1) * (y1 - y0 - 1);
    assert!(
        (2000..=2300).contains(&unhaloed),
        "{unhaloed} tiles without the halo"
    );
}

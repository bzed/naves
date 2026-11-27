// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Writing a fetched DEM as one-band Float32 GeoTIFF and reading it back with `naves_core::dem`: no network,
//! every file lives under a temporary directory.

use naves_core::dem::Dem;
use naves_core::landcover::GeoWindow;
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

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Behaviour of the ESA `WorldCover` reader.

#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]

use std::io::Cursor;
use std::path::PathBuf;

use naves_core::landcover::{GeoWindow, LandCover, LandCoverError, surface_of};
use naves_core::surface::SurfaceClass;
use tiff::encoder::{TiffEncoder, colortype};
use tiff::tags::Tag;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/landcover/koenigssee.tif")
}

/// A power of two, so that every pixel edge is exact in floating point.
const PIXEL: f64 = 1.0 / 1024.0;

/// A synthetic WorldCover-like tile: `f(col, row)` is the class code.
fn geotiff_u8(
    (west, north): (f64, f64),
    (width, height): (u32, u32),
    epsg: u16,
    f: impl Fn(u32, u32) -> u8,
) -> Vec<u8> {
    let mut data = Vec::new();
    for row in 0..height {
        for col in 0..width {
            data.push(f(col, row));
        }
    }
    let mut cur = Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut cur).unwrap();
        let mut img = enc.new_image::<colortype::Gray8>(width, height).unwrap();
        let scale = [PIXEL, PIXEL, 0.0];
        let tie = [0.0, 0.0, 0.0, west, north, 0.0];
        let keys: [u16; 16] = [1, 1, 0, 3, 1024, 0, 1, 2, 1025, 0, 1, 1, 2048, 0, 1, epsg];
        img.encoder()
            .write_tag(Tag::ModelPixelScaleTag, &scale[..])
            .unwrap();
        img.encoder()
            .write_tag(Tag::ModelTiepointTag, &tie[..])
            .unwrap();
        img.encoder()
            .write_tag(Tag::GeoKeyDirectoryTag, &keys[..])
            .unwrap();
        img.write_data(&data).unwrap();
    }
    cur.into_inner()
}

fn write(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn window(south: f64, west: f64, north: f64, east: f64) -> GeoWindow {
    GeoWindow {
        south,
        west,
        north,
        east,
    }
}

#[test]
fn classes_of_the_crop_match_gdal() {
    // values from `gdallocationinfo -valonly -geoloc` on the same file
    let lc = LandCover::open(&[fixture()], window(47.54, 12.96, 47.56, 12.99)).unwrap();
    assert_eq!(lc.class_code(47.5534, 12.9765), Some(80), "the lake");
    assert_eq!(lc.class_code(47.5595, 12.9605), Some(30), "grassland");
    assert_eq!(lc.class_code(47.541, 12.9605), Some(10), "trees");
}

#[test]
fn seam_between_two_tiles_is_continuous() {
    let dir = tempfile::tempdir().unwrap();
    // two tiles of 64 pixels that meet at lon 11.0625: the west one is trees, the east one grass
    let seam = 11.0 + 64.0 * PIXEL;
    let a = write(
        &dir,
        "a.tif",
        &geotiff_u8((11.0, 48.0), (64, 64), 4326, |_, _| 10),
    );
    let b = write(
        &dir,
        "b.tif",
        &geotiff_u8((seam, 48.0), (64, 64), 4326, |_, _| 30),
    );
    let w = window(48.0 - 64.0 * PIXEL, 11.0, 48.0, seam + 64.0 * PIXEL);
    for order in [[a.clone(), b.clone()], [b, a]] {
        let lc = LandCover::open(&order, w).unwrap();
        let lat = 48.0 - 10.5 * PIXEL;
        assert_eq!(
            lc.class_code(lat, seam - PIXEL * 0.25),
            Some(10),
            "west of the seam"
        );
        assert_eq!(lc.class_code(lat, seam), Some(30), "on the seam");
        assert_eq!(
            lc.class_code(lat, seam + PIXEL * 0.25),
            Some(30),
            "east of the seam"
        );
    }
}

#[test]
fn only_the_window_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        &dir,
        "big.tif",
        &geotiff_u8((10.0, 50.0), (4000, 4000), 4326, |col, _| {
            if col < 2000 { 10 } else { 50 }
        }),
    );
    // 10 x 10 pixels, aligned to pixel edges, inside the tile
    let west = 10.0 + 1000.0 * PIXEL;
    let north = 50.0 - 1000.0 * PIXEL;
    let lc = LandCover::open(
        &[path],
        window(north - 10.0 * PIXEL, west, north, west + 10.0 * PIXEL),
    )
    .unwrap();
    assert!(
        (100..=144).contains(&lc.pixel_count()),
        "{} pixels held",
        lc.pixel_count()
    );
    assert_eq!(
        lc.class_code(north - 5.5 * PIXEL, west + 5.5 * PIXEL),
        Some(10)
    );
    assert_eq!(
        lc.class_code(north - 5.5 * PIXEL, west + 1500.0 * PIXEL),
        None,
        "inside the file but outside the window"
    );
}

#[test]
fn a_file_that_does_not_touch_the_window_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        &dir,
        "far.tif",
        &geotiff_u8((10.0, 50.0), (64, 64), 4326, |_, _| 10),
    );
    let lc = LandCover::open(&[path], window(0.0, 0.0, 1.0, 1.0)).unwrap();
    assert_eq!(lc.pixel_count(), 0);
    assert_eq!(lc.class_code(0.5, 0.5), None);
}

#[test]
fn outside_and_no_data_are_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        &dir,
        "t.tif",
        &geotiff_u8(
            (10.0, 50.0),
            (64, 64),
            4326,
            |col, _| if col < 32 { 0 } else { 40 },
        ),
    );
    let lc = LandCover::open(
        &[path],
        window(50.0 - 64.0 * PIXEL, 10.0, 50.0, 10.0 + 64.0 * PIXEL),
    )
    .unwrap();
    let lat = 50.0 - 10.5 * PIXEL;
    assert_eq!(
        lc.class_code(lat, 10.0 + 10.5 * PIXEL),
        None,
        "code 0 is no data"
    );
    assert_eq!(lc.class_code(lat, 10.0 + 40.5 * PIXEL), Some(40));
    assert_eq!(lc.class_code(60.0, 10.0), None, "outside the file");
    assert_eq!(lc.class_code(f64::NAN, 10.0), None);
}

#[test]
fn wrong_crs_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        &dir,
        "utm.tif",
        &geotiff_u8((10.0, 50.0), (64, 64), 32632, |_, _| 10),
    );
    let err = LandCover::open(&[path], window(49.0, 9.0, 51.0, 11.0)).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("utm.tif") && text.contains("4326"), "{text}");
    assert!(matches!(err, LandCoverError::File { .. }));
}

#[test]
fn a_window_that_is_too_big_is_an_error_naming_the_file() {
    let dir = tempfile::tempdir().unwrap();
    // 17 000 pixels wide, one row: small on disk, above the limit of 16 000 per side
    let path = write(
        &dir,
        "wide.tif",
        &geotiff_u8((0.0, 1.0), (17_000, 1), 4326, |_, _| 10),
    );
    let w = window(1.0 - PIXEL, 0.0, 1.0, 17_000.0 * PIXEL);
    let err = LandCover::open(&[path], w).unwrap_err();
    assert!(err.to_string().contains("wide.tif"), "{err}");
}

#[test]
fn surface_table() {
    let table = [
        (10, Some(SurfaceClass::Forest)),
        (95, Some(SurfaceClass::Forest)),
        (20, Some(SurfaceClass::Grass)),
        (30, Some(SurfaceClass::Grass)),
        (90, Some(SurfaceClass::Grass)),
        (40, Some(SurfaceClass::Soil)),
        (50, Some(SurfaceClass::Soil)),
        (80, Some(SurfaceClass::Soil)),
        (60, Some(SurfaceClass::Rock)),
        (70, Some(SurfaceClass::Rock)),
        (100, Some(SurfaceClass::Rock)),
        (0, None),
        (11, None),
        (255, None),
    ];
    for (code, class) in table {
        assert_eq!(surface_of(code), class, "code {code}");
    }
}

#[test]
fn garbage_and_truncation_are_errors() {
    let w = window(47.54, 12.96, 47.56, 12.99);
    assert!(LandCover::from_bytes(b"not a tiff".to_vec(), w).is_err());
    let good = std::fs::read(fixture()).unwrap();
    assert!(LandCover::from_bytes(good.clone(), w).is_ok());
    assert!(LandCover::from_bytes(good[..good.len() / 2].to_vec(), w).is_err());
}

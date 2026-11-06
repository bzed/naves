// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Behaviour of the GeoTIFF DEM reader.

#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]

use std::io::Cursor;
use std::path::PathBuf;

use naves_core::dem::{Dem, DemError, DemTile};
use tiff::encoder::{TiffEncoder, colortype};
use tiff::tags::Tag;

fn fx(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/dem")
        .join(name)
}

/// Geo-referencing for a synthetic tile: upper-left corner and pixel size in degrees.
struct Geo {
    west: f64,
    north: f64,
    pixel: f64,
    epsg: u16,
}

fn geotiff_f32(width: u32, height: u32, geo: &Geo, data: &[f32]) -> Vec<u8> {
    geotiff_f32_nodata(width, height, geo, data, None)
}

fn geotiff_f32_nodata(
    width: u32,
    height: u32,
    geo: &Geo,
    data: &[f32],
    nodata: Option<&str>,
) -> Vec<u8> {
    let mut cur = Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut cur).unwrap();
        let mut img = enc
            .new_image::<colortype::Gray32Float>(width, height)
            .unwrap();
        let scale = [geo.pixel, geo.pixel, 0.0];
        let tie = [0.0, 0.0, 0.0, geo.west, geo.north, 0.0];
        let keys: [u16; 16] = [
            1, 1, 0, 3, 1024, 0, 1, 2, 1025, 0, 1, 1, 2048, 0, 1, geo.epsg,
        ];
        img.encoder()
            .write_tag(Tag::ModelPixelScaleTag, &scale[..])
            .unwrap();
        img.encoder()
            .write_tag(Tag::ModelTiepointTag, &tie[..])
            .unwrap();
        img.encoder()
            .write_tag(Tag::GeoKeyDirectoryTag, &keys[..])
            .unwrap();
        if let Some(nodata) = nodata {
            img.encoder()
                .write_tag(Tag::Unknown(42113), nodata)
                .unwrap();
        }
        img.write_data(data).unwrap();
    }
    cur.into_inner()
}

fn tile(bytes: &[u8]) -> Result<DemTile, DemError> {
    DemTile::from_reader(Cursor::new(bytes))
}

fn geo(west: f64, north: f64) -> Geo {
    Geo {
        west,
        north,
        pixel: 0.01,
        epsg: 4326,
    }
}

fn write_tmp(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.path().join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

#[test]
fn koenigssee_surface() {
    // gdallocationinfo reports 602 m at this point
    let d = Dem::open(&[fx("koenigssee.tif")]).unwrap();
    assert!((d.height(47.5534, 12.9765).unwrap() - 602.0).abs() < 1.0);
}

#[test]
fn outside_is_none() {
    let d = Dem::open(&[fx("koenigssee.tif")]).unwrap();
    assert!(d.height(47.0, 12.0).is_none());
    assert!(d.height(f64::NAN, 12.97).is_none());
}

#[test]
fn seam_is_continuous() {
    let d = Dem::open(&[fx("seam_w.tif"), fx("seam_e.tif")]).unwrap();
    let step = 1.0 / 3600.0 / 4.0; // quarter pixel
    let a = d.height(47.63, 13.0 - step).unwrap();
    let b = d.height(47.63, 13.0 + step).unwrap();
    assert!((a - b).abs() < 5.0, "step at the seam: {a} vs {b}");
}

#[test]
fn seam_is_continuous_in_either_order() {
    let d = Dem::open(&[fx("seam_e.tif"), fx("seam_w.tif")]).unwrap();
    // 12.99972 is the centre of the last column of seam_w, 13.0 the centre of the first of seam_e
    let mid = d.height(47.63, 12.9998).unwrap();
    assert!((mid - 587.4).abs() < 1.0, "{mid}");
}

#[test]
fn rejects_non_float() {
    let mut cur = Cursor::new(Vec::new());
    TiffEncoder::new(&mut cur)
        .unwrap()
        .write_image::<colortype::Gray8>(4, 4, &[0u8; 16])
        .unwrap();
    assert!(matches!(tile(cur.get_ref()), Err(DemError::Unsupported(_))));
}

#[test]
fn rejects_other_projection() {
    let bytes = geotiff_f32(
        2,
        2,
        &Geo {
            epsg: 32633,
            ..geo(0.0, 1.0)
        },
        &[1.0; 4],
    );
    assert!(matches!(tile(&bytes), Err(DemError::Unsupported(_))));
}

#[test]
fn rejects_missing_georeferencing() {
    let mut cur = Cursor::new(Vec::new());
    TiffEncoder::new(&mut cur)
        .unwrap()
        .write_image::<colortype::Gray32Float>(2, 2, &[1.0; 4])
        .unwrap();
    assert!(matches!(tile(cur.get_ref()), Err(DemError::Unsupported(_))));
}

#[test]
fn rejects_oversized_image() {
    let bytes = geotiff_f32(20_000, 1, &geo(0.0, 1.0), &vec![0.0; 20_000]);
    assert!(matches!(tile(&bytes), Err(DemError::Unsupported(_))));
}

#[test]
fn garbage_and_truncation_are_errors() {
    assert!(tile(b"not a tiff at all").is_err());
    assert!(tile(&[]).is_err());
    let bytes = std::fs::read(fx("koenigssee.tif")).unwrap();
    for len in [8, 100, bytes.len() / 2, bytes.len() - 200] {
        assert!(tile(&bytes[..len]).is_err(), "prefix of {len} bytes");
    }
}

#[test]
fn bilinear_between_pixel_centres() {
    // 3x1 pixels west..west+0.03, heights 0, 10, 20 along the row.
    let bytes = geotiff_f32(3, 1, &geo(10.0, 50.0), &[0.0, 10.0, 20.0]);
    let d = Dem::open(&[write_tmp(&tempfile::tempdir().unwrap(), "t.tif", &bytes)]).unwrap();
    // centre of the middle pixel
    assert!((d.height(49.995, 10.015).unwrap() - 10.0).abs() < 1e-4);
    // halfway between pixel 0 and 1 centres
    assert!((d.height(49.995, 10.010).unwrap() - 5.0).abs() < 1e-4);
    // beyond the last pixel centre the edge value is held
    assert!((d.height(49.995, 10.0299).unwrap() - 20.0).abs() < 0.1);
}

#[test]
fn north_is_the_first_row() {
    // 1x2 pixels: row 0 (north) = 100, row 1 (south) = 0
    let bytes = geotiff_f32(1, 2, &geo(10.0, 50.0), &[100.0, 0.0]);
    let d = Dem::open(&[write_tmp(&tempfile::tempdir().unwrap(), "t.tif", &bytes)]).unwrap();
    assert!((d.height(49.995, 10.005).unwrap() - 100.0).abs() < 1e-4);
    assert!((d.height(49.985, 10.005).unwrap() - 0.0).abs() < 1e-4);
}

#[test]
fn no_data_pixels_give_none() {
    let bytes = geotiff_f32(2, 2, &geo(10.0, 50.0), &[1.0, f32::NAN, 3.0, 4.0]);
    let d = Dem::open(&[write_tmp(&tempfile::tempdir().unwrap(), "t.tif", &bytes)]).unwrap();
    // pixel (1, 0) is NaN: its own centre and the midpoint towards pixel (0, 0) are unknown
    assert!(d.height(49.995, 10.015).is_none());
    assert!(d.height(49.995, 10.010).is_none());
    // pixel (0, 1) and its neighbours are fine
    assert!((d.height(49.985, 10.005).unwrap() - 3.0).abs() < 1e-4);
}

#[test]
fn open_names_the_failing_file() {
    let dir = tempfile::tempdir().unwrap();
    let bad = write_tmp(&dir, "bad.tif", b"junk");
    let err = Dem::open(&[bad]).unwrap_err();
    assert!(err.to_string().contains("bad.tif"), "{err}");
    let err = Dem::open(&[dir.path().join("missing.tif")]).unwrap_err();
    assert!(err.to_string().contains("missing.tif"), "{err}");
}

#[test]
fn the_files_no_data_value_is_not_a_height() {
    for nodata in ["-9999", "-32767.0"] {
        let sentinel: f32 = nodata.parse().unwrap();
        let bytes = geotiff_f32_nodata(
            2,
            2,
            &geo(10.0, 50.0),
            &[1.0, sentinel, 3.0, 4.0],
            Some(nodata),
        );
        let dir = tempfile::tempdir().unwrap();
        let path = write_tmp(&dir, "t.tif", &bytes);
        let d = Dem::open(&[path]).unwrap();
        // pixel (0, 1) and its neighbours are real heights, the centre of pixel (1, 0) has none
        assert!((d.height(49.985, 10.005).unwrap() - 3.0).abs() < 1e-4);
        assert_eq!(d.height(49.995, 10.015), None, "no-data value {nodata}");
    }
}

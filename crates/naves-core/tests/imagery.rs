// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Behaviour of the imagery reader: GeoTIFF files in the window of the map.

#![expect(
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test helper code; failing loudly is the point"
)]

use std::io::Cursor;
use std::path::PathBuf;

use naves_core::heightmap::MapGeo;
use naves_core::imagery::{ImageryError, ImageryRaster, real_px_m};
use naves_core::project::Project;
use proj4rs::Proj;
use tiff::encoder::{Compression, DeflateLevel};
use tiff::encoder::{TiffEncoder, colortype};
use tiff::tags::Tag;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/imagery/sentinel2_crop.tif")
}

/// What a synthetic file looks like.
#[derive(Clone)]
struct Spec {
    /// Upper left corner in the CRS of the file.
    origin: (f64, f64),
    /// Pixel size (positive) in the units of the CRS.
    pixel: (f64, f64),
    size: (u32, u32),
    epsg: u16,
    nodata: Option<&'static str>,
    alpha: bool,
}

impl Spec {
    fn geographic(west: f64, north: f64, pixel_deg: f64, size: (u32, u32)) -> Spec {
        Spec {
            origin: (west, north),
            pixel: (pixel_deg, pixel_deg),
            size,
            epsg: 4326,
            nodata: None,
            alpha: false,
        }
    }

    fn utm(epsg: u16, easting: f64, northing: f64, pixel_m: f64, size: (u32, u32)) -> Spec {
        Spec {
            origin: (easting, northing),
            pixel: (pixel_m, pixel_m),
            size,
            epsg,
            nodata: None,
            alpha: false,
        }
    }

    fn geo_keys(&self) -> [u16; 16] {
        if self.epsg == 4326 {
            [
                1, 1, 0, 3, 1024, 0, 1, 2, 1025, 0, 1, 1, 2048, 0, 1, self.epsg,
            ]
        } else {
            [
                1, 1, 0, 3, 1024, 0, 1, 1, 1025, 0, 1, 1, 3072, 0, 1, self.epsg,
            ]
        }
    }
}

type Pixel = [u8; 4];

/// A GeoTIFF with the full image and, for each `(width, height)` in `overviews`, a reduced one.
/// `f(col, row)` gives the colour (and alpha) of a pixel of the full image.
fn geotiff(spec: &Spec, overviews: &[(u32, u32)], f: impl Fn(u32, u32) -> Pixel) -> Vec<u8> {
    let mut cur = Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut cur).unwrap();
        let sizes = std::iter::once(spec.size).chain(overviews.iter().copied());
        for (level, (width, height)) in sizes.enumerate() {
            let (fx, fy) = (spec.size.0 / width, spec.size.1 / height);
            let mut data = Vec::new();
            for row in 0..height {
                for col in 0..width {
                    let p = f(col * fx, row * fy);
                    data.extend_from_slice(if spec.alpha { &p[..] } else { &p[..3] });
                }
            }
            if spec.alpha {
                let mut img = enc.new_image::<colortype::RGBA8>(width, height).unwrap();
                tags(&mut img, spec, level, (fx, fy));
                img.write_data(&data).unwrap();
            } else {
                let mut img = enc.new_image::<colortype::RGB8>(width, height).unwrap();
                tags(&mut img, spec, level, (fx, fy));
                img.write_data(&data).unwrap();
            }
        }
    }
    cur.into_inner()
}

fn tags<W, C, K>(
    img: &mut tiff::encoder::ImageEncoder<'_, W, C, K>,
    spec: &Spec,
    level: usize,
    (fx, fy): (u32, u32),
) where
    W: std::io::Write + std::io::Seek,
    C: tiff::encoder::colortype::ColorType,
    K: tiff::encoder::TiffKind,
{
    let scale = [
        spec.pixel.0 * f64::from(fx),
        spec.pixel.1 * f64::from(fy),
        0.0,
    ];
    let tie = [0.0, 0.0, 0.0, spec.origin.0, spec.origin.1, 0.0];
    let keys = spec.geo_keys();
    if level > 0 {
        img.encoder().write_tag(Tag::NewSubfileType, 1u32).unwrap();
    }
    img.encoder()
        .write_tag(Tag::ModelPixelScaleTag, &scale[..])
        .unwrap();
    img.encoder()
        .write_tag(Tag::ModelTiepointTag, &tie[..])
        .unwrap();
    img.encoder()
        .write_tag(Tag::GeoKeyDirectoryTag, &keys[..])
        .unwrap();
    if let Some(nodata) = spec.nodata {
        img.encoder()
            .write_tag(Tag::Unknown(42113), nodata)
            .unwrap();
    }
}

fn write(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn project(lat: f64, lon: f64, extent_m: f64) -> Project {
    Project::new("imagery test", lat, lon, extent_m)
}

/// A smooth colour field of the position, so that two files of one scene agree wherever they are sampled.
fn field(lat: f64, lon: f64) -> [u8; 3] {
    let r = 100.0 + 800.0 * (lon - 11.9);
    let g = 90.0 + 800.0 * (lat - 47.5);
    [
        r.round().clamp(0.0, 255.0) as u8,
        g.round().clamp(0.0, 255.0) as u8,
        120,
    ]
}

#[test]
fn crop_matches_gdal() {
    // values from `gdallocationinfo -wgs84 -valonly` at the centres of three pixels of the crop
    let p = project(47.5571, 12.9743, 800.0);
    let raster = ImageryRaster::open(&fixture(), &p, 0.0)
        .unwrap()
        .expect("the crop touches the map");
    let s = raster.sampler().unwrap();
    for ((lat, lon), want) in [
        (
            (47.559_998_311_832_3, 12.969_789_761_880_9),
            [100.0, 105.0, 75.0],
        ),
        ((47.557_108_415_078_8, 12.974_288_132_34), [2.0, 8.0, 5.0]),
        (
            (47.554_218_333_120_7, 12.978_786_008_404_3),
            [1.0, 9.0, 5.0],
        ),
    ] {
        let got = s.rgb_at(lat, lon);
        // the outer two are at the edge of the crop, which the map window (100 m margin) reaches
        let got = got.unwrap_or_else(|| panic!("no colour at {lat} {lon}"));
        for c in 0..3 {
            assert!(
                (got[c] - want[c]).abs() <= 1.0,
                "{lat} {lon}: {got:?} against {want:?}"
            );
        }
    }
}

/// Finds the red marker by scanning the map in 4 m steps and returns the mean map position.
fn find_marker(raster: &ImageryRaster, project: &Project) -> Option<(f64, f64)> {
    let geo = MapGeo::new(project).unwrap();
    let sampler = raster.sampler().unwrap();
    let size = project.map_size_m();
    let (mut sx, mut sy, mut count) = (0.0, 0.0, 0.0);
    let mut north = 0.0;
    while north < size {
        let mut east = 0.0;
        while east < size {
            let (lat, lon) = geo.to_geo(east, north).unwrap();
            if sampler
                .rgb_at(lat, lon)
                .is_some_and(|c| c[0] > 200.0 && c[1] < 50.0)
            {
                sx += east;
                sy += north;
                count += 1.0;
            }
            east += 4.0;
        }
        north += 4.0;
    }
    (count > 0.0).then(|| (sx / count, sy / count))
}

fn marker_file(marker: (f64, f64), pixel_deg: f64) -> (Spec, Vec<u8>) {
    let (west, north) = (12.93, 47.61);
    let (cols, rows) = (655, 491);
    let spec = Spec::geographic(west, north, pixel_deg, (cols, rows));
    let col = ((marker.1 - west) / pixel_deg).floor() as u32;
    let row = ((north - marker.0) / pixel_deg).floor() as u32;
    let bytes = geotiff(&spec, &[], |c, r| {
        if (c.abs_diff(col) <= 1) && (r.abs_diff(row) <= 1) {
            [255, 0, 0, 255]
        } else {
            [30, 30, 30, 255]
        }
    });
    (spec, bytes)
}

#[test]
fn north_is_up_and_east_is_right() {
    for horizontal in [1.0, 0.5] {
        let mut p = project(47.58, 12.97, 4000.0);
        p.scale.horizontal = horizontal;
        let geo = MapGeo::new(&p).unwrap();
        // a marker north-east of the centre
        let marker = (47.5925, 12.9825);
        let pixel_deg = 1.0 / 8192.0;
        let (_, bytes) = marker_file(marker, pixel_deg);
        let raster = ImageryRaster::from_bytes(bytes, &p, 0.0).unwrap().unwrap();
        let (mx, my) = geo.to_map(marker.0, marker.1).unwrap();
        let (fx, fy) = find_marker(&raster, &p).expect("the marker is found");
        let pixel_map_m = pixel_deg * 111_320.0 * horizontal;
        assert!(
            (fx - mx).abs() <= 2.0 * pixel_map_m && (fy - my).abs() <= 2.0 * pixel_map_m,
            "marker at ({mx:.1}, {my:.1}) found at ({fx:.1}, {fy:.1}), scale {horizontal}"
        );
        assert!(
            fx > p.map_size_m() / 2.0 && fy > p.map_size_m() / 2.0,
            "north-east of the centre"
        );
    }
}

/// A UTM file around the project centre whose pixels hold `field`.
fn field_file(epsg: u16, project: &Project) -> Vec<u8> {
    let zone = epsg % 100;
    let utm = Proj::from_proj_string(&format!(
        "+proj=utm +zone={zone} +datum=WGS84 +units=m +no_defs"
    ))
    .unwrap();
    let geo = Proj::from_proj_string("+proj=longlat +datum=WGS84 +no_defs").unwrap();
    let mut centre = (
        project.area.center_lon.to_radians(),
        project.area.center_lat.to_radians(),
    );
    proj4rs::transform::transform(&geo, &utm, &mut centre).unwrap();
    let size = 600;
    let spec = Spec::utm(
        epsg,
        (centre.0 - 3000.0).round(),
        (centre.1 + 3000.0).round(),
        10.0,
        (size, size),
    );
    geotiff(&spec, &[], |col, row| {
        let mut pt = (
            spec.origin.0 + (f64::from(col) + 0.5) * 10.0,
            spec.origin.1 - (f64::from(row) + 0.5) * 10.0,
        );
        proj4rs::transform::transform(&utm, &geo, &mut pt).unwrap();
        let [red, green, blue] = field(pt.1.to_degrees(), pt.0.to_degrees());
        [red, green, blue, 255]
    })
}

#[test]
fn file_in_the_neighbouring_zone_lines_up() {
    // the map is in zone 33 (12 °E and east); the file in zone 32 is the neighbour's
    let p = project(47.58, 12.0, 3000.0);
    let geo = MapGeo::new(&p).unwrap();
    let a = ImageryRaster::from_bytes(field_file(32632, &p), &p, 0.0)
        .unwrap()
        .unwrap();
    let b = ImageryRaster::from_bytes(field_file(32633, &p), &p, 0.0)
        .unwrap()
        .unwrap();
    let (sa, sb) = (a.sampler().unwrap(), b.sampler().unwrap());
    let size = p.map_size_m();
    for i in 0..10_u32 {
        let t = f64::from(i) / 10.0 + 0.05;
        let (lat, lon) = geo.to_geo(size * t, size * (1.0 - t * 0.8)).unwrap();
        let (ca, cb) = (sa.rgb_at(lat, lon).unwrap(), sb.rgb_at(lat, lon).unwrap());
        let want = field(lat, lon);
        for c in 0..3 {
            assert!(
                (ca[c] - cb[c]).abs() <= 1.0 + 1e-3,
                "zone 32 and 33 files differ at {lat} {lon}: {ca:?} {cb:?}"
            );
            assert!((ca[c] - f32::from(want[c])).abs() <= 1.5, "{ca:?} {want:?}");
        }
    }
}

#[test]
fn only_the_window_is_read() {
    // 2000 x 2000 pixels, compressed; the map is a few dozen pixels wide
    let spec = Spec::geographic(12.0, 48.0, 1.0 / 8192.0, (2000, 2000));
    let mut cur = Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut cur)
            .unwrap()
            .with_compression(Compression::Deflate(DeflateLevel::default()));
        let mut img = enc.new_image::<colortype::RGB8>(2000, 2000).unwrap();
        tags(&mut img, &spec, 0, (1, 1));
        let data: Vec<u8> = (0..2000 * 2000 * 3).map(|i| (i / 3 % 251) as u8).collect();
        img.write_data(&data).unwrap();
    }
    // 12.0 .. 12.244 E, 47.756 .. 48 N: a 600 m map near the middle
    let p = project(47.9, 12.12, 600.0);
    let raster = ImageryRaster::from_bytes(cur.into_inner(), &p, 0.0)
        .unwrap()
        .unwrap();
    assert!(
        raster.pixel_count() < 100 * 100,
        "{} pixels read",
        raster.pixel_count()
    );
    assert!(raster.pixel_count() > 0);
}

#[test]
fn outside_is_none() {
    let spec = Spec::geographic(12.93, 47.61, 1.0 / 8192.0, (655, 491));
    let bytes = geotiff(&spec, &[], |_, _| [10, 20, 30, 255]);
    // a map in Peru
    let far = project(-13.0, -72.0, 2000.0);
    assert!(
        ImageryRaster::from_bytes(bytes.clone(), &far, 0.0)
            .unwrap()
            .is_none()
    );
    // a map at the file's edge: a point beyond it has no colour
    let p = project(47.58, 12.97, 1000.0);
    let raster = ImageryRaster::from_bytes(bytes, &p, 0.0).unwrap().unwrap();
    let s = raster.sampler().unwrap();
    assert_eq!(s.rgb_at(47.58, 12.97), Some([10.0, 20.0, 30.0]));
    assert_eq!(s.rgb_at(47.70, 12.97), None);
    assert_eq!(s.rgb_at(f64::NAN, 12.97), None);
}

#[test]
fn no_data_and_alpha_are_none() {
    let p = project(47.58, 12.97, 1000.0);
    // alpha 0 in the east half
    let mut spec = Spec::geographic(12.93, 47.61, 1.0 / 8192.0, (655, 491));
    spec.alpha = true;
    let bytes = geotiff(&spec, &[], |c, _| {
        if c > 327 {
            [200, 100, 50, 0]
        } else {
            [200, 100, 50, 255]
        }
    });
    let r = ImageryRaster::from_bytes(bytes, &p, 0.0).unwrap().unwrap();
    let s = r.sampler().unwrap();
    assert!(s.rgb_at(47.58, 12.966).is_some(), "west half is opaque");
    assert!(
        s.rgb_at(47.58, 12.974).is_none(),
        "east half is transparent"
    );
    // NoData 0: pixels that are black in all three bands
    let mut spec = Spec::geographic(12.93, 47.61, 1.0 / 8192.0, (655, 491));
    spec.nodata = Some("0");
    let bytes = geotiff(&spec, &[], |c, _| {
        if c > 327 {
            [0, 0, 0, 255]
        } else {
            [0, 5, 0, 255]
        }
    });
    let r = ImageryRaster::from_bytes(bytes, &p, 0.0).unwrap().unwrap();
    let s = r.sampler().unwrap();
    assert!(
        s.rgb_at(47.58, 12.966).is_some(),
        "one band different from NoData is data"
    );
    assert!(s.rgb_at(47.58, 12.974).is_none(), "all three equal NoData");
}

#[test]
fn overview_is_chosen() {
    let spec = Spec::geographic(12.93, 47.61, 1.0 / 8192.0, (656, 492));
    let bytes = geotiff(&spec, &[(328, 246), (164, 123)], |_, _| [10, 20, 30, 255]);
    let p = project(47.58, 12.97, 1000.0);
    let base_m = 111_320.0 / 8192.0 * 47.58_f64.to_radians().cos().sqrt();
    let fine = ImageryRaster::from_bytes(bytes.clone(), &p, 1.0)
        .unwrap()
        .unwrap();
    assert!(
        fine.pixel_m() < base_m * 1.3 && fine.pixel_m() > base_m * 0.7,
        "the base image at a fine target: {}",
        fine.pixel_m()
    );
    let coarse = ImageryRaster::from_bytes(bytes, &p, 3.0 * base_m)
        .unwrap()
        .unwrap();
    assert!(
        coarse.pixel_m() > base_m * 1.5 && coarse.pixel_m() <= 3.0 * base_m,
        "an overview: {} against the base {base_m}",
        coarse.pixel_m()
    );
    assert!(coarse.pixel_count() < fine.pixel_count());
}

#[test]
fn too_large_without_overview_names_gdaladdo() {
    // 17 000 pixels wide, a few rows high: the window is wider than the limit
    let spec = Spec::geographic(12.0, 48.0, 1.0 / 100_000.0, (17_000, 8));
    let bytes = geotiff(&spec, &[], |_, _| [90, 90, 90, 255]);
    // the map covers the whole file: 0.17 degrees are 12.6 km
    let p = project(47.9999, 12.085, 12_000.0);
    let err = ImageryRaster::from_bytes(bytes, &p, 0.0).unwrap_err();
    assert!(err.to_string().contains("gdaladdo"), "{err}");
}

#[test]
fn wrong_crs_names_gdalwarp() {
    let spec = Spec {
        origin: (2_600_000.0, 1_200_000.0),
        pixel: (10.0, 10.0),
        size: (100, 100),
        epsg: 2056,
        nodata: None,
        alpha: false,
    };
    let bytes = geotiff(&spec, &[], |_, _| [1, 2, 3, 255]);
    let dir = tempfile::tempdir().unwrap();
    let path = write(&dir, "swiss.tif", &bytes);
    let p = project(47.58, 12.97, 1000.0);
    let err = ImageryRaster::open(&path, &p, 0.0).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("swiss.tif"), "{text}");
    assert!(text.contains("EPSG:2056"), "{text}");
    assert!(text.contains("gdalwarp -t_srs EPSG:32633"), "{text}");
}

#[test]
fn sixteen_bit_names_gdal_translate() {
    let mut cur = Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut cur).unwrap();
        let mut img = enc.new_image::<colortype::RGB16>(8, 8).unwrap();
        let spec = Spec::geographic(12.96, 47.59, 1.0 / 8192.0, (8, 8));
        tags(&mut img, &spec, 0, (1, 1));
        img.write_data(&[1000_u16; 8 * 8 * 3]).unwrap();
    }
    let p = project(47.5875, 12.9625, 1000.0);
    let err = ImageryRaster::from_bytes(cur.into_inner(), &p, 0.0).unwrap_err();
    assert!(err.to_string().contains("gdal_translate"), "{err}");
    assert!(matches!(err, ImageryError::Unsupported(_)));
}

#[test]
fn real_px_m_is_the_map_pixel_in_real_metres() {
    // 6400 m map, 256 land cells of 25 m, 60 pixels per cell: 0.417 m, 0.833 real metres at scale 0.5
    let p = project(47.58, 12.97, 12_800.0);
    assert!(
        (real_px_m(&p) - 1.0 / 1.2).abs() < 1e-9,
        "{}",
        real_px_m(&p)
    );
}

#[test]
fn from_fn_samples_the_function() {
    let r = ImageryRaster::from_fn(12.0, 47.0, 12.1, 47.1, 0.001, |lat, lon| {
        (lon < 12.05).then_some([(lat * 1000.0 % 200.0) as u8, 0, 0])
    });
    let s = r.sampler().unwrap();
    assert!(s.rgb_at(47.05, 12.02).is_some());
    assert!(s.rgb_at(47.05, 12.08).is_none());
    assert!(s.rgb_at(47.2, 12.02).is_none());
}

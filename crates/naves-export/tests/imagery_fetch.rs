// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fetching imagery into a project, against a fake HTTP layer: no test touches the network.

#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test helper code; failing loudly is the point"
)]

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use naves_core::imagery::ImageryRaster;
use naves_core::imagery::catalog::{CatalogSource, parse_sources};
use naves_core::landcover::GeoWindow;
use naves_core::project::Project;
use naves_export::geotiff_write::{RgbMosaic, write_rgb};
use naves_export::imagery_fetch::{
    FetchError, FetchRequest, Http, default_range, describe_fetch, fetch, picture_grid,
};

/// One request the fake saw: (method, URL, body).
type Seen = (String, String, String);

type Handler = Box<dyn Fn(&str, &str, Option<&str>) -> Result<Vec<u8>, FetchError> + Send + Sync>;

struct Fake {
    handler: Handler,
    seen: Mutex<Vec<Seen>>,
}

impl Fake {
    fn new(
        handler: impl Fn(&str, &str, Option<&str>) -> Result<Vec<u8>, FetchError>
        + Send
        + Sync
        + 'static,
    ) -> Fake {
        Fake {
            handler: Box::new(handler),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn urls(&self, method: &str) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _, _)| m == method)
            .map(|(_, u, _)| u.clone())
            .collect()
    }

    fn bodies(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _, _)| m == "POST")
            .map(|(_, _, b)| b.clone())
            .collect()
    }
}

impl Http for Fake {
    fn get(&self, url: &str, _max_bytes: u64, out: &mut dyn Write) -> Result<(), FetchError> {
        self.seen
            .lock()
            .unwrap()
            .push(("GET".into(), url.into(), String::new()));
        let bytes = (self.handler)("GET", url, None)?;
        out.write_all(&bytes).map_err(|e| FetchError::Http {
            url: url.to_string(),
            why: e.to_string(),
        })
    }

    fn post_json(&self, url: &str, body: &str, _max_bytes: u64) -> Result<Vec<u8>, FetchError> {
        self.seen
            .lock()
            .unwrap()
            .push(("POST".into(), url.into(), body.into()));
        (self.handler)("POST", url, Some(body))
    }
}

fn quiet(_: &str) {}

const CENTRE: (f64, f64) = (47.5570, 12.9743);

/// A project of 500 real metres around the Sentinel-2 crop.
fn project(dir: &Path) -> Project {
    let mut p = Project::new("Test", CENTRE.0, CENTRE.1, 500.0);
    p.grid.terrain_size = 256;
    p.grid.land_size = 32;
    p.imagery.brightness = 1.0;
    p.sources.dem = vec!["dem/koenigssee.tif".into()];
    std::fs::create_dir_all(dir.join("dem")).unwrap();
    std::fs::copy(
        fixture("dem/koenigssee.tif"),
        dir.join("dem/koenigssee.tif"),
    )
    .unwrap();
    p.save(dir).unwrap();
    p
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn wms(extra: &str) -> CatalogSource {
    parse_sources(&format!(
        "[[source]]\nid = \"wms-test\"\nname = \"WMS test\"\nkind = \"wms\"\nurl = \"https://example.org/wms\"\n\
         layer = \"ortho\"\nnative_m = 10.0\nlicence = \"CC BY 4.0\"\nattribution = \"Test credit\"\n{extra}\n"
    ))
    .unwrap()
    .remove(0)
}

fn stac() -> CatalogSource {
    parse_sources(
        "[[source]]\nid = \"stac-test\"\nname = \"STAC test\"\nkind = \"stac\"\nurl = \"https://stac.example.org/v1\"\n\
         collection = \"sentinel-2-c1-l2a\"\nasset = \"visual\"\nasset_hosts = [\"files.example.org\"]\n\
         native_m = 10.0\nlicence = \"Free\"\nattribution = \"Data {year}\"\n",
    )
    .unwrap()
    .remove(0)
}

/// The value of a query parameter of a URL.
fn query(url: &str, key: &str) -> String {
    url.split(['?', '&'])
        .find_map(|p| p.strip_prefix(&format!("{key}=")))
        .unwrap_or_default()
        .to_string()
}

fn size_of(url: &str) -> (u32, u32) {
    (
        query(url, "width").parse().unwrap(),
        query(url, "height").parse().unwrap(),
    )
}

fn bbox_of(url: &str) -> [f64; 4] {
    let v: Vec<f64> = query(url, "bbox")
        .split(',')
        .map(|s| s.parse().unwrap())
        .collect();
    [v[0], v[1], v[2], v[3]]
}

fn png(width: u32, height: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(width, height, |x, y| image::Rgba(f(x, y)));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn jpeg(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(width, height, image::Rgb(rgb));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
    out.into_inner()
}

/// A fake WMS that answers every `GetMap` with `f(x, y)` pixels of the size asked for.
fn wms_fake(f: impl Fn(u32, u32) -> [u8; 4] + Send + Sync + 'static) -> Fake {
    Fake::new(move |_, url, _| {
        let (w, h) = size_of(url);
        Ok(png(w, h, &f))
    })
}

fn listed(dir: &Path) -> Project {
    Project::load(dir).unwrap()
}

fn open(dir: &Path, relative: &str) -> ImageryRaster {
    ImageryRaster::open(&dir.join(relative), &listed(dir), 0.0)
        .unwrap()
        .expect("the file touches the map")
}

// ---- the GeoTIFF writer ----

#[test]
fn written_file_reads_back_with_geotiff_reader() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path());
    // 40 x 20 pixels over a box around the centre; one red pixel at column 30, row 5
    let bounds = GeoWindow {
        south: CENTRE.0 - 0.001,
        north: CENTRE.0 + 0.001,
        west: CENTRE.1 - 0.002,
        east: CENTRE.1 + 0.002,
    };
    let mut data = vec![50_u8; 40 * 20 * 3];
    let at = (5 * 40 + 30) * 3;
    data[at..at + 3].copy_from_slice(&[250, 10, 10]);
    let path = dir.path().join("out/marker.tif");
    write_rgb(
        &path,
        &RgbMosaic {
            width: 40,
            height: 20,
            data,
        },
        bounds,
    )
    .unwrap();
    let raster = ImageryRaster::open(&path, &p, 0.0).unwrap().unwrap();
    let sampler = raster.sampler().unwrap();
    // the centre of the marker pixel
    let lon = bounds.west + (30.5 / 40.0) * (bounds.east - bounds.west);
    let lat = bounds.north - (5.5 / 20.0) * (bounds.north - bounds.south);
    assert_eq!(sampler.rgb_at(lat, lon), Some([250.0, 10.0, 10.0]));
    assert_eq!(sampler.rgb_at(CENTRE.0, CENTRE.1), Some([50.0, 50.0, 50.0]));
    // readable like the project's other files, not the private mode of a temporary file
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o644);
    }
    // no temporary file is left behind
    let left: Vec<_> = std::fs::read_dir(dir.path().join("out"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(left, ["marker.tif"]);
}

// ---- WMS ----

#[test]
fn request_uses_lat_lon_order() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path());
    let http = wms_fake(|_, _| [90, 90, 90, 255]);
    fetch(
        dir.path(),
        &wms(""),
        &FetchRequest::default(),
        &http,
        &quiet,
    )
    .unwrap();
    let urls = http.urls("GET");
    assert_eq!(urls.len(), 1, "one tile covers the small map: {urls:?}");
    let url = &urls[0];
    for part in [
        "service=WMS",
        "version=1.3.0",
        "request=GetMap",
        "layers=ortho",
        "crs=EPSG:4326",
        "format=image/png",
    ] {
        assert!(url.contains(part), "{part} missing from {url}");
    }
    let [first, second, third, fourth] = bbox_of(url);
    // WMS 1.3.0 with EPSG:4326: south, west, north, east
    assert!(
        (first - (p.area.center_lat - 0.0035)).abs() < 0.001,
        "{url}"
    );
    assert!(
        (second - (p.area.center_lon - 0.0052)).abs() < 0.001,
        "{url}"
    );
    assert!(third > first && fourth > second);
}

fn small_tiles() -> CatalogSource {
    wms("max_px = 64\n")
}

#[test]
fn tiles_cover_the_window_without_gap_or_overlap() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let http = wms_fake(|_, _| [90, 90, 90, 255]);
    fetch(
        dir.path(),
        &small_tiles(),
        &FetchRequest::default(),
        &http,
        &quiet,
    )
    .unwrap();
    let urls = http.urls("GET");
    assert!(urls.len() >= 4, "{} tiles", urls.len());
    let boxes: Vec<[f64; 4]> = urls.iter().map(|u| bbox_of(u)).collect();
    let (south, west) = (
        boxes.iter().map(|b| b[0]).fold(f64::MAX, f64::min),
        boxes.iter().map(|b| b[1]).fold(f64::MAX, f64::min),
    );
    let (north, east) = (
        boxes.iter().map(|b| b[2]).fold(f64::MIN, f64::max),
        boxes.iter().map(|b| b[3]).fold(f64::MIN, f64::max),
    );
    // every tile edge is the edge of another tile or of the window; the areas add up to the window
    let area: f64 = boxes.iter().map(|b| (b[2] - b[0]) * (b[3] - b[1])).sum();
    assert!(
        (area - (north - south) * (east - west)).abs() < 1e-12,
        "gaps or overlaps: {area} against {}",
        (north - south) * (east - west)
    );
    for b in &boxes {
        let shares_west =
            (b[1] - west).abs() < 1e-12 || boxes.iter().any(|o| (o[3] - b[1]).abs() < 1e-12);
        let shares_south =
            (b[0] - south).abs() < 1e-12 || boxes.iter().any(|o| (o[2] - b[0]).abs() < 1e-12);
        assert!(shares_west && shares_south, "{b:?} has a gap next to it");
    }
}

#[test]
fn a_large_map_is_fetched_coarser() {
    // terrain 4096 means 512 land cells: at the map's own pixel the picture would be 33 000 pixels across,
    // more than the 16 000 that are accepted; a service can draw at any scale, so the pixel gets larger
    let dir = tempfile::tempdir().unwrap();
    let mut p = project(dir.path());
    p.grid.terrain_size = 4096;
    p.grid.land_size = 512;
    p.area.real_extent_m = 12_800.0;
    p.save(dir.path()).unwrap();
    let sharp = parse_sources(
        "[[source]]\nid = \"sharp\"\nname = \"Sharp\"\nkind = \"wms\"\nurl = \"https://example.org/wms\"\n\
         layer = \"l\"\nnative_m = 0.4\nlicence = \"CC BY 4.0\"\nattribution = \"A\"\n",
    )
    .unwrap()
    .remove(0);
    let (cols, rows, pixel_m) = picture_grid(&p, &sharp).unwrap();
    assert!(cols <= 16_000 && rows <= 16_000, "{cols} x {rows}");
    assert!(cols > 15_000, "as sharp as is allowed: {cols}");
    assert!(
        pixel_m > 0.8,
        "coarser than the map's own 0.42 m: {pixel_m}"
    );
    let said = describe_fetch(&p, &sharp).unwrap();
    assert!(said.contains("coarser"), "the user is told: {said}");
    // a small map is fetched at the sharper of the source's and the map's pixel, as before
    let small = project(dir.path());
    let (_, _, pixel_m) = picture_grid(&small, &sharp).unwrap();
    assert!((pixel_m - 0.4).abs() < 0.3, "{pixel_m}");
}

#[test]
fn max_px_is_respected() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let http = wms_fake(|_, _| [90, 90, 90, 255]);
    fetch(
        dir.path(),
        &small_tiles(),
        &FetchRequest::default(),
        &http,
        &quiet,
    )
    .unwrap();
    for url in http.urls("GET") {
        let (w, h) = size_of(&url);
        assert!(w <= 64 && h <= 64 && w > 0 && h > 0, "{url}");
    }
    // a service that allows more than 2048 still gets at most 2048
    let dir = tempfile::tempdir().unwrap();
    let mut p = project(dir.path());
    p.area.real_extent_m = 20_000.0;
    p.save(dir.path()).unwrap();
    let http = wms_fake(|_, _| [90, 90, 90, 255]);
    fetch(
        dir.path(),
        &wms("max_px = 6000\n"),
        &FetchRequest::default(),
        &http,
        &quiet,
    )
    .unwrap();
    for url in http.urls("GET") {
        let (w, h) = size_of(&url);
        assert!(w <= 2048 && h <= 2048, "{url}");
    }
}

#[test]
fn jpeg_answer_to_a_png_request_is_read() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let http = Fake::new(|_, url, _| {
        let (w, h) = size_of(url);
        Ok(jpeg(w, h, [200, 120, 40]))
    });
    fetch(
        dir.path(),
        &wms(""),
        &FetchRequest::default(),
        &http,
        &quiet,
    )
    .unwrap();
    let raster = open(dir.path(), "imagery/wms-test.tif");
    let c = raster
        .sampler()
        .unwrap()
        .rgb_at(CENTRE.0, CENTRE.1)
        .unwrap();
    // JPEG is lossy
    assert!(
        (c[0] - 200.0).abs() < 6.0 && (c[1] - 120.0).abs() < 6.0 && (c[2] - 40.0).abs() < 6.0,
        "{c:?}"
    );
}

#[test]
fn blank_pixels_are_no_data() {
    // the western half is transparent, a stripe is the configured blank colour, the rest is ground
    for (extra, blank_pixel) in [
        ("", [10, 10, 10, 0]),
        ("blank = [255, 255, 255]\n", [255, 255, 255, 255]),
    ] {
        let dir = tempfile::tempdir().unwrap();
        project(dir.path());
        let http = Fake::new(move |_, url, _| {
            let (w, h) = size_of(url);
            Ok(png(w, h, |x, _| {
                if x < w / 2 {
                    blank_pixel
                } else {
                    [90, 120, 60, 255]
                }
            }))
        });
        fetch(
            dir.path(),
            &wms(extra),
            &FetchRequest::default(),
            &http,
            &quiet,
        )
        .unwrap();
        let raster = open(dir.path(), "imagery/wms-test.tif");
        let sampler = raster.sampler().unwrap();
        assert!(
            sampler.rgb_at(CENTRE.0, CENTRE.1 - 0.002).is_none(),
            "{extra:?}: blank is no data"
        );
        assert_eq!(
            sampler.rgb_at(CENTRE.0, CENTRE.1 + 0.002),
            Some([90.0, 120.0, 60.0]),
            "{extra:?}"
        );
    }
}

#[test]
fn black_stays_valid() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let http = wms_fake(|_, _| [0, 0, 0, 255]);
    fetch(
        dir.path(),
        &wms(""),
        &FetchRequest::default(),
        &http,
        &quiet,
    )
    .unwrap();
    let raster = open(dir.path(), "imagery/wms-test.tif");
    let c = raster.sampler().unwrap().rgb_at(CENTRE.0, CENTRE.1);
    assert_eq!(
        c,
        Some([1.0, 1.0, 1.0]),
        "a real black pixel is not taken for no data"
    );
}

#[test]
fn service_error_xml_is_an_error_naming_the_source() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let http = Fake::new(|_, _, _| {
        Ok(b"<?xml version=\"1.0\"?><ServiceExceptionReport>Invalid request</ServiceExceptionReport>".to_vec())
    });
    let err = fetch(
        dir.path(),
        &wms(""),
        &FetchRequest::default(),
        &http,
        &quiet,
    )
    .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("wms-test") && text.contains("not an image"),
        "{text}"
    );
    assert!(
        text.contains("Invalid request"),
        "the service's words help: {text}"
    );
}

#[test]
fn empty_area_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let http = wms_fake(|_, _| [0, 0, 0, 0]);
    let err = fetch(
        dir.path(),
        &wms(""),
        &FetchRequest::default(),
        &http,
        &quiet,
    )
    .unwrap_err();
    assert!(err.to_string().contains("nothing for this area"), "{err}");
    assert!(listed(dir.path()).sources.imagery.is_empty());
    assert!(!dir.path().join("imagery/wms-test.tif").exists());
}

#[test]
fn oversized_answer_is_cut_off() {
    struct Flood;
    impl Http for Flood {
        fn get(&self, _: &str, _max: u64, out: &mut dyn Write) -> Result<(), FetchError> {
            // a server that ignores the limit: fetch must not keep what exceeds it
            let chunk = vec![0_u8; 1 << 20];
            for _ in 0..200 {
                out.write_all(&chunk).map_err(|e| FetchError::Http {
                    url: "flood".into(),
                    why: e.to_string(),
                })?;
            }
            Ok(())
        }
        fn post_json(&self, _: &str, _: &str, _: u64) -> Result<Vec<u8>, FetchError> {
            Ok(vec![b' '; 100 << 20])
        }
    }
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let err = fetch(
        dir.path(),
        &wms(""),
        &FetchRequest::default(),
        &Flood,
        &quiet,
    )
    .unwrap_err();
    assert!(err.to_string().contains("larger than"), "{err}");
    let err = fetch(
        dir.path(),
        &stac(),
        &FetchRequest::default(),
        &Flood,
        &quiet,
    )
    .unwrap_err();
    assert!(err.to_string().contains("larger than"), "{err}");
}

// ---- STAC ----

/// A scene as the search answers it: a polygon in (lon, lat) and the asset address.
fn feature(
    id: &str,
    cloud: f64,
    [west, south, east, north]: [f64; 4],
    href: &str,
    datetime: &str,
) -> String {
    format!(
        r#"{{"id":"{id}","type":"Feature","geometry":{{"type":"Polygon","coordinates":[[[{west},{south}],[{east},{south}],[{east},{north}],[{west},{north}],[{west},{south}]]]}},
        "properties":{{"datetime":"{datetime}","eo:cloud_cover":{cloud},"s2:snow_ice_percentage":0.5}},
        "assets":{{"visual":{{"href":"{href}"}}}}}}"#
    )
}

fn everything() -> [f64; 4] {
    [12.9, 47.5, 13.05, 47.6]
}

fn stac_fake(features: Vec<String>) -> Fake {
    let crop = std::fs::read(fixture("imagery/sentinel2_crop.tif")).unwrap();
    Fake::new(move |method, _url, _| {
        if method == "POST" {
            Ok(format!(
                r#"{{"type":"FeatureCollection","features":[{}]}}"#,
                features.join(",")
            )
            .into_bytes())
        } else {
            Ok(crop.clone())
        }
    })
}

fn good(id: &str, cloud: f64, area: [f64; 4]) -> String {
    feature(
        id,
        cloud,
        area,
        &format!("https://files.example.org/{id}/TCI.tif"),
        "2025-09-19T10:17:51Z",
    )
}

fn dated() -> FetchRequest {
    FetchRequest {
        from: Some("2025-05-01".into()),
        to: Some("2025-09-30".into()),
    }
}

#[test]
fn search_uses_area_dates_and_order() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let http = stac_fake(vec![good("A", 0.1, everything())]);
    fetch(dir.path(), &stac(), &dated(), &http, &quiet).unwrap();
    let bodies = http.bodies();
    assert_eq!(bodies.len(), 1, "{bodies:?}");
    let body: serde_json::Value = serde_json::from_str(&bodies[0]).unwrap();
    assert_eq!(body["collections"][0], "sentinel-2-c1-l2a");
    assert_eq!(
        body["datetime"],
        "2025-05-01T00:00:00Z/2025-09-30T23:59:59Z"
    );
    assert_eq!(body["sortby"][0]["field"], "properties.eo:cloud_cover");
    assert_eq!(body["sortby"][0]["direction"], "asc");
    assert_eq!(
        body["sortby"][1]["field"],
        "properties.s2:snow_ice_percentage"
    );
    // the polygon is the window of the map: it contains the centre and is a closed ring of (lon, lat)
    let ring = body["intersects"]["coordinates"][0].as_array().unwrap();
    assert_eq!(ring.first(), ring.last());
    let lons: Vec<f64> = ring.iter().map(|p| p[0].as_f64().unwrap()).collect();
    let lats: Vec<f64> = ring.iter().map(|p| p[1].as_f64().unwrap()).collect();
    let span = |v: &[f64]| {
        (
            v.iter().copied().fold(f64::MIN, f64::max),
            v.iter().copied().fold(f64::MAX, f64::min),
        )
    };
    assert!(span(&lons).1 < CENTRE.1 && CENTRE.1 < span(&lons).0);
    assert!(span(&lats).1 < CENTRE.0 && CENTRE.0 < span(&lats).0);
}

#[test]
fn clearest_scenes_are_taken_until_covered() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let west = [12.9, 47.5, 12.9743, 47.6];
    let east = [12.9743, 47.5, 13.05, 47.6];
    let http = stac_fake(vec![
        good("C-all", 3.0, everything()),
        good("B-east", 0.5, east),
        good("D-cloudy", 30.0, everything()),
        good("A-west", 0.1, west),
    ]);
    fetch(dir.path(), &stac(), &dated(), &http, &quiet).unwrap();
    let downloads = http.urls("GET");
    assert_eq!(downloads.len(), 2, "{downloads:?}");
    assert!(
        downloads[0].contains("A-west") && downloads[1].contains("B-east"),
        "{downloads:?}"
    );
    assert!(dir.path().join("imagery/stac-test-A-west.tif").is_file());
    assert!(dir.path().join("imagery/stac-test-B-east.tif").is_file());
    assert!(!dir.path().join("imagery/stac-test-C-all.tif").exists());
    // the downloads are gone, only the crops stay
    let names: Vec<String> = std::fs::read_dir(dir.path().join("imagery"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().all(|n| std::path::Path::new(n)
            .extension()
            .is_some_and(|e| e == "tif")
            && !n.starts_with('.')),
        "{names:?}"
    );
}

#[test]
fn stops_at_max_scenes_and_warns() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    // ten scenes, each covering a different thin strip: the window is never covered
    let scenes: Vec<String> = (0..10)
        .map(|i| {
            let x = 12.9690 + f64::from(i) * 0.0010;
            good(
                &format!("S{i}"),
                0.1 + f64::from(i),
                [x, 47.5, x + 0.0005, 47.6],
            )
        })
        .collect();
    let http = stac_fake(scenes);
    let lines = Mutex::new(Vec::new());
    // each strip covers a few points of the window; together they never cover all of it
    let result = fetch(dir.path(), &stac(), &dated(), &http, &|l| {
        lines.lock().unwrap().push(l.to_string());
    });
    result.unwrap();
    assert!(
        http.urls("GET").len() == 6,
        "exactly six scenes of ten: {}",
        http.urls("GET").len()
    );
    let lines = lines.lock().unwrap();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("warning") && l.contains('%')),
        "{lines:?}"
    );
}

#[test]
fn only_cloudy_scenes_names_the_clearest_and_suggests_other_dates() {
    // the rainy season of a place: every scene is above the limit
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let http = stac_fake(vec![
        good("A", 55.0, everything()),
        good("B", 41.0, everything()),
    ]);
    let err = fetch(dir.path(), &stac(), &dated(), &http, &quiet).unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("41 %") && text.contains("20 %"),
        "the clearest and the limit: {text}"
    );
    assert!(
        text.contains("--from") && text.contains("--to"),
        "what to do: {text}"
    );
    assert!(http.urls("GET").is_empty());
}

#[test]
fn southern_map_defaults_to_southern_summer() {
    assert_eq!(
        default_range(47.0, (2026, 11, 6)),
        [
            ("2024-05-01".to_string(), "2024-09-30".to_string()),
            ("2025-05-01".to_string(), "2025-09-30".to_string())
        ]
    );
    assert_eq!(
        default_range(-33.0, (2026, 11, 6)),
        [
            ("2023-11-01".to_string(), "2024-03-31".to_string()),
            ("2024-11-01".to_string(), "2025-03-31".to_string())
        ]
    );
}

#[test]
fn foreign_asset_host_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let bad = feature(
        "A",
        0.1,
        everything(),
        "https://evil.example.com/TCI.tif",
        "2025-09-19T10:17:51Z",
    );
    let http = stac_fake(vec![bad]);
    let err = fetch(dir.path(), &stac(), &dated(), &http, &quiet).unwrap_err();
    assert!(http.urls("GET").is_empty(), "nothing was downloaded");
    assert!(
        err.to_string().contains("no usable scene") || err.to_string().contains("host"),
        "{err}"
    );
    // plain http is rejected as well
    let plain = feature(
        "A",
        0.1,
        everything(),
        "http://files.example.org/TCI.tif",
        "2025-09-19T10:17:51Z",
    );
    let http = stac_fake(vec![plain]);
    assert!(fetch(dir.path(), &stac(), &dated(), &http, &quiet).is_err());
    assert!(http.urls("GET").is_empty());
}

#[test]
fn hostile_scene_id_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let hostile = good("../../x", 0.1, everything());
    let http = stac_fake(vec![hostile]);
    assert!(fetch(dir.path(), &stac(), &dated(), &http, &quiet).is_err());
    assert!(http.urls("GET").is_empty());
    assert!(
        !dir.path().join("x.tif").exists() && !dir.path().parent().unwrap().join("x.tif").exists()
    );
}

#[test]
fn damaged_json_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    for body in ["{", "[]", r#"{"features": 3}"#, ""] {
        let http = Fake::new(move |_, _, _| Ok(body.as_bytes().to_vec()));
        let err = fetch(dir.path(), &stac(), &dated(), &http, &quiet).unwrap_err();
        assert!(err.to_string().contains("stac-test"), "{body:?}: {err}");
    }
}

#[test]
fn attribution_gets_the_year_of_the_scene() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let http = stac_fake(vec![feature(
        "A",
        0.1,
        everything(),
        "https://files.example.org/A/TCI.tif",
        "2023-07-04T10:00:00Z",
    )]);
    let files = fetch(dir.path(), &stac(), &dated(), &http, &quiet).unwrap();
    assert_eq!(files[0].attribution, "Data 2023");
}

// ---- the project ----

#[test]
fn fetched_files_are_listed_with_licence() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let http = wms_fake(|_, _| [90, 90, 90, 255]);
    let nc = wms("").clone();
    let nc = CatalogSource {
        licence: "CC BY-NC-SA 4.0".into(),
        non_commercial: true,
        ..nc
    };
    let files = fetch(dir.path(), &nc, &FetchRequest::default(), &http, &quiet).unwrap();
    assert_eq!(files.len(), 1);
    let saved = listed(dir.path()).sources.imagery;
    assert_eq!(saved, files);
    assert_eq!(saved[0].path, PathBuf::from("imagery/wms-test.tif"));
    assert_eq!(saved[0].licence, "CC BY-NC-SA 4.0");
    assert!(saved[0].non_commercial);
    assert_eq!(saved[0].attribution, "Test credit");
}

#[test]
fn fetching_twice_lists_once() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let http = wms_fake(|_, _| [90, 90, 90, 255]);
    fetch(
        dir.path(),
        &wms(""),
        &FetchRequest::default(),
        &http,
        &quiet,
    )
    .unwrap();
    let http = wms_fake(|_, _| [120, 120, 120, 255]);
    fetch(
        dir.path(),
        &wms(""),
        &FetchRequest::default(),
        &http,
        &quiet,
    )
    .unwrap();
    assert_eq!(listed(dir.path()).sources.imagery.len(), 1);
    // and the file is the new one
    let c = open(dir.path(), "imagery/wms-test.tif")
        .sampler()
        .unwrap()
        .rgb_at(CENTRE.0, CENTRE.1);
    assert_eq!(c, Some([120.0, 120.0, 120.0]));
}

#[test]
fn failed_fetch_leaves_the_project_and_old_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let http = wms_fake(|_, _| [90, 90, 90, 255]);
    fetch(
        dir.path(),
        &wms(""),
        &FetchRequest::default(),
        &http,
        &quiet,
    )
    .unwrap();
    let toml_before = std::fs::read(dir.path().join("naves.toml")).unwrap();
    let file_before = std::fs::read(dir.path().join("imagery/wms-test.tif")).unwrap();
    let failing = Fake::new(|_, _, _| Ok(b"<ServiceException/>".to_vec()));
    assert!(
        fetch(
            dir.path(),
            &wms(""),
            &FetchRequest::default(),
            &failing,
            &quiet
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read(dir.path().join("naves.toml")).unwrap(),
        toml_before
    );
    assert_eq!(
        std::fs::read(dir.path().join("imagery/wms-test.tif")).unwrap(),
        file_before
    );
}

#[test]
fn project_stays_portable() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let http = stac_fake(vec![good("A", 0.1, everything())]);
    fetch(dir.path(), &stac(), &dated(), &http, &quiet).unwrap();
    let text = std::fs::read_to_string(dir.path().join("naves.toml")).unwrap();
    assert!(
        text.contains("path = \"imagery/stac-test-A.tif\""),
        "{text}"
    );
    assert!(
        !text.contains(dir.path().to_str().unwrap()),
        "no absolute path: {text}"
    );
}

#[cfg(unix)]
#[test]
fn fetched_project_builds_offline() {
    use naves_export::build::build;
    use naves_export::testing::FakeEnv;
    let dir = tempfile::tempdir().unwrap();
    let proj = dir.path().join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    project(&proj);
    let http = wms_fake(|x, _| {
        if x % 2 == 0 {
            [60, 90, 50, 255]
        } else {
            [70, 100, 60, 255]
        }
    });
    fetch(&proj, &wms(""), &FetchRequest::default(), &http, &quiet).unwrap();
    // `build` takes no `Http`: it cannot fetch
    let env = FakeEnv::new(&dir.path().join("fake")).unwrap();
    let out = build(&proj, &env.tools(), &quiet).unwrap();
    let credits = std::fs::read_to_string(out.mod_dir.join("ATTRIBUTION.txt")).unwrap();
    assert!(credits.contains("Test credit"), "{credits}");
}

// ---- the real client, against a server on this machine ----

use naves_export::imagery_fetch::RealHttp;

/// A server on the loopback interface that answers one request with `body` and records what it received.
fn serve_once(body: Vec<u8>) -> (String, std::sync::Arc<Mutex<String>>) {
    use std::io::Read;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/path", listener.local_addr().unwrap());
    let received = std::sync::Arc::new(Mutex::new(String::new()));
    let seen = std::sync::Arc::clone(&received);
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buf = [0_u8; 4096];
        // the head, then as much body as the head announces
        loop {
            let n = stream.read(&mut buf).unwrap();
            request.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&request).to_lowercase();
            if let Some(end) = text.find("\r\n\r\n") {
                let announced = text
                    .split("content-length: ")
                    .nth(1)
                    .and_then(|s| s.split("\r\n").next())
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                if request.len() >= end + 4 + announced || n == 0 {
                    break;
                }
            } else if n == 0 {
                break;
            }
        }
        *seen.lock().unwrap() = String::from_utf8_lossy(&request).into_owned();
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        // the client may have stopped reading already
        let _ = stream.write_all(&body);
    });
    (url, received)
}

#[test]
fn real_http_sends_the_user_agent_and_reads_the_answer() {
    let (url, received) = serve_once(b"hello".to_vec());
    let mut out = Vec::new();
    RealHttp::plain_for_tests()
        .get(&url, 1000, &mut out)
        .unwrap();
    assert_eq!(out, b"hello");
    let head = received.lock().unwrap().to_lowercase();
    assert!(
        head.contains(&format!("user-agent: naves/{}", env!("CARGO_PKG_VERSION"))),
        "{head}"
    );
}

#[test]
fn real_http_cuts_off_an_answer_above_the_limit() {
    let (url, _) = serve_once(vec![7_u8; 5000]);
    let mut out = Vec::new();
    let err = RealHttp::plain_for_tests()
        .get(&url, 1000, &mut out)
        .unwrap_err();
    assert!(err.to_string().contains("larger than"), "{err}");
    assert!(
        out.len() <= 1000,
        "nothing beyond the limit is kept: {}",
        out.len()
    );
}

#[test]
fn real_http_posts_json() {
    let (url, received) = serve_once(b"{\"features\":[]}".to_vec());
    let answer = RealHttp::plain_for_tests()
        .post_json(&url, "{\"a\":1}", 1000)
        .unwrap();
    assert_eq!(answer, b"{\"features\":[]}");
    let request = received.lock().unwrap().to_lowercase();
    assert!(request.starts_with("post /path"), "{request}");
    assert!(
        request.contains("content-type: application/json"),
        "{request}"
    );
    assert!(request.ends_with("{\"a\":1}"), "{request}");
}

#[test]
fn real_http_refuses_plain_http_by_default() {
    let (url, _) = serve_once(b"x".to_vec());
    let mut out = Vec::new();
    assert!(RealHttp::new().get(&url, 1000, &mut out).is_err());
    assert!(out.is_empty());
}

// ---- the real services (run by hand: cargo test -p naves-export -- --ignored real_) ----

#[test]
#[ignore = "needs network"]
fn real_wms_tile() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let eox = naves_core::imagery::catalog::catalog(None)
        .unwrap()
        .into_iter()
        .find(|s| s.id == "eox-cloudless-2017")
        .unwrap();
    let files = fetch(
        dir.path(),
        &eox,
        &FetchRequest::default(),
        &RealHttp::new(),
        &quiet,
    )
    .unwrap();
    assert_eq!(files.len(), 1);
    // the crop of the Sentinel-2 scene and the mosaic show the same lake: dark water at the centre of the crop
    let raster = open(dir.path(), "imagery/eox-cloudless-2017.tif");
    let c = raster.sampler().unwrap().rgb_at(47.5571, 12.9743).unwrap();
    assert!(c.iter().all(|v| *v < 120.0), "the Königssee is dark: {c:?}");
}

#[test]
#[ignore = "needs network"]
fn real_sentinel_search() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path());
    let source = naves_core::imagery::catalog::catalog(None)
        .unwrap()
        .into_iter()
        .find(|s| s.id == "sentinel2-scenes")
        .unwrap();
    let naves_core::imagery::catalog::SourceKind::Stac {
        url, collection, ..
    } = &source.kind
    else {
        panic!("a STAC source");
    };
    let window = naves_core::heightmap::MapGeo::new(&p)
        .unwrap()
        .window(50.0)
        .unwrap();
    let body = serde_json::json!({
        "collections": [collection],
        "intersects": {"type": "Polygon", "coordinates": [[
            [window.west, window.south], [window.east, window.south],
            [window.east, window.north], [window.west, window.north], [window.west, window.south]]]},
        "datetime": "2025-05-01T00:00:00Z/2025-09-30T23:59:59Z",
        "limit": 3,
    })
    .to_string();
    let answer = RealHttp::new()
        .post_json(&format!("{url}/search"), &body, 1 << 20)
        .unwrap();
    let count =
        naves_export::imagery_fetch::search_scene_count(&source, "visual", &answer).unwrap();
    assert!(count > 0, "the search finds scenes");
}

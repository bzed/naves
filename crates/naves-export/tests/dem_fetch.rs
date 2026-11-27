// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Writing a fetched DEM as one-band Float32 GeoTIFF and reading it back with `naves_core::dem`: no network,
//! every file lives under a temporary directory.

#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use image::ImageEncoder as _;
use naves_core::dem::Dem;
use naves_core::heightmap::MapGeo;
use naves_core::landcover::GeoWindow;
use naves_core::project::Project;
use naves_export::dem_fetch::{
    DEFAULT_PIXEL_M, MARGIN_M, MAX_PIXELS, MAX_SIDE_PX, TILE_PX, TilePyramid, TileStore, dem_grid,
    fetch_window, resample, terrarium, tile_range, world_px,
};
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

/// Latitude and longitude of the north-east corner (inside the z15 parent) of tile `(x, y)` at zoom `z` (Web Mercator).
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

/// A lossless webp tile of `TILE_PX` squared pixels whose height at (column, row) is `height(column, row)`.
fn terrarium_webp(side: u32, height: impl Fn(u32, u32) -> f64) -> Vec<u8> {
    let mut rgb = Vec::new();
    for row in 0..side {
        for col in 0..side {
            let v = height(col, row) + 32768.0;
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "test data in range"
            )]
            rgb.extend([
                (v / 256.0).floor() as u8,
                (v.floor() % 256.0) as u8,
                (v.fract() * 256.0).floor() as u8,
            ]);
        }
    }
    let mut out = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut out)
        .write_image(&rgb, side, side, image::ExtendedColorType::Rgb8)
        .unwrap();
    out
}

#[test]
fn terrarium_decodes_the_real_lake_tile() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/dem/mapterhorn_z16_35130_22901.webp");
    let heights = terrarium(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(heights.len(), (TILE_PX * TILE_PX) as usize);
    assert!(
        (heights[0] - 603.156_25).abs() < 1e-3,
        "first was {}",
        heights[0]
    );
    for h in &heights {
        assert!((603.031..=603.407).contains(h), "height {h}");
    }
}

#[test]
fn terrarium_round_trips_a_ramp() {
    let ramp = |c: u32, r: u32| -50.0 + f64::from(c) * 3.37 + f64::from(r) * 0.011;
    let heights = terrarium(&terrarium_webp(TILE_PX, ramp)).unwrap();
    for (row, col) in [(0u32, 0u32), (0, 511), (511, 0), (200, 300), (511, 511)] {
        let got = f64::from(heights[(row * TILE_PX + col) as usize]);
        assert!(
            (got - ramp(col, row)).abs() <= 1.0 / 256.0 + 1e-3,
            "{row},{col}: {got}"
        );
    }
}

#[test]
fn terrarium_rejects_wrong_size_and_garbage() {
    assert!(matches!(
        terrarium(&terrarium_webp(256, |_, _| 10.0)),
        Err(FetchError::Other(_))
    ));
    assert!(terrarium(b"not a webp at all, some random bytes 1234567890").is_err());
    assert!(terrarium(&[]).is_err());
}

/// Both tiles of a pyramid for the z16 tile 35130/22901 and its z15 parent, constant 1000 and 2000 m.
fn two_zoom_pyramid() -> TilePyramid {
    let mut pyramid = TilePyramid::default();
    pyramid
        .insert(16, 35130, 22901, &terrarium_webp(TILE_PX, |_, _| 1000.0))
        .unwrap();
    pyramid
        .insert(15, 17565, 11450, &terrarium_webp(TILE_PX, |_, _| 2000.0))
        .unwrap();
    pyramid
}

#[test]
fn sample_prefers_native_zoom_and_falls_back_on_an_incomplete_quad() {
    let pyramid = two_zoom_pyramid();
    let (lat0, lon0) = tile_corner(16, 35130, 22901);
    let (lat1, lon1) = tile_corner(16, 35131, 22902);
    // the middle of the z16 tile: the native value
    let inner = pyramid
        .sample(f64::midpoint(lat0, lat1), f64::midpoint(lon0, lon1), 16)
        .unwrap();
    assert!((inner - 1000.0).abs() < 1e-3, "inner {inner}");
    // the north-east corner (inside the z15 parent): the quad needs absent z16 neighbours, so the parent answers
    let edge = pyramid.sample(lat0, lon1, 16).unwrap();
    assert!((edge - 2000.0).abs() < 1e-3, "edge {edge}");
    // a coarser zoom_max never looks at z16
    let coarse = pyramid
        .sample(f64::midpoint(lat0, lat1), f64::midpoint(lon0, lon1), 15)
        .unwrap();
    assert!((coarse - 2000.0).abs() < 1e-3, "coarse {coarse}");
}

#[test]
fn sample_of_an_uncovered_point_is_none() {
    let pyramid = two_zoom_pyramid();
    assert!(pyramid.sample(-30.0, 100.0, 16).is_none());
    assert!(pyramid.sample(f64::NAN, 10.0, 16).is_none());
    assert!(TilePyramid::default().sample(47.0, 12.0, 18).is_none());
}

#[test]
fn insert_refuses_a_bad_tile() {
    let mut pyramid = TilePyramid::default();
    assert!(pyramid.insert(16, 1, 1, b"garbage").is_err());
}

/// A tile store over a map that counts the loads of every tile.
#[derive(Default)]
struct FakeStore {
    tiles: BTreeMap<(u8, u32, u32), Vec<u8>>,
    loads: Mutex<BTreeMap<(u8, u32, u32), usize>>,
}

impl TileStore for FakeStore {
    fn load(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, FetchError> {
        *self.loads.lock().unwrap().entry((z, x, y)).or_default() += 1;
        Ok(self.tiles.get(&(z, x, y)).cloned())
    }
}

impl FakeStore {
    fn total_loads(&self) -> usize {
        self.loads.lock().unwrap().values().sum()
    }
}

/// Height as a function of latitude and longitude only (so it is the same in every tile).
fn ramp_of(lat: f64, lon: f64) -> f64 {
    100.0 + (lat - 47.0) * 1000.0 + (lon - 12.0) * 500.0
}

/// A store with all tiles of `x0..=x1`, `y0..=y1` at `z`, each holding the ramp at its pixel centres.
fn ramp_store(z: u8, x0: u32, x1: u32, y0: u32, y1: u32) -> FakeStore {
    let mut store = FakeStore::default();
    let n = f64::from(1u32 << z);
    let size = n * f64::from(TILE_PX);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let webp = terrarium_webp(TILE_PX, |c, r| {
                let wx = f64::from(x) * f64::from(TILE_PX) + f64::from(c) + 0.5;
                let wy = f64::from(y) * f64::from(TILE_PX) + f64::from(r) + 0.5;
                let lon = wx / size * 360.0 - 180.0;
                let lat = (std::f64::consts::PI * (1.0 - 2.0 * wy / size))
                    .sinh()
                    .atan()
                    .to_degrees();
                ramp_of(lat, lon)
            });
            store.tiles.insert((z, x, y), webp);
        }
    }
    store
}

/// A window of `rows` tile rows at z16 around the fixture tile, a little inside its edges.
fn ramp_window(tile_rows: u32) -> GeoWindow {
    let (north, west) = tile_corner(16, 35130, 22901);
    let (south, east) = tile_corner(16, 35131, 22901 + tile_rows);
    let dy = (north - south) * 0.1;
    let dx = (east - west) * 0.1;
    GeoWindow {
        south: south + dy,
        west: west + dx,
        north: north - dy,
        east: east - dx,
    }
}

#[test]
fn resample_matches_the_ramp_inside_one_tile() {
    let window = ramp_window(1);
    let store = ramp_store(16, 35129, 35131, 22900, 22903);
    let (mosaic, bounds) = resample(&store, &window, 20.0, 16).unwrap();
    assert!(mosaic.width > 1 && mosaic.height > 1);
    assert_eq!(mosaic.data.len(), (mosaic.width * mosaic.height) as usize);
    let sx = (bounds.east - bounds.west) / f64::from(mosaic.width);
    let sy = (bounds.north - bounds.south) / f64::from(mosaic.height);
    assert_eq!(bounds.west.to_bits(), window.west.to_bits());
    assert_eq!(bounds.north.to_bits(), window.north.to_bits());
    for (c, r) in [
        (0, 0),
        (mosaic.width - 1, 0),
        (0, mosaic.height - 1),
        (mosaic.width - 1, mosaic.height - 1),
    ] {
        let lat = bounds.north - (f64::from(r) + 0.5) * sy;
        let lon = bounds.west + (f64::from(c) + 0.5) * sx;
        let got = f64::from(mosaic.data[(r * mosaic.width + c) as usize]);
        // the ramp is not exactly linear in Web Mercator; the tolerance covers 1/256 m and that
        assert!(
            (got - ramp_of(lat, lon)).abs() < 0.05,
            "({c},{r}): {got} vs {}",
            ramp_of(lat, lon)
        );
    }
}

#[test]
fn resample_is_continuous_across_band_borders_and_loads_each_tile_at_most_three_times() {
    let window = ramp_window(4);
    let store = ramp_store(16, 35129, 35131, 22900, 22906);
    let (mosaic, _) = resample(&store, &window, 30.0, 16).unwrap();
    let w = mosaic.width as usize;
    // the ramp falls by about 1000 m per degree of latitude: one row is a step of this size
    let (south, north) = (window.south, window.north);
    let step = (north - south) * 1000.0 / f64::from(mosaic.height);
    for r in 1..mosaic.height as usize {
        let jump = f64::from(mosaic.data[(r - 1) * w] - mosaic.data[r * w]);
        assert!(
            (jump - step).abs() < 0.5 * step + 0.05,
            "row {r}: jump {jump}, step {step}"
        );
    }
    let loads = store.loads.lock().unwrap();
    assert!(loads.values().all(|n| *n <= 3), "loads: {loads:?}");
    assert!(loads.keys().all(|(z, ..)| *z == 16));
}

#[test]
fn dem_grid_of_the_example_is_two_and_a_half_metres() {
    let window = fetch_window(&example_project()).unwrap();
    let (cols, rows, pixel_m) = dem_grid(&window, None).unwrap();
    assert!((pixel_m - DEFAULT_PIXEL_M).abs() < 1e-12, "pixel {pixel_m}");
    assert!(cols <= MAX_SIDE_PX && rows <= MAX_SIDE_PX);
    assert!(u64::from(cols) * u64::from(rows) <= MAX_PIXELS);
}

#[test]
fn dem_grid_grows_a_large_map_to_just_fit() {
    let mut p = Project::new("T", 47.571_769, 12.946_451, 24_500.0);
    p.scale.horizontal = 0.5;
    let window = fetch_window(&p).unwrap();
    let (cols, rows, pixel_m) = dem_grid(&window, None).unwrap();
    assert!(pixel_m > DEFAULT_PIXEL_M, "pixel {pixel_m}");
    assert!(cols <= MAX_SIDE_PX && rows <= MAX_SIDE_PX);
    assert!(u64::from(cols) * u64::from(rows) <= MAX_PIXELS);
    // within 1 % of the smallest pixel that fits: a pixel 1 % smaller does not
    let (c2, r2, _) = grid_at(&window, pixel_m * 0.99);
    assert!(
        c2 > MAX_SIDE_PX || r2 > MAX_SIDE_PX || u64::from(c2) * u64::from(r2) > MAX_PIXELS,
        "a 1 % smaller pixel still fits: {c2} x {r2}"
    );
    // a pixel that is requested and already fits stays
    let (_, _, same) = dem_grid(&window, Some(pixel_m * 1.5)).unwrap();
    assert!((same - pixel_m * 1.5).abs() < 1e-9);
}

/// The raw grid at a given pixel, through `dem_grid`'s own geometry: a requested pixel that is too small is
/// grown, so compare through `resample`'s guard instead.
fn grid_at(window: &GeoWindow, pixel_m: f64) -> (u32, u32, f64) {
    let mid = f64::midpoint(window.south, window.north).to_radians();
    let w = (window.east - window.west) * 111_320.0 * mid.cos() / pixel_m;
    let h = (window.north - window.south) * 111_132.0 / pixel_m;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "test values are small and positive"
    )]
    (w.ceil() as u32, h.ceil() as u32, pixel_m)
}

#[test]
fn dem_grid_refuses_a_bad_request() {
    let window = fetch_window(&example_project()).unwrap();
    assert!(dem_grid(&window, Some(0.0)).is_err());
    assert!(dem_grid(&window, Some(-1.0)).is_err());
    assert!(dem_grid(&window, Some(f64::NAN)).is_err());
    assert!(dem_grid(&window, Some(f64::INFINITY)).is_err());
}

#[test]
fn resample_checks_the_size_before_loading_anything() {
    let window = fetch_window(&example_project()).unwrap();
    let store = FakeStore::default();
    let err = resample(&store, &window, 0.5, 16).unwrap_err();
    assert!(matches!(err, FetchError::Other(_)), "{err}");
    assert_eq!(store.total_loads(), 0);
    assert!(resample(&store, &window, 0.0, 16).is_err());
    assert!(resample(&store, &window, 5.0, 19).is_err());
    assert!(resample(&store, &window, 5.0, 11).is_err());
    assert_eq!(store.total_loads(), 0);
}

#[test]
fn resample_names_the_tile_when_no_zoom_covers_a_pixel() {
    let window = ramp_window(1);
    let store = FakeStore::default();
    let err = resample(&store, &window, 20.0, 16).unwrap_err().to_string();
    assert!(err.contains("no tile covers"), "{err}");
    assert!(err.contains("16/351"), "{err}");
    assert!(err.contains("parents are missing"), "{err}");
}

#[test]
fn resample_falls_back_to_ancestors_for_missing_tiles() {
    let window = ramp_window(1);
    // nothing at z16, a constant tile at z15
    let mut store = FakeStore::default();
    store
        .tiles
        .insert((15, 17565, 11450), terrarium_webp(TILE_PX, |_, _| 777.0));
    store
        .tiles
        .insert((15, 17564, 11450), terrarium_webp(TILE_PX, |_, _| 777.0));
    store
        .tiles
        .insert((15, 17565, 11451), terrarium_webp(TILE_PX, |_, _| 777.0));
    store
        .tiles
        .insert((15, 17564, 11451), terrarium_webp(TILE_PX, |_, _| 777.0));
    let (mosaic, _) = resample(&store, &window, 40.0, 16).unwrap();
    assert!(mosaic.data.iter().all(|h| (h - 777.0).abs() < 1e-3));
}

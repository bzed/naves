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
    DEFAULT_PIXEL_M, DemFetchRequest, MAPTERHORN, MARGIN_M, MAX_PIXELS, MAX_SIDE_PX,
    MAX_TILE_BYTES, RealTiles, TILE_PX, TerrariumSource, TileGet, TilePyramid, TileStore, Z_MAX,
    Z_MIN, dem_grid, describe_dem_fetch, fetch_dem, fetch_window, resample, terrarium, tile_range,
    world_px, zoom_for_pixel,
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
    // the north-east corner (inside the z15 parent): the quad needs absent z16 neighbours, whose pixels the
    // parent supplies one by one, so the value lies between the two
    let edge = pyramid.sample(lat0, lon1, 16).unwrap();
    assert!(edge > 1500.0 && edge < 2000.0, "edge {edge}");
    // a coarser zoom_max never looks at z16
    let coarse = pyramid
        .sample(f64::midpoint(lat0, lat1), f64::midpoint(lon0, lon1), 15)
        .unwrap();
    assert!((coarse - 2000.0).abs() < 1e-3, "coarse {coarse}");
}

#[test]
fn sample_falls_back_per_pixel_at_a_coverage_border() {
    // A (z16, 1000 m) has no z15 parent; its east neighbour B is absent at z16 but its z15 parent holds 2000 m
    let mut pyramid = TilePyramid::default();
    pyramid
        .insert(16, 35129, 22901, &terrarium_webp(TILE_PX, |_, _| 1000.0))
        .unwrap();
    pyramid
        .insert(15, 17565, 11450, &terrarium_webp(TILE_PX, |_, _| 2000.0))
        .unwrap();
    let (lat0, lon0) = tile_corner(16, 35129, 22901);
    let (lat1, lon1) = tile_corner(16, 35130, 22902);
    let lat = f64::midpoint(lat0, lat1);
    // on the border: the quad straddles A and B
    let border = pyramid.sample(lat, lon1, 16).unwrap();
    assert!(border > 1100.0 && border < 1900.0, "border {border}");
    // in A's interior: A's own value
    let inner = pyramid.sample(lat, f64::midpoint(lon0, lon1), 16).unwrap();
    assert!((inner - 1000.0).abs() < 1e-3, "inner {inner}");
}

#[test]
fn zoom_for_pixel_is_the_coarsest_zoom_with_two_samples_per_pixel() {
    assert_eq!(zoom_for_pixel(47.57, 2.5), 16);
    assert_eq!(zoom_for_pixel(47.57, 5.0), 15);
    assert_eq!(zoom_for_pixel(0.0, 0.1), Z_MAX);
    assert_eq!(zoom_for_pixel(47.57, 1000.0), Z_MIN);
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
    assert!(err.contains("no tile at any zoom covers"), "{err}");
    assert!(err.contains("16/351"), "{err}");
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

// ---- the fetch itself: a fake tile source, a project in a temporary folder ----

/// A tile source over a function, which records every request.
struct Fake<F> {
    answer: F,
    calls: Mutex<Vec<(u8, u32, u32)>>,
}

impl<F: Fn(u8, u32, u32) -> Option<Vec<u8>> + Sync> TileGet for Fake<F> {
    fn tile(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, FetchError> {
        self.calls.lock().unwrap().push((z, x, y));
        Ok((self.answer)(z, x, y))
    }
}

fn fake<F: Fn(u8, u32, u32) -> Option<Vec<u8>> + Sync>(answer: F) -> Fake<F> {
    Fake {
        answer,
        calls: Mutex::new(Vec::new()),
    }
}

impl<F> Fake<F> {
    fn requests(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

/// A small project (a map of 1 km, a window of about 3 km) saved in a fresh folder.
fn small_project() -> (tempfile::TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::new("T", 47.571_769, 12.946_451, 2048.0);
    project.save(dir.path()).unwrap();
    (dir, project)
}

fn toml_of(dir: &std::path::Path) -> Vec<u8> {
    std::fs::read(dir.join("naves.toml")).unwrap()
}

/// A flat tile of `height` metres.
fn flat(height: f64) -> Vec<u8> {
    terrarium_webp(TILE_PX, |_, _| height)
}

/// `fetch_dem` with the notes collected.
fn run(
    dir: &std::path::Path,
    tiles: &dyn TileGet,
    request: &DemFetchRequest,
) -> (Result<Vec<std::path::PathBuf>, FetchError>, Vec<String>) {
    let notes = std::cell::RefCell::new(Vec::new());
    let result = fetch_dem(dir, &MAPTERHORN, request, tiles, &|line| {
        notes.borrow_mut().push(line.to_string());
    });
    (result, notes.into_inner())
}

fn at_zoom(z: u8) -> DemFetchRequest {
    DemFetchRequest {
        zoom: Some(z),
        ..DemFetchRequest::default()
    }
}

/// The ramp tile `(z, x, y)`.
fn ramp_tile(z: u8, x: u32, y: u32) -> Vec<u8> {
    let size = f64::from(1u32 << z) * f64::from(TILE_PX);
    terrarium_webp(TILE_PX, |c, r| {
        let wx = f64::from(x) * f64::from(TILE_PX) + f64::from(c) + 0.5;
        let wy = f64::from(y) * f64::from(TILE_PX) + f64::from(r) + 0.5;
        let lon = wx / size * 360.0 - 180.0;
        let lat = (std::f64::consts::PI * (1.0 - 2.0 * wy / size))
            .sinh()
            .atan()
            .to_degrees();
        ramp_of(lat, lon)
    })
}

/// The haloed tile range at `zoom` of the project's window.
fn range_of(project: &Project, zoom: u8) -> (u32, u32, u32, u32) {
    tile_range(&fetch_window(project).unwrap(), zoom)
}

fn tile_count(range: (u32, u32, u32, u32)) -> u32 {
    (range.1 - range.0 + 1) * (range.3 - range.2 + 1)
}

#[test]
fn a_fetch_writes_the_dem_and_lists_it() {
    let (dir, project) = small_project();
    let tiles = fake(|z, x, y| (z <= 14).then(|| ramp_tile(z, x, y)));
    let (result, notes) = run(dir.path(), &tiles, &DemFetchRequest::default());
    let listed = result.unwrap();
    assert_eq!(
        listed,
        vec![std::path::PathBuf::from("dem/mapterhorn_dtm.tif")]
    );
    assert!(notes.iter().any(|n| n.contains("zoom 14")), "{notes:?}");
    let dem = Dem::open(&[dir.path().join("dem/mapterhorn_dtm.tif")]).unwrap();
    let (lat, lon) = (project.area.center_lat, project.area.center_lon);
    let got = f64::from(dem.height(lat, lon).unwrap());
    assert!(
        (got - ramp_of(lat, lon)).abs() < 0.1,
        "{got} vs {}",
        ramp_of(lat, lon)
    );
    let reloaded = Project::load(dir.path()).unwrap();
    assert_eq!(reloaded.sources.dem, listed);
}

#[test]
fn a_second_fetch_requests_nothing_also_after_404s() {
    let (dir, project) = small_project();
    let (x0, _, y0, _) = range_of(&project, 14);
    let tile = flat(500.0);
    let answer = |z: u8, x: u32, y: u32| {
        // the corner of the range is absent at z14 but there at z13
        (z <= 14 && (z, x, y) != (14, x0, y0)).then(|| tile.clone())
    };
    let first = fake(answer);
    run(dir.path(), &first, &DemFetchRequest::default())
        .0
        .unwrap();
    assert!(first.requests() > 0);
    assert!(
        first.calls.lock().unwrap().contains(&(14, x0, y0)),
        "the absent tile was asked for"
    );
    let second = fake(answer);
    run(dir.path(), &second, &DemFetchRequest::default())
        .0
        .unwrap();
    assert_eq!(second.requests(), 0, "{:?}", second.calls.lock().unwrap());
}

#[test]
fn the_dem_is_listed_once_and_first() {
    let (dir, mut project) = small_project();
    project.sources.dem = vec![
        "dem/cop.tif".into(),
        "dem/mapterhorn_dtm.tif".into(),
        "dem/other.tif".into(),
    ];
    project.save(dir.path()).unwrap();
    let tiles = fake(|z, _, _| (z <= 13).then(|| flat(500.0)));
    run(dir.path(), &tiles, &DemFetchRequest::default())
        .0
        .unwrap();
    run(dir.path(), &tiles, &DemFetchRequest::default())
        .0
        .unwrap();
    let listed = Project::load(dir.path()).unwrap().sources.dem;
    let expect: Vec<std::path::PathBuf> = vec![
        "dem/mapterhorn_dtm.tif".into(),
        "dem/cop.tif".into(),
        "dem/other.tif".into(),
    ];
    assert_eq!(listed, expect);
}

#[test]
fn a_tile_missing_at_the_native_zoom_comes_from_its_parent() {
    let (dir, project) = small_project();
    let tiles = fake(|z, _, _| (z == 13).then(|| flat(700.0)));
    let (result, notes) = run(dir.path(), &tiles, &at_zoom(14));
    result.unwrap();
    assert!(
        !notes.iter().any(|n| n.contains("world fallback")),
        "z13 is not the fallback: {notes:?}"
    );
    let dem = Dem::open(&[dir.path().join("dem/mapterhorn_dtm.tif")]).unwrap();
    let got = dem
        .height(project.area.center_lat, project.area.center_lon)
        .unwrap();
    assert!((got - 700.0).abs() < 1e-3, "{got}");
}

#[test]
fn the_sidecar_has_the_four_fixed_lines() {
    let (dir, _) = small_project();
    let tiles = fake(|z, _, _| (z <= 13).then(|| flat(500.0)));
    run(dir.path(), &tiles, &DemFetchRequest::default())
        .0
        .unwrap();
    let text =
        std::fs::read_to_string(dir.path().join("dem/mapterhorn_dtm.attribution.txt")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text}");
    assert_eq!(
        lines[0],
        "Terrain elevation: Mapterhorn \u{2014} https://mapterhorn.com"
    );
    assert_eq!(
        lines[1],
        "Source list and licences: https://mapterhorn.com/attribution"
    );
    assert!(lines[2].starts_with("Retrieved: 20"), "{}", lines[2]);
    assert!(lines[2].contains("  Window: 47."), "{}", lines[2]);
    assert!(
        lines[2].ends_with("  Native zoom: up to z13"),
        "{}",
        lines[2]
    );
    assert_eq!(
        lines[3],
        "The national data is bare-earth (CC BY 4.0, Etalab 2.0, DL-DE, OGL where present); the world fallback is Copernicus GLO-30."
    );
}

const REFUSAL: &str = "Mapterhorn has nothing finer than Copernicus GLO-30 here; sources.dem is unchanged. Other sources exist (national portals, Sonny's 20 m, FABDEM if non-commercial), but none is implemented \u{2014} TileGet sources slot into dem_fetch.rs, other formats need their own fetcher. Pass --force to fetch the 30 m world fallback anyway.";

#[test]
fn a_region_with_only_the_world_fallback_is_refused_unless_forced() {
    let (dir, _) = small_project();
    let before = toml_of(dir.path());
    let tiles = fake(|z, _, _| (z == Z_MIN).then(|| flat(500.0)));
    let (result, notes) = run(dir.path(), &tiles, &DemFetchRequest::default());
    assert_eq!(result.unwrap_err().to_string(), REFUSAL);
    assert!(notes.iter().any(|n| n == REFUSAL), "{notes:?}");
    assert!(!dir.path().join("dem/mapterhorn_dtm.tif").exists());
    assert_eq!(toml_of(dir.path()), before);

    let forced = DemFetchRequest {
        force: true,
        ..DemFetchRequest::default()
    };
    let (result, _) = run(dir.path(), &tiles, &forced);
    result.unwrap();
    assert!(dir.path().join("dem/mapterhorn_dtm.tif").exists());
    let text =
        std::fs::read_to_string(dir.path().join("dem/mapterhorn_dtm.attribution.txt")).unwrap();
    assert!(text.contains("up to z12"), "{text}");
}

#[test]
fn tiles_that_only_the_world_fallback_has_are_counted() {
    let (dir, project) = small_project();
    let range = range_of(&project, 14);
    let (x0, _, y0, _) = range;
    let tile = flat(500.0);
    let tiles = fake(|z, x, y| match z {
        14 => ((x, y) != (x0, y0)).then(|| tile.clone()),
        12 => Some(tile.clone()),
        _ => None,
    });
    let (result, notes) = run(dir.path(), &tiles, &DemFetchRequest::default());
    result.unwrap();
    let line = format!(
        "1 of {} tiles only have the 30 m world fallback",
        tile_count(range)
    );
    assert!(notes.iter().any(|n| n.contains(&line)), "{notes:?}");
    let text =
        std::fs::read_to_string(dir.path().join("dem/mapterhorn_dtm.attribution.txt")).unwrap();
    assert!(text.contains(&line), "{text}");
}

#[test]
fn an_answer_that_is_not_a_tile_fails_naming_the_tile() {
    for bad in [b"garbage".to_vec(), terrarium_webp(64, |_, _| 5.0)] {
        let (dir, project) = small_project();
        let before = toml_of(dir.path());
        let (x0, _, y0, _) = range_of(&project, 14);
        let tile = flat(500.0);
        let tiles = fake(|z, x, y| {
            if (z, x, y) == (14, x0, y0) {
                Some(bad.clone())
            } else {
                (z <= 14).then(|| tile.clone())
            }
        });
        let err = run(dir.path(), &tiles, &at_zoom(14)).0.unwrap_err();
        assert!(err.to_string().contains(&format!("14/{x0}/{y0}")), "{err}");
        assert_eq!(toml_of(dir.path()), before);
        assert!(
            !dir.path()
                .join(format!("dem/mapterhorn/14/{x0}/{y0}.webp"))
                .exists()
        );
    }
}

#[test]
fn a_cached_tile_that_does_not_decode_names_the_tile() {
    let (dir, project) = small_project();
    let (x0, _, y0, _) = range_of(&project, 14);
    let file = dir.path().join(format!("dem/mapterhorn/14/{x0}/{y0}.webp"));
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"garbage").unwrap();
    let tiles = fake(|z, _, _| (z <= 14).then(|| flat(500.0)));
    let err = run(dir.path(), &tiles, &at_zoom(14)).0.unwrap_err();
    assert!(err.to_string().contains(&format!("14/{x0}/{y0}")), "{err}");
}

/// A server on this machine that answers one request with `status` and `body`; returns the URL template's
/// source and what was asked.
fn serve_status(
    status: &'static str,
    body: Vec<u8>,
) -> (&'static TerrariumSource, std::sync::Arc<Mutex<String>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let received = std::sync::Arc::new(Mutex::new(String::new()));
    let seen = std::sync::Arc::clone(&received);
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buf = [0_u8; 4096];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            request.extend(buf.iter().take(n));
        }
        *seen.lock().unwrap() = String::from_utf8_lossy(&request).into_owned();
        let head = format!(
            "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        // the client may have stopped reading already
        let _ = stream.write_all(&body);
    });
    let source = Box::leak(Box::new(TerrariumSource {
        id: "local",
        name: "Local",
        tile_url: Box::leak(format!("http://{addr}/{{z}}/{{x}}/{{y}}.webp").into_boxed_str()),
        attribution_url: "http://localhost/",
    }));
    (source, received)
}

#[test]
fn real_tiles_read_an_answer_and_send_the_user_agent() {
    let (source, received) = serve_status("200 OK", b"tile".to_vec());
    let got = RealTiles::plain_for_tests(source).tile(5, 6, 7).unwrap();
    assert_eq!(got, Some(b"tile".to_vec()));
    let head = received.lock().unwrap().to_lowercase();
    assert!(head.starts_with("get /5/6/7.webp"), "{head}");
    assert!(
        head.contains(&format!("user-agent: naves/{}", env!("CARGO_PKG_VERSION"))),
        "{head}"
    );
}

#[test]
fn real_tiles_turn_a_404_into_none() {
    let (source, _) = serve_status("404 Not Found", b"nope".to_vec());
    assert_eq!(
        RealTiles::plain_for_tests(source).tile(1, 2, 3).unwrap(),
        None
    );
}

#[test]
fn real_tiles_refuse_an_answer_above_the_limit() {
    let big = vec![0_u8; usize::try_from(MAX_TILE_BYTES).unwrap() + 1000];
    let (source, _) = serve_status("200 OK", big);
    let err = RealTiles::plain_for_tests(source)
        .tile(1, 2, 3)
        .unwrap_err();
    assert!(err.to_string().contains("larger than"), "{err}");
}

#[test]
fn describe_names_zoom_tiles_and_size() {
    let (dir, project) = small_project();
    let n = tile_count(range_of(&project, 14));
    let tiles = fake(|z, _, _| (z <= 14).then(|| flat(500.0)));
    let text =
        describe_dem_fetch(dir.path(), &MAPTERHORN, &DemFetchRequest::default(), &tiles).unwrap();
    assert!(text.contains("zoom 14"), "{text}");
    assert!(text.contains(&format!("{n} tiles")), "{text}");
    assert!(text.contains(" MB"), "{text}");
    assert!(text.contains("already cached"), "{text}");
}

#[test]
fn the_default_zoom_stops_at_what_the_pixel_needs() {
    let (dir, _) = small_project();
    let tile = flat(500.0);
    let tiles = fake(|_, _, _| Some(tile.clone()));
    run(dir.path(), &tiles, &DemFetchRequest::default())
        .0
        .unwrap();
    let calls = tiles.calls.lock().unwrap();
    assert!(!calls.is_empty());
    assert!(calls.iter().all(|(z, ..)| *z <= 16), "{calls:?}");
    assert!(calls.iter().any(|(z, ..)| *z == 16));
}

#[test]
fn a_tile_with_only_its_parent_does_not_need_the_neighbours_parents() {
    let (dir, project) = small_project();
    let window = fetch_window(&project).unwrap();
    let (px, py) = world_px(project.area.center_lat, project.area.center_lon, 14);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "tile indices of a point in Germany"
    )]
    let t = (
        (px / f64::from(TILE_PX)).floor() as u32,
        (py / f64::from(TILE_PX)).floor() as u32,
    );
    assert!(window.west < window.east);
    let tile = flat(500.0);
    let tiles = fake(|z, x, y| match z {
        14 => ((x, y) != t).then(|| tile.clone()),
        13 => ((x, y) == (t.0 >> 1, t.1 >> 1)).then(|| tile.clone()),
        _ => None,
    });
    run(dir.path(), &tiles, &at_zoom(14)).0.unwrap();
    let dem = Dem::open(&[dir.path().join("dem/mapterhorn_dtm.tif")]).unwrap();
    let got = dem
        .height(project.area.center_lat, project.area.center_lon)
        .unwrap();
    assert!((got - 500.0).abs() < 1e-3, "{got}");
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The build pipeline against fake tools and the Königssee DEM fixture.

#![cfg(unix)]
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::panic,
    reason = "test helper code; failing loudly is the point, and the pixel positions are small and positive"
)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use naves_core::project::Project;
use naves_export::build::{BuildError, build};
use naves_export::testing::FakeEnv;

/// A project below `dir` whose area lies inside the Königssee fixture.
fn project(dir: &Path, center: (f64, f64)) -> Project {
    let mut p = Project::new("Test", center.0, center.1, 1500.0);
    p.grid.terrain_size = 256;
    p.grid.land_size = 32;
    p.sources.dem = vec!["dem/koenigssee.tif".into()];
    std::fs::create_dir_all(dir.join("dem")).unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/dem/koenigssee.tif");
    std::fs::copy(fixture, dir.join("dem/koenigssee.tif")).unwrap();
    p.save(dir).unwrap();
    p
}

const INSIDE: (f64, f64) = (47.55, 12.975);

fn setup() -> (tempfile::TempDir, FakeEnv, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let env = FakeEnv::new(&dir.path().join("fake")).unwrap();
    let proj = dir.path().join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    (dir, env, proj)
}

fn quiet(_: &str) {}

#[test]
fn builds_mod_keys_and_mission() {
    let (_dir, env, proj) = setup();
    project(&proj, INSIDE);
    let lines = RefCell::new(Vec::new());
    let out = build(&proj, &env.tools(), &|l| {
        lines.borrow_mut().push(l.to_string());
    })
    .unwrap();
    assert_eq!(out.mod_dir, proj.join("build/@naves_test"));
    assert_eq!(
        out.mission_dir,
        proj.join("build/missions/dayzOffline.test")
    );
    assert!(!lines.borrow().is_empty(), "progress is reported");

    let pbo = std::fs::read_to_string(out.mod_dir.join("addons/naves_test.pbo")).unwrap();
    for wanted in [
        "./$PBOPREFIX$",
        "./config.cpp",
        "./test.wrp",
        "./data/layers/p_000-000.rvmat",
        "./data/layers/s_000_000_lco.paa",
        "./data/layers/m_000_000_lca.paa",
        "./ce/cfgenvironment.xml",
        "./ce/env/bear_territories.xml",
    ] {
        assert!(
            pbo.lines().any(|l| l == wanted),
            "{wanted} missing from:\n{pbo}"
        );
    }
    assert!(
        !pbo.contains(".png"),
        "PNG sources must not be packed:\n{pbo}"
    );
    assert!(pbo.contains("key="), "{pbo}");
    assert_eq!(
        std::fs::read_to_string(proj.join("build/staging/pack/naves/worlds/test/$PBOPREFIX$"))
            .unwrap(),
        "naves\\worlds\\test"
    );
    assert!(out.mod_dir.join("keys/naves_test.bikey").is_file());
    assert!(proj.join("secrets/naves_test.biprivatekey").is_file());
    assert!(
        out.mod_dir
            .join("addons/naves_test.pbo.naves_test.bisign")
            .is_file()
    );
    assert!(proj.join("build/logs/binarize.log").is_file());

    let spawns = std::fs::read_to_string(out.mission_dir.join("cfgplayerspawnpoints.xml")).unwrap();
    // 1500 m * 0.5 = 750 m map, spawn in the middle
    assert!(spawns.contains(r#"<pos x="375" z="375" />"#), "{spawns}");
}

#[test]
fn the_packed_world_is_the_binarized_one() {
    let (_dir, env, proj) = setup();
    project(&proj, INSIDE);
    build(&proj, &env.tools(), &quiet).unwrap();
    let packed = std::fs::read(proj.join("build/staging/pack/naves/worlds/test/test.wrp")).unwrap();
    assert_eq!(&packed[..4], b"OPRW", "the 8WVR must not be packed");
}

#[test]
fn no_dem_is_explained() {
    let (_dir, env, proj) = setup();
    let mut p = project(&proj, INSIDE);
    p.sources.dem.clear();
    p.save(&proj).unwrap();
    let err = build(&proj, &env.tools(), &quiet).unwrap_err();
    assert!(matches!(err, BuildError::NoDem), "{err}");
    assert!(err.to_string().contains("sources.dem"), "{err}");
}

#[test]
fn area_outside_the_dem_names_a_location_and_builds_nothing() {
    let (_dir, env, proj) = setup();
    project(&proj, (47.0, 12.0));
    let err = build(&proj, &env.tools(), &quiet).unwrap_err();
    assert!(err.to_string().contains("no DEM data at 46.99"), "{err}");
    assert!(!proj.join("build/@naves_test").exists());
    assert!(!proj.join("build/missions").exists());
}

#[test]
fn missing_dem_file_names_the_file() {
    let (_dir, env, proj) = setup();
    project(&proj, INSIDE);
    std::fs::remove_file(proj.join("dem/koenigssee.tif")).unwrap();
    let err = build(&proj, &env.tools(), &quiet).unwrap_err();
    assert!(err.to_string().contains("koenigssee.tif"), "{err}");
}

#[test]
fn rebuild_removes_stale_files_but_keeps_secrets() {
    let (_dir, env, proj) = setup();
    project(&proj, INSIDE);
    for stale in [
        "build/@naves_test/addons/old.pbo",
        "build/staging/pack/old.txt",
        "build/missions/dayzOffline.old/x",
    ] {
        let p = proj.join(stale);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "stale").unwrap();
    }
    std::fs::create_dir_all(proj.join("secrets")).unwrap();
    std::fs::write(proj.join("secrets/naves_test.biprivatekey"), "my key").unwrap();
    std::fs::write(proj.join("secrets/naves_test.bikey"), "my public key").unwrap();
    std::fs::write(proj.join("secrets/other.txt"), "keep").unwrap();

    build(&proj, &env.tools(), &quiet).unwrap();
    build(&proj, &env.tools(), &quiet).unwrap();

    assert!(!proj.join("build/@naves_test/addons/old.pbo").exists());
    assert!(!proj.join("build/staging/pack/old.txt").exists());
    assert!(!proj.join("build/missions/dayzOffline.old").exists());
    assert!(
        proj.join("build/@naves_test/addons/naves_test.pbo")
            .is_file()
    );
    assert_eq!(
        std::fs::read_to_string(proj.join("secrets/naves_test.biprivatekey")).unwrap(),
        "my key"
    );
    assert_eq!(
        std::fs::read_to_string(proj.join("secrets/other.txt")).unwrap(),
        "keep"
    );
    // the public key of the existing private key is regenerated next to it
    assert!(
        proj.join("build/@naves_test/keys/naves_test.bikey")
            .is_file()
    );
}

#[test]
fn a_failing_tool_reports_its_stderr() {
    let (_dir, env, proj) = setup();
    project(&proj, INSIDE);
    env.replace_armake2("echo 'oops: cannot pack' >&2\nexit 4")
        .unwrap();
    let err = build(&proj, &env.tools(), &quiet).unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("armake2") && text.contains("oops: cannot pack"),
        "{text}"
    );
}

#[test]
fn non_ascii_project_names_give_ascii_files() {
    let (_dir, env, proj) = setup();
    let mut p = project(&proj, INSIDE);
    p.name = "Königssee Test".to_string();
    p.world = "konigssee_test".to_string();
    p.save(&proj).unwrap();
    let out = build(&proj, &env.tools(), &quiet).unwrap();
    assert!(out.mod_dir.ends_with("@naves_konigssee_test"));
    let pbo = std::fs::read_to_string(out.mod_dir.join("addons/naves_konigssee_test.pbo")).unwrap();
    assert!(pbo.is_ascii(), "{pbo}");
    let config = std::fs::read_to_string(
        proj.join("build/staging/pack/naves/worlds/konigssee_test/config.cpp"),
    )
    .unwrap();
    assert!(config.is_ascii());
}

#[test]
fn existing_keys_are_reused_and_published() {
    let (_dir, env, proj) = setup();
    project(&proj, INSIDE);
    std::fs::create_dir_all(proj.join("secrets")).unwrap();
    std::fs::write(proj.join("secrets/naves_test.biprivatekey"), "k").unwrap();
    std::fs::write(proj.join("secrets/naves_test.bikey"), "my public key").unwrap();
    let out = build(&proj, &env.tools(), &quiet).unwrap();
    assert_eq!(
        std::fs::read_to_string(out.mod_dir.join("keys/naves_test.bikey")).unwrap(),
        "my public key"
    );
}

#[test]
fn private_key_without_public_key_is_an_error() {
    let (_dir, env, proj) = setup();
    project(&proj, INSIDE);
    std::fs::create_dir_all(proj.join("secrets")).unwrap();
    std::fs::write(proj.join("secrets/naves_test.biprivatekey"), "k").unwrap();
    let err = build(&proj, &env.tools(), &quiet).unwrap_err();
    assert!(matches!(err, BuildError::MissingPublicKey { .. }), "{err}");
    assert_eq!(
        std::fs::read_to_string(proj.join("secrets/naves_test.biprivatekey")).unwrap(),
        "k",
        "the private key is never touched"
    );
}

#[test]
fn a_project_navmesh_is_packed_where_the_config_points() {
    let (_dir, env, proj) = setup();
    project(&proj, INSIDE);
    std::fs::create_dir_all(proj.join("navmesh")).unwrap();
    std::fs::write(proj.join("navmesh/test.nm"), "NAVMESH").unwrap();
    let out = build(&proj, &env.tools(), &quiet).unwrap();
    let pbo = std::fs::read_to_string(out.mod_dir.join("addons/naves_test.pbo")).unwrap();
    assert!(pbo.lines().any(|l| l == "./navmesh/test.nm"), "{pbo}");
}

#[test]
fn without_a_navmesh_nothing_navmesh_is_packed() {
    let (_dir, env, proj) = setup();
    project(&proj, INSIDE);
    let out = build(&proj, &env.tools(), &quiet).unwrap();
    let pbo = std::fs::read_to_string(out.mod_dir.join("addons/naves_test.pbo")).unwrap();
    assert!(!pbo.contains("navmesh"), "{pbo}");
}

/// The tags of a PAA file in order, with their payloads.
fn paa_tags(d: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut o = 2;
    let mut out = Vec::new();
    while d.get(o..o + 4) == Some(b"GGAT".as_slice()) {
        let name: String = d[o + 4..o + 8]
            .iter()
            .rev()
            .map(|&b| char::from(b))
            .collect();
        let len = u32::from_le_bytes(d[o + 8..o + 12].try_into().unwrap()) as usize;
        out.push((name, d[o + 12..o + 12 + len].to_vec()));
        o += 12 + len;
    }
    out
}

#[test]
fn terrain_images_get_livonias_colour_tags() {
    let (_dir, env, proj) = setup();
    project(&proj, INSIDE);
    build(&proj, &env.tools(), &quiet).unwrap();
    let layers = proj.join("build/staging/P/naves/worlds/test/data/layers");
    let sat = std::fs::read(layers.join("s_000_000_lco.paa")).unwrap();
    let names: Vec<String> = paa_tags(&sat).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["AVGC", "MAXC", "OFFS"], "satellite");
    let avgc = &paa_tags(&sat)[0].1;
    assert_eq!(avgc[3], 255, "the satellite image is opaque");
    assert!(
        avgc[..3].iter().any(|&c| c > 0),
        "a real average colour: {avgc:?}"
    );
    let mask = std::fs::read(layers.join("m_000_000_lca.paa")).unwrap();
    let names: Vec<String> = paa_tags(&mask).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["AVGC", "MAXC", "FLAG", "OFFS"], "mask");
    // the stray byte img2paa writes after an LZO stream is gone: 4 x 4 LZO mipmap of 3 bytes, then the end marker
    let wanted: &[u8] = &[4, 0x80, 4, 0, 3, 0, 0, 0x11, 0, 0, 0, 0, 0, 0, 0, 0];
    assert!(sat.windows(wanted.len()).any(|w| w == wanted));
}

#[test]
fn a_failed_build_keeps_the_last_good_output() {
    let (_dir, env, proj) = setup();
    project(&proj, INSIDE);
    let good = build(&proj, &env.tools(), &quiet).unwrap();
    let pbo = good.mod_dir.join("addons/naves_test.pbo");
    let before = std::fs::read(&pbo).unwrap();
    // the source of the mission disappears: this fails only after the world has been binarized
    let source = env.tools().dayz_server.join("mpmissions/dayzOffline.enoch");
    std::fs::remove_dir_all(source).unwrap();
    assert!(build(&proj, &env.tools(), &quiet).is_err());
    assert_eq!(
        std::fs::read(&pbo).unwrap(),
        before,
        "the mod is still there"
    );
    assert!(
        good.mission_dir.join("init.c").is_file(),
        "so is the mission"
    );
}

// ---- ground from land cover and OSM ----

use naves_core::heightmap::MapGeo;
use naves_core::landcover::{GeoWindow, LandCover};
use naves_export::layers::tile_grid;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

/// Copies the fixtures into the project and lists them as sources.
fn with_sources(proj: &Path, land_cover: &[&str], osm: &[&str]) -> Project {
    let mut p = Project::load(proj).unwrap();
    for (dir, names, list) in [
        ("landcover", land_cover, &mut p.sources.land_cover),
        ("osm", osm, &mut p.sources.osm),
    ] {
        std::fs::create_dir_all(proj.join(dir)).unwrap();
        for name in names {
            std::fs::copy(fixture(&format!("{dir}/{name}")), proj.join(dir).join(name)).unwrap();
            list.push(PathBuf::from(dir).join(name));
        }
    }
    p.save(proj).unwrap();
    p
}

/// The pixel of the tile mask at a latitude and longitude, read from the PNG that the fake `img2paa` embeds.
fn mask_at(proj: &Path, p: &Project, lat: f64, lon: f64) -> [u8; 4] {
    let (x, y) = MapGeo::new(p).unwrap().to_map(lat, lon).unwrap();
    let grid = tile_grid(p.map_size_m(), p.grid.land_size);
    let overlap = f64::from(grid.overlap_px);
    let tile_m = f64::from(grid.tile_px - 2 * grid.overlap_px) * grid.px_m;
    let tx = (x / tile_m).floor();
    let ty_from_south = (y / tile_m).floor();
    let ty = f64::from(grid.tiles_y) - 1.0 - ty_from_south;
    let px = (x - tx * tile_m) / grid.px_m + overlap;
    let py = ((ty_from_south + 1.0) * tile_m - y) / grid.px_m + overlap;
    let name = format!("m_{:03}_{:03}_lca.paa", tx as u32, ty as u32);
    let paa = std::fs::read(
        proj.join("build/staging/P/naves/worlds/test/data/layers")
            .join(name),
    )
    .unwrap();
    let start = paa
        .windows(8)
        .position(|w| w == b"\x89PNG\r\n\x1a\n")
        .unwrap();
    let image = image::load_from_memory(&paa[start..]).unwrap().to_rgba8();
    image.get_pixel(px as u32, py as u32).0
}

/// The latitude and longitude of a spot inside the project's area where the land cover is `code` all
/// around (so that the mask pixel cannot sit on a class boundary).
fn spot_with_code(p: &Project, code: u8) -> (f64, f64) {
    let geo = MapGeo::new(p).unwrap();
    let (south_west, north_east) = (
        geo.to_geo(30.0, 30.0).unwrap(),
        geo.to_geo(p.map_size_m() - 30.0, p.map_size_m() - 30.0)
            .unwrap(),
    );
    let window = GeoWindow {
        south: south_west.0 - 0.001,
        west: south_west.1 - 0.001,
        north: north_east.0 + 0.001,
        east: north_east.1 + 0.001,
    };
    let lc = LandCover::open(&[fixture("landcover/koenigssee.tif")], window).unwrap();
    let step = 0.0001;
    let mut lat = south_west.0;
    while lat < north_east.0 {
        let mut lon = south_west.1;
        while lon < north_east.1 {
            let around = [(0.0, 0.0), (2.0, 0.0), (-2.0, 0.0), (0.0, 2.0), (0.0, -2.0)];
            if around.iter().all(|(dlat, dlon)| {
                lc.class_code(lat + dlat * step, lon + dlon * step) == Some(code)
            }) {
                return (lat, lon);
            }
            lon += step;
        }
        lat += step;
    }
    panic!("no spot with class {code} in the area");
}

#[test]
fn land_cover_reaches_the_mask() {
    let (_dir, env, proj) = setup();
    project(&proj, INSIDE);
    let p = with_sources(&proj, &["koenigssee.tif"], &[]);
    build(&proj, &env.tools(), &quiet).unwrap();
    let (lat, lon) = spot_with_code(&p, 10);
    assert_eq!(
        mask_at(&proj, &p, lat, lon),
        [0, 255, 0, 255],
        "trees: forest, green"
    );
    let (lat, lon) = spot_with_code(&p, 30);
    assert_eq!(
        mask_at(&proj, &p, lat, lon),
        [0, 0, 0, 255],
        "grassland: black"
    );
}

#[test]
fn osm_reaches_the_mask() {
    let (_dir, env, proj) = setup();
    project(&proj, INSIDE);
    let p = with_sources(&proj, &[], &["koenigssee.osm.pbf"]);
    build(&proj, &env.tools(), &quiet).unwrap();
    // the Königssee: soil, blue
    assert_eq!(mask_at(&proj, &p, 47.5534, 12.9765), [0, 0, 255, 255]);
}

#[test]
fn attribution_names_the_sources_used() {
    let (_dir, env, proj) = setup();
    project(&proj, INSIDE);
    let out = build(&proj, &env.tools(), &quiet).unwrap();
    let text = std::fs::read_to_string(out.mod_dir.join("ATTRIBUTION.txt")).unwrap();
    assert!(text.contains("COPERNICUS"), "{text}");
    assert!(
        !text.contains("OpenStreetMap") && !text.contains("WorldCover"),
        "{text}"
    );

    with_sources(&proj, &["koenigssee.tif"], &["koenigssee.osm.pbf"]);
    let out = build(&proj, &env.tools(), &quiet).unwrap();
    let text = std::fs::read_to_string(out.mod_dir.join("ATTRIBUTION.txt")).unwrap();
    assert!(text.contains("COPERNICUS"), "{text}");
    assert!(
        text.contains("© OpenStreetMap contributors") && text.contains("ODbL"),
        "{text}"
    );
    assert!(
        text.contains("© ESA WorldCover project 2021") && text.contains("CC BY 4.0"),
        "{text}"
    );
}

#[test]
fn missing_source_file_names_the_file() {
    let (_dir, env, proj) = setup();
    let mut p = project(&proj, INSIDE);
    p.sources.land_cover = vec!["landcover/nope.tif".into()];
    p.save(&proj).unwrap();
    let err = build(&proj, &env.tools(), &quiet).unwrap_err();
    assert!(err.to_string().contains("nope.tif"), "{err}");
    assert!(!proj.join("build/staging").exists(), "no tool ran");

    let mut p = Project::load(&proj).unwrap();
    p.sources.land_cover.clear();
    p.sources.osm = vec!["osm/nope.osm.pbf".into()];
    p.save(&proj).unwrap();
    let err = build(&proj, &env.tools(), &quiet).unwrap_err();
    assert!(err.to_string().contains("nope.osm.pbf"), "{err}");
}

#[test]
fn partial_coverage_warns() {
    let (_dir, env, proj) = setup();
    project(&proj, INSIDE);
    // the western half of the crop: the map's eastern half has no land cover
    with_sources(&proj, &["koenigssee_west.tif"], &[]);
    let out = build(&proj, &env.tools(), &quiet).unwrap();
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("land cover") && w.contains("50 %")),
        "{:?}",
        out.warnings
    );
}

#[test]
fn full_coverage_does_not_warn() {
    let (_dir, env, proj) = setup();
    project(&proj, INSIDE);
    with_sources(&proj, &["koenigssee.tif"], &["koenigssee.osm.pbf"]);
    let out = build(&proj, &env.tools(), &quiet).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[test]
fn cut_osm_polygons_are_reported() {
    let (_dir, env, proj) = setup();
    project(&proj, INSIDE);
    // an extract that cuts the lake: its rings do not close
    with_sources(&proj, &[], &["koenigssee_cut.osm.pbf"]);
    let out = build(&proj, &env.tools(), &quiet).unwrap();
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("OpenStreetMap") && w.contains("skipped")),
        "{:?}",
        out.warnings
    );
}

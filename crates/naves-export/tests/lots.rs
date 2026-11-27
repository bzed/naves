// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The lot stage in the build: the roads inside a lot go, the access roads are cut, the ground is shaped and the
//! lot is painted, and with the stage off every output is the build without the lot.

#![cfg(unix)]
#![expect(
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::type_complexity,
    reason = "test helper code; failing loudly is the point, and the protobuf helpers are one-off"
)]

use std::io::Cursor;
use std::path::{Path, PathBuf};

use naves_core::catalog::Family;
use naves_core::catalog::testing::synthetic_catalog;
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::heightmap::{Heightmap, MapGeo};
use naves_core::lot::{LOTS_ENABLED, LotArea, LotKind, Outcome, Placement};
use naves_core::netclean::{self, CutAccess, lost_entries, surviving_entries};
use naves_core::project::Project;
use naves_core::roadnet::{build_graph, collapse_small_rings, drop_duplicates};
use naves_core::smooth::SMOOTH_ENABLED;
use naves_core::surface::SurfaceClass;
use naves_core::validate::Rule;
use naves_export::build::build_with;
use naves_export::generate::{
    Stages, fitted_graph, fitted_graph_with, generate_with, plan_roads_with,
};
use naves_export::testing::FakeEnv;
use tiff::encoder::{TiffEncoder, colortype};
use tiff::tags::Tag;

// --- the graph of a camp site with a T of service roads ------------------------------------------------------------

fn way(id: i64, nodes: &[i64], points: &[(f64, f64)]) -> Feature {
    Feature {
        id: format!("osm/way/{id}/0"),
        way_id: id,
        source: Source::Osm,
        kind: FeatureKind::Road {
            family: Family::City,
            drivable: true,
            bridge: false,
            tunnel: false,
            roundabout: false,
            layer: 0,
            use_sidepath: false,
            paving: None,
        },
        points: points.to_vec(),
        nodes: nodes.to_vec(),
    }
}

fn flat() -> Heightmap {
    Heightmap {
        size: 250,
        cell_m: 4.0,
        h: vec![100.0; 250 * 250],
    }
}

fn camp_site() -> LotArea {
    LotArea {
        id: "osm/way/100".to_string(),
        kind: LotKind::CampSite,
        placement: Placement::Ground,
        outer: vec![
            (100.0, 100.0),
            (300.0, 100.0),
            (300.0, 300.0),
            (100.0, 300.0),
            (100.0, 100.0),
        ],
        holes: Vec::new(),
        area_m2: 40_000.0,
        surface_tag: None,
        holes_dropped: 0,
    }
}

/// A stem that enters the camp site and ends at a T inside it, and the bar of the T (two stretches).
fn camp_site_roads() -> Vec<Feature> {
    vec![
        way(1, &[1, 2], &[(50.0, 200.0), (200.0, 200.0)]),
        way(
            2,
            &[3, 2, 4],
            &[(200.0, 150.0), (200.0, 200.0), (200.0, 250.0)],
        ),
    ]
}

#[test]
fn with_the_stage_off_a_lot_changes_nothing_and_no_report_is_made() {
    let catalog = synthetic_catalog();
    let base = flat();
    let features = camp_site_roads();
    let stages = Stages {
        reroute: false,
        smooth: false,
        lots: false,
    };
    let with_lot = fitted_graph_with(
        &features,
        &catalog,
        &base,
        stages,
        &|_| false,
        &[],
        &[camp_site()],
    );
    let without = fitted_graph_with(&features, &catalog, &base, stages, &|_| false, &[], &[]);
    assert_eq!(with_lot.built, without.built);
    assert_eq!(with_lot.graph, without.graph);
    assert_eq!(with_lot.clean, without.clean);
    assert_eq!(with_lot.duplicates, without.duplicates);
    assert_eq!(with_lot.bridges, without.bridges);
    assert!(with_lot.lots.is_none());
    // the stage's time is zero when it is off (and, when on, it is the lot work only: see `FittedGraph::lots_time`)
    assert_eq!(with_lot.lots_time, std::time::Duration::ZERO);
    // and the graph is today's: the cleaned graph with the bridges fitted
    let (flat_graph, _) =
        collapse_small_rings(&build_graph(&features), &|f| catalog.tightest_arc(f));
    let (deduplicated, _) = drop_duplicates(&flat_graph);
    let (cleaned, _) = netclean::clean(&deduplicated, &|f| f.width_m(), f64::from(base.cell_m));
    assert_eq!(with_lot.built, cleaned);
    // the clean-up has no lot step, so the graph keeps all three stretches
    assert_eq!(with_lot.built.stretches.len(), 3);
}

#[test]
fn a_camp_site_holding_a_t_of_service_roads_loses_both_stretches_and_gets_one_access_cut() {
    let catalog = synthetic_catalog();
    let base = flat();
    let fitted = fitted_graph_with(
        &camp_site_roads(),
        &catalog,
        &base,
        Stages {
            reroute: false,
            smooth: false,
            lots: true,
        },
        &|_| false,
        &[],
        &[camp_site()],
    );
    let report = fitted.lots.as_ref().expect("the lot stage ran");
    assert_eq!(report.assessed.len(), 1);
    assert!(
        matches!(report.assessed[0].outcome, Outcome::Smoothed { .. }),
        "{:?}",
        report.assessed[0]
    );
    // both stretches of the bar are inside and go
    let removed: Vec<&str> = fitted
        .clean
        .lot_roads
        .iter()
        .map(|dropped| dropped.stretch.as_str())
        .collect();
    assert_eq!(removed, ["2:2-3", "2:2-4"], "{:?}", fitted.clean.lot_roads);
    // the stem is cut at the lot's edge and ends one entry inside
    assert_eq!(fitted.clean.lot_access.len(), 1);
    let cut = &fitted.clean.lot_access[0];
    assert_eq!(cut.stretch, "1:1-2");
    assert!((cut.entry.0 - 104.0).abs() < 0.5, "{:?}", cut.entry);
    // the cut stem is the graph; the bar is gone
    assert_eq!(fitted.built.stretches.len(), 1);
    assert!(
        fitted.built.stretches[0]
            .points
            .iter()
            .all(|p| p.0 <= 104.5)
    );
    // the camp site is grass unless its own tag says paved
    assert_eq!(report.class_of("osm/way/100"), Some(SurfaceClass::Grass));
}

#[test]
fn the_defaults_follow_the_constants() {
    let catalog = synthetic_catalog();
    let base = flat();
    let features = camp_site_roads();
    let fitted = fitted_graph(&features, &catalog, &base, &|_| false, &[], &[camp_site()]);
    assert_eq!(fitted.lots.is_some(), LOTS_ENABLED);
    assert_eq!(fitted.smooth.is_some(), SMOOTH_ENABLED);
}

#[test]
fn with_the_stage_off_per_call_no_lot_is_looked_at() {
    let catalog = synthetic_catalog();
    let base = flat();
    let features = camp_site_roads();
    let off = Stages {
        reroute: false,
        smooth: false,
        lots: false,
    };
    let fitted = fitted_graph_with(
        &features,
        &catalog,
        &base,
        off,
        &|_| false,
        &[],
        &[camp_site()],
    );
    assert!(fitted.lots.is_none());
    // off, the graph is the one without the lots
    let without = fitted_graph_with(&features, &catalog, &base, off, &|_| false, &[], &[]);
    assert_eq!(fitted.built, without.built);
    assert_eq!(fitted.clean, without.clean);
}

#[test]
fn the_components_are_counted_per_step_with_the_lot_step_first() {
    let catalog = synthetic_catalog();
    let base = flat();
    let features = camp_site_roads();
    let with = fitted_graph_with(
        &features,
        &catalog,
        &base,
        Stages {
            reroute: false,
            smooth: false,
            lots: true,
        },
        &|_| false,
        &[],
        &[camp_site()],
    );
    assert_eq!(with.clean.components_by_step.len(), 5);
    let without = fitted_graph_with(
        &features,
        &catalog,
        &base,
        Stages {
            reroute: false,
            smooth: false,
            lots: false,
        },
        &|_| false,
        &[],
        &[],
    );
    assert_eq!(without.clean.components_by_step.len(), 4);
    // the lot counts as one node: the removal does not split the graph
    assert_eq!(with.clean.components_before, with.clean.components_after);
    assert_eq!(with.clean.lots_split, Vec::<usize>::new());
}

#[test]
fn surviving_entries_keeps_the_cuts_whose_node_the_graph_has() {
    let catalog = synthetic_catalog();
    let base = flat();
    let fitted = fitted_graph_with(
        &camp_site_roads(),
        &catalog,
        &base,
        Stages {
            reroute: false,
            smooth: false,
            lots: true,
        },
        &|_| false,
        &[],
        &[camp_site()],
    );
    let entry = fitted.clean.lot_access[0].clone();
    // here the node is there, so the entry survives and no entry is lost
    assert_eq!(
        surviving_entries(&fitted.built, std::slice::from_ref(&entry)),
        std::slice::from_ref(&entry)
    );
    assert_eq!(fitted.clean.lot_entries_lost, 0);
    // a node the graph does not have any more (a sidepath step can take a cut piece away) is dropped
    let mut gone = entry;
    gone.node = 123_456;
    assert_eq!(surviving_entries(&fitted.built, &[gone]), Vec::new());
}

#[test]
fn the_entries_whose_node_is_gone_are_counted() {
    // the clean-up reports `CleanReport::lot_entries_lost` as `lost_entries` of its final graph and its cuts
    let graph = build_graph(&[way(1, &[1, 2], &[(0.0, 0.0), (10.0, 0.0)])]);
    let cut = |node: i64| CutAccess {
        stretch: "1:1-2".to_string(),
        lot: "osm/way/100".to_string(),
        cut_m: 4.0,
        entry: (10.0, 0.0),
        node,
    };
    let kept = cut(2);
    let gone = cut(9);
    assert_eq!(lost_entries(&graph, &[]), 0);
    assert_eq!(lost_entries(&graph, std::slice::from_ref(&kept)), 0);
    assert_eq!(lost_entries(&graph, &[kept.clone(), gone]), 1);
    // and the count is the complement of the filter it is made of
    assert_eq!(lost_entries(&graph, &[kept.clone(), cut(8), cut(7)]), 2);
}

// --- a project with a lot, through the build ----------------------------------------------------------------------

/// A protobuf varint.
fn varint(mut v: u64, out: &mut Vec<u8>) {
    while v >= 0x80 {
        out.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// A length-delimited field.
fn bytes_field(field: u64, data: &[u8], out: &mut Vec<u8>) {
    varint(field << 3 | 2, out);
    varint(data.len() as u64, out);
    out.extend_from_slice(data);
}

fn varint_field(field: u64, v: u64, out: &mut Vec<u8>) {
    varint(field << 3, out);
    varint(v, out);
}

fn packed(values: impl IntoIterator<Item = u64>) -> Vec<u8> {
    let mut out = Vec::new();
    for v in values {
        varint(v, &mut out);
    }
    out
}

/// One file block: its length-prefixed header and an uncompressed blob.
fn block(kind: &str, body: &[u8], file: &mut Vec<u8>) {
    let mut blob = Vec::new();
    bytes_field(1, body, &mut blob);
    let mut header = Vec::new();
    bytes_field(1, kind.as_bytes(), &mut header);
    varint_field(3, blob.len() as u64, &mut header);
    file.extend_from_slice(&u32::try_from(header.len()).unwrap().to_be_bytes());
    file.extend_from_slice(&header);
    file.extend_from_slice(&blob);
}

/// A minimal OSM PBF: nodes (id, lat, lon in degrees) and ways (id, node ids, tags).
fn pbf(nodes: &[(i64, f64, f64)], ways: &[(i64, Vec<i64>, Vec<(&str, &str)>)]) -> Vec<u8> {
    let mut file = Vec::new();
    let mut header = Vec::new();
    bytes_field(4, b"OsmSchema-V0.6", &mut header);
    block("OSMHeader", &header, &mut file);

    let mut strings: Vec<String> = vec![String::new()];
    let table = |s: &str, strings: &mut Vec<String>| -> u64 {
        if let Some(i) = strings.iter().position(|x| x == s) {
            i as u64
        } else {
            strings.push(s.to_string());
            (strings.len() - 1) as u64
        }
    };
    let mut group = Vec::new();
    for &(id, lat, lon) in nodes {
        let mut node = Vec::new();
        varint_field(1, zigzag(id), &mut node);
        varint_field(8, zigzag((lat * 1e7).round() as i64), &mut node);
        varint_field(9, zigzag((lon * 1e7).round() as i64), &mut node);
        bytes_field(1, &node, &mut group);
    }
    for (id, refs, tags) in ways {
        let mut way = Vec::new();
        varint_field(1, *id as u64, &mut way);
        let keys: Vec<u64> = tags.iter().map(|(k, _)| table(k, &mut strings)).collect();
        let vals: Vec<u64> = tags.iter().map(|(_, v)| table(v, &mut strings)).collect();
        bytes_field(2, &packed(keys), &mut way);
        bytes_field(3, &packed(vals), &mut way);
        let mut last = 0;
        let deltas: Vec<u64> = refs
            .iter()
            .map(|&r| {
                let d = zigzag(r - last);
                last = r;
                d
            })
            .collect();
        bytes_field(8, &packed(deltas), &mut way);
        bytes_field(3, &way, &mut group);
    }
    let mut table_msg = Vec::new();
    for s in &strings {
        bytes_field(1, s.as_bytes(), &mut table_msg);
    }
    let mut primitive = Vec::new();
    bytes_field(1, &table_msg, &mut primitive);
    bytes_field(2, &group, &mut primitive);
    block("OSMData", &primitive, &mut file);
    file
}

/// The flat height of the DEM, and the hollow at the middle of the map.
const FLAT_M: f32 = 500.0;
const HOLLOW_M: f64 = 200.0;
const DEPTH_M: f64 = 3.0;
/// The map centre, in degrees and in map metres.
const CENTRE: (f64, f64) = (47.55, 12.975);
const MAP_M: f64 = 1000.0;

/// Degrees of longitude and latitude per map metre around [`CENTRE`].
fn lon_of(x: f64) -> f64 {
    CENTRE.1 + (x - MAP_M / 2.0) / 75_100.0
}

fn lat_of(y: f64) -> f64 {
    CENTRE.0 - (y - MAP_M / 2.0) / 111_000.0
}

/// A GeoTIFF of 0.00005 degree pixels around the map with a hollow at its middle.
fn dem() -> Vec<u8> {
    let (west, north, pixel) = (12.96, 47.56, 0.00005);
    let (columns, rows) = (600_u32, 400_u32);
    let mut data = Vec::new();
    for _ in 0..rows {
        for c in 0..columns {
            let lon = west + (f64::from(c) + 0.5) * pixel;
            let across = (lon - CENTRE.1).abs() / (HOLLOW_M / 2.0 / 75_100.0);
            data.push(FLAT_M - (DEPTH_M * (1.0 - across).max(0.0)) as f32);
        }
    }
    let mut cur = Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut cur).unwrap();
        let mut img = enc
            .new_image::<colortype::Gray32Float>(columns, rows)
            .unwrap();
        let keys: [u16; 16] = [1, 1, 0, 3, 1024, 0, 1, 2, 1025, 0, 1, 1, 2048, 0, 1, 4326];
        img.encoder()
            .write_tag(Tag::ModelPixelScaleTag, &[pixel, pixel, 0.0][..])
            .unwrap();
        img.encoder()
            .write_tag(
                Tag::ModelTiepointTag,
                &[0.0, 0.0, 0.0, west, north, 0.0][..],
            )
            .unwrap();
        img.encoder()
            .write_tag(Tag::GeoKeyDirectoryTag, &keys[..])
            .unwrap();
        img.write_data(&data).unwrap();
    }
    cur.into_inner()
}

/// A project of 1000 m with a car park around the middle, a service road that ends 100 m inside it (without
/// `with_road`, no road at all) and, with `with_lot`, the car park's area in the extract.
fn project(proj: &Path, with_lot: bool, with_road: bool, broken_lot: bool) -> Project {
    project_with(proj, with_lot, with_road, broken_lot, &[], &[])
}

/// [`project`], with extra nodes and ways in the extract (a test's own roads).
fn project_with(
    proj: &Path,
    with_lot: bool,
    with_road: bool,
    broken_lot: bool,
    extra_nodes: &[(i64, f64, f64)],
    extra_ways: &[(i64, Vec<i64>, Vec<(&str, &str)>)],
) -> Project {
    std::fs::create_dir_all(proj.join("dem")).unwrap();
    std::fs::create_dir_all(proj.join("osm")).unwrap();
    std::fs::write(proj.join("dem/hollow.tif"), dem()).unwrap();
    let lot = [
        (300.0, 300.0),
        (700.0, 300.0),
        (700.0, 700.0),
        (300.0, 700.0),
    ];
    let mut nodes: Vec<(i64, f64, f64)> = lot
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| (100 + i64::try_from(i).unwrap(), lat_of(y), lon_of(x)))
        .collect();
    nodes.push((50, lat_of(500.0), lon_of(100.0)));
    nodes.push((51, lat_of(500.0), lon_of(400.0)));
    let mut ways = Vec::new();
    if with_road {
        ways.push((10, vec![50, 51], vec![("highway", "service")]));
    }
    if with_lot {
        ways.push((
            100,
            vec![100, 101, 102, 103, 100],
            vec![("amenity", "parking"), ("surface", "asphalt")],
        ));
    }
    if broken_lot {
        // a car park whose ring has two corners: the extract cannot make an area of it
        nodes.push((120, lat_of(900.0), lon_of(100.0)));
        nodes.push((121, lat_of(900.0), lon_of(120.0)));
        ways.push((101, vec![120, 121, 120], vec![("amenity", "parking")]));
    }
    nodes.extend_from_slice(extra_nodes);
    ways.extend(extra_ways.iter().cloned());
    std::fs::write(proj.join("osm/lot.osm.pbf"), pbf(&nodes, &ways)).unwrap();
    let mut p = Project::new("Test", CENTRE.0, CENTRE.1, MAP_M);
    p.scale.horizontal = 1.0;
    p.scale.height = 1.0;
    p.grid.terrain_size = 512;
    p.grid.land_size = 64;
    p.sources.dem = vec!["dem/hollow.tif".into()];
    p.sources.osm = vec!["osm/lot.osm.pbf".into()];
    p.save(proj).unwrap();
    p
}

fn quiet(_: &str) {}

fn setup() -> (tempfile::TempDir, FakeEnv, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let env = FakeEnv::new(&dir.path().join("fake")).unwrap();
    let proj = dir.path().join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    (dir, env, proj)
}

fn stages(lots: bool) -> Stages {
    Stages {
        lots,
        ..Stages::default()
    }
}

/// Every file under `root`, relative path and bytes, sorted by path.
fn tree(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                found.push((relative, std::fs::read(&path).unwrap()));
            }
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

#[test]
fn with_the_stage_off_the_build_with_a_lot_is_the_build_without_it() {
    let (_dir, env, proj) = setup();
    let p = project(&proj, true, true, false);
    let with_lot = generate_with(&proj, &p, &env.tools(), stages(false), &quiet).unwrap();
    assert!(with_lot.lots.is_none());
    // the same project without the lot's area, in a second folder
    let (_dir2, env2, proj2) = setup();
    let p2 = project(&proj2, false, true, false);
    let without = generate_with(&proj2, &p2, &env2.tools(), stages(false), &quiet).unwrap();
    // the terrain and the ground are the same, vertex for vertex and cell for cell
    assert_eq!(with_lot.heightmap.h, without.heightmap.h);
    assert_eq!(with_lot.surfaces, without.surfaces);
    // and so is every written layer file of the build
    build_with(&proj, &env.tools(), stages(false), &quiet).unwrap();
    build_with(&proj2, &env2.tools(), stages(false), &quiet).unwrap();
    let layers_of = |dir: &Path| tree(&dir.join("build/staging/P/naves/worlds/test/data/layers"));
    assert_eq!(layers_of(&proj), layers_of(&proj2));
    assert_ne!(layers_of(&proj).len(), 0);
}

#[test]
fn with_the_stage_on_the_ground_is_the_lots_target_and_its_surface_is_painted() {
    let (_dir, env, proj) = setup();
    let p = project(&proj, true, true, false);
    let lines = std::cell::RefCell::new(Vec::new());
    let generated = generate_with(&proj, &p, &env.tools(), stages(true), &|line| {
        lines.borrow_mut().push(line.to_string());
    })
    .unwrap();
    let report = generated.lots.as_ref().expect("the lot stage ran");
    assert_eq!(report.counts().smoothed, 1, "{:?}", report.counts());
    let surface = report.surfaces.first().expect("a surface");
    // every terrain vertex inside the lot is the lot's target there, except under the cut access road, whose core
    // and held joints win there (the road ends 4 m inside the lot, at x = 304)
    let cell = f64::from(generated.heightmap.cell_m);
    let mut inside = 0;
    for row in 0..generated.heightmap.size {
        for col in 0..generated.heightmap.size {
            let at = (f64::from(col) * cell, f64::from(row) * cell);
            if !surface.contains(at) || at.0 < 350.0 {
                continue;
            }
            inside += 1;
            let ground = f64::from(generated.base.at(col, row));
            let target = surface.target(at, ground);
            let shaped = f64::from(generated.heightmap.at(col, row));
            assert!(
                (shaped - target).abs() <= 1e-3,
                "({at:?}): {shaped} vs {target}"
            );
            // the grid is rasterised: only cells at least two of its own cells inside the edge are sure to be
            // painted (a cell that straddles the edge may take its centre from outside)
            if naves_core::lot::edge_distance(&report.assessed[0].area, at)
                >= 2.0 * generated.surfaces.cell_m
            {
                assert_eq!(
                    generated.surfaces.at(at.0, at.1),
                    SurfaceClass::Soil,
                    "({at:?})"
                );
            }
        }
    }
    assert!(inside > 100, "{inside} vertices inside the lot");
    // the lot was shaped: the hollow is filled where the lot's plane lies above it
    assert_ne!(generated.heightmap.h, generated.base.h);
    // paved by its own `surface=asphalt` tag, decided tarmac and painted soil (`TARMAC_PAINTED` is off) everywhere in
    // the lot, not only at its middle
    assert_eq!(report.class_of("osm/way/100"), Some(SurfaceClass::Tarmac));
    let area = &report.assessed[0].area;
    let middle = naves_core::lot::centre(area);
    assert_eq!(
        generated.surfaces.at(middle.0, middle.1),
        SurfaceClass::Soil,
        "{middle:?}"
    );
    let high = area
        .outer
        .iter()
        .fold((0.0_f64, 0.0_f64), |(x, y), p| (x.max(p.0), y.max(p.1)));
    for share in [0.25, 0.5, 0.75] {
        let at = (
            middle.0 + share * (high.0 - middle.0),
            middle.1 + share * (high.1 - middle.1),
        );
        assert_eq!(
            generated.surfaces.at(at.0, at.1),
            SurfaceClass::Soil,
            "{at:?}"
        );
    }
    // the service road ends inside the lot: it is cut there
    assert_eq!(generated.clean.lot_access.len(), 1);
    assert!(
        lines
            .borrow()
            .iter()
            .any(|line| line.starts_with("Assessing the lots (") && line.ends_with(" s)")),
        "{:?}",
        lines.borrow()
    );
}

#[test]
fn the_report_says_what_the_lot_stage_did() {
    let (_dir, env, proj) = setup();
    project(&proj, true, true, false);
    let out = build_with(&proj, &env.tools(), stages(true), &quiet).unwrap();
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(proj.join("build/report.json")).unwrap())
            .unwrap();
    let lots = &report["roads"]["lots"];
    assert_eq!(lots["enabled"], true);
    assert_eq!(lots["smoothed"], 1);
    assert_eq!(lots["kinds"]["car_park"], 1);
    assert_eq!(lots["paved"], 1);
    assert_eq!(lots["roads_removed"], 0);
    assert_eq!(lots["access_cut"], 1);
    assert_eq!(lots["holes_dropped"], 0);
    assert_eq!(lots["entries_lost"], 0);
    let lot = &lots["lots"][0];
    assert_eq!(lot["id"], "osm/way/100");
    assert_eq!(lot["kind"], "car_park");
    assert_eq!(lot["outcome"], "smoothed");
    assert_eq!(lot["surface"], "tarmac");
    assert_eq!(lot["surface_source"], "tag");
    assert!(lot["form"].is_string());
    assert!(lot["lambda"].as_f64().unwrap() <= 1.0);
    assert!(lot["x"].as_f64().unwrap() > 400.0);
    // the summary line the build prints
    let line = out.lots.as_deref().expect("a lot summary");
    assert!(
        line.starts_with("Lots: 1 (1 car park), 1 smoothed"),
        "{line}"
    );
}

#[test]
fn an_access_road_that_leaves_a_lot_from_a_close_t_keeps_a_placed_road() {
    // a service road leaves the lot's west edge from a T on a tertiary road that runs 2 m outside it. Cut 4 m inside
    // the lot it would be 6 m long, and the placement drops that (what is left after the T's trim is shorter than
    // half a piece is wide), so the lot would lose its access road (game check of 2026-11-27: a lot of the example
    // lost it so). The cut keeps at least `LOT_ACCESS_MIN_M` of the road instead, enough to be placed.
    let (_dir, env, proj) = setup();
    let nodes = [
        (60, lat_of(300.0), lon_of(298.0)),
        (61, lat_of(700.0), lon_of(298.0)),
        (62, lat_of(500.0), lon_of(298.0)),
        (63, lat_of(500.0), lon_of(340.0)),
    ];
    let ways = [
        (11, vec![60, 62, 61], vec![("highway", "tertiary")]),
        (12, vec![62, 63], vec![("highway", "service")]),
    ];
    project_with(&proj, true, false, false, &nodes, &ways);
    let out = build_with(&proj, &env.tools(), stages(true), &quiet).unwrap();
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(proj.join("build/report.json")).unwrap())
            .unwrap();
    let lots = &report["roads"]["lots"];
    assert_eq!(lots["smoothed"], 1);
    // the cut happened, and the access road it left is placed: nothing is lost
    assert_eq!(lots["access_cut"], 1);
    assert!(
        !out.warnings
            .iter()
            .any(|warning| warning.contains("shorter than half its width and not placed")),
        "{:?}",
        out.warnings
    );
    assert_eq!(lots["entries_lost"], 0);
    let line = out.lots.as_deref().expect("a lot summary");
    assert!(line.contains(", 1 access road cut"), "{line}");
    assert!(!line.contains("entry lost"), "{line}");
    // the only finding on it is the cut, not a loss
    let dropped: Vec<&str> = report["violations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|violation| violation["rule"] == Rule::RoadInLotDropped.id())
        .map(|violation| violation["message"].as_str().unwrap())
        .collect();
    assert!(
        dropped.iter().all(|message| !message.contains("is lost")),
        "{dropped:?}"
    );
    assert_eq!(
        dropped
            .iter()
            .filter(
                |message| message.starts_with("The access road 12:62-63 of lot osm/way/100 is cut")
            )
            .count(),
        1,
        "{dropped:?}"
    );
}

#[test]
fn with_the_stage_off_the_report_says_it_is_off() {
    let (_dir, env, proj) = setup();
    project(&proj, true, true, false);
    let out = build_with(&proj, &env.tools(), stages(false), &quiet).unwrap();
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(proj.join("build/report.json")).unwrap())
            .unwrap();
    assert_eq!(
        report["roads"]["lots"],
        serde_json::json!({ "enabled": false })
    );
    assert!(out.lots.is_none());
    // the lot findings do not appear either
    assert!(
        report["violations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| !v["rule"].as_str().unwrap().starts_with("lot_")
                && v["rule"] != Rule::RoadInLotDropped.id())
    );
}

#[test]
fn a_lot_in_an_extract_without_any_road_gets_its_surface_and_its_smooth_ground() {
    // a car park with no road in it is the usual case (most car parks have no road of their own): the lot stage
    // runs without a placement, the ground is shaped and the surface painted
    let (_dir, env, proj) = setup();
    let p = project(&proj, true, false, false);
    let generated = generate_with(&proj, &p, &env.tools(), stages(true), &quiet).unwrap();
    let report = generated.lots.as_ref().expect("the lot stage ran");
    assert_eq!(report.counts().smoothed, 1);
    assert_eq!(report.class_of("osm/way/100"), Some(SurfaceClass::Tarmac));
    // the lot is decided paved, and painted soil: the game ignores the mask alpha that would select the fifth layer
    // (tarmac), so a paved lot is painted with the soil layer (`TARMAC_PAINTED`)
    assert_eq!(generated.surfaces.at(500.0, 500.0), SurfaceClass::Soil);
    // the hollow inside the lot is filled: the ground is the lot's plane, not the base
    assert_ne!(generated.heightmap.h, generated.base.h);
    let surface = report.surfaces.first().expect("a surface");
    let at = (500.0, 500.0);
    let ground = f64::from(generated.base.sample(at.0, at.1).height);
    let shaped = f64::from(generated.heightmap.sample(at.0, at.1).height);
    assert!((shaped - surface.target(at, ground)).abs() <= 1e-3);
    // no road, no object
    assert_eq!(generated.objects.len(), 0);
    // and with the stage off it is the project without the lot: nothing is shaped or painted
    let (_dir2, env2, proj2) = setup();
    let p2 = project(&proj2, true, false, false);
    let off = generate_with(&proj2, &p2, &env2.tools(), stages(false), &quiet).unwrap();
    assert!(off.lots.is_none());
    assert_eq!(off.heightmap.h, off.base.h);
    assert_ne!(off.surfaces.at(500.0, 500.0), SurfaceClass::Soil);
}

#[test]
fn plan_roads_places_the_roads_without_the_lots() {
    let catalog = synthetic_catalog();
    let base = flat();
    let project = Project::new("P", 47.55, 12.975, 1000.0);
    let frame = MapGeo::new(&project).unwrap();
    let plan = plan_roads_with(
        &camp_site_roads(),
        &catalog,
        &base,
        &[],
        &[],
        &frame,
        Stages {
            reroute: false,
            smooth: false,
            lots: false,
        },
        &[],
        &quiet,
    )
    .unwrap();
    assert_ne!(plan.placed.net.roads.len(), 0);
    assert!(plan.placed.net.lots.is_none());
    assert_eq!(plan.placed.net.clean.lot_roads.len(), 0);
}

#[test]
fn a_lot_ring_the_extract_cannot_make_an_area_of_is_reported() {
    let (_dir, env, proj) = setup();
    let p = project(&proj, true, true, true);
    let generated = generate_with(&proj, &p, &env.tools(), stages(true), &quiet).unwrap();
    assert!(
        generated
            .warnings
            .iter()
            .any(|warning| warning.contains("lot ring(s) were skipped")),
        "{:?}",
        generated.warnings
    );
    // the good lot is still there
    assert_eq!(generated.lots.as_ref().expect("a report").counts().total, 1);
}

#[test]
fn with_the_stage_on_a_project_without_roads_or_lots_says_so() {
    // the stage is on and there is nothing for it: it reports that, it does not look switched off
    let (_dir, env, proj) = setup();
    let p = project(&proj, false, false, false);
    let generated = generate_with(&proj, &p, &env.tools(), stages(true), &quiet).unwrap();
    let report = generated.lots.as_ref().expect("the stage is on");
    assert_eq!(report.counts().total, 0);
    assert_eq!(report.summary(&generated.clean), "Lots: none");
    let out = build_with(&proj, &env.tools(), stages(true), &quiet).unwrap();
    assert_eq!(out.lots.as_deref(), Some("Lots: none"));
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(proj.join("build/report.json")).unwrap())
            .unwrap();
    assert_eq!(json["roads"]["lots"]["enabled"], true);
    assert_eq!(json["roads"]["lots"]["smoothed"], 0);
    assert_eq!(json["roads"]["lots"]["lots"].as_array().unwrap().len(), 0);
}

#[test]
fn the_smoothing_stage_has_its_own_progress_line_when_it_is_on() {
    // the two stages print different lines, and each only when it ran: the smoothing one is not the lot one
    let catalog = synthetic_catalog();
    let base = flat();
    let project = Project::new("P", 47.55, 12.975, 1000.0);
    let frame = MapGeo::new(&project).unwrap();
    let lines = std::cell::RefCell::new(Vec::new());
    plan_roads_with(
        &camp_site_roads(),
        &catalog,
        &base,
        &[],
        &[],
        &frame,
        Stages {
            reroute: false,
            smooth: true,
            lots: false,
        },
        &[],
        &|line| lines.borrow_mut().push(line.to_string()),
    )
    .unwrap();
    assert!(
        lines
            .borrow()
            .iter()
            .any(|line| line.starts_with("Smoothing the road lines (") && line.ends_with(" s)")),
        "{:?}",
        lines.borrow()
    );
    assert!(
        lines
            .borrow()
            .iter()
            .all(|line| !line.starts_with("Assessing the lots (")),
        "{:?}",
        lines.borrow()
    );
}

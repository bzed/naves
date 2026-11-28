// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The build applies the hand-made road edits of `edits/ways.toml`.

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

use naves_core::edits::{By, ElementId, USER_ID_BASE, WayEdits};
use naves_core::osm::OsmWay;
use naves_core::project::Project;
use naves_core::validate::Rule;
use naves_export::build::{BuildError, build_with};
use naves_export::generate::{Stages, generate_with, read_source};
use naves_export::testing::FakeEnv;
use tiff::encoder::{TiffEncoder, colortype};
use tiff::tags::Tag;

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
/// A project of 1000 m with the DEM and, with `road`, the extract with one service road (way 10) from x = 100 to
/// x = 400 along y = 500.
fn project(proj: &Path, road: bool) -> Project {
    std::fs::create_dir_all(proj.join("dem")).unwrap();
    std::fs::write(proj.join("dem/hollow.tif"), dem()).unwrap();
    let mut p = Project::new("Test", CENTRE.0, CENTRE.1, MAP_M);
    p.scale.horizontal = 1.0;
    p.scale.height = 1.0;
    p.grid.terrain_size = 512;
    p.grid.land_size = 64;
    p.sources.dem = vec!["dem/hollow.tif".into()];
    if road {
        std::fs::create_dir_all(proj.join("osm")).unwrap();
        let nodes = [
            (50, lat_of(500.0), lon_of(100.0)),
            (51, lat_of(500.0), lon_of(400.0)),
        ];
        let ways = [(10, vec![50, 51], vec![("highway", "service")])];
        std::fs::write(proj.join("osm/road.osm.pbf"), pbf(&nodes, &ways)).unwrap();
        p.sources.osm = vec!["osm/road.osm.pbf".into()];
    }
    p.save(proj).unwrap();
    p
}

/// The original of way 10, as the extract has it.
fn original() -> OsmWay {
    OsmWay {
        id: 10,
        tags: [("highway".to_string(), "service".to_string())].into(),
        nodes: vec![50, 51],
        points: vec![
            (lat_of(500.0), lon_of(100.0)),
            (lat_of(500.0), lon_of(400.0)),
        ],
    }
}

fn setup() -> (tempfile::TempDir, FakeEnv, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let env = FakeEnv::new(&dir.path().join("fake")).unwrap();
    let proj = dir.path().join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    (dir, env, proj)
}

fn quiet(_: &str) {}

fn highway(kind: &str) -> std::collections::BTreeMap<String, String> {
    [("highway".to_string(), kind.to_string())].into()
}

/// Adds a track of two user nodes at the given map metres.
fn add_track(edits: &mut WayEdits, from: (f64, f64), to: (f64, f64)) -> ElementId {
    let a = edits
        .new_node(lat_of(from.1), lon_of(from.0), By::User)
        .unwrap();
    let b = edits
        .new_node(lat_of(to.1), lon_of(to.0), By::User)
        .unwrap();
    edits
        .add_way(highway("track"), vec![a, b], By::User)
        .unwrap()
}

#[test]
fn edits_change_the_built_roads() {
    let (_dir, env, proj) = setup();
    let p = project(&proj, true);
    let before = generate_with(&proj, &p, &env.tools(), Stages::default(), &quiet).unwrap();
    assert!(before.features.iter().any(|f| f.way_id == 10));
    assert!(before.stats.pieces > 0);
    assert_eq!(before.edits.applied, 0);

    let mut edits = WayEdits::default();
    edits.delete_way(&original(), By::User).unwrap();
    edits.save(&proj).unwrap();
    let after = generate_with(&proj, &p, &env.tools(), Stages::default(), &quiet).unwrap();
    assert!(after.features.iter().all(|f| f.way_id != 10));
    assert_eq!(after.stats.pieces, 0);
    assert_eq!(after.edits.applied, 1);
    assert!(after.violations.iter().all(|v| v.rule != Rule::EditBroken));
}

#[test]
fn custom_map_without_osm() {
    let (_dir, env, proj) = setup();
    let p = project(&proj, false);
    let mut edits = WayEdits::default();
    let way = add_track(&mut edits, (100.0, 500.0), (400.0, 500.0));
    edits.save(&proj).unwrap();
    let generated = generate_with(&proj, &p, &env.tools(), Stages::default(), &quiet).unwrap();
    assert_eq!(generated.edits.applied, 1);
    assert_eq!(generated.features.len(), 1);
    assert!(
        generated.features[0]
            .id
            .starts_with(&format!("user/way/{}/", way.raw().unwrap() - (1_i64 << 40))),
        "{}",
        generated.features[0].id
    );
    assert!(generated.stats.pieces > 0, "{:?}", generated.stats);
}

#[test]
fn edit_outside_map_is_reported() {
    let (_dir, env, proj) = setup();
    let p = project(&proj, false);
    let mut edits = WayEdits::default();
    let way = add_track(&mut edits, (100.0, 5500.0), (400.0, 5500.0));
    edits.save(&proj).unwrap();
    let generated = generate_with(&proj, &p, &env.tools(), Stages::default(), &quiet).unwrap();
    let found: Vec<_> = generated
        .violations
        .iter()
        .filter(|v| v.rule == Rule::EditOutsideMap)
        .collect();
    assert_eq!(found.len(), 1, "{:?}", generated.violations);
    assert!(
        found[0].message.contains(&format!("way {way}")),
        "{}",
        found[0].message
    );
    assert_eq!(generated.edits_outside.len(), 1);
    assert_eq!(generated.features.len(), 0);
}

#[test]
fn broken_edit_does_not_stop_the_build() {
    let (_dir, env, proj) = setup();
    let p = project(&proj, false);
    let mut edits = WayEdits::default();
    let a = edits
        .new_node(lat_of(500.0), lon_of(100.0), By::User)
        .unwrap();
    edits
        .add_way(highway("track"), vec![a, ElementId::Osm(987_654)], By::User)
        .unwrap();
    edits.save(&proj).unwrap();
    let generated = generate_with(&proj, &p, &env.tools(), Stages::default(), &quiet).unwrap();
    let found: Vec<_> = generated
        .violations
        .iter()
        .filter(|v| v.rule == Rule::EditBroken)
        .collect();
    assert_eq!(found.len(), 1, "{:?}", generated.violations);
    assert!(found[0].message.contains("987654"), "{}", found[0].message);
    assert_eq!(generated.features.len(), 0);
}

#[test]
fn damaged_edits_file_stops_the_build() {
    let (_dir, env, proj) = setup();
    let p = project(&proj, false);
    let mut edits = WayEdits::default();
    edits.new_node(47.55, 12.975, By::User).unwrap();
    edits.save(&proj).unwrap();
    let file = proj.join("edits/ways.toml");
    let text = std::fs::read_to_string(&file).unwrap();
    let damaged: Vec<String> = text
        .lines()
        .map(|l| {
            if l.starts_with("lat =") {
                "lat = 200.0".to_string()
            } else {
                l.to_string()
            }
        })
        .collect();
    std::fs::write(&file, damaged.join("\n")).unwrap();
    let err = generate_with(&proj, &p, &env.tools(), Stages::default(), &quiet).unwrap_err();
    assert!(matches!(err, BuildError::Edits { .. }), "{err}");
    let text = err.to_string();
    assert!(text.contains("ways.toml"), "{text}");
    assert!(text.contains("node"), "{text}");
}

#[test]
fn the_report_counts_the_edits() {
    let (_dir, env, proj) = setup();
    project(&proj, false);
    let mut edits = WayEdits::default();
    add_track(&mut edits, (100.0, 500.0), (400.0, 500.0));
    edits.save(&proj).unwrap();
    let out = build_with(&proj, &env.tools(), Stages::default(), &quiet).unwrap();
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(proj.join("build/report.json")).unwrap())
            .unwrap();
    assert_eq!(report["edits"]["applied"], 1);
    assert_eq!(report["edits"]["broken"], 0);
    assert_eq!(report["edits"]["changed"], 0);
    assert_eq!(report["edits"]["outside"], 0);
    let line = out.edits.as_deref().expect("an edits line");
    assert!(line.contains("1 applied"), "{line}");
}

#[test]
fn read_source_applies_the_edits() {
    let (_dir, _env, proj) = setup();
    let p = project(&proj, true);
    let mut edits = WayEdits::default();
    edits.delete_way(&original(), By::User).unwrap();
    let way = add_track(&mut edits, (100.0, 500.0), (400.0, 500.0));
    edits.save(&proj).unwrap();
    let source = read_source(&proj, &p).unwrap();
    assert!(source.extract.ways.iter().any(|w| w.id == 10));
    assert!(source.ways.ways.iter().all(|w| w.id != 10));
    let ElementId::User(n) = way else {
        panic!("{way:?}")
    };
    let user_id = USER_ID_BASE + i64::try_from(n).unwrap();
    assert!(source.ways.ways.iter().any(|w| w.id == user_id));
    assert_eq!(source.report.applied, 2);
}

#[test]
fn read_source_without_osm_is_the_edits_alone() {
    let (_dir, _env, proj) = setup();
    let p = project(&proj, false);
    let mut edits = WayEdits::default();
    add_track(&mut edits, (100.0, 500.0), (400.0, 500.0));
    edits.save(&proj).unwrap();
    let source = read_source(&proj, &p).unwrap();
    assert_eq!(source.extract.ways.len(), 0);
    assert_eq!(source.ways.ways.len(), 1);
}

#[test]
fn reapply_follows_the_edit_set() {
    let (_dir, _env, proj) = setup();
    let p = project(&proj, true);
    let mut source = read_source(&proj, &p).unwrap();
    assert!(source.ways.ways.iter().any(|w| w.id == 10));
    source.edits.delete_way(&original(), By::User).unwrap();
    source.reapply();
    assert!(source.ways.ways.iter().all(|w| w.id != 10));
    assert!(source.extract.ways.iter().any(|w| w.id == 10));
}

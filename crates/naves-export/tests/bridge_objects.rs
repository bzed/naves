// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! An OSM bridge goes through the generation as a bridge object of the model that fits it, unscaled, and
//! every OSM bridge says what it was built from.

#![cfg(unix)]
#![expect(
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::type_complexity,
    reason = "test helper code; failing loudly is the point"
)]

use std::path::Path;

use naves_core::bridgekind::BridgeKind;
use naves_core::catalog::testing::synthetic_catalog;
use naves_core::objects::piece_transform;
use naves_core::project::Project;
use naves_export::generate::generate;
use naves_export::testing::FakeEnv;

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

#[test]
fn an_osm_bridge_reaches_the_object_transform_unscaled() {
    let dir = tempfile::tempdir().unwrap();
    let env = FakeEnv::new(&dir.path().join("fake")).unwrap();
    let proj = dir.path().join("proj");
    std::fs::create_dir_all(proj.join("dem")).unwrap();
    std::fs::create_dir_all(proj.join("osm")).unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/dem/koenigssee.tif");
    std::fs::copy(fixture, proj.join("dem/koenigssee.tif")).unwrap();

    // a gravel track (6 m) east to west through the middle with a bridge of 30 m (15 m on the map at half
    // scale): it gets the plain wooden deck (19.7 m), laid as it is (bridges were scaled to the road's width
    // before)
    let lat = 47.55;
    let nodes = [
        (1, lat, 12.9715),
        (2, lat, 12.9745),
        (3, lat, 12.9749),
        (4, lat, 12.9776),
    ];
    let track = vec![("highway", "track"), ("tracktype", "grade2")];
    let mut bridge_tags = track.clone();
    bridge_tags.push(("bridge", "yes"));
    let ways = [
        (10, vec![1, 2], track.clone()),
        (11, vec![2, 3], bridge_tags),
        (12, vec![3, 4], track),
    ];
    std::fs::write(proj.join("osm/bridge.osm.pbf"), pbf(&nodes, &ways)).unwrap();

    let mut p = Project::new("Test", 47.55, 12.975, 1500.0);
    p.grid.terrain_size = 256;
    p.grid.land_size = 32;
    p.sources.dem = vec!["dem/koenigssee.tif".into()];
    p.sources.osm = vec!["osm/bridge.osm.pbf".into()];
    p.save(&proj).unwrap();

    let generated = generate(&proj, &p, &env.tools(), &|_| {}).unwrap();
    let bridges: Vec<usize> = generated
        .objects
        .iter()
        .enumerate()
        .filter(|(_, o)| o.model.contains("bridge"))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(bridges.len(), 1, "{:?}", generated.warnings);
    let i = bridges[0];
    let object = &generated.objects[i];
    assert!((object.scale - 1.0).abs() < 1e-9, "{}", object.scale);

    let transform = piece_transform(object, generated.heights[i]);
    // the matrix is the rotation times the scale: its columns have the length of the scale
    for column in 0..3 {
        let length = (0..3)
            .map(|row| f64::from(transform[row * 3 + column]).powi(2))
            .sum::<f64>()
            .sqrt();
        assert!((length - 1.0).abs() < 1e-5, "column {column}: {length}");
    }
    assert!((f64::from(transform[4]) - 1.0).abs() < 1e-6);

    assert!(object.model.contains("bridge_wood2"), "{}", object.model);
    // the height is the deck level less the model's deck height; the deck follows the ground at the bridge's
    // ends, which is flat enough here to compare with its middle
    let deck = synthetic_catalog()
        .bridges()
        .iter()
        .find(|b| b.model == object.model)
        .unwrap()
        .deck_height;
    let ground = f64::from(generated.heightmap.sample(object.x, object.y).height);
    let height = f64::from(generated.heights[i]);
    assert!(
        (height + deck - ground).abs() < 1.5,
        "height {height}, deck {deck}, ground {ground}"
    );
    assert!(
        (f64::from(transform[10]) - height).abs() < 1e-6,
        "the transform carries the height"
    );
}

#[test]
fn a_bridge_between_two_junctions_without_room_is_a_panel_row_of_its_length() {
    let dir = tempfile::tempdir().unwrap();
    let env = FakeEnv::new(&dir.path().join("fake")).unwrap();
    let proj = dir.path().join("proj");
    std::fs::create_dir_all(proj.join("dem")).unwrap();
    std::fs::create_dir_all(proj.join("osm")).unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/dem/koenigssee.tif");
    std::fs::copy(fixture, proj.join("dem/koenigssee.tif")).unwrap();

    // the same track, with a side road at each end of a 7.5 m bridge (3.8 m on the map): no stretch continues
    // it, so there is no room to grow
    let lat = 47.55;
    let nodes = [
        (1, lat, 12.9715),
        (2, lat, 12.9745),
        (3, lat, 12.9746),
        (4, lat, 12.9776),
        (5, lat + 0.0015, 12.9745),
        (6, lat + 0.0015, 12.9746),
    ];
    let track = vec![("highway", "track"), ("tracktype", "grade2")];
    let mut bridge_tags = track.clone();
    bridge_tags.push(("bridge", "yes"));
    let ways = [
        (10, vec![1, 2], track.clone()),
        (11, vec![2, 3], bridge_tags),
        (12, vec![3, 4], track.clone()),
        (13, vec![2, 5], track.clone()),
        (14, vec![3, 6], track),
    ];
    std::fs::write(proj.join("osm/bridge.osm.pbf"), pbf(&nodes, &ways)).unwrap();

    let mut p = Project::new("Test", 47.55, 12.975, 1500.0);
    p.grid.terrain_size = 256;
    p.grid.land_size = 32;
    p.sources.dem = vec!["dem/koenigssee.tif".into()];
    p.sources.osm = vec!["osm/bridge.osm.pbf".into()];
    p.save(&proj).unwrap();

    let generated = generate(&proj, &p, &env.tools(), &|_| {}).unwrap();
    // there is no warning about room any more: a panel row covers the bridge and needs none (one panel,
    // stretched to the bridge's length)
    assert!(
        generated.warnings.iter().all(|w| !w.contains("no room")),
        "{:?}",
        generated.warnings
    );
    assert_eq!(generated.bridges.len(), 1, "{:?}", generated.bridges);
    let fit = &generated.bridges[0];
    assert_eq!(fit.stretches, vec!["11:2-3".to_string()]);
    match fit.kind {
        BridgeKind::Panels {
            count,
            abreast,
            scale,
        } => {
            assert_eq!((count, abreast), (1, 1));
            assert!((scale * 3.0 - fit.length).abs() < 1e-9, "{fit:?}");
        }
        _ => panic!("{fit:?}"),
    }
    assert!((fit.part_length - fit.length).abs() < 1e-9);
    assert_eq!(fit.grown, (0.0, 0.0));
    assert_eq!(fit.fallback, Vec::new());
    assert!(
        generated
            .objects
            .iter()
            .all(|o| !o.model.contains("bridge")),
        "no bridge model is laid"
    );
    // until panel rows are laid the stretch is ordinary road: the roads and junctions around it are built
    assert_ne!(generated.objects.len(), 0);
}

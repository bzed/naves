// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A road over a hollow in the ground goes through the generation as a bridge, a fill, or a report, with the
//! synthetic catalog and a synthetic DEM.

#![cfg(unix)]
#![expect(
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::type_complexity,
    reason = "test helper code; failing loudly is the point"
)]

use std::io::Cursor;
use std::path::Path;

use naves_core::project::Project;
use naves_core::validate::Rule;
use naves_export::generate::{Generated, generate};
use naves_export::testing::FakeEnv;
use tiff::encoder::{TiffEncoder, colortype};
use tiff::tags::Tag;

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

/// The ground of the DEM: flat at 500 m with a triangular hollow across it at the middle of the map.
const FLAT_M: f32 = 500.0;
/// The longitude of the hollow, and half a width of 20 m in degrees of longitude.
const HOLLOW_LON: f64 = 12.975;
const DEG_PER_M_LON: f64 = 1.0 / 75_100.0;

/// A GeoTIFF of 0.00005 degree pixels around the map with a hollow of `width_m` and `depth_m` at the middle.
fn dem(width_m: f64, depth_m: f64) -> Vec<u8> {
    let (west, north, pixel) = (12.96, 47.56, 0.00005);
    let (columns, rows) = (600_u32, 400_u32);
    let mut data = Vec::new();
    for _ in 0..rows {
        for c in 0..columns {
            let lon = west + (f64::from(c) + 0.5) * pixel;
            let across = (lon - HOLLOW_LON).abs() / (DEG_PER_M_LON * width_m / 2.0);
            data.push(FLAT_M - (depth_m * (1.0 - across).max(0.0)) as f32);
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

/// A project of 1000 m (scale 1:1) with a road from west to east through the middle, and `streams`: each
/// a line from south to north across the road at that longitude.
fn project(dir: &Path, width_m: f64, depth_m: f64, streams: &[f64]) -> Project {
    let proj = dir.join("proj");
    std::fs::create_dir_all(proj.join("dem")).unwrap();
    std::fs::create_dir_all(proj.join("osm")).unwrap();
    std::fs::write(proj.join("dem/hollow.tif"), dem(width_m, depth_m)).unwrap();
    let lat = 47.55;
    let mut nodes = vec![(1, lat, 12.9695), (2, lat, 12.9805)];
    let road = vec![("highway", "tertiary")];
    let mut ways = vec![(10, vec![1, 2], road)];
    for (i, lon) in streams.iter().enumerate() {
        let id = 100 + 2 * i64::try_from(i).unwrap();
        nodes.push((id, lat - 0.0005, *lon));
        nodes.push((id + 1, lat + 0.0005, *lon));
        ways.push((id, vec![id, id + 1], vec![("waterway", "stream")]));
    }
    std::fs::write(proj.join("osm/road.osm.pbf"), pbf(&nodes, &ways)).unwrap();
    let mut p = Project::new("Test", lat, 12.975, 1000.0);
    p.scale.horizontal = 1.0;
    p.scale.height = 1.0;
    p.grid.terrain_size = 512;
    p.grid.land_size = 64;
    p.sources.dem = vec!["dem/hollow.tif".into()];
    p.sources.osm = vec!["osm/road.osm.pbf".into()];
    p.save(&proj).unwrap();
    p
}

fn build(dir: &Path, p: &Project) -> Generated {
    let env = FakeEnv::new(&dir.join("fake")).unwrap();
    generate(&dir.join("proj"), p, &env.tools(), &|_| {}).unwrap()
}

fn bridge_indices(generated: &Generated) -> Vec<usize> {
    generated
        .objects
        .iter()
        .enumerate()
        .filter(|(_, o)| o.model.contains("bridge"))
        .map(|(i, _)| i)
        .collect()
}

/// The height of the shaped ground and of the original ground at the middle of the hollow.
fn ground_at_hollow(generated: &Generated) -> (f64, f64) {
    let x = 500.0;
    (
        f64::from(generated.heightmap.sample(x, x).height),
        f64::from(generated.base.sample(x, x).height),
    )
}

#[test]
fn a_road_over_a_hollow_gets_bridges_and_the_ground_under_them_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path(), 40.0, 8.0, &[]);
    let generated = build(dir.path(), &p);
    let bridges = bridge_indices(&generated);
    assert!(!bridges.is_empty(), "{:?}", generated.warnings);
    for &i in &bridges {
        let id = generated.objects[i].id.as_ref().unwrap();
        assert!(id.feature.contains("/bridge0"), "{id}");
        // the ground under a bridge is the original ground
        let o = &generated.objects[i];
        let (shaped, original) = (
            generated.heightmap.sample(o.x, o.y).height,
            generated.base.sample(o.x, o.y).height,
        );
        assert!((shaped - original).abs() < 0.01, "{shaped} {original}");
    }
    // the dip is bridged: nothing is reported over water and the ground is not filled up to the road
    assert!(
        generated
            .violations
            .iter()
            .all(|v| v.rule != Rule::RoadOverWater),
        "{:?}",
        generated.violations
    );
    let (shaped, original) = ground_at_hollow(&generated);
    assert!((shaped - original).abs() < 0.01, "{shaped} {original}");
    assert!(original < f64::from(FLAT_M) - 5.0, "{original}");
}

#[test]
fn building_twice_gives_the_same_objects_and_heights() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let first = build(a.path(), &project(a.path(), 40.0, 8.0, &[]));
    let second = build(b.path(), &project(b.path(), 40.0, 8.0, &[]));
    assert!(!bridge_indices(&first).is_empty());
    assert_eq!(first.objects, second.objects);
    assert_eq!(first.heights, second.heights);
    assert_eq!(first.heightmap, second.heightmap);
}

#[test]
fn a_short_dry_dip_is_filled_up_to_the_road() {
    let dir = tempfile::tempdir().unwrap();
    // 8 m deep but only about 10 m of it more than 3 m below the road: less than half a 25 m model
    let p = project(dir.path(), 16.0, 8.0, &[]);
    let generated = build(dir.path(), &p);
    assert!(bridge_indices(&generated).is_empty());
    let (shaped, original) = ground_at_hollow(&generated);
    assert!(original < f64::from(FLAT_M) - 5.0, "{original}");
    // the ground under the road is at the road's height, flat at 500 m here
    assert!((shaped - f64::from(FLAT_M)).abs() < 1.0, "{shaped}");
    assert!(
        generated
            .violations
            .iter()
            .all(|v| v.rule != Rule::RoadOverWater),
        "a dry dip is filled, not reported"
    );
}

#[test]
fn a_short_dip_over_a_stream_is_reported_and_left_as_the_build_leaves_it() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path(), 16.0, 8.0, &[HOLLOW_LON]);
    let generated = build(dir.path(), &p);
    assert!(bridge_indices(&generated).is_empty());
    let wet: Vec<_> = generated
        .violations
        .iter()
        .filter(|v| v.rule == Rule::RoadOverWater)
        .collect();
    assert_eq!(wet.len(), 1, "{:?}", generated.violations);
    assert!((wet[0].at.0 - 500.0).abs() < 10.0, "{:?}", wet[0]);
    assert!(wet[0].message.starts_with("Road crosses water "));
}

#[test]
fn a_long_wet_dip_gets_a_chain_of_bridges_and_no_report() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path(), 120.0, 16.0, &[HOLLOW_LON]);
    let generated = build(dir.path(), &p);
    let bridges = bridge_indices(&generated);
    assert!(
        bridges.len() >= 2,
        "{} {:?}",
        bridges.len(),
        generated.warnings
    );
    assert!(
        generated
            .violations
            .iter()
            .all(|v| v.rule != Rule::RoadOverWater),
        "{:?}",
        generated.violations
    );
}

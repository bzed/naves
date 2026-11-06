// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tests for the ODOL reader. The synthetic builder below writes the layout observed in version
//! 56 files (see `docs/formats/road-objects.md`); no game data is used.

#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]
#![expect(
    clippy::float_cmp,
    reason = "the values are written by the builder and read back unchanged"
)]

use naves_formats::odol::{OdolError, read_odol};

const MEMORY: f32 = 999_999_986_991_104.0; // 10^15 as stored in the files
const GEOMETRY: f32 = 9_999_999_827_968.0; // 10^13
const VISUAL: f32 = 1.0;

fn put_f32s(out: &mut Vec<u8>, values: &[f32]) {
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
    out.push(0);
}

/// One LOD as the file stores it: a selection is a name and the vertex indices it holds.
#[derive(Default)]
struct LodSpec<'a> {
    points: &'a [[f32; 3]],
    selections: &'a [(&'a str, &'a [u16])],
    properties: &'a [(&'a str, &'a str)],
    /// Also write a proxy, sub-skeletons, a texture, faces, a section and a key frame.
    rich: bool,
}

fn lod_bytes(spec: &LodSpec<'_>) -> Vec<u8> {
    let mut b = Vec::new();
    let n = u32::from(spec.rich);
    put_u32(&mut b, n); // proxies
    if spec.rich {
        put_str(&mut b, "proxy\\model.p3d");
        b.extend_from_slice(&[0; 48 + 16]);
    }
    put_u32(&mut b, n); // sub-skeletons
    if spec.rich {
        b.extend_from_slice(&[0; 4]);
    }
    put_u32(&mut b, n); // skeleton map
    if spec.rich {
        put_u32(&mut b, 1);
        b.extend_from_slice(&[0; 4]);
    }
    b.extend_from_slice(&[0; 56]); // vertex count .. radius
    put_u32(&mut b, n); // textures
    if spec.rich {
        put_str(&mut b, "a.paa");
    }
    put_u32(&mut b, 0); // materials
    put_u32(&mut b, 0); // point to vertex
    put_u32(&mut b, 0); // vertex to point
    put_u32(&mut b, n); // faces
    b.extend_from_slice(&[0; 6]);
    if spec.rich {
        b.extend_from_slice(&[3, 0, 0, 1, 0, 2, 0]);
    }
    put_u32(&mut b, n); // sections
    if spec.rich {
        b.extend_from_slice(&[0; 5 * 4 + 2 + 4]);
        b.extend_from_slice(&(-1i32).to_le_bytes());
        put_str(&mut b, "material.rvmat");
        put_u32(&mut b, 1);
        put_f32s(&mut b, &[1.0]);
    }
    put_u32(&mut b, u32::try_from(spec.selections.len()).unwrap());
    for (name, vertices) in spec.selections {
        put_str(&mut b, name);
        put_u32(&mut b, 0); // faces
        b.extend_from_slice(&[0; 5]);
        put_u32(&mut b, 0); // sections
        put_u32(&mut b, u32::try_from(vertices.len()).unwrap());
        for v in *vertices {
            b.extend_from_slice(&v.to_le_bytes());
        }
        put_u32(&mut b, 0); // weights size
    }
    put_u32(&mut b, u32::try_from(spec.properties.len()).unwrap());
    for (k, v) in spec.properties {
        put_str(&mut b, k);
        put_str(&mut b, v);
    }
    put_u32(&mut b, n); // key frames
    if spec.rich {
        put_f32s(&mut b, &[0.5]);
        put_u32(&mut b, 1);
        put_f32s(&mut b, &[1.0, 2.0, 3.0]);
    }
    b.extend_from_slice(&[0; 17]);
    put_u32(&mut b, 0); // clip flags: count
    b.extend_from_slice(&[1, 0, 0, 0, 0]); // default fill and its value
    b.extend_from_slice(&[0; 16]); // UV range
    put_u32(&mut b, 0);
    b.extend_from_slice(&[1, 0, 0, 0, 0]);
    put_u32(&mut b, 1); // UV sets
    put_u32(&mut b, u32::try_from(spec.points.len()).unwrap());
    for p in spec.points {
        put_f32s(&mut b, p);
    }
    b
}

/// A whole file: header, model info, address table and the LODs one after the other.
fn file(version: u32, lods: &[(f32, Vec<u8>)]) -> Vec<u8> {
    build(version, lods).0
}

/// Like [`file`], and also the offset of the address table.
fn build(version: u32, lods: &[(f32, Vec<u8>)]) -> (Vec<u8>, usize) {
    build_with(version, lods, false)
}

/// Like [`build`]; with `skeleton` the model info names a skeleton with two bones.
fn build_with(version: u32, lods: &[(f32, Vec<u8>)], skeleton: bool) -> (Vec<u8>, usize) {
    let mut h = Vec::new();
    h.extend_from_slice(b"ODOL");
    put_u32(&mut h, version);
    put_u32(&mut h, u32::try_from(lods.len()).unwrap());
    put_f32s(&mut h, &lods.iter().map(|l| l.0).collect::<Vec<_>>());
    h.extend_from_slice(&[0; 48]);
    put_f32s(&mut h, &[-1.0, -2.0, -3.0, 4.0, 5.0, 6.0]); // bounding box
    h.extend_from_slice(&[0; 96 + 5 + 28 + 14 + 1]);
    if skeleton {
        put_str(&mut h, "OFP2_ManSkeleton");
        h.push(0); // discrete
        put_u32(&mut h, 2);
        for (bone, parent) in [("Pelvis", ""), ("Spine", "Pelvis")] {
            put_str(&mut h, bone);
            put_str(&mut h, parent);
        }
        put_str(&mut h, ""); // pivots
    } else {
        h.push(0); // skeleton name
    }
    h.push(0); // map type
    put_u32(&mut h, 0); // mass array
    h.extend_from_slice(&[0; 34]);
    put_str(&mut h, "road");
    put_str(&mut h, "");
    h.extend_from_slice(&[0; 6]);
    h.push(0); // not animated
    let table_at = h.len();
    let table = lods.len() * 9;
    let mut start = h.len() + table;
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    for (_, data) in lods {
        starts.push(u32::try_from(start).unwrap());
        start += data.len();
        ends.push(u32::try_from(start).unwrap());
    }
    for s in &starts {
        put_u32(&mut h, *s);
    }
    for e in &ends {
        put_u32(&mut h, *e);
    }
    h.extend(std::iter::repeat_n(1, lods.len()));
    for (_, data) in lods {
        h.extend_from_slice(data);
    }
    (h, table_at)
}

fn sample() -> Vec<u8> {
    let points = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]];
    let memory = lod_bytes(&LodSpec {
        points: &points,
        selections: &[("LB", &[2]), ("le", &[0, 1]), ("pb", &[])],
        properties: &[],
        ..LodSpec::default()
    });
    let geometry = lod_bytes(&LodSpec {
        points: &[],
        selections: &[],
        properties: &[("class", "road"), ("map", "main road")],
        ..LodSpec::default()
    });
    // the visual LOD is never parsed, so it may hold anything
    file(
        56,
        &[
            (VISUAL, vec![0xAB; 40]),
            (GEOMETRY, geometry),
            (MEMORY, memory),
        ],
    )
}

#[test]
fn memory_points_are_read() {
    let m = read_odol(&sample()).unwrap();
    assert_eq!(m.version, 56);
    assert_eq!(m.memory["lb"], vec![[7.0, 8.0, 9.0]]);
    // indices out of order keep the order of the selection
    assert_eq!(m.memory["le"], vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
    assert!(m.memory["pb"].is_empty());
    assert!(!m.memory.contains_key("LB"), "names are lower-cased");
}

#[test]
fn the_bounding_box_is_read() {
    let m = read_odol(&sample()).unwrap();
    assert_eq!(m.bbox_min, [-1.0, -2.0, -3.0]);
    assert_eq!(m.bbox_max, [4.0, 5.0, 6.0]);
}

#[test]
fn properties_are_read() {
    let m = read_odol(&sample()).unwrap();
    assert_eq!(m.properties["class"], "road");
    assert_eq!(m.properties["map"], "main road");
}

#[test]
fn a_model_without_a_memory_lod_has_no_points() {
    let geometry = lod_bytes(&LodSpec {
        points: &[],
        selections: &[],
        properties: &[],
        ..LodSpec::default()
    });
    let m = read_odol(&file(56, &[(VISUAL, vec![1, 2, 3]), (GEOMETRY, geometry)])).unwrap();
    assert!(m.memory.is_empty());
}

#[test]
fn other_versions_are_rejected() {
    let mut bytes = sample();
    bytes[4..8].copy_from_slice(&54u32.to_le_bytes());
    let e = read_odol(&bytes).unwrap_err();
    assert!(matches!(e, OdolError::Version(54)));
    assert!(e.to_string().contains("54"));
}

#[test]
fn a_wrong_signature_is_rejected() {
    let mut bytes = sample();
    bytes[0] = b'X';
    assert!(matches!(read_odol(&bytes), Err(OdolError::BadMagic)));
}

#[test]
fn every_truncation_is_an_error() {
    let bytes = sample();
    for len in 0..bytes.len() {
        assert!(read_odol(&bytes[..len]).is_err(), "prefix of {len} bytes");
    }
}

#[test]
fn lengths_do_not_allocate() {
    // the number of resolutions
    let mut bytes = sample();
    bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(read_odol(&bytes), Err(OdolError::Count { .. })));
    let mut bytes = sample();
    bytes[8..12].copy_from_slice(&0x7fff_ffffu32.to_le_bytes());
    assert!(read_odol(&bytes).is_err());
}

#[test]
fn a_huge_count_inside_the_memory_lod_is_an_error() {
    let mut lod = lod_bytes(&LodSpec {
        points: &[[0.0; 3]],
        selections: &[],
        properties: &[],
        ..LodSpec::default()
    });
    // proxies count
    lod[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(read_odol(&file(56, &[(MEMORY, lod)])).is_err());
}

#[test]
fn addresses_outside_the_file_are_errors() {
    let lods = [(GEOMETRY, vec![0; 8]), (MEMORY, vec![0; 8])];
    let n = lods.len();
    let (clean, table) = build(56, &lods);
    // an end address behind the end of the file
    let mut beyond = clean.clone();
    let last_end = table + 4 * n + 4 * (n - 1);
    beyond[last_end..last_end + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        read_odol(&beyond),
        Err(OdolError::Address { lod: 1, .. })
    ));
    // an end address before its start
    let mut backwards = clean;
    backwards[table + 4 * n..table + 4 * n + 4].copy_from_slice(&0u32.to_le_bytes());
    assert!(matches!(
        read_odol(&backwards),
        Err(OdolError::Address { lod: 0, .. })
    ));
}

#[test]
fn a_compressed_array_in_the_memory_lod_is_named() {
    let points = vec![[0.0f32; 3]; 100]; // 1200 bytes: stored compressed in a real file
    let lod = lod_bytes(&LodSpec {
        points: &points,
        selections: &[],
        properties: &[],
        ..LodSpec::default()
    });
    let e = read_odol(&file(56, &[(MEMORY, lod)])).unwrap_err();
    assert!(matches!(e, OdolError::Compressed { size: 1200, .. }), "{e}");
}

#[test]
fn a_bad_vertex_index_is_an_error() {
    let lod = lod_bytes(&LodSpec {
        points: &[[0.0; 3]],
        selections: &[("lb", &[5])],
        properties: &[],
        ..LodSpec::default()
    });
    let e = read_odol(&file(56, &[(MEMORY, lod)])).unwrap_err();
    assert!(matches!(e, OdolError::VertexIndex { index: 5, .. }), "{e}");
}

#[test]
fn a_geometry_lod_that_cannot_be_read_does_not_fail_the_model() {
    let memory = lod_bytes(&LodSpec {
        points: &[[1.0, 1.0, 1.0]],
        selections: &[("lb", &[0])],
        properties: &[],
        ..LodSpec::default()
    });
    let m = read_odol(&file(56, &[(GEOMETRY, vec![0xFF; 64]), (MEMORY, memory)])).unwrap();
    assert_eq!(m.memory["lb"], vec![[1.0, 1.0, 1.0]]);
    assert!(m.properties.is_empty());
}

/// Writes the fuzz seeds from the synthetic builder:
/// `NAVES_WRITE_SEEDS=1 cargo test -p naves-formats --test odol -- --ignored write_fuzz_seeds`.
#[test]
#[ignore = "regenerates the files under fuzz/corpus/odol_read"]
fn write_fuzz_seeds() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/odol_read");
    std::fs::create_dir_all(&dir).unwrap();
    let valid = sample();
    let mut huge = valid.clone();
    huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    let mut v54 = valid.clone();
    v54[4..8].copy_from_slice(&54u32.to_le_bytes());
    let seeds: [(&str, Vec<u8>); 5] = [
        ("valid", valid.clone()),
        ("truncated_half", valid[..valid.len() / 2].to_vec()),
        ("truncated_header", valid[..40].to_vec()),
        ("huge_count", huge),
        ("version_54", v54),
    ];
    for (name, bytes) in seeds {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
}

/// Reads the pieces of `structures_roads.pbo` from the game install and checks them against the
/// research note. `NAVES_ROADS_PBO` is the path of the PBO, `NAVES_ARMAKE2` the armake2 binary
/// (default `armake2` on `PATH`).
#[test]
#[ignore = "needs DayZ"]
fn road_pieces_match_the_research_note() {
    let pbo =
        std::env::var("NAVES_ROADS_PBO").expect("set NAVES_ROADS_PBO to structures_roads.pbo");
    let armake2 = std::env::var("NAVES_ARMAKE2").unwrap_or_else(|_| "armake2".into());
    let out = std::env::temp_dir().join(format!("naves-odol-test-{}", std::process::id()));
    let status = std::process::Command::new(armake2)
        .arg("unpack")
        .arg(&pbo)
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    let read = |name: &str| {
        let bytes = std::fs::read(out.join("Parts").join(format!("{name}.p3d"))).unwrap();
        read_odol(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"))
    };
    let near = |a: f32, b: f32, tol: f32| (a - b).abs() <= tol;
    let point = |m: &naves_formats::odol::OdolModel, k: &str| m.memory[k][0];

    let straight = read("asf1_25");
    assert_eq!(point(&straight, "lb"), [-6.0, 0.0, -12.5]);
    assert_eq!(point(&straight, "pe"), [6.0, 0.0, 12.5]);
    assert_eq!(
        straight.properties.get("class").map(String::as_str),
        Some("road")
    );

    let arc = read("asf3_10 25");
    let (lb, pb, le, pe) = (
        point(&arc, "lb"),
        point(&arc, "pb"),
        point(&arc, "le"),
        point(&arc, "pe"),
    );
    assert!(near(pb[0] - lb[0], 7.0, 0.01), "width");
    let mid = |a: [f32; 3], b: [f32; 3]| [f32::midpoint(a[0], b[0]), f32::midpoint(a[2], b[2])];
    let (s, e) = (mid(lb, pb), mid(le, pe));
    assert!(near((e[0] - s[0]).hypot(e[1] - s[1]), 4.358, 0.01), "chord");
    let turn = (pe[2] - le[2]).atan2(pe[0] - le[0]).to_degrees();
    assert!(near(turn.abs(), 10.0, 0.01), "heading change {turn}");

    for (family, width) in [
        ("asf1", 12.0),
        ("asf2", 9.0),
        ("asf3", 7.0),
        ("city", 10.0),
        ("grav", 6.0),
        ("mud", 5.0),
    ] {
        let m = read(&format!("{family}_25"));
        assert!(
            near(point(&m, "pb")[0] - point(&m, "lb")[0], width, 0.01),
            "{family}"
        );
    }
    // the two pieces the first scan could not read
    for name in ["asf1_10 25", "city_10 100"] {
        let m = read(name);
        for k in ["lb", "pb", "le", "pe"] {
            assert_eq!(m.memory[k].len(), 1, "{name} {k}");
        }
    }
    std::fs::remove_dir_all(out).unwrap();
}

#[test]
fn a_rich_memory_lod_and_a_skeleton_are_read() {
    let lod = lod_bytes(&LodSpec {
        points: &[[1.0, 2.0, 3.0]],
        selections: &[("lb", &[0])],
        properties: &[("class", "road")],
        rich: true,
    });
    let geometry = lod_bytes(&LodSpec {
        properties: &[("map", "track")],
        rich: true,
        ..LodSpec::default()
    });
    let (bytes, _) = build_with(56, &[(GEOMETRY, geometry), (MEMORY, lod)], true);
    let m = read_odol(&bytes).unwrap();
    assert_eq!(m.memory["lb"], vec![[1.0, 2.0, 3.0]]);
    assert_eq!(m.properties["class"], "road");
    assert_eq!(m.properties["map"], "track");
}

#[test]
fn an_animated_model_is_refused() {
    let (mut bytes, table) = build(56, &[(MEMORY, vec![0; 8])]);
    // the byte in front of the address table is the "has animations" flag
    bytes[table - 1] = 1;
    assert!(matches!(read_odol(&bytes), Err(OdolError::Unsupported(_))));
}

#[test]
fn a_material_inside_the_memory_lod_is_refused() {
    let mut lod = lod_bytes(&LodSpec::default());
    // textures (4) and then the materials count; proxies, sub-skeletons, skeleton map, 56 bytes
    let at = 12 + 56 + 4;
    lod[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
    let e = read_odol(&file(56, &[(MEMORY, lod)])).unwrap_err();
    assert!(matches!(e, OdolError::Unsupported(_)), "{e}");
}

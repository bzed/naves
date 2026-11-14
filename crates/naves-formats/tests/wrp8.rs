// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Round-trip and robustness tests for the editable world (8WVR) format.

#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]

use naves_formats::wrp8::{EditableWorld, FormatError, WorldObject};

const MATERIAL: &str = "naves\\x\\p_000-000.rvmat";

fn sample() -> EditableWorld {
    EditableWorld {
        land_range_x: 2,
        land_range_y: 2,
        terrain_range_x: 4,
        terrain_range_y: 4,
        cell_size: 25.0,
        elevation: (0..16)
            .map(|i| f32::from(u8::try_from(i).unwrap()) * 1.5)
            .collect(),
        material_index: vec![1, 1, 1, 1],
        materials: vec![String::new(), MATERIAL.to_string()],
        objects: vec![WorldObject {
            transform: [
                1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 10.0, 20.0, 30.0,
            ],
            id: 1,
            model: "a\\b.p3d".to_string(),
        }],
    }
}

fn encode(w: &EditableWorld) -> Vec<u8> {
    let mut buf = Vec::new();
    w.write(&mut buf).unwrap();
    buf
}

fn find(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
        .unwrap()
}

/// A quarter turn about the vertical axis, worked out by hand: the model's x
/// axis ends up pointing to map -z (`[0, 0, -1]`), its z axis to +x, and the
/// first three stored floats are the image of the x axis, not the first row.
#[test]
fn transform_is_stored_column_by_column() {
    let mut w = sample();
    let quarter_turn = [
        0.0, 0.0, -1.0, // image of x
        0.0, 1.0, 0.0, // image of y
        1.0, 0.0, 0.0, // image of z
        5.0, 6.0, 7.0,
    ];
    w.objects[0].transform = quarter_turn;
    let buf = encode(&w);
    let mut expected = Vec::new();
    for v in quarter_turn {
        expected.extend_from_slice(&f32::to_le_bytes(v));
    }
    let at = find(&buf, &expected);
    assert!(at > 0);
    let back = EditableWorld::read(&mut &buf[..]).unwrap();
    assert_eq!(
        back.objects[0].transform.map(f32::to_bits),
        quarter_turn.map(f32::to_bits)
    );
}

#[test]
fn roundtrip_small_world() {
    let w = sample();
    let buf = encode(&w);
    assert_eq!(&buf[0..4], b"8WVR");
    assert_eq!(EditableWorld::read(&mut &buf[..]).unwrap(), w);
}

#[test]
fn empty_material_zero_is_only_a_terminator() {
    let buf = encode(&sample());
    let name_at = find(&buf, MATERIAL.as_bytes());
    // 00 00 00 00 (entry 0), then len 0x17, then the name.
    assert_eq!(&buf[name_at - 8..name_at - 4], &[0, 0, 0, 0]);
    assert_eq!(&buf[name_at - 4..name_at], &[0x17, 0, 0, 0]);
    // The name is followed by its own 0 terminator.
    let after = name_at + MATERIAL.len();
    assert_eq!(&buf[after..after + 4], &[0, 0, 0, 0]);
    // material_count (2) sits right before entry 0.
    assert_eq!(&buf[name_at - 12..name_at - 8], &[2, 0, 0, 0]);
}

#[test]
fn terminator_object_written_and_dropped() {
    let buf = encode(&sample());
    let tail = &buf[buf.len() - (12 * 4 + 4 + 4)..];
    for chunk in tail[..48].as_chunks::<4>().0 {
        assert!(f32::from_le_bytes(*chunk).is_nan());
    }
    assert_eq!(&tail[48..52], &i32::MAX.to_le_bytes());
    assert_eq!(&tail[52..56], &[0, 0, 0, 0]);
    assert_eq!(EditableWorld::read(&mut &buf[..]).unwrap().objects.len(), 1);
}

#[test]
fn wrong_length_is_inconsistent() {
    let mut w = sample();
    w.elevation.pop();
    let err = w.write(&mut Vec::new()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn material_zero_must_be_empty() {
    let mut w = sample();
    w.materials[0] = "x".to_string();
    let err = w.write(&mut Vec::new()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn non_ascii_material_is_rejected_on_write() {
    let mut w = sample();
    w.materials[1] = "náves.rvmat".to_string();
    let err = w.write(&mut Vec::new()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn truncated_input_is_error() {
    let buf = encode(&sample());
    for len in 0..buf.len() {
        assert!(
            EditableWorld::read(&mut &buf[..len]).is_err(),
            "prefix of {len} bytes was accepted"
        );
    }
}

#[test]
fn bad_magic() {
    let mut buf = encode(&sample());
    buf[0] = b'X';
    assert!(matches!(
        EditableWorld::read(&mut &buf[..]),
        Err(FormatError::BadMagic)
    ));
}

#[test]
fn huge_header_is_rejected_before_allocating() {
    let mut buf = Vec::new();
    buf.extend_from_slice(b"8WVR");
    for v in [1u32, 1, 0xFFFF_FFFF, 2] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf.extend_from_slice(&25.0f32.to_le_bytes());
    assert!(matches!(
        EditableWorld::read(&mut &buf[..]),
        Err(FormatError::Inconsistent(_))
    ));
}

#[test]
fn huge_material_count_is_rejected() {
    let w = sample();
    let mut buf = encode(&w);
    let name_at = find(&buf, MATERIAL.as_bytes());
    buf[name_at - 12..name_at - 8].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    assert!(matches!(
        EditableWorld::read(&mut &buf[..]),
        Err(FormatError::Inconsistent(_))
    ));
}

#[test]
fn oversized_string_is_rejected() {
    let w = sample();
    let mut buf = encode(&w);
    let name_at = find(&buf, MATERIAL.as_bytes());
    buf[name_at - 4..name_at].copy_from_slice(&0x00FF_FFFFu32.to_le_bytes());
    assert!(matches!(
        EditableWorld::read(&mut &buf[..]),
        Err(FormatError::Inconsistent(_))
    ));
}

#[test]
fn terrain_builders_terminator_is_accepted() {
    // Terrain Builder writes twelve NaNs with the bit pattern 0x7fc87f88 and an arbitrary id (observed in
    // Bohemia's Test_Terrain sample), not our i32::MAX.
    let mut buf = encode(&sample());
    let tail = buf.len() - (12 * 4 + 4 + 4);
    for i in 0..12 {
        buf[tail + i * 4..tail + i * 4 + 4].copy_from_slice(&0x7fc8_7f88_u32.to_le_bytes());
    }
    buf[tail + 48..tail + 52].copy_from_slice(&23_260_988_i32.to_le_bytes());
    let world = EditableWorld::read(&mut &buf[..]).unwrap();
    assert_eq!(world.objects.len(), 1);
}

#[test]
fn the_writer_uses_terrain_builders_nan_pattern() {
    let buf = encode(&sample());
    let tail = buf.len() - (12 * 4 + 4 + 4);
    assert_eq!(&buf[tail..tail + 4], &0x7fc8_7f88_u32.to_le_bytes());
}

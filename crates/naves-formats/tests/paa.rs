// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! PAA colour tags: what `img2paa` leaves out and the game needs on terrain images.

#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test helper code; failing loudly is the point"
)]

use naves_formats::paa::{ColourTags, PaaError, with_colour_tags, without_stray_lzo_bytes};

/// A minimal DXT1 PAA as armake2 writes it: type, an `OFFS` tag, no palette, two mipmaps, terminator.
fn paa() -> Vec<u8> {
    let mut out = vec![0x01, 0xff];
    out.extend_from_slice(b"GGATSFFO");
    out.extend_from_slice(&64u32.to_le_bytes());
    let header = 2 + 12 + 64 + 2;
    let mip0 = header;
    let mip1 = mip0 + 7 + 8;
    let mut offs = [0u32; 16];
    offs[0] = u32::try_from(mip0).unwrap();
    offs[1] = u32::try_from(mip1).unwrap();
    for o in offs {
        out.extend_from_slice(&o.to_le_bytes());
    }
    out.extend_from_slice(&0u16.to_le_bytes()); // palette
    out.extend_from_slice(&[4, 0, 4, 0, 8, 0, 0]); // 4 x 4, 8 bytes
    out.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
    out.extend_from_slice(&[2, 0, 2, 0, 8, 0, 0]);
    out.extend_from_slice(&[9, 9, 9, 9, 9, 9, 9, 9]);
    out.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    out
}

fn tags(flag: Option<u32>) -> ColourTags {
    ColourTags {
        average: [63, 68, 37, 255],
        flag,
    }
}

/// The tags of a PAA in file order: (name as written in the game's docs, payload).
fn read_tags(d: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut o = 2;
    let mut out = Vec::new();
    while &d[o..o + 4] == b"GGAT" {
        let name: String = d[o + 4..o + 8]
            .iter()
            .rev()
            .map(|&b| char::from(b))
            .collect();
        let len =
            usize::try_from(u32::from_le_bytes(d[o + 8..o + 12].try_into().unwrap())).unwrap();
        out.push((name, d[o + 12..o + 12 + len].to_vec()));
        o += 12 + len;
    }
    out
}

fn offsets(d: &[u8]) -> Vec<u32> {
    let offs = read_tags(d)
        .into_iter()
        .find(|(n, _)| n == "OFFS")
        .unwrap()
        .1;
    offs.as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect()
}

#[test]
fn livonia_tags_come_before_the_offsets() {
    let out = with_colour_tags(&paa(), &tags(None)).unwrap();
    let t = read_tags(&out);
    let names: Vec<&str> = t.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["AVGC", "MAXC", "OFFS"]);
    // the average colour is stored blue, green, red, alpha (Livonia: 25 44 3f ff)
    assert_eq!(t[0].1, [37, 68, 63, 255]);
    assert_eq!(t[1].1, [255, 255, 255, 255]);
}

#[test]
fn a_mask_gets_the_alpha_flag() {
    let out = with_colour_tags(&paa(), &tags(Some(1))).unwrap();
    let t = read_tags(&out);
    let names: Vec<&str> = t.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["AVGC", "MAXC", "FLAG", "OFFS"]);
    assert_eq!(t[2].1, 1u32.to_le_bytes());
}

#[test]
fn mipmap_offsets_follow_the_inserted_tags() {
    let before = paa();
    let out = with_colour_tags(&before, &tags(Some(1))).unwrap();
    let shift = out.len() - before.len();
    assert_eq!(shift, 3 * 16);
    let (old, new) = (offsets(&before), offsets(&out));
    for (o, n) in old.iter().zip(&new) {
        let expected = if *o == 0 {
            0
        } else {
            *o + u32::try_from(shift).unwrap()
        };
        assert_eq!(*n, expected);
    }
    // the mipmaps themselves are unchanged and sit where the offsets say
    let first = usize::try_from(new[0]).unwrap();
    assert_eq!(&out[first..], &before[usize::try_from(old[0]).unwrap()..]);
}

#[test]
fn applying_twice_replaces_instead_of_adding() {
    let once = with_colour_tags(&paa(), &tags(Some(1))).unwrap();
    let twice = with_colour_tags(&once, &tags(Some(1))).unwrap();
    assert_eq!(once, twice);
}

#[test]
fn broken_files_are_errors() {
    let good = paa();
    assert!(matches!(
        with_colour_tags(&good[..1], &tags(None)),
        Err(PaaError::Truncated)
    ));
    // a tag whose length runs past the end of the file
    assert!(matches!(
        with_colour_tags(&good[..40], &tags(None)),
        Err(PaaError::Truncated)
    ));
    // no OFFS tag
    let mut no_offs = good.clone();
    no_offs[6..10].copy_from_slice(b"XXXX");
    assert!(matches!(
        with_colour_tags(&no_offs, &tags(None)),
        Err(PaaError::NoOffsets)
    ));
    // a mipmap offset that points past the end
    let mut bad = good.clone();
    bad[14..18].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        with_colour_tags(&bad, &tags(None)),
        Err(PaaError::BadOffset(_))
    ));
}

/// A PAA with one LZO mipmap of `lzo` bytes, one raw mipmap of `raw` bytes and the end marker.
fn paa_with_mips(lzo: &[u8], raw: &[u8]) -> Vec<u8> {
    let mip0 = 2 + 12 + 64 + 2;
    let mip1 = mip0 + 7 + lzo.len();
    let mut offs = [0u32; 16];
    offs[0] = u32::try_from(mip0).unwrap();
    offs[1] = u32::try_from(mip1).unwrap();
    let mut out = vec![0x01, 0xff];
    out.extend_from_slice(b"GGATSFFO");
    out.extend_from_slice(&64u32.to_le_bytes());
    for o in offs {
        out.extend_from_slice(&o.to_le_bytes());
    }
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&[0, 0x82, 0, 2]); // 512 x 512, the high bit marks LZO
    out.extend_from_slice(&u32::try_from(lzo.len()).unwrap().to_le_bytes()[..3]);
    out.extend_from_slice(lzo);
    out.extend_from_slice(&[0, 1, 0, 1]); // 256 x 256, raw
    out.extend_from_slice(&u32::try_from(raw.len()).unwrap().to_le_bytes()[..3]);
    out.extend_from_slice(raw);
    out.extend_from_slice(&[0; 6]);
    out
}

/// An LZO stream ends with the marker 11 00 00. armake2 appends a further 00, and the game then cannot
/// read the mipmap: it draws the tile in its average colour.
#[test]
fn the_stray_byte_after_an_lzo_stream_goes() {
    let raw = [0x11, 0, 0, 0, 7, 7, 0x11, 0, 0, 0];
    let fixed = without_stray_lzo_bytes(&paa_with_mips(&[9, 9, 0x11, 0, 0, 0], &raw)).unwrap();
    assert_eq!(
        fixed,
        paa_with_mips(&[9, 9, 0x11, 0, 0], &raw),
        "offsets follow"
    );
}

/// A PAA with one 256 x 256 mipmap of `body` that is marked as LZO, and the end marker.
fn paa_with_marked_mip(kind: u8, body: &[u8]) -> Vec<u8> {
    let mip0 = 2 + 12 + 64 + 2;
    let mut offs = [0u32; 16];
    offs[0] = u32::try_from(mip0).unwrap();
    let mut out = vec![kind, 0xff];
    out.extend_from_slice(b"GGATSFFO");
    out.extend_from_slice(&64u32.to_le_bytes());
    for o in offs {
        out.extend_from_slice(&o.to_le_bytes());
    }
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&[0, 0x81, 0, 1]); // 256 x 256, the high bit marks LZO
    out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes()[..3]);
    out.extend_from_slice(body);
    out.extend_from_slice(&[0; 6]);
    out
}

/// armake2 marks a mipmap as LZO even when it kept it uncompressed because LZO did not make it smaller;
/// the game cannot read that. A mipmap of exactly the uncompressed size is raw data.
#[test]
fn an_uncompressed_mipmap_loses_its_lzo_mark() {
    for (kind, size) in [(0x01_u8, 256 * 256 / 2), (0x05, 256 * 256)] {
        let body: Vec<u8> = (0..size)
            .map(|i| u8::try_from(i * 7 % 251).unwrap())
            .collect();
        let fixed = without_stray_lzo_bytes(&paa_with_marked_mip(kind, &body)).unwrap();
        let mip = 2 + 12 + 64 + 2;
        assert_eq!(
            &fixed[mip..mip + 4],
            &[0, 0x01, 0, 1],
            "type {kind}: no LZO mark"
        );
        assert_eq!(&fixed[mip + 7..mip + 7 + size], &body[..]);
    }
}

#[test]
fn a_real_lzo_stream_keeps_its_mark() {
    // smaller than the uncompressed size, and ending with the marker
    let lzo = [9, 9, 0x11, 0, 0];
    let paa = paa_with_marked_mip(0x01, &lzo);
    assert_eq!(without_stray_lzo_bytes(&paa).unwrap(), paa);
}

#[test]
fn intact_files_stay_as_they_are() {
    let good = paa_with_mips(&[9, 9, 0x11, 0, 0], &[0x11, 0, 0, 0]);
    assert_eq!(without_stray_lzo_bytes(&good).unwrap(), good);
    assert_eq!(without_stray_lzo_bytes(&paa()).unwrap(), paa());
}

#[test]
fn a_mipmap_past_the_end_is_an_error() {
    let good = paa_with_mips(&[9, 9, 0x11, 0, 0, 0], &[1, 2]);
    assert!(matches!(
        without_stray_lzo_bytes(&good[..90]),
        Err(PaaError::Truncated)
    ));
}

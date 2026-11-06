// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! PAA textures: adding the colour tags that the game needs on terrain images.
//!
//! A PAA starts with a 2-byte type, then tags (`GGAT`, a reversed 4-byte name, a `u32` length and the
//! payload), then the palette and the mipmaps. The `OFFS` tag holds sixteen absolute file offsets of the
//! mipmaps (0 = unused). armake2's `img2paa` writes only `OFFS`; Livonia's terrain images also carry `AVGC`
//! (average colour, stored blue, green, red, alpha) and `MAXC` (maximum colour), the masks also `FLAG`. Without
//! them the game shows the satellite image as a dark red ground.

use thiserror::Error;

/// Errors of [`with_colour_tags`].
#[derive(Debug, Error)]
pub enum PaaError {
    /// The file ends inside its header or a tag.
    #[error("the PAA file ends inside its header")]
    Truncated,
    /// The file has no `OFFS` tag of 16 offsets.
    #[error("the PAA file has no OFFS tag with 16 mipmap offsets")]
    NoOffsets,
    /// A mipmap offset points into the header or past the end of the file.
    #[error("mipmap offset {0} is outside the mipmap data")]
    BadOffset(u32),
}

/// The colour tags to set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColourTags {
    /// Average colour of the image, red, green, blue, alpha.
    pub average: [u8; 4],
    /// `FLAG` value (Livonia's masks have 1), `None` for no `FLAG` tag.
    pub flag: Option<u32>,
}

const MAGIC: &[u8; 4] = b"GGAT";
const OFFS: [u8; 4] = *b"SFFO";
const AVGC: [u8; 4] = *b"CGVA";
const MAXC: [u8; 4] = *b"CXAM";
const FLAG: [u8; 4] = *b"GALF";
const OFFSETS: usize = 16;

fn tag(out: &mut Vec<u8>, name: [u8; 4], payload: &[u8]) {
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&name);
    // payloads written here are 4 or 64 bytes
    out.extend_from_slice(&u32::try_from(payload.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(payload);
}

/// Returns `paa` with `AVGC`, `MAXC` (white) and, if asked, `FLAG` set in front of its other tags, and the
/// mipmap offsets moved by the size change. Tags of these names that are already there are replaced.
///
/// # Errors
///
/// [`PaaError`] for a file that ends inside its header, has no `OFFS` tag or has an offset outside its data.
pub fn with_colour_tags(paa: &[u8], tags: &ColourTags) -> Result<Vec<u8>, PaaError> {
    let kind = paa.get(..2).ok_or(PaaError::Truncated)?;
    let mut pos = 2usize;
    let mut kept: Vec<([u8; 4], &[u8])> = Vec::new();
    while paa.get(pos..pos + 4) == Some(MAGIC.as_slice()) {
        let head = paa.get(pos + 4..pos + 12).ok_or(PaaError::Truncated)?;
        let name: [u8; 4] = head
            .get(..4)
            .and_then(|n| n.try_into().ok())
            .ok_or(PaaError::Truncated)?;
        let len: [u8; 4] = head
            .get(4..)
            .and_then(|n| n.try_into().ok())
            .ok_or(PaaError::Truncated)?;
        let len = usize::try_from(u32::from_le_bytes(len)).map_err(|_| PaaError::Truncated)?;
        let start = pos.checked_add(12).ok_or(PaaError::Truncated)?;
        let end = start.checked_add(len).ok_or(PaaError::Truncated)?;
        let payload = paa.get(start..end).ok_or(PaaError::Truncated)?;
        if ![AVGC, MAXC, FLAG].contains(&name) {
            kept.push((name, payload));
        }
        pos = end;
    }
    let header_end = pos;
    let offsets = kept
        .iter()
        .find(|(n, p)| *n == OFFS && p.len() == OFFSETS * 4)
        .map(|(_, p)| *p)
        .ok_or(PaaError::NoOffsets)?;

    let mut head = Vec::with_capacity(header_end + 48);
    head.extend_from_slice(kind);
    let [r, g, b, a] = tags.average;
    tag(&mut head, AVGC, &[b, g, r, a]);
    tag(&mut head, MAXC, &[255; 4]);
    if let Some(flag) = tags.flag {
        tag(&mut head, FLAG, &flag.to_le_bytes());
    }
    let new_end = head.len() + kept.iter().map(|(_, p)| 12 + p.len()).sum::<usize>();
    let file_len = u64::try_from(paa.len()).map_err(|_| PaaError::Truncated)?;
    let old_end = u64::try_from(header_end).map_err(|_| PaaError::Truncated)?;
    let new_end = u64::try_from(new_end).map_err(|_| PaaError::Truncated)?;

    let mut moved = Vec::with_capacity(OFFSETS * 4);
    for chunk in offsets.chunks_exact(4) {
        let raw: [u8; 4] = chunk.try_into().map_err(|_| PaaError::NoOffsets)?;
        let offset = u32::from_le_bytes(raw);
        let new = if offset == 0 {
            0
        } else {
            let at = u64::from(offset);
            if at < old_end || at >= file_len {
                return Err(PaaError::BadOffset(offset));
            }
            let shifted = (at - old_end)
                .checked_add(new_end)
                .ok_or(PaaError::BadOffset(offset))?;
            u32::try_from(shifted).map_err(|_| PaaError::BadOffset(offset))?
        };
        moved.extend_from_slice(&new.to_le_bytes());
    }
    for (name, payload) in &kept {
        if *name == OFFS {
            tag(&mut head, *name, &moved);
        } else {
            tag(&mut head, *name, payload);
        }
    }
    let rest = paa.get(header_end..).ok_or(PaaError::Truncated)?;
    head.extend_from_slice(rest);
    Ok(head)
}

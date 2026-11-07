// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Little-endian reading and writing helpers shared by the file formats.
//!
//! Reads never allocate from a length before the bytes are actually present: [`read_bytes`]
//! grows its buffer with the data it receives.

use std::io::{self, Read, Write};

fn eof(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::UnexpectedEof,
        format!("input ended inside {what}"),
    )
}

pub(crate) fn read_array<const N: usize, R: Read>(r: &mut R) -> io::Result<[u8; N]> {
    let mut buf = [0u8; N];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

pub(crate) fn read_u32<R: Read>(r: &mut R) -> io::Result<u32> {
    Ok(u32::from_le_bytes(read_array(r)?))
}

pub(crate) fn read_i32<R: Read>(r: &mut R) -> io::Result<i32> {
    Ok(i32::from_le_bytes(read_array(r)?))
}

pub(crate) fn read_f32<R: Read>(r: &mut R) -> io::Result<f32> {
    Ok(f32::from_le_bytes(read_array(r)?))
}

/// Reads exactly `len` bytes, allocating only as data arrives.
pub(crate) fn read_bytes<R: Read>(r: &mut R, len: usize) -> io::Result<Vec<u8>> {
    let want = u64::try_from(len).map_err(|_| eof("a length that does not fit"))?;
    let mut buf = Vec::new();
    r.take(want).read_to_end(&mut buf)?;
    if buf.len() == len {
        Ok(buf)
    } else {
        Err(eof("a data block"))
    }
}

pub(crate) fn read_f32s<R: Read>(r: &mut R, n: usize) -> io::Result<Vec<f32>> {
    let bytes = n.checked_mul(4).ok_or_else(|| eof("a float array"))?;
    let raw = read_bytes(r, bytes)?;
    // `as_chunks` yields `[u8; 4]` arrays, so the conversion cannot fail.
    Ok(raw
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect())
}

pub(crate) fn read_u16s<R: Read>(r: &mut R, n: usize) -> io::Result<Vec<u16>> {
    let bytes = n.checked_mul(2).ok_or_else(|| eof("a u16 array"))?;
    let raw = read_bytes(r, bytes)?;
    // `as_chunks` yields `[u8; 2]` arrays, so the conversion cannot fail.
    Ok(raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect())
}

pub(crate) fn write_u32<W: Write>(w: &mut W, v: u32) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

pub(crate) fn write_i32<W: Write>(w: &mut W, v: i32) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

pub(crate) fn write_f32s<W: Write>(w: &mut W, v: &[f32]) -> io::Result<()> {
    for x in v {
        w.write_all(&x.to_le_bytes())?;
    }
    Ok(())
}

pub(crate) fn write_u16s<W: Write>(w: &mut W, v: &[u16]) -> io::Result<()> {
    for x in v {
        w.write_all(&x.to_le_bytes())?;
    }
    Ok(())
}

/// Writes a `u32` length followed by the ASCII bytes of `s`.
pub(crate) fn write_ascii32<W: Write>(w: &mut W, s: &str) -> io::Result<()> {
    let len = u32::try_from(s.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "string too long"))?;
    write_u32(w, len)?;
    w.write_all(s.as_bytes())
}

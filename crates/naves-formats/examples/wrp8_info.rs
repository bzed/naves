// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Prints the header and a summary of an 8WVR file:
//! `cargo run -p naves-formats --example wrp8_info -- <file.wrp>`.
//! Also dumps the raw header numbers, because the reader is strict and a file it rejects still has a header.

use std::io::Read;

use naves_formats::wrp8::EditableWorld;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: wrp8_info <file.wrp>")?;
    let mut raw = [0_u8; 24];
    std::fs::File::open(&path)?.read_exact(&mut raw)?;
    let words: Vec<u32> = raw
        .get(4..)
        .unwrap_or_default()
        .chunks_exact(4)
        .filter_map(|c| <[u8; 4]>::try_from(c).ok().map(u32::from_le_bytes))
        .collect();
    std::io::Write::write_fmt(
        &mut std::io::stdout(),
        format_args!("magic {:?}, raw header words {words:?}\n", raw.get(..4)),
    )?;
    let mut file = std::io::BufReader::new(std::fs::File::open(&path)?);
    match EditableWorld::read(&mut file) {
        Ok(w) => std::io::Write::write_fmt(
            &mut std::io::stdout(),
            format_args!(
                "land {}x{}, terrain {}x{}, cell {} m, {} materials, {} objects, first materials {:?}, elevation {:?}..{:?}\n",
                w.land_range_x,
                w.land_range_y,
                w.terrain_range_x,
                w.terrain_range_y,
                w.cell_size,
                w.materials.len(),
                w.objects.len(),
                w.materials.iter().take(3).collect::<Vec<_>>(),
                w.elevation.iter().copied().reduce(f32::min),
                w.elevation.iter().copied().reduce(f32::max),
            ),
        )?,
        Err(e) => std::io::Write::write_fmt(&mut std::io::stdout(), format_args!("reader: {e}\n"))?,
    }
    Ok(())
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Regenerates the fuzz seed corpus for `wrp8_read`: `cargo run -p naves-formats --example
//! wrp8_seeds -- fuzz/corpus/wrp8_read`.

use std::path::PathBuf;

use naves_formats::wrp8::{EditableWorld, WorldObject};

fn main() -> std::io::Result<()> {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".to_string()));
    std::fs::create_dir_all(&dir)?;
    let world = EditableWorld {
        land_range_x: 2,
        land_range_y: 2,
        terrain_range_x: 4,
        terrain_range_y: 4,
        cell_size: 25.0,
        elevation: vec![100.0; 16],
        material_index: vec![1; 4],
        materials: vec![String::new(), "naves\\x\\p_000-000.rvmat".to_string()],
        objects: vec![WorldObject {
            transform: [
                1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 10.0, 20.0, 30.0,
            ],
            id: 1,
            model: "a\\b.p3d".to_string(),
        }],
    };
    let mut buf = Vec::new();
    world.write(&mut buf)?;
    std::fs::write(dir.join("roundtrip"), &buf)?;
    let (half, _) = buf.split_at(buf.len() / 2);
    let (header, _) = buf.split_at(20);
    std::fs::write(dir.join("truncated_half"), half)?;
    std::fs::write(dir.join("truncated_header"), header)?;
    Ok(())
}

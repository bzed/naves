// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Writes the flat test world used by `tools/bridge-experiment.sh` to check that binarize accepts our
//! 8WVR files: `cargo run -p naves-formats --example flat_world -- <out.wrp> <world>`.
//!
//! Terrain 256x256 vertices, land 64x64 cells of 25 m, height 100 m everywhere, every cell uses the one
//! material `naves\worlds\<world>\data\layers\p_000-000.rvmat`.

use std::path::PathBuf;

use naves_formats::wrp8::EditableWorld;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().unwrap_or_else(|| "flat.wrp".to_string()));
    let world = args.next().unwrap_or_else(|| "bridgetest".to_string());
    let world_data = EditableWorld {
        land_range_x: 64,
        land_range_y: 64,
        terrain_range_x: 256,
        terrain_range_y: 256,
        cell_size: 25.0,
        elevation: vec![100.0; 256 * 256],
        material_index: vec![1; 64 * 64],
        materials: vec![
            String::new(),
            format!("naves\\worlds\\{world}\\data\\layers\\p_000-000.rvmat"),
        ],
        objects: Vec::new(),
    };
    let mut buf = Vec::new();
    world_data.write(&mut buf)?;
    std::fs::write(out, buf)
}

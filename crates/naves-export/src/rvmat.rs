// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tile materials (`p_XXX-YYY.rvmat`): satellite image, mask and the surface layers of one terrain tile.
//!
//! The structure follows what Livonia's tile materials look like, see `docs/formats/terrain-layers.md`.

use crate::layers::{SURFACES, TileGrid};

/// Number of surface layer slots of a tile material (pairs of `_nopx` and `_ca` stages).
const LAYER_SLOTS: usize = 6;

/// A `texGen` block that maps the world position into a tile image (satellite and mask).
fn world_tex_gen(index: u8, aside: f32, pos: (f32, f32)) -> String {
    format!(
        "class TexGen{index} {{\n    uvSource = \"worldPos\";\n    class uvTransform {{\n        \
         aside[] = {{{aside}, 0, 0}};\n        up[] = {{0, 0, {aside}}};\n        \
         dir[] = {{0, -{aside}, 0}};\n        pos[] = {{{}, {}, 0}};\n    }};\n}};\n",
        pos.0, pos.1
    )
}

/// A `texGen` block that tiles a surface texture `repeat` times per tile image.
fn tex_gen(index: u8, repeat: u8, dir: u8) -> String {
    format!(
        "class TexGen{index} {{\n    uvSource = \"tex\";\n    class uvTransform {{\n        \
         aside[] = {{{repeat}, 0, 0}};\n        up[] = {{0, {repeat}, 0}};\n        \
         dir[] = {{0, 0, {dir}}};\n        pos[] = {{0, 0, 0}};\n    }};\n}};\n"
    )
}

/// The two stages (`_nopx`, `_ca`) of layer `slot`; a slot without a surface has empty textures.
fn layer_stages(slot: usize) -> String {
    let (nopx, ca) = SURFACES.get(slot).map_or(("", ""), |s| (s.nopx, s.ca));
    let first = 3 + 2 * slot;
    format!(
        "class Stage{first} {{\n    texture = \"{nopx}\";\n    texGen = 1;\n}};\n\
         class Stage{} {{\n    texture = \"{ca}\";\n    texGen = 2;\n}};\n",
        first + 1
    )
}

/// The text of the tile material of tile (`tx`, `ty`); `prefix` is the world's in-game path, for example
/// `naves\worlds\berchtesgaden`.
#[must_use]
pub fn tile_material(grid: &TileGrid, tx: u32, ty: u32, prefix: &str) -> String {
    let aside = grid.aside();
    let pos = grid.uv_pos(tx, ty);
    let layers = format!("{prefix}\\data\\layers");
    let mut text = format!(
        "ambient[] = {{1, 1, 1, 1}};\ndiffuse[] = {{0.7, 0.7, 0.7, 1}};\n\
         forcedDiffuse[] = {{0, 0, 0, 0}};\nemmisive[] = {{0, 0, 0, 0}};\nspecular[] = {{0, 0, 0, 0}};\n\
         specularPower = 0;\nPixelShaderID = \"TerrainX\";\nVertexShaderID = \"Terrain\";\n\
         class Stage0 {{\n    texture = \"{layers}\\s_{tx:03}_{ty:03}_lco.paa\";\n    texGen = 3;\n}};\n\
         class Stage1 {{\n    texture = \"{layers}\\m_{tx:03}_{ty:03}_lca.paa\";\n    texGen = 4;\n}};\n\
         class Stage2 {{\n    texture = \"#(rgb,1,1,1)color(0.5,0.5,0.5,1,cdt)\";\n    texGen = 0;\n}};\n"
    );
    for slot in 0..LAYER_SLOTS {
        text.push_str(&layer_stages(slot));
    }
    text.push_str(&world_tex_gen(3, aside, pos));
    text.push_str(&world_tex_gen(4, aside, pos));
    text.push_str(&tex_gen(0, 1, 1));
    text.push_str(&tex_gen(1, 10, 0));
    text.push_str(&tex_gen(2, 10, 0));
    text
}

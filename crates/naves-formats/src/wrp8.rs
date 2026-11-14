// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reader and writer for the editable world format (`8WVR`), the input of binarize.
//!
//! Layout (little endian, after bis-file-formats `EditableWrp.cs`): the magic `8WVR`, the land grid
//! and terrain grid sizes, the cell size, the elevation and material-index grids (south row first),
//! the material list and the objects. The writer always appends the terminator object that Terrain
//! Builder writes; the reader requires it, stops there and drops it.

use std::io::{self, Read, Write};

use crate::binio;

/// Largest accepted grid side, for both the terrain and the land grid.
const MAX_GRID: u32 = 8192;
/// Largest accepted material count.
const MAX_MATERIALS: u32 = 65_536;
/// Longest accepted string (material or model name) in bytes.
const MAX_STRING: u32 = 1024;
/// Object id of the terminator object.
/// Id written into the terminator object (Terrain Builder writes an arbitrary one; readers ignore it).
const TERMINATOR_ID: i32 = i32::MAX;
/// Bit pattern of the NaNs in Terrain Builder's terminator object (observed in Bohemia's `Test_Terrain` sample).
const TERMINATOR_NAN_BITS: u32 = 0x7fc8_7f88;

/// An editable world: heightmap, material grid, materials and objects.
#[derive(Debug, Clone, PartialEq)]
pub struct EditableWorld {
    /// Material grid width in cells.
    pub land_range_x: u32,
    /// Material grid height in cells.
    pub land_range_y: u32,
    /// Heightmap width in vertices.
    pub terrain_range_x: u32,
    /// Heightmap height in vertices.
    pub terrain_range_y: u32,
    /// Metres per land cell.
    pub cell_size: f32,
    /// `terrain_range_x * terrain_range_y` heights, south row first.
    pub elevation: Vec<f32>,
    /// `land_range_x * land_range_y` material indices, south row first; 0 = no material.
    pub material_index: Vec<u16>,
    /// Material paths; entry 0 is always the empty string.
    pub materials: Vec<String>,
    /// Placed objects.
    pub objects: Vec<WorldObject>,
}

/// One placed object.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldObject {
    /// 3×3 rotation column by column (the images of the model's x, y and z axes),
    /// then position x, height, y.
    pub transform: [f32; 12],
    /// Object id.
    pub id: i32,
    /// Model path (ASCII).
    pub model: String,
}

/// Errors reading or validating an 8WVR file.
#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    /// The file does not start with `8WVR`.
    #[error("not an 8WVR file (bad magic)")]
    BadMagic,
    /// An I/O error, including input that ends too early.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// The content is out of range or contradicts itself.
    #[error("inconsistent 8WVR data: {0}")]
    Inconsistent(String),
}

fn inconsistent<T>(msg: impl Into<String>) -> Result<T, FormatError> {
    Err(FormatError::Inconsistent(msg.into()))
}

fn grid_len(name: &str, x: u32, y: u32) -> Result<usize, FormatError> {
    if x == 0 || y == 0 || x > MAX_GRID || y > MAX_GRID {
        return inconsistent(format!("{name} grid {x}x{y} outside 1..={MAX_GRID}"));
    }
    x.checked_mul(y)
        .and_then(|n| usize::try_from(n).ok())
        .map_or_else(|| inconsistent(format!("{name} grid too large")), Ok)
}

fn check_ascii(what: &str, s: &str) -> Result<(), FormatError> {
    if !s.is_ascii() {
        return inconsistent(format!("{what} is not ASCII"));
    }
    if u32::try_from(s.len()).map_or(true, |n| n > MAX_STRING) {
        return inconsistent(format!("{what} longer than {MAX_STRING} bytes"));
    }
    Ok(())
}

fn read_string<R: Read>(r: &mut R, what: &str) -> Result<String, FormatError> {
    let len = binio::read_u32(r)?;
    if len > MAX_STRING {
        return inconsistent(format!("{what} length {len} exceeds {MAX_STRING}"));
    }
    let len = usize::try_from(len).or_else(|_| inconsistent("length does not fit"))?;
    let bytes = binio::read_bytes(r, len)?;
    let s = String::from_utf8(bytes).or_else(|_| inconsistent(format!("{what} is not ASCII")))?;
    check_ascii(what, &s)?;
    Ok(s)
}

impl EditableWorld {
    /// Checks the invariants the file format needs.
    fn validate(&self) -> Result<(), FormatError> {
        let t = grid_len("terrain", self.terrain_range_x, self.terrain_range_y)?;
        let l = grid_len("land", self.land_range_x, self.land_range_y)?;
        if self.elevation.len() != t {
            return inconsistent(format!(
                "elevation has {} values, expected {t}",
                self.elevation.len()
            ));
        }
        if self.material_index.len() != l {
            return inconsistent(format!(
                "material_index has {} values, expected {l}",
                self.material_index.len()
            ));
        }
        if !(self.cell_size.is_finite() && self.cell_size > 0.0) {
            return inconsistent("cell_size must be finite and positive");
        }
        if self.materials.first().is_none_or(|m| !m.is_empty()) {
            return inconsistent("materials[0] must be the empty string");
        }
        let count = u32::try_from(self.materials.len()).unwrap_or(u32::MAX);
        if count > MAX_MATERIALS {
            return inconsistent("too many materials");
        }
        for m in &self.materials {
            check_ascii("material name", m)?;
        }
        if self.material_index.iter().any(|&i| u32::from(i) >= count) {
            return inconsistent("material_index refers to a missing material");
        }
        for o in &self.objects {
            check_ascii("model name", &o.model)?;
            if o.model.is_empty() {
                return inconsistent(
                    "an object needs a model (an empty one marks the end of the list)",
                );
            }
        }
        Ok(())
    }

    /// Writes the world, followed by the terminator object.
    ///
    /// # Errors
    ///
    /// `InvalidInput` if the world violates the format's invariants (grid sizes, lengths,
    /// non-ASCII names), otherwise any I/O error of `w`.
    pub fn write<W: Write>(&self, w: &mut W) -> io::Result<()> {
        self.validate().map_err(|e| match e {
            FormatError::Io(e) => e,
            other => io::Error::new(io::ErrorKind::InvalidInput, other.to_string()),
        })?;
        w.write_all(b"8WVR")?;
        for v in [
            self.land_range_x,
            self.land_range_y,
            self.terrain_range_x,
            self.terrain_range_y,
        ] {
            binio::write_u32(w, v)?;
        }
        w.write_all(&self.cell_size.to_le_bytes())?;
        binio::write_f32s(w, &self.elevation)?;
        binio::write_u16s(w, &self.material_index)?;
        let count = u32::try_from(self.materials.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "too many materials"))?;
        binio::write_u32(w, count)?;
        for m in &self.materials {
            if m.is_empty() {
                binio::write_u32(w, 0)?;
            } else {
                binio::write_ascii32(w, m)?;
                binio::write_u32(w, 0)?;
            }
        }
        for o in &self.objects {
            binio::write_f32s(w, &o.transform)?;
            binio::write_i32(w, o.id)?;
            binio::write_ascii32(w, &o.model)?;
        }
        binio::write_f32s(w, &[f32::from_bits(TERMINATOR_NAN_BITS); 12])?;
        binio::write_i32(w, TERMINATOR_ID)?;
        binio::write_u32(w, 0)
    }

    /// Reads a world; the input is untrusted.
    ///
    /// # Errors
    ///
    /// [`FormatError::BadMagic`] for other files, [`FormatError::Inconsistent`] for out-of-range
    /// or contradictory content, [`FormatError::Io`] for I/O errors and truncated input
    /// (including a missing terminator object).
    pub fn read<R: Read>(r: &mut R) -> Result<Self, FormatError> {
        if binio::read_array::<4, _>(r)? != *b"8WVR" {
            return Err(FormatError::BadMagic);
        }
        let land_range_x = binio::read_u32(r)?;
        let land_range_y = binio::read_u32(r)?;
        let terrain_range_x = binio::read_u32(r)?;
        let terrain_range_y = binio::read_u32(r)?;
        let land_len = grid_len("land", land_range_x, land_range_y)?;
        let terrain_len = grid_len("terrain", terrain_range_x, terrain_range_y)?;
        let cell_size = binio::read_f32(r)?;
        let elevation = binio::read_f32s(r, terrain_len)?;
        let material_index = binio::read_u16s(r, land_len)?;
        let count = binio::read_u32(r)?;
        if count == 0 || count > MAX_MATERIALS {
            return inconsistent(format!(
                "material count {count} outside 1..={MAX_MATERIALS}"
            ));
        }
        let mut materials = Vec::new();
        for _ in 0..count {
            let len = binio::read_u32(r)?;
            if len == 0 {
                materials.push(String::new());
                continue;
            }
            if len > MAX_STRING {
                return inconsistent(format!("material name length {len} exceeds {MAX_STRING}"));
            }
            let len = usize::try_from(len).or_else(|_| inconsistent("length does not fit"))?;
            let name = String::from_utf8(binio::read_bytes(r, len)?)
                .or_else(|_| inconsistent("material name is not ASCII"))?;
            check_ascii("material name", &name)?;
            if binio::read_u32(r)? != 0 {
                return inconsistent("material name is not followed by a 0 terminator");
            }
            materials.push(name);
        }
        let mut objects = Vec::new();
        loop {
            let transform: [f32; 12] = binio::read_f32s(r, 12)?
                .try_into()
                .or_else(|_| inconsistent("transform length"))?;
            let id = binio::read_i32(r)?;
            let model = read_string(r, "model name")?;
            if model.is_empty() && transform.iter().all(|v| v.is_nan()) {
                break;
            }
            objects.push(WorldObject {
                transform,
                id,
                model,
            });
        }
        let world = Self {
            land_range_x,
            land_range_y,
            terrain_range_x,
            terrain_range_y,
            cell_size,
            elevation,
            material_index,
            materials,
            objects,
        };
        world.validate()?;
        Ok(world)
    }
}

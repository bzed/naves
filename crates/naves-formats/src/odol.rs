// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reader for the parts of binarized DayZ models (ODOL, version 56) that Náves needs: the
//! bounding box, the points of the memory LOD and the named properties.
//!
//! The layout comes from the public `bis-file-formats` reader (MIT) for the general structure
//! and from our own observation of the 1.30 files for what version 56 changes. See
//! `docs/formats/road-objects.md`. Only what is needed is read:
//!
//! * the header and the `ModelInfo`, to reach the LOD address table;
//! * the memory LOD (resolution 10¹⁵) up to its vertices;
//! * the named properties of the small geometry-type LODs (resolution 10¹³ and up).
//!
//! The visual LODs, which hold the big LZO-compressed meshes, are never parsed. An array that the
//! format stores compressed (1024 bytes or more) is reported as [`OdolError::Compressed`] instead
//! of being decoded; none occurs in a memory LOD of the road pieces.

use std::collections::BTreeMap;

use thiserror::Error;

/// The only ODOL version this reader accepts.
pub const SUPPORTED_VERSION: u32 = 56;

/// Arrays of this many bytes or more are stored compressed.
const COMPRESSION_THRESHOLD: usize = 1024;

/// A LOD whose resolution lies in this range is the memory LOD (10¹⁵ is stored as 9.99…·10¹⁴).
const MEMORY_RESOLUTION: std::ops::Range<f32> = 5.0e14..2.0e15;

/// LODs from this resolution on are geometry-type LODs (small, no embedded materials).
const GEOMETRY_RESOLUTION: f32 = 9.0e12;

/// What went wrong while reading an ODOL file.
#[derive(Debug, Error)]
pub enum OdolError {
    /// The file does not start with `ODOL`.
    #[error("not an ODOL file: the first four bytes are not \"ODOL\"")]
    BadMagic,
    /// A version other than 56.
    #[error("ODOL version {0} is not supported (only version {SUPPORTED_VERSION})")]
    Version(u32),
    /// The input ended inside a field.
    #[error("the file ends inside {what} at byte {at}")]
    Truncated {
        /// The field being read.
        what: &'static str,
        /// Byte offset where more data was needed.
        at: usize,
    },
    /// A count that cannot be right for the bytes that remain.
    #[error("{what} at byte {at} claims {count} entries, more than the file can hold")]
    Count {
        /// The array being read.
        what: &'static str,
        /// Byte offset of the count.
        at: usize,
        /// The count as stored.
        count: i64,
    },
    /// A LOD address lies outside the file or its end is before its start.
    #[error("the address of LOD {lod} ({start}..{end}) lies outside the file ({len} bytes)")]
    Address {
        /// Index of the LOD.
        lod: usize,
        /// Stored start address.
        start: u32,
        /// Stored end address.
        end: u32,
        /// Size of the file.
        len: usize,
    },
    /// An array in the memory LOD is stored compressed.
    #[error("{what} holds a compressed array of {size} bytes, which this reader does not decode")]
    Compressed {
        /// The array being read.
        what: &'static str,
        /// The uncompressed size the format announces.
        size: usize,
    },
    /// A feature of the format that this reader leaves out.
    #[error("{0} is not supported")]
    Unsupported(&'static str),
    /// A named selection points at a vertex that does not exist.
    #[error("selection \"{selection}\" names vertex {index}, but the LOD has {count}")]
    VertexIndex {
        /// The selection.
        selection: String,
        /// The vertex index.
        index: u16,
        /// How many vertices the LOD has.
        count: usize,
    },
}

/// What is read from a model.
#[derive(Debug, Clone, PartialEq)]
pub struct OdolModel {
    /// The file's version.
    pub version: u32,
    /// Lower corner of the bounding box in model coordinates.
    pub bbox_min: [f32; 3],
    /// Upper corner of the bounding box in model coordinates.
    pub bbox_max: [f32; 3],
    /// Named points of the memory LOD, with the selection names lower-cased.
    /// Empty if the model has no memory LOD.
    pub memory: BTreeMap<String, Vec<[f32; 3]>>,
    /// Named properties of the geometry-type LODs; the first LOD that sets a key wins.
    pub properties: BTreeMap<String, String>,
}

/// Reads a binarized model.
///
/// # Errors
/// Returns an [`OdolError`] for a file that is not version 56, is cut short, has counts or
/// addresses that do not fit the file, or has a compressed array in its memory LOD.
pub fn read_odol(bytes: &[u8]) -> Result<OdolModel, OdolError> {
    let mut c = Cursor::new(bytes);
    if c.take(4, "the signature")? != b"ODOL" {
        return Err(OdolError::BadMagic);
    }
    let version = c.u32("the version")?;
    if version != SUPPORTED_VERSION {
        return Err(OdolError::Version(version));
    }
    let lod_count = c.count(4, "the resolutions")?;
    let mut resolutions = Vec::new();
    for _ in 0..lod_count {
        resolutions.push(c.f32("a resolution")?);
    }
    let (bbox_min, bbox_max) = read_model_info(&mut c)?;
    let table = read_lod_table(&mut c, lod_count, bytes.len())?;

    let mut model = OdolModel {
        version,
        bbox_min,
        bbox_max,
        memory: BTreeMap::new(),
        properties: BTreeMap::new(),
    };
    let mut memory_done = false;
    for (index, resolution) in resolutions.iter().enumerate() {
        let is_memory = MEMORY_RESOLUTION.contains(resolution) && !memory_done;
        if !is_memory && *resolution < GEOMETRY_RESOLUTION {
            continue;
        }
        let Some(&(start, end)) = table.get(index) else {
            continue;
        };
        let lod_bytes =
            bytes
                .get(usize_from(start)..usize_from(end))
                .ok_or(OdolError::Address {
                    lod: index,
                    start,
                    end,
                    len: bytes.len(),
                })?;
        match read_lod(lod_bytes, is_memory) {
            Ok(lod) => {
                for (key, value) in lod.properties {
                    model.properties.entry(key).or_insert(value);
                }
                if is_memory {
                    memory_done = true;
                    model.memory = lod.memory;
                }
            }
            Err(e) if is_memory => return Err(e),
            // A geometry-type LOD only contributes its properties. If it holds something this
            // reader does not know (a compressed array, an embedded material), the model is still
            // good for its memory points, so the LOD is left out instead of failing the file.
            Err(_) => {}
        }
    }
    Ok(model)
}

fn usize_from(v: u32) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// Reads the `ModelInfo` and returns the bounding box. Fields that are not used are skipped.
fn read_model_info(c: &mut Cursor<'_>) -> Result<([f32; 3], [f32; 3]), OdolError> {
    const W: &str = "the model info";
    // special, bounding sphere, geometry sphere, remarks, and/or hints, aiming centre,
    // two colours, view density
    c.skip(6 * 4 + 12 + 8 + 4, W)?;
    let bbox_min = c.vec3(W)?;
    let bbox_max = c.vec3(W)?;
    // visual box (2), bounding centre, geometry centre, centre of mass, inverse inertia (3)
    c.skip(12 * 5 + 36, W)?;
    // auto centre, lock auto centre, can occlude, can be occluded, disable cover
    c.skip(5, W)?;
    // ht_min; version 56 adds 4 bytes here; ht_max, af_max, mf_max, m_fact, t_body
    c.skip(4 + 4 + 20, W)?;
    // force not alpha, shadow buffer source, prefer shadow volume, shadow offset and its sibling
    c.skip(1 + 4 + 1 + 8, W)?;
    c.skip(1, W)?; // animated
    if !c.cstr(W)?.is_empty() {
        // skeleton: discrete flag, bones (name and parent), pivots name
        c.skip(1, W)?;
        let bones = c.count(2, "the skeleton bones")?;
        for _ in 0..bones {
            c.cstr(W)?;
            c.cstr(W)?;
        }
        c.cstr(W)?;
    }
    c.skip(1, W)?; // map type
    c.compressed(4, "the mass array")?;
    // mass, inverse mass, armour, inverse armour; 13 LOD index bytes; minimum shadow; can blend
    c.skip(16 + 13 + 4 + 1, W)?;
    c.cstr(W)?; // class
    c.cstr(W)?; // damage
    // frequent; unused; version 56 adds a byte
    c.skip(1 + 4 + 1, W)?;
    if c.u8(W)? != 0 {
        return Err(OdolError::Unsupported("an animated model"));
    }
    Ok((bbox_min, bbox_max))
}

/// Reads the LOD address table and returns `(start, end)` per LOD, each checked against the file.
fn read_lod_table(
    c: &mut Cursor<'_>,
    lods: usize,
    len: usize,
) -> Result<Vec<(u32, u32)>, OdolError> {
    const W: &str = "the LOD address table";
    let mut starts = Vec::new();
    for _ in 0..lods {
        starts.push(c.u32(W)?);
    }
    let mut ends = Vec::new();
    for _ in 0..lods {
        ends.push(c.u32(W)?);
    }
    let permanent = c.take(lods, W)?;
    // LODs that are loaded on demand carry a short info block each.
    let loadable: usize = permanent.iter().map(|p| usize::from(*p == 0)).sum();
    c.skip(
        loadable
            .checked_mul(25)
            .ok_or(OdolError::Unsupported("that many LODs"))?,
        W,
    )?;
    let mut table = Vec::new();
    for (lod, (&start, &end)) in starts.iter().zip(&ends).enumerate() {
        let inside = start <= end && usize_from(end) <= len;
        if !inside {
            return Err(OdolError::Address {
                lod,
                start,
                end,
                len,
            });
        }
        table.push((start, end));
    }
    Ok(table)
}

struct Lod {
    memory: BTreeMap<String, Vec<[f32; 3]>>,
    properties: BTreeMap<String, String>,
}

/// Reads one LOD up to its named properties, and for the memory LOD on to its vertices.
fn read_lod(bytes: &[u8], with_points: bool) -> Result<Lod, OdolError> {
    const W: &str = "a LOD";
    let mut c = Cursor::new(bytes);
    // proxies: model name, 3×4 matrix, sequence, selection, bone, section
    for _ in 0..c.count(5, "the proxies")? {
        c.cstr(W)?;
        c.skip(48 + 16, W)?;
    }
    for _ in 0..c.count(4, "the sub-skeletons")? {
        c.skip(4, W)?;
    }
    for _ in 0..c.count(4, "the skeleton map")? {
        for _ in 0..c.count(4, "the skeleton map entry")? {
            c.skip(4, W)?;
        }
    }
    // vertex count, face area, or/and hints, bounding box (min, max, centre), radius
    c.skip(4 + 4 + 8 + 36 + 4, W)?;
    for _ in 0..c.count(1, "the textures")? {
        c.cstr(W)?;
    }
    if c.count(1, "the materials")? != 0 {
        return Err(OdolError::Unsupported("an embedded material in a LOD"));
    }
    c.compressed(2, "the point-to-vertex map")?;
    c.compressed(2, "the vertex-to-point map")?;
    // faces: count, two unused fields, then per face a length and 16-bit indices
    let faces = c.count(1, "the faces")?;
    c.skip(4 + 2, W)?;
    for _ in 0..faces {
        let corners = usize::from(c.u8(W)?);
        c.skip(corners * 2, W)?;
    }
    for _ in 0..c.count(34, "the sections")? {
        c.skip(5 * 4 + 2 + 4, W)?;
        if c.i32(W)? == -1 {
            c.cstr(W)?;
        }
        for _ in 0..c.count(4, "the section areas")? {
            c.skip(4, W)?;
        }
    }
    let selections = read_selections(&mut c)?;
    let mut properties = BTreeMap::new();
    for _ in 0..c.count(2, "the named properties")? {
        let key = c.cstr(W)?;
        let value = c.cstr(W)?;
        properties.entry(key).or_insert(value);
    }
    if !with_points {
        return Ok(Lod {
            memory: BTreeMap::new(),
            properties,
        });
    }
    let points = read_points(&mut c)?;
    let mut memory = BTreeMap::new();
    for (name, vertices) in selections {
        let mut selected = Vec::new();
        for index in vertices {
            let point = points
                .get(usize::from(index))
                .ok_or_else(|| OdolError::VertexIndex {
                    selection: name.clone(),
                    index,
                    count: points.len(),
                })?;
            selected.push(*point);
        }
        memory.insert(name, selected);
    }
    Ok(Lod { memory, properties })
}

/// The named selections: a lower-cased name and the vertex indices it holds.
fn read_selections(c: &mut Cursor<'_>) -> Result<Vec<(String, Vec<u16>)>, OdolError> {
    const W: &str = "a LOD";
    let mut selections = Vec::new();
    for _ in 0..c.count(20, "the named selections")? {
        let name = c.cstr(W)?.to_ascii_lowercase();
        c.compressed(2, "a selection's faces")?;
        c.skip(4 + 1, W)?; // unused, is sectional
        c.compressed(4, "a selection's sections")?;
        let vertices = c.compressed_u16(2, "a selection's vertices")?;
        let weights = c.count_unchecked(W)?;
        if weights >= COMPRESSION_THRESHOLD {
            return Err(OdolError::Compressed {
                what: "a selection's weights",
                size: weights,
            });
        }
        c.skip(weights, W)?;
        selections.push((name, vertices));
    }
    Ok(selections)
}

/// From the key frames on to the end of the vertices; returns the vertex positions.
fn read_points(c: &mut Cursor<'_>) -> Result<Vec<[f32; 3]>, OdolError> {
    const W: &str = "a LOD";
    // key frames, three colours/flags, bone-ref flag, size of the rest
    for _ in 0..c.count(8, "the key frames")? {
        c.skip(4, W)?;
        for _ in 0..c.count(12, "a key frame")? {
            c.skip(12, W)?;
        }
    }
    c.skip(12 + 1 + 4, W)?;
    c.condensed(4, "the clip flags")?;
    c.uv_set()?;
    for _ in 1..c.u32(W)? {
        c.uv_set()?;
    }
    let raw = c.compressed(12, "the vertices")?;
    let points: Vec<[f32; 3]> = raw
        .as_chunks::<12>()
        .0
        .iter()
        .map(|p| {
            let f = |i: usize| {
                p.get(i * 4..i * 4 + 4)
                    .and_then(|b| <[u8; 4]>::try_from(b).ok())
                    .map_or(0.0, f32::from_le_bytes)
            };
            [f(0), f(1), f(2)]
        })
        .collect();
    Ok(points)
}

/// A bounds-checked reader over a byte slice.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], OdolError> {
        let end = self.pos.checked_add(n);
        let slice = end.and_then(|e| self.data.get(self.pos..e));
        match (slice, end) {
            (Some(s), Some(e)) => {
                self.pos = e;
                Ok(s)
            }
            _ => Err(OdolError::Truncated { what, at: self.pos }),
        }
    }

    fn skip(&mut self, n: usize, what: &'static str) -> Result<(), OdolError> {
        self.take(n, what).map(drop)
    }

    fn array<const N: usize>(&mut self, what: &'static str) -> Result<[u8; N], OdolError> {
        let at = self.pos;
        <[u8; N]>::try_from(self.take(N, what)?).map_err(|_| OdolError::Truncated { what, at })
    }

    fn u8(&mut self, what: &'static str) -> Result<u8, OdolError> {
        Ok(u8::from_le_bytes(self.array(what)?))
    }

    fn u32(&mut self, what: &'static str) -> Result<u32, OdolError> {
        Ok(u32::from_le_bytes(self.array(what)?))
    }

    fn i32(&mut self, what: &'static str) -> Result<i32, OdolError> {
        Ok(i32::from_le_bytes(self.array(what)?))
    }

    fn f32(&mut self, what: &'static str) -> Result<f32, OdolError> {
        Ok(f32::from_le_bytes(self.array(what)?))
    }

    fn vec3(&mut self, what: &'static str) -> Result<[f32; 3], OdolError> {
        Ok([self.f32(what)?, self.f32(what)?, self.f32(what)?])
    }

    /// A zero-terminated string; bytes are taken as Latin-1.
    fn cstr(&mut self, what: &'static str) -> Result<String, OdolError> {
        let rest = self.data.get(self.pos..).unwrap_or_default();
        let len = rest
            .iter()
            .position(|b| *b == 0)
            .ok_or(OdolError::Truncated {
                what,
                at: self.data.len(),
            })?;
        let text = rest.get(..len).unwrap_or_default();
        self.pos += len + 1;
        Ok(text.iter().map(|b| char::from(*b)).collect())
    }

    /// A signed 32-bit count of elements of at least `min_size` bytes each, checked against the
    /// bytes that remain so that no loop or allocation runs on a made-up number.
    fn count(&mut self, min_size: usize, what: &'static str) -> Result<usize, OdolError> {
        let at = self.pos;
        let raw = self.i32(what)?;
        let too_many = || OdolError::Count {
            what,
            at,
            count: i64::from(raw),
        };
        let n = usize::try_from(raw).map_err(|_| too_many())?;
        if n.checked_mul(min_size).is_none_or(|b| b > self.remaining()) {
            return Err(too_many());
        }
        Ok(n)
    }

    /// A non-negative 32-bit size that is checked later.
    fn count_unchecked(&mut self, what: &'static str) -> Result<usize, OdolError> {
        let at = self.pos;
        let raw = self.i32(what)?;
        usize::try_from(raw).map_err(|_| OdolError::Count {
            what,
            at,
            count: i64::from(raw),
        })
    }

    /// An array stored as `count` followed by `count × elem` bytes, or compressed when that is
    /// 1024 bytes or more. Returns the raw bytes.
    fn compressed(&mut self, elem: usize, what: &'static str) -> Result<&'a [u8], OdolError> {
        let at = self.pos;
        let n = self.count_unchecked(what)?;
        let size = n.checked_mul(elem).ok_or(OdolError::Count {
            what,
            at,
            count: i64::try_from(n).unwrap_or(i64::MAX),
        })?;
        if size >= COMPRESSION_THRESHOLD {
            return Err(OdolError::Compressed { what, size });
        }
        self.take(size, what)
    }

    fn compressed_u16(&mut self, elem: usize, what: &'static str) -> Result<Vec<u16>, OdolError> {
        Ok(self
            .compressed(elem, what)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes(*b))
            .collect())
    }

    /// An array that is either one repeated value or stored like [`Cursor::compressed`].
    fn condensed(&mut self, elem: usize, what: &'static str) -> Result<(), OdolError> {
        let at = self.pos;
        let n = self.count_unchecked(what)?;
        if self.u8(what)? != 0 {
            return self.skip(elem, what);
        }
        let size = n.checked_mul(elem).ok_or(OdolError::Count {
            what,
            at,
            count: i64::try_from(n).unwrap_or(i64::MAX),
        })?;
        if size >= COMPRESSION_THRESHOLD {
            return Err(OdolError::Compressed { what, size });
        }
        self.skip(size, what)
    }

    /// One UV set: the range, the vertex count and the data (4 bytes per vertex).
    fn uv_set(&mut self) -> Result<(), OdolError> {
        const W: &str = "a UV set";
        self.skip(16, W)?;
        let at = self.pos;
        let n = usize_from(self.u32(W)?);
        if self.u8(W)? != 0 {
            return self.skip(4, W);
        }
        let size = n.checked_mul(4).ok_or(OdolError::Count {
            what: W,
            at,
            count: i64::try_from(n).unwrap_or(i64::MAX),
        })?;
        if size >= COMPRESSION_THRESHOLD {
            return Err(OdolError::Compressed { what: W, size });
        }
        self.skip(size, W)
    }
}

/// A writer for small synthetic models, for tests of the crates that read them (feature
/// `test-util`). It writes the layout [`read_odol`] expects and no game data.
#[cfg(feature = "test-util")]
pub mod synthetic {
    /// Resolution of the memory LOD as the game files store it.
    const MEMORY_RESOLUTION: f32 = 999_999_986_991_104.0;

    fn put_u32(out: &mut Vec<u8>, v: usize) {
        out.extend_from_slice(&u32::try_from(v).unwrap_or(u32::MAX).to_le_bytes());
    }

    fn put_str(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(s.as_bytes());
        out.push(0);
    }

    /// A version 56 model with one memory LOD. Each entry is a named selection holding one point.
    #[must_use]
    pub fn model_bytes(points: &[(&str, [f32; 3])]) -> Vec<u8> {
        let mut lod = Vec::new();
        lod.extend_from_slice(&[0; 12]); // proxies, sub-skeletons, skeleton map
        lod.extend_from_slice(&[0; 56]);
        lod.extend_from_slice(&[0; 4 * 4]); // textures, materials, two maps
        lod.extend_from_slice(&[0; 10]); // faces
        lod.extend_from_slice(&[0; 4]); // sections
        put_u32(&mut lod, points.len());
        for (i, (name, _)) in points.iter().enumerate() {
            put_str(&mut lod, name);
            lod.extend_from_slice(&[0; 4 + 5 + 4]);
            put_u32(&mut lod, 1);
            lod.extend_from_slice(&u16::try_from(i).unwrap_or(u16::MAX).to_le_bytes());
            lod.extend_from_slice(&[0; 4]);
        }
        lod.extend_from_slice(&[0; 4]); // properties
        lod.extend_from_slice(&[0; 4 + 17]); // key frames and the flags after them
        lod.extend_from_slice(&[0; 4]);
        lod.extend_from_slice(&[1, 0, 0, 0, 0]); // clip flags
        lod.extend_from_slice(&[0; 16 + 4]); // UV range and vertex count
        lod.extend_from_slice(&[1, 0, 0, 0, 0]);
        put_u32(&mut lod, 1); // UV sets
        put_u32(&mut lod, points.len());
        for (_, p) in points {
            for v in p {
                lod.extend_from_slice(&v.to_le_bytes());
            }
        }

        let mut file = Vec::new();
        file.extend_from_slice(b"ODOL");
        put_u32(&mut file, 56);
        put_u32(&mut file, 1);
        file.extend_from_slice(&MEMORY_RESOLUTION.to_le_bytes());
        file.extend_from_slice(&[0; 48 + 24 + 96 + 5 + 28 + 14 + 1 + 1 + 1 + 4 + 34]);
        put_str(&mut file, "road");
        put_str(&mut file, "");
        file.extend_from_slice(&[0; 6 + 1]);
        let start = file.len() + 9;
        put_u32(&mut file, start);
        put_u32(&mut file, start + lod.len());
        file.push(1);
        file.extend_from_slice(&lod);
        file
    }
}

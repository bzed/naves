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
//! * the Geometry LOD (10¹³): its vertices and its faces, as triangles;
//! * the Roadway LOD (3·10¹⁵): its vertices;
//! * the named properties of the small geometry-type LODs (resolution 10¹³ and up).
//!
//! The visual LODs, which hold the big meshes, are never parsed. An array that the format stores
//! compressed (1024 bytes or more) is LZO1X-decompressed (the `lzokay-native` crate, its decompressor
//! only) to the size it announces, at most 16 MiB; the stream ends itself, so no length is stored.

use std::borrow::Cow;
use std::collections::BTreeMap;

use thiserror::Error;

/// The only ODOL version this reader accepts.
pub const SUPPORTED_VERSION: u32 = 56;

/// Arrays of this many bytes or more are stored compressed.
const COMPRESSION_THRESHOLD: usize = 1024;

/// The most an array may announce when it is stored compressed, in bytes, and so the most that is allocated
/// for it.
const MAX_ARRAY_BYTES: usize = 16 << 20;

/// A LOD whose resolution lies in this range is the memory LOD (10¹⁵ is stored as 9.99…·10¹⁴).
const MEMORY_RESOLUTION: std::ops::Range<f32> = 5.0e14..2.0e15;

/// LODs from this resolution on are geometry-type LODs (small, no embedded materials).
const GEOMETRY_RESOLUTION: f32 = 9.0e12;

/// The Roadway LOD (3·10¹⁵ is stored as 2.99…·10¹⁵).
const ROADWAY_RESOLUTION: std::ops::Range<f32> = 2.5e15..3.5e15;

/// The Geometry LOD itself (10¹³), the one whose vertices are kept.
const GEOMETRY_ONLY: std::ops::Range<f32> = 9.5e12..1.05e13;

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
    /// An array announces more than 16 MiB.
    #[error("{what} holds an array of {size} bytes, more than this reader decodes")]
    Compressed {
        /// The array being read.
        what: &'static str,
        /// The uncompressed size the format announces.
        size: usize,
    },
    /// A compressed array that cannot be decompressed to the size it announces.
    #[error("{what} at byte {at} does not decompress to the size it announces")]
    Decompress {
        /// The array being read.
        what: &'static str,
        /// Byte offset where the compressed data starts.
        at: usize,
    },
    /// A feature of the format that this reader leaves out.
    #[error("{0} is not supported")]
    Unsupported(&'static str),
    /// A face of the Geometry LOD that has other than three or four corners.
    #[error("a face of the Geometry LOD at byte {at} has {count} corners (3 or 4 are possible)")]
    FaceCorners {
        /// Byte offset of the face's corner count within the LOD.
        at: usize,
        /// The corner count as stored.
        count: u8,
    },
    /// A face of the Geometry LOD names a vertex that does not exist.
    #[error("a face of the Geometry LOD names vertex {index}, but the LOD has {count}")]
    FaceIndex {
        /// The vertex index.
        index: u16,
        /// How many vertices the LOD has.
        count: usize,
    },
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
    /// Vertices of the Geometry LOD (resolution 10¹³ exactly, not the other geometry-type LODs such as the
    /// roadway or the land contact), in model coordinates; `None` if there is none or it cannot be read.
    pub geometry: Option<Vec<[f32; 3]>>,
    /// The faces of the Geometry LOD as triangles: indices into [`OdolModel::geometry`]. A quad `a b c d` is
    /// the triangles `a b c` and `a c d`. Empty if there is no Geometry LOD or its vertices cannot be read.
    pub geometry_faces: Vec<[u32; 3]>,
    /// Vertices of the Roadway LOD (resolution 3·10¹⁵) in model coordinates; empty if the model has none or
    /// it cannot be read (the wooden bridges' roadway LODs hold embedded materials).
    pub roadway: Vec<[f32; 3]>,
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
        geometry: None,
        geometry_faces: Vec::new(),
        roadway: Vec::new(),
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
        let is_geometry = GEOMETRY_ONLY.contains(resolution);
        match read_lod(lod_bytes, is_memory, is_geometry) {
            Ok(lod) => {
                for (key, value) in lod.properties {
                    model.properties.entry(key).or_insert(value);
                }
                if is_memory {
                    memory_done = true;
                    model.memory = lod.memory;
                } else if is_geometry {
                    model.geometry = Some(lod.points);
                    model.geometry_faces = lod.faces;
                } else if ROADWAY_RESOLUTION.contains(resolution) && model.roadway.is_empty() {
                    model.roadway = lod.points;
                }
            }
            Err(e) if is_memory || (is_geometry && e.faces) => return Err(e.error),
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
    faces: Vec<[u32; 3]>,
    points: Vec<[f32; 3]>,
    memory: BTreeMap<String, Vec<[f32; 3]>>,
    properties: BTreeMap<String, String>,
}

/// Why a LOD could not be read, and whether it was in its faces (which the Geometry LOD must get right).
struct LodFailure {
    error: OdolError,
    faces: bool,
}

impl From<OdolError> for LodFailure {
    fn from(error: OdolError) -> Self {
        Self {
            error,
            faces: false,
        }
    }
}

/// Reads one LOD up to its named properties, and for the memory LOD on to its vertices. With `keep_faces`
/// (the Geometry LOD) the faces are kept as triangles and checked against the vertices.
fn read_lod(bytes: &[u8], is_memory: bool, keep_faces: bool) -> Result<Lod, LodFailure> {
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
        return Err(OdolError::Unsupported("an embedded material in a LOD").into());
    }
    c.compressed(2, "the point-to-vertex map")?;
    c.compressed(2, "the vertex-to-point map")?;
    let faces =
        read_faces(&mut c, keep_faces).map_err(|error| LodFailure { error, faces: true })?;
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
    let points = match read_points(&mut c) {
        Ok(points) => points,
        Err(e) if is_memory => return Err(e.into()),
        // a geometry-type LOD whose vertices cannot be read still gives its properties
        Err(_) => {
            return Ok(Lod {
                faces: Vec::new(),
                points: Vec::new(),
                memory: BTreeMap::new(),
                properties,
            });
        }
    };
    // a LOD without readable vertices (some models have a Geometry LOD with faces and no vertices) has no
    // faces to map
    let faces = if points.is_empty() { Vec::new() } else { faces };
    if let Some(&index) = faces
        .iter()
        .flatten()
        .find(|i| usize_from(**i) >= points.len())
    {
        return Err(LodFailure {
            error: OdolError::FaceIndex {
                index: u16::try_from(index).unwrap_or(u16::MAX),
                count: points.len(),
            },
            faces: true,
        });
    }
    if !is_memory {
        return Ok(Lod {
            faces,
            points,
            memory: BTreeMap::new(),
            properties,
        });
    }
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
    Ok(Lod {
        faces,
        points,
        memory,
        properties,
    })
}

/// The faces: count, two unused fields, then per face a corner count (a byte) and that many 16-bit vertex
/// indices. With `keep` they are returned as triangles (a quad `a b c d` as `a b c` and `a c d`; any other
/// corner count is an error), else they are skipped. The indices are checked later, against the vertices.
fn read_faces(c: &mut Cursor<'_>, keep: bool) -> Result<Vec<[u32; 3]>, OdolError> {
    const W: &str = "the faces";
    let faces = c.count(1, W)?;
    c.skip(4 + 2, W)?;
    let mut triangles = Vec::new();
    for _ in 0..faces {
        let at = c.pos;
        let corners = c.u8(W)?;
        let raw = c.take(usize::from(corners) * 2, W)?;
        if !keep {
            continue;
        }
        let index = |i: usize| {
            raw.get(i * 2..i * 2 + 2)
                .and_then(|b| <[u8; 2]>::try_from(b).ok())
                .map_or(0, |b| u32::from(u16::from_le_bytes(b)))
        };
        match corners {
            3 => triangles.push([index(0), index(1), index(2)]),
            4 => {
                triangles.push([index(0), index(1), index(2)]);
                triangles.push([index(0), index(2), index(3)]);
            }
            count => return Err(OdolError::FaceCorners { at, count }),
        }
    }
    Ok(triangles)
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
        c.compressed(1, "a selection's weights")?;
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
    fn compressed(&mut self, elem: usize, what: &'static str) -> Result<Cow<'a, [u8]>, OdolError> {
        let at = self.pos;
        let n = self.count_unchecked(what)?;
        self.body(n, elem, what, at)
    }

    /// The `n` elements of `elem` bytes that follow a count at `at`: as they are below
    /// [`COMPRESSION_THRESHOLD`] bytes, else LZO1X-compressed (a stream that ends itself, so its length is
    /// not stored). The size is checked against [`MAX_ARRAY_BYTES`] before anything is allocated, and the
    /// output may not grow beyond it.
    fn body(
        &mut self,
        n: usize,
        elem: usize,
        what: &'static str,
        at: usize,
    ) -> Result<Cow<'a, [u8]>, OdolError> {
        let size = n.checked_mul(elem).ok_or(OdolError::Count {
            what,
            at,
            count: i64::try_from(n).unwrap_or(i64::MAX),
        })?;
        if size < COMPRESSION_THRESHOLD {
            return self.take(size, what).map(Cow::Borrowed);
        }
        if size > MAX_ARRAY_BYTES {
            return Err(OdolError::Compressed { what, size });
        }
        let rest = self.data.get(self.pos..).unwrap_or_default();
        let mut stream = std::io::Cursor::new(rest);
        let out = lzokay_native::decompress_bounded(&mut stream, size)
            .map_err(|_| OdolError::Decompress { what, at: self.pos })?;
        if out.len() != size {
            return Err(OdolError::Decompress { what, at: self.pos });
        }
        let used = usize::try_from(stream.position()).unwrap_or(usize::MAX);
        self.skip(used, what)?;
        Ok(Cow::Owned(out))
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
        self.body(n, elem, what, at).map(drop)
    }

    /// One UV set: the range, the vertex count and the data (4 bytes per vertex).
    fn uv_set(&mut self) -> Result<(), OdolError> {
        const W: &str = "a UV set";
        self.skip(16, W)?;
        self.condensed(4, W)
    }
}

/// A writer for small synthetic models, for tests of the crates that read them (feature
/// `test-util`). It writes the layout [`read_odol`] expects and no game data.
#[cfg(feature = "test-util")]
pub mod synthetic {
    /// Resolution of the memory LOD as the game files store it.
    const MEMORY_RESOLUTION: f32 = 999_999_986_991_104.0;
    /// Resolution of the roadway LOD as the game files store it.
    const ROADWAY_RESOLUTION: f32 = 2_999_999_983_755_264.0;
    /// Resolution of the geometry LOD as the game files store it.
    const GEOMETRY_RESOLUTION: f32 = 9_999_999_827_968.0;

    fn put_u32(out: &mut Vec<u8>, v: usize) {
        out.extend_from_slice(&u32::try_from(v).unwrap_or(u32::MAX).to_le_bytes());
    }

    fn put_str(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(s.as_bytes());
        out.push(0);
    }

    /// `data` (4 bytes or more) as an LZO1X stream that holds it as one run of literals and ends itself: the
    /// simplest stream a decompressor must accept, for tests of arrays that the format stores compressed.
    #[must_use]
    pub fn lzo_literals(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let len = data.len();
        if len <= 238 {
            out.push(u8::try_from(len + 17).unwrap_or(u8::MAX));
        } else {
            let extra = len - 18;
            let zeros = (extra - 1) / 255;
            out.push(0);
            out.resize(1 + zeros, 0);
            out.push(u8::try_from(extra - 255 * zeros).unwrap_or(u8::MAX));
        }
        out.extend_from_slice(data);
        out.extend_from_slice(&[17, 0, 0]);
        out
    }

    /// An array of `elem`-byte elements as the format stores it after its count: as it is, or from 1024 bytes
    /// on as an LZO stream.
    fn array_body(out: &mut Vec<u8>, bytes: &[u8]) {
        if bytes.len() >= 1024 {
            out.extend_from_slice(&lzo_literals(bytes));
        } else {
            out.extend_from_slice(bytes);
        }
    }

    /// One LOD: named selections of one point each (`names` index into `vertices`) and the vertices.
    fn lod(names: &[&str], vertices: &[[f32; 3]], faces: &[Vec<u16>]) -> Vec<u8> {
        let mut lod = Vec::new();
        lod.extend_from_slice(&[0; 12]); // proxies, sub-skeletons, skeleton map
        lod.extend_from_slice(&[0; 56]);
        lod.extend_from_slice(&[0; 4 * 4]); // textures, materials, two maps
        put_u32(&mut lod, faces.len());
        lod.extend_from_slice(&[0; 6]); // two unused fields
        for face in faces {
            lod.push(u8::try_from(face.len()).unwrap_or(u8::MAX));
            for index in face {
                lod.extend_from_slice(&index.to_le_bytes());
            }
        }
        lod.extend_from_slice(&[0; 4]); // sections
        put_u32(&mut lod, names.len());
        for (i, name) in names.iter().enumerate() {
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
        put_u32(&mut lod, vertices.len());
        let raw: Vec<u8> = vertices
            .iter()
            .flat_map(|p| p.iter().flat_map(|v| v.to_le_bytes()))
            .collect();
        array_body(&mut lod, &raw);
        lod
    }

    /// A version 56 model with one memory LOD. Each entry is a named selection holding one point.
    #[must_use]
    pub fn model_bytes(points: &[(&str, [f32; 3])]) -> Vec<u8> {
        model_bytes_with_geometry(points, &[])
    }

    /// [`model_bytes`] with a geometry LOD that has the vertices `geometry` (none: no geometry LOD).
    #[must_use]
    pub fn model_bytes_with_geometry(
        points: &[(&str, [f32; 3])],
        geometry: &[[f32; 3]],
    ) -> Vec<u8> {
        model_bytes_with_lods(points, geometry, &[], &[])
    }

    /// [`model_bytes_with_geometry`] with the faces of the geometry LOD (one entry per face: 3 or 4 vertex
    /// indices) and a roadway LOD with the vertices `roadway` (none: no roadway LOD).
    #[must_use]
    pub fn model_bytes_with_lods(
        points: &[(&str, [f32; 3])],
        geometry: &[[f32; 3]],
        faces: &[Vec<u16>],
        roadway: &[[f32; 3]],
    ) -> Vec<u8> {
        let names: Vec<&str> = points.iter().map(|(n, _)| *n).collect();
        let vertices: Vec<[f32; 3]> = points.iter().map(|(_, p)| *p).collect();
        let mut lods: Vec<(f32, Vec<u8>)> = Vec::new();
        if !geometry.is_empty() {
            lods.push((GEOMETRY_RESOLUTION, lod(&[], geometry, faces)));
        }
        lods.push((MEMORY_RESOLUTION, lod(&names, &vertices, &[])));
        if !roadway.is_empty() {
            lods.insert(0, (ROADWAY_RESOLUTION, lod(&[], roadway, &[])));
        }

        let mut file = Vec::new();
        file.extend_from_slice(b"ODOL");
        put_u32(&mut file, 56);
        put_u32(&mut file, lods.len());
        for (resolution, _) in &lods {
            file.extend_from_slice(&resolution.to_le_bytes());
        }
        file.extend_from_slice(&[0; 48 + 24 + 96 + 5 + 28 + 14 + 1 + 1 + 1 + 4 + 34]);
        put_str(&mut file, "road");
        put_str(&mut file, "");
        file.extend_from_slice(&[0; 6 + 1]);
        // the table: starts, ends, one "permanent" byte per LOD, then the LODs
        let table = file.len() + 4 * 2 * lods.len() + lods.len();
        let mut at = table;
        let mut ends = Vec::new();
        for (_, lod) in &lods {
            put_u32(&mut file, at);
            at += lod.len();
            ends.push(at);
        }
        for end in ends {
            put_u32(&mut file, end);
        }
        file.extend_from_slice(&vec![1; lods.len()]);
        for (_, lod) in &lods {
            file.extend_from_slice(lod);
        }
        file
    }
}

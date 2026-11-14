// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Land polygons from OpenStreetMap `.osm.pbf` extracts: water, rock and built-up areas.
//!
//! Forest and grass tags are deliberately not read: ESA `WorldCover` draws those edges better (10 m, every
//! pixel), and `landuse=forest` polygons are large and coarse.
//!
//! A PBF file stores nodes, then ways, then relations, and a country-sized extract must not be held in
//! memory. [`read_land`] therefore reads a file in three passes: the relations that describe areas, then
//! the ways that are areas or relation members, then only the nodes those ways use.
//!
//! The `osmpbf` crate can panic on a damaged file (unchecked delta sums, an unknown member type). The
//! reader runs under `catch_unwind` and turns such a panic into [`OsmError::Malformed`]. The unit test of the
//! guard, the damaged-file tests and the fuzz target `osm_read` (which silences libFuzzer's aborting panic hook, so
//! that the guard can do its work) check that nothing else escapes.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use osmpbf::{Element, ElementReader, RelMemberType};

use crate::landcover::GeoWindow;
use crate::surface::SurfaceClass;

/// Errors reading OSM data.
#[derive(Debug, thiserror::Error)]
pub enum OsmError {
    /// The PBF decoder rejected the data.
    #[error("cannot read OSM data: {0}")]
    Pbf(#[from] osmpbf::Error),
    /// The data made the decoder fail in a way it does not report itself (a damaged file).
    #[error("damaged OSM data: {0}")]
    Malformed(String),
    /// An error in one of the files given to [`read_land`].
    #[error("{}: {source}", path.display())]
    File {
        /// The file concerned.
        path: PathBuf,
        /// What went wrong.
        source: Box<OsmError>,
    },
}

/// A (latitude, longitude) pair in degrees.
type Point = (f64, f64);

/// An area of one ground class.
#[derive(Debug, Clone, PartialEq)]
pub struct LandPolygon {
    /// The ground class the area paints.
    pub class: SurfaceClass,
    /// Higher wins where areas overlap.
    pub priority: u8,
    /// The outer ring, closed (the first point is repeated at the end), as (lat, lon).
    pub outer: Vec<(f64, f64)>,
    /// Holes in the area, closed rings.
    pub holes: Vec<Vec<(f64, f64)>>,
}

/// The areas of a window and the number of rings that could not be built.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OsmLand {
    /// The areas, ways first and relations after them, each in id order.
    pub polygons: Vec<LandPolygon>,
    /// Rings skipped because they are unclosed, miss nodes or ways, or have no outer ring (the usual
    /// result of an extract that cuts a polygon at its border).
    pub skipped_rings: usize,
}

/// A way as a member of an area: its node ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawWay {
    /// Node ids in order.
    pub nodes: Vec<i64>,
    /// The relation member role was `inner`.
    pub inner: bool,
}

/// An outer ring and the holes in it, as (lat, lon) points.
pub type RingsWithHoles = (Vec<(f64, f64)>, Vec<Vec<(f64, f64)>>);

/// The polygons made from a set of ways.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Rings {
    /// Outer ring and holes of every polygon.
    pub polygons: Vec<RingsWithHoles>,
    /// Rings that could not be built.
    pub skipped: usize,
}

/// The class and priority of an area with these tags.
fn classify<'a>(tags: impl Iterator<Item = (&'a str, &'a str)>) -> Option<(SurfaceClass, u8)> {
    let mut best: Option<(SurfaceClass, u8)> = None;
    for (key, value) in tags {
        let found = match (key, value) {
            ("natural", "water") | ("landuse", "reservoir") => Some((SurfaceClass::Soil, 3)),
            ("natural", "glacier" | "bare_rock" | "scree" | "shingle") => {
                Some((SurfaceClass::Rock, 2))
            }
            ("landuse", "residential" | "industrial" | "commercial" | "farmyard" | "military") => {
                Some((SurfaceClass::Soil, 1))
            }
            _ => None,
        };
        if let Some((class, priority)) = found
            && best.is_none_or(|(_, p)| priority > p)
        {
            best = Some((class, priority));
        }
    }
    best
}

/// Even-odd test of `point` against a closed ring.
fn ring_contains(ring: &[Point], point: Point) -> bool {
    let mut inside = false;
    for pair in ring.windows(2) {
        let [(lat_a, lon_a), (lat_b, lon_b)] = pair else {
            continue;
        };
        if (*lat_a > point.0) != (*lat_b > point.0) {
            let at = lon_a + (point.0 - lat_a) / (lat_b - lat_a) * (lon_b - lon_a);
            if point.1 < at {
                inside = !inside;
            }
        }
    }
    inside
}

/// Joins ways end to end into closed rings; counts what cannot be closed or resolved in `skipped`.
fn join_rings(
    ways: &[&[i64]],
    nodes: &BTreeMap<i64, (f64, f64)>,
    skipped: &mut usize,
) -> Vec<Vec<Point>> {
    // node id -> the ways that start or end there, in input order
    let mut at_end: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for (index, way) in ways.iter().enumerate() {
        if let (Some(first), Some(last)) = (way.first(), way.last()) {
            at_end.entry(*first).or_default().push(index);
            if first != last {
                at_end.entry(*last).or_default().push(index);
            }
        }
    }
    let mut used = vec![false; ways.len()];
    let mut rings = Vec::new();
    for (start, way) in ways.iter().enumerate() {
        if used.get(start).copied().unwrap_or(true) {
            continue;
        }
        if let Some(flag) = used.get_mut(start) {
            *flag = true;
        }
        let mut chain: Vec<i64> = way.to_vec();
        let closed = loop {
            let (Some(first), Some(last)) = (chain.first().copied(), chain.last().copied()) else {
                break false;
            };
            if chain.len() >= 2 && first == last {
                break true;
            }
            let next = at_end.get(&last).and_then(|candidates| {
                candidates
                    .iter()
                    .copied()
                    .find(|&c| !used.get(c).copied().unwrap_or(true))
            });
            let Some(next) = next else { break false };
            if let Some(flag) = used.get_mut(next) {
                *flag = true;
            }
            let Some(other) = ways.get(next) else {
                break false;
            };
            if other.first() == Some(&last) {
                chain.extend(other.iter().skip(1));
            } else {
                chain.extend(other.iter().rev().skip(1));
            }
        };
        // a ring needs three corners and the closing point
        if !closed || chain.len() < 4 {
            *skipped += 1;
            continue;
        }
        let points: Option<Vec<Point>> = chain.iter().map(|id| nodes.get(id).copied()).collect();
        match points {
            Some(points) => rings.push(points),
            None => *skipped += 1,
        }
    }
    rings
}

/// Joins the member ways end to end into rings and pairs holes with the outer ring that contains them.
///
/// A ring that cannot be closed (an extract cut it), misses a node, has fewer than three corners, or is a
/// hole inside no outer ring is skipped and counted in [`Rings::skipped`].
#[must_use]
pub fn assemble(ways: &[RawWay], nodes: &BTreeMap<i64, (f64, f64)>) -> Rings {
    let mut skipped = 0;
    let outer: Vec<&[i64]> = ways
        .iter()
        .filter(|w| !w.inner)
        .map(|w| w.nodes.as_slice())
        .collect();
    let inner: Vec<&[i64]> = ways
        .iter()
        .filter(|w| w.inner)
        .map(|w| w.nodes.as_slice())
        .collect();
    let mut polygons: Vec<(Vec<Point>, Vec<Vec<Point>>)> = join_rings(&outer, nodes, &mut skipped)
        .into_iter()
        .map(|ring| (ring, Vec::new()))
        .collect();
    for hole in join_rings(&inner, nodes, &mut skipped) {
        let owner = hole.first().and_then(|&corner| {
            polygons
                .iter_mut()
                .find(|(outer, _)| ring_contains(outer, corner))
        });
        match owner {
            Some((_, holes)) => holes.push(hole),
            None => skipped += 1,
        }
    }
    Rings { polygons, skipped }
}

/// A relation that describes an area.
struct Relation {
    id: i64,
    class: SurfaceClass,
    priority: u8,
    /// (way id, role is `inner`)
    members: Vec<(i64, bool)>,
}

/// A way that is an area or a member of one.
struct WayRecord {
    id: i64,
    area: Option<(SurfaceClass, u8)>,
    nodes: Vec<i64>,
}

fn relations_of<R: Read + Send>(reader: ElementReader<R>) -> Result<Vec<Relation>, OsmError> {
    let found = reader.par_map_reduce(
        |element| match element {
            Element::Relation(relation) => {
                let is_area = relation
                    .tags()
                    .any(|(key, value)| key == "type" && value == "multipolygon");
                match classify(relation.tags()) {
                    Some((class, priority)) if is_area => vec![Relation {
                        id: relation.id(),
                        class,
                        priority,
                        members: relation
                            .members()
                            .filter(|m| m.member_type == RelMemberType::Way)
                            .map(|m| (m.member_id, m.role().is_ok_and(|role| role == "inner")))
                            .collect(),
                    }],
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        },
        Vec::new,
        |mut a, mut b| {
            a.append(&mut b);
            a
        },
    )?;
    Ok(found)
}

fn ways_of<R: Read + Send>(
    reader: ElementReader<R>,
    members: &BTreeSet<i64>,
) -> Result<Vec<WayRecord>, OsmError> {
    let found = reader.par_map_reduce(
        |element| match element {
            Element::Way(way) => {
                let area = classify(way.tags());
                if area.is_some() || members.contains(&way.id()) {
                    vec![WayRecord {
                        id: way.id(),
                        area,
                        nodes: way.refs().collect(),
                    }]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        },
        Vec::new,
        |mut a, mut b| {
            a.append(&mut b);
            a
        },
    )?;
    Ok(found)
}

fn nodes_of<R: Read + Send>(
    reader: ElementReader<R>,
    wanted: &[i64],
) -> Result<Vec<(i64, Point)>, OsmError> {
    let found = reader.par_map_reduce(
        |element| match element {
            Element::Node(n) if wanted.binary_search(&n.id()).is_ok() => {
                vec![(n.id(), (n.lat(), n.lon()))]
            }
            Element::DenseNode(n) if wanted.binary_search(&n.id()).is_ok() => {
                vec![(n.id(), (n.lat(), n.lon()))]
            }
            _ => Vec::new(),
        },
        Vec::new,
        |mut a, mut b| {
            a.append(&mut b);
            a
        },
    )?;
    Ok(found)
}

fn touches(window: GeoWindow, ring: &[Point]) -> bool {
    let (mut south, mut north) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut west, mut east) = (f64::INFINITY, f64::NEG_INFINITY);
    for &(lat, lon) in ring {
        south = south.min(lat);
        north = north.max(lat);
        west = west.min(lon);
        east = east.max(lon);
    }
    south <= window.north && north >= window.south && west <= window.east && east >= window.west
}

/// A group of ways that make one area: a tagged way, or the members of a multipolygon relation.
struct Group {
    class: SurfaceClass,
    priority: u8,
    ways: Vec<RawWay>,
    /// Member ways that are in none of the sources.
    missing: usize,
}

/// The south-west and north-east corner of the nodes of `ways` that are known, if any.
fn extent(ways: &[RawWay], nodes: &BTreeMap<i64, Point>) -> Option<GeoWindow> {
    let mut known = ways
        .iter()
        .flat_map(|w| w.nodes.iter())
        .filter_map(|id| nodes.get(id));
    let &(lat, lon) = known.next()?;
    let mut window = GeoWindow {
        south: lat,
        west: lon,
        north: lat,
        east: lon,
    };
    for &(lat, lon) in known {
        window.south = window.south.min(lat);
        window.north = window.north.max(lat);
        window.west = window.west.min(lon);
        window.east = window.east.max(lon);
    }
    Some(window)
}

fn overlaps(a: GeoWindow, b: GeoWindow) -> bool {
    a.south <= b.north && a.north >= b.south && a.west <= b.east && a.east >= b.west
}

/// The three passes over every source (`count` of them); `open(i)` gives a fresh reader of source `i`, and
/// `name(i)` its file for error messages. Relations, ways and nodes of all sources are merged before the
/// rings are assembled, so a polygon that every extract cuts at its own border can still be whole.
fn read_sources<R: Read + Send>(
    count: usize,
    open: impl Fn(usize) -> osmpbf::Result<ElementReader<R>>,
    name: impl Fn(usize) -> Option<PathBuf>,
    window: GeoWindow,
) -> Result<OsmLand, OsmError> {
    let at = |index: usize, error: OsmError| match name(index) {
        Some(path) => OsmError::File {
            path,
            source: Box::new(error),
        },
        None => error,
    };
    let mut relations = Vec::new();
    for index in 0..count {
        let reader = open(index).map_err(|e| at(index, e.into()))?;
        relations.extend(relations_of(reader).map_err(|e| at(index, e))?);
    }
    // the same id in several sources: keep one, the same one whatever the order of the sources
    relations.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.members.cmp(&b.members)));
    relations.dedup_by_key(|r| r.id);
    let members: BTreeSet<i64> = relations
        .iter()
        .flat_map(|r| r.members.iter().map(|&(id, _)| id))
        .collect();
    let mut ways = Vec::new();
    for index in 0..count {
        let reader = open(index).map_err(|e| at(index, e.into()))?;
        ways.extend(ways_of(reader, &members).map_err(|e| at(index, e))?);
    }
    ways.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.nodes.cmp(&b.nodes)));
    ways.dedup_by_key(|w| w.id);
    let mut wanted: Vec<i64> = ways.iter().flat_map(|w| w.nodes.iter().copied()).collect();
    wanted.sort_unstable();
    wanted.dedup();
    let mut found = Vec::new();
    for index in 0..count {
        let reader = open(index).map_err(|e| at(index, e.into()))?;
        found.extend(nodes_of(reader, &wanted).map_err(|e| at(index, e))?);
    }
    drop(wanted);
    found.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.0.total_cmp(&b.1.0))
            .then_with(|| a.1.1.total_cmp(&b.1.1))
    });
    found.dedup_by_key(|n| n.0);
    let nodes: BTreeMap<i64, Point> = found.into_iter().collect();

    let mut groups: Vec<Group> = Vec::new();
    for way in &ways {
        if let Some((class, priority)) = way.area {
            groups.push(Group {
                class,
                priority,
                ways: vec![RawWay {
                    nodes: way.nodes.clone(),
                    inner: false,
                }],
                missing: 0,
            });
        }
    }
    let by_id: BTreeMap<i64, &WayRecord> = ways.iter().map(|w| (w.id, w)).collect();
    for relation in &relations {
        let mut group = Group {
            class: relation.class,
            priority: relation.priority,
            ways: Vec::new(),
            missing: 0,
        };
        for &(id, inner) in &relation.members {
            match by_id.get(&id) {
                Some(way) => group.ways.push(RawWay {
                    nodes: way.nodes.clone(),
                    inner,
                }),
                None => group.missing += 1,
            }
        }
        groups.push(group);
    }

    let mut land = OsmLand::default();
    for group in groups {
        // Only what lies on the map counts, so that the extract's cut polygons far away neither cost time
        // nor show up as skipped rings.
        if !extent(&group.ways, &nodes).is_some_and(|e| overlaps(e, window)) {
            continue;
        }
        let rings = assemble(&group.ways, &nodes);
        land.skipped_rings += rings.skipped + group.missing;
        for (outer, holes) in rings.polygons {
            if touches(window, &outer) {
                land.polygons.push(LandPolygon {
                    class: group.class,
                    priority: group.priority,
                    outer,
                    holes,
                });
            }
        }
    }
    Ok(land)
}

/// Runs `read` and turns a panic inside the PBF decoder into [`OsmError::Malformed`].
fn guarded<T>(read: impl FnOnce() -> Result<T, OsmError>) -> Result<T, OsmError> {
    catch_unwind(AssertUnwindSafe(read)).unwrap_or_else(|_| {
        Err(OsmError::Malformed(
            "the decoder failed on this data".to_string(),
        ))
    })
}

/// Reads water, rock and built-up areas that touch `window` from `.osm.pbf` files.
///
/// Elements are processed in id order, so the result does not depend on the order of the file's blocks or of the
/// files. The relations, ways and nodes of all files are merged first: a polygon that each extract cuts at its own
/// border is assembled from the parts of all of them. Only areas that lie on the window count, also in
/// [`OsmLand::skipped_rings`].
///
/// # Errors
///
/// [`OsmError::File`], naming the first file that cannot be read or is damaged.
pub fn read_land(paths: &[PathBuf], window: GeoWindow) -> Result<OsmLand, OsmError> {
    guarded(|| {
        read_sources(
            paths.len(),
            |i| match paths.get(i) {
                Some(path) => ElementReader::from_path(path),
                None => Err(osmpbf::Error::from(std::io::Error::other("no such source"))),
            },
            |i| paths.get(i).cloned(),
            window,
        )
    })
}

/// Like [`read_land`] for data that is already in memory (the entry point of the fuzz target).
///
/// # Errors
///
/// [`OsmError::Pbf`] or [`OsmError::Malformed`] for damaged data.
pub fn read_land_from_bytes(bytes: &[u8], window: GeoWindow) -> Result<OsmLand, OsmError> {
    guarded(|| {
        read_sources(
            1,
            |_| Ok(ElementReader::new(Cursor::new(bytes))),
            |_| None,
            window,
        )
    })
}

// --- highway ways -------------------------------------------------------------------------------------------------

/// The tags of a highway way that are kept: the ones the road classification reads.
const WAY_TAGS: [&str; 11] = [
    "highway",
    "tracktype",
    "surface",
    "smoothness",
    "service",
    "sac_scale",
    "trail_visibility",
    "bridge",
    "tunnel",
    "area",
    "waterway",
];

/// The `waterway` values that are kept: the water lines that are wide or steady enough to be a dip in a road.
const WATERWAYS: [&str; 3] = ["stream", "river", "canal"];

/// A way with a `highway` tag, or a stream, river or canal, and the positions of its nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct OsmWay {
    /// The OSM way id.
    pub id: i64,
    /// The tags that the classification reads (`highway`, `tracktype`, `surface`, `smoothness`, `service`, `sac_scale`, `trail_visibility`, `bridge`, `tunnel`, `area`, `waterway`).
    pub tags: BTreeMap<String, String>,
    /// Node ids in order.
    pub nodes: Vec<i64>,
    /// (latitude, longitude) of each node, in the order of `nodes`.
    pub points: Vec<(f64, f64)>,
}

/// The highway ways and the streams, rivers and canals that touch a window.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OsmWays {
    /// The highway ways and the water ways (streams, rivers, canals), each once, in id order.
    pub ways: Vec<OsmWay>,
    /// Highway or water ways that touch the window but miss nodes in all sources (an extract that cut
    /// them), and so were left out.
    pub skipped: usize,
}

/// A highway way as read from a file, before its nodes are resolved.
struct HighwayRecord {
    id: i64,
    tags: Vec<(String, String)>,
    nodes: Vec<i64>,
}

fn window_nodes_of<R: Read + Send>(
    reader: ElementReader<R>,
    window: GeoWindow,
) -> Result<Vec<i64>, OsmError> {
    let inside = |lat: f64, lon: f64| {
        lat >= window.south && lat <= window.north && lon >= window.west && lon <= window.east
    };
    let found = reader.par_map_reduce(
        |element| match element {
            Element::Node(n) if inside(n.lat(), n.lon()) => vec![n.id()],
            Element::DenseNode(n) if inside(n.lat(), n.lon()) => vec![n.id()],
            _ => Vec::new(),
        },
        Vec::new,
        |mut a, mut b| {
            a.append(&mut b);
            a
        },
    )?;
    Ok(found)
}

fn highways_of<R: Read + Send>(
    reader: ElementReader<R>,
    inside: &[i64],
) -> Result<Vec<HighwayRecord>, OsmError> {
    let found = reader.par_map_reduce(
        |element| match element {
            Element::Way(way)
                if way.tags().any(|(key, value)| {
                    key == "highway" || (key == "waterway" && WATERWAYS.contains(&value))
                }) =>
            {
                if way.refs().any(|id| inside.binary_search(&id).is_ok()) {
                    vec![HighwayRecord {
                        id: way.id(),
                        tags: way
                            .tags()
                            .filter(|(key, _)| WAY_TAGS.contains(key))
                            .map(|(key, value)| (key.to_string(), value.to_string()))
                            .collect(),
                        nodes: way.refs().collect(),
                    }]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        },
        Vec::new,
        |mut a, mut b| {
            a.append(&mut b);
            a
        },
    )?;
    Ok(found)
}

/// Three passes over every source: the nodes inside the window, the highway ways that use one of them,
/// then the positions of all nodes of those ways. Only the window's nodes and the ways that touch it are
/// held, whatever the size of the extract.
fn read_way_sources<R: Read + Send>(
    count: usize,
    open: impl Fn(usize) -> osmpbf::Result<ElementReader<R>>,
    name: impl Fn(usize) -> Option<PathBuf>,
    window: GeoWindow,
) -> Result<OsmWays, OsmError> {
    let at = |index: usize, error: OsmError| match name(index) {
        Some(path) => OsmError::File {
            path,
            source: Box::new(error),
        },
        None => error,
    };
    let mut inside = Vec::new();
    for index in 0..count {
        let reader = open(index).map_err(|e| at(index, e.into()))?;
        inside.extend(window_nodes_of(reader, window).map_err(|e| at(index, e))?);
    }
    inside.sort_unstable();
    inside.dedup();
    let mut records = Vec::new();
    for index in 0..count {
        let reader = open(index).map_err(|e| at(index, e.into()))?;
        records.extend(highways_of(reader, &inside).map_err(|e| at(index, e))?);
    }
    drop(inside);
    // the same id in several sources: keep one, the same one whatever the order of the sources
    records.sort_by(|a, b| {
        a.id.cmp(&b.id)
            .then_with(|| a.nodes.cmp(&b.nodes))
            .then_with(|| a.tags.cmp(&b.tags))
    });
    records.dedup_by_key(|r| r.id);
    let mut wanted: Vec<i64> = records
        .iter()
        .flat_map(|w| w.nodes.iter().copied())
        .collect();
    wanted.sort_unstable();
    wanted.dedup();
    let mut found = Vec::new();
    for index in 0..count {
        let reader = open(index).map_err(|e| at(index, e.into()))?;
        found.extend(nodes_of(reader, &wanted).map_err(|e| at(index, e))?);
    }
    drop(wanted);
    found.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.0.total_cmp(&b.1.0))
            .then_with(|| a.1.1.total_cmp(&b.1.1))
    });
    found.dedup_by_key(|n| n.0);
    let nodes: BTreeMap<i64, Point> = found.into_iter().collect();

    let mut result = OsmWays::default();
    for record in records {
        let points: Option<Vec<Point>> = record
            .nodes
            .iter()
            .map(|id| nodes.get(id).copied())
            .collect();
        match points {
            Some(points) if points.len() >= 2 => result.ways.push(OsmWay {
                id: record.id,
                tags: record.tags.into_iter().collect(),
                nodes: record.nodes,
                points,
            }),
            _ => result.skipped += 1,
        }
    }
    Ok(result)
}

/// Reads the highway ways that touch `window` from `.osm.pbf` files, with the positions of their nodes.
///
/// A way is kept if one of its nodes lies in the window; its other nodes may lie outside, so that a road
/// can be clipped at the map's edge afterwards. The ways of all files are merged and come in id order, whatever
/// the order of the files. Ways that miss nodes in every file are counted in [`OsmWays::skipped`].
///
/// # Errors
///
/// [`OsmError::File`], naming the first file that cannot be read or is damaged.
pub fn read_ways(paths: &[PathBuf], window: GeoWindow) -> Result<OsmWays, OsmError> {
    guarded(|| {
        read_way_sources(
            paths.len(),
            |i| match paths.get(i) {
                Some(path) => ElementReader::from_path(path),
                None => Err(osmpbf::Error::from(std::io::Error::other("no such source"))),
            },
            |i| paths.get(i).cloned(),
            window,
        )
    })
}

/// Like [`read_ways`] for data that is already in memory (the entry point of the fuzz target).
///
/// # Errors
///
/// [`OsmError::Pbf`] or [`OsmError::Malformed`] for damaged data.
pub fn read_ways_from_bytes(bytes: &[u8], window: GeoWindow) -> Result<OsmWays, OsmError> {
    guarded(|| {
        read_way_sources(
            1,
            |_| Ok(ElementReader::new(Cursor::new(bytes))),
            |_| None,
            window,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_in_the_decoder_becomes_an_error() {
        // resume_unwind raises a panic without running the panic hook, like a panic inside a dependency
        let result: Result<OsmLand, _> =
            guarded(|| std::panic::resume_unwind(Box::new("decoder failure")));
        assert!(matches!(result, Err(OsmError::Malformed(_))), "{result:?}");
    }

    #[test]
    fn a_result_passes_through_the_guard() {
        assert_eq!(
            guarded(|| Ok(OsmLand::default())).unwrap(),
            OsmLand::default()
        );
    }
}

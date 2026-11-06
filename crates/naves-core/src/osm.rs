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
//! reader runs under `catch_unwind` and turns such a panic into [`OsmError::Malformed`]; the fuzz target
//! `osm_read` and the damaged-file tests check that nothing else escapes.

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
) -> Result<BTreeMap<i64, (f64, f64)>, OsmError> {
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
    Ok(found.into_iter().collect())
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

/// The three passes over one source; `open` gives a fresh reader for each.
fn read_passes<R: Read + Send>(
    open: impl Fn() -> osmpbf::Result<ElementReader<R>>,
    window: GeoWindow,
) -> Result<OsmLand, OsmError> {
    let mut relations = relations_of(open()?)?;
    relations.sort_by_key(|r| r.id);
    let members: BTreeSet<i64> = relations
        .iter()
        .flat_map(|r| r.members.iter().map(|&(id, _)| id))
        .collect();
    let mut ways = ways_of(open()?, &members)?;
    ways.sort_by_key(|w| w.id);
    let mut wanted: Vec<i64> = ways.iter().flat_map(|w| w.nodes.iter().copied()).collect();
    wanted.sort_unstable();
    wanted.dedup();
    let nodes = nodes_of(open()?, &wanted)?;
    drop(wanted);

    let mut land = OsmLand::default();
    let mut keep = |class: SurfaceClass, priority: u8, rings: Rings| {
        land.skipped_rings += rings.skipped;
        for (outer, holes) in rings.polygons {
            if touches(window, &outer) {
                land.polygons.push(LandPolygon {
                    class,
                    priority,
                    outer,
                    holes,
                });
            }
        }
    };
    for way in &ways {
        if let Some((class, priority)) = way.area {
            let raw = RawWay {
                nodes: way.nodes.clone(),
                inner: false,
            };
            keep(class, priority, assemble(&[raw], &nodes));
        }
    }
    let by_id: BTreeMap<i64, &WayRecord> = ways.iter().map(|w| (w.id, w)).collect();
    for relation in &relations {
        let mut missing = 0;
        let raw: Vec<RawWay> = relation
            .members
            .iter()
            .filter_map(|&(id, inner)| {
                let found = by_id.get(&id);
                if found.is_none() {
                    missing += 1;
                }
                found.map(|w| RawWay {
                    nodes: w.nodes.clone(),
                    inner,
                })
            })
            .collect();
        let mut rings = assemble(&raw, &nodes);
        rings.skipped += missing;
        keep(relation.class, relation.priority, rings);
    }
    Ok(land)
}

/// Runs `read` and turns a panic inside the PBF decoder into [`OsmError::Malformed`].
fn guarded(read: impl FnOnce() -> Result<OsmLand, OsmError>) -> Result<OsmLand, OsmError> {
    catch_unwind(AssertUnwindSafe(read)).unwrap_or_else(|_| {
        Err(OsmError::Malformed(
            "the decoder failed on this data".to_string(),
        ))
    })
}

/// Reads water, rock and built-up areas that touch `window` from `.osm.pbf` files.
///
/// Elements are processed in id order, so the result does not depend on the order of the file's blocks.
///
/// # Errors
///
/// [`OsmError::File`], naming the first file that cannot be read or is damaged.
pub fn read_land(paths: &[PathBuf], window: GeoWindow) -> Result<OsmLand, OsmError> {
    let mut all = OsmLand::default();
    for path in paths {
        let land = guarded(|| read_passes(|| ElementReader::from_path(path), window)).map_err(
            |source| OsmError::File {
                path: path.clone(),
                source: Box::new(source),
            },
        )?;
        all.polygons.extend(land.polygons);
        all.skipped_rings += land.skipped_rings;
    }
    Ok(all)
}

/// Like [`read_land`] for data that is already in memory (the entry point of the fuzz target).
///
/// # Errors
///
/// [`OsmError::Pbf`] or [`OsmError::Malformed`] for damaged data.
pub fn read_land_from_bytes(bytes: &[u8], window: GeoWindow) -> Result<OsmLand, OsmError> {
    guarded(|| read_passes(|| Ok(ElementReader::new(Cursor::new(bytes))), window))
}

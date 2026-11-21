// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Lots: car parks, camp sites and similar areas.
//!
//! [`areas`] turns the [`LotPolygon`]s that [`crate::osm::read_land`] reads into map metres and says what each one
//! is ([`LotKind`]), whether it lies on the ground ([`Placement`]), how big it is and what its `surface` tag says.
//! Inside means even-odd inside the outer ring and in no hole ([`contains`]); a self-intersecting ring is not
//! rejected, and its area is counted from samples on a [`LOT_SAMPLE_M`] grid.

use crate::heightmap::MapGeo;
use crate::osm::{LotPolygon, OsmLand};

/// The distance between the samples that count the area of a lot, in metres.
pub const LOT_SAMPLE_M: f64 = 1.0;

/// The most sample rows one lot may have (50 km); a taller ring is not on any map and is skipped.
const MAX_ROWS: f64 = 50_000.0;

/// What the lot is, by its tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LotKind {
    /// `amenity=parking`.
    CarPark,
    /// `amenity=tourist_bus_parking`.
    BusParking,
    /// `highway=rest_area`.
    RestArea,
    /// `tourism=camp_site`.
    CampSite,
    /// `tourism=caravan_site`.
    CaravanSite,
}

/// Where the lot lies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// On the ground, in its own area.
    Ground,
    /// A strip along a street (`parking=street_side`, `lane` or `layby`).
    StreetSide,
    /// Not at ground level: underground, a multi-storey or roof car park, a building, another `layer`.
    NotAtGround,
}

/// Whether a `surface` tag names a paved or an unpaved surface; the type of
/// [`FeatureKind::Road::paving`](crate::features::FeatureKind::Road).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Paving {
    /// Asphalt, concrete, paving stones and the like.
    Paved,
    /// Gravel, dirt, grass and the like.
    Unpaved,
}

impl Paving {
    /// The paving of a `surface` tag value; `None` for a value that is on neither list.
    #[must_use]
    pub fn from_surface(value: &str) -> Option<Paving> {
        paving(value)
    }
}

/// The paving of a `surface` tag value (the spec's two lists); `None` for a value that is on neither list.
#[must_use]
pub fn paving(value: &str) -> Option<Paving> {
    match value {
        "asphalt" | "paved" | "concrete" | "concrete:plates" | "concrete:lanes"
        | "paving_stones" | "sett" | "cobblestone" | "unhewn_cobblestone" | "chipseal"
        | "bricks" | "metal" | "wood" => Some(Paving::Paved),
        "unpaved" | "gravel" | "fine_gravel" | "compacted" | "dirt" | "earth" | "ground"
        | "grass" | "grass_paver" | "mud" | "sand" | "pebblestone" | "rock" | "woodchips" => {
            Some(Paving::Unpaved)
        }
        _ => None,
    }
}

/// A lot in map metres.
#[derive(Debug, Clone, PartialEq)]
pub struct LotArea {
    /// `osm/way/<id>`, or `osm/relation/<id>/<part>` (one part per outer ring).
    pub id: String,
    /// What the lot is.
    pub kind: LotKind,
    /// Where it lies.
    pub placement: Placement,
    /// The outer ring, closed, as map (x, y) in metres.
    pub outer: Vec<(f64, f64)>,
    /// Holes in the lot, closed rings. A hole is taken to lie inside the outer ring.
    pub holes: Vec<Vec<(f64, f64)>>,
    /// The area of the outer ring less the holes, counted on a [`LOT_SAMPLE_M`] grid.
    pub area_m2: f64,
    /// What the `surface` tag of the lot says, if anything.
    pub surface_tag: Option<Paving>,
}

fn kind_of(tags: &std::collections::BTreeMap<String, String>) -> Option<LotKind> {
    let get = |key: &str| tags.get(key).map(String::as_str);
    match (get("amenity"), get("highway"), get("tourism")) {
        (Some("parking"), _, _) => Some(LotKind::CarPark),
        (Some("tourist_bus_parking"), _, _) => Some(LotKind::BusParking),
        (_, Some("rest_area"), _) => Some(LotKind::RestArea),
        (_, _, Some("camp_site")) => Some(LotKind::CampSite),
        (_, _, Some("caravan_site")) => Some(LotKind::CaravanSite),
        _ => None,
    }
}

fn placement_of(tags: &std::collections::BTreeMap<String, String>) -> Placement {
    let get = |key: &str| tags.get(key).map(String::as_str);
    let parking = get("parking");
    // a layer that is not 0 (or not a number) is not at ground level; so is a building
    let layer_off = get("layer").is_some_and(|v| v.trim().parse::<i32>() != Ok(0));
    if matches!(parking, Some("underground" | "multi-storey" | "rooftop"))
        || get("building").is_some_and(|v| v != "no")
        || matches!(get("location"), Some("underground" | "rooftop"))
        || layer_off
    {
        Placement::NotAtGround
    } else if matches!(parking, Some("street_side" | "lane" | "layby")) {
        Placement::StreetSide
    } else {
        Placement::Ground
    }
}

fn project_ring(frame: &MapGeo, ring: &[(f64, f64)]) -> Option<Vec<(f64, f64)>> {
    ring.iter()
        .map(|&(lat, lon)| {
            frame
                .to_map(lat, lon)
                .ok()
                .filter(|(x, y)| x.is_finite() && y.is_finite())
        })
        .collect()
}

/// The number of distinct corners of a closed ring.
fn corners(ring: &[(f64, f64)]) -> usize {
    let mut sorted: Vec<(f64, f64)> = ring.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    sorted.dedup();
    sorted.len()
}

/// The number of sample centres (a grid of [`LOT_SAMPLE_M`], offset by half a cell) inside `ring`, even-odd.
/// `None` if the ring is too tall for the sample rows.
fn sample_count(ring: &[(f64, f64)]) -> Option<f64> {
    let (mut min_x, mut min_y, mut max_y) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for &(x, y) in ring {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    if (max_y - min_y) / LOT_SAMPLE_M > MAX_ROWS {
        return None;
    }
    let mut total = 0.0;
    let mut crossings: Vec<f64> = Vec::new();
    let mut y = min_y + LOT_SAMPLE_M / 2.0;
    while y < max_y {
        crossings.clear();
        for pair in ring.windows(2) {
            let [(xa, ya), (xb, yb)] = pair else { continue };
            if (*ya > y) != (*yb > y) {
                crossings.push(xa + (y - ya) / (yb - ya) * (xb - xa));
            }
        }
        crossings.sort_by(f64::total_cmp);
        for [from, to] in crossings.as_chunks::<2>().0 {
            // sample centres at min_x + (i + 0.5) * step inside [from, to)
            let first = ((from - min_x) / LOT_SAMPLE_M - 0.5).ceil();
            let last = ((to - min_x) / LOT_SAMPLE_M - 0.5).ceil();
            total += (last - first).max(0.0);
        }
        y += LOT_SAMPLE_M;
    }
    Some(total)
}

fn area_of(outer: &[(f64, f64)], holes: &[Vec<(f64, f64)>]) -> Option<f64> {
    let mut count = sample_count(outer)?;
    for hole in holes {
        count -= sample_count(hole)?;
    }
    Some(count.max(0.0) * LOT_SAMPLE_M * LOT_SAMPLE_M)
}

fn make_area(frame: &MapGeo, lot: &LotPolygon) -> Option<LotArea> {
    let kind = kind_of(&lot.tags)?;
    let outer = project_ring(frame, &lot.outer)?;
    if corners(&outer) < 3 {
        return None;
    }
    let mut holes = Vec::new();
    for hole in &lot.holes {
        let hole = project_ring(frame, hole)?;
        // a hole of fewer than three corners has no area
        if corners(&hole) >= 3 {
            holes.push(hole);
        }
    }
    let area_m2 = area_of(&outer, &holes)?;
    let id = if lot.relation {
        format!("osm/relation/{}/{}", lot.id, lot.part)
    } else {
        format!("osm/way/{}", lot.id)
    };
    Some(LotArea {
        id,
        kind,
        placement: placement_of(&lot.tags),
        outer,
        holes,
        area_m2,
        surface_tag: lot
            .tags
            .get("surface")
            .and_then(|value| Paving::from_surface(value)),
    })
}

/// The lots of `land` in map metres, ways before relations and each in id order (the parts of a relation in ring
/// order), and the number of lots skipped: fewer than three distinct corners, a point that cannot be projected or
/// is not finite, a ring too tall to count, or tags that make no lot.
#[must_use]
pub fn areas(land: &OsmLand, frame: &MapGeo) -> (Vec<LotArea>, usize) {
    let mut lots: Vec<&LotPolygon> = land.lots.iter().collect();
    lots.sort_by_key(|lot| (lot.relation, lot.id, lot.part));
    let mut skipped = 0;
    let mut found = Vec::new();
    for lot in lots {
        match make_area(frame, lot) {
            Some(area) => found.push(area),
            None => skipped += 1,
        }
    }
    (found, skipped)
}

fn ring_contains(ring: &[(f64, f64)], (px, py): (f64, f64)) -> bool {
    let mut inside = false;
    for pair in ring.windows(2) {
        let [(xa, ya), (xb, yb)] = pair else { continue };
        if (*ya > py) != (*yb > py) && px < xa + (py - ya) / (yb - ya) * (xb - xa) {
            inside = !inside;
        }
    }
    inside
}

/// Whether `p` is inside the lot: even-odd inside the outer ring and in none of the holes.
#[must_use]
pub fn contains(area: &LotArea, p: (f64, f64)) -> bool {
    ring_contains(&area.outer, p) && !area.holes.iter().any(|hole| ring_contains(hole, p))
}

fn segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

/// The distance in metres from `p` to the nearest edge of the lot, the edges of its holes included (infinite for
/// a lot without edges).
#[must_use]
pub fn edge_distance(area: &LotArea, p: (f64, f64)) -> f64 {
    std::iter::once(&area.outer)
        .chain(area.holes.iter())
        .flat_map(|ring| ring.windows(2))
        .filter_map(|pair| match pair {
            [a, b] => Some(segment_distance(p, *a, *b)),
            _ => None,
        })
        .fold(f64::INFINITY, f64::min)
}

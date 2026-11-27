// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Lots: car parks, camp sites and similar areas.
//!
//! [`areas`] turns the [`LotPolygon`]s that [`crate::osm::read_land`] reads into map metres and says what each one
//! is ([`LotKind`]), whether it lies on the ground ([`Placement`]), how big it is and what its `surface` tag says.
//! Inside means even-odd inside the outer ring and in no hole ([`contains`]); a self-intersecting ring is not
//! rejected, and its area is the number of samples on a 1 m grid for which [`contains`] is true, counted
//! on the part of the lot that lies on the map. The grid is fixed at 1 m.

mod paving;
mod surface;

pub use paving::{SurfaceSource, surfaces, surfaces_with, surfaces_with_sources};
pub(crate) use surface::touches;
pub use surface::{
    Assessed, Form, LotSurface, NO_GROUP, Outcome, Polygon, Surface, assess, local_relief,
};

use crate::netclean::CleanReport;
use crate::surface::SurfaceClass;

/// The switch of the lot stage: on since the judging and the game check confirmed it. Off (a build with
/// [`crate::lot::LOTS_ENABLED`] set to `false`, or `Stages { lots: false, .. }` per call), the stage does nothing: no
/// lot is assessed, no road is removed and no ground is shaped.
pub const LOTS_ENABLED: bool = true;
/// Digitising tolerance of "inside", and of "touching" for groups, in metres.
pub const LOT_INSIDE_TOL_M: f64 = 1.0;
/// A lot whose plane is within this of its ground (95th percentile) gets the plane, in metres.
pub const LOT_PLANE_FIT_M: f64 = 0.5;
/// The share of a stretch's length that must lie inside a lot by more than [`LOT_INSIDE_TOL_M`] for the stretch to
/// count as inside it (a road drawn on the outline lies within the tolerance everywhere and is not inside).
pub const LOT_INSIDE_SHARE: f64 = 0.5;
/// How far inside the edge of a lot a cut access road ends, in metres (`limits::RAMP_JOINT_MIN_M`: one terrain cell
/// at 2048).
pub const LOT_ENTRY_M: f64 = crate::limits::RAMP_JOINT_MIN_M;
/// The least length, in metres, that a cut access road keeps (from its other end): a road that starts at a T beside the
/// lot would otherwise be left with a few metres, which the placement cannot lay (a T's junction piece takes half its
/// length, 6.25 m, from the road, and what is left must be longer than half a piece is wide). 15 m holds the junction's
/// half, one piece and an end cap. The cut goes [`LOT_ENTRY_M`] beyond the lot's edge, or at this length from the other
/// end where that is farther; a road no longer than this is not cut.
pub const LOT_ACCESS_MIN_M: f64 = 15.0;
/// The smallest radius of curvature of a lot's surface, in metres: the gentlest road's vertical radius
/// (`limits::min_vertical_radius(Family::Asf1)`).
pub const LOT_MIN_RADIUS_M: f64 = 600.0;
/// The "short distance" of "no local height changes": six terrain cells, in metres.
pub const LOT_RELIEF_LENGTH_M: f64 = 24.0;
/// The largest move of a vertex; beyond it the change is scaled for the whole lot, in metres.
pub const LOT_MAX_MOVE_M: f64 = 3.0;
/// The slope (95th percentile) above which a lot is reported steep.
pub const LOT_STEEP: f64 = crate::limits::CROSS_FALL_LIMIT;
/// Fewer terrain vertices inside make a lot surface only.
pub const LOT_MIN_VERTICES: usize = 3;
/// The share of the ground in water above which a lot is excluded.
pub const LOT_WATER_MAX: f64 = 0.5;

use crate::heightmap::MapGeo;
use crate::osm::{LotPolygon, OsmLand};

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

impl LotKind {
    /// Every kind, in the order the reports count them.
    pub const ALL: [LotKind; 5] = [
        LotKind::CarPark,
        LotKind::BusParking,
        LotKind::RestArea,
        LotKind::CampSite,
        LotKind::CaravanSite,
    ];

    /// The stable name of the kind, for the report and the findings.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            LotKind::CarPark => "car_park",
            LotKind::BusParking => "bus_parking",
            LotKind::RestArea => "rest_area",
            LotKind::CampSite => "camp_site",
            LotKind::CaravanSite => "caravan_site",
        }
    }

    /// What the kind is called in words, singular and plural.
    #[must_use]
    pub const fn words(self) -> (&'static str, &'static str) {
        match self {
            LotKind::CarPark => ("car park", "car parks"),
            LotKind::BusParking => ("coach park", "coach parks"),
            LotKind::RestArea => ("rest area", "rest areas"),
            LotKind::CampSite => ("camp site", "camp sites"),
            LotKind::CaravanSite => ("caravan site", "caravan sites"),
        }
    }
}

impl Form {
    /// `plane` or `quadratic`.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Form::Plane => "plane",
            Form::Quadratic => "quadratic",
        }
    }
}

impl Outcome {
    /// `smoothed`, `surface_only` or `excluded`.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Outcome::Smoothed { .. } => "smoothed",
            Outcome::SurfaceOnly { .. } => "surface_only",
            Outcome::Excluded { .. } => "excluded",
        }
    }

    /// Why a lot is not smoothed: `street_side`, `no_vertex`, `not_at_ground` or `in_water`; `None` for a smoothed
    /// lot.
    #[must_use]
    pub fn reason(self) -> Option<&'static str> {
        match self {
            Outcome::Smoothed { .. } => None,
            Outcome::SurfaceOnly { reason } | Outcome::Excluded { reason } => Some(reason),
        }
    }
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
    /// Holes in the lot, closed rings.
    pub holes: Vec<Vec<(f64, f64)>>,
    /// The area in square metres: the samples of a fixed 1 m grid inside [`contains`], on the map only. A lot that
    /// lies off the map has area 0, so a caller that computes a share of the area must guard against zero.
    pub area_m2: f64,
    /// What the `surface` tag of the lot says, if anything.
    pub surface_tag: Option<Paving>,
    /// Holes of the lot's ring that were dropped because they have fewer than three distinct corners (no area, no
    /// point inside). Not part of the skipped count, which counts lots.
    pub holes_dropped: usize,
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

/// The sample indices `[from, to)` of the row at `y` that lie inside `ring` (even-odd), clipped to `0..=cap`.
fn row_spans(
    ring: &[(f64, f64)],
    y: f64,
    cap: f64,
    crossings: &mut Vec<f64>,
    spans: &mut Vec<(f64, f64)>,
) {
    crossings.clear();
    for pair in ring.windows(2) {
        let [(xa, ya), (xb, yb)] = pair else { continue };
        if (*ya > y) != (*yb > y) {
            crossings.push(xa + (y - ya) / (yb - ya) * (xb - xa));
        }
    }
    crossings.sort_by(f64::total_cmp);
    for [from, to] in crossings.as_chunks::<2>().0 {
        // the sample centres are at i + 0.5
        let first = (from - 0.5).ceil().clamp(0.0, cap);
        let last = (to - 0.5).ceil().clamp(0.0, cap);
        if last > first {
            spans.push((first, last));
        }
    }
}

/// A row or sample index as an integer, for a value already clamped to `0..=limit`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to the map, a few thousand at most"
)]
fn index(value: f64, limit: f64) -> u32 {
    value.clamp(0.0, limit) as u32
}

/// The number of samples of the 1 m grid (centres at `i + 0.5`) inside the outer ring and in no hole, on the map
/// (`0..size` in both directions): one count per row, the outer ring's spans less the union of the holes' spans.
fn area_of(outer: &[(f64, f64)], holes: &[Vec<(f64, f64)>], size: f64) -> f64 {
    let cap = (size - 0.5).ceil().max(0.0);
    let (mut min_y, mut max_y) = (f64::INFINITY, f64::NEG_INFINITY);
    for &(_, y) in outer {
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    // rows are integers, bounded by the map
    let first_row = index((min_y - 0.5).floor().max(0.0), cap);
    let end_row = index((max_y + 0.5).ceil().max(0.0), cap);
    let mut total = 0.0;
    let mut spans: Vec<(f64, f64)> = Vec::new();
    let mut cut: Vec<(f64, f64)> = Vec::new();
    let mut crossings: Vec<f64> = Vec::new();
    for row in first_row..end_row {
        let y = f64::from(row) + 0.5;
        spans.clear();
        row_spans(outer, y, cap, &mut crossings, &mut spans);
        if spans.is_empty() {
            continue;
        }
        cut.clear();
        for hole in holes {
            row_spans(hole, y, cap, &mut crossings, &mut cut);
        }
        cut.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        // merge the holes' spans into disjoint ones
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for &(from, to) in &cut {
            match merged.last_mut() {
                Some(last) if from <= last.1 => last.1 = last.1.max(to),
                _ => merged.push((from, to)),
            }
        }
        for &(from, to) in &spans {
            let covered: f64 = merged
                .iter()
                .map(|&(a, b)| (to.min(b) - from.max(a)).max(0.0))
                .sum();
            total += (to - from) - covered;
        }
    }
    total
}

fn make_area(frame: &MapGeo, lot: &LotPolygon) -> Option<LotArea> {
    let kind = kind_of(&lot.tags)?;
    let outer = project_ring(frame, &lot.outer)?;
    if corners(&outer) < 3 {
        return None;
    }
    let mut holes = Vec::new();
    let mut holes_dropped = 0;
    for hole in &lot.holes {
        // a point that cannot be projected leaves the hole's extent unknown: the lot is skipped
        let hole = project_ring(frame, hole)?;
        if corners(&hole) < 3 {
            // no area and no point inside: dropping it changes nothing about the lot
            holes_dropped += 1;
        } else {
            holes.push(hole);
        }
    }
    let area_m2 = area_of(&outer, &holes, frame.map_size_m());
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
        surface_tag: lot.tags.get("surface").and_then(|value| paving(value)),
        holes_dropped,
    })
}

/// The lots of `land` in map metres, ways before relations and each in id order (the parts of a relation in ring
/// order), and the number of lots skipped: fewer than three distinct corners, a point that cannot be projected or
/// is not finite (in the outer ring or in a hole), or tags that make no lot. A hole with fewer than three distinct
/// corners and finite points is dropped and the lot stays ([`LotArea::holes_dropped`]).
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

pub(crate) fn ring_contains(ring: &[(f64, f64)], (px, py): (f64, f64)) -> bool {
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

pub(crate) fn segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

/// The centre of the lot, for the findings and the report: the mean of its outer ring without the closing point.
#[must_use]
pub fn centre(area: &LotArea) -> (f64, f64) {
    let mut points = area.outer.as_slice();
    // a closing point is the first point again, bit for bit; comparing the bits keeps `clippy::float_cmp` out of it
    let closed = points.len() > 1 && {
        match (points.first(), points.last()) {
            (Some(first), Some(last)) => {
                first.0.to_bits() == last.0.to_bits() && first.1.to_bits() == last.1.to_bits()
            }
            _ => false,
        }
    };
    if closed {
        points = points.get(..points.len() - 1).unwrap_or(points);
    }
    if points.is_empty() {
        return (0.0, 0.0);
    }
    let sum = points
        .iter()
        .fold((0.0, 0.0), |(x, y), p| (x + p.0, y + p.1));
    #[expect(
        clippy::cast_precision_loss,
        reason = "an outer ring has a few thousand points at most"
    )]
    let count = points.len() as f64;
    (sum.0 / count, sum.1 / count)
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

/// `1 car park` or `3 car parks`.
fn plural(number: usize, one: &str, many: &str) -> String {
    if number == 1 {
        format!("{number} {one}")
    } else {
        format!("{number} {many}")
    }
}

/// How many lots of a report there are, by kind, outcome, form and reason, and by surface class.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LotCounts {
    /// Every lot.
    pub total: usize,
    /// The lots of every kind that occurs, in the order of [`LotKind::ALL`].
    pub kinds: Vec<(LotKind, usize)>,
    /// The smoothed lots of every form that occurs, the plane before the quadratic.
    pub forms: Vec<(Form, usize)>,
    /// Smoothed lots.
    pub smoothed: usize,
    /// Lots that get a surface only.
    pub surface_only: usize,
    /// Lots that are left alone.
    pub excluded: usize,
    /// The lots that get a surface only or are excluded, by reason, in a fixed order.
    pub reasons: Vec<(&'static str, usize)>,
    /// Every kind that occurs with every outcome that occurs, in the order of [`LotKind::ALL`] and of the outcomes
    /// (smoothed with its form first, then the reasons).
    pub outcomes: Vec<(LotKind, Outcome, usize)>,
    /// Lots painted paved ([`SurfaceClass::Tarmac`]): every lot that gets a surface, a surface-only lot included.
    /// [`LotReport::summary`] counts the smoothed lots of the two classes, not these.
    pub paved: usize,
    /// Lots painted grass or anything else (see [`LotCounts::paved`]).
    pub grass: usize,
    /// Smoothed lots whose slope is above [`LOT_STEEP`].
    pub steep: usize,
    /// Smoothed lots whose change the move guard scaled ([`LOT_MAX_MOVE_M`]).
    pub move_limited: usize,
}

/// What the lot stage found and decided: every assessed lot, the smooth surface of every group and the surface class
/// of every lot that gets one. The roads that were removed and the access roads that were cut are in the
/// [`CleanReport`] of the same graph.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LotReport {
    /// Every lot, sorted by id (see [`assess`]).
    pub assessed: Vec<Assessed>,
    /// The smooth surface of every group of smoothed lots, in the order of their lowest member id.
    pub surfaces: Vec<LotSurface>,
    /// The surface class of every lot that is not excluded, sorted by id.
    pub classes: Vec<(String, SurfaceClass)>,
    /// What decided the class of the lot at the same index in [`LotReport::classes`].
    pub sources: Vec<SurfaceSource>,
}

impl LotReport {
    /// The group's smooth surface of `lot`; `None` for a lot that has none (surface only or excluded).
    #[must_use]
    pub fn surface_of(&self, lot: &Assessed) -> Option<&LotSurface> {
        self.surfaces.get(lot.group)
    }

    /// The surface class of the lot with `id`; `None` for a lot that gets none.
    #[must_use]
    pub fn class_of(&self, id: &str) -> Option<SurfaceClass> {
        self.classes
            .iter()
            .find(|(lot, _)| lot == id)
            .map(|(_, class)| *class)
    }

    /// What decided the class of the lot with `id`; `None` for a lot that gets none.
    #[must_use]
    pub fn source_of(&self, id: &str) -> Option<SurfaceSource> {
        self.classes
            .iter()
            .position(|(lot, _)| lot == id)
            .and_then(|index| self.sources.get(index).copied())
    }

    /// The factor the move guard scaled the lot's change by, where it scaled it (below 1); `None` for a lot that
    /// has no surface or whose change is not scaled.
    #[must_use]
    pub fn move_limited(&self, lot: &Assessed) -> Option<f64> {
        self.surface_of(lot)
            .map(|surface| surface.surface.lambda)
            .filter(|lambda| *lambda < 1.0)
    }

    /// The counts of the lots of this report.
    #[must_use]
    pub fn counts(&self) -> LotCounts {
        let mut counts = LotCounts {
            total: self.assessed.len(),
            ..LotCounts::default()
        };
        for kind in LotKind::ALL {
            let number = self
                .assessed
                .iter()
                .filter(|lot| lot.area.kind == kind)
                .count();
            if number > 0 {
                counts.kinds.push((kind, number));
            }
        }
        let forms = [Form::Plane, Form::Quadratic];
        for form in forms {
            let number = self
                .assessed
                .iter()
                .filter(|lot| lot.outcome == Outcome::Smoothed { form })
                .count();
            if number > 0 {
                counts.forms.push((form, number));
            }
        }
        let with = |outcome: Outcome| {
            self.assessed
                .iter()
                .filter(|lot| lot.outcome == outcome)
                .count()
        };
        counts.smoothed = with(Outcome::Smoothed { form: Form::Plane })
            + with(Outcome::Smoothed {
                form: Form::Quadratic,
            });
        counts.surface_only = with(Outcome::SurfaceOnly {
            reason: "street_side",
        }) + with(Outcome::SurfaceOnly {
            reason: "no_vertex",
        });
        counts.excluded = with(Outcome::Excluded {
            reason: "not_at_ground",
        }) + with(Outcome::Excluded { reason: "in_water" });
        let mut outcomes: Vec<Outcome> = forms
            .iter()
            .map(|&form| Outcome::Smoothed { form })
            .collect();
        for reason in ["street_side", "no_vertex", "not_at_ground", "in_water"] {
            let number = self
                .assessed
                .iter()
                .filter(|lot| lot.outcome.reason() == Some(reason))
                .count();
            if number > 0 {
                counts.reasons.push((reason, number));
                outcomes.push(match reason {
                    "street_side" | "no_vertex" => Outcome::SurfaceOnly { reason },
                    _ => Outcome::Excluded { reason },
                });
            }
        }
        for kind in LotKind::ALL {
            for outcome in &outcomes {
                let number = self
                    .assessed
                    .iter()
                    .filter(|lot| lot.area.kind == kind && lot.outcome == *outcome)
                    .count();
                if number > 0 {
                    counts.outcomes.push((kind, *outcome, number));
                }
            }
        }
        for (_, class) in &self.classes {
            if *class == SurfaceClass::Tarmac {
                counts.paved += 1;
            } else {
                counts.grass += 1;
            }
        }
        counts.steep = self
            .assessed
            .iter()
            .filter(|lot| lot.steep && matches!(lot.outcome, Outcome::Smoothed { .. }))
            .count();
        counts.move_limited = self
            .assessed
            .iter()
            .filter(|lot| self.move_limited(lot).is_some())
            .count();
        counts
    }

    /// One line for the build summary: the lots of each kind, how many were smoothed with which form and surface,
    /// how many are steep or move limited, how many get a surface only or are excluded, and what `clean` removed and
    /// cut.
    #[must_use]
    pub fn summary(&self, clean: &CleanReport) -> String {
        let counts = self.counts();
        if counts.total == 0 {
            return "Lots: none".to_string();
        }
        let kinds: Vec<String> = counts
            .kinds
            .iter()
            .map(|&(kind, number)| {
                let (one, many) = kind.words();
                plural(number, one, many)
            })
            .collect();
        let form = |wanted: Form| {
            counts
                .forms
                .iter()
                .find(|(form, _)| *form == wanted)
                .map_or(0, |(_, number)| *number)
        };
        // the surface classes of the smoothed lots: they are what the parenthesised part describes
        let smoothed_paved = self
            .assessed
            .iter()
            .filter(|lot| matches!(lot.outcome, Outcome::Smoothed { .. }))
            .filter(|lot| self.class_of(&lot.area.id) == Some(SurfaceClass::Tarmac))
            .count();
        let smoothed_grass = counts.smoothed - smoothed_paved;
        let metres: f64 = clean.lot_roads.iter().map(|dropped| dropped.length_m).sum();
        // the access roads that got no entry at all (their node went in a later step, or the placement dropped
        // what the cut left), which the line names so that a lot without an access road is not silent
        let lost = if clean.lot_entries_lost == 0 {
            String::new()
        } else {
            format!(
                ", {} lost",
                plural(clean.lot_entries_lost, "entry", "entries")
            )
        };
        format!(
            "Lots: {} ({}), {} smoothed ({} plane, {} quadratic; {} paved, {} grass; {} steep, {} move limited), \
             {} surface only, {} excluded; {} ({metres:.0} m) removed, {} cut{lost}",
            counts.total,
            kinds.join(", "),
            counts.smoothed,
            form(Form::Plane),
            form(Form::Quadratic),
            smoothed_paved,
            smoothed_grass,
            counts.steep,
            counts.move_limited,
            counts.surface_only,
            counts.excluded,
            plural(clean.lot_roads.len(), "road", "roads"),
            plural(clean.lot_access.len(), "access road", "access roads"),
        )
    }
}

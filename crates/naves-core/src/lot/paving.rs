// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Paved or grass, per lot (`docs/superpowers/specs/2026-11-21-car-parks-design.md`, "The surface").
//!
//! For every lot that is not excluded, in the order of the ids ([`surfaces`]):
//!
//! 1. A paved or unpaved `surface` tag on the lot itself wins ([`LotArea::surface_tag`]).
//! 2. A camp site (`tourism=camp_site`) is grass, unless its own tag says paved (the maintainer's ruling: a camp site is
//!    meadow with pitches, and a paved car park inside it is a lot of its own).
//! 3. Any other lot (car park, coach parking, rest area, caravan site) is decided by the roads that serve it. These are
//!    the stretches the lot step removed from inside it ([`CleanReport::lot_roads`]) and the part inside it of the
//!    access roads it cut ([`CleanReport::lot_access`]), each metre counted by its way's `surface` tag, and by its
//!    family where the way has none or an unknown one (`Asf1`, `Asf2`, `Asf3` and `City` paved, `Grav` and `Mud` grass).
//!    Paved if the paved metres are at least the grass metres, a tie goes to paved.
//! 4. A lot without roads of its own that is in a group (lots that touch, [`crate::netclean::lot::groups`]) takes the
//!    decision of the group: the roads of all its members, pooled.
//! 5. A lot still without roads takes the roads whose earthwork touches it: every stretch of the graph before the lot
//!    step, except those removed in a lot, bridges and tunnels, counted by the metres that lie in the lot or within
//!    `core_reach_m(half the family's width, cell) + cell` of its edge.
//! 6. Nothing at all: grass.
//!
//! The metres inside a lot are those of the road's ways, not the stretch's average, when the road features are given
//! ([`surfaces_with`]): a stretch can be made of ways with different tags. Without them each metre of a stretch counts
//! by the stretch's own shares ([`Stretch::paved_m`], [`Stretch::unpaved_m`]), which is exact for a stretch that was
//! removed whole and an average for the part of an access road that lies in the lot.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::Family;
use crate::conform::core_reach_m;
use crate::features::{Feature, FeatureKind};
use crate::lot::{LotArea, LotKind, Outcome, Paving, contains, edge_distance, segment_distance};
use crate::netclean::CleanReport;
use crate::netclean::lot::groups;
use crate::roadnet::{EPS_M, RoadGraph, Stretch, bounds, polyline_length};
use crate::surface::SurfaceClass;

use super::Assessed;

/// A point in map metres.
type Point = (f64, f64);

/// Metres below which a sum counts as nothing, and the slack of a tie.
const TIE_M: f64 = 1e-6;
/// The length of the pieces a line is cut into to be counted, in metres.
const PIECE_M: f64 = 0.5;
/// How far a feature's line may lie from a stretch's line and still be the way the stretch was made from, in metres.
const WAY_M: f64 = 0.75;

/// Metres of road that are paved and metres that are grass.
#[derive(Debug, Clone, Copy, Default)]
struct Tally {
    paved: f64,
    grass: f64,
}

impl Tally {
    fn add(&mut self, other: Tally) {
        self.paved += other.paved;
        self.grass += other.grass;
    }

    fn is_empty(&self) -> bool {
        self.paved + self.grass <= TIE_M
    }

    /// Paved if the paved metres are at least the grass metres (a tie goes to paved), grass without any road.
    fn class(&self) -> SurfaceClass {
        if !self.is_empty() && self.paved + TIE_M >= self.grass {
            SurfaceClass::Tarmac
        } else {
            SurfaceClass::Grass
        }
    }
}

/// Whether a road family is paved when its way has no (known) `surface` tag.
fn family_paved(family: Family) -> bool {
    matches!(
        family,
        Family::Asf1 | Family::Asf2 | Family::Asf3 | Family::City
    )
}

/// The road features that can be the ways of `stretch`: same family, some point near its bounds.
fn candidates<'a>(stretch: &Stretch, features: &'a [Feature]) -> Vec<&'a Feature> {
    let (min, max) = bounds(&stretch.points);
    features
        .iter()
        .filter(|f| {
            matches!(f.kind, FeatureKind::Road { family, .. } if family == stretch.family)
                && f.points.iter().any(|p| {
                    p.0 >= min.0 - WAY_M
                        && p.0 <= max.0 + WAY_M
                        && p.1 >= min.1 - WAY_M
                        && p.1 <= max.1 + WAY_M
                })
        })
        .collect()
}

/// What the nearest way says about a place.
enum Tag {
    /// No way lies that near.
    NoWay,
    /// The nearest way has no known `surface` tag.
    Untagged,
    /// The nearest way is tagged.
    Tagged(Paving),
}

/// The `surface` of the nearest of `ways` within [`WAY_M`] of `at`.
fn paving_at(at: Point, ways: &[&Feature]) -> Tag {
    let mut best: Option<(f64, Option<Paving>)> = None;
    for way in ways {
        let FeatureKind::Road { paving, .. } = way.kind else {
            continue;
        };
        let distance = way
            .points
            .windows(2)
            .filter_map(|pair| match pair {
                [a, b] => Some(segment_distance(at, *a, *b)),
                _ => None,
            })
            .fold(f64::INFINITY, f64::min);
        if distance <= WAY_M && best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, paving));
        }
    }
    match best {
        None => Tag::NoWay,
        Some((_, None)) => Tag::Untagged,
        Some((_, Some(paving))) => Tag::Tagged(paving),
    }
}

/// The metres of `line` (a part or the whole of `stretch`) at the places where `keep` is true, paved and grass. With
/// `features` every piece counts by the way it lies on; without, by the stretch's shares.
fn tally_line(
    line: &[Point],
    stretch: &Stretch,
    features: &[Feature],
    keep: &dyn Fn(Point) -> bool,
) -> Tally {
    let mut tally = Tally::default();
    if line.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return tally;
    }
    let total = polyline_length(&stretch.points);
    let (share_paved, share_grass) = if total > EPS_M {
        let paved = (stretch.paved_m / total).clamp(0.0, 1.0);
        let grass = (stretch.unpaved_m / total).clamp(0.0, 1.0 - paved);
        (paved, grass)
    } else {
        (0.0, 0.0)
    };
    let ways = if features.is_empty() {
        Vec::new()
    } else {
        candidates(stretch, features)
    };
    for pair in line.windows(2) {
        let [a, b] = pair else { continue };
        let length = (b.0 - a.0).hypot(b.1 - a.1);
        if length <= 0.0 {
            continue;
        }
        let pieces = (length / PIECE_M).ceil().clamp(1.0, 1.0e6);
        let piece = length / pieces;
        let mut i = 0.0;
        while i < pieces {
            let f = (i + 0.5) / pieces;
            let at = (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
            i += 1.0;
            if !keep(at) {
                continue;
            }
            let by_family = if family_paved(stretch.family) {
                (piece, 0.0)
            } else {
                (0.0, piece)
            };
            let (paved, grass) = if features.is_empty() {
                let rest = 1.0 - share_paved - share_grass;
                (
                    piece * share_paved + by_family.0 * rest,
                    piece * share_grass + by_family.1 * rest,
                )
            } else {
                match paving_at(at, &ways) {
                    Tag::Tagged(Paving::Paved) => (piece, 0.0),
                    Tag::Tagged(Paving::Unpaved) => (0.0, piece),
                    Tag::NoWay | Tag::Untagged => by_family,
                }
            };
            tally.paved += paved;
            tally.grass += grass;
        }
    }
    tally
}

/// The bounds of a lot's outer ring, widened by `by`.
fn lot_bounds(area: &LotArea, by: f64) -> (Point, Point) {
    let (min, max) = bounds(&area.outer);
    ((min.0 - by, min.1 - by), (max.0 + by, max.1 + by))
}

/// The roads whose earthwork touches `area`: every stretch of `graph` that is not in `removed`, a bridge or a tunnel,
/// counted where it lies in the lot or within the reach of its edge.
fn beside(
    area: &LotArea,
    graph: &RoadGraph,
    removed: &BTreeSet<&str>,
    cell_m: f64,
    features: &[Feature],
) -> Tally {
    let mut tally = Tally::default();
    for stretch in &graph.stretches {
        if stretch.bridge || stretch.tunnel || removed.contains(stretch.id.as_str()) {
            continue;
        }
        let reach = core_reach_m(stretch.family.width_m() / 2.0, cell_m) + cell_m;
        let (low, high) = lot_bounds(area, reach);
        let (min, max) = bounds(&stretch.points);
        if !(min.0 <= high.0 && max.0 >= low.0 && min.1 <= high.1 && max.1 >= low.1) {
            continue;
        }
        tally.add(tally_line(&stretch.points, stretch, features, &|p| {
            (p.0 >= low.0 && p.0 <= high.0 && p.1 >= low.1 && p.1 <= high.1)
                && (contains(area, p) || edge_distance(area, p) <= reach)
        }));
    }
    tally
}

/// [`surfaces_with`] without the road features: each metre of a stretch counts by the stretch's own shares of paved,
/// unpaved and untagged metres.
#[must_use]
pub fn surfaces(
    assessed: &[Assessed],
    graph: &RoadGraph,
    clean: &CleanReport,
    cell_m: f64,
) -> Vec<(String, SurfaceClass)> {
    surfaces_with(assessed, graph, clean, cell_m, &[])
}

/// Decides paved or grass for every lot of `assessed` that is not excluded, one per lot in the order of the ids, as
/// described in the module. `graph` is the road graph before the lot step (the clean-up's input), `clean` its report
/// ([`CleanReport::lot_roads`], [`CleanReport::lot_access`]), `cell_m` the terrain cell in metres (for the roads
/// beside a lot; a value that is not a finite number is taken as 0), and `features` the road features the graph was
/// built from: with them the metres inside a lot are counted per way, with an empty list per stretch.
///
/// A lot gets [`SurfaceClass::Tarmac`] if it is paved and [`SurfaceClass::Grass`] if not.
#[must_use]
pub fn surfaces_with(
    assessed: &[Assessed],
    graph: &RoadGraph,
    clean: &CleanReport,
    cell_m: f64,
    features: &[Feature],
) -> Vec<(String, SurfaceClass)> {
    let cell_m = if cell_m.is_finite() && cell_m > 0.0 {
        cell_m
    } else {
        0.0
    };
    let mut lots: Vec<&Assessed> = assessed
        .iter()
        .filter(|a| !matches!(a.outcome, Outcome::Excluded { .. }))
        .collect();
    lots.sort_by(|a, b| a.area.id.cmp(&b.area.id));
    let by_stretch: BTreeMap<&str, &Stretch> =
        graph.stretches.iter().map(|s| (s.id.as_str(), s)).collect();
    let areas: BTreeMap<&str, &LotArea> = assessed
        .iter()
        .map(|a| (a.area.id.as_str(), &a.area))
        .collect();
    // the roads of a lot: what was removed inside it, and the part of an access road that lay inside it
    let mut own: BTreeMap<&str, Tally> = BTreeMap::new();
    for dropped in &clean.lot_roads {
        if let Some(stretch) = by_stretch.get(dropped.stretch.as_str()) {
            let tally = tally_line(&stretch.points, stretch, features, &|_| true);
            own.entry(dropped.lot.as_str()).or_default().add(tally);
        }
    }
    for cut in &clean.lot_access {
        if let (Some(stretch), Some(area)) = (
            by_stretch.get(cut.stretch.as_str()),
            areas.get(cut.lot.as_str()),
        ) {
            let tally = tally_line(&stretch.points, stretch, features, &|p| contains(area, p));
            own.entry(cut.lot.as_str()).or_default().add(tally);
        }
    }
    // the roads of the groups, pooled
    let mut group_of: BTreeMap<&str, usize> = BTreeMap::new();
    let found = groups(assessed);
    let mut pooled: Vec<Tally> = vec![Tally::default(); found.len()];
    for group in &found {
        for member in &group.members {
            group_of.insert(member.as_str(), group.group);
            if let (Some(tally), Some(slot)) =
                (own.get(member.as_str()), pooled.get_mut(group.group))
            {
                slot.add(*tally);
            }
        }
    }
    let removed: BTreeSet<&str> = clean.lot_roads.iter().map(|d| d.stretch.as_str()).collect();
    lots.into_iter()
        .map(|lot| {
            let id = lot.area.id.as_str();
            let class = if let Some(tag) = lot.area.surface_tag {
                match tag {
                    Paving::Paved => SurfaceClass::Tarmac,
                    Paving::Unpaved => SurfaceClass::Grass,
                }
            } else if lot.area.kind == LotKind::CampSite {
                SurfaceClass::Grass
            } else if let Some(tally) = own.get(id).filter(|t| !t.is_empty()) {
                tally.class()
            } else if let Some(tally) = group_of
                .get(id)
                .and_then(|g| pooled.get(*g))
                .filter(|t| !t.is_empty())
            {
                tally.class()
            } else {
                beside(&lot.area, graph, &removed, cell_m, features).class()
            };
            (id.to_string(), class)
        })
        .collect()
}

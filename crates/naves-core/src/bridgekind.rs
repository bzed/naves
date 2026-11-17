// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What an OpenStreetMap bridge is built from, and how far it grows into the road beside it.
//!
//! [`choose`] is the table of the design: a bridge of at most [`FILL_MAX_M`] is plain road on the terrain, a
//! bridge shorter than [`MODEL_FILL`] of every model its road may have is a row of concrete panels, a longer one
//! the shortest model that covers it (a chain of the family's longest model when none does). A part longer than
//! the bridge takes the missing length from the road at both ends, but only along a straight line
//! ([`straight_room`]); where it does not fit there, the next shorter kind is taken. [`fit_bridges`] does this for
//! every bridge of a road graph and says for each what it became and why.
//!
//! All lengths are metres on the map.

use std::f64::consts::{PI, TAU};
use std::fmt;

use crate::catalog::{BridgeModel, Family, MAX_PART_LENGTH_M, PieceCatalog, Slab};
use crate::roadnet::{RoadGraph, Stretch, crossings_of, grow_into, polyline_length, slice_points};
use crate::roadplace::{bridge_for, model_name};

/// A bridge of at most this length is plain road on the terrain (filled), with no finding.
pub const FILL_MAX_M: f64 = 3.0;
/// A model is used for a bridge that is at least this share of the model's length.
pub const MODEL_FILL: f64 = 2.0 / 3.0;
/// The road a bridge grows into may turn by at most this many degrees in total, from the bridge's end on.
pub const GROW_TURN_MAX_DEG: f64 = 5.0;
/// The road a bridge grows into keeps at least this much of its length (so a grown bridge ends this far
/// before a junction, another bridge or a dead end).
pub const GROW_KEEP_M: f64 = 1.0;
/// A model on posts is used only where the original ground under every post is at most this far above the
/// deck level (and within the posts' reach below it).
pub const POST_ABOVE_DECK_M: f64 = 0.5;
/// The original ground along a bridge is sampled every this many metres.
pub const GROUND_STEP_M: f64 = 1.0;
/// Models that are never laid end to end (the design's non-goal: chains of `bridge_wood2`).
pub const UNCHAINED_MODELS: [&str; 1] = ["bridge_wood2"];
/// The most panels in a row, and the most models in a chain.
pub const MAX_PARTS: usize = 1000;

/// Lengths closer than this are equal.
const EPS_M: f64 = 1e-6;

/// What an OpenStreetMap bridge is built from.
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeKind {
    /// Plain road on the terrain.
    Fill,
    /// Concrete panels laid crosswise, `count` of them along the bridge's line, each scaled by `scale`.
    Panels {
        /// How many panels.
        count: usize,
        /// The scale of each panel (at least 1).
        scale: f64,
    },
    /// A bridge model (its path in the catalog), `count` of them end to end.
    Model {
        /// The model's path.
        model: String,
        /// How many end to end.
        count: usize,
    },
}

/// Why a bridge is not built from the part the table gives first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fallback {
    /// The model has no room along the straight road: a panel row of the bridge's own length instead.
    ModelNoRoom {
        /// The model's path.
        model: String,
    },
    /// The panel row has no room along the straight road and lost panels.
    PanelsShortened {
        /// How many panels the bridge's length wanted.
        wanted: usize,
    },
    /// One panel is longer than the bridge and the straight road beside it: it is laid centred on the bridge,
    /// reaching into the road beside it up to [`GROW_KEEP_M`] before a junction.
    OnePanelPastTheStraight,
    /// Not even one panel fits between the junctions: plain road.
    NoRoom,
    /// The install has no panels and no bridge model that fits: plain road.
    NoParts,
}

impl fmt::Display for Fallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fallback::ModelNoRoom { model } => write!(
                f,
                "{} has no room along the straight road",
                model_name(model)
            ),
            Fallback::PanelsShortened { wanted } => write!(
                f,
                "a row of {wanted} panels has no room along the straight road"
            ),
            Fallback::OnePanelPastTheStraight => {
                write!(f, "one panel is longer than the straight road")
            }
            Fallback::NoRoom => write!(f, "not even one panel fits between the junctions"),
            Fallback::NoParts => write!(
                f,
                "the install has no concrete panels and no bridge model that fits"
            ),
        }
    }
}

/// What [`choose`] decided for a bridge.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    /// What the bridge is built from.
    pub kind: BridgeKind,
    /// The length of the part (a model chain or a panel row); 0 for a fill.
    pub length: f64,
    /// How far the part reaches past the bridge's start and past its end along the road. Negative where the
    /// part is shorter than the bridge (it is centred on it).
    pub grown: (f64, f64),
    /// Why it is not the part the table gives first, in the order the steps were taken; empty if it is.
    pub fallback: Vec<Fallback>,
}

impl Choice {
    fn fill(fallback: Vec<Fallback>) -> Choice {
        Choice {
            kind: BridgeKind::Fill,
            length: 0.0,
            grown: (0.0, 0.0),
            fallback,
        }
    }
}

/// An OpenStreetMap bridge as [`choose`] sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeSite {
    /// Its length along its line, before any growth.
    pub length: f64,
    /// The road's surface.
    pub family: Family,
    /// The room along the straight road beyond its start and beyond its end (see [`straight_room`]).
    pub room: (f64, f64),
    /// The room along the road to the next junction, another bridge or a dead end, whatever the road's turns,
    /// [`GROW_KEEP_M`] kept, beyond its start and beyond its end; at least `room`.
    pub reach: (f64, f64),
    /// The original ground along its line and the straight road beside it, as `(along, height)` sorted by
    /// `along`: metres from the bridge's start (negative before it), every [`GROUND_STEP_M`].
    pub ground: Vec<(f64, f64)>,
}

/// `value` if it is a number and not negative, else 0.
fn clean(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

/// The whole number `value`, if it is one from 0 to [`MAX_PARTS`].
fn whole(value: f64) -> Option<usize> {
    if !(value.is_finite() && (0.0..=f64::from(u32::try_from(MAX_PARTS).ok()?)).contains(&value)) {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a whole number from 0 to MAX_PARTS, checked above"
    )]
    Some(value.round() as usize)
}

/// `count` as a length factor.
fn times(count: usize) -> f64 {
    u32::try_from(count).map_or(f64::INFINITY, f64::from)
}

/// Where a part `part` long lies on a bridge `length` long with `room` beyond each end: how far it reaches past
/// the start and past the end. Evenly at both ends; where one side has less room the other takes the rest. A
/// part shorter than the bridge is centred on it. `None` if it does not fit.
fn place(part: f64, length: f64, room: (f64, f64)) -> Option<(f64, f64)> {
    let missing = part - length;
    if !missing.is_finite() {
        return None;
    }
    if missing <= 0.0 {
        return Some((missing / 2.0, missing / 2.0));
    }
    let before = (missing / 2.0).min(room.0);
    let after = (missing - before).min(room.1);
    let before = (missing - after).min(room.0);
    (before + after + EPS_M >= missing).then_some((before, after))
}

/// The original ground at `along` on `ground`, interpolated; beyond its ends the nearest sample; NaN without
/// samples.
fn ground_at(ground: &[(f64, f64)], along: f64) -> f64 {
    let Some(&(first_at, first)) = ground.first() else {
        return f64::NAN;
    };
    if along <= first_at {
        return first;
    }
    for pair in ground.windows(2) {
        if let &[(a, ha), (b, hb)] = pair
            && along <= b
        {
            let t = if b - a > EPS_M {
                (along - a) / (b - a)
            } else {
                0.0
            };
            return ha + (hb - ha) * t;
        }
    }
    ground.last().map_or(f64::NAN, |p| p.1)
}

/// Whether the posts of `count` copies of `model`, laid from `-grown.0` along the bridge, all reach the
/// original ground: under each post the ground is within the model's reach below the deck level (the line
/// between the ground at the two ends of the part) and at most [`POST_ABOVE_DECK_M`] above it. The ground is
/// taken on the bridge's line (not beside it); ground that is not a number reaches nothing.
fn posts_reach(model: &BridgeModel, count: usize, site: &BridgeSite, grown: (f64, f64)) -> bool {
    let start = -grown.0;
    let end = site.length + grown.1;
    let (g0, g1) = (ground_at(&site.ground, start), ground_at(&site.ground, end));
    let deck = |along: f64| {
        if end - start > EPS_M {
            g0 + (g1 - g0) * (along - start) / (end - start)
        } else {
            g0
        }
    };
    let shape = model.shape();
    let reach = shape.reach_down;
    (0..count).all(|k| {
        let offset = start + times(k) * model.length;
        shape.stands().all(|(a0, a1, _, _)| {
            let (from, to) = (offset + a0, offset + a1);
            // the ends of the post's extent and every sample between them
            let inner = site
                .ground
                .iter()
                .filter(|(at, _)| *at > from && *at < to)
                .map(|(at, h)| (*at, *h));
            [
                (from, ground_at(&site.ground, from)),
                (to, ground_at(&site.ground, to)),
            ]
            .into_iter()
            .chain(inner)
            .all(|(at, h)| {
                let below = deck(at) - h;
                below.is_finite() && below <= reach + EPS_M && -below <= POST_ABOVE_DECK_M
            })
        })
    })
}

/// The model of `models` (shortest first) a bridge `length` long gets, and how many end to end, leaving out
/// the paths in `excluded`: the shortest that is at least as long as the bridge, of those the bridge fills to
/// at least [`MODEL_FILL`]; for a bridge longer than all of them, a chain of the longest (a model of
/// [`UNCHAINED_MODELS`] once, where it covers [`MODEL_FILL`] of the bridge).
fn pick<'a>(
    models: &[&'a BridgeModel],
    length: f64,
    excluded: &[String],
) -> Option<(&'a BridgeModel, usize)> {
    let applicable: Vec<&BridgeModel> = models
        .iter()
        .copied()
        .filter(|m| !excluded.contains(&m.model))
        .filter(|m| length + EPS_M >= MODEL_FILL * m.length)
        .collect();
    if let Some(covering) = applicable.iter().find(|m| m.length + EPS_M >= length) {
        return Some((covering, 1));
    }
    let longest = applicable.last()?;
    if UNCHAINED_MODELS.contains(&model_name(&longest.model).as_str()) {
        return (longest.length + EPS_M >= MODEL_FILL * length).then_some((longest, 1));
    }
    let count = whole((length / longest.length - EPS_M).ceil())?.max(1);
    Some((longest, count))
}

/// The width of the road piece of `family`; 0 where it is not known.
fn piece_width(catalog: &PieceCatalog, family: Family) -> f64 {
    clean(catalog.pieces(family).first().map_or(0.0, |p| p.width))
}

/// The panel the rows are laid from: the first by model path of those with a usable size.
fn panel(catalog: &PieceCatalog) -> Option<&Slab> {
    catalog
        .panels()
        .iter()
        .filter(|s| clean(s.along) > 0.0 && clean(s.across) > 0.0)
        .min_by(|a, b| a.model.cmp(&b.model))
}

/// A panel row for `site`, with `fallback` so far: its own length, fewer panels where it has no room, one
/// panel past the straight road where it stays [`GROW_KEEP_M`] before the junctions, else a fill.
fn panel_row(
    catalog: &PieceCatalog,
    site: &BridgeSite,
    room: (f64, f64),
    mut fallback: Vec<Fallback>,
) -> Choice {
    let Some(slab) = panel(catalog) else {
        fallback.push(Fallback::NoParts);
        return Choice::fill(fallback);
    };
    let scale = (piece_width(catalog, site.family) / slab.across).max(1.0);
    let unit = slab.along * scale;
    let length = site.length;
    let wanted = whole((length / unit - EPS_M).ceil())
        .unwrap_or(MAX_PARTS)
        .clamp(1, MAX_PARTS);
    let row = |count: usize, grown: (f64, f64), fallback: Vec<Fallback>| Choice {
        kind: BridgeKind::Panels { count, scale },
        length: times(count) * unit,
        grown,
        fallback,
    };
    if let Some(grown) = place(times(wanted) * unit, length, room) {
        return row(wanted, grown, fallback);
    }
    let fit = whole(((length + room.0 + room.1) / unit + EPS_M).floor())
        .unwrap_or(0)
        .min(wanted);
    if fit >= 1
        && let Some(grown) = place(times(fit) * unit, length, room)
    {
        fallback.push(Fallback::PanelsShortened { wanted });
        return row(fit, grown, fallback);
    }
    let over = (unit - length) / 2.0;
    let reach = (clean(site.reach.0), clean(site.reach.1));
    if over <= reach.0 + EPS_M && over <= reach.1 + EPS_M {
        fallback.push(Fallback::OnePanelPastTheStraight);
        return row(1, (over, over), fallback);
    }
    fallback.push(Fallback::NoRoom);
    Choice::fill(fallback)
}

/// What the bridge `site` is built from, and where it lies: the design's table, then the next shorter kind
/// where the part has no room along the straight road.
///
/// - At most [`FILL_MAX_M`] (or a length that is not a number): [`BridgeKind::Fill`].
/// - A model of [`bridge_for`] the bridge fills to at least [`MODEL_FILL`]: the shortest that is at least as long
///   as the bridge; for a longer bridge a chain of the longest (a model of [`UNCHAINED_MODELS`] alone, where it
///   covers [`MODEL_FILL`] of the bridge). That model, if it fits the straight room; a model on posts only where
///   every post reaches the original ground (within the model's reach below the deck level, the line between the
///   ground at the part's two ends, and at most [`POST_ABOVE_DECK_M`] above it), else the next model. A model
///   without room becomes a panel row of the bridge's length.
/// - Otherwise a panel row: the catalog's first panel, scaled by the road piece's width over the panel's long
///   side (never below 1), as many along the line as cover the bridge; fewer where they have no room, down to
///   one; one panel longer than all the room is laid centred if it stays [`GROW_KEEP_M`] before the
///   junctions; else a fill. Without panels: a fill ([`Fallback::NoParts`]).
#[must_use]
pub fn choose(catalog: &PieceCatalog, site: &BridgeSite) -> Choice {
    let length = site.length;
    if length.is_nan() || length <= FILL_MAX_M {
        return Choice::fill(Vec::new());
    }
    let room = (clean(site.room.0), clean(site.room.1));
    let models = bridge_for(catalog, site.family);
    let mut excluded: Vec<String> = Vec::new();
    while let Some((model, count)) = pick(&models, length, &excluded) {
        let part = times(count) * model.length;
        let Some(grown) = place(part, length, room) else {
            return panel_row(
                catalog,
                site,
                room,
                vec![Fallback::ModelNoRoom {
                    model: model.model.clone(),
                }],
            );
        };
        if model.posts() && !posts_reach(model, count, site, grown) {
            excluded.push(model.model.clone());
            continue;
        }
        return Choice {
            kind: BridgeKind::Model {
                model: model.model.clone(),
                count,
            },
            length: part,
            grown,
            fallback: Vec::new(),
        };
    }
    panel_row(catalog, site, room, Vec::new())
}

/// What [`choose`] gives: only the kind.
#[must_use]
pub fn bridge_kind(catalog: &PieceCatalog, site: &BridgeSite) -> BridgeKind {
    choose(catalog, site).kind
}

/// The stretch that continues stretch `i` at its start or end and that a bridge may grow into: the only other
/// one at a crossing of exactly two stretches, if it is plain road (not a bridge, not one about to be, not a
/// tunnel, not a loop).
fn continuation(graph: &RoadGraph, i: usize, at_start: bool) -> Option<usize> {
    let this = graph.stretches.get(i)?;
    let node = if at_start { this.start } else { this.end };
    let crossing = graph.crossings.iter().find(|c| c.node == node)?;
    let &[first, second] = crossing.stretches.as_slice() else {
        return None;
    };
    let other = if first == i { second } else { first };
    let next = graph.stretches.get(other)?;
    (other != i
        && !next.bridge
        && next.bridge_kind.is_none()
        && !next.tunnel
        && next.start != next.end)
        .then_some(other)
}

/// The heading of the way from `a` to `b`, in radians; `None` for no way or numbers that are not.
fn heading(a: (f64, f64), b: (f64, f64)) -> Option<f64> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let h = dx.atan2(dy);
    (dx.hypot(dy) > EPS_M && h.is_finite()).then_some(h)
}

/// The turn from heading `a` to heading `b`, in degrees, 0 to 180.
fn turn_deg(a: f64, b: f64) -> f64 {
    let d = (b - a).rem_euclid(TAU);
    (if d > PI { TAU - d } else { d }).to_degrees()
}

/// The road beyond one end of a bridge: the neighbour's points from the bridge's end outwards, how far it is
/// straight, and the total turn reached at each of its points.
struct End {
    /// The neighbour, by index.
    stretch: usize,
    /// Its points from the bridge's end outwards.
    points: Vec<(f64, f64)>,
    /// Per point after the first: the distance from the bridge's end and the total turn up to there, in
    /// degrees, from the bridge's own heading at its end on.
    turns: Vec<(f64, f64)>,
    /// How far from the bridge's end the road stays within [`GROW_TURN_MAX_DEG`] of turn.
    straight: f64,
    /// The neighbour's length.
    length: f64,
}

impl End {
    /// The straight room: [`End::straight`], with [`GROW_KEEP_M`] of the neighbour kept.
    fn room(&self) -> f64 {
        clean(self.straight.min(self.length - GROW_KEEP_M))
    }

    /// The room whatever the turns.
    fn reach(&self) -> f64 {
        clean(self.length - GROW_KEEP_M)
    }

    /// The total turn of the first `d` metres of the road, in degrees.
    fn turn_over(&self, d: f64) -> f64 {
        let mut turn = 0.0;
        let mut from = 0.0;
        for &(to, total) in &self.turns {
            if from >= d - EPS_M {
                break;
            }
            turn = total;
            from = to;
        }
        turn
    }
}

/// The road beyond the start (`at_start`) or end of bridge stretch `i`, unless `other_than` is the neighbour
/// there (a stretch whose two ends meet one neighbour grows into it once).
fn end_of(graph: &RoadGraph, i: usize, at_start: bool, other_than: Option<usize>) -> Option<End> {
    let bridge = graph.stretches.get(i)?;
    let n = continuation(graph, i, at_start).filter(|n| Some(*n) != other_than)?;
    let neighbour = graph.stretches.get(n)?;
    let node = if at_start { bridge.start } else { bridge.end };
    let mut points = neighbour.points.clone();
    if neighbour.start != node {
        points.reverse();
    }
    let at = *points.first()?;
    // the bridge's heading out of its end: from its last point apart from the end to the end
    let own: Vec<(f64, f64)> = if at_start {
        bridge.points.clone()
    } else {
        bridge.points.iter().rev().copied().collect()
    };
    let out = own
        .iter()
        .skip(1)
        .find_map(|p| heading(*p, at))
        .or_else(|| {
            points
                .windows(2)
                .find_map(|w| heading(*w.first()?, *w.get(1)?))
        });
    let mut turns = Vec::new();
    let (mut total, mut along, mut previous) = (0.0, 0.0, out);
    let mut straight = None;
    for w in points.windows(2) {
        let &[a, b] = w else { continue };
        let seg = (b.0 - a.0).hypot(b.1 - a.1);
        if !seg.is_finite() {
            straight.get_or_insert(along);
            break;
        }
        if let Some(h) = heading(a, b) {
            total += previous.map_or(0.0, |p| turn_deg(p, h));
            previous = Some(h);
        }
        if total > GROW_TURN_MAX_DEG + 1e-9 {
            straight.get_or_insert(along);
        }
        along += seg;
        turns.push((along, total));
    }
    let length = polyline_length(&neighbour.points);
    Some(End {
        stretch: n,
        points,
        turns,
        straight: straight.unwrap_or(along),
        length: if length.is_finite() { length } else { 0.0 },
    })
}

/// The ends of bridge stretch `i`: the road beyond its start and beyond its end.
fn ends_of(graph: &RoadGraph, i: usize) -> (Option<End>, Option<End>) {
    let before = end_of(graph, i, true, None);
    let after = end_of(graph, i, false, before.as_ref().map(|e| e.stretch));
    (before, after)
}

/// How far a bridge on stretch `i` may grow beyond its start and beyond its end: along the stretch that
/// continues it there (at a crossing of exactly two stretches, plain road), as long as that road turns by at
/// most [`GROW_TURN_MAX_DEG`] in total from the bridge's own heading at its end on, keeping [`GROW_KEEP_M`] of
/// it. 0 at a junction, a dead end, another bridge or a stretch not in the graph.
#[must_use]
pub fn straight_room(graph: &RoadGraph, stretch: usize) -> (f64, f64) {
    let (before, after) = ends_of(graph, stretch);
    (
        before.as_ref().map_or(0.0, End::room),
        after.as_ref().map_or(0.0, End::room),
    )
}

/// The point `d` metres along `points` (the last point beyond its end, the first before its start).
fn point_at(points: &[(f64, f64)], d: f64) -> Option<(f64, f64)> {
    let total = polyline_length(points);
    let d = d.clamp(0.0, total.max(0.0));
    slice_points(points, d, total.max(d))
        .first()
        .copied()
        .or_else(|| points.last().copied())
}

/// The original ground along bridge stretch `i` and `room` of the road beyond its ends (at most
/// [`MAX_PART_LENGTH_M`], the most a part can reach past a bridge), every [`GROUND_STEP_M`].
fn ground_along(
    graph: &RoadGraph,
    i: usize,
    ends: (&Option<End>, &Option<End>),
    room: (f64, f64),
    ground: &dyn Fn((f64, f64)) -> f64,
) -> Vec<(f64, f64)> {
    let Some(bridge) = graph.stretches.get(i) else {
        return Vec::new();
    };
    let (before, after) = (room.0.min(MAX_PART_LENGTH_M), room.1.min(MAX_PART_LENGTH_M));
    let mut line: Vec<(f64, f64)> = Vec::new();
    if let Some(end) = ends.0 {
        let mut part = slice_points(&end.points, 0.0, before);
        part.reverse();
        line.extend(part);
    }
    line.extend(bridge.points.iter().copied());
    if let Some(end) = ends.1 {
        line.extend(slice_points(&end.points, 0.0, after));
    }
    line.dedup();
    let total = polyline_length(&line);
    if !total.is_finite() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut d = 0.0_f64;
    loop {
        let at = d.min(total);
        if let Some(p) = point_at(&line, at) {
            out.push((at - before, ground(p)));
        }
        if at >= total {
            break;
        }
        d += GROUND_STEP_M;
    }
    out
}

/// What an OpenStreetMap bridge became, for the build's report.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeFit {
    /// The OpenStreetMap bridge stretches it is made of, in order along it: more than one where bridge ways
    /// follow each other (they are one bridge).
    pub stretches: Vec<String>,
    /// Its stretch in the graph [`fit_bridges`] returns.
    pub stretch: String,
    /// The middle of its line, in map metres.
    pub at: (f64, f64),
    /// The road's surface.
    pub family: Family,
    /// Its length before any growth.
    pub length: f64,
    /// What it is built from.
    pub kind: BridgeKind,
    /// The length of the part; 0 for a fill.
    pub part_length: f64,
    /// How far the part reaches past the bridge's start and end (negative: it ends inside the bridge).
    pub grown: (f64, f64),
    /// The total turn of the road it grew into at its start and at its end, in degrees.
    pub turn_deg: (f64, f64),
    /// Why it is not the part the table gives first.
    pub fallback: Vec<Fallback>,
}

/// Whether stretch `s` can be joined with a bridge that follows it.
fn joinable(s: &Stretch) -> bool {
    s.bridge && !s.tunnel && !s.roundabout && s.start != s.end && s.bridge_kind.is_none()
}

/// `graph` with bridge stretches that follow each other (joined at nodes of exactly two stretches, both
/// bridges) made one stretch, and for each stretch of the result that is a bridge the ids it was made of.
///
/// The joined stretch has the id of the lower of its two outer stretches, the surface of its longest part (the
/// first among equals), the highest layer and is drivable if any part is; it runs from the lower of its two
/// outer nodes. A ring of bridges is left as it is.
fn join_bridges(graph: &RoadGraph) -> (RoadGraph, Vec<(String, Vec<String>)>) {
    let n = graph.stretches.len();
    let mut links: Vec<Vec<(usize, i64)>> = vec![Vec::new(); n];
    for c in &graph.crossings {
        if let &[a, b] = c.stretches.as_slice()
            && a != b
            && graph.stretches.get(a).is_some_and(joinable)
            && graph.stretches.get(b).is_some_and(joinable)
        {
            if let Some(l) = links.get_mut(a) {
                l.push((b, c.node));
            }
            if let Some(l) = links.get_mut(b) {
                l.push((a, c.node));
            }
        }
    }
    let mut used = vec![false; n];
    let mut out: Vec<Stretch> = Vec::new();
    let mut members: Vec<(String, Vec<String>)> = Vec::new();
    // walk each chain from its end with the lower id; stretches are in the order of their ids
    for i in 0..n {
        let Some(s) = graph.stretches.get(i) else {
            continue;
        };
        if used.get(i).copied().unwrap_or(true) {
            continue;
        }
        let degree = links.get(i).map_or(0, Vec::len);
        if degree >= 2 {
            // inside a chain, or on a ring: taken when its chain is walked; a ring stays as it is
            continue;
        }
        if let Some(u) = used.get_mut(i) {
            *u = true;
        }
        let mut chain = vec![i];
        let mut previous = i;
        let mut at = i;
        while let Some(&(next, _)) = links
            .get(at)
            .and_then(|l| l.iter().find(|(o, _)| *o != previous && *o != at))
        {
            if used.get(next).copied().unwrap_or(true) {
                break;
            }
            if let Some(u) = used.get_mut(next) {
                *u = true;
            }
            chain.push(next);
            previous = at;
            at = next;
        }
        if chain.len() == 1 {
            if s.bridge {
                members.push((s.id.clone(), vec![s.id.clone()]));
            }
            out.push(s.clone());
            continue;
        }
        let Some(joined) = join(graph, &chain) else {
            continue;
        };
        members.push((
            joined.id.clone(),
            chain
                .iter()
                .filter_map(|&k| graph.stretches.get(k).map(|s| s.id.clone()))
                .collect(),
        ));
        out.push(joined);
    }
    // the rest: stretches on a ring of bridges
    for (i, s) in graph.stretches.iter().enumerate() {
        if !used.get(i).copied().unwrap_or(true) {
            if s.bridge {
                members.push((s.id.clone(), vec![s.id.clone()]));
            }
            out.push(s.clone());
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    members.sort_by(|a, b| a.0.cmp(&b.0));
    let crossings = crossings_of(&out);
    (
        RoadGraph {
            stretches: out,
            crossings,
        },
        members,
    )
}

/// The stretches `chain` (in order, each meeting the next at a node) as one stretch; `None` for no stretches.
fn join(graph: &RoadGraph, chain: &[usize]) -> Option<Stretch> {
    let parts: Vec<&Stretch> = chain
        .iter()
        .filter_map(|&k| graph.stretches.get(k))
        .collect();
    let first = parts.first()?;
    // the node where the first part meets the second: the first part runs towards it
    let shared = parts.get(1).map_or(first.end, |second| {
        if first.end == second.start || first.end == second.end {
            first.end
        } else {
            first.start
        }
    });
    let mut node = if first.end == shared {
        first.start
    } else {
        first.end
    };
    let start = node;
    let mut points: Vec<(f64, f64)> = Vec::new();
    let mut longest: Option<(f64, Family)> = None;
    for part in &parts {
        let mut p = part.points.clone();
        let next = if part.start == node {
            part.end
        } else {
            p.reverse();
            part.start
        };
        if !p.is_empty() && points.last() == p.first() {
            p.remove(0);
        }
        points.extend(p);
        node = next;
        let len = polyline_length(&part.points);
        if longest.is_none_or(|(l, _)| len > l + EPS_M) {
            longest = Some((len, part.family));
        }
    }
    let end = node;
    let mut joined = Stretch {
        id: parts.iter().map(|p| p.id.clone()).min().unwrap_or_default(),
        family: longest.map_or(first.family, |l| l.1),
        drivable: parts.iter().any(|p| p.drivable),
        layer: parts.iter().map(|p| p.layer).max().unwrap_or(first.layer),
        points,
        start,
        end,
        ..(*first).clone()
    };
    if joined.start > joined.end {
        joined.points.reverse();
        std::mem::swap(&mut joined.start, &mut joined.end);
    }
    Some(joined)
}

/// Every OpenStreetMap bridge of `graph` built from the part `choose` gives it, grown along the straight road
/// as far as the part needs, and for every bridge what it became and why.
///
/// Bridge stretches that follow each other are first made one stretch ([`BridgeFit::stretches`] names them).
/// Then, in the order of the stretches, each bridge's [`BridgeSite`] is measured (its length, [`straight_room`],
/// the room to the next junction and the original ground from `ground`, a height at a map position) and
/// `choose` decides; the part's growth past each end (where it is positive) is moved from the road beside it
/// to the bridge, so a later bridge sees the road that is left. The nodes stay where they are in the graph,
/// only their points move. Every stretch that was a bridge gets its [`Stretch::bridge_kind`].
///
/// Until panel rows are laid, a bridge built from panels or filled is no bridge in the result (`bridge` is
/// false); one built from a model stays a bridge.
#[must_use]
pub fn fit_bridges(
    graph: &RoadGraph,
    choose: &dyn Fn(&BridgeSite) -> Choice,
    ground: &dyn Fn((f64, f64)) -> f64,
) -> (RoadGraph, Vec<BridgeFit>) {
    let (mut g, members) = join_bridges(graph);
    let mut fits = Vec::new();
    for i in 0..g.stretches.len() {
        let Some(s) = g.stretches.get(i) else {
            continue;
        };
        if !s.bridge || s.tunnel || s.bridge_kind.is_some() {
            continue;
        }
        let length = polyline_length(&s.points);
        let length = if length.is_finite() { length } else { f64::NAN };
        let (before, after) = ends_of(&g, i);
        let room = (
            before.as_ref().map_or(0.0, End::room),
            after.as_ref().map_or(0.0, End::room),
        );
        let site = BridgeSite {
            length,
            family: s.family,
            room,
            reach: (
                before.as_ref().map_or(0.0, End::reach),
                after.as_ref().map_or(0.0, End::reach),
            ),
            ground: ground_along(&g, i, (&before, &after), room, ground),
        };
        let at = point_at(&s.points, length / 2.0).unwrap_or_default();
        let (id, family) = (s.id.clone(), s.family);
        let mut choice = choose(&site);
        let grows = !matches!(choice.kind, BridgeKind::Fill);
        // whatever `choose` says, a part never reaches closer than GROW_KEEP_M to the next junction
        choice.grown = (
            choice.grown.0.min(site.reach.0),
            choice.grown.1.min(site.reach.1),
        );
        let turn = |end: &Option<End>, d: f64| end.as_ref().map_or(0.0, |e| e.turn_over(d));
        let turn_deg = if grows {
            (turn(&before, choice.grown.0), turn(&after, choice.grown.1))
        } else {
            (0.0, 0.0)
        };
        if grows {
            if let Some(end) = &before {
                grow_into(&mut g, i, true, end.stretch, choice.grown.0);
            }
            if let Some(end) = &after {
                grow_into(&mut g, i, false, end.stretch, choice.grown.1);
            }
        }
        if let Some(s) = g.stretches.get_mut(i) {
            // until panel rows are laid: only a model is a bridge
            s.bridge = matches!(choice.kind, BridgeKind::Model { .. });
            s.bridge_kind = Some(choice.kind.clone());
        }
        g.crossings = crossings_of(&g.stretches);
        fits.push(BridgeFit {
            stretches: members
                .iter()
                .find(|(m, _)| *m == id)
                .map_or_else(|| vec![id.clone()], |(_, ids)| ids.clone()),
            stretch: id,
            at,
            family,
            length,
            kind: choice.kind,
            part_length: choice.length,
            grown: if grows { choice.grown } else { (0.0, 0.0) },
            turn_deg,
            fallback: choice.fallback,
        });
    }
    (g, fits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_takes_the_missing_length_evenly_or_from_the_side_with_room() {
        assert_eq!(place(10.0, 6.0, (5.0, 5.0)), Some((2.0, 2.0)));
        assert_eq!(place(10.0, 6.0, (0.5, 5.0)), Some((0.5, 3.5)));
        assert_eq!(place(10.0, 6.0, (5.0, 1.0)), Some((3.0, 1.0)));
        assert_eq!(place(10.0, 6.0, (1.0, 1.0)), None);
        assert_eq!(place(4.0, 6.0, (0.0, 0.0)), Some((-1.0, -1.0)));
        assert_eq!(place(f64::NAN, 6.0, (1.0, 1.0)), None);
    }

    #[test]
    fn the_ground_is_interpolated_and_held_beyond_its_ends() {
        let g = [(-1.0, 10.0), (0.0, 12.0), (1.0, 11.0)];
        assert!((ground_at(&g, -0.5) - 11.0).abs() < 1e-12);
        assert!((ground_at(&g, 0.5) - 11.5).abs() < 1e-12);
        assert!((ground_at(&g, -5.0) - 10.0).abs() < 1e-12);
        assert!((ground_at(&g, 5.0) - 11.0).abs() < 1e-12);
        assert!(ground_at(&[], 0.0).is_nan());
    }

    #[test]
    fn turns_are_measured_the_short_way_round() {
        assert!((turn_deg(0.0, 0.1_f64.to_radians()) - 0.1).abs() < 1e-9);
        assert!((turn_deg(3.1, -3.1) - (TAU - 6.2).to_degrees()).abs() < 1e-9);
        assert!((turn_deg(0.0, PI) - 180.0).abs() < 1e-9);
        assert_eq!(heading((0.0, 0.0), (0.0, 0.0)), None);
        assert_eq!(heading((0.0, 0.0), (f64::NAN, 1.0)), None);
    }

    #[test]
    fn whole_numbers_are_bounded() {
        assert_eq!(whole(3.0), Some(3));
        assert_eq!(whole(-1.0), None);
        assert_eq!(whole(f64::NAN), None);
        assert_eq!(whole(1e9), None);
        assert!(times(usize::MAX).is_infinite());
    }
}

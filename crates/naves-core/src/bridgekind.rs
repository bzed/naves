// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What an OpenStreetMap bridge is built from, and how far it grows into the road beside it.
//!
//! [`choose`] is the table of the design: a bridge of at most [`FILL_MAX_M`] is plain road on the terrain, a
//! bridge shorter than [`MODEL_FILL`] of every model its road may have is a row of concrete panels, a longer one
//! the shortest model that covers it (a chain of the family's longest model when none does). A panel row covers
//! its bridge exactly and never grows (its panels are scaled to the bridge, [`panel_row_size`]). A model longer
//! than the bridge takes the missing length from the road at both ends, but only along a straight line
//! ([`straight_room`]) and only on a bridge that is itself straight; where it does not fit, the bridge is a
//! panel row. [`fit_bridges`] does this for every bridge of a road graph and says for each what it became and
//! why.
//!
//! All lengths are metres on the map.

use std::f64::consts::{PI, TAU};
use std::fmt;

use crate::catalog::{BridgeModel, Family, PieceCatalog, Slab};
use crate::roadnet::{
    RoadGraph, Stretch, built_as_bridge, crossings_of, grow_into, polyline_length, slice_points,
};
use crate::roadplace::{BRIDGE_NARROWER_M, bridge_for, model_name};

/// A bridge of at most this length is plain road on the terrain (filled), with no finding; not where it
/// touches a node where only bridges meet.
pub const FILL_MAX_M: f64 = 3.0;
/// A model is used for a bridge that is at least this share of the model's length.
pub const MODEL_FILL: f64 = 2.0 / 3.0;
/// A model and the road it grows into may turn by at most this many degrees in total: the bridge's own line
/// and the grown parts at both ends together.
pub const GROW_TURN_MAX_DEG: f64 = 5.0;
/// The road a bridge grows into keeps at least this much of its length (so a grown bridge ends this far
/// before a junction, another bridge or a dead end).
pub const GROW_KEEP_M: f64 = 1.0;
/// A model on posts is used only where the original ground under every post is at most this far above the
/// deck level (and within the posts' reach below it).
pub const POST_ABOVE_DECK_M: f64 = 0.5;
/// The original ground under a post is sampled every this many metres, along and across.
pub const GROUND_STEP_M: f64 = 1.0;
/// Models that are never laid end to end (the design's non-goal: chains of `bridge_wood2`).
pub const UNCHAINED_MODELS: [&str; 1] = ["bridge_wood2"];
/// The most panels in a row, and the most models in a chain (a guard against absurd input).
pub const MAX_PARTS: usize = 1000;

/// Lengths closer than this are equal.
const EPS_M: f64 = 1e-6;
/// The most ground samples along or across one post.
const MAX_POST_SAMPLES: usize = 64;

/// What an OpenStreetMap bridge is built from.
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeKind {
    /// Plain road on the terrain.
    Fill,
    /// Concrete panels laid crosswise, one after the other along the bridge's own line from its start to its
    /// end, each on the line's heading there: `count` of them, each scaled by `scale` (so that they cover the
    /// bridge exactly).
    Panels {
        /// How many panels.
        count: usize,
        /// The scale of each panel.
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
#[derive(Debug, Clone, PartialEq)]
pub enum Fallback {
    /// The model has no room along the straight road: a panel row instead.
    ModelNoRoom {
        /// The model's path.
        model: String,
    },
    /// The bridge, with the road the model would grow into, turns by more than [`GROW_TURN_MAX_DEG`]: a panel
    /// row instead.
    ModelBends {
        /// The model's path.
        model: String,
        /// The total turn, in degrees.
        deg: f64,
    },
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
            Fallback::ModelBends { model, deg } => write!(
                f,
                "{} would lie on a line that turns by {deg:.1} degrees",
                model_name(model)
            ),
            Fallback::NoParts => write!(
                f,
                "the install has no concrete panels and no bridge model that fits"
            ),
        }
    }
}

/// Why the room beyond one end of a bridge ends where it does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RoomEnd {
    /// The end is at a crossing of three or more stretches.
    Junction,
    /// The end is at a node where only bridges meet: the bridge reaches the node itself.
    BridgeNode,
    /// The stretch beyond the end is another bridge.
    Bridge,
    /// The end is a dead end.
    DeadEnd,
    /// The road beyond turns by more than [`GROW_TURN_MAX_DEG`] at the end itself.
    KinkAtJoint {
        /// The turn there, in degrees.
        deg: f64,
    },
    /// The road beyond turns by more than [`GROW_TURN_MAX_DEG`] in total after `m` metres.
    TurnAfter {
        /// How far the road is straight enough.
        m: f64,
        /// The total turn where it ends.
        deg: f64,
    },
    /// The stretch beyond ends (it keeps [`GROW_KEEP_M`]).
    ShortNeighbour,
    /// The stretch beyond is a tunnel.
    Tunnel,
}

impl fmt::Display for RoomEnd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RoomEnd::Junction => write!(f, "junction"),
            RoomEnd::BridgeNode => write!(f, "node of bridges"),
            RoomEnd::Bridge => write!(f, "another bridge"),
            RoomEnd::DeadEnd => write!(f, "dead end"),
            RoomEnd::KinkAtJoint { deg } => write!(f, "turn of {deg:.1} degrees at the end"),
            RoomEnd::TurnAfter { m, deg } => {
                write!(f, "turn of {deg:.1} degrees after {m:.1} m")
            }
            RoomEnd::ShortNeighbour => write!(f, "the road beyond ends"),
            RoomEnd::Tunnel => write!(f, "tunnel"),
        }
    }
}

/// What [`choose`] decided for a bridge.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    /// What the bridge is built from.
    pub kind: BridgeKind,
    /// The length of the part (a model chain, or the bridge's own length for a panel row); 0 for a fill.
    pub length: f64,
    /// How far the part reaches past the bridge's start and past its end along the road: 0 for a panel row,
    /// negative where a model is shorter than the bridge (it is centred on it).
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

/// The total turn of the road beyond an end of a bridge at each of its points, as `(metres from the end,
/// degrees)` in order.
pub type Bends = Vec<(f64, f64)>;

/// An OpenStreetMap bridge as [`choose`] sees it.
pub struct BridgeSite<'a> {
    /// Its length along its line, before any growth.
    pub length: f64,
    /// The road's surface.
    pub family: Family,
    /// The room along the straight road beyond its start and beyond its end (see [`straight_room`]).
    pub room: (f64, f64),
    /// The total turn of its own line, in degrees (the sum of the turns at its points).
    pub turn_deg: f64,
    /// Per end (start, end): the total turn of the road beyond it, from the bridge's own heading at that end
    /// on, at each point of that road, as `(metres from the end, degrees)` in order.
    pub bends: (Bends, Bends),
    /// Whether one of its ends is a node where only bridges meet.
    pub at_bridge_node: bool,
    /// The original ground at `(along, across)`: metres along its line from its start (negative before it,
    /// beyond its length after it, on the road it would grow into) and metres to the left of the line.
    pub ground: &'a dyn Fn(f64, f64) -> f64,
}

impl fmt::Debug for BridgeSite<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BridgeSite")
            .field("length", &self.length)
            .field("family", &self.family)
            .field("room", &self.room)
            .field("turn_deg", &self.turn_deg)
            .field("bends", &self.bends)
            .field("at_bridge_node", &self.at_bridge_node)
            .finish_non_exhaustive()
    }
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

/// Where a model chain `part` long lies on a bridge `length` long with `room` beyond each end: how far it
/// reaches past the start and past the end. Evenly at both ends; where one side has less room the other takes
/// the rest. A part shorter than the bridge is centred on it. `None` if it does not fit.
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

/// The total turn of the first `d` metres of the road beyond an end (`bends`, see [`BridgeSite::bends`]), in
/// degrees; 0 for no growth.
fn turn_over(bends: &[(f64, f64)], d: f64) -> f64 {
    let mut turn = 0.0;
    let mut from = 0.0;
    for &(to, total) in bends {
        if from >= d - EPS_M {
            break;
        }
        turn = total;
        from = to;
    }
    turn
}

/// `from`, `to` and the points every [`GROUND_STEP_M`] between them (at most [`MAX_POST_SAMPLES`]).
fn samples(from: f64, to: f64) -> Vec<f64> {
    let mut out = vec![from, to];
    let mut at = from + GROUND_STEP_M;
    while at < to && out.len() < MAX_POST_SAMPLES {
        out.push(at);
        at += GROUND_STEP_M;
    }
    out
}

/// Whether the posts of `count` copies of `model`, laid from `-grown.0` along the bridge, all reach the
/// original ground: under every standing rectangle of the model ([`crate::catalog::PartShape::stands`]), at
/// its corners and every [`GROUND_STEP_M`] along and across it, the ground is within the model's reach below
/// the deck level (the line between the ground at the two ends of the part, on the bridge's line) and at most
/// [`POST_ABOVE_DECK_M`] above it. Ground that is not a number reaches nothing.
fn posts_reach(model: &BridgeModel, count: usize, site: &BridgeSite, grown: (f64, f64)) -> bool {
    let start = -grown.0;
    let end = site.length + grown.1;
    let ground = site.ground;
    let (g0, g1) = (ground(start, 0.0), ground(end, 0.0));
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
        shape.stands().all(|(a0, a1, c0, c1)| {
            samples(offset + a0, offset + a1).into_iter().all(|along| {
                samples(c0, c1).into_iter().all(|across| {
                    let below = deck(along) - ground(along, across);
                    below.is_finite() && below <= reach + EPS_M && -below <= POST_ABOVE_DECK_M
                })
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

/// How many panels of `slab` a row over a bridge `length` long on a road `road_width` wide has, and their
/// scale, so that they cover the bridge exactly: the count nearest the nominal scale
/// `max(1, road_width / slab.across)` (at least one), or one fewer where that row would be more than
/// [`BRIDGE_NARROWER_M`] narrower than the road. A row wider than the road is always fine. A road width that is
/// not a number counts as 0; at most [`MAX_PARTS`] panels.
#[must_use]
pub fn panel_row_size(length: f64, road_width: f64, slab: &Slab) -> (usize, f64) {
    let width = clean(road_width);
    let nominal = (width / slab.across).max(1.0);
    let n0 = whole((length / (slab.along * nominal)).round())
        .unwrap_or(MAX_PARTS)
        .clamp(1, MAX_PARTS);
    let scale = |n: usize| length / (slab.along * times(n));
    let narrowest = (width - BRIDGE_NARROWER_M) / slab.across;
    let n = if n0 > 1 && scale(n0) < narrowest - 1e-12 {
        n0 - 1
    } else {
        n0
    };
    (n, scale(n))
}

/// A panel row over `site`, with `fallback` so far: it covers the bridge exactly (see [`panel_row_size`]) and
/// needs no room; without panels in the catalog a fill.
fn panel_row(catalog: &PieceCatalog, site: &BridgeSite, mut fallback: Vec<Fallback>) -> Choice {
    let Some(slab) = panel(catalog) else {
        fallback.push(Fallback::NoParts);
        return Choice::fill(fallback);
    };
    let (count, scale) = panel_row_size(site.length, piece_width(catalog, site.family), slab);
    Choice {
        kind: BridgeKind::Panels { count, scale },
        length: site.length,
        grown: (0.0, 0.0),
        fallback,
    }
}

/// What the bridge `site` is built from, and where it lies: the design's table, then a panel row where a
/// model does not fit.
///
/// - A length that is not a number or not positive, or at most [`FILL_MAX_M`] unless the bridge touches a
///   node where only bridges meet: [`BridgeKind::Fill`].
/// - A model of [`bridge_for`] the bridge fills to at least [`MODEL_FILL`]: the shortest that is at least as long
///   as the bridge; for a longer bridge a chain of the longest (a model of [`UNCHAINED_MODELS`] alone, where it
///   covers [`MODEL_FILL`] of the bridge). That model, if it fits the straight room and the bridge's own turn
///   and the turn of the grown parts are at most [`GROW_TURN_MAX_DEG`] together; a model on posts only where
///   every post reaches the original ground (within the model's reach below the deck level, the line between
///   the ground at the part's two ends, and at most [`POST_ABOVE_DECK_M`] above it), else the next model. A
///   model without room, or on a line that turns too much, becomes a panel row.
/// - Otherwise a panel row of the bridge's own length (see [`panel_row_size`]). Without panels: a fill
///   ([`Fallback::NoParts`]).
#[must_use]
pub fn choose(catalog: &PieceCatalog, site: &BridgeSite) -> Choice {
    let length = site.length;
    if length.is_nan() || length <= EPS_M || (length <= FILL_MAX_M && !site.at_bridge_node) {
        return Choice::fill(Vec::new());
    }
    if !length.is_finite() {
        return Choice::fill(Vec::new());
    }
    let room = (clean(site.room.0), clean(site.room.1));
    let models = bridge_for(catalog, site.family);
    let mut excluded: Vec<String> = Vec::new();
    while let Some((model, count)) = pick(&models, length, &excluded) {
        let part = times(count) * model.length;
        let Some(grown) = place(part, length, room) else {
            let fallback = vec![Fallback::ModelNoRoom {
                model: model.model.clone(),
            }];
            return panel_row(catalog, site, fallback);
        };
        let turn = clean(site.turn_deg)
            + turn_over(&site.bends.0, grown.0)
            + turn_over(&site.bends.1, grown.1);
        if turn.is_nan() || turn > GROW_TURN_MAX_DEG + 1e-9 || !site.turn_deg.is_finite() {
            let fallback = vec![Fallback::ModelBends {
                model: model.model.clone(),
                deg: if site.turn_deg.is_finite() {
                    turn
                } else {
                    f64::INFINITY
                },
            }];
            return panel_row(catalog, site, fallback);
        }
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
    panel_row(catalog, site, Vec::new())
}

/// What [`choose`] gives: only the kind.
#[must_use]
pub fn bridge_kind(catalog: &PieceCatalog, site: &BridgeSite) -> BridgeKind {
    choose(catalog, site).kind
}

/// Whether `s` is a bridge for the room of another: a bridge, or an OpenStreetMap bridge built from its own part
/// (a filled one is plain road).
fn is_bridge(s: &Stretch) -> bool {
    s.bridge || built_as_bridge(s)
}

/// The stretch that continues stretch `i` at its start or end and that a bridge may grow into: the only other
/// one at a crossing of exactly two stretches, if it is plain road (not a bridge, not a tunnel, not a loop);
/// else why there is none.
fn continuation(graph: &RoadGraph, i: usize, at_start: bool) -> Result<usize, RoomEnd> {
    let this = graph.stretches.get(i).ok_or(RoomEnd::DeadEnd)?;
    let node = if at_start { this.start } else { this.end };
    let crossing = graph
        .crossings
        .iter()
        .find(|c| c.node == node)
        .ok_or(RoomEnd::DeadEnd)?;
    match crossing.stretches.as_slice() {
        [] | [_] => Err(RoomEnd::DeadEnd),
        &[first, second] => {
            let other = if first == i { second } else { first };
            let next = graph.stretches.get(other).ok_or(RoomEnd::DeadEnd)?;
            if is_bridge(next) {
                Err(RoomEnd::Bridge)
            } else if next.tunnel {
                Err(RoomEnd::Tunnel)
            } else if other == i || next.start == next.end {
                Err(RoomEnd::Junction)
            } else {
                Ok(other)
            }
        }
        many => {
            let all_bridges = many
                .iter()
                .all(|&k| graph.stretches.get(k).is_some_and(is_bridge));
            Err(if all_bridges {
                RoomEnd::BridgeNode
            } else {
                RoomEnd::Junction
            })
        }
    }
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

/// The total turn of the line `points`: the sum of the turns at its points, in degrees.
fn line_turn(points: &[(f64, f64)]) -> f64 {
    let mut total = 0.0;
    let mut previous: Option<f64> = None;
    for w in points.windows(2) {
        let &[a, b] = w else { continue };
        if let Some(h) = heading(a, b) {
            total += previous.map_or(0.0, |p| turn_deg(p, h));
            previous = Some(h);
        }
    }
    total
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
    /// How far from the bridge's end the road stays within [`GROW_TURN_MAX_DEG`] of turn, and the turn where
    /// it stops; `None` if it does not stop before the neighbour's end.
    straight: Option<(f64, f64)>,
    /// The neighbour's length.
    length: f64,
}

impl End {
    /// The straight room: as far as the road is straight, with [`GROW_KEEP_M`] of the neighbour kept.
    fn room(&self) -> f64 {
        let straight = self.straight.map_or(self.length, |s| s.0);
        clean(straight.min(self.length - GROW_KEEP_M))
    }

    /// The room whatever the turns.
    fn reach(&self) -> f64 {
        clean(self.length - GROW_KEEP_M)
    }

    /// Why the room ends.
    fn why(&self) -> RoomEnd {
        match self.straight {
            Some((m, deg)) if m < self.length - GROW_KEEP_M => {
                if m <= EPS_M {
                    RoomEnd::KinkAtJoint { deg }
                } else {
                    RoomEnd::TurnAfter { m, deg }
                }
            }
            _ => RoomEnd::ShortNeighbour,
        }
    }
}

/// The road beyond the start (`at_start`) or end of bridge stretch `i`, unless `other_than` is the neighbour
/// there (a stretch whose two ends meet one neighbour grows into it once); else why there is none.
fn end_of(
    graph: &RoadGraph,
    i: usize,
    at_start: bool,
    other_than: Option<usize>,
) -> Result<End, RoomEnd> {
    let bridge = graph.stretches.get(i).ok_or(RoomEnd::DeadEnd)?;
    let n = continuation(graph, i, at_start)?;
    if Some(n) == other_than {
        return Err(RoomEnd::Junction);
    }
    let neighbour = graph.stretches.get(n).ok_or(RoomEnd::DeadEnd)?;
    let node = if at_start { bridge.start } else { bridge.end };
    let mut points = neighbour.points.clone();
    if neighbour.start != node {
        points.reverse();
    }
    let at = *points.first().ok_or(RoomEnd::ShortNeighbour)?;
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
            straight.get_or_insert((along, total));
            break;
        }
        if let Some(h) = heading(a, b) {
            total += previous.map_or(0.0, |p| turn_deg(p, h));
            previous = Some(h);
        }
        if total > GROW_TURN_MAX_DEG + 1e-9 {
            straight.get_or_insert((along, total));
        }
        along += seg;
        turns.push((along, total));
    }
    let length = polyline_length(&neighbour.points);
    Ok(End {
        stretch: n,
        points,
        turns,
        straight,
        length: if length.is_finite() { length } else { 0.0 },
    })
}

/// The ends of bridge stretch `i`: the road beyond its start and beyond its end.
fn ends_of(graph: &RoadGraph, i: usize) -> (Result<End, RoomEnd>, Result<End, RoomEnd>) {
    let before = end_of(graph, i, true, None);
    let after = end_of(graph, i, false, before.as_ref().ok().map(|e| e.stretch));
    (before, after)
}

/// The room of an end, 0 without a road beyond.
fn room_of(end: &Result<End, RoomEnd>) -> f64 {
    end.as_ref().map_or(0.0, End::room)
}

/// Why the room of an end ends.
fn why_of(end: &Result<End, RoomEnd>) -> RoomEnd {
    match end {
        Ok(e) => e.why(),
        Err(why) => *why,
    }
}

/// How far a bridge on stretch `i` may grow beyond its start and beyond its end: along the stretch that
/// continues it there (at a crossing of exactly two stretches, plain road; a filled bridge is plain road), as
/// long as that road turns by at most [`GROW_TURN_MAX_DEG`] in total from the bridge's own heading at its end
/// on, keeping [`GROW_KEEP_M`] of it. 0 at a junction, a dead end, another bridge or a stretch not in the
/// graph.
#[must_use]
pub fn straight_room(graph: &RoadGraph, stretch: usize) -> (f64, f64) {
    let (before, after) = ends_of(graph, stretch);
    (room_of(&before), room_of(&after))
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

/// The line of bridge stretch `i` with `room` of the road beyond its ends, and the length of the part before
/// it.
fn site_line(
    graph: &RoadGraph,
    i: usize,
    ends: (&Result<End, RoomEnd>, &Result<End, RoomEnd>),
    room: (f64, f64),
) -> (Vec<(f64, f64)>, f64) {
    let Some(bridge) = graph.stretches.get(i) else {
        return (Vec::new(), 0.0);
    };
    let mut line: Vec<(f64, f64)> = Vec::new();
    let mut before = 0.0;
    if let Ok(end) = ends.0 {
        let mut part = slice_points(&end.points, 0.0, room.0);
        part.reverse();
        before = polyline_length(&part);
        line.extend(part);
    }
    line.extend(bridge.points.iter().copied());
    if let Ok(end) = ends.1 {
        line.extend(slice_points(&end.points, 0.0, room.1));
    }
    line.dedup();
    (line, if before.is_finite() { before } else { 0.0 })
}

/// The original ground at `along` (from the bridge's start, `before` metres into `line`) and `across` (to the
/// left of the line), with `ground` the height at a map position.
fn ground_beside(
    line: &[(f64, f64)],
    before: f64,
    along: f64,
    across: f64,
    ground: &dyn Fn((f64, f64)) -> f64,
) -> f64 {
    let at = along + before;
    let Some(point) = point_at(line, at) else {
        return f64::NAN;
    };
    let normal = match (point_at(line, at - 0.5), point_at(line, at + 0.5)) {
        (Some(back), Some(ahead)) => {
            let (dx, dy) = (ahead.0 - back.0, ahead.1 - back.1);
            let length = dx.hypot(dy);
            (length > EPS_M && length.is_finite()).then(|| (-dy / length, dx / length))
        }
        _ => None,
    };
    let (nx, ny) = normal.unwrap_or((0.0, 0.0));
    ground((point.0 + across * nx, point.1 + across * ny))
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
    /// The total turn of its own line, in degrees.
    pub own_turn_deg: f64,
    /// What it is built from.
    pub kind: BridgeKind,
    /// The length of the part; the bridge's length for a panel row, 0 for a fill.
    pub part_length: f64,
    /// How far the part reaches past the bridge's start and end, signed: a model that grows at one end only
    /// has 0 at the other; negative where a model is shorter than the bridge (centred on it); 0 for a panel row.
    pub grown: (f64, f64),
    /// The total turn of the road it grew into at its start and at its end, in degrees.
    pub turn_deg: (f64, f64),
    /// The straight room beyond its start and its end.
    pub room: (f64, f64),
    /// Why that room ends, at its start and at its end.
    pub room_end: (RoomEnd, RoomEnd),
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
/// The joined stretch has the id of the lower of its two outer stretches (so the objects of that way keep
/// their ids and its edits apply; edits of the other ways are orphaned), the surface of its longest part (the
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
        // the first part is the outer stretch with the lower id (see `join_bridges`)
        id: first.id.clone(),
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

/// Every OpenStreetMap bridge of `graph` built from the part `choose` gives it, a model grown along the
/// straight road as far as it needs, and for every bridge what it became and why.
///
/// Bridge stretches that follow each other are first made one stretch ([`BridgeFit::stretches`] names them).
/// Then, in the order of the stretches, each bridge's [`BridgeSite`] is measured (its length and own turn,
/// [`straight_room`] and the turns of the road beyond, whether it touches a node of bridges, and the original
/// ground from `ground`, a height at a map position) and `choose` decides. A model's growth past each end (where
/// it is positive, never closer than [`GROW_KEEP_M`] to the next junction) is moved from the road beside it to
/// the bridge, so a later bridge sees the road that is left. The nodes stay where they are in the graph, only
/// their points move. A panel row covers the bridge and never grows. Every stretch that was a bridge gets its
/// [`Stretch::bridge_kind`].
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
        let own_turn = line_turn(&s.points);
        let (before, after) = ends_of(&g, i);
        let room = (room_of(&before), room_of(&after));
        let room_end = (why_of(&before), why_of(&after));
        let reach = (
            before.as_ref().map_or(0.0, End::reach),
            after.as_ref().map_or(0.0, End::reach),
        );
        let (line, offset) = site_line(&g, i, (&before, &after), room);
        let beside = |along: f64, across: f64| ground_beside(&line, offset, along, across, ground);
        let bends =
            |end: &Result<End, RoomEnd>| end.as_ref().map(|e| e.turns.clone()).unwrap_or_default();
        let site = BridgeSite {
            length,
            family: s.family,
            room,
            turn_deg: own_turn,
            bends: (bends(&before), bends(&after)),
            at_bridge_node: room_end.0 == RoomEnd::BridgeNode || room_end.1 == RoomEnd::BridgeNode,
            ground: &beside,
        };
        let at = point_at(&s.points, length / 2.0).unwrap_or_default();
        let (id, family) = (s.id.clone(), s.family);
        let mut choice = choose(&site);
        let grows = matches!(choice.kind, BridgeKind::Model { .. });
        if !grows {
            choice.grown = (0.0, 0.0);
        }
        // whatever `choose` says, a part never reaches closer than GROW_KEEP_M to the next junction
        choice.grown = (choice.grown.0.min(reach.0), choice.grown.1.min(reach.1));
        let turn = |end: &Result<End, RoomEnd>, d: f64| {
            end.as_ref().map_or(0.0, |e| turn_over(&e.turns, d))
        };
        let turn_deg = (turn(&before, choice.grown.0), turn(&after, choice.grown.1));
        if grows {
            if let Ok(end) = &before {
                grow_into(&mut g, i, true, end.stretch, choice.grown.0);
            }
            if let Ok(end) = &after {
                grow_into(&mut g, i, false, end.stretch, choice.grown.1);
            }
        }
        if let Some(s) = g.stretches.get_mut(i) {
            // until panel rows are laid: only a model is a bridge
            s.bridge = grows;
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
            own_turn_deg: own_turn,
            kind: choice.kind,
            part_length: choice.length,
            grown: choice.grown,
            turn_deg,
            room,
            room_end,
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
    fn the_ground_beside_a_line_is_to_its_left() {
        // a line running east: left is north
        let line = [(0.0, 0.0), (10.0, 0.0)];
        let north = |p: (f64, f64)| p.1;
        assert!((ground_beside(&line, 0.0, 5.0, 2.0, &north) - 2.0).abs() < 1e-9);
        assert!((ground_beside(&line, 0.0, 5.0, -3.0, &north) + 3.0).abs() < 1e-9);
        assert!(ground_beside(&[], 0.0, 5.0, 2.0, &north).is_nan());
        // a line of one point: no direction, the point itself
        assert!((ground_beside(&[(1.0, 4.0)], 0.0, 0.0, 2.0, &north) - 4.0).abs() < 1e-9);
    }

    #[test]
    fn samples_cover_a_post_and_are_bounded() {
        assert_eq!(samples(0.0, 2.5), vec![0.0, 2.5, 1.0, 2.0]);
        assert_eq!(samples(1.0, 1.0), vec![1.0, 1.0]);
        assert_eq!(samples(0.0, 1e9).len(), MAX_POST_SAMPLES);
        assert!((turn_over(&[(2.0, 1.0), (4.0, 3.0)], 3.0) - 3.0).abs() < 1e-12);
        assert!(turn_over(&[(2.0, 1.0)], 0.0).abs() < 1e-12);
        assert!((line_turn(&[(0.0, 0.0), (1.0, 0.0), (2.0, 1.0)]) - 45.0).abs() < 1e-9);
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

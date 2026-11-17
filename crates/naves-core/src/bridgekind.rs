// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What an OpenStreetMap bridge is built from, and how far it grows into the road beside it.
//!
//! [`choose`] is the table of the design: a bridge of at most [`FILL_MAX_M`] is plain road on the terrain, a
//! bridge shorter than [`MODEL_FILL`] of every model its road may have is a row of concrete panels, a longer one
//! the shortest model that covers it (a chain of the family's longest model when none does). A panel row covers
//! its bridge exactly and never grows (its panels are scaled to the bridge and laid as a grid as wide as the
//! road, [`panel_grid`]). A model longer
//! than the bridge takes the missing length from the road at both ends, but only along a straight line
//! ([`straight_room`]) and only on a bridge that is itself straight; where it does not fit, the bridge is a
//! panel row. [`fit_bridges`] does this for every bridge of a road graph and says for each what it became and
//! why.
//!
//! All lengths are metres on the map.

use std::f64::consts::{PI, TAU};
use std::fmt;

use crate::bridgefit::{Below, BelowFit, Chain, Fit, Need, below_fits, below_interval, fit_over};
use crate::catalog::{BridgeModel, Family, PartShape, PieceCatalog, Slab};
use crate::roadnet::{
    BridgeOverRoad, RoadGraph, Stretch, built_as_bridge, crossings_of, crossings_under, grow_into,
    polyline_length, slice_points,
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
/// The most panels along a row or side by side, and the most models in a chain (a guard against absurd input).
pub const MAX_PARTS: usize = 1000;
/// A panel is scaled by at least this much.
pub const PANEL_SCALE_MIN: f64 = 0.75;
/// A panel is scaled by at most this much.
pub const PANEL_SCALE_MAX: f64 = 3.0;

/// The radius, in metres, taken for the tightest curve a road can make where the family's pieces have no arc.
pub const LEAD_RADIUS_M: f64 = 15.0;
/// The shortest straight lead on a bridge's axis, in metres.
pub const LEAD_MIN_M: f64 = 4.0;
/// A neighbour that leaves a bridge's end within this many degrees of the bridge's own heading needs no lead.
pub const LEAD_MIN_TURN_DEG: f64 = 1.0;
/// How far, in degrees, the line from the end of the lead to the neighbour's own path may turn away from that
/// path (the lead's far end is taken far enough along the path for that, as far as there is room).
pub const LEAD_BLEND_DEG: f64 = 5.0;

/// Lengths closer than this are equal.
const EPS_M: f64 = 1e-6;
/// The most ground samples along or across one post.
const MAX_POST_SAMPLES: usize = 64;

/// What an OpenStreetMap bridge is built from.
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeKind {
    /// Plain road on the terrain.
    Fill,
    /// Concrete panels laid crosswise as a grid, `count` along the bridge's own line from its start to its end
    /// (each on the line's heading there) and `abreast` side by side, all scaled by `scale` (so that they cover
    /// the bridge exactly and come near the road's width).
    Panels {
        /// How many panels along the bridge.
        count: usize,
        /// How many panels side by side.
        abreast: usize,
        /// The scale of every panel.
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

impl fmt::Display for BridgeKind {
    /// `fill`, `panels 3 x 2 at scale 0.98` (along x abreast) or `bridge_wood2 x 1`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BridgeKind::Fill => write!(f, "fill"),
            BridgeKind::Panels {
                count,
                abreast,
                scale,
            } => write!(f, "panels {count} x {abreast} at scale {scale:.2}"),
            BridgeKind::Model { model, count } => write!(f, "{} x {count}", model_name(model)),
        }
    }
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
    /// The model is shorter than the bridge, which meets other bridges at both ends: a panel grid instead.
    ModelBetweenBridges {
        /// The model's path.
        model: String,
    },
    /// The model would stand on a road below the bridge (or end on it) wherever it lies within the straight
    /// room: a panel row instead.
    ModelBlocksRoad {
        /// The model's path.
        model: String,
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
            Fallback::ModelBetweenBridges { model } => write!(
                f,
                "{} is shorter than the bridge, which meets other bridges at both ends",
                model_name(model)
            ),
            Fallback::ModelBlocksRoad { model } => write!(
                f,
                "{} would stand on the road below the bridge",
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
    /// The radius of the tightest curve of the road's family, for the lead on the bridge's axis: the chord over
    /// twice the sine of half the turn of its tightest arc piece, [`LEAD_RADIUS_M`] without one.
    pub lead_radius: f64,
    /// How far the part was slid along the bridge's line from where the table puts it so that the roads below
    /// pass through its opening, in metres (positive towards the bridge's end); 0 if it was not.
    pub slid: f64,
    /// What the part got for each road below the bridge (see [`BridgeSite::below`]).
    pub below: Vec<BelowFit>,
}

impl Choice {
    fn fill(fallback: Vec<Fallback>) -> Choice {
        Choice {
            kind: BridgeKind::Fill,
            length: 0.0,
            grown: (0.0, 0.0),
            fallback,
            lead_radius: LEAD_RADIUS_M,
            slid: 0.0,
            below: Vec::new(),
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
    /// Whether the room ends at a node where only bridges meet, at its start and at its end: a model shorter
    /// than the bridge then reaches that node (it is not chosen where both ends are such nodes).
    pub node_ends: (bool, bool),
    /// The original ground at `(along, across)`: metres along its line from its start (negative before it,
    /// beyond its length after it, on the road it would grow into) and metres to the left of the line.
    pub ground: &'a dyn Fn(f64, f64) -> f64,
    /// The roads that cross below it (see [`crate::roadnet::crossings_under`]): the part must lie so that each
    /// passes through its opening (see [`crate::bridgefit`]).
    pub below: Vec<Below>,
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
            .field("node_ends", &self.node_ends)
            .field("below", &self.below)
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

/// The panel the grids are laid from: the first by model path of those with a usable size.
#[must_use]
pub fn panel_slab(catalog: &PieceCatalog) -> Option<&Slab> {
    catalog
        .panels()
        .iter()
        .filter(|s| clean(s.along) > 0.0 && clean(s.across) > 0.0)
        .min_by(|a, b| a.model.cmp(&b.model))
}

/// How many panels abreast a grid of panels `slab` scaled by `scale` gets on a road `road_width` wide, and how far
/// the grid's width is from the road's: every count from 1 up to the first whose width exceeds the road's (that
/// one included), the nearest to the road's width among those at most [`BRIDGE_NARROWER_M`] narrower than the
/// road, else the nearest of all; the fewer panels on a tie. `(abreast, |width - road_width|, at least the
/// road's width less BRIDGE_NARROWER_M)`.
fn abreast(scale: f64, road_width: f64, slab: &Slab) -> (usize, f64, bool) {
    let one = slab.across * scale;
    let mut best: Option<(usize, f64, bool)> = None;
    for k in 1..=MAX_PARTS {
        let width = times(k) * one;
        let off = (width - road_width).abs();
        let wide = width >= road_width - BRIDGE_NARROWER_M - 1e-9;
        let better = best.is_none_or(|(_, b_off, b_wide)| {
            (wide && !b_wide) || (wide == b_wide && off < b_off - 1e-9)
        });
        if better {
            best = Some((k, off, wide));
        }
        if width.is_nan() || width > road_width {
            break;
        }
    }
    best.unwrap_or((1, f64::INFINITY, false))
}

/// The grid of panels `slab` over a bridge `length` long on a road `road_width` wide: how many along it, how
/// many side by side, and their one scale, so that the panels along cover the bridge exactly.
///
/// The candidates are every count along whose scale `length / (slab.along * count)` lies in
/// [`PANEL_SCALE_MIN`]..=[`PANEL_SCALE_MAX`], each with the counts abreast from 1 up to the first whose width exceeds the road's. Of
/// those whose grid is at most [`BRIDGE_NARROWER_M`] narrower than the road the one nearest the road's width,
/// else the nearest of all; on a tie the fewest panels, then the fewest along. Where no count along gives a
/// scale in range (not above [`FILL_MAX_M`] with the catalog's panel, but the input is untrusted), the count
/// nearest `length / slab.along`. A road width that is not a number counts as 0; at most [`MAX_PARTS`] along and
/// abreast; never a panic.
#[must_use]
pub fn panel_grid(length: f64, road_width: f64, slab: &Slab) -> (usize, usize, f64) {
    let width = clean(road_width);
    let scale = |n: usize| length / (slab.along * times(n));
    let first = whole((length / (slab.along * PANEL_SCALE_MAX) - 1e-9).ceil())
        .unwrap_or(MAX_PARTS)
        .max(1);
    let last = whole((length / (slab.along * PANEL_SCALE_MIN) + 1e-9).floor()).unwrap_or(MAX_PARTS);
    let mut best: Option<(usize, usize, f64, f64, bool)> = None;
    for n in first..=last.min(MAX_PARTS) {
        let s = scale(n);
        if !(PANEL_SCALE_MIN - 1e-9..=PANEL_SCALE_MAX + 1e-9).contains(&s) {
            continue;
        }
        let (k, off, wide) = abreast(s, width, slab);
        let panels = n.saturating_mul(k);
        let better = best.is_none_or(|(b_n, b_k, _, b_off, b_wide)| {
            if wide != b_wide {
                return wide;
            }
            if (off - b_off).abs() > 1e-9 {
                return off < b_off;
            }
            let b_panels = b_n.saturating_mul(b_k);
            panels < b_panels || (panels == b_panels && n < b_n)
        });
        if better {
            best = Some((n, k, s, off, wide));
        }
    }
    if let Some((n, k, s, _, _)) = best {
        return (n, k, s);
    }
    let n = whole((length / slab.along).round())
        .unwrap_or(MAX_PARTS)
        .clamp(1, MAX_PARTS);
    let s = scale(n);
    let (k, _, _) = abreast(s, width, slab);
    (n, k, s)
}

/// The radius of the tightest curve of `family`'s pieces ([`LEAD_RADIUS_M`] without an arc).
fn lead_radius(catalog: &PieceCatalog, family: Family) -> f64 {
    catalog
        .tightest_arc(family)
        .filter(|r| r.is_finite() && *r > 0.0)
        .unwrap_or(LEAD_RADIUS_M)
}

/// A panel row over `site`, with `fallback` so far: it covers the bridge exactly (see [`panel_grid`]) and
/// needs no room; without panels in the catalog a fill.
fn panel_row(catalog: &PieceCatalog, site: &BridgeSite, mut fallback: Vec<Fallback>) -> Choice {
    let Some(slab) = panel_slab(catalog) else {
        fallback.push(Fallback::NoParts);
        return Choice::fill(fallback);
    };
    let (count, abreast, scale) = panel_grid(site.length, piece_width(catalog, site.family), slab);
    Choice {
        kind: BridgeKind::Panels {
            count,
            abreast,
            scale,
        },
        length: site.length,
        grown: (0.0, 0.0),
        fallback,
        lead_radius: lead_radius(catalog, site.family),
        slid: 0.0,
        below: Vec::new(),
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
/// - Otherwise a panel row of the bridge's own length (see [`panel_grid`]). Without panels: a fill
///   ([`Fallback::NoParts`]).
///
/// Where roads cross below the bridge ([`BridgeSite::below`]) the part must also let each pass through its
/// opening (see [`crate::bridgefit`]), in this order: the part the table gives, slid along the bridge's line
/// within its straight room (and its turn limit, and its posts); the panel row of the bridge's own length
/// ([`Fallback::ModelBlocksRoad`]); else the placement that leaves the least of the roads' intervals under a
/// solid part or outside the deck, which [`Choice::below`] reports. Standing geometry (the cross walls of
/// `bridge_concrete3`) never passes a road. A panel row is never lengthened.
#[must_use]
pub fn choose(catalog: &PieceCatalog, site: &BridgeSite) -> Choice {
    let choice = choose_part(catalog, site);
    if site.below.is_empty() || matches!(choice.kind, BridgeKind::Fill) {
        return choice;
    }
    over_roads(catalog, site, choice)
}

/// The table of [`choose`] without the roads below the bridge.
fn choose_part(catalog: &PieceCatalog, site: &BridgeSite) -> Choice {
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
        // a model shorter than its bridge reaches a node where other bridges meet it; between two such nodes it
        // has no place
        let grown = if grown.0 < 0.0 || grown.1 < 0.0 {
            let gap = -(grown.0 + grown.1);
            match site.node_ends {
                (true, true) => {
                    let fallback = vec![Fallback::ModelBetweenBridges {
                        model: model.model.clone(),
                    }];
                    return panel_row(catalog, site, fallback);
                }
                (true, false) => (0.0, -gap),
                (false, true) => (-gap, 0.0),
                (false, false) => grown,
            }
        } else {
            grown
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
            lead_radius: lead_radius(catalog, site.family),
            slid: 0.0,
            below: Vec::new(),
        };
    }
    panel_row(catalog, site, Vec::new())
}

/// The range of metres a model chain `part` long may reach past the start of a bridge `length` long (`grown.0`),
/// from the room beyond its ends and the nodes of bridges it keeps to; `None` where it has no place.
fn growth_range(
    part: f64,
    length: f64,
    room: (f64, f64),
    node_ends: (bool, bool),
) -> Option<(f64, f64)> {
    let missing = part - length;
    if !missing.is_finite() {
        return None;
    }
    if missing >= 0.0 {
        let lo = (missing - room.1).max(0.0);
        let hi = room.0.min(missing);
        return (lo <= hi + EPS_M).then_some((lo, hi.max(lo)));
    }
    match node_ends {
        (true, true) => None,
        (true, false) => Some((0.0, 0.0)),
        (false, true) => Some((missing, missing)),
        (false, false) => Some((missing, 0.0)),
    }
}

/// What every road below `site` needs along a part of shape `shape`.
fn needs_of(catalog: &PieceCatalog, site: &BridgeSite, shape: &PartShape) -> Vec<Need> {
    site.below
        .iter()
        .map(|b| Need {
            station: b.station,
            reach: below_interval(piece_width(catalog, b.family), b.angle, shape),
        })
        .collect()
}

/// `choice` made to let the roads below `site` pass: see [`choose`].
fn over_roads(catalog: &PieceCatalog, site: &BridgeSite, mut choice: Choice) -> Choice {
    let room = (clean(site.room.0), clean(site.room.1));
    match choice.kind.clone() {
        BridgeKind::Panels {
            count,
            abreast,
            scale,
        } => {
            let Some(slab) = panel_slab(catalog) else {
                return choice;
            };
            let shape = PartShape::panels(slab, count, abreast, scale);
            let chain = Chain {
                shape: &shape,
                count: 1,
            };
            let needs = needs_of(catalog, site, &shape);
            choice.below = below_fits(&chain, 0.0, &site.below, &needs);
            choice
        }
        BridgeKind::Model { model, count } => {
            let Some(bm) = catalog.bridges().iter().find(|b| b.model == model) else {
                return choice;
            };
            let shape = bm.shape();
            let chain = Chain {
                shape: &shape,
                count,
            };
            let needs = needs_of(catalog, site, &shape);
            let wanted = -choice.grown.0;
            let range = growth_range(choice.length, site.length, room, site.node_ends)
                .map_or((wanted, wanted), |(lo, hi)| (-hi, -lo));
            let missing_len = choice.length - site.length;
            let valid = |start: f64| {
                let (g0, g1) = (-start, missing_len + start);
                let turn = clean(site.turn_deg)
                    + turn_over(&site.bends.0, g0.max(0.0))
                    + turn_over(&site.bends.1, g1.max(0.0));
                turn <= GROW_TURN_MAX_DEG + 1e-9
                    && (!bm.posts() || posts_reach(bm, count, site, (g0, g1)))
            };
            let fit = fit_over(&chain, range, wanted, &needs, &valid);
            if let Fit::Blocked { missing, .. } = fit {
                // the next step: a panel row of the bridge's own length, if it lets the roads pass better
                let alt = panel_row(
                    catalog,
                    site,
                    vec![Fallback::ModelBlocksRoad {
                        model: model.clone(),
                    }],
                );
                if matches!(alt.kind, BridgeKind::Panels { .. }) {
                    let alt = over_roads(catalog, site, alt);
                    let panels_missing: f64 = alt.below.iter().map(|b| b.missing).sum();
                    if panels_missing + EPS_M < missing {
                        return alt;
                    }
                }
            }
            let start = fit.start();
            choice.grown = (-start, missing_len + start);
            choice.slid = start - wanted;
            choice.below = below_fits(&chain, start, &site.below, &needs);
            choice
        }
        BridgeKind::Fill => choice,
    }
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

/// What was done at one end of a bridge so that the road meets it on its own axis.
///
/// A bridge's deck lies node to node on its OpenStreetMap line, as a model on its ports and a grid on its
/// steps. A road that leaves the node turned away from the deck cannot arrive on the deck's heading at the node
/// (it would need an S-bend), so its last stretch before the node is made straight on the deck's axis.
#[derive(Debug, Clone, PartialEq)]
pub enum Lead {
    /// There is no neighbour to move (a junction, a node of bridges, a dead end): the road meets the deck as it
    /// is, and a miss is reported by the placement.
    Beyond(RoomEnd),
    /// The road beyond leaves within [`LEAD_MIN_TURN_DEG`] of the deck's heading (or, with no room for a lead,
    /// within [`LEAD_BLEND_DEG`]): it needs none, nothing is moved.
    Straight,
    /// The neighbour's first `replaced` metres are replaced by a straight of `straight` metres on the deck's
    /// axis from the node, and a line from its end to the neighbour's own path.
    Laid {
        /// The turn of the neighbour against the deck's heading, in degrees.
        turn_deg: f64,
        /// The length of the straight on the axis.
        straight: f64,
        /// How much of the neighbour's own line was replaced.
        replaced: f64,
    },
    /// The neighbour is too short for a lead: nothing is moved.
    NoRoom {
        /// The turn of the neighbour against the deck's heading, in degrees.
        turn_deg: f64,
        /// The room there was, in metres.
        room: f64,
    },
    /// The bridge is plain road: there is nothing to meet.
    Unbuilt,
}

impl fmt::Display for Lead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Lead::Beyond(why) => write!(f, "no lead ({why})"),
            Lead::Straight => write!(f, "none needed"),
            Lead::Laid {
                turn_deg,
                straight,
                replaced,
            } => write!(
                f,
                "lead of {straight:.1} m for a turn of {turn_deg:.1} degrees ({replaced:.1} m replaced)"
            ),
            Lead::NoRoom { turn_deg, room } => write!(
                f,
                "no room for a lead ({room:.1} m) for a turn of {turn_deg:.1} degrees"
            ),
            Lead::Unbuilt => write!(f, "plain road"),
        }
    }
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
    /// Where the part's first end lies, in metres from the bridge's own start along it (the bridge's start in
    /// the stored direction of the stretch): `-grown.0`, so negative where the part grows past the start and
    /// positive where a short model is centred, or keeps to a node of bridges. The report, the post check and
    /// the placement all read this one place.
    pub part_start: f64,
    /// What was done at its start and at its end so that the road meets the deck on its axis.
    pub lead: (Lead, Lead),
    /// The total turn of the road it grew into at its start and at its end, in degrees.
    pub turn_deg: (f64, f64),
    /// The straight room beyond its start and its end.
    pub room: (f64, f64),
    /// Why that room ends, at its start and at its end.
    pub room_end: (RoomEnd, RoomEnd),
    /// Why it is not the part the table gives first.
    pub fallback: Vec<Fallback>,
    /// How far the part was slid along the bridge's line from where the table puts it, so that the roads
    /// below pass through its opening, in metres (positive towards the bridge's end); 0 if it was not.
    pub slid: f64,
    /// What the part got for each road below the bridge, in the order of the roads' stretches.
    pub below: Vec<BelowFit>,
}

impl BridgeFit {
    /// One line for the build's report and summary: the bridge, where it is, what it was built from and why not
    /// from the part the table gives first.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut line = format!(
            "bridge {} at ({:.0}, {:.0}), {:.1} m: {}",
            self.stretches.join("+"),
            self.at.0,
            self.at.1,
            self.length,
            self.kind
        );
        if !self.fallback.is_empty() {
            let why: Vec<String> = self.fallback.iter().map(ToString::to_string).collect();
            line = format!("{line} ({})", why.join("; "));
        }
        if self.slid.abs() > EPS_M {
            line = format!("{line}, slid {:.1} m for the road below", self.slid);
        }
        let missing: f64 = self.below.iter().map(|b| b.missing).sum();
        if missing > EPS_M {
            line =
                format!("{line}, {missing:.1} m of the opening the road below needs are missing");
        }
        line
    }
}

/// Makes the road beyond the end (`at_start`: the start) of bridge stretch `i` meet the deck on its axis; see
/// [`Lead`]. `steps` is how many steps a grid has (1 for a model): the deck's heading at its end is that of the
/// chord of the last step, as the placement lays it. `radius` is the tightest curve of the road's family.
///
/// The neighbour's line from the node outwards is `[node, a, q, ..]`: `a` on the deck's axis `d` metres out, with
/// `d = max(`[`LEAD_MIN_M`]`, 2 R tan(turn / 2))`, and `q` on its own path far enough along that the line from
/// `a` to `q` turns away from the path by at most [`LEAD_BLEND_DEG`] (limited to half the neighbour and to what
/// is left of it less [`GROW_KEEP_M`]); the points of the path before `q` are dropped. The turn then happens at
/// `a`, on plain road, where the pieces can curve it.
fn lead_in(
    graph: &mut RoadGraph,
    i: usize,
    at_start: bool,
    end: &Result<End, RoomEnd>,
    steps: usize,
    radius: f64,
) -> Lead {
    match end {
        Ok(end) => lead_beyond(graph, i, at_start, end, steps, radius).unwrap_or(Lead::Unbuilt),
        Err(why) => Lead::Beyond(*why),
    }
}

/// [`lead_in`] for a neighbour that exists; `None` where the line is not usable (no heading, not in the graph).
fn lead_beyond(
    graph: &mut RoadGraph,
    i: usize,
    at_start: bool,
    end: &End,
    steps: usize,
    radius: f64,
) -> Option<Lead> {
    let bridge = graph.stretches.get(i)?;
    let node_id = if at_start { bridge.start } else { bridge.end };
    // the deck's points with the end in question last
    let deck: Vec<(f64, f64)> = if at_start {
        bridge.points.iter().rev().copied().collect()
    } else {
        bridge.points.clone()
    };
    let total = polyline_length(&deck);
    let step = total / times(steps.max(1));
    let from = point_at(&deck, (total - step).max(0.0))?;
    let node = deck.last().copied()?;
    let out = heading(from, node)?;
    let own = end
        .points
        .windows(2)
        .find_map(|w| heading(*w.first()?, *w.get(1)?))?;
    let turn = turn_deg(out, own);
    if turn < LEAD_MIN_TURN_DEG {
        return Some(Lead::Straight);
    }
    let room = (end.length / 2.0).min(end.reach()).max(0.0);
    let half = turn.to_radians() / 2.0;
    let straight = (2.0 * radius * half.tan()).max(LEAD_MIN_M);
    let blend = LEAD_BLEND_DEG.to_radians();
    let wanted = straight * (turn.to_radians().sin() / blend.tan() + turn.to_radians().cos());
    let replaced = wanted.min(room);
    if !(straight.is_finite() && replaced >= straight + LEAD_MIN_M) {
        // a turn the pieces take within the tolerated heading miss needs no lead, room or none
        if turn <= LEAD_BLEND_DEG {
            return Some(Lead::Straight);
        }
        return Some(Lead::NoRoom {
            turn_deg: turn,
            room,
        });
    }
    let axis = (node.0 + straight * out.sin(), node.1 + straight * out.cos());
    let mut away = vec![node, axis];
    away.extend(slice_points(
        &end.points,
        replaced,
        end.length.max(replaced),
    ));
    let n = graph.stretches.get_mut(end.stretch)?;
    if n.start != node_id {
        away.reverse();
    }
    n.points = away;
    Some(Lead::Laid {
        turn_deg: turn,
        straight,
        replaced,
    })
}

/// The thickness of the deck of the bridge on stretch `s`, for the clearance over a road below: a panel grid is as
/// thick as one panel at its scale, a model as measured on it (`None` where it cannot be measured).
#[must_use]
pub fn deck_thickness(catalog: &PieceCatalog, s: &Stretch) -> Option<f64> {
    match &s.bridge_kind {
        Some(BridgeKind::Panels { scale, .. }) => {
            panel_slab(catalog).map(|slab| slab.thickness * scale)
        }
        _ => crate::roadplace::bridge_model(catalog, s).and_then(|m| {
            (m.thickness_source != crate::catalog::ThicknessSource::Unknown)
                .then_some(m.deck_thickness)
        }),
    }
}

/// The thickness of the deck of the bridge of `c` over the road below it, for the clearance the height solve
/// keeps: what hangs under the deck over the road's interval ([`below_interval`], the footprint with the skew),
/// not the deepest point of the part and not its crown. A panel grid is as thick as one panel at its scale. A
/// bridge that has no part of its own (over a dip) and a part whose interval is not on it get
/// [`deck_thickness`]. `None` where the part cannot be measured.
#[must_use]
pub fn crossing_thickness(catalog: &PieceCatalog, c: &BridgeOverRoad) -> Option<f64> {
    let upper = c.upper;
    let Some(BridgeKind::Model { count, .. }) = &upper.bridge_kind else {
        return deck_thickness(catalog, upper);
    };
    let model = crate::roadplace::bridge_model(catalog, upper)?;
    if model.thickness_source == crate::catalog::ThicknessSource::Unknown {
        return None;
    }
    let shape = model.shape();
    let chain = Chain {
        shape: &shape,
        count: *count,
    };
    let reach = below_interval(
        piece_width(catalog, c.lower.family),
        c.crossing.angle,
        &shape,
    );
    // where the road lies on the part: from the part's first end, which is `bridge_offset` into the stretch
    let at = c.crossing.upper_s - upper.bridge_offset;
    Some(
        chain
            .thickness(at - reach.footprint, at + reach.footprint)
            .unwrap_or(model.deck_thickness),
    )
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

/// The roads that cross below bridge stretch `i` of `graph`, as the fit needs them.
fn roads_below(graph: &RoadGraph, i: usize) -> Vec<Below> {
    crossings_under(graph, i)
        .iter()
        .filter_map(|c| {
            let lower = graph.stretches.get(c.lower)?;
            Some(Below {
                station: c.upper_s,
                angle: c.angle,
                family: lower.family,
                at: c.at,
                lower: lower.id.clone(),
            })
        })
        .collect()
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
/// A bridge built from panels or from a model stays a bridge in the result (`bridge` is true); a filled one is
/// plain road (`bridge` is false).
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
            node_ends: (
                room_end.0 == RoomEnd::BridgeNode,
                room_end.1 == RoomEnd::BridgeNode,
            ),
            ground: &beside,
            below: roads_below(&g, i),
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
            // a panel grid and a model are bridges, a fill is plain road
            s.bridge = !matches!(choice.kind, BridgeKind::Fill);
            // a short model lies this far from the stretch's start (growth is already in the stretch)
            s.bridge_offset = (-choice.grown.0).max(0.0);
            s.bridge_kind = Some(choice.kind.clone());
        }
        let lead = if matches!(choice.kind, BridgeKind::Fill) {
            (Lead::Unbuilt, Lead::Unbuilt)
        } else {
            let steps = match &choice.kind {
                BridgeKind::Panels { count, .. } => *count,
                _ => 1,
            };
            let ends = ends_of(&g, i);
            (
                lead_in(&mut g, i, true, &ends.0, steps, choice.lead_radius),
                lead_in(&mut g, i, false, &ends.1, steps, choice.lead_radius),
            )
        };
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
            part_start: 0.0 - choice.grown.0,
            lead,
            turn_deg,
            room,
            room_end,
            fallback: choice.fallback,
            slid: choice.slid,
            below: choice.below,
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

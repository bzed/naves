// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Places the whole road network: the chains of through roads in rank order, junction pieces where side
//! roads meet them, side roads that start at the junction's ports, end caps at dead ends.
//!
//! The solve is hierarchical rather than joint: a through road is fitted first and decides where a
//! junction sits; a side road then starts at a port of that junction and is closed against whatever else
//! is already fixed. A road that is edited therefore cannot move one that was placed before it.
//!
//! Heading and coordinate conventions are those of [`crate::roadfit`].

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{PI, TAU};

use crate::bridgekind::BridgeKind;
use crate::catalog::{BridgeModel, Family, Junction, PieceCatalog, PieceKind, Port, RoadPiece};
use crate::roadfit::{
    EndGoal, FitError, FitPiece, GAP_CLOSED_M, GAP_TOLERATED_M, KINK_TOLERATED_DEG, Pose, fit_line,
};
use crate::roadnet::{Chain, Ring, RoadGraph, Stretch, chains_with, overlay_rank, rings};

/// Two junctions on one road must be at least this far apart (the length of a junction piece).
/// A road that meets a bridge's end turned away from the deck's heading by more than this, in degrees, is a kink
/// (a finding) even where its position is right.
pub const HEADING_MISS_DEG: f64 = 5.0;
/// The least distance between two junctions on one road: the length of a junction piece (its ports lie half of it from the
/// centre).
pub(crate) const JUNCTION_SPACING_M: f64 = 12.5;
/// A junction piece has to sit this close to the crossing it replaces.
const JUNCTION_TOLERANCE_M: f64 = 3.0;
/// The steps in which a junction is tried along its through road when a road cannot close against it.
pub const JUNCTION_SHIFT_STEP_M: f64 = 6.25;
/// The furthest a junction is moved from its crossing.
pub const JUNCTION_SHIFT_MAX_M: f64 = 25.0;
/// The number of steps up to [`JUNCTION_SHIFT_MAX_M`] on each side.
const JUNCTION_SHIFT_STEPS: u32 = 4;
/// A road that ends under another one reaches this far under its edge (but not past half its width: the
/// centre line is never crossed).
const SIDE_OVERLAP_M: f64 = 2.0;
/// The end of a road under another one is at least this far from the other road's centre line.
const MIN_UNDER_M: f64 = 0.1;
/// Where the free end of a fit lands cannot be steered exactly (the pieces are 6.25 m and longer), so the
/// cut is tried at these fractions of the top road's half width until the end lies under its edge.
const CUT_TRIES: [f64; 5] = [1.0, 0.75, 0.5, 0.25, 0.1];
/// Where roads meet without a junction piece, the surface of the lower road is this far below the surface of
/// the higher road, so that the two do not flicker where they overlap.
pub const OVERLAY_DROP_M: f64 = 0.05;
/// A deck slides this far along its line, at most [`BRIDGE_SLIDE_STEPS`] times each way, to let the road close on it.
const BRIDGE_SLIDE_STEP_M: f64 = 0.25;
/// See [`BRIDGE_SLIDE_STEP_M`]: 2.5 m each way at most, a tenth of a bridge model. The line puts the deck's ends on
/// the banks; a deck slid by metres has one end over the gap and the other on the ground beyond (research 9.6).
const BRIDGE_SLIDE_STEPS: u32 = 10;
/// A deck is laid on its chord only if the road, fitted to the deck's own place, arrives within this of the
/// chord's heading: the arcs leave a degree or two, a larger error is a road that turns into the bridge (a span
/// on a curve), and forcing it onto the chord puts a tight turn right before the deck (research 9.6).
const BRIDGE_TURN_MAX_DEG: f64 = 2.5;
/// A bridge model shorter than this is not used: a length near zero would ask for endless pieces.
pub const MIN_BRIDGE_M: f64 = 1.0;
/// A bridge model may be this much narrower than the piece of its road, in metres; wider is always fine. Bridges
/// are never scaled.
pub const BRIDGE_NARROWER_M: f64 = 1.0;
/// Which bridge model serves which roads, by the model's file name: the one table to change.
pub const BRIDGE_MODELS: [(&str, &[Family]); 5] = [
    ("bridge_wood2", &[Family::Grav, Family::Mud]),
    ("bridge_wood_25", &[Family::Grav, Family::Mud]),
    ("bridge_concrete3", &[Family::Asf3]),
    ("bridge_metal_25", &[Family::Asf1, Family::City]),
    ("bridge_stone_25", &[Family::Asf2, Family::Asf3]),
];

/// The file name of a model path, lower case, without folder and extension.
pub(crate) fn model_name(path: &str) -> String {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    name.strip_suffix(".p3d")
        .unwrap_or(name)
        .to_ascii_lowercase()
}

/// Every bridge model for a road of `family`: the models [`BRIDGE_MODELS`] names for it whose deck is at most
/// [`BRIDGE_NARROWER_M`] narrower than the family's piece (wider is always fine), shortest first, then by
/// model path. Never scaled. A model whose length is not finite or below [`MIN_BRIDGE_M`], or whose width is
/// not finite or not positive, is never offered (the catalog is untrusted); a road width that is not finite
/// counts as 0.
#[must_use]
pub fn bridge_for(catalog: &PieceCatalog, family: Family) -> Vec<&BridgeModel> {
    let width = catalog
        .pieces(family)
        .first()
        .map_or(0.0, |p| p.width)
        .max(0.0);
    let width = if width.is_finite() { width } else { 0.0 };
    let mut fitting: Vec<&BridgeModel> = catalog
        .bridges()
        .iter()
        .filter(|b| {
            let name = model_name(&b.model);
            BRIDGE_MODELS
                .iter()
                .any(|(model, families)| *model == name && families.contains(&family))
        })
        .filter(|b| {
            b.length.is_finite() && b.length >= MIN_BRIDGE_M && b.width.is_finite() && b.width > 0.0
        })
        .filter(|b| b.width + BRIDGE_NARROWER_M + 1e-6 >= width)
        .collect();
    fitting.sort_by(|a, b| a.length.total_cmp(&b.length).then(a.model.cmp(&b.model)));
    fitting
}

/// The longest model of [`bridge_for`] (the first in model order among equals): what a bridge over a dip gets
/// (an OpenStreetMap bridge gets the model [`crate::bridgekind::fit_bridges`] chose, see [`bridge_model`]).
#[must_use]
pub fn longest_bridge(catalog: &PieceCatalog, family: Family) -> Option<&BridgeModel> {
    bridge_for(catalog, family)
        .into_iter()
        .reduce(|best, b| if b.length > best.length { b } else { best })
}

/// The bridge model laid on bridge stretch `stretch`: the model its [`Stretch::bridge_kind`] names, else (a
/// bridge over a dip, or a stretch whose bridges were not fitted) [`longest_bridge`]. `None` where the catalog
/// has no such model.
#[must_use]
pub fn bridge_model<'a>(catalog: &'a PieceCatalog, stretch: &Stretch) -> Option<&'a BridgeModel> {
    match &stretch.bridge_kind {
        Some(BridgeKind::Model { model, .. }) => {
            catalog.bridges().iter().find(|b| &b.model == model)
        }
        _ => longest_bridge(catalog, stretch.family),
    }
}

/// The identity of a placed object along a stretch: it does not change with the direction a chain is
/// walked, nor with the other roads.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Slot {
    /// The stretch id, or `<lower id>|<higher id>` for a junction between two stretches.
    pub stretch: String,
    /// The position along the stretch in its stored direction; `u32::MAX` and `u32::MAX - 1` are the end
    /// caps at its stored end and start.
    pub index: u32,
}

/// What a placed object is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacedKind {
    /// A straight or an arc.
    Piece,
    /// A T or X junction.
    Junction,
    /// A bridge model.
    Bridge,
    /// A dead end.
    EndCap,
}

/// One object of the network.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedRoad {
    /// Its identity.
    pub slot: Slot,
    /// What it is.
    pub kind: PlacedKind,
    /// The surface.
    pub family: Family,
    /// Whether a vehicle has to be able to drive it.
    pub drivable: bool,
    /// Model path.
    pub model: String,
    /// Position and rotation of the model's origin.
    pub origin: Pose,
    /// Where the road enters, in the direction of the chain.
    pub start: Pose,
    /// Where the road leaves.
    pub end: Pose,
    /// Width of the road.
    pub width: f64,
    /// Uniform scale of the model: 1 for everything the placement lays except the panels of a bridge.
    pub scale: f64,
    /// How many objects of this model lie side by side across the road, centred on the line: 1, except for a
    /// step of a panel grid (`road.width` is then the grid's width).
    pub abreast: usize,
}

/// A stretch whose fit did not meet what it had to.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadGap {
    /// The stretch id.
    pub stretch: String,
    /// Where it should have met.
    pub at: (f64, f64),
    /// The distance by which it misses.
    pub gap_m: f64,
    /// The heading difference there.
    pub heading_error_deg: f64,
}

/// A place where roads meet without a junction piece and the higher road runs on over the lower one: the
/// lower road ends (or is interrupted) under the higher road's edge, `OVERLAY_DROP_M` below its surface.
#[derive(Debug, Clone, PartialEq)]
pub struct Overlay {
    /// The crossing, in map metres.
    pub at: (f64, f64),
    /// The ids of the stretches of the lower roads that end or are interrupted here.
    pub lower: Vec<String>,
}

/// A road that meets the end of a bridge turned away from the deck's heading, at the deck's port.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadKink {
    /// The id of the bridge stretch.
    pub stretch: String,
    /// The port, in map metres.
    pub at: (f64, f64),
    /// How far the road's heading is from the deck's, in degrees.
    pub heading_error_deg: f64,
}

/// Everything that is placed, and what could not be.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlacedNetwork {
    /// The objects, chain by chain in driving order.
    pub roads: Vec<PlacedRoad>,
    /// Stretches that miss what they had to meet.
    pub gaps: Vec<RoadGap>,
    /// Roads that meet the end of a bridge turned away from its deck (a kink, apart from the position gaps).
    pub kinks: Vec<RoadKink>,
    /// Crossings where roads meet without a junction piece and no road can lie on top of the others (no
    /// through road passes: the roads all end there).
    pub joins_without_junction: Vec<(f64, f64)>,
    /// Crossings where roads meet without a junction piece and the lower roads end under the higher one.
    pub overlays: Vec<Overlay>,
    /// Tunnel stretches that got no pieces, with their first point.
    pub tunnels_omitted: Vec<(String, (f64, f64))>,
    /// Indices into `roads`, in driving order, one list per placed run of a chain.
    pub chains: Vec<Vec<usize>>,
    /// Things worth knowing that did not stop the placement.
    pub warnings: Vec<String>,
    /// Parts of roads left out because a higher road runs along them (one road drawn twice in the source).
    pub duplicates: Vec<crate::roadnet::DroppedDuplicate>,
    /// What the clean-up of the graph did (set by the build after the placement, like `duplicates`).
    pub clean: crate::netclean::CleanReport,
    /// What every OpenStreetMap bridge was built from (set by the build after the placement, like
    /// `duplicates`).
    pub bridge_fits: Vec<crate::bridgekind::BridgeFit>,
    /// The roundabouts that are placed as a closed loop (the others, smaller than `ROUNDABOUT_KEEP_M` or than the tightest arc of their
    /// family, are ordinary crossings).
    pub rings: Vec<crate::roadnet::Ring>,
}

/// Why the network could not be placed.
#[derive(Debug, thiserror::Error)]
pub enum PlaceError {
    /// The fit failed for input that cannot be fitted.
    #[error("cannot fit road pieces to stretch {stretch}: {source}")]
    Fit {
        /// The stretch id.
        stretch: String,
        /// Why not.
        source: FitError,
    },
}

/// How far the junctions are moved along their through roads, by the `(chain, link)` of their plan.
type Shifts = BTreeMap<(usize, usize), f64>;

/// A stretch end: the stretch and whether the end is its stored start.
type Arm = (usize, bool);

/// A junction piece that replaces a crossing of a through road.
struct Plan<'a> {
    chain: usize,
    /// Between link `link` and link `link + 1`.
    link: usize,
    point: (f64, f64),
    junction: Cow<'a, Junction>,
    /// The side arms and whether each leaves to the right of the direction of travel.
    sides: Vec<(Arm, bool)>,
}

/// An X junction turned by 90 degrees: its side ports become the through ports and the other way round.
fn swapped(j: &Junction) -> Junction {
    let turn = |p: &Port| Port {
        heading: wrap(p.heading + PI),
        ..*p
    };
    let mut ports = j.side_ports.iter();
    let (Some(a), Some(b)) = (ports.next(), ports.next()) else {
        return j.clone();
    };
    Junction {
        model: j.model.clone(),
        through: j.side,
        side: j.through,
        start: turn(a),
        end: *b,
        side_ports: vec![turn(&j.start), j.end],
    }
}

fn wrap(a: f64) -> f64 {
    let r = a.rem_euclid(TAU);
    if r > PI { r - TAU } else { r }
}

fn rotate(dx: f64, dz: f64, theta: f64) -> (f64, f64) {
    let (s, c) = theta.sin_cos();
    (dx * c + dz * s, -dx * s + dz * c)
}

fn pose_of(port: &Port) -> Pose {
    Pose {
        x: port.x,
        y: port.z,
        heading: port.heading,
    }
}

/// Where a model sits: its origin and rotation.
#[derive(Clone, Copy)]
struct Frame {
    origin: Pose,
}

impl Frame {
    /// The frame in which the model port `entry` lies at `at`.
    fn through(entry: &Pose, at: Pose) -> Frame {
        let theta = wrap(at.heading - entry.heading);
        let (dx, dy) = rotate(entry.x, entry.y, theta);
        Frame {
            origin: Pose {
                x: at.x - dx,
                y: at.y - dy,
                heading: theta,
            },
        }
    }

    fn pose(&self, model: &Pose) -> Pose {
        let (dx, dy) = rotate(model.x, model.y, self.origin.heading);
        Pose {
            x: self.origin.x + dx,
            y: self.origin.y + dy,
            heading: wrap(model.heading + self.origin.heading),
        }
    }
}

/// The line of a run of links, with the arclength at which each link starts.
struct Polyline {
    points: Vec<(f64, f64)>,
    cum: Vec<f64>,
    link_start: Vec<f64>,
}

impl Polyline {
    fn total(&self) -> f64 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// The pose on the line at arclength `s`, heading along it.
    fn pose_at(&self, s: f64) -> Pose {
        let s = s.clamp(0.0, self.total());
        for (i, pair) in self.points.windows(2).enumerate() {
            let (Some(&s0), Some(&s1)) = (self.cum.get(i), self.cum.get(i + 1)) else {
                continue;
            };
            let [a, b] = pair else { continue };
            if s <= s1 && s1 > s0 {
                let t = (s - s0) / (s1 - s0);
                return Pose {
                    x: a.0 + t * (b.0 - a.0),
                    y: a.1 + t * (b.1 - a.1),
                    heading: (b.0 - a.0).atan2(b.1 - a.1),
                };
            }
        }
        let last = self.points.last().copied().unwrap_or_default();
        Pose {
            x: last.0,
            y: last.1,
            heading: 0.0,
        }
    }

    /// The part between two arclengths.
    fn slice(&self, from: f64, to: f64) -> Vec<(f64, f64)> {
        let (from, to) = (from.max(0.0), to.min(self.total()));
        let start = self.pose_at(from);
        let mut out = vec![(start.x, start.y)];
        for (p, &s) in self.points.iter().zip(&self.cum) {
            if s > from + 1e-9 && s < to - 1e-9 {
                out.push(*p);
            }
        }
        let end = self.pose_at(to);
        out.push((end.x, end.y));
        out
    }

    /// The arclength of the point of the line nearest to `at`.
    fn project(&self, at: (f64, f64)) -> f64 {
        let mut best = (f64::INFINITY, 0.0);
        for (i, pair) in self.points.windows(2).enumerate() {
            let (Some(&s0), Some(&s1)) = (self.cum.get(i), self.cum.get(i + 1)) else {
                continue;
            };
            let [a, b] = pair else { continue };
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len2 = dx * dx + dy * dy;
            let t = if len2 > 0.0 {
                (((at.0 - a.0) * dx + (at.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let d = (at.0 - a.0 - t * dx).hypot(at.1 - a.1 - t * dy);
            if d < best.0 {
                best = (d, s0 + t * (s1 - s0));
            }
        }
        best.1
    }

    fn link_of(&self, s: f64) -> usize {
        self.link_start
            .iter()
            .rposition(|&start| start <= s + 1e-9)
            .unwrap_or(0)
    }
}

/// How far the end of a fit that arrives under another road misses the window under that road's edge: 0 when
/// the end lies between [`MIN_UNDER_M`] and `edge` short of the crossing at the end of `line`, measured along
/// the road. Not the Euclidean distance: a fit can overshoot its path, and an end past the crossing projects
/// onto the end of the line, which is a miss.
fn end_violation(line: &Polyline, end: Pose, edge: f64) -> f64 {
    let d = line.total() - line.project((end.x, end.y));
    (d - edge).max(MIN_UNDER_M - d).max(0.0)
}

/// A forced element of a run.
enum Event {
    /// A junction piece, by index into the plans.
    Junction(usize),
    /// A run of bridge models along link `k` of the chain.
    Bridge(usize),
}

impl Junction {
    /// Half the length of the junction along the through road.
    fn half_length(&self) -> f64 {
        (self.end.x - self.start.x).hypot(self.end.z - self.start.z) / 2.0
    }
}

/// What placing a bridge produced: the road fitted up to it and the bridge models.
struct PlacedBridge {
    fit: Vec<FitPiece>,
    items: Vec<Item>,
}

/// Something placed, before its slot is known.
struct Item {
    road: PlacedRoad,
    /// Arclength on the run (a junction: of its crossing).
    s: f64,
    link: Option<usize>,
}

/// The analysis of the network and the state of the placement.
struct Placer<'a> {
    graph: &'a RoadGraph,
    catalog: &'a PieceCatalog,
    chains: Vec<Chain>,
    /// For each stretch, the chain that holds it.
    chain_of: Vec<usize>,
    /// Whether a chain is a placed roundabout: it lies on top of every road that meets it and closes on itself.
    ring_chain: Vec<bool>,
    /// The live (not tunnel) stretch ends at each node.
    arms_at: BTreeMap<i64, Vec<Arm>>,
    plans: Vec<Plan<'a>>,
    /// The first plan of each (chain, link), by index into `plans`.
    plan_index: BTreeMap<(usize, usize), usize>,
    /// Chains that have to be placed before a chain.
    needs: Vec<Vec<usize>>,
    /// Poses fixed by placed junctions: where a side arm starts, heading away from the through road.
    fixed: BTreeMap<Arm, Pose>,
    /// How much of the end of a side arm to leave out, because it ends on a through road without junction.
    trim: BTreeMap<Arm, f64>,
    /// The half width of the road that a trimmed arm ends under: the end has to lie within it.
    edge: BTreeMap<Arm, f64>,
    /// Link boundaries `(chain, k)` between link `k` and `k + 1` where the chain is interrupted: it is the
    /// lower road at an overlay and the higher road lies across it.
    cuts: BTreeSet<(usize, usize)>,
    overlay_points: BTreeSet<(u64, u64)>,
    out: PlacedNetwork,
    joins: BTreeSet<(u64, u64)>,
    /// How far each junction is moved along its through road, by `(chain, link)` of its plan.
    shifts: Shifts,
    /// The junction a fixed side arm starts at.
    fixed_from: BTreeMap<Arm, (usize, usize)>,
    /// The shifts found for junctions whose side road did not close, by `(chain, link)` of their plan: what
    /// the next placement is asked to do.
    found: Shifts,
    /// How far the through road's fit ended from the goal before each junction, by `(chain, link)`.
    through_gaps: Shifts,
}

fn bits(p: (f64, f64)) -> (u64, u64) {
    (p.0.to_bits(), p.1.to_bits())
}

impl<'a> Placer<'a> {
    fn new(graph: &'a RoadGraph, catalog: &'a PieceCatalog, shifts: Shifts) -> Self {
        // a ring smaller than the tightest arc of its family cannot be laid out: its stretches are roads (the
        // build has collapsed the rings below `ROUNDABOUT_KEEP_M` before, see `collapse_small_rings`)
        let rings: Vec<Ring> = rings(graph)
            .into_iter()
            .filter(|r| {
                catalog
                    .tightest_arc(r.family)
                    .is_some_and(|tightest| r.radius >= tightest)
            })
            .collect();
        let chains = chains_with(graph, &rings);
        let ring_stretches: BTreeSet<usize> = rings
            .iter()
            .flat_map(|r| r.stretches.iter().copied())
            .collect();
        let ring_chain: Vec<bool> = chains
            .iter()
            .map(|c| {
                !c.links.is_empty() && c.links.iter().all(|l| ring_stretches.contains(&l.stretch))
            })
            .collect();
        let mut chain_of = vec![0; graph.stretches.len()];
        for (c, chain) in chains.iter().enumerate() {
            for link in &chain.links {
                if let Some(slot) = chain_of.get_mut(link.stretch) {
                    *slot = c;
                }
            }
        }
        let mut arms_at: BTreeMap<i64, Vec<Arm>> = BTreeMap::new();
        for (i, s) in graph.stretches.iter().enumerate() {
            if s.tunnel {
                continue;
            }
            arms_at.entry(s.start).or_default().push((i, true));
            arms_at.entry(s.end).or_default().push((i, false));
        }
        let needs = vec![Vec::new(); chains.len()];
        Placer {
            graph,
            catalog,
            chains,
            chain_of,
            ring_chain,
            arms_at,
            plans: Vec::new(),
            plan_index: BTreeMap::new(),
            needs,
            fixed: BTreeMap::new(),
            trim: BTreeMap::new(),
            edge: BTreeMap::new(),
            cuts: BTreeSet::new(),
            overlay_points: BTreeSet::new(),
            out: PlacedNetwork {
                rings,
                ..PlacedNetwork::default()
            },
            joins: BTreeSet::new(),
            shifts,
            fixed_from: BTreeMap::new(),
            found: BTreeMap::new(),
            through_gaps: BTreeMap::new(),
        }
    }

    fn is_ring(&self, chain: usize) -> bool {
        self.ring_chain.get(chain).copied().unwrap_or(false)
    }

    /// Whether the road of chain `side` lies on top of the road of chain `through` where they meet without a
    /// junction piece: a ring over everything, else [`overlay_rank`].
    fn lies_over(&self, side: usize, through: usize) -> bool {
        match (self.is_ring(side), self.is_ring(through)) {
            (true, false) => true,
            (false, true) => false,
            _ => self
                .chains
                .get(side)
                .zip(self.chains.get(through))
                .is_some_and(|(a, b)| overlay_rank(a, b, self.graph) == Ordering::Less),
        }
    }

    fn join(&mut self, point: (f64, f64)) {
        if self.joins.insert(bits(point)) {
            self.out.joins_without_junction.push(point);
        }
    }

    /// The line of links `first..=last` of chain `c`.
    fn polyline(&self, c: usize, first: usize, last: usize) -> Polyline {
        let mut points: Vec<(f64, f64)> = Vec::new();
        let mut link_start = Vec::new();
        let mut cum: Vec<f64> = Vec::new();
        let mut total = 0.0;
        for link in self
            .chains
            .get(c)
            .map(|ch| ch.links.get(first..=last).unwrap_or_default())
            .unwrap_or_default()
        {
            let Some(s) = self.graph.stretches.get(link.stretch) else {
                continue;
            };
            let ordered: Vec<(f64, f64)> = if link.reversed {
                s.points.iter().rev().copied().collect()
            } else {
                s.points.clone()
            };
            link_start.push(total);
            for p in ordered {
                if let Some(prev) = points.last() {
                    let d = (p.0 - prev.0).hypot(p.1 - prev.1);
                    if d < 1e-9 {
                        continue;
                    }
                    total += d;
                }
                points.push(p);
                cum.push(total);
            }
        }
        Polyline {
            points,
            cum,
            link_start,
        }
    }

    /// Where a stretch end leaves its node, as a heading.
    fn leaving_heading(&self, arm: Arm) -> f64 {
        let Some(s) = self.graph.stretches.get(arm.0) else {
            return 0.0;
        };
        let along: Vec<(f64, f64)> = if arm.1 {
            s.points.clone()
        } else {
            s.points.iter().rev().copied().collect()
        };
        let from = along.first().copied().unwrap_or_default();
        let target = along
            .iter()
            .copied()
            .find(|p| (p.0 - from.0).hypot(p.1 - from.1) >= 4.0)
            .or_else(|| along.last().copied())
            .unwrap_or(from);
        (target.0 - from.0).atan2(target.1 - from.1)
    }

    fn family_width(&self, family: Family) -> f64 {
        self.catalog.pieces(family).first().map_or(0.0, |p| p.width)
    }

    /// Decides junctions, joins without a junction, trims and the order of the chains.
    fn analyse(&mut self) {
        let mut seen_nodes: BTreeSet<i64> = BTreeSet::new();
        for c in 0..self.chains.len() {
            let Some(chain) = self.chains.get(c).cloned() else {
                continue;
            };
            let line = self.polyline(c, 0, chain.links.len().saturating_sub(1));
            let mut last_junction: Option<f64> = None;
            // a ring has one more crossing: where it closes
            let closed = self.is_ring(c);
            let crossings = if closed {
                chain.links.len()
            } else {
                chain.links.len().saturating_sub(1)
            };
            for i in 0..crossings {
                let next = (i + 1) % chain.links.len().max(1);
                let (Some(a), Some(b)) = (chain.links.get(i), chain.links.get(next)) else {
                    continue;
                };
                let exit: Arm = (a.stretch, a.reversed);
                let entry: Arm = (b.stretch, !b.reversed);
                let node = self.node_of(exit);
                seen_nodes.insert(node);
                let sides: Vec<Arm> = self
                    .arms_at
                    .get(&node)
                    .map(|v| {
                        v.iter()
                            .copied()
                            .filter(|x| *x != exit && *x != entry)
                            .collect()
                    })
                    .unwrap_or_default();
                if sides.is_empty() {
                    continue;
                }
                let s_cross = if next == 0 {
                    line.total()
                } else {
                    line.link_start.get(next).copied().unwrap_or(0.0)
                };
                let point = line.pose_at(s_cross);
                let at = (point.x, point.y);
                let too_close = last_junction.is_some_and(|l| s_cross - l < JUNCTION_SPACING_M);
                let beside_bridge = [a.stretch, b.stretch].iter().any(|i| {
                    self.graph
                        .stretches
                        .get(*i)
                        .is_some_and(|s| s.tunnel || s.bridge)
                });
                let plan = if too_close || beside_bridge || closed {
                    None
                } else {
                    self.plan_junction(c, i, at, point.heading, &chain, &sides)
                };
                if let Some(plan) = plan {
                    last_junction = Some(s_cross);
                    for (arm, _) in &plan.sides {
                        let d = self.chain_of.get(arm.0).copied().unwrap_or(c);
                        if d != c
                            && let Some(n) = self.needs.get_mut(d)
                            && !n.contains(&c)
                        {
                            n.push(c);
                        }
                    }
                    self.plan_index
                        .entry((plan.chain, plan.link))
                        .or_insert(self.plans.len());
                    self.plans.push(plan);
                } else {
                    self.overlay_at(c, i, at, &sides, true);
                }
            }
        }
        // crossings that no chain passes through and where three or more roads meet
        let loose: Vec<(i64, Vec<Arm>)> = self
            .arms_at
            .iter()
            .filter(|(n, arms)| !seen_nodes.contains(n) && arms.len() >= 3)
            .map(|(n, a)| (*n, a.clone()))
            .collect();
        for (_, arms) in loose {
            if let Some(p) = arms.first().and_then(|a| self.arm_point(*a)) {
                self.join(p);
            }
        }
        self.out
            .joins_without_junction
            .sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        self.out
            .overlays
            .sort_by(|a, b| a.at.0.total_cmp(&b.at.0).then(a.at.1.total_cmp(&b.at.1)));
    }

    fn node_of(&self, arm: Arm) -> i64 {
        self.graph
            .stretches
            .get(arm.0)
            .map_or(0, |s| if arm.1 { s.start } else { s.end })
    }

    fn arm_point(&self, arm: Arm) -> Option<(f64, f64)> {
        let s = self.graph.stretches.get(arm.0)?;
        if arm.1 {
            s.points.first()
        } else {
            s.points.last()
        }
        .copied()
    }

    /// How far from the crossing a road that ends under a road of half width `half` stops: 2 m inside its
    /// edge, but not nearer to the centre line than half of `half`.
    fn cut_depth(half: f64) -> f64 {
        (half - SIDE_OVERLAP_M).max(half / 2.0)
    }

    /// Notes that the end of `arm` lies under a road of half width `half`: it stops [`Self::cut_depth`]
    /// from the crossing. A bridge is not shortened.
    fn end_under(&mut self, arm: Arm, half: f64) {
        if self.graph.stretches.get(arm.0).is_some_and(|s| s.bridge) {
            return;
        }
        self.trim.insert(arm, Self::cut_depth(half));
        self.edge.insert(arm, half);
    }

    fn stretch_id(&self, stretch: usize) -> Option<String> {
        self.graph.stretches.get(stretch).map(|s| s.id.clone())
    }

    /// Roads that meet at a crossing of the chain `c` (between its links `boundary` and `boundary + 1`)
    /// where the game has no junction piece: the road that lies on top runs on, the others end under its
    /// edge. Where the side roads are all the higher ones and cross the chain, the chain is interrupted
    /// (`may_cut`); otherwise a higher road that ends on it lies on top of it and the chain runs on.
    fn overlay_at(
        &mut self,
        c: usize,
        boundary: usize,
        at: (f64, f64),
        sides: &[Arm],
        may_cut: bool,
    ) {
        let Some(chain) = self.chains.get(c).cloned() else {
            return;
        };
        let next = (boundary + 1) % chain.links.len().max(1);
        let (Some(a), Some(b)) = (chain.links.get(boundary), chain.links.get(next)) else {
            return;
        };
        let exit: Arm = (a.stretch, a.reversed);
        let entry: Arm = (b.stretch, !b.reversed);
        let half_of = |this: &Self, stretch: usize| {
            this.graph
                .stretches
                .get(stretch)
                .map_or(0.0, |s| this.family_width(s.family) / 2.0)
        };
        let outranks = |this: &Self, arm: Arm| {
            let d = this.chain_of.get(arm.0).copied().unwrap_or(c);
            d != c && this.lies_over(d, c)
        };
        let plain = |this: &Self, stretch: usize| {
            this.graph
                .stretches
                .get(stretch)
                .is_some_and(|s| !s.bridge && !s.tunnel)
        };
        let mut lower: Vec<String> = Vec::new();
        // the higher roads cross only when they leave on opposite sides; two on one side are a fork,
        // and cutting the chain there would leave a hole on the far side
        let through = self.leaving_heading(entry);
        let side_of = |this: &Self, arm: Arm| (this.leaving_heading(arm) - through).sin() > 0.0;
        let crossing = sides.len() == 2
            && sides
                .first()
                .zip(sides.get(1))
                .is_some_and(|(p, q)| side_of(self, *p) != side_of(self, *q))
            && sides.iter().all(|arm| outranks(self, *arm))
            && plain(self, a.stretch)
            && plain(self, b.stretch);
        if may_cut && crossing {
            // the higher road crosses: this road stops under its edge on both sides
            let half = sides
                .iter()
                .map(|arm| half_of(self, arm.0))
                .fold(0.0, f64::max);
            for arm in [exit, entry] {
                self.end_under(arm, half);
            }
            self.cuts.insert((c, boundary));
            lower.extend(
                [self.stretch_id(a.stretch), self.stretch_id(b.stretch)]
                    .into_iter()
                    .flatten(),
            );
        } else {
            // this road runs on: the side roads end on it (on top of it if they are the higher ones)
            let half = half_of(self, a.stretch);
            let mut under = false;
            for arm in sides {
                self.end_under(*arm, half);
                if self.chain_of.get(arm.0) == Some(&c) {
                    // the other end of this very chain (a ring): it is not another road
                } else if outranks(self, *arm) {
                    under = true;
                } else {
                    lower.extend(self.stretch_id(arm.0));
                }
            }
            if under {
                lower.extend(
                    [self.stretch_id(a.stretch), self.stretch_id(b.stretch)]
                        .into_iter()
                        .flatten(),
                );
            }
        }
        lower.sort();
        lower.dedup();
        if self.overlay_points.insert(bits(at)) {
            self.out.overlays.push(Overlay { at, lower });
        }
    }

    fn plan_junction(
        &self,
        chain_index: usize,
        link: usize,
        at: (f64, f64),
        through_heading: f64,
        chain: &Chain,
        sides: &[Arm],
    ) -> Option<Plan<'a>> {
        let through = self
            .graph
            .stretches
            .get(chain.links.get(link)?.stretch)?
            .family;
        let arm_side = |arm: Arm| (self.leaving_heading(arm) - through_heading).sin() > 0.0;
        let family_of = |arm: Arm| self.graph.stretches.get(arm.0).map(|s| s.family);
        let junction = match sides {
            [one] => Cow::Borrowed(self.catalog.t_junction(through, family_of(*one)?)?),
            [one, two] => {
                if family_of(*one)? != family_of(*two)? || arm_side(*one) == arm_side(*two) {
                    return None;
                }
                let cross = family_of(*one)?;
                match self.catalog.x_junction(through, cross) {
                    Some(j) => Cow::Borrowed(j),
                    // an X looks the same either way round: the piece with the roles swapped,
                    // turned so that its through ports lie on the crossing arms
                    None => Cow::Owned(swapped(self.catalog.x_junction(cross, through)?)),
                }
            }
            _ => return None,
        };
        Some(Plan {
            chain: chain_index,
            link,
            point: at,
            junction,
            sides: sides.iter().map(|a| (*a, arm_side(*a))).collect(),
        })
    }

    fn plan_at(&self, chain: usize, link: usize) -> Option<usize> {
        self.plan_index.get(&(chain, link)).copied()
    }

    fn live_arms(&self, node: i64) -> usize {
        self.arms_at.get(&node).map_or(0, Vec::len)
    }

    fn pieces(&self, family: Family) -> &[RoadPiece] {
        self.catalog.pieces(family)
    }

    fn cap(&self, family: Family) -> Option<&RoadPiece> {
        self.pieces(family)
            .iter()
            .find(|p| p.kind == PieceKind::EndCap)
    }

    /// Places chain `c` after the chains it depends on.
    fn place_chain(&mut self, c: usize, state: &mut [u8]) -> Result<(), PlaceError> {
        if state.get(c).copied().unwrap_or(2) != 0 {
            return Ok(());
        }
        if let Some(s) = state.get_mut(c) {
            *s = 1;
        }
        for d in self.needs.get(c).cloned().unwrap_or_default() {
            self.place_chain(d, state)?;
        }
        let links = self.chains.get(c).map_or(0, |ch| ch.links.len());
        let mut first = 0;
        while first < links {
            let tunnel = |k: usize| {
                self.chains
                    .get(c)
                    .and_then(|ch| ch.links.get(k))
                    .and_then(|l| self.graph.stretches.get(l.stretch))
                    .is_some_and(|s| s.tunnel)
            };
            if tunnel(first) {
                if let Some(s) = self
                    .chains
                    .get(c)
                    .and_then(|ch| ch.links.get(first))
                    .and_then(|l| self.graph.stretches.get(l.stretch))
                {
                    let at = s.points.first().copied().unwrap_or_default();
                    self.out.tunnels_omitted.push((s.id.clone(), at));
                }
                first += 1;
                continue;
            }
            let mut last = first;
            while last + 1 < links && !tunnel(last + 1) && !self.cuts.contains(&(c, last)) {
                last += 1;
            }
            self.place_unit(c, first, last)?;
            first = last + 1;
        }
        if let Some(s) = state.get_mut(c) {
            *s = 2;
        }
        Ok(())
    }

    fn report_gap(&mut self, stretch: &str, goal: Pose, end: Pose) {
        let gap = (end.x - goal.x).hypot(end.y - goal.y);
        if gap > GAP_TOLERATED_M {
            self.out.gaps.push(RoadGap {
                stretch: stretch.to_string(),
                at: (goal.x, goal.y),
                gap_m: gap,
                heading_error_deg: wrap(end.heading - goal.heading).abs().to_degrees(),
            });
        }
    }

    /// Reports a road that meets a bridge's end more than [`HEADING_MISS_DEG`] off the deck's heading, at the
    /// bridge `stretch`'s port `goal`: a kink (not a gap, not counted when junctions are moved).
    fn report_kink(&mut self, stretch: &str, goal: Pose, end: Pose) {
        let heading_error = wrap(end.heading - goal.heading).abs().to_degrees();
        if heading_error > HEADING_MISS_DEG {
            self.out.kinks.push(RoadKink {
                stretch: stretch.to_string(),
                at: (goal.x, goal.y),
                heading_error_deg: heading_error,
            });
        }
    }

    fn fit_part(
        &mut self,
        path: &[(f64, f64)],
        pose: Pose,
        goal: EndGoal,
        family: Family,
        stretch: &str,
    ) -> Result<(Vec<FitPiece>, Pose), PlaceError> {
        let fit =
            fit_line(path, pose, goal, self.pieces(family)).map_err(|source| PlaceError::Fit {
                stretch: stretch.to_string(),
                source,
            })?;
        let end = fit.pieces.last().map_or(pose, |p| p.end);
        if let EndGoal::Fixed(g) = goal {
            self.report_gap(stretch, g, end);
        }
        Ok((fit.pieces, end))
    }

    /// Places a run of links without tunnels.
    #[expect(
        clippy::too_many_lines,
        reason = "one pass over the parts of a run, kept together"
    )]
    fn place_unit(&mut self, c: usize, first: usize, last: usize) -> Result<(), PlaceError> {
        let line = self.polyline(c, first, last);
        let links: Vec<_> = self
            .chains
            .get(c)
            .map(|ch| ch.links.get(first..=last).unwrap_or_default().to_vec())
            .unwrap_or_default();
        let (Some(head), Some(tail)) = (links.first(), links.last()) else {
            return Ok(());
        };
        let Some(first_stretch) = self.graph.stretches.get(head.stretch) else {
            return Ok(());
        };
        let family = first_stretch.family;
        let drivable = first_stretch.drivable;
        let entry_arm: Arm = (head.stretch, !head.reversed);
        let exit_arm: Arm = (tail.stretch, tail.reversed);
        let (start_node, end_node) = (self.node_of(entry_arm), self.node_of(exit_arm));
        let whole_ring = self.is_ring(c)
            && first == 0
            && self
                .chains
                .get(c)
                .is_some_and(|ch| ch.links.len() == links.len());
        let ring = start_node == end_node
            && (whole_ring || (links.len() == 1 && self.live_arms(start_node) == 2));
        let total = line.total();
        let trim_start = self.trim.get(&entry_arm).copied().unwrap_or(0.0);
        let trim_end = self.trim.get(&exit_arm).copied().unwrap_or(0.0);
        let fixed_start = self.fixed.get(&entry_arm).copied();
        let fixed_end = self.fixed.get(&exit_arm).copied();
        let cap_start = fixed_start.is_none() && start_node >= 0 && self.live_arms(start_node) == 1;
        let cap_end = fixed_end.is_none() && end_node >= 0 && self.live_arms(end_node) == 1;
        let width = self.pieces(family).first().map_or(0.0, |p| p.width);
        let only_roads = links.iter().all(|l| {
            self.graph
                .stretches
                .get(l.stretch)
                .is_some_and(|s| !s.bridge)
        });
        if only_roads
            && !ring
            && fixed_start.is_none()
            && fixed_end.is_none()
            && trim_start + trim_end > 0.0
            && total - trim_start - trim_end < width / 2.0
        {
            // what is left of the road between the roads it ends under is hardly longer than a piece is wide
            let id = self
                .graph
                .stretches
                .get(head.stretch)
                .map(|s| s.id.clone())
                .unwrap_or_default();
            self.out.warnings.push(format!(
                "stretch {id} is {total:.1} m long and ends under other roads: it is shorter than half its width and not placed"
            ));
            return Ok(());
        }

        let mut items: Vec<Item> = Vec::new();
        let start_pose = fixed_start.unwrap_or_else(|| line.pose_at(trim_start));
        let mut pose = start_pose;
        // a side road starts at a port, half a junction away from the crossing at the end of its line
        let mut s_cur = if fixed_start.is_some() {
            JUNCTION_SPACING_M / 2.0
        } else {
            trim_start
        };
        let make = |piece: &FitPiece, kind: PlacedKind, s: f64, link: Option<usize>| Item {
            road: PlacedRoad {
                slot: Slot {
                    stretch: String::new(),
                    index: 0,
                },
                kind,
                family,
                drivable,
                model: piece.model.clone(),
                origin: piece.origin,
                start: piece.start,
                end: piece.end,
                width,
                scale: 1.0,
                abreast: 1,
            },
            s,
            link,
        };
        // the cap at the start: reversed, so that its open end meets the road
        if cap_start && let Some(cap) = self.cap(family).cloned() {
            {
                let back = cap.reversed();
                let frame = Frame::through(&pose_of(&back.end), start_pose);
                let piece = FitPiece {
                    model: cap.model.clone(),
                    origin: frame.origin,
                    start: frame.pose(&pose_of(&back.start)),
                    end: frame.pose(&pose_of(&back.end)),
                };
                items.push(make(&piece, PlacedKind::EndCap, 0.0, Some(0)));
            }
        }
        // the forced elements of the run, in order: junctions and bridges
        let crossings = self.crossings(c, first, last, &line);
        let mut events: Vec<(f64, Event)> = (first..last)
            .filter_map(|k| self.plan_at(c, k))
            .filter_map(|plan_id| {
                let plan = self.plans.get(plan_id)?;
                let s_cross = crossings.get(&plan_id).copied()?;
                Some((
                    s_cross - plan.junction.half_length(),
                    Event::Junction(plan_id),
                ))
            })
            .collect();
        for k in first..=last {
            if self.bridge_chord(c, k).is_some_and(|chord| chord > 0.0)
                && let Some(s0) = line.link_start.get(k - first)
            {
                events.push((*s0, Event::Bridge(k)));
            }
        }
        events.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut dropped: Vec<usize> = Vec::new();
        let starts: Vec<f64> = events.iter().map(|e| e.0).collect();
        for (i, (s_event, event)) in events.into_iter().enumerate() {
            let plan_id = match event {
                Event::Junction(plan_id) => plan_id,
                Event::Bridge(k) => {
                    let placed = self.place_bridge(
                        &line,
                        (c, first),
                        k - first,
                        family,
                        drivable,
                        &mut pose,
                        &mut s_cur,
                        starts.get(i + 1).copied().unwrap_or_else(|| line.total()) - s_event - 1.0,
                    )?;
                    for p in &placed.fit {
                        let s = line.project((
                            f64::midpoint(p.start.x, p.end.x),
                            f64::midpoint(p.start.y, p.end.y),
                        ));
                        items.push(make(p, PlacedKind::Piece, s, None));
                    }
                    for mut item in placed.items {
                        item.link = Some(k - first);
                        items.push(item);
                    }
                    continue;
                }
            };
            let Some((k, junction)) = self
                .plans
                .get(plan_id)
                .map(|p| (p.link, p.junction.clone()))
            else {
                continue;
            };
            let s_cross = crossings.get(&plan_id).copied().unwrap_or(0.0);
            let half = junction.half_length();
            let s_goal = s_cross - half;
            let stretch_id = links
                .get(k - first)
                .and_then(|l| self.graph.stretches.get(l.stretch))
                .map(|s| s.id.clone())
                .unwrap_or_default();
            if s_goal - s_cur > 1.0 {
                let path = line.slice(s_cur, s_goal);
                let goal = line.pose_at(s_goal);
                let (pieces, end) =
                    self.fit_part(&path, pose, EndGoal::Fixed(goal), family, &stretch_id)?;
                if let Some(plan) = self.plans.get(plan_id) {
                    let key = (plan.chain, plan.link);
                    self.through_gaps
                        .insert(key, (end.x - goal.x).hypot(end.y - goal.y));
                }
                for p in &pieces {
                    let s = line.project((
                        f64::midpoint(p.start.x, p.end.x),
                        f64::midpoint(p.start.y, p.end.y),
                    ));
                    items.push(make(p, PlacedKind::Piece, s, None));
                }
                pose = end;
                s_cur = s_goal;
            }
            // place the junction at the end of the fit
            let Some(plan) = self.plans.get(plan_id) else {
                continue;
            };
            let crossing = line
                .link_start
                .get(k + 1 - first)
                .copied()
                .unwrap_or(s_cross);
            let at = if (s_cross - crossing).abs() < 1e-9 {
                plan.point
            } else {
                let moved = line.pose_at(s_cross);
                (moved.x, moved.y)
            };
            let placed = Self::place_junction(plan, at, pose, line.pose_at(s_cross).heading);
            match placed {
                Some((frame, flipped, exit)) => {
                    let (entry_port, exit_port) = if flipped {
                        (flip(&junction.end), flip(&junction.start))
                    } else {
                        (pose_of(&junction.start), pose_of(&junction.end))
                    };
                    let piece = FitPiece {
                        model: junction.model.clone(),
                        origin: frame.origin,
                        start: frame.pose(&entry_port),
                        end: frame.pose(&exit_port),
                    };
                    items.push(make(&piece, PlacedKind::Junction, s_cross, Some(k - first)));
                    // the side ports become the fixed starts of the side roads
                    let ports = self.side_poses(plan_id, &frame);
                    let key = (plan.chain, plan.link);
                    for (arm, port) in ports {
                        self.fixed.insert(arm, port);
                        self.fixed_from.insert(arm, key);
                    }
                    pose = exit;
                    s_cur = s_cross + half;
                }
                None => dropped.push(plan_id),
            }
        }
        for plan_id in dropped {
            if let Some(plan) = self.plans.get(plan_id) {
                let at = plan.point;
                let sides: Vec<Arm> = plan.sides.iter().map(|s| s.0).collect();
                let boundary = plan.link;
                // this chain is placed already: it cannot be interrupted any more
                self.overlay_at(c, boundary, at, &sides, false);
            }
        }
        // the last part
        let stretch_id = self
            .graph
            .stretches
            .get(tail.stretch)
            .map(|s| s.id.clone())
            .unwrap_or_default();
        let (mut path, goal) = if ring {
            (line.slice(s_cur, total), EndGoal::Fixed(start_pose))
        } else if let Some(port) = fixed_end {
            // arrive at the port of a junction on the road at the other end
            let s_end = (total - JUNCTION_SPACING_M / 2.0)
                .max(s_cur + 1.0)
                .min(total);
            let mut path = line.slice(s_cur, s_end);
            path.push((port.x, port.y));
            let arrive = Pose {
                heading: wrap(port.heading + PI),
                ..port
            };
            (path, EndGoal::Fixed(arrive))
        } else {
            let s_end = (total - trim_end).max(s_cur);
            (line.slice(s_cur, s_end), EndGoal::Free)
        };
        if trim_end > 0.0
            && let Some(&edge) = self.edge.get(&exit_arm)
            && !ring
            && fixed_end.is_none()
            && path.len() >= 2
        {
            // the road arrives under another one: the pieces cannot be steered to a metre, so the cut is
            // moved until the last piece ends under the edge of the road above, short of its centre line
            let mut best: Option<(f64, Vec<FitPiece>, Pose)> = None;
            let depths =
                std::iter::once(Self::cut_depth(edge)).chain(CUT_TRIES.iter().map(|f| edge * f));
            for depth in depths {
                let s_end = (total - depth).max(s_cur);
                let mut trial = line.slice(s_cur, s_end);
                trial.dedup();
                if trial.len() < 2 {
                    continue;
                }
                let (pieces, end) =
                    self.fit_part(&trial, pose, EndGoal::Free, family, &stretch_id)?;
                let violation = end_violation(&line, end, edge);
                let better = best.as_ref().is_none_or(|(v, _, _)| violation < *v);
                if better {
                    best = Some((violation, pieces, end));
                }
                if violation <= 0.0 {
                    break;
                }
            }
            if let Some((_, pieces, end)) = best {
                for p in &pieces {
                    let s = line.project((
                        f64::midpoint(p.start.x, p.end.x),
                        f64::midpoint(p.start.y, p.end.y),
                    ));
                    items.push(make(p, PlacedKind::Piece, s, None));
                }
                pose = end;
                path = Vec::new();
            }
        }
        if path.len() >= 2 && path.windows(2).any(|p| matches!(p, [a, b] if a != b)) {
            path.dedup();
            let before = pose;
            let (pieces, end) = self.fit_part(&path, pose, goal, family, &stretch_id)?;
            if let EndGoal::Fixed(arrive) = goal
                && fixed_end.is_some()
                && !ring
            {
                let gap = (end.x - arrive.x).hypot(end.y - arrive.y);
                if gap > GAP_TOLERATED_M {
                    self.repair_port(exit_arm, &path, before, family, gap);
                }
            }
            for p in &pieces {
                let s = line.project((
                    f64::midpoint(p.start.x, p.end.x),
                    f64::midpoint(p.start.y, p.end.y),
                ));
                items.push(make(p, PlacedKind::Piece, s, None));
            }
            pose = end;
        }
        if cap_end && let Some(cap) = self.cap(family).cloned() {
            {
                let frame = Frame::through(&pose_of(&cap.start), pose);
                let piece = FitPiece {
                    model: cap.model.clone(),
                    origin: frame.origin,
                    start: frame.pose(&pose_of(&cap.start)),
                    end: frame.pose(&pose_of(&cap.end)),
                };
                items.push(make(
                    &piece,
                    PlacedKind::EndCap,
                    total,
                    Some(links.len() - 1),
                ));
            }
        }
        self.finish_unit(&links, &line, items, cap_start, cap_end);
        Ok(())
    }

    /// The straight distance between the ends of link `k` of chain `c`, if it is a bridge.
    fn bridge_chord(&self, c: usize, k: usize) -> Option<f64> {
        let link = self.chains.get(c)?.links.get(k)?;
        let stretch = self.graph.stretches.get(link.stretch)?;
        if !stretch.bridge || stretch.tunnel {
            return None;
        }
        let (first, last) = (stretch.points.first()?, stretch.points.last()?);
        Some((last.0 - first.0).hypot(last.1 - first.1))
    }

    /// Fits the road up to link `k` of the run (which starts at link `first` of chain `c`) and lays the bridge models along its chord.
    #[expect(clippy::too_many_arguments, reason = "the state of one run, passed on")]
    fn place_bridge(
        &mut self,
        line: &Polyline,
        (c, first): (usize, usize),
        k: usize,
        family: Family,
        drivable: bool,
        pose: &mut Pose,
        s_cur: &mut f64,
        deck_limit: f64,
    ) -> Result<PlacedBridge, PlaceError> {
        let mut out = PlacedBridge {
            fit: Vec::new(),
            items: Vec::new(),
        };
        let s0 = line.link_start.get(k).copied().unwrap_or(0.0);
        let s1 = line
            .link_start
            .get(k + 1)
            .copied()
            .unwrap_or_else(|| line.total());
        let (p0, p1) = (line.pose_at(s0), line.pose_at(s1));
        let heading = (p1.x - p0.x).atan2(p1.y - p0.y);
        let chord = (p1.x - p0.x).hypot(p1.y - p0.y);
        let chained = self.chains.get(c).and_then(|ch| ch.links.get(first + k));
        let stretch = chained.and_then(|l| self.graph.stretches.get(l.stretch));
        let id = stretch.map(|s| s.id.clone()).unwrap_or_default();
        let reversed = chained.is_some_and(|l| l.reversed);
        if let Some(BridgeKind::Panels {
            count,
            abreast,
            scale,
        }) = stretch.and_then(|s| s.bridge_kind.as_ref())
        {
            let (count, abreast, scale) = (*count, *abreast, *scale);
            return self.place_panels(
                line,
                k,
                (family, drivable),
                (count, abreast, scale),
                (pose, s_cur),
                &id,
            );
        }
        let fitted = match stretch.and_then(|s| s.bridge_kind.as_ref()) {
            Some(BridgeKind::Model { count, .. }) => Some(*count),
            _ => None,
        };
        let model = match stretch {
            Some(s) => bridge_model(self.catalog, s),
            None => longest_bridge(self.catalog, family),
        };
        let Some(model) = model else {
            return Ok(out);
        };
        let length = model.length;
        let (count, shift) = match fitted {
            Some(count) => {
                let count = f64::from(u32::try_from(count).unwrap_or(u32::MAX)).max(1.0);
                let part = count * length;
                (
                    count,
                    Self::offset_in_link(
                        stretch.map_or(0.0, |s| s.bridge_offset),
                        (s1 - s0) - part,
                        reversed,
                    ),
                )
            }
            None => ((chord / length).round().max(1.0), 0.0),
        };
        // a model lies on the straight line of its bridge, shifted along it where it is shorter than it
        let s0 = s0 + shift;
        let p0 = Pose {
            x: p0.x + shift * heading.sin(),
            y: p0.y + shift * heading.cos(),
            heading,
        };
        // the model lies the way round the stretch is stored: its first port at the stretch's start
        let (entry, exit) = if reversed {
            (flip(&model.end), flip(&model.start))
        } else {
            (pose_of(&model.start), pose_of(&model.end))
        };
        if s0 - *s_cur > 1.0 {
            // the deck lies on its line; the approach closes on the deck's port
            let (pieces, end, port) = self.fit_approach(
                line,
                (*s_cur, s0),
                (p0, heading),
                (count * length, deck_limit),
                *pose,
                family,
                &id,
            )?;
            out.fit = pieces;
            *pose = port.unwrap_or(end);
            self.report_kink(&id, p0, *pose);
        } else if (pose.x - p0.x).hypot(pose.y - p0.y) < 1.0 {
            self.report_kink(&id, p0, *pose);
            pose.heading = heading;
        }
        out.items = Self::lay_models(model, count, (entry, exit), (family, drivable), (pose, s0));
        *s_cur = if fitted.is_some() {
            (s0 + count * length).min(s1)
        } else {
            s1
        };
        Ok(out)
    }

    /// `count` copies of `model` end to end from `pose` (a model's first port at the pose, `entry` and `exit`
    /// being the ports in the order the road is driven), `s0` being the arclength of the first.
    fn lay_models(
        model: &BridgeModel,
        count: f64,
        (entry, exit): (Pose, Pose),
        (family, drivable): (Family, bool),
        (pose, s0): (&mut Pose, f64),
    ) -> Vec<Item> {
        let mut items = Vec::new();
        let mut laid = 0.0;
        while laid < count {
            let frame = Frame::through(&entry, *pose);
            let start = frame.pose(&entry);
            let end = frame.pose(&exit);
            items.push(Item {
                road: PlacedRoad {
                    slot: Slot {
                        stretch: String::new(),
                        index: 0,
                    },
                    kind: PlacedKind::Bridge,
                    family,
                    drivable,
                    model: model.model.clone(),
                    origin: frame.origin,
                    start,
                    end,
                    width: model.width,
                    scale: 1.0,
                    abreast: 1,
                },
                s: s0 + laid * model.length,
                link: None,
            });
            *pose = end;
            laid += 1.0;
        }
        items
    }

    /// How far from the start of the link, in driving direction, a model `gap` shorter than the stretch starts:
    /// where [`crate::bridgekind::fit_bridges`] put it (`offset` from the stored start), counted from the other
    /// end where the link is walked against the stored direction.
    fn offset_in_link(offset: f64, gap: f64, reversed: bool) -> f64 {
        if !(gap > 0.0 && gap.is_finite()) {
            return 0.0;
        }
        let stored = offset.clamp(0.0, gap);
        if reversed { gap - stored } else { stored }
    }

    /// Lays a grid of `count` steps along link `k` of the run, each of `abreast` panels `scale` times the size
    /// of the panel: the steps share the line of the bridge from its start to its end evenly, each on the
    /// line's heading at its centre. The road is fitted up to the start of the grid; the grid has no road
    /// under it.
    fn place_panels(
        &mut self,
        line: &Polyline,
        k: usize,
        (family, drivable): (Family, bool),
        (count, abreast, scale): (usize, usize, f64),
        (pose, s_cur): (&mut Pose, &mut f64),
        id: &str,
    ) -> Result<PlacedBridge, PlaceError> {
        let mut out = PlacedBridge {
            fit: Vec::new(),
            items: Vec::new(),
        };
        let Some(slab) = crate::bridgekind::panel_slab(self.catalog) else {
            return Ok(out);
        };
        let s0 = line.link_start.get(k).copied().unwrap_or(0.0);
        let s1 = line
            .link_start
            .get(k + 1)
            .copied()
            .unwrap_or_else(|| line.total());
        let steps = f64::from(u32::try_from(count.max(1)).unwrap_or(u32::MAX));
        let step = (s1 - s0) / steps;
        if !(step.is_finite() && step > 0.0) {
            return Ok(out);
        }
        // every step runs chord to chord: from the line's point at its start to the one at its end, on the
        // heading of that chord; the grid's two ends are ports with the heading of the first and last step
        let chord = |i: usize| {
            let n = f64::from(u32::try_from(i).unwrap_or(u32::MAX));
            let (from, to) = (
                line.pose_at(s0 + n * step),
                line.pose_at(s0 + (n + 1.0) * step),
            );
            let heading = (to.x - from.x).atan2(to.y - from.y);
            (Pose { heading, ..from }, Pose { heading, ..to })
        };
        let first_step = chord(0).0;
        let last_step = chord(count.max(1) - 1).1;
        if s0 - *s_cur > 1.0 {
            // the approach is fitted to the grid's start on its own heading (with a lead that is the deck's axis),
            // so that it closes in position; what turn is left is a kink
            let path = line.slice(*s_cur, s0);
            let arrive = match path.as_slice() {
                [.., a, b] if a != b => Pose {
                    heading: (b.0 - a.0).atan2(b.1 - a.1),
                    ..first_step
                },
                _ => first_step,
            };
            let (pieces, end) = self.fit_part(&path, *pose, EndGoal::Fixed(arrive), family, id)?;
            self.report_kink(id, first_step, end);
            out.fit = pieces;
            *pose = end;
        } else {
            // no approach: the road is where the chain is, and the grid starts on the line; say so if they differ
            self.report_gap(id, first_step, *pose);
            self.report_kink(id, first_step, *pose);
        }
        let turn = if slab.along_x { PI / 2.0 } else { 0.0 };
        let width =
            f64::from(u32::try_from(abreast.max(1)).unwrap_or(u32::MAX)) * slab.across * scale;
        for i in 0..count.max(1) {
            let (from, to) = chord(i);
            let n = f64::from(u32::try_from(i).unwrap_or(u32::MAX));
            out.items.push(Item {
                road: PlacedRoad {
                    slot: Slot {
                        stretch: String::new(),
                        index: 0,
                    },
                    kind: PlacedKind::Bridge,
                    family,
                    drivable,
                    model: slab.model.clone(),
                    origin: Pose {
                        x: f64::midpoint(from.x, to.x),
                        y: f64::midpoint(from.y, to.y),
                        heading: wrap(from.heading + turn),
                    },
                    start: from,
                    end: to,
                    width,
                    scale,
                    abreast: abreast.max(1),
                },
                s: s0 + (n + 0.5) * step,
                link: None,
            });
        }
        *pose = last_step;
        *s_cur = s1;
        Ok(out)
    }

    /// Fits the road from `pose` up to the port of a deck that starts at `deck` (a point on the line of the
    /// bridge, with the heading of its chord) at `s0`, `from_to` being where on the line the fit runs.
    /// A fit ends a degree or two off a heading, and a deck laid along that heading leaves its line (3.5 m
    /// over 100 m in the test of research 9.6): so the deck is laid on its line and slides along it in
    /// [`BRIDGE_SLIDE_STEP_M`] steps, the shorter slide first and `+` before `-`, until the approach ends
    /// within [`GAP_CLOSED_M`] and [`KINK_TOLERATED_DEG`] of the port. `reach` is the length of the deck and
    /// the room after its start on the line (the deck does not slide into what follows). Returns the pieces,
    /// the pose where they end and, if they closed on the port, the port, which is where the deck starts then.
    /// If no slide closes, or the fit to the deck's own place arrives more than [`BRIDGE_TURN_MAX_DEG`] off the
    /// chord (the road turns into the bridge), that fit is returned and the deck starts where it ends, as
    /// before: a deck moved further would stand on other ground.
    #[expect(
        clippy::too_many_arguments,
        reason = "the state of one approach, passed on"
    )]
    fn fit_approach(
        &mut self,
        line: &Polyline,
        from_to: (f64, f64),
        deck: (Pose, f64),
        reach: (f64, f64),
        pose: Pose,
        family: Family,
        stretch: &str,
    ) -> Result<(Vec<FitPiece>, Pose, Option<Pose>), PlaceError> {
        let (s_cur, s0) = from_to;
        let (p0, heading) = (deck.0, deck.1);
        let at = |slide: f64| Pose {
            x: p0.x + slide * heading.sin(),
            y: p0.y + slide * heading.cos(),
            heading,
        };
        'slides: for step in 0..=BRIDGE_SLIDE_STEPS {
            for sign in [1.0, -1.0] {
                if step == 0 && sign < 0.0 {
                    continue;
                }
                let slide = sign * BRIDGE_SLIDE_STEP_M * f64::from(step);
                let start_s = s0 + slide;
                if start_s - s_cur <= 1.0 || (slide > 0.0 && slide + reach.0 > reach.1) {
                    continue;
                }
                let goal = at(slide);
                let path = line.slice(s_cur, start_s);
                let fit = fit_line(&path, pose, EndGoal::Fixed(goal), self.pieces(family))
                    .map_err(|source| PlaceError::Fit {
                        stretch: stretch.to_string(),
                        source,
                    })?;
                if fit.gap_m <= GAP_CLOSED_M && fit.heading_error_deg <= KINK_TOLERATED_DEG {
                    let end = fit.pieces.last().map_or(pose, |p| p.end);
                    return Ok((fit.pieces, end, Some(goal)));
                }
                if step == 0 && fit.heading_error_deg > BRIDGE_TURN_MAX_DEG {
                    // the road arrives turned against the chord: the deck follows the road
                    break 'slides;
                }
            }
        }
        // no slide closes: the deck stays at its place and starts where the approach ends
        let (pieces, end) = self.fit_part(
            &line.slice(s_cur, s0),
            pose,
            EndGoal::Fixed(at(0.0)),
            family,
            stretch,
        )?;
        Ok((pieces, end, None))
    }

    /// Where a junction goes if its entry is at `at`, trying both ways round; the frame, whether it
    /// is flipped, and the pose where the through road leaves. `None` if it is more than the tolerance
    /// from the crossing.
    fn place_junction(
        plan: &Plan<'_>,
        point: (f64, f64),
        at: Pose,
        through_heading: f64,
    ) -> Option<(Frame, bool, Pose)> {
        let j = &*plan.junction;
        let mut best: Option<(Frame, bool, Pose, bool)> = None;
        for flipped in [false, true] {
            let (entry, exit) = if flipped {
                (flip(&j.end), flip(&j.start))
            } else {
                (pose_of(&j.start), pose_of(&j.end))
            };
            let frame = Frame::through(&entry, at);
            let exit_pose = frame.pose(&exit);
            // does the model's side port lie on the side where the real arm leaves?
            let wanted = plan.sides.first().map(|s| s.1);
            let port_right = j.side_ports.first().map(|port| {
                let world = frame.pose(&pose_of(port));
                (world.x - at.x) * through_heading.cos() - (world.y - at.y) * through_heading.sin()
                    > 0.0
            });
            let matches = j.side_ports.len() > 1 || wanted == port_right;
            if matches && best.is_none() {
                best = Some((frame, flipped, exit_pose, matches));
            }
        }
        let (frame, flipped, exit, _) = best?;
        let (entry, _) = if flipped {
            (flip(&j.end), flip(&j.start))
        } else {
            (pose_of(&j.start), pose_of(&j.end))
        };
        let a = frame.pose(&entry);
        let centre = (f64::midpoint(a.x, exit.x), f64::midpoint(a.y, exit.y));
        let off = (centre.0 - point.0).hypot(centre.1 - point.1);
        (off <= JUNCTION_TOLERANCE_M).then_some((frame, flipped, exit))
    }

    /// A side road that ends at the port of the junction `arm` starts from did not reach it (`gap` m short):
    /// tries the junction at [`JUNCTION_SHIFT_STEP_M`] steps along its through road, up to
    /// [`JUNCTION_SHIFT_MAX_M`] each way, `+` before `-`, refitting the end of the road (`path`, which ends
    /// at the port, from `pose`) to the port the junction would have there. The first shift that closes
    /// the road, else the one with the smallest gap, if it is smaller than `gap`, is noted in `found`
    /// for the next placement. The junction is taken on the line of its through road, which the placed
    /// junction follows to a fraction of a metre.
    fn repair_port(&mut self, arm: Arm, path: &[(f64, f64)], pose: Pose, family: Family, gap: f64) {
        let Some(&key) = self.fixed_from.get(&arm) else {
            return;
        };
        if self.is_ring(key.0) || self.found.contains_key(&key) {
            return;
        }
        let (Some(plan_id), Some(links), Some((_, rest))) = (
            self.plan_at(key.0, key.1),
            self.chains.get(key.0).map(|ch| ch.links.len()),
            path.split_last(),
        ) else {
            return;
        };
        let Some(plan) = self.plans.get(plan_id) else {
            return;
        };
        let line = self.polyline(key.0, 0, links.saturating_sub(1));
        let (Some(s_cross), half) = (
            line.link_start.get(key.1 + 1).copied(),
            plan.junction.half_length(),
        ) else {
            return;
        };
        let mut best: Option<(f64, f64)> = None;
        for step in 1..=JUNCTION_SHIFT_STEPS {
            for sign in [1.0, -1.0] {
                let shift = sign * JUNCTION_SHIFT_STEP_M * f64::from(step);
                let moved = s_cross + shift;
                if moved - half < 1.0 || moved + half > line.total() - 1.0 {
                    continue;
                }
                let at = line.pose_at(moved);
                let Some((frame, _, _)) = Self::place_junction(
                    plan,
                    (at.x, at.y),
                    line.pose_at(moved - half),
                    at.heading,
                ) else {
                    continue;
                };
                let Some(port) = self
                    .side_poses(plan_id, &frame)
                    .into_iter()
                    .find(|(a, _)| *a == arm)
                    .map(|(_, port)| port)
                else {
                    continue;
                };
                let arrive = Pose {
                    heading: wrap(port.heading + PI),
                    ..port
                };
                let mut trial = rest.to_vec();
                trial.push((port.x, port.y));
                // a fit that fails is no candidate
                let Ok(fit) = fit_line(&trial, pose, EndGoal::Fixed(arrive), self.pieces(family))
                else {
                    continue;
                };
                let end = fit.pieces.last().map_or(pose, |p| p.end);
                let missed = (end.x - arrive.x).hypot(end.y - arrive.y);
                if best.is_none_or(|(g, _)| missed < g) {
                    best = Some((missed, shift));
                }
                if missed <= GAP_TOLERATED_M {
                    self.found.insert(key, shift);
                    return;
                }
            }
        }
        if let Some((missed, shift)) = best
            && missed < gap
        {
            self.found.insert(key, shift);
        }
    }

    /// Where the junctions of the run `first..=last` of chain `c` sit along its line: the crossings
    /// themselves, moved along the line by the shift asked for. A shift is dropped (the junction stays at
    /// its crossing) if the junction would come within a metre of the ends of the run, over a bridge, on a
    /// ring, or less than one junction length from the junction before or after it, both at their final
    /// places; so a shifted junction never reaches or passes its neighbour.
    fn crossings(
        &self,
        c: usize,
        first: usize,
        last: usize,
        line: &Polyline,
    ) -> BTreeMap<usize, f64> {
        let bridges: Vec<(f64, f64)> = (first..=last)
            .filter(|k| self.bridge_chord(c, *k).is_some_and(|chord| chord > 0.0))
            .filter_map(|k| {
                Some((
                    line.link_start.get(k - first).copied()?,
                    line.link_start
                        .get(k + 1 - first)
                        .copied()
                        .unwrap_or_else(|| line.total()),
                ))
            })
            .collect();
        // (plan, crossing, half length, final place, whether it is moved)
        let mut at: Vec<(usize, f64, f64, f64, bool)> = Vec::new();
        for k in first..last {
            let (Some(plan_id), Some(s_cross)) = (
                self.plan_at(c, k),
                line.link_start.get(k + 1 - first).copied(),
            ) else {
                continue;
            };
            let Some(plan) = self.plans.get(plan_id) else {
                continue;
            };
            let half = plan.junction.half_length();
            let wanted = self.shifts.get(&(c, k)).copied().unwrap_or(0.0);
            let moved = s_cross + wanted;
            let fits = wanted.abs() > 1e-9
                && !self.is_ring(c)
                && moved - half >= 1.0
                && moved + half <= line.total() - 1.0
                && bridges
                    .iter()
                    .all(|(b0, b1)| moved + half <= *b0 || moved - half >= *b1);
            at.push((
                plan_id,
                s_cross,
                half,
                if fits { moved } else { s_cross },
                fits,
            ));
        }
        let mut spots: Vec<Spot> = at
            .iter()
            .map(|&(_, crossing, _, place, moved)| Spot {
                crossing,
                place,
                moved,
            })
            .collect();
        keep_apart(&mut spots);
        at.iter()
            .zip(&spots)
            .map(|(a, spot)| (a.0, spot.place))
            .collect()
    }

    /// The world poses of the side ports of a placed junction, matched with the side arms.
    fn side_poses(&self, plan_id: usize, frame: &Frame) -> Vec<(Arm, Pose)> {
        let Some(plan) = self.plans.get(plan_id) else {
            return Vec::new();
        };
        let ports: Vec<Pose> = plan
            .junction
            .side_ports
            .iter()
            .map(|p| frame.pose(&pose_of(p)))
            .collect();
        // the through heading is the one of the frame's entry; right of it is where the cross product
        // with the port direction is positive
        let through = frame.pose(&pose_of(&plan.junction.start)).heading;
        let right = |port: &Pose| (port.heading - through).sin() > 0.0;
        let mut out = Vec::new();
        let mut used = vec![false; ports.len()];
        for (arm, arm_right) in &plan.sides {
            let pick = ports
                .iter()
                .enumerate()
                .find(|(i, p)| !used.get(*i).copied().unwrap_or(true) && right(p) == *arm_right)
                .or_else(|| {
                    ports
                        .iter()
                        .enumerate()
                        .find(|(i, _)| !used.get(*i).copied().unwrap_or(true))
                });
            if let Some((i, port)) = pick {
                if let Some(u) = used.get_mut(i) {
                    *u = true;
                }
                out.push((*arm, *port));
            }
        }
        out
    }

    /// Gives the items their slots and adds them to the network.
    fn finish_unit(
        &mut self,
        links: &[crate::roadnet::ChainLink],
        line: &Polyline,
        mut items: Vec<Item>,
        cap_start: bool,
        cap_end: bool,
    ) {
        let n = links.len();
        for item in &mut items {
            if item.link.is_none() {
                item.link = Some(line.link_of(item.s).min(n.saturating_sub(1)));
            }
        }
        let id_of = |k: usize| {
            links
                .get(k)
                .and_then(|l| self.graph.stretches.get(l.stretch))
                .map(|s| s.id.clone())
                .unwrap_or_default()
        };
        // pieces per link in driving order
        let mut per_link: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, item) in items.iter().enumerate() {
            if matches!(item.road.kind, PlacedKind::Piece | PlacedKind::Bridge)
                && let Some(v) = item.link.and_then(|k| per_link.get_mut(k))
            {
                v.push(i);
            }
        }
        for (k, v) in per_link.iter().enumerate() {
            let reversed = links.get(k).is_some_and(|l| l.reversed);
            let ordered: Vec<usize> = if reversed {
                v.iter().rev().copied().collect()
            } else {
                v.clone()
            };
            for (index, &i) in ordered.iter().enumerate() {
                if let Some(item) = items.get_mut(i) {
                    item.road.slot = Slot {
                        stretch: id_of(k),
                        index: u32::try_from(index).unwrap_or(u32::MAX - 2),
                    };
                }
            }
        }
        let mut caps_seen = 0;
        for item in &mut items {
            match item.road.kind {
                PlacedKind::Junction => {
                    let k = item.link.unwrap_or(0);
                    let (a, b) = (id_of(k), id_of(k + 1));
                    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                    item.road.slot = Slot {
                        stretch: format!("{lo}|{hi}"),
                        index: 0,
                    };
                }
                PlacedKind::EndCap => {
                    let at_unit_start = cap_start && caps_seen == 0;
                    caps_seen += 1;
                    let _ = cap_end;
                    let (k, entering) = if at_unit_start {
                        (0, true)
                    } else {
                        (n.saturating_sub(1), false)
                    };
                    let reversed = links.get(k).is_some_and(|l| l.reversed);
                    // the stored start of the stretch is where the chain enters it unless reversed
                    let at_stored_start = entering != reversed;
                    item.road.slot = Slot {
                        stretch: id_of(k),
                        index: if at_stored_start {
                            u32::MAX - 1
                        } else {
                            u32::MAX
                        },
                    };
                }
                _ => {}
            }
        }
        let base = self.out.roads.len();
        let ids: Vec<usize> = (0..items.len()).map(|i| base + i).collect();
        self.out.roads.extend(items.into_iter().map(|i| i.road));
        self.out.chains.push(ids);
    }
}

/// A model port used the other way round: same place, direction turned by π.
fn flip(port: &Port) -> Pose {
    Pose {
        x: port.x,
        y: port.z,
        heading: wrap(port.heading + PI),
    }
}

/// Places the network of `graph` with the pieces of `catalog`.
///
/// Chains are placed in rank order (a chain after those whose junctions it hangs on); a through road gets
/// a junction piece where a side road meets it, if the catalog has one for the pair and the fit passes
/// within 3 m of the crossing; side roads start at the junction's ports. Dead ends get an end cap, ends cut
/// at the map's edge (negative node ids) get none, tunnels get no pieces and are reported.
///
/// # Errors
/// [`PlaceError`] if a stretch cannot be fitted at all.
pub fn place_network(
    graph: &RoadGraph,
    catalog: &PieceCatalog,
) -> Result<PlacedNetwork, PlaceError> {
    let mut base = place_with_shifts(graph, catalog, BTreeMap::new())?;
    let mut shifts = base.found.clone();
    // the shifts found while placing the side roads are placed for real; if a through road's gap before
    // a junction grew, the shifts on that chain are taken back and the network is placed once more
    for attempt in 0..2 {
        if shifts.is_empty() {
            break;
        }
        let moved = match place_with_shifts(graph, catalog, shifts.clone()) {
            Ok(moved) => moved,
            Err(error) => {
                base.net
                    .warnings
                    .push(format!("junctions not moved to close roads: {error}"));
                break;
            }
        };
        let grown = through_grown(&base.through_gaps, &moved.through_gaps, &shifts);
        if grown.is_empty() {
            return Ok(if improves(&base.net, &moved.net) {
                moved.net
            } else {
                base.net
            });
        }
        if attempt == 1 {
            break;
        }
        shifts.retain(|key, _| !grown.contains(key));
    }
    Ok(base.net)
}

/// A junction on its through road: where its crossing is, where it sits, and whether that is moved.
struct Spot {
    crossing: f64,
    place: f64,
    moved: bool,
}

/// Puts a moved junction back at its crossing where it would be less than [`JUNCTION_SPACING_M`] from the junction
/// before or after it (the spots are in driving order), both at their final places. Putting one back can
/// free its neighbour, so this goes on until nothing changes.
fn keep_apart(spots: &mut [Spot]) {
    loop {
        let mut changed = false;
        for n in 0..spots.len() {
            let Some(spot) = spots.get(n) else { continue };
            if !spot.moved {
                continue;
            }
            let place = spot.place;
            let crossing = spot.crossing;
            let near = |m: Option<usize>| {
                m.and_then(|m| spots.get(m))
                    .is_some_and(|other| (other.place - place).abs() < JUNCTION_SPACING_M)
            };
            if (near(n.checked_sub(1)) || near(n.checked_add(1)))
                && let Some(spot) = spots.get_mut(n)
            {
                spot.place = crossing;
                spot.moved = false;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// The shifted junctions whose through road ends further from the goal of its approach than before, and more
/// than [`GAP_TOLERATED_M`] from it.
fn through_grown(base: &Shifts, moved: &Shifts, shifts: &Shifts) -> BTreeSet<(usize, usize)> {
    shifts
        .keys()
        .filter(|key| {
            let before = base.get(key).copied().unwrap_or(0.0);
            moved
                .get(key)
                .is_some_and(|gap| *gap > before + 1e-9 && *gap > GAP_TOLERATED_M)
        })
        .copied()
        .collect()
}

/// Whether a placement with moved junctions is better than the one without: fewer gaps (then a smaller sum of
/// them), and no junction piece lost and no join without a junction added.
fn improves(base: &PlacedNetwork, moved: &PlacedNetwork) -> bool {
    let junctions = |net: &PlacedNetwork| {
        net.roads
            .iter()
            .filter(|r| r.kind == PlacedKind::Junction)
            .count()
    };
    let gaps = |net: &PlacedNetwork| {
        (
            net.gaps.len(),
            net.gaps.iter().map(|g| g.gap_m).sum::<f64>(),
        )
    };
    let (before, after) = (gaps(base), gaps(moved));
    let fewer = after.0 < before.0 || (after.0 == before.0 && after.1 < before.1);
    fewer
        && junctions(moved) >= junctions(base)
        && moved.joins_without_junction.len() <= base.joins_without_junction.len()
}

/// One placement of the network.
struct Run {
    net: PlacedNetwork,
    /// The shifts that would close the side roads that did not close.
    found: Shifts,
    /// The gap of the through road before each junction, see [`Placer::through_gaps`].
    through_gaps: Shifts,
}

/// One placement with the junctions at `shifts` along their through roads.
fn place_with_shifts(
    graph: &RoadGraph,
    catalog: &PieceCatalog,
    shifts: Shifts,
) -> Result<Run, PlaceError> {
    let mut placer = Placer::new(graph, catalog, shifts);
    placer.analyse();
    let mut state = vec![0u8; placer.chains.len()];
    for c in 0..placer.chains.len() {
        placer.place_chain(c, &mut state)?;
    }
    Ok(Run {
        net: placer.out,
        found: placer.found,
        through_gaps: placer.through_gaps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(x: f64, y: f64) -> Pose {
        Pose { x, y, heading: 0.0 }
    }

    /// A road going north for 10 m that ends at the crossing (0, 10).
    fn line() -> Polyline {
        Polyline {
            points: vec![(0.0, 0.0), (0.0, 10.0)],
            cum: vec![0.0, 10.0],
            link_start: vec![0.0],
        }
    }

    fn spots(list: &[(f64, f64, bool)]) -> Vec<Spot> {
        list.iter()
            .map(|&(crossing, place, moved)| Spot {
                crossing,
                place,
                moved,
            })
            .collect()
    }

    #[test]
    fn a_junction_moved_up_to_its_neighbour_stays() {
        // a junction shifted 25 m lands 5 m from the one 30 m on: it stays, the neighbour is untouched
        let mut list = spots(&[(0.0, 25.0, true), (30.0, 30.0, false)]);
        keep_apart(&mut list);
        assert!((list[0].place - 0.0).abs() < 1e-9 && !list[0].moved);
        assert!((list[1].place - 30.0).abs() < 1e-9);
        // one that passes its neighbour (shifted past it) stays too, so the order is kept
        let mut list = spots(&[(0.0, 25.0, true), (20.0, 20.0, false)]);
        keep_apart(&mut list);
        assert!((list[0].place - 0.0).abs() < 1e-9);
        // a shift that keeps a junction length from both neighbours is kept
        let mut list = spots(&[(0.0, 0.0, false), (30.0, 17.5, true), (60.0, 60.0, false)]);
        keep_apart(&mut list);
        assert!((list[1].place - 17.5).abs() < 1e-9 && list[1].moved);
        // two moved junctions that meet: the first goes back, which frees the second
        let mut list = spots(&[(0.0, 12.5, true), (25.0, 20.0, true)]);
        keep_apart(&mut list);
        assert!(!list[0].moved && list[1].moved);
    }

    #[test]
    fn a_junction_whose_through_gap_grows_loses_its_shift() {
        let base: Shifts = [((0, 1), 0.2), ((0, 2), 2.0), ((1, 1), 0.0), ((2, 1), 0.0)].into();
        // junction (0, 1) now misses by 1.5 m (grown, over the tolerance), (0, 2) by less than before,
        // (1, 1) by 0.4 m (grown, within the tolerance), (2, 1) by 3 m but it is not shifted
        let moved: Shifts = [((0, 1), 1.5), ((0, 2), 1.0), ((1, 1), 0.4), ((2, 1), 3.0)].into();
        let shifts: Shifts = [((0, 1), 6.25), ((0, 2), -6.25), ((1, 1), 12.5)].into();
        assert_eq!(
            through_grown(&base, &moved, &shifts),
            BTreeSet::from([(0, 1)])
        );
        assert!(through_grown(&base, &base, &shifts).is_empty());
    }

    fn network(gaps: usize, junctions: usize, joins: usize) -> PlacedNetwork {
        let mut net = PlacedNetwork::default();
        for _ in 0..gaps {
            net.gaps.push(RoadGap {
                stretch: String::new(),
                at: (0.0, 0.0),
                gap_m: 1.0,
                heading_error_deg: 0.0,
            });
        }
        for _ in 0..junctions {
            net.roads.push(PlacedRoad {
                slot: Slot {
                    stretch: String::new(),
                    index: 0,
                },
                kind: PlacedKind::Junction,
                family: Family::Asf3,
                drivable: true,
                model: String::new(),
                origin: Pose {
                    x: 0.0,
                    y: 0.0,
                    heading: 0.0,
                },
                start: Pose {
                    x: 0.0,
                    y: 0.0,
                    heading: 0.0,
                },
                end: Pose {
                    x: 0.0,
                    y: 0.0,
                    heading: 0.0,
                },
                width: 6.0,
                scale: 1.0,
                abreast: 1,
            });
        }
        net.joins_without_junction = vec![(0.0, 0.0); joins];
        net
    }

    #[test]
    fn fewer_gaps_do_not_pay_for_a_lost_junction_or_a_new_join() {
        let base = network(5, 3, 1);
        assert!(improves(&base, &network(4, 3, 1)));
        assert!(!improves(&base, &network(5, 3, 1)));
        // a junction dropped (its side roads then have no port to miss)
        assert!(!improves(&base, &network(2, 2, 1)));
        // a join without junction added
        assert!(!improves(&base, &network(2, 3, 2)));
    }

    #[test]
    fn an_end_inside_the_window_is_no_miss() {
        assert!(end_violation(&line(), pose(0.0, 8.0), 3.0) <= 0.0);
    }

    #[test]
    fn an_end_short_of_the_edge_is_a_miss() {
        assert!((end_violation(&line(), pose(0.0, 5.0), 3.0) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn an_end_past_the_centre_line_is_a_miss_whatever_its_distance() {
        // 0.5 m beyond the crossing: its Euclidean distance (0.5) lies inside the window, but it is past
        // the centre line
        assert!(end_violation(&line(), pose(0.0, 10.5), 3.0) > 0.0);
        // and one that overshoots sideways as well
        assert!(end_violation(&line(), pose(1.0, 11.0), 3.0) > 0.0);
    }
}

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

use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{PI, TAU};

use crate::catalog::{BridgeModel, Family, Junction, PieceCatalog, PieceKind, Port, RoadPiece};
use crate::roadfit::{EndGoal, FitError, FitPiece, GAP_TOLERATED_M, Pose, fit_line};
use crate::roadnet::{Chain, RoadGraph, chains};

/// Two junctions on one road must be at least this far apart (the length of a junction piece).
const JUNCTION_SPACING_M: f64 = 12.5;
/// A junction piece has to sit this close to the crossing it replaces.
const JUNCTION_TOLERANCE_M: f64 = 3.0;
/// A side road without a junction piece ends this far inside the through road's edge.
const SIDE_OVERLAP_M: f64 = 2.0;
/// A bridge stretch shorter than this stays an ordinary road.
const MIN_BRIDGE_M: f64 = 10.0;
/// A bridge model narrower than its road is scaled up to the road's width, but at most this much.
pub const MAX_BRIDGE_SCALE: f64 = 1.5;

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
    /// Width of the road; of a scaled bridge, its scaled width.
    pub width: f64,
    /// Uniform scale of the model: 1 for everything but a bridge that was made as wide as its road.
    pub scale: f64,
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

/// Everything that is placed, and what could not be.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlacedNetwork {
    /// The objects, chain by chain in driving order.
    pub roads: Vec<PlacedRoad>,
    /// Stretches that miss what they had to meet.
    pub gaps: Vec<RoadGap>,
    /// Crossings where roads meet without a junction piece.
    pub joins_without_junction: Vec<(f64, f64)>,
    /// Tunnel stretches that got no pieces, with their first point.
    pub tunnels_omitted: Vec<(String, (f64, f64))>,
    /// Indices into `roads`, in driving order, one list per placed run of a chain.
    pub chains: Vec<Vec<usize>>,
    /// Things worth knowing that did not stop the placement.
    pub warnings: Vec<String>,
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

/// A stretch end: the stretch and whether the end is its stored start.
type Arm = (usize, bool);

/// A junction piece that replaces a crossing of a through road.
struct Plan<'a> {
    chain: usize,
    /// Between link `link` and link `link + 1`.
    link: usize,
    point: (f64, f64),
    junction: &'a Junction,
    /// The side arms and whether each leaves to the right of the direction of travel.
    sides: Vec<(Arm, bool)>,
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
    out: PlacedNetwork,
    joins: BTreeSet<(u64, u64)>,
}

fn bits(p: (f64, f64)) -> (u64, u64) {
    (p.0.to_bits(), p.1.to_bits())
}

impl<'a> Placer<'a> {
    fn new(graph: &'a RoadGraph, catalog: &'a PieceCatalog) -> Self {
        let chains = chains(graph);
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
            arms_at,
            plans: Vec::new(),
            plan_index: BTreeMap::new(),
            needs,
            fixed: BTreeMap::new(),
            trim: BTreeMap::new(),
            out: PlacedNetwork::default(),
            joins: BTreeSet::new(),
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
            for i in 0..chain.links.len().saturating_sub(1) {
                let (Some(a), Some(b)) = (chain.links.get(i), chain.links.get(i + 1)) else {
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
                let s_cross = line.link_start.get(i + 1).copied().unwrap_or(0.0);
                let point = line.pose_at(s_cross);
                let at = (point.x, point.y);
                let too_close = last_junction.is_some_and(|l| s_cross - l < JUNCTION_SPACING_M);
                let beside_bridge = [a.stretch, b.stretch].iter().any(|i| {
                    self.graph
                        .stretches
                        .get(*i)
                        .is_some_and(|s| s.tunnel || s.bridge)
                });
                let plan = if too_close || beside_bridge {
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
                    self.join(at);
                    self.trim_sides(&chain, i, &sides);
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

    /// Side arms that end on a through road without junction piece stop at the edge of the through road.
    fn trim_sides(&mut self, chain: &Chain, boundary: usize, sides: &[Arm]) {
        let Some(link) = chain.links.get(boundary) else {
            return;
        };
        let family = self.graph.stretches.get(link.stretch).map(|s| s.family);
        let amount = family.map_or(0.0, |f| {
            (self.family_width(f) / 2.0 - SIDE_OVERLAP_M).max(0.0)
        });
        for arm in sides {
            self.trim.insert(*arm, amount);
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
            [one] => self.catalog.t_junction(through, family_of(*one)?)?,
            [one, two] => {
                if family_of(*one)? != family_of(*two)? || arm_side(*one) == arm_side(*two) {
                    return None;
                }
                self.catalog.x_junction(through, family_of(*one)?)?
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
            while last + 1 < links && !tunnel(last + 1) {
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
        let ring = links.len() == 1 && start_node == end_node && self.live_arms(start_node) == 2;
        let total = line.total();
        let trim_start = self.trim.get(&entry_arm).copied().unwrap_or(0.0);
        let trim_end = self.trim.get(&exit_arm).copied().unwrap_or(0.0);
        let fixed_start = self.fixed.get(&entry_arm).copied();
        let fixed_end = self.fixed.get(&exit_arm).copied();
        let cap_start = fixed_start.is_none() && start_node >= 0 && self.live_arms(start_node) == 1;
        let cap_end = fixed_end.is_none() && end_node >= 0 && self.live_arms(end_node) == 1;
        let width = self.pieces(family).first().map_or(0.0, |p| p.width);

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
        let mut events: Vec<(f64, Event)> = (first..last)
            .filter_map(|k| self.plan_at(c, k))
            .filter_map(|plan_id| {
                let plan = self.plans.get(plan_id)?;
                let s_cross = line.link_start.get(plan.link + 1 - first).copied()?;
                Some((
                    s_cross - plan.junction.half_length(),
                    Event::Junction(plan_id),
                ))
            })
            .collect();
        for k in first..=last {
            if self
                .bridge_chord(c, k)
                .is_some_and(|chord| chord >= MIN_BRIDGE_M)
                && let Some(s0) = line.link_start.get(k - first)
            {
                events.push((*s0, Event::Bridge(k)));
            }
        }
        events.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut dropped: Vec<usize> = Vec::new();
        for (_, event) in events {
            let plan_id = match event {
                Event::Junction(plan_id) => plan_id,
                Event::Bridge(k) => {
                    let placed = self.place_bridge(
                        &line,
                        c,
                        k - first,
                        family,
                        drivable,
                        &mut pose,
                        &mut s_cur,
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
            let Some((k, junction)) = self.plans.get(plan_id).map(|p| (p.link, p.junction)) else {
                continue;
            };
            let s_cross = line.link_start.get(k + 1 - first).copied().unwrap_or(0.0);
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
            let placed = Self::place_junction(plan, pose, line.pose_at(s_cross).heading);
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
                    for (arm, port) in ports {
                        self.fixed.insert(arm, port);
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
                self.join(at);
                if let Some(chain) = self.chains.get(c).cloned() {
                    self.trim_sides(&chain, boundary, &sides);
                }
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
        if path.len() >= 2 && path.windows(2).any(|p| matches!(p, [a, b] if a != b)) {
            path.dedup();
            let (pieces, end) = self.fit_part(&path, pose, goal, family, &stretch_id)?;
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

    /// The bridge model for a road of `family` and the scale to lay it with: wooden models for gravel
    /// and dirt, the others for the paved families. The narrowest model that is at least as wide as the
    /// road, at scale 1; else the widest, scaled up to the road's width but at most
    /// [`MAX_BRIDGE_SCALE`], with a warning when that is not enough. Never scaled down.
    fn choose_bridge(&mut self, family: Family, stretch: &str) -> Option<(&'a BridgeModel, f64)> {
        let wooden = matches!(family, Family::Grav | Family::Mud);
        let all = self.catalog.bridges();
        let of_material: Vec<&BridgeModel> = all
            .iter()
            .filter(|b| b.model.contains("wood") == wooden)
            .collect();
        let pool: Vec<&BridgeModel> = if of_material.is_empty() {
            all.iter().collect()
        } else {
            of_material
        };
        let width = self.family_width(family);
        let enough = pool
            .iter()
            .filter(|b| b.width + 1e-6 >= width)
            .min_by(|x, y| x.width.total_cmp(&y.width))
            .copied();
        if let Some(model) = enough {
            return Some((model, 1.0));
        }
        let widest = pool
            .iter()
            .max_by(|x, y| x.width.total_cmp(&y.width))
            .copied()?;
        let scale = if widest.width > 0.0 {
            (width / widest.width).clamp(1.0, MAX_BRIDGE_SCALE)
        } else {
            1.0
        };
        if widest.width * scale + 1e-6 < width {
            self.out.warnings.push(format!(
                "bridge on {stretch}: the widest bridge model is {:.1} m, scaled up to {:.1} m; the road is {width:.1} m wide",
                widest.width,
                widest.width * scale
            ));
        }
        Some((widest, scale))
    }

    /// Fits the road up to link `k` of the run and lays the bridge models along its chord.
    #[expect(clippy::too_many_arguments, reason = "the state of one run, passed on")]
    fn place_bridge(
        &mut self,
        line: &Polyline,
        c: usize,
        k: usize,
        family: Family,
        drivable: bool,
        pose: &mut Pose,
        s_cur: &mut f64,
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
        let id = self
            .chains
            .get(c)
            .and_then(|ch| ch.links.get(k))
            .and_then(|l| self.graph.stretches.get(l.stretch))
            .map(|s| s.id.clone())
            .unwrap_or_default();
        let Some((model, scale)) = self.choose_bridge(family, &id) else {
            return Ok(out);
        };
        if s0 - *s_cur > 1.0 {
            let path = line.slice(*s_cur, s0);
            let goal = Pose {
                x: p0.x,
                y: p0.y,
                heading,
            };
            let (pieces, end) = self.fit_part(&path, *pose, EndGoal::Fixed(goal), family, &id)?;
            out.fit = pieces;
            *pose = end;
        } else if (pose.x - p0.x).hypot(pose.y - p0.y) < 1.0 {
            pose.heading = heading;
        }
        let scaled = |port: &Port| Port {
            x: port.x * scale,
            z: port.z * scale,
            ..*port
        };
        let (model_start, model_end) = (scaled(&model.start), scaled(&model.end));
        let length = model.length * scale;
        let count = (chord / length).round().max(1.0);
        let mut laid = 0.0;
        while laid < count {
            let frame = Frame::through(&pose_of(&model_start), *pose);
            let start = frame.pose(&pose_of(&model_start));
            let end = frame.pose(&pose_of(&model_end));
            out.items.push(Item {
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
                    width: model.width * scale,
                    scale,
                },
                s: s0 + laid * length,
                link: None,
            });
            *pose = end;
            laid += 1.0;
        }
        *s_cur = s1;
        Ok(out)
    }

    /// Where a junction goes if its entry is at `at`, trying both ways round; the frame, whether it
    /// is flipped, and the pose where the through road leaves. `None` if it is more than the tolerance
    /// from the crossing.
    fn place_junction(
        plan: &Plan<'_>,
        at: Pose,
        through_heading: f64,
    ) -> Option<(Frame, bool, Pose)> {
        let j = plan.junction;
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
        let off = (centre.0 - plan.point.0).hypot(centre.1 - plan.point.1);
        (off <= JUNCTION_TOLERANCE_M).then_some((frame, flipped, exit))
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
    let mut placer = Placer::new(graph, catalog);
    placer.analyse();
    let mut state = vec![0u8; placer.chains.len()];
    for c in 0..placer.chains.len() {
        placer.place_chain(c, &mut state)?;
    }
    Ok(placer.out)
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Junctions, their heights from the ground, the links between them, and the repair.
//!
//! A junction is a place where roads meet: a junction piece, a crossing where roads meet without a junction
//! piece, or a chain end that lies on another road. Each gets one height, the median of the original ground
//! around it; a link is the road between two neighbouring junctions, from the joint that holds one to the joint
//! that holds the other. Where the ground of two junctions needs
//! a steeper grade between them than their road may have, [`repair`] moves both toward each other by just
//! enough, the one on the more important road less. The design is
//! `docs/superpowers/specs/2026-11-13-road-heights-design.md`, section 4.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::Family;
use crate::heightmap::Heightmap;
use crate::limits::grade_ceiling;
use crate::profile::{ON_ROAD_M, RoadStrip, chain_holds, meeting_bounds, meeting_on, rank};
use crate::roadnet::RoadGraph;
use crate::roadplace::{PlacedKind, PlacedNetwork};

/// Radius of the circle the ground is sampled in.
pub const GROUND_RADIUS_M: f64 = 10.0;
/// Junctions this close along a road are one.
pub const MERGE_M: f64 = 3.0;
/// Rounds of [`repair`].
pub const REPAIR_ROUNDS: usize = 50;
/// The most [`repair`] moves a junction from its ground median, in metres. A link that is still over its
/// ceiling then is unresolved.
pub const MAX_MOVE_M: f64 = 3.0;

/// How far beyond a junction's patch a chain end or a junction piece may lie and still belong to the junction:
/// the side ports of a junction piece lie as far from its centre as the patch reaches, and the centre of the
/// piece may be off the crossing by a few metres.
pub const HOLD_M: f64 = 4.0;

/// How far from a crossing a road may pass and still be one of its arms.
pub(crate) const ARM_REACH_M: f64 = 13.0;
/// A projection this close to the end of a road is the end.
const END_M: f64 = 3.0;
/// A junction piece whose centre is this close to a station is the one the station lies on.
const PIECE_STATION_M: f64 = 4.0;
/// A chain end this close to a crossing's junction belongs to it.
const NEAR_CROSSING_M: f64 = 12.0;
/// Chain ends on another road this close together share a junction.
const END_CLUSTER_M: f64 = 6.0;
/// What [`repair`] moves a link beyond its excess, so that it ends up under its ceiling.
const REPAIR_MARGIN_M: f64 = 0.001;
/// A link over its ceiling by no more than this counts as repaired.
const REPAIR_TOLERANCE_M: f64 = 0.01;
/// The shortest link, in metres.
const MIN_LINK_M: f64 = 1.0;

/// What kind of meeting a junction is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JunctionKind {
    /// A placed junction piece.
    Piece,
    /// Roads meet at a crossing without a junction piece.
    Join,
    /// A chain ends on another road without a crossing.
    EndOnRoad,
}

/// A place where roads meet, with one height for all of them.
#[derive(Debug, Clone, PartialEq)]
pub struct Junction {
    /// Position in map metres.
    pub at: (f64, f64),
    /// What meets here.
    pub kind: JunctionKind,
    /// Median of the original ground.
    pub ground: f64,
    /// The height the roads get; `ground` until `repair` moves it.
    pub height: f64,
    /// Rank of the most important road meeting here.
    pub rank: u8,
    /// Radius of the level patch: half the length of the junction piece, or half the width of the widest
    /// road meeting here.
    pub patch_m: f64,
    /// The ids of the stretches of the roads that belong to this junction (they end here, cross here in the graph,
    /// or are the road a chain end lies on), junction pieces' `a|b` ids split. A chain that has none of them is
    /// not held to this junction, whatever its distance. Empty: not recorded, every chain counts.
    pub stretches: BTreeSet<String>,
}

/// The road between two neighbouring junctions.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    /// Index into [`Junctions::points`], the lower of the two.
    pub a: usize,
    /// Index into [`Junctions::points`], the higher of the two.
    pub b: usize,
    /// Road length along the chain between the two joints that are held to the two junctions, at least 1 m.
    pub length: f64,
    /// `grade_ceiling` of the road's family.
    pub ceiling: f64,
}

/// All junctions of a network and the links between them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Junctions {
    /// The junctions, sorted by position.
    pub points: Vec<Junction>,
    /// The links, sorted by their ends and length.
    pub links: Vec<Link>,
    /// Links still over their ceiling after `repair`, by index into `links`.
    pub unresolved: Vec<usize>,
}

/// `n` as a float; counts here are far below 2^32.
fn count(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// The median of the original ground in a circle of `radius` metres around `at`: the samples on a 1 m grid
/// through `at` that lie inside the circle and inside the map. With no sample inside, the height at `at`,
/// clamped to the map.
#[must_use]
pub fn ground_median(base: &Heightmap, at: (f64, f64), radius: f64) -> f64 {
    let mut offsets = vec![0.0];
    let mut d = 1.0;
    while d <= radius {
        offsets.push(d);
        offsets.push(-d);
        d += 1.0;
    }
    let extent = f64::from(base.size.saturating_sub(1)) * f64::from(base.cell_m);
    let inside = |v: f64| (0.0..=extent).contains(&v);
    let mut heights = Vec::new();
    for dx in &offsets {
        for dy in &offsets {
            let (x, y) = (at.0 + dx, at.1 + dy);
            if dx.hypot(*dy) <= radius && inside(x) && inside(y) {
                heights.push(f64::from(base.sample(x, y).height));
            }
        }
    }
    heights.sort_by(f64::total_cmp);
    let n = heights.len();
    let middle = |i: usize| heights.get(i).copied();
    let median = if n == 0 {
        None
    } else if n % 2 == 1 {
        middle(n / 2)
    } else {
        middle(n / 2 - 1)
            .zip(middle(n / 2))
            .map(|(low, high)| f64::midpoint(low, high))
    };
    median.unwrap_or_else(|| f64::from(base.sample(at.0, at.1).height))
}

/// One chain as a polyline, for finding where roads meet.
struct Line {
    /// The joints of the chain, at height 0, for [`meeting_on`].
    strip: RoadStrip,
    /// Stretch ids of the chain's roads, junction pieces' `a|b` ids split.
    ids: BTreeSet<String>,
    family: Family,
    /// Station of each joint along the polyline.
    stations: Vec<f64>,
    length: f64,
    bounds: [f64; 4],
}

impl Line {
    fn first(&self) -> (f64, f64) {
        self.strip.joints.first().map_or((0.0, 0.0), |j| (j.0, j.1))
    }

    fn last(&self) -> (f64, f64) {
        self.strip.joints.last().map_or((0.0, 0.0), |j| (j.0, j.1))
    }

    /// The nearest point of the polyline to `p`: its distance and station.
    fn project(&self, p: (f64, f64)) -> (f64, f64) {
        let mut best = (f64::INFINITY, 0.0);
        for (pair, stations) in self.strip.joints.windows(2).zip(self.stations.windows(2)) {
            let ([from, to], [s0, s1]) = (pair, stations) else {
                continue;
            };
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            let length2 = dx * dx + dy * dy;
            let t = if length2 > 0.0 {
                (((p.0 - from.0) * dx + (p.1 - from.1) * dy) / length2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let distance = (p.0 - from.0 - t * dx).hypot(p.1 - from.1 - t * dy);
            if distance < best.0 {
                best = (distance, s0 + t * (s1 - s0));
            }
        }
        best
    }

    /// Half the length of the junction piece that carries `station`, or 0.
    fn piece_half(&self, station: f64) -> f64 {
        for (k, stations) in self.stations.windows(2).enumerate() {
            let [s0, s1] = stations else { continue };
            let piece = self.strip.junction.get(k).copied().unwrap_or(false);
            if piece && (f64::midpoint(*s0, *s1) - station).abs() < PIECE_STATION_M {
                return 0.5 * (s1 - s0);
            }
        }
        0.0
    }

    /// The end a station is at (`false` the start, `true` the end), if it is at one.
    fn end_at(&self, station: f64) -> Option<bool> {
        if station <= END_M {
            Some(false)
        } else if station >= self.length - END_M {
            Some(true)
        } else {
            None
        }
    }

    fn end_point(&self, end: bool) -> (f64, f64) {
        if end { self.last() } else { self.first() }
    }

    fn end_station(&self, end: bool) -> f64 {
        if end { self.length } else { 0.0 }
    }
}

/// The chains of `net` as polylines, in an order that does not depend on the order of the input.
fn lines_of(net: &PlacedNetwork) -> Vec<Line> {
    let mut lines = Vec::new();
    for chain in &net.chains {
        let roads: Vec<_> = chain.iter().filter_map(|i| net.roads.get(*i)).collect();
        let Some(first) = roads.first() else { continue };
        let mut joints = vec![(first.start.x, first.start.y, 0.0)];
        joints.extend(roads.iter().map(|r| (r.end.x, r.end.y, 0.0)));
        let mut stations = vec![0.0];
        for pair in joints.windows(2) {
            if let [from, to] = pair {
                let last = stations.last().copied().unwrap_or(0.0);
                stations.push(last + (to.0 - from.0).hypot(to.1 - from.1));
            }
        }
        let strip = RoadStrip {
            slots: chain.clone(),
            joints,
            bridge: roads.iter().map(|r| r.kind == PlacedKind::Bridge).collect(),
            junction: roads
                .iter()
                .map(|r| r.kind == PlacedKind::Junction)
                .collect(),
            side_slope: Vec::new(),
            half_width: first.width / 2.0,
            rank: rank(first.family),
        };
        let ids = roads
            .iter()
            .flat_map(|r| r.slot.stretch.split('|').map(str::to_string))
            .collect();
        lines.push(Line {
            bounds: meeting_bounds(&strip),
            strip,
            ids,
            family: first.family,
            length: stations.last().copied().unwrap_or(0.0),
            stations,
        });
    }
    lines.sort_by(|a, b| {
        let key = |l: &Line| (l.first(), l.last());
        let (ka, kb) = (key(a), key(b));
        ka.0.0
            .total_cmp(&kb.0.0)
            .then(ka.0.1.total_cmp(&kb.0.1))
            .then(ka.1.0.total_cmp(&kb.1.0))
            .then(ka.1.1.total_cmp(&kb.1.1))
            .then(a.length.total_cmp(&b.length))
            .then(a.ids.cmp(&b.ids))
    });
    lines
}

/// A road at a junction: where along its chain, and half the length of the junction piece it carries there.
#[derive(Debug, Clone, Copy)]
struct Arm {
    line: usize,
    station: f64,
    half: f64,
}

/// A junction before its height and the merging.
struct Raw {
    at: (f64, f64),
    kind: JunctionKind,
    arms: Vec<Arm>,
}

fn kind_order(kind: JunctionKind) -> u8 {
    match kind {
        JunctionKind::Piece => 0,
        JunctionKind::Join => 1,
        JunctionKind::EndOnRoad => 2,
    }
}

fn by_position(a: &(f64, f64), b: &(f64, f64)) -> std::cmp::Ordering {
    a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// The junctions at the crossings of the graph where three or more roads meet, and the chain ends they
/// take (`claimed`).
fn crossing_junctions(
    graph: &RoadGraph,
    lines: &[Line],
    claimed: &mut BTreeSet<(usize, bool)>,
) -> Vec<Raw> {
    let mut by_id: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, line) in lines.iter().enumerate() {
        for id in &line.ids {
            by_id.entry(id.as_str()).or_default().push(index);
        }
    }
    let mut out = Vec::new();
    for crossing in &graph.crossings {
        let stretches: Vec<_> = crossing
            .stretches
            .iter()
            .filter_map(|i| graph.stretches.get(*i))
            .filter(|s| !s.tunnel)
            .collect();
        if stretches.len() < 3 {
            continue;
        }
        let candidates: BTreeSet<usize> = stretches
            .iter()
            .filter_map(|s| by_id.get(s.id.as_str()))
            .flatten()
            .copied()
            .collect();
        let mut arms = Vec::new();
        let mut ends = Vec::new();
        for index in candidates {
            let Some(line) = lines.get(index) else {
                continue;
            };
            let (reach, station) = line.project(crossing.point);
            if reach > ARM_REACH_M {
                continue;
            }
            match line.end_at(station) {
                Some(end) => {
                    ends.push((index, end));
                    arms.push(Arm {
                        line: index,
                        station: line.end_station(end),
                        half: 0.0,
                    });
                }
                None => arms.push(Arm {
                    line: index,
                    station,
                    half: line.piece_half(station),
                }),
            }
        }
        let distinct: BTreeSet<usize> = arms.iter().map(|a| a.line).collect();
        if distinct.len() < 2 {
            continue;
        }
        claimed.extend(ends);
        let kind = if arms.iter().any(|a| a.half > 0.0) {
            JunctionKind::Piece
        } else {
            JunctionKind::Join
        };
        out.push(Raw {
            at: crossing.point,
            kind,
            arms,
        });
    }
    out
}

/// A chain end that lies on another road: the end, its position, and the other road.
struct OnRoad {
    line: usize,
    end: bool,
    at: (f64, f64),
    owner: usize,
}

/// Adds the junctions of chain ends that lie on another road without a crossing: to the crossing junction
/// within [`NEAR_CROSSING_M`] if there is one, else to a junction of their own.
fn end_junctions(lines: &[Line], claimed: &BTreeSet<(usize, bool)>, raw: &mut Vec<Raw>) {
    let mut found: Vec<OnRoad> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        for end in [false, true] {
            if claimed.contains(&(index, end)) {
                continue;
            }
            let at = line.end_point(end);
            let owner = lines
                .iter()
                .enumerate()
                .filter(|(other, l)| {
                    *other != index
                        && (l.bounds[0]..=l.bounds[2]).contains(&at.0)
                        && (l.bounds[1]..=l.bounds[3]).contains(&at.1)
                })
                .filter_map(|(other, l)| {
                    meeting_on(&l.strip, other, at, ON_ROAD_M, ON_ROAD_M)
                        .map(|(distance, _)| (distance, other))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            if let Some((_, owner)) = owner {
                found.push(OnRoad {
                    line: index,
                    end,
                    at,
                    owner,
                });
            }
        }
    }
    found.sort_by(|a, b| {
        by_position(&a.at, &b.at)
            .then(a.line.cmp(&b.line))
            .then(a.end.cmp(&b.end))
    });
    let crossings = raw.len();
    for item in found {
        let Some(line) = lines.get(item.line) else {
            continue;
        };
        let own = Arm {
            line: item.line,
            station: line.end_station(item.end),
            half: 0.0,
        };
        let near_crossing = raw
            .iter()
            .enumerate()
            .take(crossings)
            .map(|(i, r)| (i, distance(r.at, item.at)))
            .filter(|(_, d)| *d < NEAR_CROSSING_M)
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
            .map(|(i, _)| i);
        let own_junction = raw
            .iter()
            .enumerate()
            .skip(crossings)
            .find(|(_, r)| distance(r.at, item.at) < END_CLUSTER_M)
            .map(|(i, _)| i);
        if let Some(target) = near_crossing {
            if let Some(r) = raw.get_mut(target) {
                r.arms.push(own);
            }
            continue;
        }
        let Some(owner) = lines.get(item.owner) else {
            continue;
        };
        let (_, station) = owner.project(item.at);
        let on = Arm {
            line: item.owner,
            station: owner
                .end_at(station)
                .map_or(station, |end| owner.end_station(end)),
            half: 0.0,
        };
        match own_junction.and_then(|i| raw.get_mut(i)) {
            Some(r) => {
                r.arms.push(own);
                if !r.arms.iter().any(|a| a.line == item.owner) {
                    r.arms.push(on);
                }
            }
            None => raw.push(Raw {
                at: item.at,
                kind: JunctionKind::EndOnRoad,
                arms: vec![own, on],
            }),
        }
    }
}

/// A link before merging: indices into the raw junctions.
struct RawLink {
    a: usize,
    b: usize,
    length: f64,
    ceiling: f64,
}

/// The links along every chain, between the junctions on it in the order of their stations. Junction pieces
/// are taken off the length at both ends; the free ends of a chain are not junctions and carry no link.
fn raw_links(lines: &[Line], raw: &[Raw]) -> Vec<RawLink> {
    let mut per_line: Vec<Vec<(f64, usize, f64)>> = vec![Vec::new(); lines.len()];
    for (junction, r) in raw.iter().enumerate() {
        for arm in &r.arms {
            if let Some(entries) = per_line.get_mut(arm.line) {
                entries.push((arm.station, junction, arm.half));
            }
        }
    }
    let mut out = Vec::new();
    for (line, entries) in lines.iter().zip(per_line.iter_mut()) {
        entries.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        entries.dedup_by_key(|e| e.1);
        for pair in entries.windows(2) {
            let [(sa, a, half_a), (sb, b, half_b)] = pair else {
                continue;
            };
            if a == b {
                continue;
            }
            out.push(RawLink {
                a: *a.min(b),
                b: *a.max(b),
                length: ((sb - half_b) - (sa + half_a)).max(MIN_LINK_M),
                ceiling: grade_ceiling(line.family),
            });
        }
    }
    out
}

fn find_root(parent: &[usize], mut i: usize) -> usize {
    while let Some(p) = parent.get(i).copied() {
        if p == i {
            break;
        }
        i = p;
    }
    i
}

/// The junctions of the network, each at the median of the original ground, with the links between them,
/// repaired. Junctions are sorted by position (x, then y) and links by their ends and length, so the result
/// does not depend on the order of the roads.
#[must_use]
pub fn find_junctions(net: &PlacedNetwork, graph: &RoadGraph, base: &Heightmap) -> Junctions {
    let lines = lines_of(net);
    let mut claimed = BTreeSet::new();
    let mut raw = crossing_junctions(graph, &lines, &mut claimed);
    end_junctions(&lines, &claimed, &mut raw);
    raw.sort_by(|a, b| by_position(&a.at, &b.at).then(kind_order(a.kind).cmp(&kind_order(b.kind))));
    let mut junctions = merged(&lines, &raw, &raw_links(&lines, &raw), base);
    junctions.links = held_links(net, f64::from(base.cell_m), &junctions);
    repair(&mut junctions);
    junctions
}

/// The links of the junctions as the roads hold them: along every chain, the road between two consecutive
/// joints that are held (by [`chain_holds`], the rules `solve_profiles` uses) to different junctions. Its
/// length is the length along the chain between the two joints, at least 1 m, so that a link that `repair`
/// made climbable is climbable between the places where the heights are actually held. Sorted by ends and
/// length, which does not depend on the order of the roads.
fn held_links(net: &PlacedNetwork, cell: f64, junctions: &Junctions) -> Vec<Link> {
    let mut out = Vec::new();
    for chain in &net.chains {
        let Some(family) = chain
            .first()
            .and_then(|i| net.roads.get(*i))
            .map(|r| r.family)
        else {
            continue;
        };
        let holds = chain_holds(net, chain, cell, junctions);
        let mut along = 0.0;
        let mut previous: Option<((f64, f64), usize, f64)> = None;
        let mut last: Option<(f64, f64)> = None;
        for (at, hold) in &holds {
            if let Some(from) = last {
                along += distance(from, *at);
            }
            last = Some(*at);
            let Some(hold) = hold else { continue };
            if let Some((_, junction, station)) = previous
                && junction != hold.junction
            {
                out.push(Link {
                    a: junction.min(hold.junction),
                    b: junction.max(hold.junction),
                    length: (along - station).max(MIN_LINK_M),
                    ceiling: grade_ceiling(family),
                });
            }
            previous = Some((*at, hold.junction, along));
        }
    }
    out.sort_by(|x, y| {
        x.a.cmp(&y.a)
            .then(x.b.cmp(&y.b))
            .then(x.length.total_cmp(&y.length))
            .then(x.ceiling.total_cmp(&y.ceiling))
    });
    out
}

/// The junction that the raw junctions `members` make: at the mean of their points, the first kind, the
/// highest rank and the largest patch.
fn one_junction(lines: &[Line], raw: &[Raw], members: &[usize], base: &Heightmap) -> Junction {
    let width_of = |arms: &[Arm]| {
        arms.iter()
            .filter_map(|a| lines.get(a.line))
            .map(|l| l.strip.half_width)
            .fold(0.0, f64::max)
    };
    let (mut sx, mut sy) = (0.0, 0.0);
    let mut kind = JunctionKind::EndOnRoad;
    let (mut top_rank, mut patch) = (0, 0.0_f64);
    let mut stretches = BTreeSet::new();
    for r in members.iter().filter_map(|m| raw.get(*m)) {
        sx += r.at.0;
        sy += r.at.1;
        if kind_order(r.kind) < kind_order(kind) {
            kind = r.kind;
        }
        top_rank = r
            .arms
            .iter()
            .filter_map(|a| lines.get(a.line))
            .map(|l| rank(l.family))
            .fold(top_rank, u8::max);
        let own_patch = if r.kind == JunctionKind::Piece {
            r.arms.iter().map(|a| a.half).fold(0.0, f64::max)
        } else {
            width_of(&r.arms)
        };
        patch = patch.max(own_patch);
        for line in r.arms.iter().filter_map(|a| lines.get(a.line)) {
            stretches.extend(line.ids.iter().cloned());
        }
    }
    let n = count(members.len());
    let at = (sx / n, sy / n);
    let ground = ground_median(base, at, GROUND_RADIUS_M);
    Junction {
        at,
        kind,
        ground,
        height: ground,
        rank: top_rank,
        patch_m: patch,
        stretches,
    }
}

/// The raw junctions with those joined by a link of [`MERGE_M`] or less made one, at the median of the
/// ground, and the links between what is left, sorted.
fn merged(lines: &[Line], raw: &[Raw], raw_links: &[RawLink], base: &Heightmap) -> Junctions {
    // junctions joined by a link of MERGE_M or less are one
    let mut parent: Vec<usize> = (0..raw.len()).collect();
    for link in raw_links.iter().filter(|l| l.length <= MERGE_M) {
        let (ra, rb) = (find_root(&parent, link.a), find_root(&parent, link.b));
        if let Some(p) = parent.get_mut(ra.max(rb)) {
            *p = ra.min(rb);
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..raw.len() {
        groups.entry(find_root(&parent, i)).or_default().push(i);
    }
    let mut merged: Vec<Junction> = Vec::new();
    let mut group_of = vec![0; raw.len()];
    for (group, members) in groups.values().enumerate() {
        for member in members {
            if let Some(slot) = group_of.get_mut(*member) {
                *slot = group;
            }
        }
        merged.push(one_junction(lines, raw, members, base));
    }

    // sorted by position, and the links with them
    let mut order: Vec<usize> = (0..merged.len()).collect();
    order.sort_by(|a, b| {
        let at = |i: &usize| merged.get(*i).map_or((0.0, 0.0), |m| m.at);
        by_position(&at(a), &at(b))
    });
    let mut new_index = vec![0; merged.len()];
    for (new, old) in order.iter().enumerate() {
        if let Some(slot) = new_index.get_mut(*old) {
            *slot = new;
        }
    }
    let points: Vec<Junction> = order
        .iter()
        .filter_map(|i| merged.get(*i).cloned())
        .collect();
    let target = |raw_index: usize| {
        group_of
            .get(raw_index)
            .and_then(|g| new_index.get(*g))
            .copied()
    };
    let mut out_links: Vec<Link> = raw_links
        .iter()
        .filter_map(|l| {
            let (a, b) = (target(l.a)?, target(l.b)?);
            (a != b).then(|| Link {
                a: a.min(b),
                b: a.max(b),
                length: l.length,
                ceiling: l.ceiling,
            })
        })
        .collect();
    out_links.sort_by(|x, y| {
        x.a.cmp(&y.a)
            .then(x.b.cmp(&y.b))
            .then(x.length.total_cmp(&y.length))
            .then(x.ceiling.total_cmp(&y.ceiling))
    });
    Junctions {
        points,
        links: out_links,
        unresolved: Vec::new(),
    }
}

/// Moves junctions toward each other where the grade between their ground heights is over the ceiling of
/// the link between them.
///
/// [`REPAIR_ROUNDS`] rounds over the links in their stored order. A link over its ceiling by `excess` moves
/// both ends by `excess` and a margin of 1 mm in all, shared by mobility `2^-rank`: the junction on the more
/// important road moves less. No junction moves more than [`MAX_MOVE_M`] from its ground median. Links still over by more than 1 cm after the last round are listed in
/// [`Junctions::unresolved`].
pub fn repair(junctions: &mut Junctions) {
    let mobility = |j: &Junction| 0.5_f64.powi(i32::from(j.rank));
    let excess = |points: &[Junction], link: &Link| {
        let rise = points
            .get(link.a)
            .zip(points.get(link.b))
            .map(|(a, b)| (a.height - b.height).abs());
        rise.map(|rise| rise - link.ceiling * link.length)
    };
    for _ in 0..REPAIR_ROUNDS {
        for link in &junctions.links {
            let Some(over) = excess(&junctions.points, link).filter(|e| *e > 0.0) else {
                continue;
            };
            let Some((a, b)) = junctions
                .points
                .get(link.a)
                .zip(junctions.points.get(link.b))
                .map(|(a, b)| (mobility(a), mobility(b)))
            else {
                continue;
            };
            let moved = over + REPAIR_MARGIN_M;
            let (share_a, share_b) = (moved * a / (a + b), moved * b / (a + b));
            let a_is_higher = junctions
                .points
                .get(link.a)
                .zip(junctions.points.get(link.b))
                .is_some_and(|(pa, pb)| pa.height > pb.height);
            // the higher end goes down by its share, the lower goes up by its share
            let sign = if a_is_higher { 1.0 } else { -1.0 };
            for (index, delta) in [(link.a, -sign * share_a), (link.b, sign * share_b)] {
                if let Some(p) = junctions.points.get_mut(index) {
                    p.height =
                        (p.height + delta).clamp(p.ground - MAX_MOVE_M, p.ground + MAX_MOVE_M);
                }
            }
        }
    }
    junctions.unresolved = junctions
        .links
        .iter()
        .enumerate()
        .filter(|(_, link)| excess(&junctions.points, link).is_some_and(|e| e > REPAIR_TOLERANCE_M))
        .map(|(i, _)| i)
        .collect();
}

impl Junctions {
    /// The junction whose patch holds `point`, the nearest if several.
    #[must_use]
    pub fn at(&self, point: (f64, f64)) -> Option<&Junction> {
        self.near(point, 0.0)
    }

    /// The nearest junction within `reach` metres beyond whose patch `point` lies; `at` is `near` with a
    /// reach of 0. Chain ends and junction pieces use [`HOLD_M`].
    #[must_use]
    pub fn near(&self, point: (f64, f64), reach: f64) -> Option<&Junction> {
        self.index_near(point, reach)
            .and_then(|i| self.points.get(i))
    }

    /// The index in [`Junctions::points`] of what [`Junctions::near`] finds.
    #[must_use]
    fn index_near(&self, point: (f64, f64), reach: f64) -> Option<usize> {
        self.points
            .iter()
            .enumerate()
            .map(|(i, j)| (distance(j.at, point), i, j))
            .filter(|(d, _, j)| *d <= j.patch_m + reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, i, _)| i)
    }
}

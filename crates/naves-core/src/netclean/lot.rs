// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Step 0 of the clean-up: no streets inside a lot (see `docs/superpowers/specs/2026-11-21-car-parks-design.md`,
//! "No streets inside").
//!
//! A *group* is the lots that touch ([`groups`]); its shape is the union of its members. A stretch lies *inside* a
//! group if every metre-sample of it is inside a member or within [`LOT_INSIDE_TOL_M`] of a member's edge, and at
//! least [`LOT_INSIDE_SHARE`] of the samples are inside by more than that (a road drawn on the outline stays). Every
//! inside stretch is removed, whatever its tags, except a bridge, a tunnel and a stretch with a point that is not a
//! number. After that a node inside a member by more than the tolerance with exactly one stretch left, not a node of
//! the map edge (a negative id), is a dead end in the lot: its stretch is cut where its line last enters the group
//! and ends [`LOT_ENTRY_M`] beyond that, at a new node (an id above all others, in the order of the stretch ids); the
//! piece keeps the stretch's id. Roads that cross a lot and roads with both ends outside stay whole.
//!
//! A group counts as one node for the connectivity check (a vehicle drives across a lot): the removal for a group
//! that would split a component counted that way is not made, and the group is reported.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use super::sidepath::splits;
use super::{degrees, middle, rebuilt, root};
use crate::lot::{
    Assessed, LOT_ENTRY_M, LOT_INSIDE_SHARE, LOT_INSIDE_TOL_M, Outcome, Polygon, ring_contains,
    segment_distance, touches,
};
use crate::roadnet::{
    EPS_M, RoadGraph, Stretch, first_free_node, point_along, polyline_length, samples, slice_points,
};

/// A position in map metres.
type Point = (f64, f64);

/// A point closer than this to an outline is on it, in metres.
const EDGE_M: f64 = 1e-6;
/// How far either side of a crossing the line is probed for being inside, in metres.
const PROBE_M: f64 = 1e-3;

/// A group of lots that touch: its members and their outlines.
///
/// [`LotGroup::group`] is the group's index in the list [`groups`] returns (ordered by the lowest member id); it is
/// not the group number of [`Assessed::group`], which is the index of a smooth surface. [`groups`] holds every lot that
/// is not excluded and not a street strip, smoothed or not, whereas the surface groups hold only the smoothed ones, so
/// the two numberings differ (a lot outside every surface group has `Assessed::group == NO_GROUP`).
#[derive(Debug, Clone, PartialEq)]
pub struct LotGroup {
    /// The group's number: its index in the list [`groups`] returned.
    pub group: usize,
    /// The ids of the members, sorted.
    pub members: Vec<String>,
    /// The members' outer rings and holes, in member order.
    pub polygons: Vec<Polygon>,
    /// The members' areas ([`LotArea::area_m2`](crate::lot::LotArea::area_m2): the samples of a 1 m grid inside, so a
    /// self-intersecting outline is counted as [`contains`](crate::lot::contains) sees it), in member order. The
    /// smallest member that holds a road is the one reported for it.
    pub areas_m2: Vec<f64>,
}

/// A stretch wholly inside a lot that was removed.
#[derive(Debug, Clone, PartialEq)]
pub struct DroppedInLot {
    /// The id of the stretch.
    pub stretch: String,
    /// The id of the lot it lies in: the smallest member of its group that holds the stretch.
    pub lot: String,
    /// Its length in metres.
    pub length_m: f64,
    /// Its middle, in map metres.
    pub at: (f64, f64),
}

/// An access road that was cut at the edge of a lot.
#[derive(Debug, Clone, PartialEq)]
pub struct CutAccess {
    /// The id of the stretch, which it keeps.
    pub stretch: String,
    /// The id of the lot it enters: the smallest member of its group that contains the new end.
    pub lot: String,
    /// The length cut off, in metres.
    pub cut_m: f64,
    /// Where the road now ends, in map metres.
    pub entry: (f64, f64),
    /// The new node at that end.
    pub node: i64,
}

/// The groups of the lots that are not excluded and not street strips (`Outcome::Excluded`,
/// `Outcome::SurfaceOnly { reason: "street_side" }`): lots that touch (an outline point of one inside the other, or
/// edges within [`LOT_INSIDE_TOL_M`]) are one group, a surface-only `no_vertex` lot included.
/// The lots are looked at in the order of their ids and the groups are ordered by their lowest member id, so the
/// result does not depend on the order of `assessed`.
#[must_use]
pub fn groups(assessed: &[Assessed]) -> Vec<LotGroup> {
    let mut lots: Vec<&Assessed> = assessed
        .iter()
        .filter(|a| {
            !matches!(
                a.outcome,
                Outcome::Excluded { .. }
                    | Outcome::SurfaceOnly {
                        reason: "street_side"
                    }
            )
        })
        .collect();
    lots.sort_by(|a, b| a.area.id.cmp(&b.area.id));
    let mut parent: Vec<usize> = (0..lots.len()).collect();
    for (i, a) in lots.iter().enumerate() {
        for (j, b) in lots.iter().enumerate().skip(i + 1) {
            if touches(&a.area, &b.area, LOT_INSIDE_TOL_M) {
                let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
                if let Some(slot) = parent.get_mut(ri.max(rj)) {
                    *slot = ri.min(rj);
                }
            }
        }
    }
    let mut by_root: BTreeMap<usize, Vec<&Assessed>> = BTreeMap::new();
    for (i, lot) in lots.iter().enumerate() {
        by_root.entry(root(&mut parent, i)).or_default().push(*lot);
    }
    by_root
        .into_values()
        .enumerate()
        .map(|(group, members)| LotGroup {
            group,
            members: members.iter().map(|m| m.area.id.clone()).collect(),
            polygons: members
                .iter()
                .map(|m| (m.area.outer.clone(), m.area.holes.clone()))
                .collect(),
            areas_m2: members.iter().map(|m| m.area.area_m2).collect(),
        })
        .collect()
}

// ----- shapes -----

fn polygon_contains(polygon: &Polygon, p: Point) -> bool {
    ring_contains(&polygon.0, p) && !polygon.1.iter().any(|hole| ring_contains(hole, p))
}

fn edge_distance(polygon: &Polygon, p: Point) -> f64 {
    std::iter::once(&polygon.0)
        .chain(polygon.1.iter())
        .flat_map(|ring| ring.windows(2))
        .filter_map(|pair| match pair {
            [a, b] => Some(segment_distance(p, *a, *b)),
            _ => None,
        })
        .fold(f64::INFINITY, f64::min)
}

/// Inside the polygon or within the tolerance of its edge.
fn is_near(polygon: &Polygon, p: Point) -> bool {
    polygon_contains(polygon, p) || edge_distance(polygon, p) <= LOT_INSIDE_TOL_M
}

/// Inside the polygon by more than the tolerance.
fn is_strict(polygon: &Polygon, p: Point) -> bool {
    polygon_contains(polygon, p) && edge_distance(polygon, p) > LOT_INSIDE_TOL_M
}

/// A group with what the tests on it need.
struct Shape<'a> {
    group: &'a LotGroup,
    /// min x, min y, max x, max y of the members' outer rings, widened by the tolerance.
    bounds: (f64, f64, f64, f64),
    /// The members' indices, smallest area ([`LotGroup::areas_m2`]) first (the order of the ids on a tie).
    by_size: Vec<usize>,
}

impl<'a> Shape<'a> {
    fn new(group: &'a LotGroup) -> Self {
        let mut bounds = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for (outer, _) in &group.polygons {
            for &(x, y) in outer {
                bounds = (
                    bounds.0.min(x),
                    bounds.1.min(y),
                    bounds.2.max(x),
                    bounds.3.max(y),
                );
            }
        }
        let mut by_size: Vec<(usize, f64)> = (0..group.polygons.len())
            .map(|i| (i, group.areas_m2.get(i).copied().unwrap_or(f64::INFINITY)))
            .collect();
        by_size.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        Shape {
            group,
            bounds: (
                bounds.0 - LOT_INSIDE_TOL_M,
                bounds.1 - LOT_INSIDE_TOL_M,
                bounds.2 + LOT_INSIDE_TOL_M,
                bounds.3 + LOT_INSIDE_TOL_M,
            ),
            by_size: by_size.into_iter().map(|(i, _)| i).collect(),
        }
    }

    /// Whether `p` can be near the group at all (false for a point that is not a number).
    fn in_bounds(&self, p: Point) -> bool {
        p.0 >= self.bounds.0 && p.0 <= self.bounds.2 && p.1 >= self.bounds.1 && p.1 <= self.bounds.3
    }

    fn near(&self, p: Point) -> bool {
        self.in_bounds(p) && self.group.polygons.iter().any(|m| is_near(m, p))
    }

    fn strict(&self, p: Point) -> bool {
        self.in_bounds(p) && self.group.polygons.iter().any(|m| is_strict(m, p))
    }

    /// Inside a member and not on its outline (more than [`EDGE_M`] from it).
    fn well_inside(&self, p: Point) -> bool {
        self.group
            .polygons
            .iter()
            .any(|m| polygon_contains(m, p) && edge_distance(m, p) > EDGE_M)
    }

    /// The id of a member by its index.
    fn member(&self, i: usize) -> String {
        self.group.members.get(i).cloned().unwrap_or_default()
    }

    /// The smallest member that holds all of `places`, else the smallest that is near the one in the middle, else the
    /// first.
    fn lot_of(&self, places: &[Point]) -> String {
        let holding = self.by_size.iter().copied().find(|&i| {
            self.group
                .polygons
                .get(i)
                .is_some_and(|m| holds(std::slice::from_ref(m), places))
        });
        let middle_one = places.get(places.len() / 2).copied().and_then(|p| {
            self.by_size
                .iter()
                .copied()
                .find(|&i| self.group.polygons.get(i).is_some_and(|m| is_near(m, p)))
        });
        self.member(holding.or(middle_one).unwrap_or(0))
    }

    /// The smallest member that is near `p`, else the first.
    fn lot_at(&self, p: Point) -> String {
        let found = self
            .by_size
            .iter()
            .copied()
            .find(|&i| self.group.polygons.get(i).is_some_and(|m| is_near(m, p)));
        self.member(found.unwrap_or(0))
    }
}

/// A count as a number.
fn count(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// Whether the samples are all inside (or near the edge of) one of `polygons` and at least [`LOT_INSIDE_SHARE`] of
/// them inside by more than the tolerance.
fn holds(polygons: &[Polygon], places: &[Point]) -> bool {
    if places.is_empty() {
        return false;
    }
    let mut strict = 0_usize;
    for &p in places {
        if !polygons.iter().any(|m| is_near(m, p)) {
            return false;
        }
        if polygons.iter().any(|m| is_strict(m, p)) {
            strict += 1;
        }
    }
    count(strict) >= LOT_INSIDE_SHARE * count(places.len())
}

// ----- one group -----

/// A stretch that may be judged: not a bridge, not a tunnel, every point a number, a line.
fn judgeable(s: &Stretch) -> bool {
    !s.bridge
        && !s.tunnel
        && s.points.len() >= 2
        && s.points.iter().all(|p| p.0.is_finite() && p.1.is_finite())
}

/// The new nodes of a cut stretch: which end (true for the `end`) and the node.
type NewNodes = Vec<(bool, i64)>;

/// One end of a stretch that is cut.
struct CutEnd {
    /// The stretch's `end` (else its `start`).
    at_end: bool,
    /// What is reported; the node is put in when the ids are given.
    report: CutAccess,
}

/// A stretch to be cut at one or both of its dead ends.
struct Cut {
    index: usize,
    /// The line as it is after, from the stretch's start to its end as before.
    points: Vec<Point>,
    before_m: f64,
    /// The cut ends, the start's first.
    ends: Vec<CutEnd>,
}

/// What a group does to the graph.
struct Plan {
    /// The group's number.
    group: usize,
    removed: Vec<(usize, DroppedInLot)>,
    cuts: Vec<Cut>,
    /// The ids of the inside stretches that stay because they are part of a road through the lot.
    through: Vec<String>,
    /// The nodes that the removal leaves with exactly one stretch (cut or not): the stub step leaves them alone.
    ends: BTreeSet<i64>,
}

impl Plan {
    fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.cuts.is_empty() && self.through.is_empty()
    }
}

/// The parameter along `from`-`to` where it meets `edge_from`-`edge_to` (0 to 1), if it does and they are not
/// parallel.
fn crossing(from: Point, to: Point, edge_from: Point, edge_to: Point) -> Option<f64> {
    let line = (to.0 - from.0, to.1 - from.1);
    let edge = (edge_to.0 - edge_from.0, edge_to.1 - edge_from.1);
    let den = line.0 * edge.1 - line.1 * edge.0;
    if den.abs() < 1e-12 {
        return None;
    }
    let gap = (edge_from.0 - from.0, edge_from.1 - from.1);
    let along_line = (gap.0 * edge.1 - gap.1 * edge.0) / den;
    let along_edge = (gap.0 * line.1 - gap.1 * line.0) / den;
    ((0.0..=1.0).contains(&along_line) && (0.0..=1.0).contains(&along_edge)).then_some(along_line)
}

/// How far along `points` the line last enters the group: the last crossing of an outline with the line not well
/// inside a member just before it and well inside just after (so a road that runs along an outline enters where it
/// turns away from it, whichever edge it is).
fn last_entry(shape: &Shape<'_>, points: &[Point]) -> Option<f64> {
    let total = polyline_length(points);
    let mut hits: Vec<f64> = Vec::new();
    let mut at = 0.0;
    for pair in points.windows(2) {
        let [a, b] = pair else { continue };
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        for (outer, holes) in &shape.group.polygons {
            for ring in std::iter::once(outer).chain(holes.iter()) {
                for edge in ring.windows(2) {
                    if let [c, d] = edge
                        && let Some(t) = crossing(*a, *b, *c, *d)
                    {
                        hits.push(at + t * len);
                    }
                }
            }
        }
        at += len;
    }
    hits.sort_by(|x, y| y.total_cmp(x));
    hits.into_iter().find(|&d| {
        let probe = |along: f64| point_along(points, along.clamp(0.0, total));
        !shape.well_inside(probe(d - PROBE_M)) && shape.well_inside(probe(d + PROBE_M))
    })
}

/// `line` (a dead end last) cut [`LOT_ENTRY_M`] beyond its last entry: the new line, its end and the length cut off.
fn cut_end(shape: &Shape<'_>, line: &[Point]) -> Option<(Vec<Point>, Point, f64)> {
    let total = polyline_length(line);
    let stop = last_entry(shape, line)? + LOT_ENTRY_M;
    if total - stop <= EPS_M {
        return None;
    }
    let points = slice_points(line, 0.0, stop);
    let entry = points.last().copied()?;
    (points.len() >= 2).then_some((points, entry, total - stop))
}

/// The inside stretches (graph index and stretch) that lie on the shortest chain (by length, then by the order of the
/// stretches) between two exits: nodes that are an end of an inside stretch and for which `exit` is true. The result
/// holds graph indices.
fn through_chains(inside: &[(usize, &Stretch)], exit: &dyn Fn(i64) -> bool) -> BTreeSet<usize> {
    let mut at: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for (k, (_, s)) in inside.iter().enumerate() {
        at.entry(s.start).or_default().push(k);
        at.entry(s.end).or_default().push(k);
    }
    let exits: Vec<i64> = at.keys().copied().filter(|&n| exit(n)).collect();
    let mut kept: BTreeSet<usize> = BTreeSet::new();
    for (k, &from) in exits.iter().enumerate() {
        // Dijkstra over the inside stretches from `from`; a label is (metres, the stretches walked)
        let mut best: BTreeMap<i64, (f64, Vec<usize>)> = BTreeMap::new();
        let mut done: BTreeSet<i64> = BTreeSet::new();
        best.insert(from, (0.0, Vec::new()));
        while let Some((&node, label)) = best
            .iter()
            .filter(|(n, _)| !done.contains(*n))
            .min_by(|a, b| a.1.0.total_cmp(&b.1.0).then_with(|| a.1.1.cmp(&b.1.1)))
        {
            let label = label.clone();
            done.insert(node);
            for &j in at.get(&node).map(Vec::as_slice).unwrap_or_default() {
                let Some((_, s)) = inside.get(j) else {
                    continue;
                };
                let next = if s.start == node { s.end } else { s.start };
                let mut path = label.1.clone();
                path.push(j);
                let reach = (label.0 + polyline_length(&s.points), path);
                let better = best.get(&next).is_none_or(|old| {
                    reach.0.total_cmp(&old.0).then_with(|| reach.1.cmp(&old.1)) == Ordering::Less
                });
                if better && !done.contains(&next) {
                    best.insert(next, reach);
                }
            }
        }
        for &to in exits.iter().skip(k + 1) {
            if let Some((_, path)) = best.get(&to) {
                kept.extend(path.iter().filter_map(|&j| inside.get(j).map(|(i, _)| *i)));
            }
        }
    }
    kept
}

/// The access roads of `shape` to cut: stretches with a dead end inside it (`left` is what stays at a node).
fn plan_cuts(
    graph: &RoadGraph,
    judged: &[bool],
    shape: &Shape<'_>,
    claimed: &mut BTreeSet<usize>,
    left: &dyn Fn(i64) -> usize,
) -> Vec<Cut> {
    let mut cuts: Vec<Cut> = Vec::new();
    for (i, s) in graph.stretches.iter().enumerate() {
        if claimed.contains(&i) || !judged.get(i).copied().unwrap_or(false) {
            continue;
        }
        let ends = [
            (s.start, s.points.first(), false),
            (s.end, s.points.last(), true),
        ];
        let mut line = s.points.clone();
        let mut cut_ends: Vec<CutEnd> = Vec::new();
        for (node, place, at_end) in ends {
            let Some(&place) = place else { continue };
            if !shape.in_bounds(place) || node <= 0 || left(node) != 1 || !shape.strict(place) {
                continue;
            }
            let oriented: Vec<Point> = if at_end {
                line.clone()
            } else {
                line.iter().rev().copied().collect()
            };
            let Some((mut cut, entry, cut_m)) = cut_end(shape, &oriented) else {
                continue;
            };
            if !at_end {
                cut.reverse();
            }
            line = cut;
            cut_ends.push(CutEnd {
                at_end,
                report: CutAccess {
                    stretch: s.id.clone(),
                    lot: shape.lot_at(entry),
                    cut_m,
                    entry,
                    node: 0,
                },
            });
        }
        if !cut_ends.is_empty() {
            claimed.insert(i);
            cuts.push(Cut {
                index: i,
                points: line,
                before_m: polyline_length(&s.points),
                ends: cut_ends,
            });
        }
    }
    cuts
}

/// What `shape` does to the graph: the stretches inside it (less those of a road through it) and the access roads to
/// cut. Stretches in `claimed` are left to the group that took them; the ones this plan takes or keeps are added.
/// `judged` says per stretch whether it can be judged at all ([`judgeable`]).
fn plan_group(
    graph: &RoadGraph,
    input: &BTreeMap<i64, usize>,
    judged: &[bool],
    shape: &Shape<'_>,
    claimed: &mut BTreeSet<usize>,
) -> Plan {
    let mut plan = Plan {
        group: shape.group.group,
        removed: Vec::new(),
        cuts: Vec::new(),
        through: Vec::new(),
        ends: BTreeSet::new(),
    };
    let mut inside: Vec<(usize, &Stretch, DroppedInLot)> = Vec::new();
    for (i, s) in graph.stretches.iter().enumerate() {
        if claimed.contains(&i) || !judged.get(i).copied().unwrap_or(false) {
            continue;
        }
        if !s.points.first().is_some_and(|&p| shape.near(p)) {
            continue;
        }
        let places: Vec<Point> = samples(&s.points).iter().map(|m| m.at).collect();
        if !holds(&shape.group.polygons, &places) {
            continue;
        }
        claimed.insert(i);
        inside.push((
            i,
            s,
            DroppedInLot {
                stretch: s.id.clone(),
                lot: shape.lot_of(&places),
                length_m: polyline_length(&s.points),
                at: middle(&s.points),
            },
        ));
    }
    // what stays at a node if every inside stretch went
    let mut lost: BTreeMap<i64, usize> = BTreeMap::new();
    for (_, s, _) in &inside {
        for node in [s.start, s.end] {
            *lost.entry(node).or_default() += 1;
        }
    }
    let left_of = |lost: &BTreeMap<i64, usize>, node: i64| {
        input
            .get(&node)
            .copied()
            .unwrap_or(0)
            .saturating_sub(lost.get(&node).copied().unwrap_or(0))
    };
    // a road through the lot stays whole: the shortest chain between two nodes that a road leaves the lot from
    let pairs: Vec<(usize, &Stretch)> = inside.iter().map(|(i, s, _)| (*i, *s)).collect();
    let through = through_chains(&pairs, &|n| left_of(&lost, n) >= 1);
    let mut gone: BTreeMap<i64, usize> = BTreeMap::new();
    for (i, s, report) in inside {
        if through.contains(&i) {
            plan.through.push(report.stretch);
        } else {
            for node in [s.start, s.end] {
                *gone.entry(node).or_default() += 1;
            }
            plan.removed.push((i, report));
        }
    }
    let lost = gone;
    plan.ends = lost
        .keys()
        .copied()
        .filter(|&n| left_of(&lost, n) == 1)
        .collect();
    plan.cuts = plan_cuts(graph, judged, shape, claimed, &|n| left_of(&lost, n));
    plan
}

// ----- the graph -----

/// What the lot step did.
#[derive(Debug, Clone, PartialEq)]
pub struct LotStep {
    /// The graph after the step.
    pub graph: RoadGraph,
    /// The stretches removed.
    pub dropped: Vec<DroppedInLot>,
    /// The access roads cut.
    pub access: Vec<CutAccess>,
    /// The groups left as they are because their removal would have split a component.
    pub unchanged: Vec<usize>,
    /// The ids of the inside stretches that stay because they are part of a road through a lot.
    pub through: Vec<String>,
    /// The nodes the step leaves with exactly one stretch, cut or not (the new ends of the cut access roads included):
    /// the stub step must not take their stretch for a dead-end stub.
    pub entries: BTreeSet<i64>,
}

/// The graph after `plans`: their stretches removed, their access roads cut, the new nodes numbered from `first` in
/// the order of the stretch ids (the start of a stretch before its end); the removed stretches, the cuts and the
/// nodes the step leaves as the only end of a stretch.
fn apply(graph: &RoadGraph, plans: &[&Plan], first: i64) -> LotStep {
    let mut dropped: Vec<DroppedInLot> = Vec::new();
    let mut gone: BTreeSet<usize> = BTreeSet::new();
    let mut cuts: Vec<&Cut> = Vec::new();
    let mut through: Vec<String> = Vec::new();
    let mut entries: BTreeSet<i64> = BTreeSet::new();
    for plan in plans {
        for (i, report) in &plan.removed {
            gone.insert(*i);
            dropped.push(report.clone());
        }
        cuts.extend(plan.cuts.iter());
        through.extend(plan.through.iter().cloned());
        entries.extend(plan.ends.iter().copied());
    }
    through.sort();
    dropped.sort_by(|a, b| a.stretch.cmp(&b.stretch));
    cuts.sort_by(|a, b| {
        graph
            .stretches
            .get(a.index)
            .map(|s| &s.id)
            .cmp(&graph.stretches.get(b.index).map(|s| &s.id))
            .then(a.index.cmp(&b.index))
    });
    let mut access: Vec<CutAccess> = Vec::new();
    let mut at: BTreeMap<usize, (NewNodes, &Cut)> = BTreeMap::new();
    let mut node = first;
    for cut in cuts {
        let mut nodes = Vec::new();
        for end in &cut.ends {
            let mut report = end.report.clone();
            report.node = node;
            entries.insert(node);
            nodes.push((end.at_end, node));
            access.push(report);
            node = node.saturating_add(1);
        }
        at.insert(cut.index, (nodes, cut));
    }
    if dropped.is_empty() && access.is_empty() {
        return LotStep {
            graph: graph.clone(),
            dropped,
            access,
            unchanged: Vec::new(),
            through,
            entries,
        };
    }
    let stretches: Vec<Stretch> = graph
        .stretches
        .iter()
        .enumerate()
        .filter(|(i, _)| !gone.contains(i))
        .map(|(i, s)| match at.get(&i) {
            Some((nodes, cut)) => {
                let (mut start, mut end) = (s.start, s.end);
                for &(at_end, node) in nodes {
                    if at_end {
                        end = node;
                    } else {
                        start = node;
                    }
                }
                let mut points = cut.points.clone();
                if start > end {
                    std::mem::swap(&mut start, &mut end);
                    points.reverse();
                }
                Stretch {
                    points,
                    start,
                    end,
                    ..s.clone()
                }
                .rescaled(cut.before_m)
            }
            None => s.clone(),
        })
        .collect();
    LotStep {
        graph: rebuilt(stretches),
        dropped,
        access,
        unchanged: Vec::new(),
        through,
        entries,
    }
}

/// A stretch with only its nodes, for counting components.
fn thin(s: &Stretch, start: i64, end: i64) -> Stretch {
    Stretch {
        id: String::new(),
        family: s.family,
        drivable: s.drivable,
        bridge: s.bridge,
        tunnel: s.tunnel,
        roundabout: s.roundabout,
        layer: s.layer,
        points: Vec::new(),
        start,
        end,
        bridge_kind: None,
        bridge_offset: 0.0,
        use_sidepath: false,
        paved_m: 0.0,
        unpaved_m: 0.0,
    }
}

/// The group of a node (as the virtual node it is made), by the node's place.
fn virtual_node(shapes: &[Shape<'_>], point: Option<&Point>) -> Option<i64> {
    let k = shapes
        .iter()
        .position(|sh| point.is_some_and(|&p| sh.near(p)))?;
    Some(i64::try_from(k).map_or(i64::MIN, |k| i64::MIN.saturating_add(k)))
}

/// The graph with every node that is inside (or at the edge of) a group made the node of that group, for
/// connectivity: only the nodes of the stretches are left, in no particular geometry. `known` holds the nodes of
/// `graph` below `fresh` (their place does not change from one call to the next); the others are looked up.
fn quotient(
    graph: &RoadGraph,
    shapes: &[Shape<'_>],
    fresh: i64,
    known: &mut BTreeMap<i64, Option<i64>>,
) -> RoadGraph {
    let mut local: BTreeMap<i64, Option<i64>> = BTreeMap::new();
    for s in &graph.stretches {
        for (node, point) in [(s.start, s.points.first()), (s.end, s.points.last())] {
            let table = if node < fresh {
                &mut *known
            } else {
                &mut local
            };
            table
                .entry(node)
                .or_insert_with(|| virtual_node(shapes, point));
        }
    }
    let map = |node: i64| {
        let found = if node < fresh {
            known.get(&node)
        } else {
            local.get(&node)
        };
        found.copied().flatten().unwrap_or(node)
    };
    RoadGraph {
        stretches: graph
            .stretches
            .iter()
            .map(|s| thin(s, map(s.start), map(s.end)))
            .collect(),
        crossings: Vec::new(),
    }
}

/// [`drop_lot_roads_full`] with the test for a split as an argument: it gets the graph before and after, each with
/// every group made one node.
fn drop_lot_roads_with(
    graph: &RoadGraph,
    groups: &[LotGroup],
    splits: &dyn Fn(&RoadGraph, &RoadGraph) -> bool,
) -> LotStep {
    let shapes: Vec<Shape<'_>> = groups.iter().map(Shape::new).collect();
    let input = degrees(graph);
    let judged: Vec<bool> = graph.stretches.iter().map(judgeable).collect();
    let mut claimed: BTreeSet<usize> = BTreeSet::new();
    let plans: Vec<Plan> = shapes
        .iter()
        .map(|shape| plan_group(graph, &input, &judged, shape, &mut claimed))
        .filter(|plan| !plan.is_empty())
        .collect();
    if plans.is_empty() {
        return LotStep {
            graph: graph.clone(),
            dropped: Vec::new(),
            access: Vec::new(),
            unchanged: Vec::new(),
            through: Vec::new(),
            entries: BTreeSet::new(),
        };
    }
    let first = first_free_node(graph);
    let mut known: BTreeMap<i64, Option<i64>> = BTreeMap::new();
    let before = quotient(graph, &shapes, first, &mut known);
    let mut splits_with = |chosen: &[&Plan]| {
        let after = apply(graph, chosen, first).graph;
        splits(&before, &quotient(&after, &shapes, first, &mut known))
    };
    // each group alone first (they touch different stretches); the ones that fit are then tried together, one at a
    // time, in case they cut a piece off between them
    let fitting: Vec<&Plan> = plans.iter().filter(|p| !splits_with(&[p])).collect();
    let accepted: Vec<&Plan> = if splits_with(&fitting) {
        let mut accepted: Vec<&Plan> = Vec::new();
        for plan in fitting {
            let mut trial = accepted.clone();
            trial.push(plan);
            if !splits_with(&trial) {
                accepted = trial;
            }
        }
        accepted
    } else {
        fitting
    };
    let unchanged: Vec<usize> = plans
        .iter()
        .filter(|p| !accepted.iter().any(|a| a.group == p.group))
        .map(|p| p.group)
        .collect();
    LotStep {
        unchanged,
        ..apply(graph, &accepted, first)
    }
}

/// [`drop_lot_roads`] with everything the step found out, for [`super::clean_with`].
#[must_use]
pub fn drop_lot_roads_full(graph: &RoadGraph, groups: &[LotGroup]) -> LotStep {
    drop_lot_roads_with(graph, groups, &splits)
}

/// Takes the roads inside the groups out of the graph and cuts the access roads at the edge, as described in the
/// module. Returns the graph, the stretches removed and the access roads cut (each in the order of the stretch ids),
/// and the numbers of the groups whose removal would have split a component (with every group counted as one node) and
/// that were left as they are. A stretch that two groups both claim goes to the first. See [`drop_lot_roads_full`] for
/// the rest.
#[must_use]
pub fn drop_lot_roads(
    graph: &RoadGraph,
    groups: &[LotGroup],
) -> (RoadGraph, Vec<DroppedInLot>, Vec<CutAccess>, Vec<usize>) {
    let step = drop_lot_roads_full(graph, groups);
    (step.graph, step.dropped, step.access, step.unchanged)
}

/// The connected components of the graph with every group counted as one node: nodes inside a group, or at the
/// edge of it, are one. Without groups this is [`super::components`].
#[must_use]
pub fn components_with(graph: &RoadGraph, groups: &[LotGroup]) -> usize {
    components_each(&[graph], groups)
        .first()
        .copied()
        .unwrap_or_default()
}

/// [`components_with`] for several graphs, the groups' shapes made once.
pub(super) fn components_each(graphs: &[&RoadGraph], groups: &[LotGroup]) -> Vec<usize> {
    if groups.is_empty() {
        return graphs.iter().map(|g| super::components(g)).collect();
    }
    let shapes: Vec<Shape<'_>> = groups.iter().map(Shape::new).collect();
    graphs
        .iter()
        .map(|g| super::components(&quotient(g, &shapes, i64::MIN, &mut BTreeMap::new())))
        .collect()
}

/// The junctions (three or more stretch ends) of `before` at which a stretch was removed and that are no junction in
/// `after` (a node of two stretches, one or none).
pub(super) fn junctions_removed(
    before: &RoadGraph,
    after: &RoadGraph,
    dropped: &[DroppedInLot],
) -> usize {
    let (was, is) = (degrees(before), degrees(after));
    let ids: BTreeSet<&str> = dropped.iter().map(|d| d.stretch.as_str()).collect();
    before
        .stretches
        .iter()
        .filter(|s| ids.contains(s.id.as_str()))
        .flat_map(|s| [s.start, s.end])
        .filter(|n| was.get(n).copied().unwrap_or(0) >= 3 && is.get(n).copied().unwrap_or(0) < 3)
        .collect::<BTreeSet<i64>>()
        .len()
}

#[cfg(test)]
mod tests {
    //! The test for a split is an argument of the private variant, so a split that "by construction" never happens
    //! can be forced.
    use super::*;
    use crate::catalog::Family;
    use crate::features::{Feature, FeatureKind, Source};
    use crate::roadnet::build_graph;

    fn road(way: i64, nodes: &[i64], points: &[Point]) -> Feature {
        Feature {
            id: format!("osm/way/{way}/0"),
            way_id: way,
            source: Source::Osm,
            kind: FeatureKind::Road {
                family: Family::Grav,
                drivable: true,
                bridge: false,
                tunnel: false,
                roundabout: false,
                layer: 0,
                use_sidepath: false,
                paving: None,
            },
            points: points.to_vec(),
            nodes: nodes.to_vec(),
        }
    }

    fn square(group: usize, x0: f64) -> LotGroup {
        let ring = vec![
            (x0, 0.0),
            (x0 + 40.0, 0.0),
            (x0 + 40.0, 40.0),
            (x0, 40.0),
            (x0, 0.0),
        ];
        LotGroup {
            group,
            members: vec![format!("osm/way/{group}")],
            polygons: vec![(ring, Vec::new())],
            areas_m2: vec![1600.0],
        }
    }

    /// A road inside each of two lots 200 m apart.
    fn two_lots() -> (RoadGraph, [LotGroup; 2]) {
        let g = build_graph(&[
            road(1, &[1, 2], &[(5.0, 20.0), (35.0, 20.0)]),
            road(2, &[3, 4], &[(205.0, 20.0), (235.0, 20.0)]),
        ]);
        (g, [square(0, 0.0), square(1, 200.0)])
    }

    #[test]
    fn a_removal_that_would_split_leaves_the_group_as_it_is() {
        let (g, lots) = two_lots();
        let step = drop_lot_roads_with(&g, &lots, &|_, _| true);
        assert_eq!(
            (&step.graph, step.dropped.len(), step.access.len()),
            (&g, 0, 0)
        );
        assert_eq!(step.unchanged, [0, 1]);
        // the real test finds nothing to cut off
        let (out, dropped, _, split) = drop_lot_roads(&g, &lots);
        assert_eq!((out.stretches.len(), dropped.len(), split.len()), (0, 2, 0));
    }

    #[test]
    fn the_groups_are_tried_one_at_a_time_and_the_first_that_fits_goes() {
        let (g, lots) = two_lots();
        // refuses a removal of two stretches at once, not of one
        let refuses_two = |before: &RoadGraph, after: &RoadGraph| {
            before.stretches.len() - after.stretches.len() > 1
        };
        let step = drop_lot_roads_with(&g, &lots, &refuses_two);
        assert_eq!(step.unchanged, [1]);
        assert_eq!(step.graph.stretches.len(), 1);
        assert_eq!(step.dropped.len(), 1);
        assert_eq!(step.dropped[0].stretch, "1:1-2");
    }
}

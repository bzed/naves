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

/// The area of a polygon by the shoelace formula, holes taken off (never below 0).
fn polygon_area(polygon: &Polygon) -> f64 {
    let ring = |points: &[Point]| -> f64 {
        points
            .windows(2)
            .map(|pair| match pair {
                [a, b] => a.0 * b.1 - b.0 * a.1,
                _ => 0.0,
            })
            .sum::<f64>()
            .abs()
            / 2.0
    };
    (ring(&polygon.0) - polygon.1.iter().map(|h| ring(h)).sum::<f64>()).max(0.0)
}

/// A group with what the tests on it need.
struct Shape<'a> {
    group: &'a LotGroup,
    /// min x, min y, max x, max y of the members' outer rings, widened by the tolerance.
    bounds: (f64, f64, f64, f64),
    /// The members' indices, smallest area first (the order of the ids on a tie).
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
        let mut by_size: Vec<(usize, f64)> = group
            .polygons
            .iter()
            .enumerate()
            .map(|(i, p)| (i, polygon_area(p)))
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

    /// Inside a member, no tolerance.
    fn in_union(&self, p: Point) -> bool {
        self.group.polygons.iter().any(|m| polygon_contains(m, p))
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

/// A stretch to be cut: where it ends, from the node that stays to the new end.
struct Cut {
    index: usize,
    /// The node that stays.
    other: i64,
    /// The line from `other` to the new end.
    points: Vec<Point>,
    before_m: f64,
    report: CutAccess,
}

/// What a group does to the graph.
struct Plan {
    /// The group's number.
    group: usize,
    removed: Vec<(usize, DroppedInLot)>,
    cuts: Vec<Cut>,
}

impl Plan {
    fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.cuts.is_empty()
    }
}

/// The parameter along `a`-`b` where it meets `c`-`d` (0 to 1), if it does and they are not parallel.
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

/// How far along `points` the line last enters the group: the last crossing of an outline with the line outside the
/// union just before it and inside just after.
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
        !shape.in_union(probe(d - PROBE_M)) && shape.in_union(probe(d + PROBE_M))
    })
}

/// The cut of stretch `index` at its end `dead_end` (true for its `end`), if there is one to make.
fn cut_at(shape: &Shape<'_>, index: usize, s: &Stretch, dead_end: bool) -> Option<Cut> {
    let line: Vec<Point> = if dead_end {
        s.points.clone()
    } else {
        s.points.iter().rev().copied().collect()
    };
    let total = polyline_length(&line);
    let enter = last_entry(shape, &line)?;
    let stop = enter + LOT_ENTRY_M;
    if total - stop <= EPS_M {
        return None;
    }
    let points = slice_points(&line, 0.0, stop);
    let entry = points.last().copied()?;
    if points.len() < 2 {
        return None;
    }
    Some(Cut {
        index,
        other: if dead_end { s.start } else { s.end },
        before_m: polyline_length(&s.points),
        report: CutAccess {
            stretch: s.id.clone(),
            lot: shape.lot_at(entry),
            cut_m: total - stop,
            entry,
            node: 0,
        },
        points,
    })
}

/// What `shape` does to the graph: the stretches inside it and the access roads to cut. Stretches in `claimed` are
/// left to the group that took them; the ones this plan takes are added.
fn plan_group(
    graph: &RoadGraph,
    input: &BTreeMap<i64, usize>,
    shape: &Shape<'_>,
    claimed: &mut BTreeSet<usize>,
) -> Plan {
    let mut plan = Plan {
        group: shape.group.group,
        removed: Vec::new(),
        cuts: Vec::new(),
    };
    let mut lost: BTreeMap<i64, usize> = BTreeMap::new();
    for (i, s) in graph.stretches.iter().enumerate() {
        if claimed.contains(&i) || !judgeable(s) {
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
        for node in [s.start, s.end] {
            *lost.entry(node).or_default() += 1;
        }
        plan.removed.push((
            i,
            DroppedInLot {
                stretch: s.id.clone(),
                lot: shape.lot_of(&places),
                length_m: polyline_length(&s.points),
                at: middle(&s.points),
            },
        ));
    }
    let left = |node: i64| {
        input
            .get(&node)
            .copied()
            .unwrap_or(0)
            .saturating_sub(lost.get(&node).copied().unwrap_or(0))
    };
    for (i, s) in graph.stretches.iter().enumerate() {
        if claimed.contains(&i) || !judgeable(s) {
            continue;
        }
        let ends = [
            (s.end, s.points.last(), true),
            (s.start, s.points.first(), false),
        ];
        for (node, place, dead_end) in ends {
            let Some(&place) = place else { continue };
            if node > 0
                && left(node) == 1
                && shape.strict(place)
                && let Some(cut) = cut_at(shape, i, s, dead_end)
            {
                claimed.insert(i);
                plan.cuts.push(cut);
                break;
            }
        }
    }
    plan
}

// ----- the graph -----

/// The graph after `plans`: their stretches removed, their access roads cut, the new nodes numbered from `first` in
/// the order of the stretch ids; the removed stretches and the cuts as reported.
fn apply(
    graph: &RoadGraph,
    plans: &[&Plan],
    first: i64,
) -> (RoadGraph, Vec<DroppedInLot>, Vec<CutAccess>) {
    let mut dropped: Vec<DroppedInLot> = Vec::new();
    let mut gone: BTreeSet<usize> = BTreeSet::new();
    let mut cuts: Vec<&Cut> = Vec::new();
    for plan in plans {
        for (i, report) in &plan.removed {
            gone.insert(*i);
            dropped.push(report.clone());
        }
        cuts.extend(plan.cuts.iter());
    }
    if dropped.is_empty() && cuts.is_empty() {
        return (graph.clone(), dropped, Vec::new());
    }
    dropped.sort_by(|a, b| a.stretch.cmp(&b.stretch));
    cuts.sort_by(|a, b| {
        a.report
            .stretch
            .cmp(&b.report.stretch)
            .then(a.index.cmp(&b.index))
    });
    let mut access: Vec<CutAccess> = Vec::new();
    let mut at: BTreeMap<usize, (i64, &Cut)> = BTreeMap::new();
    let mut node = first;
    for cut in cuts {
        let mut report = cut.report.clone();
        report.node = node;
        access.push(report);
        at.insert(cut.index, (node, cut));
        node = node.saturating_add(1);
    }
    let stretches: Vec<Stretch> = graph
        .stretches
        .iter()
        .enumerate()
        .filter(|(i, _)| !gone.contains(i))
        .map(|(i, s)| match at.get(&i) {
            Some(&(node, cut)) => Stretch {
                start: cut.other,
                end: node,
                points: cut.points.clone(),
                ..s.clone()
            }
            .rescaled(cut.before_m),
            None => s.clone(),
        })
        .collect();
    (rebuilt(stretches), dropped, access)
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

/// The graph with every node that is inside (or at the edge of) a group made the node of that group, for
/// connectivity: only the nodes of the stretches are left, in no particular geometry.
fn quotient(graph: &RoadGraph, shapes: &[Shape<'_>]) -> RoadGraph {
    let mut place: BTreeMap<i64, i64> = BTreeMap::new();
    let mut seen: BTreeSet<i64> = BTreeSet::new();
    for s in &graph.stretches {
        for (node, point) in [(s.start, s.points.first()), (s.end, s.points.last())] {
            if !seen.insert(node) {
                continue;
            }
            let found = point.and_then(|&p| shapes.iter().position(|sh| sh.near(p)));
            if let Some(k) = found {
                let virt = i64::try_from(k).map_or(i64::MIN, |k| i64::MIN.saturating_add(k));
                place.insert(node, virt);
            }
        }
    }
    let map = |node: i64| place.get(&node).copied().unwrap_or(node);
    RoadGraph {
        stretches: graph
            .stretches
            .iter()
            .map(|s| thin(s, map(s.start), map(s.end)))
            .collect(),
        crossings: Vec::new(),
    }
}

/// [`drop_lot_roads`] with the test for a split as an argument: it gets the graph before and after, each with
/// every group made one node.
fn drop_lot_roads_with(
    graph: &RoadGraph,
    groups: &[LotGroup],
    splits: &dyn Fn(&RoadGraph, &RoadGraph) -> bool,
) -> (RoadGraph, Vec<DroppedInLot>, Vec<CutAccess>, Vec<usize>) {
    let shapes: Vec<Shape<'_>> = groups.iter().map(Shape::new).collect();
    let input = degrees(graph);
    let mut claimed: BTreeSet<usize> = BTreeSet::new();
    let plans: Vec<Plan> = shapes
        .iter()
        .map(|shape| plan_group(graph, &input, shape, &mut claimed))
        .filter(|plan| !plan.is_empty())
        .collect();
    if plans.is_empty() {
        return (graph.clone(), Vec::new(), Vec::new(), Vec::new());
    }
    let first = first_free_node(graph);
    let before = quotient(graph, &shapes);
    let splits_with = |chosen: &[&Plan]| {
        let (after, _, _) = apply(graph, chosen, first);
        splits(&before, &quotient(&after, &shapes))
    };
    let every: Vec<&Plan> = plans.iter().collect();
    let accepted: Vec<&Plan> = if splits_with(&every) {
        // one group at a time: the ones that would cut a piece off stay as they are
        let mut accepted: Vec<&Plan> = Vec::new();
        for plan in &every {
            let mut trial = accepted.clone();
            trial.push(plan);
            if !splits_with(&trial) {
                accepted = trial;
            }
        }
        accepted
    } else {
        every
    };
    let unchanged: Vec<usize> = plans
        .iter()
        .filter(|p| !accepted.iter().any(|a| a.group == p.group))
        .map(|p| p.group)
        .collect();
    let (out, dropped, access) = apply(graph, &accepted, first);
    (out, dropped, access, unchanged)
}

/// Takes the roads inside the groups out of the graph and cuts the access roads at the edge, as described in the
/// module. Returns the graph, the stretches removed and the access roads cut (each in the order of the stretch ids),
/// and the numbers of the groups whose removal would have split a component (with every group counted as one node) and
/// that were left as they are. A stretch that two groups both claim goes to the first.
#[must_use]
pub fn drop_lot_roads(
    graph: &RoadGraph,
    groups: &[LotGroup],
) -> (RoadGraph, Vec<DroppedInLot>, Vec<CutAccess>, Vec<usize>) {
    drop_lot_roads_with(graph, groups, &splits)
}

/// The connected components of the graph with every group counted as one node: nodes inside a group, or at the
/// edge of it, are one. Without groups this is [`super::components`].
#[must_use]
pub fn components_with(graph: &RoadGraph, groups: &[LotGroup]) -> usize {
    if groups.is_empty() {
        return super::components(graph);
    }
    let shapes: Vec<Shape<'_>> = groups.iter().map(Shape::new).collect();
    super::components(&quotient(graph, &shapes))
}

/// The junctions (three or more stretch ends) at which a removed stretch ended.
pub(super) fn junctions_removed(graph: &RoadGraph, dropped: &[DroppedInLot]) -> usize {
    let degree = degrees(graph);
    let ids: BTreeSet<&str> = dropped.iter().map(|d| d.stretch.as_str()).collect();
    graph
        .stretches
        .iter()
        .filter(|s| ids.contains(s.id.as_str()))
        .flat_map(|s| [s.start, s.end])
        .filter(|n| degree.get(n).copied().unwrap_or(0) >= 3)
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
        let (out, dropped, cut, split) = drop_lot_roads_with(&g, &lots, &|_, _| true);
        assert_eq!((&out, dropped.len(), cut.len()), (&g, 0, 0));
        assert_eq!(split, [0, 1]);
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
        let (out, dropped, _, split) = drop_lot_roads_with(&g, &lots, &refuses_two);
        assert_eq!(split, [1]);
        assert_eq!(out.stretches.len(), 1);
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].stretch, "1:1-2");
    }
}

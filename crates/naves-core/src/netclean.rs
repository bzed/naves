// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The clean-up of the road graph before anything is built on it: sidepaths, stubs, close junctions and bridge
//! approaches, in this order (see `docs/superpowers/specs/2026-11-18-network-cleanup-design.md`).
//!
//! Every step is `fn(&RoadGraph, ...) -> (RoadGraph, reports)`, deterministic, and never makes the graph fall
//! apart: the number of its connected components never grows. A step that is switched off (its `..._ENABLED`
//! constant) returns the graph unchanged, bit for bit. Steps 2 (stubs and specks) and 3 (close junctions) are
//! done; the sidepath and approach steps are still empty and return the graph as it is.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::Family;
use crate::roadnet::{
    Ring, RoadGraph, Stretch, crossings_of, family_rank, line_crossings, polyline_length,
};

/// The furthest a sidepath runs from its road, in map metres.
pub const SIDEPATH_MAX_M: f64 = 15.0;
/// The shortest parallel run that counts as a sidepath without a tag that says so, in map metres.
pub const SIDEPATH_MIN_RUN_M: f64 = 30.0;
/// The longest dead-end stub that is removed, in map metres.
pub const STUB_MAX_M: f64 = 6.0;
/// A piece shorter than this is a speck, in map metres.
pub const SPECK_M: f64 = 0.5;
/// Junctions closer than this are merged into one, in map metres.
pub const JUNCTION_MERGE_M: f64 = 6.0;
/// How far from a bridge's end a junction is moved, in map metres.
pub const APPROACH_M: f64 = 8.0;

/// Whether sidepaths are removed (step 1).
pub const SIDEPATHS_ENABLED: bool = true;
/// Whether stubs are removed (step 2).
pub const STUBS_ENABLED: bool = true;
/// Whether close junctions are merged (step 3).
pub const MERGE_ENABLED: bool = true;
/// Whether bridge approaches are made (step 4).
pub const APPROACHES_ENABLED: bool = true;

/// A part of a stretch that runs beside a road and was removed.
#[derive(Debug, Clone, PartialEq)]
pub struct DroppedSidepath {
    /// The id of the stretch the part was removed from.
    pub stretch: String,
    /// The id of the road it ran beside.
    pub road: String,
    /// Where the removed part starts, in metres along the stretch.
    pub from_m: f64,
    /// Where it ends.
    pub to_m: f64,
    /// The middle of the removed part, in map metres.
    pub at: (f64, f64),
}

/// A short dead end that was removed.
#[derive(Debug, Clone, PartialEq)]
pub struct DroppedStub {
    /// The id of the stretch.
    pub stretch: String,
    /// Its length in metres.
    pub length_m: f64,
    /// Its middle, in map metres.
    pub at: (f64, f64),
}

/// A point of a line that was closer than [`SPECK_M`] to the point before and was removed.
#[derive(Debug, Clone, PartialEq)]
pub struct DroppedSpeck {
    /// The id of the stretch.
    pub stretch: String,
    /// Where the point was, in map metres.
    pub at: (f64, f64),
}

/// Close junctions that were made one.
#[derive(Debug, Clone, PartialEq)]
pub struct MergedJunction {
    /// The node that stays.
    pub kept: i64,
    /// The nodes that went into it.
    pub merged: Vec<i64>,
    /// The ids of the stretches that were removed on the way.
    pub removed: Vec<String>,
    /// Where the junction is now, in map metres.
    pub at: (f64, f64),
}

/// A cluster of close junctions that was left as it is, with the reason.
#[derive(Debug, Clone, PartialEq)]
pub struct NotMerged {
    /// The nodes of the cluster, in ascending order.
    pub nodes: Vec<i64>,
    /// Why it was not merged.
    pub reason: &'static str,
    /// The middle of the cluster, in map metres.
    pub at: (f64, f64),
}

/// A junction that was moved off the end of a bridge.
#[derive(Debug, Clone, PartialEq)]
pub struct Approach {
    /// The id of the bridge stretch.
    pub bridge: String,
    /// Which end: 0 for its start, 1 for its end.
    pub end: u8,
    /// Where the approach is, in map metres.
    pub at: (f64, f64),
}

/// What the clean-up did. Nothing is changed that is not reported.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CleanReport {
    /// Parts of stretches removed as sidepaths of a road.
    pub sidepaths: Vec<DroppedSidepath>,
    /// Dead-end stubs removed.
    pub stubs: Vec<DroppedStub>,
    /// Junctions merged.
    pub merged: Vec<MergedJunction>,
    /// Clusters of close junctions that were left as they are.
    pub not_merged: Vec<NotMerged>,
    /// Points that were closer than [`SPECK_M`] to the point before and were removed.
    pub specks: Vec<DroppedSpeck>,
    /// Bridge ends that got an approach.
    pub approaches: Vec<Approach>,
    /// Roundabouts that were turned into plain junctions (set by the caller that collapses them).
    pub roundabouts: Vec<Ring>,
    /// The connected components of the graph before the clean-up.
    pub components_before: usize,
    /// The connected components of the graph after it.
    pub components_after: usize,
    /// The connected components after each step, in the order of the steps (no step makes more).
    pub components_by_step: Vec<usize>,
    /// The length of the drivable stretches before the clean-up, in metres.
    pub drivable_m_before: f64,
    /// The length of the drivable stretches after it, in metres.
    pub drivable_m_after: f64,
}

/// The root of `i` in the union-find `parent`.
fn root(parent: &mut [usize], mut i: usize) -> usize {
    while let Some(&p) = parent.get(i) {
        if p == i {
            break;
        }
        // path halving
        let up = parent.get(p).copied().unwrap_or(p);
        if let Some(slot) = parent.get_mut(i) {
            *slot = up;
        }
        i = up;
    }
    i
}

/// The number of connected components of the graph: stretches that share a node are in one. An empty graph has
/// none.
#[must_use]
pub fn components(graph: &RoadGraph) -> usize {
    let mut index: BTreeMap<i64, usize> = BTreeMap::new();
    for s in &graph.stretches {
        for node in [s.start, s.end] {
            let next = index.len();
            index.entry(node).or_insert(next);
        }
    }
    let mut parent: Vec<usize> = (0..index.len()).collect();
    for s in &graph.stretches {
        if let (Some(&a), Some(&b)) = (index.get(&s.start), index.get(&s.end)) {
            let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
            if let Some(slot) = parent.get_mut(ra.max(rb)) {
                *slot = ra.min(rb);
            }
        }
    }
    (0..parent.len())
        .filter(|&i| root(&mut parent, i) == i)
        .count()
}

/// The result of a step that did nothing: the graph as it is and no report.
fn unchanged<T>(graph: &RoadGraph) -> (RoadGraph, Vec<T>) {
    (graph.clone(), Vec::new())
}

/// Step 1: the parts of stretches that run beside a road of higher rank and disturb it. Not done yet.
#[must_use]
pub fn drop_sidepaths(
    graph: &RoadGraph,
    widths: &dyn Fn(Family) -> f64,
    cell_m: f64,
) -> (RoadGraph, Vec<DroppedSidepath>) {
    drop_sidepaths_if(SIDEPATHS_ENABLED, graph, widths, cell_m)
}

fn drop_sidepaths_if(
    enabled: bool,
    graph: &RoadGraph,
    _widths: &dyn Fn(Family) -> f64,
    _cell_m: f64,
) -> (RoadGraph, Vec<DroppedSidepath>) {
    if !enabled {
        return unchanged(graph);
    }
    unchanged(graph)
}

/// Whether `a` is a number below `b`: false if either is not a number (a point that is not a number is never
/// taken for close to another).
fn known_less(a: f64, b: f64) -> bool {
    a.partial_cmp(&b) == Some(Ordering::Less)
}

/// The number of stretch ends at every node (a stretch that starts and ends at one node counts twice).
fn degrees(graph: &RoadGraph) -> BTreeMap<i64, usize> {
    let mut degree: BTreeMap<i64, usize> = BTreeMap::new();
    for s in &graph.stretches {
        for node in [s.start, s.end] {
            *degree.entry(node).or_default() += 1;
        }
    }
    degree
}

/// The indices of the stretches in the order of their ids (then of their place, so the order is total).
fn by_id(graph: &RoadGraph) -> Vec<usize> {
    let mut order: Vec<usize> = (0..graph.stretches.len()).collect();
    order.sort_by(|&a, &b| {
        let (sa, sb) = (graph.stretches.get(a), graph.stretches.get(b));
        sa.map(|s| &s.id).cmp(&sb.map(|s| &s.id)).then(a.cmp(&b))
    });
    order
}

/// The graph with `stretches` (put in the order of their ids) and the crossings that follow from them.
fn rebuilt(mut stretches: Vec<Stretch>) -> RoadGraph {
    stretches.sort_by(|a, b| a.id.cmp(&b.id));
    let crossings = crossings_of(&stretches);
    RoadGraph {
        stretches,
        crossings,
    }
}

/// The point half way along the line.
fn middle(points: &[(f64, f64)]) -> (f64, f64) {
    let half = polyline_length(points) / 2.0;
    let mut run = 0.0;
    for pair in points.windows(2) {
        if let [a, b] = pair {
            let len = (b.0 - a.0).hypot(b.1 - a.1);
            if len > 0.0 && run + len >= half {
                let t = (half - run) / len;
                return (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            }
            run += len;
        }
    }
    points.first().copied().unwrap_or_default()
}

/// A position in map metres.
type Point = (f64, f64);

/// The points without those closer than [`SPECK_M`] to the one before; the first and the last point stay (a line
/// of two points is kept whole). A point that is not a number is kept. Also the points that were removed.
fn without_specks(points: &[Point]) -> (Vec<Point>, Vec<Point>) {
    let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
        return (Vec::new(), Vec::new());
    };
    if points.len() < 3 {
        return (points.to_vec(), Vec::new());
    }
    let apart = |a: (f64, f64), b: (f64, f64)| !known_less((a.0 - b.0).hypot(a.1 - b.1), SPECK_M);
    let mut kept = vec![first];
    let mut removed = Vec::new();
    for &p in points.iter().skip(1).take(points.len() - 2) {
        if kept.last().is_none_or(|&q| apart(q, p)) {
            kept.push(p);
        } else {
            removed.push(p);
        }
    }
    // the last point stays where it is: a point just before it goes instead
    if kept.len() > 1
        && let Some(&q) = kept.last()
        && !apart(q, last)
    {
        kept.pop();
        removed.push(q);
    }
    kept.push(last);
    (kept, removed)
}

/// Step 2: dead ends shorter than [`STUB_MAX_M`] that are not drivable, or drivable and shorter than their own
/// width, and points closer than [`SPECK_M`] to the point before. A dead end is a stretch with one free end whose
/// other end is a node where something else is joined; a stretch that stands alone, a bridge, a tunnel and a
/// roundabout stretch are not stubs. The graph is looked at in the order of the ids, and a stub that is taken away
/// is no longer there for the next one, so two stubs that are the whole of a piece of road do not go both.
#[must_use]
pub fn drop_stubs(
    graph: &RoadGraph,
    widths: &dyn Fn(Family) -> f64,
) -> (RoadGraph, Vec<DroppedStub>) {
    let (out, stubs, _) = stubs_and_specks_if(STUBS_ENABLED, graph, widths);
    (out, stubs)
}

/// [`drop_stubs`] with the specks that were removed, and the switch as an argument.
fn stubs_and_specks_if(
    enabled: bool,
    graph: &RoadGraph,
    widths: &dyn Fn(Family) -> f64,
) -> (RoadGraph, Vec<DroppedStub>, Vec<DroppedSpeck>) {
    if !enabled {
        return (graph.clone(), Vec::new(), Vec::new());
    }
    // a dead end is decided on the graph as it was given; the count that goes down as stubs go only keeps the
    // pieces that are left of one road together
    let input = degrees(graph);
    let mut degree = input.clone();
    let mut gone: Vec<bool> = vec![false; graph.stretches.len()];
    let mut stubs = Vec::new();
    for i in by_id(graph) {
        let Some(s) = graph.stretches.get(i) else {
            continue;
        };
        let length = polyline_length(&s.points);
        let short = length < STUB_MAX_M && (!s.drivable || length < widths(s.family));
        if !short || s.bridge || s.tunnel || s.roundabout || s.start == s.end {
            continue;
        }
        let was = |n: i64| input.get(&n).copied().unwrap_or(0);
        // a free end that is a node of the map: a negative id is a road cut at the edge, which goes on outside
        let hangs_on = if was(s.start) == 1 && s.start > 0 {
            s.end
        } else if was(s.end) == 1 && s.end > 0 {
            s.start
        } else {
            continue;
        };
        if degree.get(&hangs_on).copied().unwrap_or(0) < 2 {
            continue;
        }
        if let Some(d) = degree.get_mut(&hangs_on) {
            *d -= 1;
        }
        if let Some(flag) = gone.get_mut(i) {
            *flag = true;
        }
        stubs.push(DroppedStub {
            stretch: s.id.clone(),
            length_m: length,
            at: middle(&s.points),
        });
    }
    let mut specks = Vec::new();
    let mut stretches = Vec::new();
    for (s, &dropped) in graph.stretches.iter().zip(&gone) {
        if dropped {
            continue;
        }
        let (points, removed) = without_specks(&s.points);
        specks.extend(removed.into_iter().map(|at| DroppedSpeck {
            stretch: s.id.clone(),
            at,
        }));
        stretches.push(Stretch {
            points,
            ..s.clone()
        });
    }
    if stubs.is_empty() && specks.is_empty() {
        return (graph.clone(), stubs, specks);
    }
    specks.sort_by(|a, b| {
        a.stretch
            .cmp(&b.stretch)
            .then(a.at.0.total_cmp(&b.at.0))
            .then(a.at.1.total_cmp(&b.at.1))
    });
    (rebuilt(stretches), stubs, specks)
}

/// The union-find root of `node`, the lowest node id of its set.
fn find(parent: &mut BTreeMap<i64, i64>, node: i64) -> i64 {
    let mut at = node;
    while let Some(&up) = parent.get(&at) {
        if up == at {
            break;
        }
        at = up;
    }
    at
}

/// Replaces the start of `points` by `to`, dropping the points within [`JUNCTION_MERGE_M`] of it along the line
/// (the last point is never dropped), so that the new end runs straight to the first point that is further away.
fn reaimed(points: &[(f64, f64)], to: (f64, f64)) -> Vec<(f64, f64)> {
    let mut out = vec![to];
    let mut run = 0.0;
    let last = points.len().saturating_sub(1);
    for (i, pair) in points.windows(2).enumerate() {
        if let [a, b] = pair {
            run += (b.0 - a.0).hypot(b.1 - a.1);
            if i + 1 == last || !known_less(run, JUNCTION_MERGE_M) {
                out.push(*b);
            }
        }
    }
    out
}

/// Junction nodes (three or more stretch ends) that are joined, one to the next, by short stretches.
struct Cluster {
    nodes: BTreeSet<i64>,
}

/// Step 3: junctions joined by a stretch shorter than [`JUNCTION_MERGE_M`] become one. A cluster of them is merged
/// only if all its nodes lie within twice that distance of each other, no roundabout node is among them, at most
/// one of them is the end of a bridge or a tunnel, and no longer stretch joins two of them; otherwise it is left as
/// it is and reported. The node that stays is the bridge end, or else the node whose roads rank highest (lowest id on
/// a tie); the short stretches between the nodes go, and every other stretch that ended at one of them ends at the
/// node that stays, its last [`JUNCTION_MERGE_M`] re-aimed in a straight line.
#[must_use]
pub fn merge_junctions(graph: &RoadGraph) -> (RoadGraph, Vec<MergedJunction>, Vec<NotMerged>) {
    merge_junctions_if(MERGE_ENABLED, graph)
}

/// What is known of the nodes of a graph.
struct NodeFacts {
    at: BTreeMap<i64, (f64, f64)>,
    degree: BTreeMap<i64, usize>,
    bridge_end: BTreeSet<i64>,
    /// For a node joined to a bridge or tunnel end by a stretch of its own shorter than [`JUNCTION_MERGE_M`]: those
    /// ends.
    near_deck: BTreeMap<i64, BTreeSet<i64>>,
    roundabout: BTreeSet<i64>,
}

fn node_facts(graph: &RoadGraph, order: &[usize]) -> NodeFacts {
    let mut at = BTreeMap::new();
    let mut bridge_end = BTreeSet::new();
    let mut roundabout = BTreeSet::new();
    for s in order.iter().filter_map(|&i| graph.stretches.get(i)) {
        for (node, point) in [(s.start, s.points.first()), (s.end, s.points.last())] {
            if let Some(&p) = point {
                at.entry(node).or_insert(p);
            }
            if s.bridge || s.tunnel {
                bridge_end.insert(node);
            }
            if s.roundabout {
                roundabout.insert(node);
            }
        }
    }
    let mut near_deck: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
    for s in &graph.stretches {
        if s.start == s.end
            || s.bridge
            || s.tunnel
            || !known_less(polyline_length(&s.points), JUNCTION_MERGE_M)
        {
            continue;
        }
        for (node, deck) in [(s.start, s.end), (s.end, s.start)] {
            if bridge_end.contains(&deck) {
                near_deck.entry(node).or_default().insert(deck);
            }
        }
    }
    NodeFacts {
        at,
        degree: degrees(graph),
        bridge_end,
        near_deck,
        roundabout,
    }
}

/// The cluster's nodes that are, or hang by a short stretch on, a bridge or tunnel end that is not itself in the
/// cluster: a junction there is as good as the end of the deck, and none may be moved off it.
fn deck_nodes(facts: &NodeFacts, cluster: &Cluster) -> Vec<i64> {
    cluster
        .nodes
        .iter()
        .copied()
        .filter(|n| {
            facts.bridge_end.contains(n)
                || facts
                    .near_deck
                    .get(n)
                    .is_some_and(|decks| decks.iter().any(|d| !cluster.nodes.contains(d)))
        })
        .collect()
}

/// Why a cluster is not merged, or the node that stays if it is.
fn judge(graph: &RoadGraph, facts: &NodeFacts, cluster: &Cluster) -> Result<i64, &'static str> {
    if cluster.nodes.iter().any(|n| facts.roundabout.contains(n)) {
        return Err("a roundabout is among the junctions");
    }
    let deck_nodes = deck_nodes(facts, cluster);
    if deck_nodes.len() > 1 {
        return Err(
            "two ends of bridges or tunnels, or junctions next to them, are among the junctions",
        );
    }
    let place = |n: &i64| facts.at.get(n).copied();
    let limit = 2.0 * JUNCTION_MERGE_M;
    for a in &cluster.nodes {
        for b in &cluster.nodes {
            if let (Some(p), Some(q)) = (place(a), place(b))
                && !matches!(
                    (p.0 - q.0).hypot(p.1 - q.1).partial_cmp(&limit),
                    Some(Ordering::Less | Ordering::Equal)
                )
            {
                return Err("the junctions are further apart than twice the merge distance");
            }
        }
    }
    let inside = |s: &Stretch| cluster.nodes.contains(&s.start) && cluster.nodes.contains(&s.end);
    for s in graph.stretches.iter().filter(|s| inside(s)) {
        if s.start == s.end || !known_less(polyline_length(&s.points), JUNCTION_MERGE_M) {
            return Err("a longer stretch joins junctions of the cluster");
        }
    }
    if let Some(&deck) = deck_nodes.first() {
        return Ok(deck);
    }
    // the node whose roads rank highest (the others' stretches within the cluster do not count), lowest id on a tie
    let rank = |n: i64| {
        graph
            .stretches
            .iter()
            .filter(|s| (s.start == n || s.end == n) && !inside(s))
            .map(|s| family_rank(s.family))
            .min()
            .unwrap_or(u8::MAX)
    };
    cluster
        .nodes
        .iter()
        .copied()
        .min_by_key(|&n| (rank(n), n))
        .ok_or("no junction")
}

/// The clusters of junction nodes joined, one to the next, by a stretch of their own that is shorter than
/// [`JUNCTION_MERGE_M`], by the lowest node id of each.
fn clusters_of(graph: &RoadGraph, facts: &NodeFacts) -> BTreeMap<i64, Cluster> {
    let junction = |n: i64| facts.degree.get(&n).copied().unwrap_or(0) >= 3;
    let mut parent: BTreeMap<i64, i64> = BTreeMap::new();
    for s in graph.stretches.iter().filter(|s| {
        s.start != s.end
            && !s.roundabout
            && junction(s.start)
            && junction(s.end)
            && known_less(polyline_length(&s.points), JUNCTION_MERGE_M)
    }) {
        parent.entry(s.start).or_insert(s.start);
        parent.entry(s.end).or_insert(s.end);
        let (ra, rb) = (find(&mut parent, s.start), find(&mut parent, s.end));
        if let Some(slot) = parent.get_mut(&ra.max(rb)) {
            *slot = ra.min(rb);
        }
    }
    let nodes: Vec<i64> = parent.keys().copied().collect();
    let mut clusters: BTreeMap<i64, Cluster> = BTreeMap::new();
    for node in nodes {
        let root = find(&mut parent, node);
        clusters
            .entry(root)
            .or_insert_with(|| Cluster {
                nodes: BTreeSet::new(),
            })
            .nodes
            .insert(node);
    }
    clusters
}

/// The mean of `places` (the origin for none).
fn mean_of(places: &[(f64, f64)]) -> (f64, f64) {
    let count = f64::from(u32::try_from(places.len()).unwrap_or(u32::MAX)).max(1.0);
    (
        places.iter().map(|p| p.0).sum::<f64>() / count,
        places.iter().map(|p| p.1).sum::<f64>() / count,
    )
}

/// Where the nodes of `cluster` that go (`moved`: node to the node that stays and its place) are moved to, the
/// stretch as it is after: its ends at the kept nodes, the last [`JUNCTION_MERGE_M`] at each straight, in the order of
/// its node ids.
fn moved_stretch(s: &Stretch, moved: &BTreeMap<i64, (i64, (f64, f64))>) -> Stretch {
    let mut s = s.clone();
    if let Some(&(node, at)) = moved.get(&s.start) {
        s.start = node;
        s.points = reaimed(&s.points, at);
    }
    if let Some(&(node, at)) = moved.get(&s.end) {
        s.end = node;
        let mut back: Vec<(f64, f64)> = s.points.iter().rev().copied().collect();
        back = reaimed(&back, at);
        back.reverse();
        s.points = back;
    }
    if s.start > s.end {
        std::mem::swap(&mut s.start, &mut s.end);
        s.points.reverse();
    }
    s
}

/// Whether straightening the ends of the stretches that meet the nodes in `moved` makes one of them cross a road it
/// did not cross before: the crossings with the stretches that share no node with it are counted before and after.
fn crosses_more(graph: &RoadGraph, moved: &BTreeMap<i64, (i64, (f64, f64))>) -> bool {
    let kept: BTreeSet<i64> = moved.values().map(|v| v.0).collect();
    let count = |line: &[(f64, f64)], nodes: [i64; 2], own: &str| {
        graph
            .stretches
            .iter()
            .filter(|o| {
                o.id != own
                    && ![o.start, o.end]
                        .iter()
                        .any(|n| nodes.contains(n) || kept.contains(n) || moved.contains_key(n))
            })
            .map(|o| line_crossings(line, &o.points).len())
            .sum::<usize>()
    };
    graph
        .stretches
        .iter()
        .filter(|s| moved.contains_key(&s.start) || moved.contains_key(&s.end))
        .any(|s| {
            let now = moved_stretch(s, moved);
            let nodes = [s.start, s.end];
            count(&now.points, nodes, &s.id) > count(&s.points, nodes, &s.id)
        })
}

fn merge_junctions_if(
    enabled: bool,
    graph: &RoadGraph,
) -> (RoadGraph, Vec<MergedJunction>, Vec<NotMerged>) {
    if !enabled {
        return (graph.clone(), Vec::new(), Vec::new());
    }
    let order = by_id(graph);
    let facts = node_facts(graph, &order);
    let clusters = clusters_of(graph, &facts);
    let mut merged = Vec::new();
    let mut not_merged = Vec::new();
    // node -> the node that stays and its place
    let mut moved: BTreeMap<i64, (i64, (f64, f64))> = BTreeMap::new();
    let mut removed_ids: BTreeSet<String> = BTreeSet::new();
    for cluster in clusters.values() {
        let places: Vec<(f64, f64)> = cluster
            .nodes
            .iter()
            .filter_map(|n| facts.at.get(n).copied())
            .collect();
        let verdict = judge(graph, &facts, cluster).and_then(|kept| {
            let at = *facts.at.get(&kept).ok_or("a junction has no place")?;
            let here: BTreeMap<i64, (i64, (f64, f64))> = cluster
                .nodes
                .iter()
                .filter(|&&n| n != kept)
                .map(|&n| (n, (kept, at)))
                .collect();
            if crosses_more(graph, &here) {
                return Err("a straightened road would cross another road");
            }
            Ok((kept, at, here))
        });
        match verdict {
            Err(reason) => not_merged.push(NotMerged {
                nodes: cluster.nodes.iter().copied().collect(),
                reason,
                at: mean_of(&places),
            }),
            Ok((kept, at, here)) => {
                let removed: Vec<String> = order
                    .iter()
                    .filter_map(|&i| graph.stretches.get(i))
                    .filter(|s| cluster.nodes.contains(&s.start) && cluster.nodes.contains(&s.end))
                    .map(|s| s.id.clone())
                    .collect();
                merged.push(MergedJunction {
                    kept,
                    merged: here.keys().copied().collect(),
                    removed: removed.clone(),
                    at,
                });
                removed_ids.extend(removed);
                moved.extend(here);
            }
        }
    }
    if merged.is_empty() {
        return (graph.clone(), merged, not_merged);
    }
    let stretches: Vec<Stretch> = graph
        .stretches
        .iter()
        .filter(|s| !removed_ids.contains(&s.id))
        .map(|s| moved_stretch(s, &moved))
        .collect();
    merged.sort_by_key(|m| m.kept);
    let out = rebuilt(stretches);
    // short pairs that the merge made (a straightened road got shorter): reported, not merged again, so that a
    // cluster never grows past what was measured on the input
    let after = node_facts(&out, &by_id(&out));
    for cluster in clusters_of(&out, &after).values() {
        if not_merged
            .iter()
            .any(|n| cluster.nodes.iter().all(|c| n.nodes.contains(c)))
        {
            continue;
        }
        let places: Vec<(f64, f64)> = cluster
            .nodes
            .iter()
            .filter_map(|n| after.at.get(n).copied())
            .collect();
        not_merged.push(NotMerged {
            nodes: cluster.nodes.iter().copied().collect(),
            reason: "junctions that a merge made close to each other",
            at: mean_of(&places),
        });
    }
    (out, merged, not_merged)
}

/// Step 4: a junction on the end of a bridge is moved [`APPROACH_M`] off the deck. Not done yet.
#[must_use]
pub fn make_approaches(graph: &RoadGraph) -> (RoadGraph, Vec<Approach>) {
    make_approaches_if(APPROACHES_ENABLED, graph)
}

fn make_approaches_if(enabled: bool, graph: &RoadGraph) -> (RoadGraph, Vec<Approach>) {
    if !enabled {
        return unchanged(graph);
    }
    unchanged(graph)
}

/// The length of the drivable stretches, in metres.
fn drivable_m(graph: &RoadGraph) -> f64 {
    graph
        .stretches
        .iter()
        .filter(|s| s.drivable)
        .map(|s| polyline_length(&s.points))
        .sum()
}

/// Cleans the graph: sidepaths, stubs, close junctions and bridge approaches, in this order. `widths` is the
/// road width of a family and `cell_m` the cell of the terrain, both in map metres. The report's `roundabouts`
/// are left empty; whoever collapsed the roundabouts sets them.
#[must_use]
pub fn clean(
    graph: &RoadGraph,
    widths: &dyn Fn(Family) -> f64,
    cell_m: f64,
) -> (RoadGraph, CleanReport) {
    let (graph1, sidepaths) = drop_sidepaths(graph, widths, cell_m);
    let (graph2, stubs, specks) = stubs_and_specks_if(STUBS_ENABLED, &graph1, widths);
    let (graph3, merged, not_merged) = merge_junctions(&graph2);
    let (graph4, approaches) = make_approaches(&graph3);
    let report = CleanReport {
        sidepaths,
        stubs,
        merged,
        not_merged,
        specks,
        approaches,
        roundabouts: Vec::new(),
        components_before: components(graph),
        components_after: components(&graph4),
        components_by_step: [&graph1, &graph2, &graph3, &graph4]
            .iter()
            .map(|g| components(g))
            .collect(),
        drivable_m_before: drivable_m(graph),
        drivable_m_after: drivable_m(&graph4),
    };
    (graph4, report)
}

#[cfg(test)]
mod tests {
    //! The switches of the steps are constants, so the switched-off path is tested on the private variants.
    use super::*;

    fn graph() -> RoadGraph {
        RoadGraph {
            stretches: vec![Stretch {
                id: "1:1-2".to_string(),
                family: Family::Asf3,
                drivable: true,
                bridge: false,
                tunnel: false,
                roundabout: false,
                layer: 0,
                points: vec![(0.0, 0.0), (5.0, 0.0)],
                start: 1,
                end: 2,
                bridge_kind: None,
                bridge_offset: 0.0,
                use_sidepath: false,
            }],
            crossings: Vec::new(),
        }
    }

    fn with_stub_and_close_junctions() -> RoadGraph {
        let stretch = |id: &str, family, drivable, points: Vec<(f64, f64)>, start, end| Stretch {
            id: id.to_string(),
            family,
            drivable,
            points,
            start,
            end,
            ..graph().stretches[0].clone()
        };
        let mut stretches = vec![
            stretch(
                "1:1-2",
                Family::Asf1,
                true,
                vec![(0.0, 0.0), (50.0, 0.0)],
                1,
                2,
            ),
            stretch(
                "1:2-3",
                Family::Asf1,
                true,
                vec![(50.0, 0.0), (54.0, 0.0)],
                2,
                3,
            ),
            stretch(
                "1:3-4",
                Family::Asf1,
                true,
                vec![(54.0, 0.0), (100.0, 0.0)],
                3,
                4,
            ),
            stretch(
                "2:2-5",
                Family::Grav,
                true,
                vec![(50.0, 0.0), (50.0, 40.0)],
                2,
                5,
            ),
            stretch(
                "3:3-6",
                Family::Grav,
                true,
                vec![(54.0, 0.0), (54.0, -40.0)],
                3,
                6,
            ),
            stretch(
                "4:4-7",
                Family::Mud,
                false,
                vec![(100.0, 0.0), (100.0, 3.0)],
                4,
                7,
            ),
        ];
        stretches.sort_by(|a, b| a.id.cmp(&b.id));
        let crossings = crossings_of(&stretches);
        RoadGraph {
            stretches,
            crossings,
        }
    }

    #[test]
    fn a_step_that_is_switched_off_returns_the_graph_unchanged_though_it_would_change_it() {
        let g = with_stub_and_close_junctions();
        let w = |f: Family| f.width_m();
        let (out, stubs, specks) = stubs_and_specks_if(true, &g, &w);
        assert_eq!((stubs.len(), specks.len(), out.stretches.len()), (1, 0, 5));
        let (out, stubs, specks) = stubs_and_specks_if(false, &g, &w);
        assert_eq!((&out, stubs.len(), specks.len()), (&g, 0, 0));
        let (out, merged, _) = merge_junctions_if(true, &g);
        assert_eq!((merged.len(), out.stretches.len()), (1, 5));
        let (out, merged, not_merged) = merge_junctions_if(false, &g);
        assert_eq!((&out, merged.len(), not_merged.len()), (&g, 0, 0));
    }

    #[test]
    fn the_empty_steps_that_are_left_return_the_graph_unchanged() {
        let g = graph();
        let w = |f: Family| f.width_m();
        let (out, report) = drop_sidepaths_if(false, &g, &w, 8.0);
        assert_eq!((&out, report.len()), (&g, 0));
        let (out, report) = make_approaches_if(false, &g);
        assert_eq!((&out, report.len()), (&g, 0));
    }
}

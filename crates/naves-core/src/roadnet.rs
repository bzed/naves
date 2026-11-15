// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The road graph: roads cut into stretches from crossing to crossing, and the chains of through roads.
//!
//! Ways are joined through nodes where exactly two of them meet and nothing changes (the single biggest
//! improvement of the fitting spike: a road that OSM stores as twenty ways is one line). A stretch ends at a
//! crossing, a dead end, or where the family, `drivable`, `bridge` or `tunnel` changes.
//!
//! Everything here is independent of the order of the input: stretches are stored in the order of their
//! ids, each in a canonical direction.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::f64::consts::{FRAC_PI_4, PI, TAU};

use crate::catalog::Family;
use crate::features::{Feature, FeatureKind};

/// A road from crossing to crossing.
#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent tags of the road, as in the feature they come from"
)]
pub struct Stretch {
    /// `<lowest way id>:<start node>-<end node>`: the stable part of the identity of everything placed on
    /// this stretch. It does not depend on the order of the ways.
    pub id: String,
    /// The surface.
    pub family: Family,
    /// Whether a vehicle has to be able to drive it.
    pub drivable: bool,
    /// A bridge.
    pub bridge: bool,
    /// A tunnel.
    pub tunnel: bool,
    /// Part of a roundabout (`junction=roundabout` or `circular`); see [`rings`].
    pub roundabout: bool,
    /// The `layer` tag of the way, -5 to 5: of two stretches that cross, the one with the higher layer is
    /// above (see [`over_crossings`]).
    pub layer: i8,
    /// Positions in map metres, from `start` to `end`.
    pub points: Vec<(f64, f64)>,
    /// Node id at the first point. Of the two ends, this is the one with the lower id.
    pub start: i64,
    /// Node id at the last point.
    pub end: i64,
}

/// A node where stretches meet.
#[derive(Debug, Clone, PartialEq)]
pub struct Crossing {
    /// The node id.
    pub node: i64,
    /// Its position in map metres.
    pub point: (f64, f64),
    /// The stretches that start or end here, by index into [`RoadGraph::stretches`], each once.
    pub stretches: Vec<usize>,
}

/// All stretches and the crossings between them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoadGraph {
    /// The stretches, in the order of their ids.
    pub stretches: Vec<Stretch>,
    /// The crossings, in the order of their node ids.
    pub crossings: Vec<Crossing>,
}

/// One stretch in a chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainLink {
    /// Index into [`RoadGraph::stretches`].
    pub stretch: usize,
    /// The chain runs through the stretch from its `end` to its `start`.
    pub reversed: bool,
}

/// A through road: stretches that continue each other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    /// The stretches in driving order.
    pub links: Vec<ChainLink>,
}

/// What must be equal for two segments to belong to one stretch.
#[derive(Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent tags of the road, as in the feature they come from"
)]
struct Attrs {
    family: Family,
    drivable: bool,
    bridge: bool,
    tunnel: bool,
    roundabout: bool,
    layer: i8,
}

/// The piece of a way between two consecutive nodes.
struct Seg {
    a: i64,
    b: i64,
    pa: (f64, f64),
    pb: (f64, f64),
    attrs: Attrs,
    way: i64,
}

fn segments(features: &[Feature]) -> Vec<Seg> {
    let mut segs = Vec::new();
    for feature in features {
        let FeatureKind::Road {
            family,
            drivable,
            bridge,
            tunnel,
            roundabout,
            layer,
            ..
        } = feature.kind
        else {
            continue;
        };
        let attrs = Attrs {
            family,
            drivable,
            bridge,
            tunnel,
            roundabout,
            layer,
        };
        let pairs = feature.nodes.iter().zip(&feature.points);
        for ((&a, &pa), (&b, &pb)) in pairs.clone().zip(pairs.skip(1)) {
            if a != b {
                segs.push(Seg {
                    a,
                    b,
                    pa,
                    pb,
                    attrs,
                    way: feature.way_id,
                });
            }
        }
    }
    segs
}

/// A stretch as the walk finds it, before it gets its canonical form.
struct Walked {
    nodes: Vec<i64>,
    points: Vec<(f64, f64)>,
    attrs: Attrs,
    way: i64,
    /// Every node on it is a plain joint: nothing marks where it starts.
    ring: bool,
}

/// Cuts the roads among `features` into stretches and finds their crossings. Paths are not roads.
#[must_use]
pub fn build_graph(features: &[Feature]) -> RoadGraph {
    let segs = segments(features);
    let mut incident: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for (i, s) in segs.iter().enumerate() {
        incident.entry(s.a).or_default().push(i);
        incident.entry(s.b).or_default().push(i);
    }
    // a node is a joint when exactly two segments with the same attributes meet there
    let joint = |node: i64| match incident.get(&node).map(Vec::as_slice) {
        Some(&[x, y]) => segs.get(x).map(|s| s.attrs) == segs.get(y).map(|s| s.attrs),
        _ => false,
    };
    let other = |node: i64, current: usize| {
        incident
            .get(&node)
            .and_then(|v| v.iter().copied().find(|&i| i != current))
    };
    let mut visited = vec![false; segs.len()];
    let mut walked = Vec::new();
    // stretches that start at a break first, then the rings that have none
    for pass in 0..2 {
        for s in 0..segs.len() {
            let Some(seg) = segs.get(s) else { continue };
            let starts_at_break = !joint(seg.a) || !joint(seg.b);
            if visited.get(s).copied().unwrap_or(true) || (pass == 0 && !starts_at_break) {
                continue;
            }
            walked.push(walk(s, &segs, &mut visited, &joint, &other));
        }
    }
    let mut stretches: Vec<Stretch> = walked.into_iter().filter_map(canonical).collect();
    stretches.sort_by(|a, b| {
        a.id.cmp(&b.id).then_with(|| {
            let key = |s: &Stretch| s.points.first().copied().unwrap_or_default();
            key(a)
                .0
                .total_cmp(&key(b).0)
                .then(key(a).1.total_cmp(&key(b).1))
        })
    });
    // the same id twice (two loops from one way at one node): number them
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for s in &mut stretches {
        let n = seen.entry(s.id.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            s.id = format!("{}#{n}", s.id);
        }
    }
    let crossings = crossings(&stretches);
    RoadGraph {
        stretches,
        crossings,
    }
}

/// The stretch through segment `s`, extended in both directions up to the next break.
fn walk(
    s: usize,
    segs: &[Seg],
    visited: &mut [bool],
    joint: &impl Fn(i64) -> bool,
    other: &impl Fn(i64, usize) -> Option<usize>,
) -> Walked {
    if let Some(v) = visited.get_mut(s) {
        *v = true;
    }
    // (segment, walked from a to b)
    let mut seq: VecDeque<(usize, bool)> = VecDeque::from([(s, true)]);
    let mut ring = false;
    let first = segs.get(s);
    let mut node = first.map_or(0, |x| x.b);
    let mut current = s;
    while joint(node) {
        let Some(next) = other(node, current) else {
            break;
        };
        if next == s {
            ring = true;
            break;
        }
        if visited.get(next).copied().unwrap_or(true) {
            break;
        }
        if let Some(v) = visited.get_mut(next) {
            *v = true;
        }
        let Some(n) = segs.get(next) else { break };
        let forward = n.a == node;
        seq.push_back((next, forward));
        node = if forward { n.b } else { n.a };
        current = next;
    }
    if !ring {
        node = first.map_or(0, |x| x.a);
        current = s;
        while joint(node) {
            let Some(next) = other(node, current) else {
                break;
            };
            if visited.get(next).copied().unwrap_or(true) {
                break;
            }
            if let Some(v) = visited.get_mut(next) {
                *v = true;
            }
            let Some(n) = segs.get(next) else { break };
            // walked so that it ends at `node`
            let forward = n.b == node;
            seq.push_front((next, forward));
            node = if forward { n.a } else { n.b };
            current = next;
        }
    }
    let mut nodes = Vec::new();
    let mut points = Vec::new();
    let mut way = i64::MAX;
    let attrs = first.map(|x| x.attrs);
    for &(i, forward) in &seq {
        let Some(seg) = segs.get(i) else { continue };
        way = way.min(seg.way);
        let (na, pa, nb, pb) = if forward {
            (seg.a, seg.pa, seg.b, seg.pb)
        } else {
            (seg.b, seg.pb, seg.a, seg.pa)
        };
        if nodes.is_empty() {
            nodes.push(na);
            points.push(pa);
        }
        nodes.push(nb);
        points.push(pb);
    }
    Walked {
        nodes,
        points,
        attrs: attrs.unwrap_or(Attrs {
            family: Family::Asf3,
            drivable: true,
            bridge: false,
            tunnel: false,
            roundabout: false,
            layer: 0,
        }),
        way,
        ring,
    }
}

/// The stretch in its canonical direction and, for a ring, starting at its lowest node.
fn canonical(mut w: Walked) -> Option<Stretch> {
    if w.nodes.len() < 2 {
        return None;
    }
    if w.ring && w.nodes.len() >= 3 {
        // rotate the closed loop so that it starts at the lowest node
        w.nodes.pop();
        w.points.pop();
        let at = w
            .nodes
            .iter()
            .enumerate()
            .min_by_key(|&(_, n)| *n)
            .map_or(0, |(i, _)| i);
        w.nodes.rotate_left(at);
        w.points.rotate_left(at);
        w.nodes.extend(w.nodes.first().copied());
        w.points.extend(w.points.first().copied());
    }
    let (start, end) = (*w.nodes.first()?, *w.nodes.last()?);
    let n = w.nodes.len();
    let turn = if start == end {
        // a loop: leave towards the lower neighbour
        n >= 3 && w.nodes.get(1) > w.nodes.get(n - 2)
    } else {
        start > end
    };
    if turn {
        w.nodes.reverse();
        w.points.reverse();
    }
    let (start, end) = (*w.nodes.first()?, *w.nodes.last()?);
    Some(Stretch {
        id: format!("{}:{start}-{end}", w.way),
        family: w.attrs.family,
        drivable: w.attrs.drivable,
        bridge: w.attrs.bridge,
        tunnel: w.attrs.tunnel,
        roundabout: w.attrs.roundabout,
        layer: w.attrs.layer,
        points: w.points,
        start,
        end,
    })
}

/// A node's position and the stretches that end there.
type NodeEntry = (Option<(f64, f64)>, BTreeSet<usize>);

fn crossings(stretches: &[Stretch]) -> Vec<Crossing> {
    let mut at: BTreeMap<i64, NodeEntry> = BTreeMap::new();
    for (i, s) in stretches.iter().enumerate() {
        for (node, point) in [
            (s.start, s.points.first().copied()),
            (s.end, s.points.last().copied()),
        ] {
            let entry = at.entry(node).or_default();
            entry.0 = entry.0.or(point);
            entry.1.insert(i);
        }
    }
    at.into_iter()
        .filter(|(_, (_, set))| set.len() >= 2)
        .map(|(node, (point, set))| Crossing {
            node,
            point: point.unwrap_or_default(),
            stretches: set.into_iter().collect(),
        })
        .collect()
}

/// Two stretches whose lines cross without a shared node, the first on a bridge over the second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverCrossing {
    /// The bridge, an index into [`RoadGraph::stretches`].
    pub upper: usize,
    /// The road below it.
    pub lower: usize,
    /// Where their lines cross, in map metres.
    pub at: (f64, f64),
    /// The distance of `at` from the start of the upper stretch along its line.
    pub upper_s: f64,
    /// The distance of `at` from the start of the lower stretch along its line.
    pub lower_s: f64,
}

/// Stretches that cross without a shared node and are not at one height: `upper` is a bridge and `lower` is no
/// tunnel and no bridge of a higher or equal layer. Untagged crossings are not here (nothing says which is
/// above), nor are crossings of stretches that share a node (they join, see [`RoadGraph::crossings`]). Sorted
/// by the two stretches and the place.
#[must_use]
pub fn over_crossings(graph: &RoadGraph) -> Vec<OverCrossing> {
    let boxes: Vec<((f64, f64), (f64, f64))> =
        graph.stretches.iter().map(|s| bounds(&s.points)).collect();
    let mut out = Vec::new();
    for (i, a) in graph.stretches.iter().enumerate() {
        for (j, b) in graph.stretches.iter().enumerate().skip(i + 1) {
            let upper_is_a = match (a.bridge, b.bridge) {
                (true, false) => true,
                (false, true) => false,
                (true, true) if a.layer != b.layer => a.layer > b.layer,
                _ => continue,
            };
            let (upper, lower) = if upper_is_a { (a, b) } else { (b, a) };
            let shared = [upper.start, upper.end]
                .iter()
                .any(|n| *n == lower.start || *n == lower.end);
            let apart = boxes.get(i).zip(boxes.get(j)).is_some_and(|(p, q)| {
                p.1.0 < q.0.0 || q.1.0 < p.0.0 || p.1.1 < q.0.1 || q.1.1 < p.0.1
            });
            if shared || apart || upper.tunnel || lower.tunnel {
                continue;
            }
            let (u, l) = if upper_is_a { (i, j) } else { (j, i) };
            out.extend(
                line_crossings(&upper.points, &lower.points)
                    .into_iter()
                    .map(|(at, upper_s, lower_s)| OverCrossing {
                        upper: u,
                        lower: l,
                        at,
                        upper_s,
                        lower_s,
                    }),
            );
        }
    }
    out
}

/// Where two polylines cross: the point and the distance along each.
fn line_crossings(first: &[(f64, f64)], second: &[(f64, f64)]) -> Vec<((f64, f64), f64, f64)> {
    let mut found = Vec::new();
    let mut along_first = 0.0;
    for pair in first.windows(2) {
        let [from, to] = pair else { continue };
        let dir = (to.0 - from.0, to.1 - from.1);
        let mut along_second = 0.0;
        for other in second.windows(2) {
            let [o_from, o_to] = other else { continue };
            let o_dir = (o_to.0 - o_from.0, o_to.1 - o_from.1);
            let denom = dir.0 * o_dir.1 - dir.1 * o_dir.0;
            if denom.abs() > 1e-12 {
                let gap = (o_from.0 - from.0, o_from.1 - from.1);
                let on_first = (gap.0 * o_dir.1 - gap.1 * o_dir.0) / denom;
                let on_second = (gap.0 * dir.1 - gap.1 * dir.0) / denom;
                if (0.0..=1.0).contains(&on_first) && (0.0..=1.0).contains(&on_second) {
                    found.push((
                        (from.0 + on_first * dir.0, from.1 + on_first * dir.1),
                        along_first + on_first * dir.0.hypot(dir.1),
                        along_second + on_second * o_dir.0.hypot(o_dir.1),
                    ));
                }
            }
            along_second += o_dir.0.hypot(o_dir.1);
        }
        along_first += dir.0.hypot(dir.1);
    }
    found
}

/// A stretch end at a crossing.
type Arm = (usize, bool);

/// Where a stretch leaves a crossing, as a compass heading in radians clockwise from north: towards the
/// first point at least 4 m away, or the last point.
fn arm_heading(stretch: &Stretch, at_start: bool, from: (f64, f64)) -> f64 {
    let along: Vec<(f64, f64)> = if at_start {
        stretch.points.clone()
    } else {
        stretch.points.iter().rev().copied().collect()
    };
    let target = along
        .iter()
        .copied()
        .find(|p| (p.0 - from.0).hypot(p.1 - from.1) >= 4.0)
        .or_else(|| along.last().copied())
        .unwrap_or(from);
    (target.0 - from.0).atan2(target.1 - from.1)
}

/// The angle in [0, π] between two directions.
fn angle_between(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(TAU);
    if d > PI { TAU - d } else { d }
}

/// How far from straight a road turns that arrives along arm `a` and leaves along arm `b`.
fn deviation(a: f64, b: f64) -> f64 {
    PI - angle_between(a, b)
}

/// At each crossing, the two arms that continue each other (see [`chains`]), both ways round.
fn through_pairs(graph: &RoadGraph, rings: &[Ring]) -> BTreeMap<Arm, Arm> {
    let mut partner: BTreeMap<Arm, Arm> = BTreeMap::new();
    let on_ring: BTreeSet<usize> = rings
        .iter()
        .flat_map(|r| r.stretches.iter().copied())
        .collect();
    // the arms at each node, in the order of the stretches, a stretch's start before its end
    let mut arms_at: BTreeMap<i64, Vec<Arm>> = BTreeMap::new();
    for (i, s) in graph.stretches.iter().enumerate() {
        arms_at.entry(s.start).or_default().push((i, true));
        arms_at.entry(s.end).or_default().push((i, false));
    }
    for crossing in &graph.crossings {
        let arms: Vec<Arm> = arms_at.get(&crossing.node).cloned().unwrap_or_default();
        let heading = |arm: Arm| {
            graph
                .stretches
                .get(arm.0)
                .map_or(0.0, |s| arm_heading(s, arm.1, crossing.point))
        };
        // the two arms of a ring are the ring, whatever else leaves here
        if let [a, b] = arms
            .iter()
            .copied()
            .filter(|arm| on_ring.contains(&arm.0))
            .collect::<Vec<Arm>>()
            .as_slice()
        {
            partner.insert(*a, *b);
            partner.insert(*b, *a);
            continue;
        }
        let family = |arm: Arm| graph.stretches.get(arm.0).map(|s| s.family);
        let mut best: Option<(f64, Arm, Arm)> = None;
        for (i, &a) in arms.iter().enumerate() {
            for &b in arms.iter().skip(i + 1) {
                if family(a) != family(b) {
                    continue;
                }
                let d = deviation(heading(a), heading(b));
                if d < FRAC_PI_4 && best.is_none_or(|(bd, _, _)| d < bd) {
                    best = Some((d, a, b));
                }
            }
        }
        if let Some((_, a, b)) = best {
            partner.insert(a, b);
            partner.insert(b, a);
        }
    }
    partner
}

/// A roundabout: stretches tagged `junction=roundabout` (or `circular`) that form one closed loop.
#[derive(Debug, Clone, PartialEq)]
pub struct Ring {
    /// The stretches of the loop, by index into [`RoadGraph::stretches`], in ascending order.
    pub stretches: Vec<usize>,
    /// The middle of the loop, in map metres: the mean of its points, each weighted by the road length
    /// beside it.
    pub centre: (f64, f64),
    /// The mean distance of the loop's centre line from `centre`, in metres.
    pub radius: f64,
    /// The surface of the loop.
    pub family: Family,
}

/// The roundabouts of the graph: the closed loops of `roundabout` stretches. A loop is a set of stretches that
/// meet each other only two at a node (so that no third roundabout way leaves it), all of one family and
/// drivability and none a bridge or a tunnel. An open arc, a loop with a way that is not tagged as a
/// roundabout in it, or one with a branch is not a ring: its stretches stay ordinary roads. The rings are
/// ordered by their lowest stretch.
#[must_use]
pub fn rings(graph: &RoadGraph) -> Vec<Ring> {
    let candidates: Vec<usize> = graph
        .stretches
        .iter()
        .enumerate()
        .filter(|(_, s)| s.roundabout && !s.bridge && !s.tunnel)
        .map(|(i, _)| i)
        .collect();
    // candidate arms at each node: a loop stretch has two at its one node
    let mut at: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for &i in &candidates {
        if let Some(s) = graph.stretches.get(i) {
            at.entry(s.start).or_default().push(i);
            at.entry(s.end).or_default().push(i);
        }
    }
    let mut seen: BTreeSet<usize> = BTreeSet::new();
    let mut found = Vec::new();
    for &first in &candidates {
        if seen.contains(&first) {
            continue;
        }
        let mut component: BTreeSet<usize> = BTreeSet::from([first]);
        let mut todo = vec![first];
        while let Some(i) = todo.pop() {
            let Some(s) = graph.stretches.get(i) else {
                continue;
            };
            for node in [s.start, s.end] {
                for &j in at.get(&node).map(Vec::as_slice).unwrap_or_default() {
                    if component.insert(j) {
                        todo.push(j);
                    }
                }
            }
        }
        seen.extend(component.iter().copied());
        let members: Vec<&Stretch> = component
            .iter()
            .filter_map(|i| graph.stretches.get(*i))
            .collect();
        let simple = members.iter().all(|s| {
            [s.start, s.end]
                .iter()
                .all(|n| at.get(n).is_some_and(|v| v.len() == 2))
        });
        let uniform = members.windows(2).all(|w| match w {
            [a, b] => a.family == b.family && a.drivable == b.drivable,
            _ => true,
        });
        let Some(family) = members.first().map(|s| s.family) else {
            continue;
        };
        if !simple || !uniform {
            continue;
        }
        let (centre, radius) = circle_of(&members);
        found.push(Ring {
            stretches: component.into_iter().collect(),
            centre,
            radius,
            family,
        });
    }
    found
}

/// The graph with every ring of a radius below `tightest(family)` (its family's tightest arc; no arc: any ring)
/// replaced by a plain crossing: the ring's stretches go, and the roads that joined it end at one node at the
/// ring's centre, where the usual rules for roads that meet (a junction piece or the overlay rule) apply. A road
/// that joined the ring at two of its nodes is a chord of nothing and goes with it. A stretch whose end is
/// moved keeps its id; it is turned round if its ends are no longer in the order of their node ids.
#[must_use]
pub fn collapse_small_rings(
    graph: &RoadGraph,
    tightest: &dyn Fn(Family) -> Option<f64>,
) -> RoadGraph {
    let small: Vec<Ring> = rings(graph)
        .into_iter()
        .filter(|r| tightest(r.family).is_none_or(|t| r.radius < t))
        .collect();
    if small.is_empty() {
        return graph.clone();
    }
    let gone: BTreeSet<usize> = small
        .iter()
        .flat_map(|r| r.stretches.iter().copied())
        .collect();
    // ring node -> (the node of the crossing, its place)
    let mut merged: BTreeMap<i64, (i64, (f64, f64))> = BTreeMap::new();
    for ring in &small {
        let nodes: BTreeSet<i64> = ring
            .stretches
            .iter()
            .filter_map(|i| graph.stretches.get(*i))
            .flat_map(|s| [s.start, s.end])
            .collect();
        if let Some(&first) = nodes.first() {
            for node in nodes {
                merged.insert(node, (first, ring.centre));
            }
        }
    }
    let mut stretches: Vec<Stretch> = Vec::new();
    for (i, s) in graph.stretches.iter().enumerate() {
        if gone.contains(&i) {
            continue;
        }
        let (at_start, at_end) = (merged.get(&s.start), merged.get(&s.end));
        if let (Some(a), Some(b)) = (at_start, at_end)
            && a.0 == b.0
        {
            continue;
        }
        let mut s = s.clone();
        if let Some(&(node, centre)) = at_start {
            s.start = node;
            if let Some(p) = s.points.first_mut() {
                *p = centre;
            }
        }
        if let Some(&(node, centre)) = at_end {
            s.end = node;
            if let Some(p) = s.points.last_mut() {
                *p = centre;
            }
        }
        if s.start > s.end {
            std::mem::swap(&mut s.start, &mut s.end);
            s.points.reverse();
        }
        stretches.push(s);
    }
    let crossings = crossings(&stretches);
    RoadGraph {
        stretches,
        crossings,
    }
}

/// The centre and mean radius of the closed line of `members`, every point weighted by half the length of the
/// segments on both sides of it.
fn circle_of(members: &[&Stretch]) -> ((f64, f64), f64) {
    let mut weighted: Vec<((f64, f64), f64)> = Vec::new();
    for s in members {
        for pair in s.points.windows(2) {
            if let [a, b] = pair {
                let half = (b.0 - a.0).hypot(b.1 - a.1) / 2.0;
                weighted.push((*a, half));
                weighted.push((*b, half));
            }
        }
    }
    let total: f64 = weighted.iter().map(|(_, w)| w).sum();
    if total <= 0.0 {
        return ((0.0, 0.0), 0.0);
    }
    let centre = (
        weighted.iter().map(|(p, w)| p.0 * w).sum::<f64>() / total,
        weighted.iter().map(|(p, w)| p.1 * w).sum::<f64>() / total,
    );
    let radius = weighted
        .iter()
        .map(|(p, w)| (p.0 - centre.0).hypot(p.1 - centre.1) * w)
        .sum::<f64>()
        / total;
    (centre, radius)
}

/// The placement rank of a family: lower is more important.
fn family_rank(f: Family) -> u8 {
    match f {
        Family::Asf1 => 0,
        Family::Asf2 => 1,
        Family::City => 2,
        Family::Asf3 => 3,
        Family::Grav => 4,
        Family::Mud => 5,
    }
}

/// The length of a chain along its stretches.
fn chain_length(graph: &RoadGraph, c: &Chain) -> f64 {
    c.links
        .iter()
        .filter_map(|l| graph.stretches.get(l.stretch))
        .map(|s| polyline_length(&s.points))
        .sum()
}

/// Which of two chains lies on top where they meet without a junction piece: `Less` if `a` does. The family
/// rank decides (Asf1, Asf2, City, Asf3, Grav, Mud), then the longer chain, then the lower stretch id, so
/// that the answer does not depend on the order of the arguments' roads.
#[must_use]
pub fn overlay_rank(a: &Chain, b: &Chain, graph: &RoadGraph) -> std::cmp::Ordering {
    let key = |c: &Chain| {
        let first = c.links.first().and_then(|l| graph.stretches.get(l.stretch));
        let id = c
            .links
            .iter()
            .filter_map(|l| graph.stretches.get(l.stretch))
            .map(|s| s.id.as_str())
            .min()
            .unwrap_or_default();
        (first.map_or(0, |s| family_rank(s.family)), id)
    };
    let (ka, kb) = (key(a), key(b));
    ka.0.cmp(&kb.0)
        .then_with(|| chain_length(graph, b).total_cmp(&chain_length(graph, a)))
        .then_with(|| ka.1.cmp(kb.1))
}

/// The chains of through roads, in placement order: by family rank (Asf1, Asf2, City, Asf3, Grav, Mud),
/// then longest first, then by id.
///
/// At each crossing the two arms of the same family whose way through turns least, if less than 45°, are one
/// through road. The other arms start or end there.
#[must_use]
pub fn chains(graph: &RoadGraph) -> Vec<Chain> {
    chains_with(graph, &rings(graph))
}

/// [`chains`] with `rings` taken as the roundabouts: at every node of a ring its two ring arms are one
/// through road, whatever their angle, so that a ring is one closed chain.
#[must_use]
pub fn chains_with(graph: &RoadGraph, rings: &[Ring]) -> Vec<Chain> {
    let partner = through_pairs(graph, rings);
    let mut visited = vec![false; graph.stretches.len()];
    let mut found: Vec<Chain> = Vec::new();
    for s in 0..graph.stretches.len() {
        if visited.get(s).copied().unwrap_or(true) {
            continue;
        }
        // go back to the head: enter stretch `head.0` at its start (`head.1`) or its end
        let mut head: Arm = (s, true);
        for _ in 0..=graph.stretches.len() {
            let Some(&(t, t_arm)) = partner.get(&head) else {
                break;
            };
            let before = (t, !t_arm);
            if before == (s, true) {
                // the loop is closed: start where we began
                break;
            }
            head = before;
        }
        let mut links = Vec::new();
        let mut enter = head;
        loop {
            if visited.get(enter.0).copied().unwrap_or(true) {
                break;
            }
            if let Some(v) = visited.get_mut(enter.0) {
                *v = true;
            }
            links.push(ChainLink {
                stretch: enter.0,
                reversed: !enter.1,
            });
            match partner.get(&(enter.0, !enter.1)) {
                Some(&next) => enter = next,
                None => break,
            }
        }
        found.push(Chain { links });
    }
    let length = |c: &Chain| chain_length(graph, c);
    let key = |c: &Chain| {
        let first = c.links.first().and_then(|l| graph.stretches.get(l.stretch));
        (
            first.map_or(0, |s| family_rank(s.family)),
            first.map_or(String::new(), |s| s.id.clone()),
        )
    };
    found.sort_by(|a, b| {
        key(a)
            .0
            .cmp(&key(b).0)
            .then_with(|| length(b).total_cmp(&length(a)))
            .then_with(|| key(a).1.cmp(&key(b).1))
    });
    found
}

/// A part of a stretch that is to be a bridge, in metres along the stretch from its start.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeSpan {
    /// Index into [`RoadGraph::stretches`].
    pub stretch: usize,
    /// Where the bridge starts.
    pub from_m: f64,
    /// Where the bridge ends.
    pub to_m: f64,
}

/// A span shorter than this is dropped, a remainder shorter than this is not a piece.
const EPS_M: f64 = 1e-6;
/// A span stops this far short of a junction or a dead end.
const CLEARANCE_M: f64 = 1.0;
/// A neighbour that gives room to a bridge keeps at least this much.
const KEEP_M: f64 = 1.0;

/// A polyline.
type Points = Vec<(f64, f64)>;

pub(crate) fn polyline_length(points: &[(f64, f64)]) -> f64 {
    points
        .windows(2)
        .map(|p| match p {
            [a, b] => (b.0 - a.0).hypot(b.1 - a.1),
            _ => 0.0,
        })
        .sum()
}

/// The part of `points` between the distances `from` and `to` along it, with interpolated ends.
pub(crate) fn slice_points(points: &[(f64, f64)], from: f64, to: f64) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::new();
    let mut at = 0.0;
    for w in points.windows(2) {
        let &[a, b] = w else { continue };
        let seg = (b.0 - a.0).hypot(b.1 - a.1);
        let (s0, s1) = (at, at + seg);
        at = s1;
        if s1 < from || s0 > to || seg <= 0.0 {
            continue;
        }
        let lerp = |d: f64| {
            let f = ((d - s0) / seg).clamp(0.0, 1.0);
            (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)
        };
        let mut candidates = Vec::with_capacity(2);
        if out.is_empty() {
            candidates.push(lerp(from));
        }
        if to <= s1 {
            candidates.push(lerp(to));
        } else {
            candidates.push(b);
        }
        for p in candidates {
            if out.last() != Some(&p) {
                out.push(p);
            }
        }
        if to <= s1 {
            break;
        }
    }
    out
}

/// A node id above all the ids in `graph`.
fn first_free_node(graph: &RoadGraph) -> i64 {
    graph
        .stretches
        .iter()
        .flat_map(|s| [s.start, s.end])
        .max()
        .map_or(1, |m| m.saturating_add(1))
}

/// The part of stretch `i`, in metres along it, that a bridge may cover: a stretch end that continues into
/// exactly one other stretch may be reached, a crossing of three or more stretches or a dead end is kept
/// 1 m clear. `None` for a stretch that is not in the graph.
#[must_use]
pub fn bridge_room(graph: &RoadGraph, i: usize) -> Option<(f64, f64)> {
    let s = graph.stretches.get(i)?;
    let through = |node: i64| {
        graph
            .crossings
            .iter()
            .any(|c| c.node == node && c.stretches.len() == 2 && c.stretches.contains(&i))
    };
    let lo = if through(s.start) { 0.0 } else { CLEARANCE_M };
    let hi = polyline_length(&s.points) - if through(s.end) { 0.0 } else { CLEARANCE_M };
    Some((lo, hi))
}

/// The graph with the spans cut out of their stretches as bridge stretches. The part before the first
/// span keeps the stretch's id; a span is `<id>/bridge<n>` and the road after it `<id>/after<n>`, `n`
/// counted from the stretch's start. A span that reaches a crossing with three or more stretches, or the
/// end of a dead end, is shortened to stop 1 m before it. Overlapping spans are merged. Spans on bridges
/// and tunnels are ignored.
///
/// The new ends get node ids above all others, in the order of the stretches and along each stretch, so
/// the result does not depend on anything else. A cut piece is therefore not always in the canonical
/// direction (lower node id first) that [`build_graph`] gives its stretches; it keeps the direction of
/// the stretch it was cut from.
#[must_use]
pub fn with_bridge_spans(graph: &RoadGraph, spans: &[BridgeSpan]) -> RoadGraph {
    let mut next_node = first_free_node(graph);
    let mut stretches: Vec<Stretch> = Vec::new();
    for (i, s) in graph.stretches.iter().enumerate() {
        let total = polyline_length(&s.points);
        let (lo, hi) = bridge_room(graph, i).unwrap_or((0.0, total));
        let mut mine: Vec<(f64, f64)> = spans
            .iter()
            .filter(|sp| sp.stretch == i && sp.from_m.is_finite() && sp.to_m.is_finite())
            .map(|sp| (sp.from_m.max(lo), sp.to_m.min(hi)))
            .filter(|(f, t)| t - f > EPS_M)
            .collect();
        if s.bridge || s.tunnel || mine.is_empty() {
            stretches.push(s.clone());
            continue;
        }
        mine.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for (f, t) in mine {
            match merged.last_mut() {
                Some(last) if f <= last.1 + EPS_M => last.1 = last.1.max(t),
                _ => merged.push((f, t)),
            }
        }
        let mut node = s.start;
        let mut cursor = 0.0;
        let mut piece = |from: f64, to: f64, id: String, bridge: bool, node: &mut i64| {
            let end = if to >= total - EPS_M {
                s.end
            } else {
                let n = next_node;
                next_node = next_node.saturating_add(1);
                n
            };
            stretches.push(Stretch {
                id,
                bridge,
                points: slice_points(&s.points, from, to),
                start: *node,
                end,
                ..s.clone()
            });
            *node = end;
        };
        for (k, &(f, t)) in merged.iter().enumerate() {
            if f - cursor > EPS_M {
                let id = if k == 0 {
                    s.id.clone()
                } else {
                    format!("{}/after{}", s.id, k - 1)
                };
                piece(cursor, f, id, false, &mut node);
            }
            piece(f, t, format!("{}/bridge{k}", s.id), true, &mut node);
            cursor = t;
        }
        if total - cursor > EPS_M {
            let id = format!("{}/after{}", s.id, merged.len() - 1);
            piece(cursor, total, id, false, &mut node);
        }
    }
    stretches.sort_by(|a, b| a.id.cmp(&b.id));
    let crossings = crossings(&stretches);
    RoadGraph {
        stretches,
        crossings,
    }
}

/// The ids of the bridge stretches of `before` that are no bridge in `after`: a bridge stretch of `after`
/// with the same id, or one cut from it by [`with_bridge_spans`] (`<id>/bridge<n>`), keeps it a bridge.
#[must_use]
pub fn lost_bridges<'a>(before: &'a RoadGraph, after: &RoadGraph) -> Vec<&'a str> {
    let kept = |id: &str| {
        let cut = format!("{id}/bridge");
        after
            .stretches
            .iter()
            .any(|s| s.bridge && (s.id == id || s.id.starts_with(&cut)))
    };
    before
        .stretches
        .iter()
        .filter(|s| s.bridge && !kept(&s.id))
        .map(|s| s.id.as_str())
        .collect()
}

/// The stretch that continues stretch `i` at its start or end: the only other one at a crossing of exactly
/// two stretches, if it is a plain road (not a bridge, a tunnel or a loop).
fn continuation(graph: &RoadGraph, i: usize, at_start: bool) -> Option<usize> {
    let this = graph.stretches.get(i)?;
    let node = if at_start { this.start } else { this.end };
    let crossing = graph.crossings.iter().find(|c| c.node == node)?;
    let &[first, second] = crossing.stretches.as_slice() else {
        return None;
    };
    let other = if first == i { second } else { first };
    let next = graph.stretches.get(other)?;
    (other != i && !next.bridge && !next.tunnel && next.start != next.end).then_some(other)
}

/// Splits `points` at distance `d` from its start into the part up to there and the part from there.
fn split_points(points: &[(f64, f64)], d: f64) -> (Points, Points) {
    let total = polyline_length(points);
    (
        slice_points(points, 0.0, d),
        slice_points(points, d, total.max(d)),
    )
}

/// Moves `t` metres of stretch `n`, from the crossing at its `bridge_at_start` side of bridge `i`, over to
/// the bridge.
fn grow(graph: &mut RoadGraph, i: usize, bridge_at_start: bool, n: usize, t: f64) {
    if t <= EPS_M {
        return;
    }
    let Some(bridge) = graph.stretches.get(i) else {
        return;
    };
    let node = if bridge_at_start {
        bridge.start
    } else {
        bridge.end
    };
    let Some(neighbour) = graph.stretches.get(n) else {
        return;
    };
    let from_node = neighbour.start == node;
    let mut away = neighbour.points.clone();
    if !from_node {
        away.reverse();
    }
    let (mut head, mut rest) = split_points(&away, t);
    if !from_node {
        rest.reverse();
    }
    if let Some(s) = graph.stretches.get_mut(n) {
        s.points = rest;
    }
    if let Some(b) = graph.stretches.get_mut(i) {
        if bridge_at_start {
            head.reverse();
            head.extend(b.points.iter().skip(1).copied());
            b.points = head;
        } else {
            b.points.pop();
            b.points.extend(head);
        }
    }
}

/// For each bridge stretch shorter than `needed(stretch)`, the same stretch grown evenly at both ends into
/// the stretches that continue it (crossings of exactly two stretches), as far as they allow: if one side
/// allows less than half of what is missing the other takes the rest. A neighbour keeps at least 1 m. The
/// nodes stay where they are in the graph, only their points move, so no id changes. A bridge that cannot
/// reach `needed` that way is no bridge in the result (`bridge` is false) and its neighbours are as they
/// were.
#[must_use]
pub fn widen_bridges(graph: &RoadGraph, needed: &dyn Fn(&Stretch) -> f64) -> RoadGraph {
    let mut g = graph.clone();
    for i in 0..g.stretches.len() {
        let Some(s) = g.stretches.get(i) else {
            continue;
        };
        if !s.bridge || s.tunnel {
            continue;
        }
        let want = needed(s);
        let missing = want - polyline_length(&s.points);
        if !want.is_finite() || missing <= EPS_M {
            continue;
        }
        let before = continuation(&g, i, true);
        let after = continuation(&g, i, false).filter(|&a| Some(a) != before);
        let room = |n: Option<usize>| {
            n.and_then(|n| g.stretches.get(n))
                .map_or(0.0, |n| (polyline_length(&n.points) - KEEP_M).max(0.0))
        };
        let (room_before, room_after) = (room(before), room(after));
        if room_before + room_after + EPS_M < missing {
            if let Some(s) = g.stretches.get_mut(i) {
                s.bridge = false;
            }
            continue;
        }
        let take_before = (missing / 2.0).min(room_before);
        let take_after = (missing - take_before).min(room_after);
        let take_before = (missing - take_after).min(room_before);
        if let Some(n) = before {
            grow(&mut g, i, true, n, take_before);
        }
        if let Some(n) = after {
            grow(&mut g, i, false, n, take_after);
        }
    }
    g.crossings = crossings(&g.stretches);
    g
}

/// Two roads along each other for at least this long are one road drawn twice.
pub const DUPLICATE_MIN_M: f64 = 25.0;
/// Two roads whose headings differ by more than this (in degrees, a road and its reverse count as equal)
/// are no duplicates.
pub const DUPLICATE_MAX_HEADING_DEG: f64 = 15.0;
/// The distance between the points where a stretch is compared with the roads that rank higher.
const DUPLICATE_STEP_M: f64 = 1.0;
/// A leaving point closer than this to another on the same stretch is the same node.
const LEAVE_MERGE_M: f64 = 0.01;

/// A part of a road that is dropped because a higher-ranked road runs along it.
#[derive(Debug, Clone, PartialEq)]
pub struct DroppedDuplicate {
    /// The id of the stretch the part was dropped from.
    pub stretch: String,
    /// The id of the stretch that stays.
    pub kept: String,
    /// Where the dropped part starts, in metres along the original stretch.
    pub from_m: f64,
    /// Where it ends.
    pub to_m: f64,
    /// The middle of the dropped part, in map metres.
    pub at: (f64, f64),
}

/// A part of a stretch that stays, which the lower-ranked stretches are compared with.
struct KeptPiece {
    stretch: usize,
    chain: usize,
    /// Metres along the original stretch where this piece starts.
    offset: f64,
    points: Vec<(f64, f64)>,
    /// The width of its family.
    width: f64,
    /// Bridge and tunnel: only like is compared with like.
    class: (bool, bool),
    min: (f64, f64),
    max: (f64, f64),
}

/// Where a point of a lower road lies on a higher one.
#[derive(Clone, Copy)]
struct Hit {
    stretch: usize,
    /// Metres along the original stretch.
    along: f64,
    at: (f64, f64),
}

/// A part of a stretch to be dropped.
struct Run {
    from: f64,
    to: f64,
    first: Hit,
    last: Hit,
}

/// The node where a lower road leaves a higher one.
struct Leave {
    stretch: usize,
    along: f64,
    node: i64,
    at: (f64, f64),
    /// The higher stretch has to be cut here (not at one of its ends).
    split: bool,
}

/// What happens to a stretch when it is rebuilt.
enum Mark {
    Split {
        along: f64,
        node: i64,
    },
    Drop {
        from: f64,
        to: f64,
        before: Option<(i64, (f64, f64))>,
        after: Option<(i64, (f64, f64))>,
    },
}

impl Mark {
    fn start(&self) -> f64 {
        match self {
            Mark::Split { along, .. } => *along,
            Mark::Drop { from, .. } => *from,
        }
    }
}

fn bounds(points: &[(f64, f64)]) -> ((f64, f64), (f64, f64)) {
    let mut min = (f64::INFINITY, f64::INFINITY);
    let mut max = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in points {
        min = (min.0.min(p.0), min.1.min(p.1));
        max = (max.0.max(p.0), max.1.max(p.1));
    }
    (min, max)
}

/// A point of a stretch that is compared with the higher roads.
struct Sample {
    /// Metres along the stretch.
    along: f64,
    at: (f64, f64),
    /// The unit direction of the segment it lies on.
    dir: (f64, f64),
}

/// The points at every `DUPLICATE_STEP_M` along `points` and at its end.
fn samples(points: &[(f64, f64)]) -> Vec<Sample> {
    let mut out = Vec::new();
    let total = polyline_length(points);
    let mut at = 0.0;
    let mut next = 0.0;
    for w in points.windows(2) {
        let &[a, b] = w else { continue };
        let seg = (b.0 - a.0).hypot(b.1 - a.1);
        if seg <= 0.0 {
            continue;
        }
        let dir = ((b.0 - a.0) / seg, (b.1 - a.1) / seg);
        while next < at + seg {
            let f = (next - at) / seg;
            out.push(Sample {
                along: next,
                at: (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f),
                dir,
            });
            next += DUPLICATE_STEP_M;
        }
        at += seg;
        if (at - total).abs() < EPS_M && out.last().is_some_and(|last| at - last.along > EPS_M) {
            out.push(Sample {
                along: at,
                at: b,
                dir,
            });
        }
    }
    out
}

/// The nearest point of the kept pieces within half the narrower width, on a segment whose direction is
/// within [`DUPLICATE_MAX_HEADING_DEG`] of `dir`.
fn nearest_hit(at: (f64, f64), dir: (f64, f64), width: f64, pieces: &[&KeptPiece]) -> Option<Hit> {
    let cos_limit = DUPLICATE_MAX_HEADING_DEG.to_radians().cos();
    let mut best: Option<(f64, Hit)> = None;
    for piece in pieces {
        let half = width.min(piece.width) / 2.0;
        if at.0 < piece.min.0 - half
            || at.0 > piece.max.0 + half
            || at.1 < piece.min.1 - half
            || at.1 > piece.max.1 + half
        {
            continue;
        }
        let mut along = 0.0;
        for pair in piece.points.windows(2) {
            let &[from, to] = pair else { continue };
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            let len = dx.hypot(dy);
            if len <= 0.0 {
                continue;
            }
            let frac =
                (((at.0 - from.0) * dx + (at.1 - from.1) * dy) / (len * len)).clamp(0.0, 1.0);
            let foot = (from.0 + dx * frac, from.1 + dy * frac);
            let dist = (at.0 - foot.0).hypot(at.1 - foot.1);
            let aligned = (dir.0 * dx + dir.1 * dy).abs() / len >= cos_limit;
            if dist < half && aligned && best.is_none_or(|(nearest, _)| dist < nearest) {
                best = Some((
                    dist,
                    Hit {
                        stretch: piece.stretch,
                        along: piece.offset + along + len * frac,
                        at: foot,
                    },
                ));
            }
            along += len;
        }
    }
    best.map(|(_, hit)| hit)
}

/// The point `d` metres along `points`.
fn point_along(points: &[(f64, f64)], d: f64) -> (f64, f64) {
    slice_points(points, d, d)
        .first()
        .copied()
        .or_else(|| points.first().copied())
        .unwrap_or_default()
}

/// The runs of consecutive samples that have a hit and are at least [`DUPLICATE_MIN_M`] long.
fn duplicate_runs(samples: &[Sample], hits: &[Option<Hit>]) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut start: Option<usize> = None;
    for k in 0..=samples.len() {
        let hit = hits.get(k).copied().flatten();
        match (start, hit) {
            (None, Some(_)) => start = Some(k),
            (Some(a), None) => {
                start = None;
                let b = k - 1;
                let (Some(first), Some(last)) = (
                    hits.get(a).copied().flatten(),
                    hits.get(b).copied().flatten(),
                ) else {
                    continue;
                };
                let (Some(from), Some(to)) = (samples.get(a), samples.get(b)) else {
                    continue;
                };
                if to.along - from.along >= DUPLICATE_MIN_M {
                    runs.push(Run {
                        from: from.along,
                        to: to.along,
                        first,
                        last,
                    });
                }
            }
            _ => {}
        }
    }
    runs
}

/// Drops the parts of lower-ranked roads that run along a higher-ranked road: one road drawn twice in the
/// source. Two stretches of different chains are one road when they run for at least [`DUPLICATE_MIN_M`]
/// closer than half the narrower road's width and within [`DUPLICATE_MAX_HEADING_DEG`] of each other; the
/// one that [`overlay_rank`] puts lower loses that part (equal family and length: the higher stretch id). A
/// part is compared with what stays of the higher roads, in order of rank.
///
/// The rest of a road that goes on after the dropped part ends on the kept road: the kept stretch is cut
/// there and the rest ends at the new crossing, so that the overlay rule of the placement meets them. Cut
/// pieces are named (`<id>`, then `<id>/rest<n>`, so they cannot collide with the
/// `/after<n>` pieces of [`with_bridge_spans`]) and the new nodes get ids above all others. Bridges are compared
/// only with bridges and tunnels only with tunnels; such a stretch is never cut: where it runs along a kept one
/// for [`DUPLICATE_MIN_M`] it is dropped whole. The dropped parts come sorted by stretch id and
/// position.
#[must_use]
pub fn drop_duplicates(graph: &RoadGraph) -> (RoadGraph, Vec<DroppedDuplicate>) {
    let (runs_of, mut dropped) = find_duplicates(graph);
    if dropped.is_empty() {
        return (graph.clone(), dropped);
    }
    dropped.sort_by(|x, y| {
        x.stretch
            .cmp(&y.stretch)
            .then(x.from_m.total_cmp(&y.from_m))
    });
    (cut_runs(graph, &runs_of), dropped)
}

/// A bridge or tunnel is never cut: if any part of it is to be dropped, all of it is.
fn whole(runs: &[Run], total: f64) -> Vec<Run> {
    match (runs.first(), runs.last()) {
        (Some(first), Some(last)) => vec![Run {
            from: 0.0,
            to: total,
            first: first.first,
            last: last.last,
        }],
        _ => Vec::new(),
    }
}

/// The parts to drop, by stretch, and their report, in order of rank.
fn find_duplicates(graph: &RoadGraph) -> (BTreeMap<usize, Vec<Run>>, Vec<DroppedDuplicate>) {
    let all = chains(graph);
    let mut chain_of = vec![0_usize; graph.stretches.len()];
    for (ci, c) in all.iter().enumerate() {
        for l in &c.links {
            if let Some(slot) = chain_of.get_mut(l.stretch) {
                *slot = ci;
            }
        }
    }
    let chain_index = |i: usize| chain_of.get(i).copied().unwrap_or(0);
    let mut order: Vec<usize> = (0..graph.stretches.len())
        .filter(|&i| graph.stretches.get(i).is_some_and(|s| s.points.len() >= 2))
        .collect();
    order.sort_by(|&x, &y| {
        match (all.get(chain_index(x)), all.get(chain_index(y))) {
            (Some(cx), Some(cy)) => overlay_rank(cx, cy, graph),
            _ => std::cmp::Ordering::Equal,
        }
        .then(x.cmp(&y))
    });
    let mut pieces: Vec<KeptPiece> = Vec::new();
    let mut runs_of: BTreeMap<usize, Vec<Run>> = BTreeMap::new();
    let mut dropped: Vec<DroppedDuplicate> = Vec::new();
    for &i in &order {
        let Some(s) = graph.stretches.get(i) else {
            continue;
        };
        let width = s.family.width_m();
        let (min, max) = bounds(&s.points);
        let here = chain_index(i);
        let class = (s.bridge, s.tunnel);
        let near: Vec<&KeptPiece> = pieces
            .iter()
            .filter(|k| {
                let reach = width.max(k.width) / 2.0;
                k.chain != here
                    && k.class == class
                    && k.min.0 <= max.0 + reach
                    && k.max.0 >= min.0 - reach
                    && k.min.1 <= max.1 + reach
                    && k.max.1 >= min.1 - reach
            })
            .collect();
        let sampled = samples(&s.points);
        let hits: Vec<Option<Hit>> = if near.is_empty() {
            Vec::new()
        } else {
            sampled
                .iter()
                .map(|sample| nearest_hit(sample.at, sample.dir, width, &near))
                .collect()
        };
        let mut runs = duplicate_runs(&sampled, &hits);
        let total = polyline_length(&s.points);
        if class != (false, false) {
            runs = whole(&runs, total);
        }
        let mut cursor = 0.0;
        let retain = |from: f64, to: f64, pieces: &mut Vec<KeptPiece>| {
            if to - from > EPS_M {
                let points = slice_points(&s.points, from, to);
                let (min, max) = bounds(&points);
                pieces.push(KeptPiece {
                    stretch: i,
                    chain: here,
                    offset: from,
                    points,
                    width,
                    class,
                    min,
                    max,
                });
            }
        };
        for run in &runs {
            retain(cursor, run.from, &mut pieces);
            cursor = run.to;
            let mid = f64::midpoint(run.from, run.to);
            let kept_id = sampled
                .iter()
                .position(|sample| sample.along >= mid)
                .and_then(|k| hits.get(k).copied().flatten())
                .and_then(|h| graph.stretches.get(h.stretch))
                .map(|k| k.id.clone())
                .unwrap_or_default();
            dropped.push(DroppedDuplicate {
                stretch: s.id.clone(),
                kept: kept_id,
                from_m: run.from,
                to_m: run.to,
                at: point_along(&s.points, mid),
            });
        }
        retain(cursor, total, &mut pieces);
        if !runs.is_empty() {
            runs_of.insert(i, runs);
        }
    }
    (runs_of, dropped)
}

/// The graph with the runs cut out of their stretches. The rest of a road ends on the road that stays, which
/// is cut there.
fn cut_runs(graph: &RoadGraph, runs_of: &BTreeMap<usize, Vec<Run>>) -> RoadGraph {
    let mut next_node = first_free_node(graph);
    // where the rest of a road meets the road that stays
    let mut leaves: Vec<Leave> = Vec::new();
    let mut leave_at = |hit: Hit, leaves: &mut Vec<Leave>| -> (i64, (f64, f64)) {
        if let Some(l) = leaves
            .iter()
            .find(|l| l.stretch == hit.stretch && (l.along - hit.along).abs() < LEAVE_MERGE_M)
        {
            return (l.node, l.at);
        }
        let Some(kept) = graph.stretches.get(hit.stretch) else {
            return (0, hit.at);
        };
        let length = polyline_length(&kept.points);
        let (node, split) = if hit.along <= EPS_M {
            (kept.start, false)
        } else if hit.along >= length - EPS_M {
            (kept.end, false)
        } else {
            let n = next_node;
            next_node = next_node.saturating_add(1);
            (n, true)
        };
        leaves.push(Leave {
            stretch: hit.stretch,
            along: hit.along,
            node,
            at: hit.at,
            split,
        });
        (node, hit.at)
    };
    let mut marks: BTreeMap<usize, Vec<Mark>> = BTreeMap::new();
    for (&i, runs) in runs_of {
        let Some(s) = graph.stretches.get(i) else {
            continue;
        };
        let total = polyline_length(&s.points);
        for run in runs {
            let before = (run.from > EPS_M).then(|| leave_at(run.first, &mut leaves));
            let after = (run.to < total - EPS_M).then(|| leave_at(run.last, &mut leaves));
            marks.entry(i).or_default().push(Mark::Drop {
                from: run.from,
                to: run.to,
                before,
                after,
            });
        }
    }
    for l in leaves.iter().filter(|l| l.split) {
        marks.entry(l.stretch).or_default().push(Mark::Split {
            along: l.along,
            node: l.node,
        });
    }
    let mut stretches: Vec<Stretch> = Vec::new();
    for (i, s) in graph.stretches.iter().enumerate() {
        let Some(mut list) = marks.remove(&i) else {
            stretches.push(s.clone());
            continue;
        };
        list.sort_by(|x, y| x.start().total_cmp(&y.start()));
        rebuild_stretch(s, &list, &mut stretches);
    }
    stretches.sort_by(|x, y| x.id.cmp(&y.id));
    let crossings = crossings(&stretches);
    RoadGraph {
        stretches,
        crossings,
    }
}

/// A point in map metres.
type Point = (f64, f64);

/// Pushes the pieces that are left of `s` after the marks (sorted along it) are applied.
fn rebuild_stretch(s: &Stretch, list: &[Mark], out: &mut Vec<Stretch>) {
    let total = polyline_length(&s.points);
    let mut cursor = 0.0;
    let mut node = s.start;
    let mut head: Option<Point> = None;
    let mut part = 0_usize;
    let mut emit = |from: f64,
                    to: f64,
                    ends: (i64, i64),
                    joins: (Option<Point>, Option<Point>),
                    part: usize| {
        if to - from <= EPS_M {
            return;
        }
        let mut points = slice_points(&s.points, from, to);
        if let Some(first) = joins.0 {
            points.insert(0, first);
        }
        if let Some(last) = joins.1 {
            points.push(last);
        }
        let id = if part == 0 {
            s.id.clone()
        } else {
            format!("{}/rest{}", s.id, part - 1)
        };
        out.push(Stretch {
            id,
            points,
            start: ends.0,
            end: ends.1,
            ..s.clone()
        });
    };
    for mark in list {
        match *mark {
            Mark::Split { along, node: n } => {
                emit(cursor, along, (node, n), (head, None), part);
                cursor = along;
                node = n;
                head = None;
            }
            Mark::Drop {
                from,
                to,
                before,
                after,
            } => {
                let (end, tail) = before.map_or((node, None), |(n, at)| (n, Some(at)));
                emit(cursor, from, (node, end), (head, tail), part);
                cursor = to;
                (node, head) = after.map_or((s.end, None), |(n, at)| (n, Some(at)));
            }
        }
        part += 1;
    }
    emit(cursor, total, (node, s.end), (head, None), part);
}

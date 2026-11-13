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
struct Attrs {
    family: Family,
    drivable: bool,
    bridge: bool,
    tunnel: bool,
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
        } = feature.kind
        else {
            continue;
        };
        let attrs = Attrs {
            family,
            drivable,
            bridge,
            tunnel,
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
fn through_pairs(graph: &RoadGraph) -> BTreeMap<Arm, Arm> {
    let mut partner: BTreeMap<Arm, Arm> = BTreeMap::new();
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

/// The chains of through roads, in placement order: by family rank (Asf1, Asf2, City, Asf3, Grav, Mud),
/// then longest first, then by id.
///
/// At each crossing the two arms of the same family whose way through turns least, if less than 45°, are one
/// through road. The other arms start or end there.
#[must_use]
pub fn chains(graph: &RoadGraph) -> Vec<Chain> {
    let partner = through_pairs(graph);
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
    let rank = |f: Family| match f {
        Family::Asf1 => 0,
        Family::Asf2 => 1,
        Family::City => 2,
        Family::Asf3 => 3,
        Family::Grav => 4,
        Family::Mud => 5,
    };
    let length = |c: &Chain| -> f64 {
        c.links
            .iter()
            .filter_map(|l| graph.stretches.get(l.stretch))
            .map(|s| {
                s.points
                    .windows(2)
                    .map(|p| match p {
                        [a, b] => (b.0 - a.0).hypot(b.1 - a.1),
                        _ => 0.0,
                    })
                    .sum::<f64>()
            })
            .sum()
    };
    let key = |c: &Chain| {
        let first = c.links.first().and_then(|l| graph.stretches.get(l.stretch));
        (
            first.map_or(0, |s| rank(s.family)),
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

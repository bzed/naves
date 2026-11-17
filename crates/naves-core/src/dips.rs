// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dips: where a planned road stands high above the original ground.

use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{PI, TAU};

use crate::heightmap::{Heightmap, MapGeo};
use crate::osm::LandPolygon;
use crate::profile::Profiles;
use crate::roadnet::{BridgeSpan, RoadGraph, bridge_room, polyline_length, slice_points};
use crate::roadplace::{PlacedKind, PlacedNetwork};
use crate::validate::{WATER_PRIORITY, in_water, outer_bounds};

/// A road piece further than this from the line of its stretch is not under that line (a junction piece
/// between two stretches is not one of either).
const MAX_OFF_LINE_M: f64 = 8.0;
/// The road is read at least this often along a stretch, and at least once per cell of the heightmap.
const MAX_STEP_M: f64 = 4.0;
/// A step is never finer than this.
const MIN_STEP_M: f64 = 0.5;
/// A bridge ends this much short of what is needed, in metres, before one more model is added.
const SLACK_M: f64 = 1e-9;

/// A road this far above the original ground is crossing a dip.
pub const DIP_M: f64 = 3.0;
/// A dip shallower than this never gets a bridge: it is filled when dry and reported when a water line
/// crosses it. [`DIP_M`] is where a run starts to count as a dip at all.
pub const BRIDGE_DIP_M: f64 = 8.0;
/// The road may leave the straight line of a bridge model by this much.
pub const BRIDGE_STRAIGHT_M: f64 = 2.0;

/// At each end of a dip bridge span the line over 4 m inside the span runs within this of the chord of the model
/// at that end. Outside the span the stricter [`SPAN_APPROACH_DEG`] holds over [`SPAN_APPROACH_M`].
pub const SPAN_END_STRAIGHT_DEG: f64 = 20.0;
/// The line within this many metres outside a span end is measured: at least 4 m of it
/// must exist.
pub const SPAN_APPROACH_M: f64 = 12.5;
/// Every point of the line within [`SPAN_APPROACH_M`] outside a span end is seen from the end within this
/// of the chord of the model at that end.
pub const SPAN_APPROACH_DEG: f64 = 10.0;
/// Less line than this outside a span end fails the end rule.
const SPAN_MEASURE_MIN_M: f64 = 4.0;
/// A span that fails [`SPAN_END_STRAIGHT_DEG`] is moved along the run in steps of this.
const SPAN_SHIFT_STEP_M: f64 = 0.5;

/// A run of a stretch where the road stands more than [`DIP_M`] above the original ground.
#[derive(Debug, Clone, PartialEq)]
pub struct Dip {
    /// Index into `RoadGraph::stretches`.
    pub stretch: usize,
    /// Where the run starts, in metres along the stretch.
    pub from_m: f64,
    /// Where the run ends, in metres along the stretch.
    pub to_m: f64,
    /// The most the road stands above the ground in the run.
    pub depth_m: f64,
    /// Water lies under the run.
    pub over_water: bool,
    /// Middle of the run, in map metres.
    pub at: (f64, f64),
}

/// What is done about a dip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DipAction {
    /// A bridge or a chain of bridges.
    Bridge,
    /// The ground is filled up to the road.
    Fill,
    /// Left as it is and reported.
    Report,
}

/// A road piece of a stretch with the planned height at both of its ends.
struct Line {
    a: (f64, f64),
    b: (f64, f64),
    ha: f64,
    hb: f64,
}

/// The planned road under a point: the point on the road nearest to it, its distance and the planned height.
fn road_under(lines: &[Line], p: (f64, f64)) -> Option<(f64, (f64, f64), f64)> {
    let mut best: Option<(f64, (f64, f64), f64)> = None;
    for line in lines {
        let (dx, dy) = (line.b.0 - line.a.0, line.b.1 - line.a.1);
        let length2 = dx * dx + dy * dy;
        let t = if length2 > 0.0 {
            (((p.0 - line.a.0) * dx + (p.1 - line.a.1) * dy) / length2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let q = (line.a.0 + t * dx, line.a.1 + t * dy);
        let d = (p.0 - q.0).hypot(p.1 - q.1);
        if best.is_none_or(|(b, _, _)| d < b) {
            best = Some((d, q, line.ha + (line.hb - line.ha) * t));
        }
    }
    best
}

/// The pieces of every stretch with their planned heights, by stretch id, and the stretches that have a
/// bridge model placed on them.
fn planned_roads<'a>(
    net: &'a PlacedNetwork,
    profiles: &Profiles,
) -> (BTreeMap<&'a str, Vec<Line>>, BTreeSet<&'a str>) {
    let mut lines: BTreeMap<&str, Vec<Line>> = BTreeMap::new();
    for strip in &profiles.strips {
        for (position, &road) in strip.slots.iter().enumerate() {
            let (Some(placed), Some(from), Some(to)) = (
                net.roads.get(road),
                strip.joints.get(position),
                strip.joints.get(position + 1),
            ) else {
                continue;
            };
            if placed.kind == PlacedKind::Piece {
                lines
                    .entry(placed.slot.stretch.as_str())
                    .or_default()
                    .push(Line {
                        a: (from.0, from.1),
                        b: (to.0, to.1),
                        ha: from.2,
                        hb: to.2,
                    });
            }
        }
    }
    let bridged = net
        .roads
        .iter()
        .filter(|r| r.kind == PlacedKind::Bridge)
        .map(|r| r.slot.stretch.as_str())
        .collect();
    (lines, bridged)
}

/// Points every `step` metres along `points`, with the distance from the start, the last at the end.
fn samples(points: &[(f64, f64)], step: f64) -> Vec<(f64, (f64, f64))> {
    let total = polyline_length(points);
    let mut out = Vec::new();
    let mut segment = 0;
    let mut before = 0.0;
    let mut s = 0.0_f64;
    loop {
        // the segment that holds distance `s`
        while let [a, b, ..] = points.get(segment..).unwrap_or_default() {
            let length = (b.0 - a.0).hypot(b.1 - a.1);
            if before + length >= s || points.get(segment + 2).is_none() {
                break;
            }
            before += length;
            segment += 1;
        }
        let at = match points.get(segment..).unwrap_or_default() {
            [a, b, ..] => {
                let length = (b.0 - a.0).hypot(b.1 - a.1);
                let f = if length > 0.0 {
                    ((s - before) / length).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)
            }
            [a] => *a,
            [] => break,
        };
        out.push((s, at));
        if s >= total {
            break;
        }
        s = (s + step).min(total);
    }
    out
}

/// Whether the segments `a`-`b` and `c`-`d` meet.
fn segments_meet(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    let side = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| {
        (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
    };
    let (d1, d2) = (side(a, b, c), side(a, b, d));
    let (d3, d4) = (side(c, d, a), side(c, d, b));
    d1 * d2 <= 0.0 && d3 * d4 <= 0.0
}

/// Whether the polylines cross.
fn lines_cross(road: &[(f64, f64)], water: &[(f64, f64)]) -> bool {
    let bounds = |line: &[(f64, f64)]| {
        line.iter().fold(
            (f64::MAX, f64::MAX, f64::MIN, f64::MIN),
            |(x0, y0, x1, y1), p| (x0.min(p.0), y0.min(p.1), x1.max(p.0), y1.max(p.1)),
        )
    };
    let (a, b) = (bounds(road), bounds(water));
    // a water line whose box does not touch the run's box cannot cross it
    if a.0 > b.2 || b.0 > a.2 || a.1 > b.3 || b.1 > a.3 {
        return false;
    }
    road.windows(2).any(|r| {
        water.windows(2).any(|w| match (r, w) {
            ([a, b], [c, d]) => segments_meet(*a, *b, *c, *d),
            _ => false,
        })
    })
}

/// The dips of a placed network: where the planned road stands more than [`DIP_M`] above the original
/// ground `base`.
///
/// The road is read along each stretch of `graph` every cell of the heightmap (at most 4 m) from the
/// pieces and heights of `profiles`; a point with no piece within 8 m of it, or with a height that is not a
/// number, ends a run and is never part of one. A stretch that is a tunnel, or a bridge with a bridge model
/// placed on it, has no dips; a bridge stretch that got no model is road. `depth_m` is the most the road
/// stands above the ground in the run.
///
/// A run is over water when one of the `water` lines (streams, rivers, canals, in map metres) crosses the
/// centre line of the stretch inside the run, or its middle lies in one of the `lakes` (only areas of the
/// priority of water count). A line beside the road does not make it wet.
///
/// The dips are in the order of the stretches, then along each.
#[must_use]
pub fn find_dips(
    graph: &RoadGraph,
    net: &PlacedNetwork,
    profiles: &Profiles,
    base: &Heightmap,
    water: &[Vec<(f64, f64)>],
    lakes: &[LandPolygon],
    frame: &MapGeo,
) -> Vec<Dip> {
    let (roads, bridged) = planned_roads(net, profiles);
    let step = f64::from(base.cell_m).clamp(MIN_STEP_M, MAX_STEP_M);
    let lakes: Vec<_> = lakes
        .iter()
        .filter(|p| p.priority == WATER_PRIORITY)
        .map(|p| (outer_bounds(p), p))
        .collect();
    let mut out = Vec::new();
    for (index, stretch) in graph.stretches.iter().enumerate() {
        if stretch.tunnel || (stretch.bridge && bridged.contains(stretch.id.as_str())) {
            continue;
        }
        let Some(lines) = roads.get(stretch.id.as_str()) else {
            continue;
        };
        let total = polyline_length(&stretch.points);
        let excess: Vec<(f64, f64)> = samples(&stretch.points, step)
            .into_iter()
            .map(|(s, p)| {
                let over = road_under(lines, p)
                    .filter(|(d, _, _)| *d <= MAX_OFF_LINE_M)
                    .map_or(f64::NAN, |(_, q, h)| {
                        h - f64::from(base.sample(q.0, q.1).height)
                    });
                (s, over)
            })
            .collect();
        let mut run: Vec<(f64, f64)> = Vec::new();
        for k in 0..=excess.len() {
            match excess.get(k) {
                Some(&(s, over)) if over.is_finite() && over > DIP_M => run.push((s, over)),
                _ => {
                    if let (Some(first), Some(last)) = (run.first(), run.last()) {
                        let from_m = (first.0 - step / 2.0).max(0.0);
                        let to_m = (last.0 + step / 2.0).min(total);
                        let depth_m = run.iter().map(|r| r.1).fold(f64::MIN, f64::max);
                        let middle = f64::midpoint(from_m, to_m);
                        let at = point_at(&stretch.points, middle);
                        let part = slice_points(&stretch.points, from_m, to_m);
                        let over_water = water.iter().any(|w| lines_cross(&part, w))
                            || frame
                                .to_geo(at.0, at.1)
                                .is_ok_and(|(lat, lon)| in_water(&lakes, lat, lon));
                        out.push(Dip {
                            stretch: index,
                            from_m,
                            to_m,
                            depth_m,
                            over_water,
                            at,
                        });
                    }
                    run.clear();
                }
            }
        }
    }
    out
}

/// The point `d` metres along `points`.
fn point_at(points: &[(f64, f64)], d: f64) -> (f64, f64) {
    slice_points(points, d, d)
        .first()
        .or_else(|| points.first())
        .copied()
        .unwrap_or_default()
}

/// The distance of `p` from the segment `a`-`b`.
fn off_chord(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length2 = dx * dx + dy * dy;
    let t = if length2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
}

/// The heading in radians of the line from `from` to the first of `along` at least 4 m away, or the last.
fn heading_towards(along: impl Iterator<Item = (f64, f64)>, from: (f64, f64)) -> Option<f64> {
    let mut last = None;
    for p in along {
        last = Some(p);
        if (p.0 - from.0).hypot(p.1 - from.1) >= 4.0 {
            break;
        }
    }
    let p = last?;
    let (dx, dy) = (p.0 - from.0, p.1 - from.1);
    (dx.hypot(dy) > 0.0).then(|| dx.atan2(dy))
}

/// The angle in degrees between two headings in radians.
fn heading_gap_deg(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(TAU);
    d.min(TAU - d).to_degrees()
}

/// Whether the `count` models of `length` from `from_m` each stay within [`BRIDGE_STRAIGHT_M`] of their
/// chord.
fn models_straight(points: &[(f64, f64)], from_m: f64, length: f64, count: f64) -> bool {
    let mut k = 0.0;
    while k < count {
        let part = slice_points(points, from_m + k * length, from_m + (k + 1.0) * length);
        let (Some(first), Some(last)) = (part.first(), part.last()) else {
            return false;
        };
        if part
            .iter()
            .any(|p| off_chord(*p, *first, *last) > BRIDGE_STRAIGHT_M)
        {
            return false;
        }
        k += 1.0;
    }
    true
}

/// Whether the line leaves both ends of the span along the chords of the models there: see
/// [`SPAN_APPROACH_M`] and [`SPAN_END_STRAIGHT_DEG`].
fn ends_straight(points: &[(f64, f64)], from_m: f64, length: f64, count: f64) -> bool {
    let first_model = (from_m, from_m + length);
    let last_model = (from_m + (count - 1.0) * length, from_m + count * length);
    end_runs_along_chord(points, first_model, true)
        && end_runs_along_chord(points, last_model, false)
}

/// Whether the line runs along the chord of the model `(from, to)` at its start (`at_start`) or end: every
/// point within [`SPAN_APPROACH_M`] outside, seen from the end, within [`SPAN_APPROACH_DEG`] of the chord
/// and the line over 4 m inside within [`SPAN_END_STRAIGHT_DEG`].
fn end_runs_along_chord(points: &[(f64, f64)], (from, to): (f64, f64), at_start: bool) -> bool {
    let part = slice_points(points, from, to);
    let (Some(first), Some(last)) = (part.first().copied(), part.last().copied()) else {
        return false;
    };
    let chord = (last.0 - first.0).atan2(last.1 - first.1);
    let (at, d) = if at_start { (first, from) } else { (last, to) };
    let total = polyline_length(points);
    let (outside, inside) = if at_start {
        (
            slice_points(points, (d - SPAN_APPROACH_M).max(0.0), d),
            slice_points(points, d, total),
        )
    } else {
        (
            slice_points(points, d, (d + SPAN_APPROACH_M).min(total)),
            slice_points(points, 0.0, d),
        )
    };
    let reach = if at_start { d } else { total - d };
    if reach < SPAN_MEASURE_MIN_M {
        return false;
    }
    // the direction of travel between a point outside and the end
    let outside_ok = outside
        .iter()
        .filter(|p| (p.0 - at.0).hypot(p.1 - at.1) > 1e-9)
        .all(|p| {
            let h = if at_start {
                (at.0 - p.0).atan2(at.1 - p.1)
            } else {
                (p.0 - at.0).atan2(p.1 - at.1)
            };
            heading_gap_deg(h, chord) <= SPAN_APPROACH_DEG
        });
    let inside_h = if at_start {
        heading_towards(inside.iter().skip(1).copied(), at)
    } else {
        heading_towards(inside.iter().rev().skip(1).copied(), at).map(|h| h + PI)
    };
    outside_ok && inside_h.is_none_or(|h| heading_gap_deg(h, chord) <= SPAN_END_STRAIGHT_DEG)
}

/// The line of stretch `i` with the stretch before it and the one after it, where the stretch ends at a
/// through node (a node of exactly two stretches), and the length of the part added before it.
fn run_line(graph: &RoadGraph, i: usize) -> Option<(Vec<(f64, f64)>, f64)> {
    let stretch = graph.stretches.get(i)?;
    let other = |node: i64| {
        let crossing = graph
            .crossings
            .iter()
            .find(|c| c.node == node && c.stretches.len() == 2 && c.stretches.contains(&i))?;
        let j = *crossing.stretches.iter().find(|j| **j != i)?;
        graph.stretches.get(j)
    };
    let mut points = Vec::new();
    let mut offset = 0.0;
    if let Some(before) = other(stretch.start) {
        let mut line = before.points.clone();
        if before.start == stretch.start {
            line.reverse();
        }
        offset = polyline_length(&line);
        points = line;
    }
    points.extend(stretch.points.iter().copied());
    if let Some(after) = other(stretch.end) {
        let mut line = after.points.clone();
        if after.end == stretch.end {
            line.reverse();
        }
        points.extend(line);
    }
    points.dedup();
    Some((points, offset))
}

/// What to do about `dip`; `model_length` is the length of the bridge model the road would get, if
/// any.
///
/// A bridge when the dip is at least [`BRIDGE_DIP_M`] deep, a model exists, the run is at least half a
/// model long, the span (the run grown to a whole number of models, evenly at both ends) stays within the room [`bridge_room`] gives and, over every model
/// length of it, the stretch's points are within [`BRIDGE_STRAIGHT_M`] of the chord; the span is then the
/// second value. Otherwise a fill when no water lies under the run, else a report.
#[must_use]
pub fn decide(
    dip: &Dip,
    graph: &RoadGraph,
    model_length: Option<f64>,
) -> (DipAction, Option<BridgeSpan>) {
    let otherwise = if dip.over_water {
        DipAction::Report
    } else {
        DipAction::Fill
    };
    let span = (|| {
        if !dip.depth_m.is_finite() || dip.depth_m < BRIDGE_DIP_M {
            return None;
        }
        let length =
            model_length.filter(|l| l.is_finite() && *l >= crate::roadplace::MIN_BRIDGE_M)?;
        let (line, offset) = run_line(graph, dip.stretch)?;
        let run = dip.to_m - dip.from_m;
        if !run.is_finite() || run * 2.0 + SLACK_M < length {
            return None;
        }
        let count = (run / length - SLACK_M).ceil().max(1.0);
        let middle = f64::midpoint(dip.from_m, dip.to_m);
        let (lo, hi) = bridge_room(graph, dip.stretch)?;
        // no room for the centred span, or it is not straight over every model: no bridge, and
        // shifting does not help
        if middle - count * length / 2.0 < lo - SLACK_M
            || middle + count * length / 2.0 > hi + SLACK_M
            || !models_straight(&line, offset + middle - count * length / 2.0, length, count)
        {
            return None;
        }
        // Where the centred span leaves its ends in a bend it may move along the run as far as it still
        // covers the run, in steps of `SPAN_SHIFT_STEP_M`: the shorter shift first, the earlier position
        // on a tie. Every candidate passes both tests.
        let slack = (count * length - run) / 2.0;
        let mut steps = 0.0_f64;
        loop {
            let shift = steps * SPAN_SHIFT_STEP_M;
            if shift > slack + SLACK_M {
                return None;
            }
            for signed in [-shift, shift] {
                let (from_m, to_m) = (
                    middle + signed - count * length / 2.0,
                    middle + signed + count * length / 2.0,
                );
                if from_m < lo - SLACK_M || to_m > hi + SLACK_M {
                    continue;
                }
                let at = offset + from_m;
                if models_straight(&line, at, length, count)
                    && ends_straight(&line, at, length, count)
                {
                    return Some(BridgeSpan {
                        stretch: dip.stretch,
                        from_m,
                        to_m,
                    });
                }
            }
            steps += 1.0;
        }
    })();
    match span {
        Some(span) => (DipAction::Bridge, Some(span)),
        None => (otherwise, None),
    }
}

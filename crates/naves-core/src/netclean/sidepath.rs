// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Step 1 of the clean-up: a lesser road that runs close beside a higher road is removed where it disturbs, and
//! whatever joined it is joined to the road instead.
//!
//! Only graph stretches are looked at (roads and tracks); footways, paths and cycleways are painted strokes that
//! shape no terrain and are not in the graph. The stretches of one family are judged together, the highest family
//! first, each against the roads of a higher family as the earlier families left them, so that a road that was
//! removed is never a road another stretch is joined to.
//!
//! A *run* is the part of a stretch whose samples (every metre) lie within [`SIDEPATH_MAX_M`] of a higher road on one
//! side of it, whatever way the stretch heads there, with at least two samples that head the way the road does (a road
//! that joins at an angle is not beside it). Runs that meet at a node are one *group*. A group is a candidate if the
//! parts of its runs between their first and last sample that head the way the road does are together at least
//! [`SIDEPATH_MIN_RUN_M`] long, or runs beside a road tagged `foot=use_sidepath`; it is removed if it
//! disturbs, that is if somewhere along the part that heads along the road the earthworks of the two roads (half the
//! width and the shoulder ring of [`core_reach_m`]) overlap or leave less than one terrain cell between them, or if it
//! passes a bridge of the road (within the bridge's length and one cell at each end). Otherwise it stays, with the
//! reason `clear`.
//!
//! What was joined to a removed part is not lost: every stretch that met it at a node goes on along its own heading to
//! the road (within twice [`SIDEPATH_MAX_M`]) or else straight to the nearest point; the rest of a stretch at an end of
//! a removed part inside it meets the road shallowly ahead of that end. The road is cut there by a new node unless a
//! node is within [`JUNCTION_SPACING_M`] (the length of a junction piece; a road piece shorter than two of them gets no new
//! node), two new nodes on one road piece are that far apart, and none is nearer than that to the end of a bridge (a
//! piece shorter than two junction pieces with a bridge or tunnel at both ends is not joined). The road is within reach
//! if its nearest point is within [`SIDEPATH_MAX_M`]; the connector to the place the joint moved to may be twice as long.
//! If such a connector (also one moved onto an earlier new node) would cross a third road or be too long, or the road is
//! out of reach or cannot be joined there, the run next to that joint stays (the shorter of the two at a node between
//! two runs) and is reported. A short end piece that would be left between the road and a node that is already on the
//! road the run is beside (or free) goes with the run. If the removal would split a component of the network (checked
//! by membership of its nodes, so that a component that goes whole does not make up for it), the groups are tried one
//! at a time and the ones that would split it stay (`would_split`). A joint that would land on an end node of the run's
//! own stretch that the run does not reach (a switchback's lower leg joined to the junction it started from) is
//! refused and the run stays (`loops_back`). A short end piece goes with the run also when the run covers the other end
//! of its stretch and that end is free: a dead-end spur is removed whole, not joined to the road.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    DroppedSidepath, KeptSidepath, SIDEPATH_MAX_M, SIDEPATH_MIN_RUN_M, SPECK_M, known_less, rebuilt,
};
use crate::catalog::Family;
use crate::conform::core_reach_m;
use crate::roadnet::{
    DUPLICATE_MAX_HEADING_DEG, DUPLICATE_STEP_M, EPS_M, Hit, Joint, KeptPiece, RoadGraph, Run,
    Stretch, bounds, cut_runs_named, family_rank, line_crossings, nearest_hit_within, point_along,
    polyline_length, samples,
};
use crate::roadplace::JUNCTION_SPACING_M;

/// A position in map metres.
type Point = (f64, f64);

/// The family rank of the lowest family (the one with the highest number).
const LOWEST_RANK: u8 = 5;
/// A crossing this close to an end of a connector is the connector's own end.
const TOUCH_M: f64 = 0.05;
/// The word of the pieces a cut stretch is named after (`<id>/side<n>`).
const PIECE_WORD: &str = "side";

/// A part of a stretch that runs beside a road.
struct SideRun {
    /// Index of the stretch in the graph.
    stretch: usize,
    /// Metres along the stretch where the part starts and ends.
    from: f64,
    to: f64,
    /// Where the part starts and ends before short end pieces were added to it.
    raw: (f64, f64),
    /// Where the road is met at the two ends: the nearest point of the road (not on a bridge).
    first: Hit,
    last: Hit,
    /// The two ends of the part on the stretch.
    first_at: Point,
    last_at: Point,
    /// The length of the part between its first and its last sample that heads the way the road does.
    aligned_span: f64,
    /// The direction of the stretch at the two ends of the part.
    first_dir: Point,
    last_dir: Point,
    /// Index of the road the middle of the part runs beside.
    road: usize,
    /// On which side of the stretch (+1 left, -1 right, going from its start to its end) the road lies; 0 if
    /// it is not known.
    side: i8,
    tagged: bool,
    disturbs: bool,
}

/// What became of a run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// Not removed: its group is not a candidate, does not disturb, or was refused.
    Off,
    /// To be removed.
    On,
    /// Kept after all, for the reason.
    Kept(&'static str),
}

/// Runs that meet at nodes.
struct Group {
    runs: Vec<usize>,
    /// It is long enough, or runs beside a tagged road.
    candidate: bool,
    disturbs: bool,
}

/// What is known of a graph for one family.
struct Ctx<'a> {
    graph: &'a RoadGraph,
    rank: u8,
    runs: Vec<SideRun>,
    groups: Vec<Group>,
    /// The runs of each stretch, by stretch index, in order along it.
    of_stretch: BTreeMap<usize, Vec<usize>>,
    /// For each node the stretches that end there and whether it is their end (else their start).
    ends: BTreeMap<i64, Vec<(usize, bool)>>,
    /// Whether the stretch has usable points.
    usable: Vec<bool>,
    /// The nodes at the end of a bridge or a tunnel.
    deck_ends: BTreeSet<i64>,
}

/// The stretches of `graph` in the order of their ids.
fn sorted(graph: &RoadGraph) -> RoadGraph {
    let mut stretches = graph.stretches.clone();
    stretches.sort_by(|a, b| {
        a.id.cmp(&b.id)
            .then(a.start.cmp(&b.start))
            .then(a.end.cmp(&b.end))
    });
    if stretches == graph.stretches {
        return graph.clone();
    }
    rebuilt(stretches)
}

fn usable(s: &Stretch) -> bool {
    s.points.len() >= 2 && s.points.iter().all(|p| p.0.is_finite() && p.1.is_finite())
}

/// Which side of the sample's direction `at` lies on: 1 left, -1 right, 0 on it.
fn side_of(dir: Point, from: Point, at: Point) -> i8 {
    let cross = dir.0 * (at.1 - from.1) - dir.1 * (at.0 - from.0);
    if cross > 1e-9 {
        1
    } else if cross < -1e-9 {
        -1
    } else {
        0
    }
}

/// The distance from `p` to the segment `a`-`b`.
fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2.max(f64::MIN_POSITIVE)).clamp(0.0, 1.0);
    (p.0 - (a.0 + dx * t)).hypot(p.1 - (a.1 + dy * t))
}

/// The line of a bridge stretch with one cell more at each end, along its first and last segment.
fn extended(points: &[Point], cell: f64) -> Vec<Point> {
    // a cell that is not a number (or not positive) adds nothing
    let cell = cell.max(0.0).min(f64::from(u16::MAX));
    let mut line = points.to_vec();
    let push = |from: Point, to: Point| -> Option<Point> {
        let len = (to.0 - from.0).hypot(to.1 - from.1);
        (len > 0.0).then(|| {
            (
                to.0 + (to.0 - from.0) / len * cell,
                to.1 + (to.1 - from.1) / len * cell,
            )
        })
    };
    if let [a, b, ..] = points
        && let Some(p) = push(*b, *a)
    {
        line.insert(0, p);
    }
    if let [.., a, b] = points
        && let Some(p) = push(*a, *b)
    {
        line.push(p);
    }
    line
}

/// Whether `at` lies beyond the end of the road `hit` is on, where the nearest point of the road is its end: a stretch
/// that goes past the end of a road does not run beside it there.
fn beyond_end(graph: &RoadGraph, hit: Hit, at: Point) -> bool {
    let Some(road) = graph.stretches.get(hit.stretch) else {
        return false;
    };
    let length = polyline_length(&road.points);
    let points = &road.points;
    let ends = if hit.along <= EPS_M {
        points.first().zip(points.get(1))
    } else if hit.along >= length - EPS_M {
        points
            .last()
            .zip(points.len().checked_sub(2).and_then(|i| points.get(i)))
    } else {
        None
    };
    // the end and the point next to it: beyond if `at` is on the far side of the end from the road
    ends.is_some_and(|(&end, &next)| {
        (at.0 - end.0) * (end.0 - next.0) + (at.1 - end.1) * (end.1 - next.1) > EPS_M
    })
}

/// Where the road is met when it is on a bridge: at the nearer end of the bridge, so that no node is made on a deck.
fn off_the_deck(graph: &RoadGraph, hit: Hit) -> Hit {
    let Some(road) = graph.stretches.get(hit.stretch).filter(|s| s.bridge) else {
        return hit;
    };
    let length = polyline_length(&road.points);
    let (along, end) = if hit.along <= length / 2.0 {
        (0.0, road.points.first())
    } else {
        (length, road.points.last())
    };
    Hit {
        stretch: hit.stretch,
        along,
        at: end.copied().unwrap_or(hit.at),
    }
}

/// The roads and bridges of a higher rank that the stretches of one family are compared with.
struct Higher {
    pieces: Vec<KeptPiece>,
    /// Every bridge of a higher family.
    bridges: Vec<BridgeLine>,
}

/// A bridge stretch of a higher family: its line one cell longer at both ends, the box of that line, the stretch and
/// its nodes.
struct BridgeLine {
    line: Vec<Point>,
    area: (Point, Point),
    stretch: usize,
    nodes: [i64; 2],
}

/// What a run is made of while it is found: its samples, with the nearest road of each.
struct Sampled<'a> {
    graph: &'a RoadGraph,
    stretch: usize,
    samples: Vec<crate::roadnet::Sample>,
    /// For each sample the nearest road within reach (the nearest one that heads the way the stretch does, if
    /// there is one), the side it is on and whether it heads the way the stretch does.
    hits: Vec<Option<(f64, Hit, i8, bool)>>,
    bridges: Vec<&'a BridgeLine>,
}

impl Sampled<'_> {
    /// The run of the samples `a` to `last`: whether it disturbs and whether a road beside it is tagged.
    fn run(
        &self,
        a: usize,
        last: usize,
        side: i8,
        widths: &dyn Fn(Family) -> f64,
        cell: f64,
    ) -> Option<SideRun> {
        let own_half = self
            .graph
            .stretches
            .get(self.stretch)
            .map_or(0.0, |s| widths(s.family) / 2.0);
        let (mut disturbs, mut tagged) = (false, false);
        for (sample, (d, h, _, along_the_road)) in self
            .samples
            .iter()
            .zip(&self.hits)
            .skip(a)
            .take(last - a + 1)
            .filter_map(|(sample, hit)| hit.map(|h| (sample, h)))
        {
            let road = self.graph.stretches.get(h.stretch);
            tagged |= road.is_some_and(|r| r.use_sidepath);
            // the part that bends towards the road to join it is in the run, but it is the part beside the road that
            // disturbs
            if !along_the_road {
                continue;
            }
            let road_half = road.map_or(0.0, |r| widths(r.family) / 2.0);
            disturbs |= known_less(
                d,
                core_reach_m(own_half, cell) + core_reach_m(road_half, cell) + cell,
            );
            // only a bridge of the road it runs beside: the hit stretch itself or one that joins it
            disturbs |= self.bridges.iter().any(|b| {
                let of_road = b.stretch == h.stretch
                    || road.is_some_and(|r| b.nodes.iter().any(|n| *n == r.start || *n == r.end));
                of_road
                    && b.line.windows(2).any(|w| {
                        matches!(w, [p, q] if segment_distance(sample.at, *p, *q) <= SIDEPATH_MAX_M)
                    })
            });
        }
        let ((_, first, ..), (_, end, ..)) = (
            self.hits.get(a).copied().flatten()?,
            self.hits.get(last).copied().flatten()?,
        );
        let (from, to) = (self.samples.get(a)?, self.samples.get(last)?);
        let middle = self
            .hits
            .get(usize::midpoint(a, last))
            .copied()
            .flatten()
            .map_or(first.stretch, |(_, h, ..)| h.stretch);
        let aligned: Vec<f64> = self
            .samples
            .iter()
            .zip(&self.hits)
            .skip(a)
            .take(last - a + 1)
            .filter(|(_, hit)| hit.is_some_and(|(.., along_the_road)| along_the_road))
            .map(|(sample, _)| sample.along)
            .collect();
        let aligned_span = aligned
            .first()
            .zip(aligned.last())
            .map_or(0.0, |(first, last)| last - first);
        Some(SideRun {
            aligned_span,
            stretch: self.stretch,
            from: from.along,
            to: to.along,
            raw: (from.along, to.along),
            first: off_the_deck(self.graph, first),
            last: off_the_deck(self.graph, end),
            first_at: from.at,
            last_at: to.at,
            first_dir: from.dir,
            last_dir: to.dir,
            road: middle,
            side,
            tagged,
            disturbs,
        })
    }

    /// The runs: stretches of samples with a road within reach on one side, whatever way the stretch heads there, that
    /// have at least two samples that head the way the road does (a road that joins at an angle is not beside it).
    fn runs(&self, widths: &dyn Fn(Family) -> f64, cell: f64) -> Vec<SideRun> {
        let mut found = Vec::new();
        let mut open: Option<(usize, i8)> = None;
        for k in 0..=self.hits.len() {
            let here = self.hits.get(k).copied().flatten();
            let next = match (open, here) {
                (Some((a, side)), Some((_, _, s2, _))) if side == 0 || s2 == 0 || side == s2 => {
                    open = Some((a, if side == 0 { s2 } else { side }));
                    continue;
                }
                (_, Some((_, _, s2, _))) => Some((k, s2)),
                (_, None) => None,
            };
            let along_the_road = |a: usize, last: usize| {
                self.hits
                    .iter()
                    .skip(a)
                    .take(last - a + 1)
                    .filter(|h| h.is_some_and(|(.., aligned)| aligned))
                    .count()
                    >= 2
            };
            if let Some((a, side)) = open
                && k > a + 1
                && along_the_road(a, k - 1)
            {
                found.extend(self.run(a, k - 1, side, widths, cell));
            }
            open = next;
        }
        found
    }
}

/// The runs of the stretches of `rank` beside the roads of a higher rank.
fn find_runs(
    graph: &RoadGraph,
    rank: u8,
    widths: &dyn Fn(Family) -> f64,
    cell: f64,
) -> Vec<SideRun> {
    let higher = Higher {
        pieces: graph
            .stretches
            .iter()
            .enumerate()
            .filter(|(_, s)| family_rank(s.family) < rank && !s.tunnel && usable(s))
            .map(|(i, s)| KeptPiece::whole(i, s, widths(s.family)))
            .collect(),
        bridges: graph
            .stretches
            .iter()
            .enumerate()
            .filter(|(_, s)| family_rank(s.family) < rank && s.bridge && !s.tunnel && usable(s))
            .map(|(stretch, s)| {
                let line = extended(&s.points, cell);
                let area = bounds(&line);
                BridgeLine {
                    line,
                    area,
                    stretch,
                    nodes: [s.start, s.end],
                }
            })
            .collect(),
    };
    let mut found = Vec::new();
    for (index, s) in graph.stretches.iter().enumerate() {
        let candidate = family_rank(s.family) == rank
            && !s.bridge
            && !s.tunnel
            && !s.roundabout
            && s.start != s.end
            && usable(s);
        if !candidate {
            continue;
        }
        let (min, max) = bounds(&s.points);
        let near_box = |lo: Point, hi: Point| {
            lo.0 <= max.0 + SIDEPATH_MAX_M
                && hi.0 >= min.0 - SIDEPATH_MAX_M
                && lo.1 <= max.1 + SIDEPATH_MAX_M
                && hi.1 >= min.1 - SIDEPATH_MAX_M
        };
        let near: Vec<&KeptPiece> = higher
            .pieces
            .iter()
            .filter(|k| near_box(k.min, k.max))
            .collect();
        if near.is_empty() {
            continue;
        }
        let points = samples(&s.points);
        let aligned = DUPLICATE_MAX_HEADING_DEG.to_radians().cos();
        let hits = points
            .iter()
            .map(|sample| {
                let nearest = |cos: f64| {
                    nearest_hit_within(sample.at, sample.dir, &|_| SIDEPATH_MAX_M, cos, &near)
                        .filter(|(_, hit)| !beyond_end(graph, *hit, sample.at))
                };
                let (found, along) = match nearest(aligned) {
                    Some(hit) => (Some(hit), true),
                    None => (nearest(0.0), false),
                };
                found.map(|(d, hit)| (d, hit, side_of(sample.dir, sample.at, hit.at), along))
            })
            .collect();
        let along = Sampled {
            graph,
            stretch: index,
            samples: points,
            hits,
            bridges: higher
                .bridges
                .iter()
                .filter(|b| near_box(b.area.0, b.area.1))
                .collect(),
        };
        found.extend(along.runs(widths, cell));
    }
    found
}

impl<'a> Ctx<'a> {
    fn new(graph: &'a RoadGraph, rank: u8, widths: &dyn Fn(Family) -> f64, cell: f64) -> Ctx<'a> {
        let runs = find_runs(graph, rank, widths, cell);
        let mut of_stretch: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (k, run) in runs.iter().enumerate() {
            of_stretch.entry(run.stretch).or_default().push(k);
        }
        let mut ends: BTreeMap<i64, Vec<(usize, bool)>> = BTreeMap::new();
        for (i, s) in graph.stretches.iter().enumerate() {
            ends.entry(s.start).or_default().push((i, false));
            ends.entry(s.end).or_default().push((i, true));
        }
        let mut ctx = Ctx {
            graph,
            rank,
            runs,
            groups: Vec::new(),
            of_stretch,
            ends,
            usable: graph.stretches.iter().map(usable).collect(),
            deck_ends: graph
                .stretches
                .iter()
                .filter(|s| s.bridge || s.tunnel)
                .flat_map(|s| [s.start, s.end])
                .collect(),
        };
        ctx.groups = ctx.group_runs();
        ctx.extend_short_ends();
        ctx
    }

    fn stretch(&self, i: usize) -> Option<&'a Stretch> {
        self.graph.stretches.get(i)
    }

    fn total(&self, i: usize) -> f64 {
        self.stretch(i).map_or(0.0, |s| polyline_length(&s.points))
    }

    /// Whether the run covers the start (or the end) of its stretch as it was found.
    fn covers(&self, run: &SideRun, at_end: bool) -> bool {
        if at_end {
            run.raw.1 >= self.total(run.stretch) - EPS_M
        } else {
            run.raw.0 <= EPS_M
        }
    }

    /// Whether `node` is on the road `run` runs beside: a node of that road stretch, or one that a higher stretch
    /// passes through (two or more of them end there). A higher road that merely ends at the node is not on the road.
    fn on_road(&self, node: i64, run: &SideRun) -> bool {
        let higher_ends = self
            .ends
            .get(&node)
            .into_iter()
            .flatten()
            .filter(|(x, _)| {
                self.stretch(*x)
                    .is_some_and(|s| family_rank(s.family) < self.rank)
            })
            .count();
        higher_ends >= 2
            || self
                .stretch(run.road)
                .is_some_and(|road| road.start == node || road.end == node)
    }

    /// The point `along` metres along the road `road` as a place to join it: a node of the road if one is within
    /// [`JUNCTION_SPACING_M`] (the nearer end node of a piece shorter than two of them), and never nearer than that to
    /// the end of a bridge or tunnel (it moves along the road to that distance). A road
    /// that is itself a bridge is met as it is. `None` where the road cannot be joined: a piece shorter than two
    /// junction pieces with a bridge or tunnel at both ends.
    fn snapped(&self, hit: Hit) -> Option<Hit> {
        let Some(road) = self.stretch(hit.stretch).filter(|r| !r.bridge && !r.tunnel) else {
            return Some(hit);
        };
        let length = self.total(hit.stretch);
        let (deck_start, deck_end) = (
            self.deck_ends.contains(&road.start),
            self.deck_ends.contains(&road.end),
        );
        let spacing = JUNCTION_SPACING_M;
        let mut along = hit.along.clamp(0.0, length);
        if length < 2.0 * spacing {
            // too short for a new node: the nearer end that is no deck end
            let (to_start, to_end) = (!deck_start, !deck_end);
            along = match (to_start, to_end) {
                (true, true) if along <= length - along => 0.0,
                (_, true) => length,
                (true, false) => 0.0,
                (false, false) => return None,
            };
        } else if along < spacing {
            // a node of the road within a junction piece's length is used; a deck end is kept the same distance off
            along = if deck_start { spacing } else { 0.0 };
        } else if length - along < spacing {
            along = if deck_end { length - spacing } else { length };
        }
        along = along.clamp(0.0, length);
        if (along - hit.along).abs() < EPS_M {
            return Some(hit);
        }
        Some(Hit {
            stretch: hit.stretch,
            along,
            at: point_along(&road.points, along),
        })
    }

    /// Where the rest of a stretch that ends inside a run meets the road at the `at_end` end of the run: a point
    /// ahead of the end, towards the removed part, by twice the distance from the road (not further than a connector
    /// of twice [`SIDEPATH_MAX_M`]) and within the part of the road the run lies along, so that it merges at a
    /// shallow angle instead of a corner. `None` if the road is a bridge, has no heading there or cannot be joined
    /// there.
    fn shallow(&self, run: &SideRun, at_end: bool) -> Option<Hit> {
        let (hit, from, dir, other) = if at_end {
            (run.last, run.last_at, run.last_dir, run.first)
        } else {
            (run.first, run.first_at, run.first_dir, run.last)
        };
        let road = self.stretch(hit.stretch).filter(|r| !r.bridge)?;
        let d = (from.0 - hit.at.0).hypot(from.1 - hit.at.1);
        let tangent = tangent_at(&road.points, hit.along)?;
        // the direction from this end towards the removed part
        let toward = if at_end { (-dir.0, -dir.1) } else { dir };
        let sign = if toward.0 * tangent.0 + toward.1 * tangent.1 >= 0.0 {
            1.0
        } else {
            -1.0
        };
        let limit = 2.0 * SIDEPATH_MAX_M;
        let reach = (2.0 * d).min((limit * limit - d * d).max(0.0).sqrt());
        let length = self.total(hit.stretch);
        let (lo, hi) = if other.stretch == hit.stretch {
            (hit.along.min(other.along), hit.along.max(other.along))
        } else {
            (0.0, length)
        };
        let along = (hit.along + sign * reach).clamp(lo, hi);
        self.snapped(Hit {
            stretch: hit.stretch,
            along,
            at: point_along(&road.points, along),
        })
    }

    /// Where a stretch that leaves `from` along `dir` meets the road `road`, if within twice [`SIDEPATH_MAX_M`].
    fn ray_hit(&self, from: Point, dir: Point, road: usize) -> Option<Hit> {
        let line = self.stretch(road).filter(|l| !l.bridge)?;
        let mut best: Option<(f64, f64)> = None;
        let mut run = 0.0;
        for w in line.points.windows(2) {
            let [from_pt, to_pt] = w else { continue };
            let (ex, ey) = (to_pt.0 - from_pt.0, to_pt.1 - from_pt.1);
            let len = ex.hypot(ey);
            let denom = dir.0 * ey - dir.1 * ex;
            if denom.abs() > 1e-12 && len > 0.0 {
                let (gx, gy) = (from_pt.0 - from.0, from_pt.1 - from.1);
                let ahead = (gx * ey - gy * ex) / denom;
                let across = (gx * dir.1 - gy * dir.0) / denom;
                if ahead > EPS_M
                    && ahead <= 2.0 * SIDEPATH_MAX_M
                    && (0.0..=1.0).contains(&across)
                    && best.is_none_or(|(nearest, _)| ahead < nearest)
                {
                    best = Some((ahead, run + across * len));
                }
            }
            run += len;
        }
        let (_, along) = best?;
        let hit = Hit {
            stretch: road,
            along,
            at: point_along(&line.points, along),
        };
        // a point that has to move to a node or off a deck end no longer continues the heading: the plain connector
        // is made instead
        let snapped = self.snapped(hit)?;
        ((snapped.along - hit.along).abs() <= 1.0).then_some(snapped)
    }

    /// Groups of runs that meet at a node where exactly two of them end, on the same side of the road.
    fn group_runs(&self) -> Vec<Group> {
        let mut parent: Vec<usize> = (0..self.runs.len()).collect();
        // node -> (run, the node is the end of its stretch)
        let mut at_node: BTreeMap<i64, Vec<(usize, bool)>> = BTreeMap::new();
        for (k, run) in self.runs.iter().enumerate() {
            let Some(s) = self.stretch(run.stretch) else {
                continue;
            };
            for at_end in [false, true] {
                if self.covers(run, at_end) {
                    at_node
                        .entry(if at_end { s.end } else { s.start })
                        .or_default()
                        .push((k, at_end));
                }
            }
        }
        for list in at_node.values() {
            if let [(a, a_end), (b, b_end)] = list[..] {
                let (Some(p), Some(q)) = (self.runs.get(a), self.runs.get(b)) else {
                    continue;
                };
                // the side of the road going away from the node along each run: the same road side means opposite sides
                let away = |run: &SideRun, at_end: bool| if at_end { -run.side } else { run.side };
                if p.stretch != q.stretch && away(p, a_end) + away(q, b_end) == 0 {
                    let (ra, rb) = (super::root(&mut parent, a), super::root(&mut parent, b));
                    if let Some(slot) = parent.get_mut(ra.max(rb)) {
                        *slot = ra.min(rb);
                    }
                }
            }
        }
        let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for k in 0..self.runs.len() {
            let r = super::root(&mut parent, k);
            by_root.entry(r).or_default().push(k);
        }
        by_root
            .into_values()
            .map(|runs| {
                let length: f64 = runs
                    .iter()
                    .filter_map(|&k| self.runs.get(k))
                    .map(|r| r.aligned_span)
                    .sum();
                let any = |f: &dyn Fn(&SideRun) -> bool| {
                    runs.iter().filter_map(|&k| self.runs.get(k)).any(f)
                };
                Group {
                    candidate: length >= SIDEPATH_MIN_RUN_M || any(&|r| r.tagged),
                    disturbs: any(&|r| r.disturbs),
                    runs,
                }
            })
            .collect()
    }

    /// The end pieces of a stretch that would be left between a removed run and an end that is free or already on
    /// a higher road, shorter than [`SIDEPATH_MAX_M`], go with the run: they would only be a loop beside the road.
    fn extend_short_ends(&mut self) {
        let wanted: Vec<(usize, bool)> = self
            .groups
            .iter()
            .filter(|g| g.candidate && g.disturbs)
            .flat_map(|g| g.runs.iter().copied())
            .flat_map(|k| [(k, false), (k, true)])
            .collect();
        for (k, at_end) in wanted {
            let Some(run) = self.runs.get(k) else {
                continue;
            };
            let total = self.total(run.stretch);
            // the run really begins up to one sample step before its first sample
            let gap = if at_end { total - run.to } else { run.from };
            let Some(s) = self.stretch(run.stretch) else {
                continue;
            };
            let node = if at_end { s.end } else { s.start };
            // the end is free, or on the road the run is beside
            let free_or_on_road = self
                .ends
                .get(&node)
                .into_iter()
                .flatten()
                .all(|(x, _)| *x == run.stretch)
                || self.on_road(node, run);
            // the run covers the other end of a dead-end spur: the whole spur goes, it is not joined to the road
            let other = if at_end { s.start } else { s.end };
            // (the far end is taken in its turn when its own rest is short, as it is free)
            let far_gap = if at_end { run.from } else { total - run.to };
            let whole_spur = (far_gap <= EPS_M || far_gap - DUPLICATE_STEP_M < SIDEPATH_MAX_M)
                && self
                    .ends
                    .get(&other)
                    .into_iter()
                    .flatten()
                    .all(|(x, _)| *x == run.stretch);
            if gap > EPS_M
                && gap - DUPLICATE_STEP_M < SIDEPATH_MAX_M
                && (free_or_on_road || whole_spur)
                && let Some(run) = self.runs.get_mut(k)
            {
                if at_end {
                    run.to = total;
                } else {
                    run.from = 0.0;
                }
            }
        }
    }

    /// Whether the joint `found` is an end node of the run's own stretch that the run does not reach: the rest would
    /// loop back onto the road it came from.
    fn loops_back(&self, run: &SideRun, found: Hit) -> bool {
        let (Some(s), Some(road)) = (self.stretch(run.stretch), self.stretch(found.stretch)) else {
            return false;
        };
        let length = self.total(found.stretch);
        let node = if found.along <= EPS_M {
            road.start
        } else if found.along >= length - EPS_M {
            road.end
        } else {
            return false;
        };
        (node == s.start && run.from > EPS_M)
            || (node == s.end && run.to < self.total(run.stretch) - EPS_M)
    }

    /// Whether the run `k`, which reaches the `at_end` end of its stretch, reaches the other end too, and that end is
    /// free: the stretch is a dead-end spur that goes whole.
    fn whole_spur(&self, k: usize, at_end: bool) -> bool {
        let Some(r) = self.runs.get(k) else {
            return false;
        };
        let far_reached = if at_end {
            r.from <= EPS_M
        } else {
            r.to >= self.total(r.stretch) - EPS_M
        };
        far_reached
            && self.stretch(r.stretch).is_some_and(|s| {
                self.ends
                    .get(if at_end { &s.start } else { &s.end })
                    .into_iter()
                    .flatten()
                    .all(|(x, _)| *x == r.stretch)
            })
    }

    /// The runs that end at `node` (the stretch's start or end is the node and the run reaches it) and are `On`.
    fn covering(&self, state: &[State], node: i64) -> Vec<(usize, bool)> {
        let mut out = Vec::new();
        for &(x, at_end) in self.ends.get(&node).into_iter().flatten() {
            let total = self.total(x);
            for &k in self.of_stretch.get(&x).into_iter().flatten() {
                let Some(run) = self.runs.get(k) else {
                    continue;
                };
                let reaches = if at_end {
                    run.to >= total - EPS_M
                } else {
                    run.from <= EPS_M
                };
                if reaches && state.get(k) == Some(&State::On) && !out.contains(&(k, at_end)) {
                    out.push((k, at_end));
                }
            }
        }
        out
    }

    /// Whether a straight connector from `from` to `to` would be longer than `limit` (`would_disconnect`) or would
    /// cross a road that stays (`crosses`).
    fn bad_connector(
        &self,
        state: &[State],
        from: Point,
        to: Point,
        limit: f64,
    ) -> Option<&'static str> {
        if known_less(limit, (to.0 - from.0).hypot(to.1 - from.1)) {
            return Some("would_disconnect");
        }
        let (min, max) = (
            (from.0.min(to.0), from.1.min(to.1)),
            (from.0.max(to.0), from.1.max(to.1)),
        );
        let line = [from, to];
        for (i, s) in self.graph.stretches.iter().enumerate() {
            let removed = self
                .of_stretch
                .get(&i)
                .into_iter()
                .flatten()
                .any(|&k| state.get(k) == Some(&State::On));
            if removed || !self.usable.get(i).copied().unwrap_or(false) {
                continue;
            }
            let (lo, hi) = bounds(&s.points);
            if lo.0 > max.0 || hi.0 < min.0 || lo.1 > max.1 || hi.1 < min.1 {
                continue;
            }
            let apart = |p: Point, q: Point| (p.0 - q.0).hypot(p.1 - q.1) > TOUCH_M;
            if line_crossings(&line, &s.points)
                .iter()
                .any(|(at, ..)| apart(*at, from) && apart(*at, to))
            {
                return Some("crosses");
            }
        }
        None
    }

    /// Why the joint from `from` to the road cannot be made at `snapped`, the place `nearest` (the nearest point of the
    /// road) moved to: the road is out of reach (`nearest` further than [`SIDEPATH_MAX_M`]) or cannot be joined
    /// there (`would_disconnect`), or the connector is longer than twice [`SIDEPATH_MAX_M`] or crosses a road that
    /// stays.
    fn joint_reason(
        &self,
        state: &[State],
        from: Point,
        nearest: Point,
        snapped: Option<Hit>,
    ) -> Option<&'static str> {
        let Some(snapped) = snapped else {
            return Some("would_disconnect");
        };
        if known_less(
            SIDEPATH_MAX_M,
            (nearest.0 - from.0).hypot(nearest.1 - from.1),
        ) {
            return Some("would_disconnect");
        }
        self.bad_connector(state, from, snapped.at, 2.0 * SIDEPATH_MAX_M)
    }

    /// Of the runs `removed` that end at one node, the one that stays when the node cannot be joined: the shorter.
    fn shorter_of(&self, removed: &[usize]) -> Option<usize> {
        removed.iter().copied().min_by(|&a, &b| {
            let len = |k: usize| self.runs.get(k).map_or(0.0, |r| r.to - r.from);
            len(a).total_cmp(&len(b)).then(a.cmp(&b))
        })
    }

    /// What has to be joined to the road at `node`, if anything: the runs that end there and are removed, and the
    /// stretches that stay.
    fn node_need(&self, state: &[State], node: i64) -> Option<NodeNeed> {
        let removed = self.covering(state, node);
        if removed.is_empty() {
            return None;
        }
        let kept: Vec<usize> = self
            .ends
            .get(&node)
            .into_iter()
            .flatten()
            .filter(|(x, at_end)| {
                !removed.iter().any(|&(k, e)| {
                    e == *at_end && self.runs.get(k).is_some_and(|r| r.stretch == *x)
                })
            })
            .map(|(x, _)| *x)
            .collect();
        if kept.is_empty() {
            return None;
        }
        let first_run = removed.iter().find_map(|&(k, _)| self.runs.get(k))?;
        // a dead-end spur that goes whole leaves a node that two or more roads still pass through: nothing to join
        if kept.len() >= 2
            && removed
                .iter()
                .all(|&(k, at_end)| self.whole_spur(k, at_end))
        {
            return None;
        }
        let on_road = self.on_road(node, first_run);
        let beside_kept_run = kept.iter().any(|x| {
            self.of_stretch
                .get(x)
                .into_iter()
                .flatten()
                .any(|&k| matches!(state.get(k), Some(State::Kept(_))))
        });
        if on_road || beside_kept_run {
            return None;
        }
        // the run that decides where the connector goes: the first by stretch and place
        let &(designated, at_end) = removed.iter().min_by(|a, b| {
            let key = |&(k, e): &(usize, bool)| {
                self.runs
                    .get(k)
                    .map_or((usize::MAX, 0.0, e), |r| (r.stretch, r.from, e))
            };
            let (ka, kb) = (key(a), key(b));
            ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
        })?;
        let run = self.runs.get(designated)?;
        let nearest = if at_end { run.last } else { run.first };
        let plain = self.snapped(nearest);
        let target = plain.unwrap_or(nearest);
        // a stretch that joined the removed part goes on along its own heading to the road, if that is near enough
        let ray = kept.iter().min().and_then(|&x| {
            let joining = self.stretch(x).filter(|j| j.start != j.end)?;
            let [end, next] = if joining.start == node {
                [joining.points.first()?, joining.points.get(1)?]
            } else {
                [
                    joining.points.last()?,
                    joining.points.get(joining.points.len().checked_sub(2)?)?,
                ]
            };
            let len = (end.0 - next.0).hypot(end.1 - next.1);
            (len > 0.0).then_some(())?;
            let dir = ((end.0 - next.0) / len, (end.1 - next.1) / len);
            self.ray_hit(*end, dir, target.stretch)
        });
        let source = self
            .stretch(run.stretch)
            .and_then(|s| {
                if at_end {
                    s.points.last()
                } else {
                    s.points.first()
                }
            })
            .copied()?;
        Some(NodeNeed {
            node,
            designated,
            at_end,
            removed: removed.iter().map(|&(k, _)| k).collect(),
            source,
            nearest,
            plain,
            target,
            ray,
        })
    }

    /// Where the rest of a stretch that ends inside the run `k` meets the road at its two ends: the first of the
    /// shallow and the nearest point whose connector is fine, or the reason that none is.
    fn inside_ends(
        &self,
        state: &[State],
        run: &SideRun,
    ) -> Result<[Option<Hit>; 2], &'static str> {
        let total = self.total(run.stretch);
        let mut chosen = [None; 2];
        for (end, (inside, from)) in [
            (run.from > EPS_M, run.first_at),
            (run.to < total - EPS_M, run.last_at),
        ]
        .into_iter()
        .enumerate()
        {
            if !inside {
                continue;
            }
            let at_end = end == 1;
            let shallow = self.shallow(run, at_end).filter(|target| {
                self.bad_connector(state, from, target.at, 2.0 * SIDEPATH_MAX_M)
                    .is_none()
            });
            let found = if let Some(target) = shallow {
                target
            } else {
                let nearest = if at_end { run.last } else { run.first };
                let plain = self.snapped(nearest);
                if let Some(reason) = self.joint_reason(state, from, nearest.at, plain) {
                    return Err(reason);
                }
                plain.unwrap_or(nearest)
            };
            if self.loops_back(run, found) {
                return Err("loops_back");
            }
            if let Some(slot) = chosen.get_mut(end) {
                *slot = Some(found);
            }
        }
        Ok(chosen)
    }

    /// The runs of the groups in `selected` are removed, except those that cannot be joined to the road: the state
    /// of every run, the joints to make and where the rest of each stretch meets the road.
    fn plan(&self, selected: &BTreeSet<usize>) -> Plan {
        let mut state = vec![State::Off; self.runs.len()];
        for &g in selected {
            for &k in self.groups.get(g).map_or(&[][..], |g| &g.runs[..]) {
                if let Some(slot) = state.get_mut(k) {
                    *slot = State::On;
                }
            }
        }
        loop {
            let mut drop: Option<(usize, &'static str)> = None;
            let mut ends = vec![[None; 2]; self.runs.len()];
            // the ends of a part inside a stretch are joined to the road
            for (k, run) in self
                .runs
                .iter()
                .enumerate()
                .filter(|(k, _)| state.get(*k) == Some(&State::On))
            {
                match self.inside_ends(&state, run) {
                    Ok(chosen) => {
                        if let Some(slot) = ends.get_mut(k) {
                            *slot = chosen;
                        }
                    }
                    Err(reason) => {
                        drop = Some((k, reason));
                        break;
                    }
                }
            }
            let mut needs = Vec::new();
            let nodes: BTreeSet<i64> = self
                .runs
                .iter()
                .enumerate()
                .filter(|(k, _)| state.get(*k) == Some(&State::On))
                .filter_map(|(_, r)| self.stretch(r.stretch).map(|s| [s.start, s.end]))
                .flatten()
                .collect();
            if drop.is_none() {
                for node in nodes {
                    let Some(mut need) = self.node_need(&state, node) else {
                        continue;
                    };
                    let straight = need.ray.filter(|ray| {
                        self.bad_connector(&state, need.source, ray.at, 2.0 * SIDEPATH_MAX_M)
                            .is_none()
                    });
                    if let Some(ray) = straight {
                        need.target = ray;
                    } else if let Some(reason) =
                        self.joint_reason(&state, need.source, need.nearest.at, need.plain)
                    {
                        // of the runs that end at the node the shorter stays, with what is joined to it
                        drop = self.shorter_of(&need.removed).map(|k| (k, reason));
                        break;
                    }
                    needs.push(need);
                }
            }
            if drop.is_none() {
                drop = self.space_new_nodes(&state, &mut ends, &mut needs);
            }
            match drop {
                Some((k, reason)) => {
                    if let Some(slot) = state.get_mut(k) {
                        *slot = State::Kept(reason);
                    }
                }
                None => return Plan { state, needs, ends },
            }
        }
    }
}

/// What is decided about the removal of a set of groups.
struct Plan {
    state: Vec<State>,
    needs: Vec<NodeNeed>,
    /// For each run the points of the road where the rest of the stretch meets it, at the two ends inside it.
    ends: Vec<[Option<Hit>; 2]>,
}

/// The unit direction of the polyline `along` metres from its start.
fn tangent_at(points: &[Point], along: f64) -> Option<Point> {
    let mut run = 0.0;
    let mut last = None;
    for w in points.windows(2) {
        let [a, b] = w else { continue };
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        if len <= 0.0 {
            continue;
        }
        last = Some(((b.0 - a.0) / len, (b.1 - a.1) / len));
        if run + len >= along {
            break;
        }
        run += len;
    }
    last
}

/// A node where the removal of runs leaves stretches that have to be joined to the road.
struct NodeNeed {
    node: i64,
    /// The run whose joint is made.
    designated: usize,
    at_end: bool,
    removed: Vec<usize>,
    /// The node's place.
    source: Point,
    /// The nearest point of the road to the designated run's end.
    nearest: Hit,
    /// Where the road is joined if not by the ray: `nearest` moved to a place a junction fits, if there is one.
    plain: Option<Hit>,
    /// Where the road is met: the plain place, or where the joining stretch goes on to it.
    target: Hit,
    /// Where the joining stretch meets the road if it goes on along its own heading.
    ray: Option<Hit>,
}

/// What a removal of groups did.
struct Outcome {
    graph: RoadGraph,
    dropped: Vec<DroppedSidepath>,
    /// Runs that stay though their group is removed, with the reason.
    blocked: Vec<(usize, &'static str)>,
}

/// Makes the stretch end on `at`, a node of the road, instead of on `old`.
fn rewire(s: &mut Stretch, old: i64, new: i64, at: Point) {
    // a connector shorter than a speck is not made: the end moves onto the road
    let short =
        |p: Option<&Point>| p.is_some_and(|p| !known_less(SPECK_M, (p.0 - at.0).hypot(p.1 - at.1)));
    if s.start == old {
        if short(s.points.first()) {
            s.points.remove(0);
        }
        s.points.insert(0, at);
        s.start = new;
    }
    if s.end == old {
        if short(s.points.last()) {
            s.points.pop();
        }
        s.points.push(at);
        s.end = new;
    }
    if s.start > s.end {
        std::mem::swap(&mut s.start, &mut s.end);
        s.points.reverse();
    }
}

impl Ctx<'_> {
    /// The graph without the runs of the groups in `selected` (but for those that could not be joined to the road),
    /// the stretches that met them joined to the road, and the report.
    fn evaluate(&self, selected: &BTreeSet<usize>) -> Outcome {
        let Plan { state, needs, ends } = self.plan(selected);
        let blocked: Vec<(usize, &'static str)> = state
            .iter()
            .enumerate()
            .filter_map(|(k, s)| match s {
                State::Kept(reason) => Some((k, *reason)),
                _ => None,
            })
            .collect();
        let mut runs_of: BTreeMap<usize, Vec<Run>> = BTreeMap::new();
        let mut on: Vec<usize> = Vec::new();
        for (k, run) in self.runs.iter().enumerate() {
            if state.get(k) != Some(&State::On) {
                continue;
            }
            on.push(k);
            let joined = |at_end: bool| {
                needs
                    .iter()
                    .find(|n| n.designated == k && n.at_end == at_end)
            };
            let [inside_first, inside_last] = ends.get(k).copied().unwrap_or([None; 2]);
            runs_of.entry(run.stretch).or_default().push(Run {
                from: run.from,
                to: run.to,
                first: joined(false)
                    .map(|n| n.target)
                    .or(inside_first)
                    .unwrap_or(run.first),
                last: joined(true)
                    .map(|n| n.target)
                    .or(inside_last)
                    .unwrap_or(run.last),
                join_start: joined(false).is_some(),
                join_end: joined(true).is_some(),
            });
        }
        if on.is_empty() {
            return Outcome {
                graph: self.graph.clone(),
                dropped: Vec::new(),
                blocked,
            };
        }
        let (cut, joints) = cut_runs_named(self.graph, &runs_of, PIECE_WORD);
        let mut stretches = cut.stretches;
        let mut connectors = self.join_to_road(&mut stretches, &joints, &needs);
        let dropped = on
            .iter()
            .filter_map(|&k| {
                let run = self.runs.get(k)?;
                let s = self.stretch(run.stretch)?;
                let mut joined = connectors.remove(&k).unwrap_or_default();
                joined.sort_by(|a, b| a.0.cmp(&b.0));
                Some(DroppedSidepath {
                    stretch: s.id.clone(),
                    road: self.stretch(run.road)?.id.clone(),
                    from_m: run.from,
                    to_m: run.to,
                    at: point_along(&s.points, f64::midpoint(run.from, run.to)),
                    connectors: joined,
                })
            })
            .collect();
        Outcome {
            graph: rebuilt(stretches),
            dropped,
            blocked,
        }
    }

    /// Two new nodes on one road piece are kept [`JUNCTION_SPACING_M`] apart: of the points where the plan meets a road
    /// piece (in the order of the pieces and of the places along them), a later one within that distance of an earlier
    /// new node uses that node, if the connector to it passes the checks of [`Self::bad_connector`] (within twice
    /// [`SIDEPATH_MAX_M`]). Otherwise the run whose joint it is has to stay: that run and the reason.
    fn space_new_nodes(
        &self,
        state: &[State],
        ends: &mut [[Option<Hit>; 2]],
        needs: &mut [NodeNeed],
    ) -> Option<(usize, &'static str)> {
        // (piece, place, which point): a point of an end inside a stretch, or of a joint at a node
        let mut points: Vec<(usize, f64, usize, usize)> = Vec::new();
        for (k, pair) in ends.iter().enumerate() {
            for (i, hit) in pair.iter().enumerate() {
                if let Some(hit) = hit {
                    points.push((hit.stretch, hit.along, k, i));
                }
            }
        }
        for (n, need) in needs.iter().enumerate() {
            points.push((need.target.stretch, need.target.along, n, 2));
        }
        points.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });
        let mut made: Vec<Hit> = Vec::new();
        for (stretch, along, index, which) in points {
            let length = self.total(stretch);
            if along <= EPS_M || along >= length - EPS_M {
                continue;
            }
            let earlier = made
                .iter()
                .find(|m| m.stretch == stretch && (m.along - along).abs() < JUNCTION_SPACING_M)
                .copied();
            let Some(hit) = earlier else {
                if let Some(h) = Self::hit_at(ends, needs, index, which) {
                    made.push(h);
                }
                continue;
            };
            // where the connector starts, and the run that stays if it cannot be made
            let source = if which == 2 {
                needs
                    .get(index)
                    .and_then(|n| Some((n.source, self.shorter_of(&n.removed)?)))
            } else {
                self.runs
                    .get(index)
                    .map(|r| (if which == 0 { r.first_at } else { r.last_at }, index))
            };
            let Some((from, keep)) = source else { continue };
            if let Some(reason) = self.bad_connector(state, from, hit.at, 2.0 * SIDEPATH_MAX_M) {
                return Some((keep, reason));
            }
            if which == 2 {
                if let Some(need) = needs.get_mut(index) {
                    need.target = hit;
                }
            } else if let Some(slot) = ends.get_mut(index).and_then(|p| p.get_mut(which)) {
                *slot = Some(hit);
            }
        }
        None
    }

    fn hit_at(
        ends: &[[Option<Hit>; 2]],
        needs: &[NodeNeed],
        index: usize,
        which: usize,
    ) -> Option<Hit> {
        if which == 2 {
            needs.get(index).map(|n| n.target)
        } else {
            ends.get(index)
                .and_then(|p| p.get(which).copied().flatten())
        }
    }

    /// Makes the stretches that met a removed part end on the node made on the road, with the connector as their new
    /// last piece, keeps the order of the ends of what was made here, and returns the connectors by the run that
    /// made them.
    fn join_to_road(
        &self,
        stretches: &mut [Stretch],
        joints: &[Joint],
        needs: &[NodeNeed],
    ) -> BTreeMap<usize, Vec<(String, Point)>> {
        let mut connectors: BTreeMap<usize, Vec<(String, Point)>> = BTreeMap::new();
        for joint in joints {
            let Some(old) = self
                .stretch(joint.stretch)
                .map(|s| if joint.at_end { s.end } else { s.start })
            else {
                continue;
            };
            let Some(need) = needs.iter().find(|n| n.node == old) else {
                continue;
            };
            for s in stretches
                .iter_mut()
                .filter(|s| s.start == old || s.end == old)
            {
                rewire(s, old, joint.node, joint.at);
                connectors
                    .entry(need.designated)
                    .or_default()
                    .push((s.id.clone(), joint.at));
            }
        }
        // a piece that was made here keeps the order of its ends
        let before: BTreeMap<&str, &Stretch> = self
            .graph
            .stretches
            .iter()
            .map(|s| (s.id.as_str(), s))
            .collect();
        for s in stretches.iter_mut() {
            if s.start > s.end
                && before
                    .get(s.id.as_str())
                    .is_none_or(|b| b.points != s.points)
            {
                std::mem::swap(&mut s.start, &mut s.end);
                s.points.reverse();
            }
        }
        connectors
    }

    /// Where the middle of run `k` is.
    fn middle_of(&self, k: usize) -> Point {
        self.runs
            .get(k)
            .and_then(|r| {
                self.stretch(r.stretch)
                    .map(|s| point_along(&s.points, f64::midpoint(r.from, r.to)))
            })
            .unwrap_or_default()
    }

    fn kept(&self, k: usize, reason: &'static str) -> Option<KeptSidepath> {
        let run = self.runs.get(k)?;
        Some(KeptSidepath {
            stretch: self.stretch(run.stretch)?.id.clone(),
            road: self.stretch(run.road)?.id.clone(),
            reason,
            at: self.middle_of(k),
        })
    }
}

/// The component of every node of the graph (the lowest node of it).
fn node_components(graph: &RoadGraph) -> BTreeMap<i64, i64> {
    let mut parent: BTreeMap<i64, i64> = BTreeMap::new();
    for s in &graph.stretches {
        parent.entry(s.start).or_insert(s.start);
        parent.entry(s.end).or_insert(s.end);
        let (ra, rb) = (
            super::find(&mut parent, s.start),
            super::find(&mut parent, s.end),
        );
        if let Some(slot) = parent.get_mut(&ra.max(rb)) {
            *slot = ra.min(rb);
        }
    }
    let nodes: Vec<i64> = parent.keys().copied().collect();
    nodes
        .into_iter()
        .map(|n| (n, super::find(&mut parent, n)))
        .collect()
}

/// Whether `after` splits a component of `before`: the nodes of one component of `before` that are still nodes
/// of `after` do not all lie in one component of `after`. A component that goes whole does not count, and
/// does not make up for another that is split.
fn splits(before: &RoadGraph, after: &RoadGraph) -> bool {
    let (was, is) = (node_components(before), node_components(after));
    let mut found: BTreeMap<i64, i64> = BTreeMap::new();
    was.iter().any(|(node, component)| {
        is.get(node)
            .is_some_and(|now| *found.entry(*component).or_insert(*now) != *now)
    })
}

/// The removal of the groups in `wanted`, all at once if `splits` finds nothing cut off, else the groups one at a time:
/// the outcome and the groups that were removed.
fn accept(
    ctx: &Ctx,
    graph: &RoadGraph,
    wanted: &BTreeSet<usize>,
    splits: &dyn Fn(&RoadGraph, &RoadGraph) -> bool,
) -> (Outcome, BTreeSet<usize>) {
    let all = ctx.evaluate(wanted);
    if !splits(graph, &all.graph) {
        return (all, wanted.clone());
    }
    let mut accepted: BTreeSet<usize> = BTreeSet::new();
    for &g in wanted {
        let mut trial = accepted.clone();
        trial.insert(g);
        if !splits(graph, &ctx.evaluate(&trial).graph) {
            accepted = trial;
        }
    }
    (ctx.evaluate(&accepted), accepted)
}

/// One family: the graph with its sidepaths removed, and what was removed and what was kept.
fn one_family(
    graph: &RoadGraph,
    rank: u8,
    widths: &dyn Fn(Family) -> f64,
    cell: f64,
) -> (RoadGraph, Vec<DroppedSidepath>, Vec<KeptSidepath>) {
    let ctx = Ctx::new(graph, rank, widths, cell);
    let mut kept = Vec::new();
    let mut wanted: BTreeSet<usize> = BTreeSet::new();
    for (g, group) in ctx.groups.iter().enumerate() {
        match (group.candidate, group.disturbs, group.runs.first()) {
            (true, true, _) => {
                wanted.insert(g);
            }
            (true, false, Some(&k)) => kept.extend(ctx.kept(k, "clear")),
            _ => {}
        }
    }
    if wanted.is_empty() {
        return (graph.clone(), Vec::new(), kept);
    }
    let (outcome, accepted) = accept(&ctx, graph, &wanted, &splits);
    for &g in wanted.difference(&accepted) {
        kept.extend(
            ctx.groups
                .get(g)
                .and_then(|group| group.runs.first())
                .and_then(|&k| ctx.kept(k, "would_split")),
        );
    }
    for &(k, reason) in &outcome.blocked {
        kept.extend(ctx.kept(k, reason));
    }
    (outcome.graph, outcome.dropped, kept)
}

/// Step 1: see the module.
pub(super) fn drop_sidepaths(
    graph: &RoadGraph,
    widths: &dyn Fn(Family) -> f64,
    cell_m: f64,
) -> (RoadGraph, Vec<DroppedSidepath>, Vec<KeptSidepath>) {
    let mut current = sorted(graph);
    let mut dropped = Vec::new();
    let mut kept = Vec::new();
    for rank in 1..=LOWEST_RANK {
        let (next, d, k) = one_family(&current, rank, widths, cell_m);
        current = next;
        dropped.extend(d);
        kept.extend(k);
    }
    dropped.sort_by(|a, b| {
        a.stretch
            .cmp(&b.stretch)
            .then(a.from_m.total_cmp(&b.from_m))
    });
    kept.sort_by(|a, b| {
        a.stretch
            .cmp(&b.stretch)
            .then(a.at.0.total_cmp(&b.at.0))
            .then(a.at.1.total_cmp(&b.at.1))
    });
    if dropped.is_empty() {
        return (graph.clone(), dropped, kept);
    }
    (current, dropped, kept)
}

#[cfg(test)]
mod tests {
    //! The membership check is private; its fallback is not reached by the fixtures of the public tests.
    use super::*;

    fn graph(links: &[(i64, i64)]) -> RoadGraph {
        let stretches = links
            .iter()
            .map(|&(start, end)| Stretch {
                id: format!("{start}-{end}"),
                family: Family::Asf3,
                drivable: true,
                bridge: false,
                tunnel: false,
                roundabout: false,
                layer: 0,
                points: vec![(0.0, 0.0), (1.0, 0.0)],
                start,
                end,
                bridge_kind: None,
                bridge_offset: 0.0,
                use_sidepath: false,
            })
            .collect();
        rebuilt(stretches)
    }

    #[test]
    fn groups_are_tried_one_at_a_time_when_all_together_would_split() {
        let stretch = |id: &str, family, points: Vec<Point>, start, end| Stretch {
            id: id.to_string(),
            family,
            points,
            start,
            end,
            ..graph(&[(1, 2)]).stretches[0].clone()
        };
        let g = rebuilt(vec![
            stretch("1", Family::Asf3, vec![(0.0, 0.0), (100.0, 0.0)], 1, 2),
            stretch("2", Family::Grav, vec![(0.0, 5.0), (100.0, 5.0)], 3, 4),
            stretch("3", Family::Grav, vec![(0.0, -5.0), (100.0, -5.0)], 5, 6),
        ]);
        let ctx = Ctx::new(&g, 4, &|f| f.width_m(), 4.0);
        let wanted: BTreeSet<usize> = (0..ctx.groups.len()).collect();
        assert_eq!(wanted.len(), 2);
        let (all, accepted) = accept(&ctx, &g, &wanted, &|_, _| false);
        assert_eq!((accepted.len(), all.graph.stretches.len()), (2, 1));
        // a check that finds a split whenever stretch 2 is gone: only the other group goes
        let (outcome, accepted) = accept(&ctx, &g, &wanted, &|_, after| {
            after.stretches.iter().all(|s| s.id != "2")
        });
        assert_eq!(accepted.len(), 1);
        let ids: Vec<&str> = outcome
            .graph
            .stretches
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(ids, ["1", "2"]);
    }

    #[test]
    fn reach_is_measured_to_the_road_and_the_moved_connector_may_be_twice_as_long() {
        // only the public fixtures' runs lie within reach of the road, so the reach guard is tested here
        let g = rebuilt(vec![Stretch {
            points: vec![(0.0, 0.0), (100.0, 0.0)],
            ..graph(&[(1, 2)]).stretches[0].clone()
        }]);
        let ctx = Ctx::new(&g, LOWEST_RANK, &|f| f.width_m(), 4.0);
        let at = |x: f64| Hit {
            stretch: 0,
            along: x,
            at: (x, 0.0),
        };
        // 10 m to the road, 16 m to where the joint moved
        assert_eq!(
            ctx.joint_reason(&[], (50.0, 10.0), (50.0, 0.0), Some(at(62.5))),
            None
        );
        // 16 m to the road: out of reach, wherever the joint is
        assert_eq!(
            ctx.joint_reason(&[], (50.0, 16.0), (50.0, 0.0), Some(at(50.0))),
            Some("would_disconnect")
        );
        // a connector longer than twice the reach, or no place to join
        assert_eq!(
            ctx.joint_reason(&[], (50.0, 10.0), (50.0, 0.0), Some(at(80.0))),
            Some("would_disconnect")
        );
        assert_eq!(
            ctx.joint_reason(&[], (50.0, 10.0), (50.0, 0.0), None),
            Some("would_disconnect")
        );
    }

    #[test]
    fn a_component_that_is_cut_in_two_is_a_split_and_one_that_goes_whole_is_not() {
        let before = graph(&[(1, 2), (2, 3), (5, 6)]);
        assert!(splits(&before, &graph(&[(1, 2), (3, 4), (5, 6)])));
        // the component {5, 6} is gone, the other one is as it was
        assert!(!splits(&before, &graph(&[(1, 2), (2, 3)])));
        // a component that goes whole does not make up for another that is cut
        assert!(splits(&before, &graph(&[(1, 2), (3, 4)])));
        // nodes that went and new nodes are no split
        assert!(!splits(&before, &graph(&[(1, 2), (2, 9), (9, 10)])));
    }
}

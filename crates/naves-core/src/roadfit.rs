// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fits road pieces to a line: one line in, a sequence of vanilla pieces out.
//!
//! A beam search chooses the arcs and straights that follow the line (each state is the pose after a
//! number of pieces; progress along the line is measured in a window ahead of the last progress, so that a
//! hairpin does not jump across to the other leg). Then, for a line that has to end in a given pose, the
//! lengths of the straight runs are solved so that the end lands on it: with the arcs fixed, the end position is
//! linear in the run lengths.
//!
//! Solvers considered and not used (research section 3.1): MILP and CP-SAT. They want a dependency of
//! hundreds of megabytes for a problem that this search solves per stretch in milliseconds.
//!
//! Coordinates are map metres, headings radians clockwise from north.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{PI, TAU};

use crate::catalog::{PieceKind, RoadPiece};

/// A gap this small between the end of the fit and its goal counts as closed.
pub const GAP_CLOSED_M: f64 = 0.10;
/// A gap up to this is tolerated.
pub const GAP_TOLERATED_M: f64 = 0.50;
/// A heading error up to this is tolerated.
pub const KINK_TOLERATED_DEG: f64 = 1.0;

/// How many finished candidates are tried for closing on a fixed end.
const CLOSURE_CANDIDATES: usize = 300;
/// How many states are kept per bucket of progress.
const BEAM_WIDTH: usize = 24;
/// A state is dropped when it costs this much more than the best one in its bucket of progress.
const COST_MARGIN: f64 = 12.0;
/// Metres of progress per bucket.
const BUCKET_M: f64 = 5.0;
/// How far ahead of its start the first state looks for the line.
const WINDOW_M: f64 = 60.0;
/// A state is finished when its progress is this close to the end of the line.
const FINISH_M: f64 = 3.0;
/// Over the last metres of a line with a fixed end, the heading is pulled towards the goal's.
const GOAL_PULL_M: f64 = 30.0;
/// The shortest straight run that is not nothing.
const MIN_RUN_M: f64 = 6.25;
/// Straight piece lengths, longest first.
const STRAIGHTS: [f64; 3] = [25.0, 12.5, 6.25];

/// A position and a direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// East.
    pub x: f64,
    /// North.
    pub y: f64,
    /// Radians clockwise from north.
    pub heading: f64,
}

/// What the end of the fit has to meet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EndGoal {
    /// Nothing: the fit ends where the line ends.
    Free,
    /// A pose that is already fixed (a junction port, a piece already placed).
    Fixed(Pose),
}

/// One placed piece.
#[derive(Debug, Clone, PartialEq)]
pub struct FitPiece {
    /// Model path.
    pub model: String,
    /// Where the model's origin goes, with the rotation of the model.
    pub origin: Pose,
    /// Where the road enters the piece.
    pub start: Pose,
    /// Where the road leaves the piece.
    pub end: Pose,
}

/// The result of a fit.
#[derive(Debug, Clone, PartialEq)]
pub struct StretchFit {
    /// The pieces in order; a straight may overlap the one before it.
    pub pieces: Vec<FitPiece>,
    /// Distance between the end of the fit and a fixed goal; 0 for a free end.
    pub gap_m: f64,
    /// Difference between the heading at the end and the goal's; 0 for a free end.
    pub heading_error_deg: f64,
    /// Median distance of the pieces from the line.
    pub median_deviation_m: f64,
    /// Largest distance of the pieces from the line.
    pub max_deviation_m: f64,
}

/// Input that cannot be fitted at all.
#[derive(Debug, thiserror::Error)]
pub enum FitError {
    /// Fewer than two points.
    #[error("a line needs at least two points")]
    TooFewPoints,
    /// A coordinate is not a number.
    #[error("the line or the poses hold a value that is not a number")]
    NotFinite,
    /// No straight or arc to build with.
    #[error("there are no road pieces to fit with")]
    NoPieces,
}

fn wrap(a: f64) -> f64 {
    let r = a.rem_euclid(TAU);
    if r > PI { r - TAU } else { r }
}

/// Rotates a model vector clockwise by `theta`.
fn rotate(dx: f64, dz: f64, theta: f64) -> (f64, f64) {
    let (s, c) = theta.sin_cos();
    (dx * c + dz * s, -dx * s + dz * c)
}

/// A piece as it may be used: straight, or an arc in one of its two directions.
struct Variant {
    model: String,
    straight: bool,
    /// The port at which the road enters, in model coordinates.
    start: (f64, f64, f64),
    /// Vector from entry to exit in model coordinates.
    delta: (f64, f64),
    turn: f64,
    chord: f64,
}

fn variants(pieces: &[RoadPiece]) -> Vec<Variant> {
    let mut out = Vec::new();
    for piece in pieces {
        if piece.kind == PieceKind::EndCap {
            continue;
        }
        let straight = piece.kind == PieceKind::Straight;
        let mut forms = vec![piece.clone()];
        if !straight {
            forms.push(piece.reversed());
        }
        for p in forms {
            out.push(Variant {
                model: p.model.clone(),
                straight,
                start: (p.start.x, p.start.z, p.start.heading),
                delta: (p.end.x - p.start.x, p.end.z - p.start.z),
                turn: p.heading_change(),
                chord: p.chord(),
            });
        }
    }
    out
}

impl Variant {
    /// The pose after placing the variant with its entry at `at`.
    fn end_from(&self, at: Pose) -> Pose {
        let theta = at.heading - self.start.2;
        let (dx, dy) = rotate(self.delta.0, self.delta.1, theta);
        Pose {
            x: at.x + dx,
            y: at.y + dy,
            heading: wrap(at.heading + self.turn),
        }
    }

    fn place(&self, at: Pose) -> FitPiece {
        let theta = at.heading - self.start.2;
        let (ox, oy) = rotate(self.start.0, self.start.1, theta);
        FitPiece {
            model: self.model.clone(),
            origin: Pose {
                x: at.x - ox,
                y: at.y - oy,
                heading: wrap(theta),
            },
            start: at,
            end: self.end_from(at),
        }
    }
}

/// The line with cumulative lengths.
struct Line {
    points: Vec<(f64, f64)>,
    along: Vec<f64>,
    /// The bounding box `[min x, min y, max x, max y]` of each block of [`BLOCK`] segments.
    boxes: Vec<[f64; 4]>,
}

/// Segments per bounding box of [`Line`].
const BLOCK: usize = 16;
/// A block is passed over when its box is this much farther than a distance already found: far more than the
/// rounding of a distance of map coordinates, so that a passed-over block can never hold the nearest point.
const PRUNE_MARGIN_M: f64 = 1e-6;
/// A search over at least this many blocks first looks at the block whose box is nearest to the point.
const SEED_BLOCKS: usize = 8;

/// Where a point is on the line.
struct Near {
    distance: f64,
    s: f64,
    tangent: f64,
}

impl Line {
    fn new(points: &[(f64, f64)]) -> Line {
        let mut along = vec![0.0];
        let mut total = 0.0;
        for pair in points.windows(2) {
            if let [a, b] = pair {
                total += (b.0 - a.0).hypot(b.1 - a.1);
            }
            along.push(total);
        }
        let segments = points.len().saturating_sub(1);
        let boxes = (0..segments.div_ceil(BLOCK))
            .map(|b| {
                let first = b * BLOCK;
                let last = (first + BLOCK).min(segments);
                let mut bounds = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for p in points.get(first..=last).unwrap_or_default() {
                    bounds = [
                        bounds[0].min(p.0),
                        bounds[1].min(p.1),
                        bounds[2].max(p.0),
                        bounds[3].max(p.1),
                    ];
                }
                bounds
            })
            .collect();
        Line {
            points: points.to_vec(),
            along,
            boxes,
        }
    }

    fn length(&self) -> f64 {
        self.along.last().copied().unwrap_or(0.0)
    }

    /// The distance from `point` to the box of block `block`: a lower bound of the distance to its segments.
    fn box_distance(&self, block: usize, point: (f64, f64)) -> f64 {
        let Some(b) = self.boxes.get(block) else {
            return f64::INFINITY;
        };
        let dx = (b[0] - point.0).max(point.0 - b[2]).max(0.0);
        let dy = (b[1] - point.1).max(point.1 - b[3]).max(0.0);
        dx.hypot(dy)
    }

    /// The distance and arc length of the nearest point of segment `i`, if the segment lies in the window
    /// `[lo, hi]` of arc length and the point is not clearly farther than `cutoff` (then `None` as well: it
    /// cannot beat it).
    fn probe(
        &self,
        i: usize,
        point: (f64, f64),
        lo: f64,
        hi: f64,
        cutoff: f64,
    ) -> Option<(f64, f64)> {
        let (&s0, &s1) = (self.along.get(i)?, self.along.get(i + 1)?);
        if s1 < lo || s0 > hi {
            return None;
        }
        let (from, to) = (self.points.get(i)?, self.points.get(i + 1)?);
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let len2 = dx * dx + dy * dy;
        let along = if len2 > 0.0 {
            (((point.0 - from.0) * dx + (point.1 - from.1) * dy) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        // keep to the window
        let s = (s0 + along * (s1 - s0)).clamp(lo, hi);
        let along = if s1 > s0 { (s - s0) / (s1 - s0) } else { 0.0 };
        let q = (from.0 + along * dx, from.1 + along * dy);
        let (ex, ey) = (point.0 - q.0, point.1 - q.1);
        // the square root is the costly part: leave it out when the point is clearly no nearer
        if ex * ex + ey * ey > cutoff * cutoff * (1.0 + 1e-9) {
            return None;
        }
        Some((ex.hypot(ey), s))
    }

    /// The direction of segment `i`, as a heading.
    fn tangent(&self, i: usize) -> f64 {
        match (self.points.get(i), self.points.get(i + 1)) {
            (Some(from), Some(to)) => (to.0 - from.0).atan2(to.1 - from.1),
            _ => 0.0,
        }
    }

    /// The nearest point of the line among those whose arc length lies in `[lo, hi]`; of equal ones, the first.
    ///
    /// Looks only at the segments of the window (the arc lengths never decrease along the line) and passes
    /// over blocks of segments whose box is farther than a distance already found.
    ///
    /// Crate considered for the pruning: `rstar` (an R*-tree, 0.14.0, MIT or Apache-2.0, maintained by the georust
    /// group, many users). Not used: the query here is the nearest point of one polyline within a window of arc
    /// length that the segments' order already gives by binary search, the line is built once per placed part and
    /// searched about a hundred thousand times per piece, so a tree per line would be built for few queries, and the
    /// result must be bit for bit the old linear scan's (the first of equal distances, the same arithmetic per
    /// segment), which a tree's traversal order does not promise. 16 segments per box and a seed block are
    /// enough for the speed that is needed.
    fn nearest(&self, point: (f64, f64), lo: f64, hi: f64) -> Near {
        let mut best = Near {
            distance: f64::INFINITY,
            s: lo,
            tangent: 0.0,
        };
        let segments = self.points.len().saturating_sub(1);
        let first = self
            .along
            .get(1..)
            .map_or(0, |ends| ends.partition_point(|&s1| s1 < lo));
        let last = self.along.get(..segments).map_or(0, |starts| {
            starts.partition_point(|&s0| s0.partial_cmp(&hi) != Some(Ordering::Greater))
        });
        // a distance that some segment of the window has: blocks farther than it cannot hold the nearest point
        let mut seed = f64::INFINITY;
        if last.saturating_sub(first) >= SEED_BLOCKS * BLOCK {
            let nearest_block = (first / BLOCK..last.div_ceil(BLOCK))
                .map(|b| (self.box_distance(b, point), b))
                .min_by(|x, y| x.0.total_cmp(&y.0));
            if let Some((_, b)) = nearest_block {
                for i in (b * BLOCK).max(first)..((b + 1) * BLOCK).min(last) {
                    if let Some((d, _)) = self.probe(i, point, lo, hi, seed) {
                        seed = seed.min(d);
                    }
                }
            }
        }
        let mut i = first;
        while i < last {
            let block = i / BLOCK;
            let end = ((block + 1) * BLOCK).min(last);
            if self.box_distance(block, point) - PRUNE_MARGIN_M > best.distance.min(seed) {
                i = end;
                continue;
            }
            for j in i..end {
                if let Some((distance, s)) = self.probe(j, point, lo, hi, best.distance)
                    && distance < best.distance
                {
                    best = Near {
                        distance,
                        s,
                        tangent: self.tangent(j),
                    };
                }
            }
            i = end;
        }
        best
    }
}

#[derive(Clone, Copy)]
struct State {
    /// The last move in the arena, or `u32::MAX` for the start.
    node: u32,
    pose: Pose,
    s: f64,
    cost: f64,
}

struct Candidate {
    state: State,
    parent: u32,
    variant: u16,
    bucket: i64,
    key: (i64, i64, i64, i64),
}

fn bucket_of(s: f64) -> i64 {
    // progress is never negative
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a floor of a small positive number"
    )]
    let b = (s / BUCKET_M).floor() as i64;
    b
}

fn quantised(v: f64, step: f64) -> i64 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a rounded pose component, far inside i64"
    )]
    let q = (v / step).round() as i64;
    q
}

/// The sequence of variants (indices) from the search, and its cost.
struct Found {
    moves: Vec<u16>,
    cost: f64,
}

/// The states one piece further from `state`, for every variant.
fn expand(
    state: &State,
    line: &Line,
    goal: Option<Pose>,
    vars: &[Variant],
    out: &mut Vec<Candidate>,
) {
    let total = line.length();
    for (index, v) in vars.iter().enumerate() {
        let end = v.end_from(state.pose);
        // a piece cannot get further along the line than its own length (and some slack)
        let reach = (state.s + v.chord * 1.5 + 5.0).min(total);
        let from = (state.s - 1.0).max(0.0);
        let near = line.nearest((end.x, end.y), from, reach);
        let mid = line.nearest(
            (
                f64::midpoint(state.pose.x, end.x),
                f64::midpoint(state.pose.y, end.y),
            ),
            from,
            reach,
        );
        let s = state.s.max(near.s);
        if s < state.s + 0.05 && s < total - FINISH_M {
            continue;
        }
        let dh = wrap(end.heading - near.tangent);
        // towards the end of a line that has to meet a pose, the heading is pulled to the pose's
        let pull = goal.map_or(0.0, |g| {
            let weight = ((s - (total - GOAL_PULL_M)) / GOAL_PULL_M).clamp(0.0, 1.0);
            weight * 100.0 * wrap(end.heading - g.heading).powi(2)
        });
        let lateral = f64::midpoint(mid.distance.powi(2), near.distance.powi(2));
        let cost =
            state.cost + v.chord * (pull + lateral + 100.0 * dh * dh) + 5.0 + 20.0 * v.turn.abs();
        let Ok(variant) = u16::try_from(index) else {
            continue;
        };
        out.push(Candidate {
            state: State {
                node: 0,
                pose: end,
                s,
                cost,
            },
            parent: state.node,
            variant,
            bucket: bucket_of(s),
            key: (
                bucket_of(s),
                quantised(end.x, 0.5),
                quantised(end.y, 0.5),
                quantised(end.heading.to_degrees(), 1.0),
            ),
        });
    }
}

fn search(line: &Line, start: Pose, goal: Option<Pose>, vars: &[Variant]) -> Vec<Found> {
    let total = line.length();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a piece count guard from a length"
    )]
    let guard = (total / 2.0) as usize + 16;
    let mut arena: Vec<(u32, u16)> = Vec::new();
    let first = line.nearest((start.x, start.y), 0.0, WINDOW_M);
    let root = State {
        node: u32::MAX,
        pose: start,
        s: first.s,
        cost: 0.0,
    };
    let mut frontier = vec![root];
    let mut done: Vec<(State, f64)> = Vec::new();
    let mut best_partial = root;
    for _ in 0..guard {
        if frontier.is_empty() {
            break;
        }
        let mut candidates: Vec<Candidate> = Vec::new();
        for state in &frontier {
            expand(state, line, goal, vars, &mut candidates);
        }
        candidates.sort_by(|x, y| {
            x.state
                .cost
                .total_cmp(&y.state.cost)
                .then(x.parent.cmp(&y.parent))
                .then(x.variant.cmp(&y.variant))
        });
        let mut seen: BTreeSet<(i64, i64, i64, i64)> = BTreeSet::new();
        let mut per_bucket: BTreeMap<i64, (usize, f64)> = BTreeMap::new();
        frontier.clear();
        for c in candidates {
            if !seen.insert(c.key) {
                continue;
            }
            let (kept, best) = per_bucket.entry(c.bucket).or_insert((0, c.state.cost));
            if *kept >= BEAM_WIDTH || c.state.cost > *best + COST_MARGIN {
                continue;
            }
            *kept += 1;
            let Ok(node) = u32::try_from(arena.len()) else {
                continue;
            };
            arena.push((c.parent, c.variant));
            let state = State { node, ..c.state };
            if state.s > best_partial.s
                || (state.s >= best_partial.s && state.cost < best_partial.cost)
            {
                best_partial = state;
            }
            if state.s >= total - FINISH_M {
                done.push((state, state.cost));
            } else {
                frontier.push(state);
            }
        }
    }
    let rebuild = |state: &State| {
        let mut moves = Vec::new();
        let mut node = state.node;
        while let Some(&(parent, variant)) = arena.get(usize::try_from(node).unwrap_or(usize::MAX))
        {
            moves.push(variant);
            node = parent;
        }
        moves.reverse();
        moves
    };
    if done.is_empty() {
        return vec![Found {
            moves: rebuild(&best_partial),
            cost: best_partial.cost,
        }];
    }
    done.sort_by(|x, y| x.1.total_cmp(&y.1).then(x.0.node.cmp(&y.0.node)));
    done.iter()
        .take(CLOSURE_CANDIDATES)
        .map(|(state, cost)| Found {
            moves: rebuild(state),
            cost: *cost,
        })
        .collect()
}

/// An arc, or a straight run of a length.
enum Element {
    Arc(u16),
    Run(f64),
}

fn elements(found: &Found, vars: &[Variant]) -> Vec<Element> {
    let mut out: Vec<Element> = Vec::new();
    for &m in &found.moves {
        let Some(v) = vars.get(usize::from(m)) else {
            continue;
        };
        if v.straight {
            match out.last_mut() {
                Some(Element::Run(l)) => *l += v.chord,
                _ => out.push(Element::Run(v.chord)),
            }
        } else {
            out.push(Element::Arc(m));
        }
    }
    out
}

/// The pose at the start of each element and at the end, with the run lengths as they are.
fn walk(start: Pose, elems: &[Element], vars: &[Variant]) -> (Vec<Pose>, Pose) {
    let mut at = start;
    let mut starts = Vec::new();
    for e in elems {
        starts.push(at);
        match e {
            Element::Arc(i) => {
                if let Some(v) = vars.get(usize::from(*i)) {
                    at = v.end_from(at);
                }
            }
            Element::Run(l) => {
                at = Pose {
                    x: at.x + l * at.heading.sin(),
                    y: at.y + l * at.heading.cos(),
                    heading: at.heading,
                };
            }
        }
    }
    (starts, at)
}

/// Changes the lengths of the straight runs as little as possible so that the end lands on `goal`;
/// a run is either nothing or at least [`MIN_RUN_M`].
fn close(start: Pose, elems: &mut [Element], vars: &[Variant], goal: (f64, f64)) {
    for _ in 0..8 {
        let (starts, end) = walk(start, elems, vars);
        let want = (goal.0 - end.x, goal.1 - end.y);
        if want.0.hypot(want.1) < 1e-9 {
            return;
        }
        // directions of the runs that may still change
        let mut free: Vec<(usize, (f64, f64))> = Vec::new();
        for (i, (e, s)) in elems.iter().zip(&starts).enumerate() {
            if let Element::Run(_) = e {
                free.push((i, (s.heading.sin(), s.heading.cos())));
            }
        }
        let Some(delta) = min_norm(&free.iter().map(|f| f.1).collect::<Vec<_>>(), want) else {
            return;
        };
        let mut clipped = false;
        for ((i, _), d) in free.iter().zip(delta) {
            if let Some(Element::Run(l)) = elems.get_mut(*i) {
                let wanted = *l + d;
                let allowed = if wanted < MIN_RUN_M / 2.0 {
                    0.0
                } else {
                    wanted.max(MIN_RUN_M)
                };
                clipped |= (allowed - wanted).abs() > 1e-9;
                *l = allowed;
            }
        }
        if !clipped {
            return;
        }
    }
}

/// The smallest `δ` with `Σ δᵢ dᵢ = want`, or the closest that exists if the directions are parallel.
fn min_norm(dirs: &[(f64, f64)], want: (f64, f64)) -> Option<Vec<f64>> {
    let first = dirs.first()?;
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for d in dirs {
        sxx += d.0 * d.0;
        sxy += d.0 * d.1;
        syy += d.1 * d.1;
    }
    let det = sxx * syy - sxy * sxy;
    let trace = sxx + syy;
    if det.abs() > 1e-9 * trace * trace {
        let lambda = (
            (syy * want.0 - sxy * want.1) / det,
            (sxx * want.1 - sxy * want.0) / det,
        );
        Some(
            dirs.iter()
                .map(|d| d.0 * lambda.0 + d.1 * lambda.1)
                .collect(),
        )
    } else {
        // all runs point the same way (or opposite): only that component can be closed
        let unit = *first;
        let weight: f64 = dirs
            .iter()
            .map(|d| (d.0 * unit.0 + d.1 * unit.1).powi(2))
            .sum();
        let along = (unit.0 * want.0 + unit.1 * want.1) / weight;
        Some(
            dirs.iter()
                .map(|d| (d.0 * unit.0 + d.1 * unit.1) * along)
                .collect(),
        )
    }
}

/// The pieces for a straight run of `length` starting at `at`. The last piece ends exactly at the end of
/// the run and may overlap the one before it by less than 6.25 m; a run is covered with as few pieces as
/// that allows.
fn run_pieces(length: f64, at: Pose, straights: &[(String, Variant)]) -> Vec<FitPiece> {
    let eps = 1e-6;
    // (offset of the start along the run, index into the straights, longest first)
    let mut sizes: Vec<(f64, usize)> = Vec::new();
    let mut pos = 0.0;
    while length - pos > 25.0 + eps {
        sizes.push((pos, 0));
        pos += 25.0;
    }
    let rem = length - pos;
    if rem > eps {
        // quarter-lengths needed to cover what is left
        let units = (rem / 6.25 - eps).ceil().clamp(1.0, 4.0);
        let exact = (units * 6.25 - rem).abs() < eps;
        let alone = sizes.is_empty();
        let tail: &[usize] = match (f64_index(units), exact || !alone) {
            (1, _) => &[2],
            (2, true) => &[1],
            (2, false) => &[2, 2],
            (3, _) => &[1, 2],
            (_, true) => &[0],
            _ => &[1, 1],
        };
        let mut at_offset = pos;
        for (n, &i) in tail.iter().enumerate() {
            let size = STRAIGHTS.get(i).copied().unwrap_or(25.0);
            if n + 1 == tail.len() {
                // the last piece ends exactly at the end of the run
                at_offset = length - size;
            }
            sizes.push((at_offset, i));
            at_offset += size;
        }
    }
    let mut out = Vec::new();
    for (offset, i) in sizes {
        let Some((_, v)) = straights.get(i) else {
            continue;
        };
        let here = Pose {
            x: at.x + offset * at.heading.sin(),
            y: at.y + offset * at.heading.cos(),
            heading: at.heading,
        };
        out.push(v.place(here));
    }
    out
}

fn f64_index(v: f64) -> usize {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a small index stored as f64"
    )]
    let i = v as usize;
    i
}

/// Builds the pieces of a result from its elements.
fn assemble(
    start: Pose,
    elems: &[Element],
    vars: &[Variant],
    straights: &[(String, Variant)],
) -> (Vec<FitPiece>, Pose) {
    let mut at = start;
    let mut pieces = Vec::new();
    for e in elems {
        match e {
            Element::Arc(i) => {
                if let Some(v) = vars.get(usize::from(*i)) {
                    let p = v.place(at);
                    at = p.end;
                    pieces.push(p);
                }
            }
            Element::Run(l) => {
                if *l > 1e-9 {
                    pieces.extend(run_pieces(*l, at, straights));
                    at = Pose {
                        x: at.x + l * at.heading.sin(),
                        y: at.y + l * at.heading.cos(),
                        heading: at.heading,
                    };
                }
            }
        }
    }
    (pieces, at)
}

/// The three straight pieces, longest first, as variants. Falls back to the nearest lengths available.
fn straight_set(vars: &[Variant]) -> Vec<(String, Variant)> {
    STRAIGHTS
        .iter()
        .filter_map(|want| {
            vars.iter()
                .filter(|v| v.straight)
                .min_by(|a, b| (a.chord - want).abs().total_cmp(&(b.chord - want).abs()))
                .map(|v| {
                    (
                        v.model.clone(),
                        Variant {
                            model: v.model.clone(),
                            straight: true,
                            start: v.start,
                            delta: v.delta,
                            turn: v.turn,
                            chord: v.chord,
                        },
                    )
                })
        })
        .collect()
}

fn deviations(pieces: &[FitPiece], line: &Line) -> (f64, f64) {
    let mut d: Vec<f64> = Vec::new();
    for p in pieces {
        for q in [
            (p.start.x, p.start.y),
            (
                f64::midpoint(p.start.x, p.end.x),
                f64::midpoint(p.start.y, p.end.y),
            ),
            (p.end.x, p.end.y),
        ] {
            d.push(line.nearest(q, 0.0, line.length()).distance);
        }
    }
    d.sort_by(f64::total_cmp);
    let max = d.last().copied().unwrap_or(0.0);
    let median = d.get(d.len() / 2).copied().unwrap_or(0.0);
    (median, max)
}

/// Fits pieces to `path`, starting at `start`.
///
/// A poor fit is a result, not an error: look at [`StretchFit::gap_m`] and the deviations.
///
/// # Errors
/// [`FitError`] only for input that cannot be fitted at all.
pub fn fit_line(
    path: &[(f64, f64)],
    start: Pose,
    goal: EndGoal,
    pieces: &[RoadPiece],
) -> Result<StretchFit, FitError> {
    if path.len() < 2 {
        return Err(FitError::TooFewPoints);
    }
    let goal_pose = match goal {
        EndGoal::Fixed(p) => Some(p),
        EndGoal::Free => None,
    };
    let finite = |p: &Pose| p.x.is_finite() && p.y.is_finite() && p.heading.is_finite();
    if path.iter().any(|p| !p.0.is_finite() || !p.1.is_finite())
        || !finite(&start)
        || goal_pose.is_some_and(|g| !finite(&g))
    {
        return Err(FitError::NotFinite);
    }
    let vars = variants(pieces);
    let straights = straight_set(&vars);
    if straights.is_empty() || !vars.iter().any(|v| v.straight) {
        return Err(FitError::NoPieces);
    }
    let line = Line::new(path);
    let mut best: Option<(f64, Vec<Element>, Pose)> = None;
    for found in search(&line, start, goal_pose, &vars) {
        let mut elems = elements(&found, &vars);
        if let Some(g) = goal_pose {
            close(start, &mut elems, &vars, (g.x, g.y));
        }
        let (_, end) = walk(start, &elems, &vars);
        let (gap_m, heading_error_deg) = goal_pose.map_or((0.0, 0.0), |g| {
            (
                (end.x - g.x).hypot(end.y - g.y),
                wrap(end.heading - g.heading).abs().to_degrees(),
            )
        });
        let score = gap_m + 0.2 * heading_error_deg + 1e-3 * found.cost / line.length().max(1.0);
        if best.as_ref().is_none_or(|(s, _, _)| score < *s) {
            best = Some((score, elems, end));
        }
    }
    let (_, elems, end) = best.ok_or(FitError::NoPieces)?;
    let (pieces, _) = assemble(start, &elems, &vars, &straights);
    let (gap_m, heading_error_deg) = goal_pose.map_or((0.0, 0.0), |g| {
        (
            (end.x - g.x).hypot(end.y - g.y),
            wrap(end.heading - g.heading).abs().to_degrees(),
        )
    });
    let (median_deviation_m, max_deviation_m) = deviations(&pieces, &line);
    Ok(StretchFit {
        pieces,
        gap_m,
        heading_error_deg,
        median_deviation_m,
        max_deviation_m,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator, so that the tests need no input and no dependency.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 11
        }

        /// A number in `[0, 1)`.
        fn unit(&mut self) -> f64 {
            #[expect(clippy::cast_precision_loss, reason = "53 bits of a generator")]
            let v = self.next() as f64 / (1u64 << 53) as f64;
            v
        }

        fn below(&mut self, n: usize) -> usize {
            usize::try_from(self.next()).unwrap_or(0) % n.max(1)
        }
    }

    /// The scan over every segment that [`Line::nearest`] replaces.
    fn scan(line: &Line, point: (f64, f64), lo: f64, hi: f64) -> Near {
        let mut best = Near {
            distance: f64::INFINITY,
            s: lo,
            tangent: 0.0,
        };
        for (i, pair) in line.points.windows(2).enumerate() {
            let (Some(&s0), Some(&s1)) = (line.along.get(i), line.along.get(i + 1)) else {
                continue;
            };
            if s1 < lo || s0 > hi {
                continue;
            }
            let [from, to] = pair else { continue };
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            let len2 = dx * dx + dy * dy;
            let along = if len2 > 0.0 {
                (((point.0 - from.0) * dx + (point.1 - from.1) * dy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let s = (s0 + along * (s1 - s0)).clamp(lo, hi);
            let along = if s1 > s0 { (s - s0) / (s1 - s0) } else { 0.0 };
            let q = (from.0 + along * dx, from.1 + along * dy);
            let distance = (point.0 - q.0).hypot(point.1 - q.1);
            if distance < best.distance {
                best = Near {
                    distance,
                    s,
                    tangent: dx.atan2(dy),
                };
            }
        }
        best
    }

    fn same(a: &Near, b: &Near) -> bool {
        a.distance.to_bits() == b.distance.to_bits()
            && a.s.to_bits() == b.s.to_bits()
            && a.tangent.to_bits() == b.tangent.to_bits()
    }

    /// A random walk on a coarse grid (so that points repeat and distances tie) or on a fine one.
    fn walk_line(rng: &mut Lcg, points: usize, coarse: bool) -> Vec<(f64, f64)> {
        let (mut x, mut y) = (0.0_f64, 0.0_f64);
        let mut out = Vec::with_capacity(points);
        for _ in 0..points {
            if coarse {
                x += (f64::from(u8::try_from(rng.below(5)).unwrap_or(0)) - 2.0) * 2.0;
                y += (f64::from(u8::try_from(rng.below(5)).unwrap_or(0)) - 2.0) * 2.0;
            } else {
                x += rng.unit() * 8.0 - 3.0;
                y += rng.unit() * 8.0 - 3.0;
            }
            out.push((x, y));
        }
        out
    }

    #[test]
    fn nearest_is_bitwise_the_scan() {
        let mut rng = Lcg(20_261_121);
        for round in 0..400 {
            let count = [0, 1, 2, 3, 17, 33, 100, 300, 700][round % 9];
            let points = walk_line(&mut rng, count, round % 2 == 0);
            let line = Line::new(&points);
            let total = line.length();
            for _ in 0..40 {
                let point = if rng.below(3) == 0 {
                    // exactly on a vertex, or on the grid
                    points
                        .get(rng.below(points.len()))
                        .copied()
                        .unwrap_or((1.0, 1.0))
                } else {
                    (rng.unit() * 200.0 - 50.0, rng.unit() * 200.0 - 50.0)
                };
                let vertex = line
                    .along
                    .get(rng.below(line.along.len()))
                    .copied()
                    .unwrap_or(0.0);
                let windows = [
                    (0.0, total),
                    (0.0, WINDOW_M),
                    (rng.unit() * total, rng.unit() * total),
                    (rng.unit() * total, total + 10.0),
                    (-5.0, rng.unit() * total),
                    (vertex, vertex),
                    (vertex, vertex + 30.0),
                    (vertex - 30.0, vertex),
                    (total + 1.0, total + 5.0),
                    (-9.0, -1.0),
                    (total, total),
                    (0.0, 0.0),
                ];
                for (lo, hi) in windows {
                    // `clamp` (also in the scan) panics for `lo > hi`; no caller passes one
                    let (lo, hi) = if lo > hi { (hi, lo) } else { (lo, hi) };
                    let (new, old) = (line.nearest(point, lo, hi), scan(&line, point, lo, hi));
                    assert!(same(&new, &old), "round {round} window {lo} {hi}");
                }
            }
        }
    }

    #[test]
    fn nearest_keeps_the_first_of_equal_segments() {
        // the same square traced twice: a point at its centre is equally far from all eight sides
        let square = [
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (0.0, 0.0),
        ];
        let mut points: Vec<(f64, f64)> = Vec::new();
        for _ in 0..40 {
            points.extend_from_slice(&square);
        }
        let line = Line::new(&points);
        let (new, old) = (
            line.nearest((5.0, 5.0), 0.0, line.length()),
            scan(&line, (5.0, 5.0), 0.0, line.length()),
        );
        assert!(same(&new, &old));
        assert!((new.s - 5.0).abs() < 1e-12);
    }
}

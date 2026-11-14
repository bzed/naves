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
}

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
        Line {
            points: points.to_vec(),
            along,
        }
    }

    fn length(&self) -> f64 {
        self.along.last().copied().unwrap_or(0.0)
    }

    /// The nearest point of the line among those whose arc length lies in `[lo, hi]`.
    fn nearest(&self, point: (f64, f64), lo: f64, hi: f64) -> Near {
        let mut best = Near {
            distance: f64::INFINITY,
            s: lo,
            tangent: 0.0,
        };
        for (i, pair) in self.points.windows(2).enumerate() {
            let (Some(&s0), Some(&s1)) = (self.along.get(i), self.along.get(i + 1)) else {
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
            // keep to the window
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

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The heights of the whole road network as one quadratic program: phase A of
//! `docs/superpowers/specs/2026-11-16-height-solve-design.md` (no clearance yet), with the rows (N) of roads that share terrain cells.
//!
//! Every variable of every chain (see [`Chain`]) has a height that is an affine expression ([`Expr`]) in the
//! unknowns: its own unknown (a free variable), the plane of the junction that holds it, the plane of a ring,
//! or the height of the road its end lies on. The unknowns are numbered, in this order: the free heights,
//! chain by chain in placement order and variable by variable; then one height per junction that holds a
//! variable (not on a ring), in the order of [`Junctions::points`]; then one tilt per such junction with a
//! through road, in the same order; then the slacks, in the order their rows are made. Each height and tilt
//! unknown is the offset from its target (the ground, the junction's ground, the tilt of the ground), which
//! keeps the numbers near zero.

use std::collections::{BTreeMap, BTreeSet};

use crate::heightmap::Heightmap;
use crate::junctions::{Junction, Junctions, RingPlane};
use crate::limits::{grade_aim, grade_ceiling, min_vertical_radius};
use crate::qp::{Problem, QpError, Row};
use crate::roadplace::{PlacedKind, PlacedNetwork};

use super::{
    BRIDGE_CLEARANCE_M, CROSSING_REACH_M, Chain, ChainEnds, GradeExcess, GradeExcessKind,
    MIN_STENCIL_M, ProfileError, Profiles, REFERENCE_M, RoadStrip, SMOOTHNESS, chain_of, end_joins,
    holds, joint_heights, meeting_bounds, ring_of, road_on_stretch, segment_grades, strip_of,
};

mod nearby;

pub(crate) use nearby::Apart;

/// Weight of a junction's height against the ground's.
const JUNCTION_GROUND: f64 = 2.0;
/// Weight of a junction's tilt against the ground's.
const JUNCTION_TILT: f64 = 2.0;
/// The lever, in metres, that turns a difference of tilt into a height in the cost.
const TILT_LEVER_M: f64 = 25.0;
/// Weight of the grade above the aim.
const AIM: f64 = 1.0;
/// Weight of the slacks of the rows that are kept unless nothing else is possible.
const HARD: f64 = 1.0e4;
/// How far over a limit a grade or a change of grade may be before it is a finding.
const EXCESS_TOLERANCE: f64 = 1.0e-4;

/// An affine expression `Σ c_k x_k + constant` in the unknowns.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Expr {
    /// The terms `(unknown, coefficient)`, each unknown once, in the order of the unknowns.
    pub terms: Vec<(usize, f64)>,
    /// The constant.
    pub constant: f64,
}

impl Expr {
    fn constant(constant: f64) -> Expr {
        Expr {
            terms: Vec::new(),
            constant,
        }
    }

    /// `x_unknown + offset`.
    fn unknown(unknown: usize, offset: f64) -> Expr {
        Expr {
            terms: vec![(unknown, 1.0)],
            constant: offset,
        }
    }

    /// `Σ factor · part`, each unknown once.
    fn sum(parts: &[(&Expr, f64)]) -> Expr {
        let mut terms: BTreeMap<usize, f64> = BTreeMap::new();
        let mut constant = 0.0;
        for (part, factor) in parts {
            for &(k, c) in &part.terms {
                *terms.entry(k).or_insert(0.0) += factor * c;
            }
            constant += factor * part.constant;
        }
        Expr {
            terms: terms.into_iter().filter(|(_, c)| *c != 0.0).collect(),
            constant,
        }
    }

    fn value(&self, x: &[f64]) -> f64 {
        self.terms
            .iter()
            .map(|&(k, c)| c * x.get(k).copied().unwrap_or(0.0))
            .sum::<f64>()
            + self.constant
    }

    /// The terms of the expression with `(unknown, coefficient)` added.
    fn terms_with(&self, unknown: usize, coefficient: f64) -> Vec<(usize, f64)> {
        let mut terms = self.terms.clone();
        terms.push((unknown, coefficient));
        terms
    }
}

/// Where the height of a variable comes from, before the unknowns are numbered.
#[derive(Debug, Clone, Copy)]
enum Source {
    /// A height that is given (a ring's plane).
    Constant(f64),
    /// The plane of junction `junction`: its height, its tilt times `lever`, less `drop`.
    Held {
        junction: usize,
        lever: f64,
        drop: f64,
    },
    /// Linear between the variables `from` and `to` of the chain `owner`, `t` of the way.
    Meets {
        owner: usize,
        from: usize,
        to: usize,
        t: f64,
    },
    /// Its own unknown.
    Free,
}

/// One chain of the model.
pub(crate) struct ModelChain {
    /// The roads of the chain, as indices into [`PlacedNetwork::roads`].
    roads: Vec<usize>,
    /// The index of its first road.
    first: usize,
    chain: Chain,
    /// The height of each variable.
    heights: Vec<Expr>,
    /// The junction that holds each variable.
    holders: Vec<Option<usize>>,
    /// The grade ceiling of each segment.
    ceiling: Vec<f64>,
    /// The smallest vertical radius.
    radius: f64,
    /// The slack `f` of the grade rows of each segment that has them.
    over_ceiling: Vec<Option<usize>>,
}

/// The unknowns of a junction: its height and, with a through road, its tilt.
#[derive(Debug, Clone, Copy)]
struct JunctionUnknowns {
    height: usize,
    tilt: Option<usize>,
    /// The tilt of the ground along the through road, what the tilt unknown is measured from.
    ground_tilt: f64,
}

/// The quadratic program of the heights of a network, and what is needed to read its solution.
pub(crate) struct Model {
    pub(crate) chains: Vec<ModelChain>,
    junctions: Vec<Option<JunctionUnknowns>>,
    pub(crate) problem: Problem,
}

fn invalid(what: String) -> ProfileError {
    ProfileError::Invalid { what }
}

/// The ring whose plane a junction lies on.
fn ring_of_junction<'a>(junctions: &'a Junctions, junction: &Junction) -> Option<&'a RingPlane> {
    junctions
        .rings
        .iter()
        .find(|ring| !ring.stretches.is_disjoint(&junction.stretches))
}

/// The run of segment `i`, at least [`MIN_STENCIL_M`].
fn run_of(chain: &Chain, i: usize) -> f64 {
    chain
        .spacing
        .get(i)
        .copied()
        .unwrap_or(REFERENCE_M)
        .max(MIN_STENCIL_M)
}

/// The length of the deck that variable `v` stands for: along its joints.
fn deck_length(chain: &Chain, v: usize) -> f64 {
    chain
        .joints
        .windows(2)
        .filter_map(|pair| match pair {
            [a, b] if a.2 == v && b.2 == v => Some((b.0 - a.0).hypot(b.1 - a.1)),
            _ => None,
        })
        .sum()
}

/// The grade limits of each segment of a chain: [`segment_grades`] gives one per road that is not a bridge,
/// and the segment a road ends is the one between the variable before it and the variable it opens.
fn limits_per_segment(
    chain: &Chain,
    roads: &[usize],
    net: &PlacedNetwork,
    per_road: &[f64],
    base: f64,
) -> Vec<f64> {
    let mut out = vec![base; chain.spacing.len()];
    let mut counted = 0;
    for (k, road) in roads.iter().enumerate() {
        if net
            .roads
            .get(*road)
            .is_none_or(|r| r.kind == PlacedKind::Bridge)
        {
            continue;
        }
        let limit = per_road.get(counted).copied().unwrap_or(base);
        counted += 1;
        if let (Some(from), Some(to)) = (chain.joints.get(k), chain.joints.get(k + 1))
            && to.2 != from.2
            && let Some(slot) = to.2.checked_sub(1).and_then(|s| out.get_mut(s))
        {
            *slot = limit;
        }
    }
    out
}

/// What pass one finds for a chain: everything except the numbered unknowns.
struct Draft {
    roads: Vec<usize>,
    first: usize,
    chain: Chain,
    ring: bool,
    sources: Vec<Source>,
    holders: Vec<Option<usize>>,
    ground: Vec<f64>,
    aim: Vec<f64>,
    ceiling: Vec<f64>,
    radius: f64,
}

/// The height of a ring's plane at `at`; an error that says where if it is not a number.
fn ring_height(plane: &RingPlane, at: (f64, f64)) -> Result<f64, ProfileError> {
    let height = plane.height_at(at);
    if height.is_finite() {
        Ok(height)
    } else {
        Err(invalid(format!(
            "the plane of the ring at ({:.1}, {:.1}) has no height at ({:.1}, {:.1})",
            plane.at.0, plane.at.1, at.0, at.1
        )))
    }
}

/// Sets the source of every variable that a junction holds: the junction's plane, or a constant where the
/// junction lies on a ring.
fn hold_sources(
    sources: &mut [Source],
    holds: &[Option<holds::Hold>],
    chain: &Chain,
    junctions: &Junctions,
    ids: &BTreeSet<String>,
) -> Result<(), ProfileError> {
    for ((slot, hold), at) in sources.iter_mut().zip(holds).zip(&chain.at) {
        let Some(hold) = hold else { continue };
        let Some(j) = junctions.points.get(hold.junction) else {
            continue;
        };
        let drop = holds::overlay_drop(j, ids);
        *slot = match ring_of_junction(junctions, j) {
            Some(plane) => Source::Constant(ring_height(plane, *at)? - drop),
            None => Source::Held {
                junction: hold.junction,
                lever: j
                    .tilt_axis
                    .map_or(0.0, |u| u.0 * (at.0 - j.at.0) + u.1 * (at.1 - j.at.1)),
                drop,
            },
        };
    }
    Ok(())
}

/// The ground target of each variable of a chain; a deck is held to the ground at its abutments, not to the
/// valley floor under it.
fn ground_targets(chain: &Chain, hm: &Heightmap) -> Result<Vec<f64>, ProfileError> {
    let ground_at = |p: (f64, f64)| f64::from(hm.sample(p.0, p.1).height);
    chain
        .at
        .iter()
        .zip(&chain.deck)
        .map(|(at, deck)| {
            let target = match deck {
                Some((from, to)) => f64::midpoint(ground_at(*from), ground_at(*to)),
                None => ground_at(*at),
            };
            if target.is_finite() {
                Ok(target)
            } else {
                Err(invalid(format!(
                    "no ground data under the road at ({:.1}, {:.1})",
                    at.0, at.1
                )))
            }
        })
        .collect()
}

/// Pass one: the chains, which junction holds which variable, the meetings, the targets and the limits.
fn drafts(
    net: &PlacedNetwork,
    hm: &Heightmap,
    junctions: &Junctions,
) -> Result<Vec<Draft>, ProfileError> {
    let cell = f64::from(hm.cell_m);
    let ends = ChainEnds::new(net);
    let mut strips: Vec<RoadStrip> = Vec::new();
    let mut bounds: Vec<[f64; 4]> = Vec::new();
    let mut drafts: Vec<Draft> = Vec::new();
    for roads in &net.chains {
        let Some(chain) = chain_of(net, roads, cell, junctions) else {
            continue;
        };
        let Some((first, road)) = roads.first().and_then(|i| Some((*i, net.roads.get(*i)?))) else {
            continue;
        };
        let family = road.family;
        let count = chain.at.len();
        let holds =
            holds::junction_heights(net, &chain, roads, road.width / 2.0, cell, junctions, true);
        let flags: Vec<Option<f64>> = holds.iter().map(|h| h.map(|h| h.height)).collect();
        let (_, joins) = end_joins(&chain, &strips, &bounds, cell, &flags);
        let ring = ring_of(net, roads, junctions);
        let mut sources = vec![Source::Free; count];
        // a meeting found later for the same variable wins, as it does today
        for (var, meet) in &joins {
            let owner = drafts.get(meet.owner);
            let variable =
                |joint: usize| owner.and_then(|o| o.chain.joints.get(joint)).map(|j| j.2);
            if let (Some(slot), Some(from), Some(to)) = (
                sources.get_mut(*var),
                variable(meet.segment),
                variable(meet.segment + 1),
            ) {
                *slot = Source::Meets {
                    owner: meet.owner,
                    from,
                    to,
                    t: meet.t,
                };
            }
        }
        // a junction beats a meeting
        hold_sources(
            &mut sources,
            &holds,
            &chain,
            junctions,
            &holds::stretch_ids(net, roads),
        )?;
        if let Some(plane) = ring {
            for (slot, at) in sources.iter_mut().zip(&chain.at) {
                *slot = Source::Constant(ring_height(plane, *at)?);
            }
        }
        let holders: Vec<Option<usize>> = holds.iter().map(|h| h.map(|h| h.junction)).collect();
        // the limits: a ring is held everywhere (as today), so no meeting grade lowers it
        let held: Vec<Option<f64>> = if ring.is_some() {
            vec![Some(0.0); count]
        } else {
            flags
        };
        let (aim_base, ceiling_base) = (grade_aim(family), grade_ceiling(family));
        let aim = limits_per_segment(
            &chain,
            roads,
            net,
            &segment_grades(net, &ends, &chain, roads, aim_base, &held),
            aim_base,
        );
        let ceiling = limits_per_segment(
            &chain,
            roads,
            net,
            &segment_grades(net, &ends, &chain, roads, ceiling_base, &held),
            ceiling_base,
        );
        let ground = if ring.is_some() {
            Vec::new()
        } else {
            ground_targets(&chain, hm)?
        };
        strips.push(strip_of(net, roads, joint_heights(&chain, &[]), &[], road));
        if let Some(strip) = strips.last() {
            bounds.push(meeting_bounds(strip));
        }
        drafts.push(Draft {
            roads: roads.clone(),
            first,
            chain,
            ring: ring.is_some(),
            sources,
            holders,
            ground,
            aim,
            ceiling,
            radius: min_vertical_radius(family),
        });
    }
    Ok(drafts)
}

/// Counts unknowns and adds the slack rows and costs.
struct Builder {
    problem: Problem,
    next: usize,
}

impl Builder {
    fn unknown(&mut self) -> usize {
        let k = self.next;
        self.next += 1;
        k
    }

    fn row(&mut self, terms: Vec<(usize, f64)>, lo: f64, hi: f64) {
        self.problem.rows.push(Row { terms, lo, hi });
    }

    /// `−(limit + e) ≤ d ≤ limit + e` with `e ≥ 0`; returns `e`.
    fn band(&mut self, d: &Expr, limit: f64) -> usize {
        let e = self.unknown();
        self.row(d.terms_with(e, -1.0), f64::NEG_INFINITY, limit - d.constant);
        self.row(d.terms_with(e, 1.0), -limit - d.constant, f64::INFINITY);
        self.row(vec![(e, 1.0)], 0.0, f64::INFINITY);
        e
    }

    /// The rows (G) of a segment whose height difference is `d` over `run` metres: returns `f`.
    fn grade(&mut self, d: &Expr, run: f64, aim: f64, ceiling: f64) -> usize {
        let e = self.band(d, aim * run);
        let f = self.unknown();
        self.row(
            vec![(e, 1.0), (f, -1.0)],
            f64::NEG_INFINITY,
            (ceiling - aim).max(0.0) * run,
        );
        self.row(vec![(f, 1.0)], 0.0, f64::INFINITY);
        self.problem.add_square(AIM, &[(e, 1.0)], 0.0);
        self.problem.add_linear(HARD, f);
        self.problem.add_square(HARD, &[(f, 1.0)], 0.0);
        f
    }

    /// A row (B): the change of grade `change` within `limit`.
    fn bend(&mut self, change: &Expr, limit: f64) {
        if change.terms.is_empty() {
            return;
        }
        let b = self.band(change, limit);
        self.problem.add_linear(HARD * REFERENCE_M, b);
        self.problem.add_square(HARD, &[(b, REFERENCE_M)], 0.0);
    }
}

/// The change of grade at each bend of a chain: `(variable, where, change, run)`; a deck variable has one per
/// deck end, between the grade of its approach and the level deck.
fn bends_of(
    chain: &Chain,
    grade: &dyn Fn(usize) -> Option<Expr>,
) -> Vec<(usize, (f64, f64), Expr, f64)> {
    let count = chain.at.len();
    let mut out = Vec::new();
    for i in 0..count {
        let at = chain.at.get(i).copied().unwrap_or_default();
        if let Some((start, end)) = chain.deck.get(i).copied().flatten() {
            let length = deck_length(chain, i);
            if let Some(before) = i.checked_sub(1).and_then(grade) {
                let run = f64::midpoint(run_of(chain, i - 1), length);
                out.push((i, start, Expr::sum(&[(&before, -1.0)]), run));
            }
            if let Some(after) = grade(i) {
                let run = f64::midpoint(run_of(chain, i), length);
                out.push((i, end, after, run));
            }
        } else if let (Some(before), Some(after)) = (i.checked_sub(1).and_then(grade), grade(i)) {
            let run = f64::midpoint(run_of(chain, i - 1), run_of(chain, i));
            out.push((i, at, Expr::sum(&[(&after, 1.0), (&before, -1.0)]), run));
        }
    }
    out
}

/// The grade of segment `i` of a chain whose variables have the heights `heights`.
fn grade_expr(chain: &Chain, heights: &[Expr], i: usize) -> Option<Expr> {
    let (from, to) = (heights.get(i)?, heights.get(i + 1)?);
    let run = run_of(chain, i);
    Some(Expr::sum(&[(to, 1.0 / run), (from, -1.0 / run)]))
}

/// The height unknown of every junction that holds a variable (not on a ring), then the tilt unknown of each
/// of them that has a through road.
fn junction_unknowns(
    builder: &mut Builder,
    drafts: &[Draft],
    junctions: &Junctions,
) -> Result<Vec<Option<JunctionUnknowns>>, ProfileError> {
    let holding: BTreeSet<usize> = drafts
        .iter()
        .flat_map(|d| &d.sources)
        .filter_map(|s| match s {
            Source::Held { junction, .. } => Some(*junction),
            _ => None,
        })
        .collect();
    let mut unknowns: Vec<Option<JunctionUnknowns>> = vec![None; junctions.points.len()];
    for &j in &holding {
        let Some(point) = junctions.points.get(j) else {
            continue;
        };
        if !point.ground.is_finite() {
            return Err(invalid(format!(
                "the junction at ({:.1}, {:.1}) has no ground height",
                point.at.0, point.at.1
            )));
        }
        let height = builder.unknown();
        if let Some(slot) = unknowns.get_mut(j) {
            *slot = Some(JunctionUnknowns {
                height,
                tilt: None,
                ground_tilt: 0.0,
            });
        }
    }
    for (j, slot) in unknowns.iter_mut().enumerate() {
        let (Some(unknown), Some(point)) = (slot.as_mut(), junctions.points.get(j)) else {
            continue;
        };
        let Some(axis) = point.tilt_axis else {
            continue;
        };
        let ground_tilt = point.gradient.0 * axis.0 + point.gradient.1 * axis.1;
        if !ground_tilt.is_finite() || !point.tilt_cap.is_finite() || point.tilt_cap < 0.0 {
            return Err(invalid(format!(
                "the junction at ({:.1}, {:.1}) has a tilt of {ground_tilt} and a cap of {}",
                point.at.0, point.at.1, point.tilt_cap
            )));
        }
        unknown.tilt = Some(builder.unknown());
        unknown.ground_tilt = ground_tilt;
    }
    Ok(unknowns)
}

/// The height of every variable, chain by chain: the owner of a meeting comes before the chain that meets it.
fn height_exprs(
    drafts: &[Draft],
    free: &[Vec<Option<usize>>],
    unknowns: &[Option<JunctionUnknowns>],
    junctions: &Junctions,
) -> Result<Vec<Vec<Expr>>, ProfileError> {
    let mut heights: Vec<Vec<Expr>> = Vec::with_capacity(drafts.len());
    for (draft, free) in drafts.iter().zip(free) {
        let mut chain_heights = Vec::with_capacity(draft.sources.len());
        for (v, source) in draft.sources.iter().enumerate() {
            let expr = match *source {
                Source::Constant(height) => Expr::constant(height),
                Source::Free => match (free.get(v).copied().flatten(), draft.ground.get(v)) {
                    (Some(k), Some(ground)) => Expr::unknown(k, *ground),
                    _ => return Err(invalid(format!("variable {v} has no unknown"))),
                },
                Source::Held {
                    junction,
                    lever,
                    drop,
                } => {
                    let (Some(point), Some(u)) = (
                        junctions.points.get(junction),
                        unknowns.get(junction).copied().flatten(),
                    ) else {
                        return Err(invalid(format!("junction {junction} has no unknown")));
                    };
                    let mut expr = Expr::unknown(u.height, point.ground - drop);
                    if let Some(s) = u.tilt {
                        expr.terms.push((s, lever));
                        expr.constant += lever * u.ground_tilt;
                    }
                    expr
                }
                Source::Meets { owner, from, to, t } => {
                    let owner = heights.get(owner);
                    let (Some(a), Some(b)) = (
                        owner.and_then(|o: &Vec<Expr>| o.get(from)),
                        owner.and_then(|o: &Vec<Expr>| o.get(to)),
                    ) else {
                        return Err(invalid(format!("a meeting of variable {v} has no road")));
                    };
                    Expr::sum(&[(a, 1.0 - t), (b, t)])
                }
            };
            chain_heights.push(expr);
        }
        heights.push(chain_heights);
    }
    Ok(heights)
}

/// Builds the problem of the heights of `net` (see the module documentation).
pub(crate) fn build(
    net: &PlacedNetwork,
    hm: &Heightmap,
    junctions: &Junctions,
    apart: &[Apart],
) -> Result<Model, ProfileError> {
    let drafts = drafts(net, hm, junctions)?;
    let mut builder = Builder {
        problem: Problem::new(0),
        next: 0,
    };
    // the free heights, then the junction heights, then the junction tilts
    let mut free: Vec<Vec<Option<usize>>> = Vec::with_capacity(drafts.len());
    for draft in &drafts {
        free.push(
            draft
                .sources
                .iter()
                .map(|s| matches!(s, Source::Free).then(|| builder.unknown()))
                .collect(),
        );
    }
    let unknowns = junction_unknowns(&mut builder, &drafts, junctions)?;
    let heights = height_exprs(&drafts, &free, &unknowns, junctions)?;
    nearby::add(&mut builder, net, hm, &drafts, &heights, junctions, apart);
    // the cost and the rows of each chain
    let mut chains = Vec::with_capacity(drafts.len());
    for (draft, chain_heights) in drafts.into_iter().zip(heights) {
        let mut over_ceiling = vec![None; draft.chain.spacing.len()];
        if !draft.ring {
            add_chain(&mut builder, &draft, &chain_heights, &mut over_ceiling);
        }
        chains.push(ModelChain {
            roads: draft.roads,
            first: draft.first,
            chain: draft.chain,
            heights: chain_heights,
            holders: draft.holders,
            ceiling: draft.ceiling,
            radius: draft.radius,
            over_ceiling,
        });
    }
    // the junctions: their height and tilt near the ground's, the tilt within its cap (T)
    for (j, u) in unknowns.iter().enumerate() {
        let (Some(u), Some(point)) = (u, junctions.points.get(j)) else {
            continue;
        };
        builder
            .problem
            .add_square(JUNCTION_GROUND, &[(u.height, 1.0)], 0.0);
        if let Some(s) = u.tilt {
            builder
                .problem
                .add_square(JUNCTION_TILT, &[(s, TILT_LEVER_M)], 0.0);
            builder.row(
                vec![(s, 1.0)],
                -point.tilt_cap - u.ground_tilt,
                point.tilt_cap - u.ground_tilt,
            );
        }
    }
    builder.problem.unknowns = builder.next;
    Ok(Model {
        chains,
        junctions: unknowns,
        problem: builder.problem,
    })
}

/// The cost and the rows (G) and (B) of one chain that is not a ring.
fn add_chain(
    builder: &mut Builder,
    draft: &Draft,
    heights: &[Expr],
    over_ceiling: &mut [Option<usize>],
) {
    let chain = &draft.chain;
    // the ground
    for (h, ground) in heights.iter().zip(&draft.ground) {
        if !h.terms.is_empty() {
            builder
                .problem
                .add_square(1.0, &h.terms, h.constant - ground);
        }
    }
    // the smoothness: the change of grade per metre at every bend, scaled to REFERENCE_M. A bend is an interior
    // variable, or a deck end (between the grade of its approach and the level deck, with the run of the rows (B)):
    // one stencil across a deck would reward equal grades on both sides, the jump off one end the rows forbid
    let grade = |i: usize| grade_expr(chain, heights, i);
    let bends = bends_of(chain, &grade);
    for (_, _, change, run) in &bends {
        let kappa = Expr::sum(&[(change, REFERENCE_M * REFERENCE_M / run)]);
        if !kappa.terms.is_empty() {
            builder
                .problem
                .add_square(SMOOTHNESS, &kappa.terms, kappa.constant);
        }
    }
    // (G)
    for (i, slot) in over_ceiling.iter_mut().enumerate() {
        let (Some(from), Some(to)) = (heights.get(i), heights.get(i + 1)) else {
            continue;
        };
        let d = Expr::sum(&[(to, 1.0), (from, -1.0)]);
        if d.terms.is_empty() {
            continue;
        }
        let aim = draft.aim.get(i).copied().unwrap_or(0.0);
        let ceiling = draft.ceiling.get(i).copied().unwrap_or(aim);
        *slot = Some(builder.grade(&d, run_of(chain, i), aim, ceiling));
    }
    // (B)
    for (_, _, change, run) in &bends {
        builder.bend(change, run / draft.radius);
    }
}

/// Solves the model with `solver` and reads the profiles and the junctions from the solution.
pub(crate) fn solve(
    model: &Model,
    net: &PlacedNetwork,
    junctions: &Junctions,
    side: &[f64],
    solver: &dyn Fn(&Problem) -> Result<Vec<f64>, QpError>,
) -> Result<(Profiles, Junctions), ProfileError> {
    let x = solve_x(model, solver)?;
    read(model, net, junctions, side, &x)
}

/// The solution of the problem of `model` found by `solver`: one value per unknown.
pub(crate) fn solve_x(
    model: &Model,
    solver: &dyn Fn(&Problem) -> Result<Vec<f64>, QpError>,
) -> Result<Vec<f64>, ProfileError> {
    let x = solver(&model.problem).map_err(|e| match e {
        QpError::NotSolved { status } => ProfileError::NotSolved { status },
        QpError::Invalid { what } => ProfileError::Invalid { what },
    })?;
    if x.len() != model.problem.unknowns {
        return Err(invalid(format!(
            "the solver gave {} values for {} unknowns",
            x.len(),
            model.problem.unknowns
        )));
    }
    Ok(x)
}

/// The profiles and the junctions that the solution `x` of the problem of `model` stands for.
pub(crate) fn read(
    model: &Model,
    net: &PlacedNetwork,
    junctions: &Junctions,
    side: &[f64],
    x: &[f64],
) -> Result<(Profiles, Junctions), ProfileError> {
    let mut profiles = Profiles::default();
    for mc in &model.chains {
        let heights: Vec<f64> = mc.heights.iter().map(|h| h.value(x)).collect();
        if heights.iter().any(|h| !h.is_finite()) {
            return Err(ProfileError::NotSolved {
                status: "a height is not a number".to_string(),
            });
        }
        let Some(first) = net.roads.get(mc.first) else {
            continue;
        };
        profiles.excesses.extend(excesses_of(mc, &heights));
        profiles.strips.push(strip_of(
            net,
            &mc.roads,
            joint_heights(&mc.chain, &heights),
            side,
            first,
        ));
    }
    let mut out = junctions.clone();
    for (point, u) in out.points.iter_mut().zip(&model.junctions) {
        let Some(u) = u else { continue };
        point.height = point.ground + x.get(u.height).copied().unwrap_or(0.0);
        point.gradient = match (u.tilt, point.tilt_axis) {
            (Some(s), Some(axis)) => {
                let tilt = u.ground_tilt + x.get(s).copied().unwrap_or(0.0);
                (tilt * axis.0, tilt * axis.1)
            }
            _ => (0.0, 0.0),
        };
    }
    out.unresolved = unresolved(model, x, junctions);
    Ok((profiles, out))
}

/// The links of the junctions that hold an end of a segment over its ceiling (`f > 1e-4 · run`): the link
/// between the nearest held variables before and after the segment, when they are held to two junctions.
fn unresolved(model: &Model, x: &[f64], junctions: &Junctions) -> Vec<usize> {
    let mut pairs: BTreeSet<(usize, usize)> = BTreeSet::new();
    for mc in &model.chains {
        for (i, f) in mc.over_ceiling.iter().enumerate() {
            let Some(f) = f else { continue };
            let over = x.get(*f).copied().unwrap_or(0.0).max(0.0);
            if over <= EXCESS_TOLERANCE * run_of(&mc.chain, i) {
                continue;
            }
            let holder = |v: usize| mc.holders.get(v).copied().flatten();
            if holder(i).is_none() && holder(i + 1).is_none() {
                continue;
            }
            let before = (0..=i).rev().find_map(holder);
            let after = (i + 1..mc.holders.len()).find_map(holder);
            if let (Some(a), Some(b)) = (before, after)
                && a != b
            {
                pairs.insert((a.min(b), a.max(b)));
            }
        }
    }
    junctions
        .links
        .iter()
        .enumerate()
        .filter(|(_, link)| pairs.contains(&(link.a, link.b)))
        .map(|(i, _)| i)
        .collect()
}

/// Where a solved chain is over its limits: the grades over the ceiling of their segment and the changes of
/// grade over `run / R`, each by more than [`EXCESS_TOLERANCE`]; at a deck, per deck end.
fn excesses_of(mc: &ModelChain, heights: &[f64]) -> Vec<GradeExcess> {
    let chain = &mc.chain;
    let constants: Vec<Expr> = heights.iter().map(|h| Expr::constant(*h)).collect();
    let grade = |i: usize| grade_expr(chain, &constants, i);
    let mut excesses = Vec::new();
    for (_, at, change, run) in bends_of(chain, &grade) {
        let change = change.constant;
        if change.abs() > run / mc.radius + EXCESS_TOLERANCE {
            excesses.push(GradeExcess {
                at,
                kind: GradeExcessKind::VerticalBend,
                grade: change.abs() / run,
                limit: 1.0 / mc.radius,
            });
        }
    }
    for i in 0..chain.spacing.len() {
        let Some(g) = grade(i).map(|g| g.constant) else {
            continue;
        };
        let limit = mc.ceiling.get(i).copied().unwrap_or(f64::INFINITY);
        if g.abs() > limit + EXCESS_TOLERANCE {
            excesses.push(GradeExcess {
                at: chain.at.get(i).copied().unwrap_or_default(),
                kind: GradeExcessKind::Grade,
                grade: g,
                limit,
            });
        }
    }
    excesses
}

/// A bridge over a road whose deck thickness is known: what the rows (C) are made from.
pub(crate) struct Crossing {
    /// Where the lines of the two roads cross.
    pub at: (f64, f64),
    /// The id of the stretch of the bridge.
    pub upper: String,
    /// The id of the stretch of the road below.
    pub lower: String,
    /// The distance from the surface of the deck to its underside.
    pub thickness: f64,
}

/// A place on a chain: `t` of the way from its variable `from` to its variable `to`.
struct Place {
    chain: usize,
    from: usize,
    to: usize,
    t: f64,
}

/// The place of the stretch `id` nearest `at`, within the half width of its road and [`CROSSING_REACH_M`]: what
/// `surface_near` finds on the strips, so that the rows are made for what the clearance is measured on.
fn place_near(model: &Model, net: &PlacedNetwork, at: (f64, f64), id: &str) -> Option<Place> {
    let mut best: Option<(f64, Place)> = None;
    for (ci, mc) in model.chains.iter().enumerate() {
        let half = net.roads.get(mc.first).map_or(0.0, |r| r.width / 2.0);
        for (k, road) in mc.roads.iter().enumerate() {
            if !net.roads.get(*road).is_some_and(|r| road_on_stretch(r, id)) {
                continue;
            }
            let (Some(from), Some(to)) = (mc.chain.joints.get(k), mc.chain.joints.get(k + 1))
            else {
                continue;
            };
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            let length2 = dx * dx + dy * dy;
            let t = if length2 > 1e-12 {
                (((at.0 - from.0) * dx + (at.1 - from.1) * dy) / length2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let off = (at.0 - from.0 - t * dx).hypot(at.1 - from.1 - t * dy);
            if off <= half + CROSSING_REACH_M && best.as_ref().is_none_or(|b| off < b.0) {
                best = Some((
                    off,
                    Place {
                        chain: ci,
                        from: from.2,
                        to: to.2,
                        t,
                    },
                ));
            }
        }
    }
    best.map(|(_, place)| place)
}

/// The height of the surface at `place`.
fn height_at(model: &Model, place: &Place) -> Option<Expr> {
    let heights = &model.chains.get(place.chain)?.heights;
    Some(Expr::sum(&[
        (heights.get(place.from)?, 1.0 - place.t),
        (heights.get(place.to)?, place.t),
    ]))
}

/// Phase B: the rows (C) of every crossing in `crossings` that is placed, given the solution `x` of the problem
/// without them. Returns whether any were added (the problem has more unknowns and rows then).
///
/// The deck is held at least [`BRIDGE_CLEARANCE_M`] and its thickness above the surface of the road below as it
/// is in `solved`, less a slack that costs as the other hard ones; the two variables of the road below that
/// bracket the crossing are held to their height in `solved` exactly (a variable that is a constant has no
/// unknown and no row; a variable two crossings share has one row). Phase B stays feasible: the solution of
/// phase A satisfies every row of it, with its slacks, and the deck's slack has room.
pub(crate) fn add_clearances(
    model: &mut Model,
    net: &PlacedNetwork,
    solved: &[f64],
    crossings: &[Crossing],
) -> bool {
    let mut builder = Builder {
        next: model.problem.unknowns,
        problem: std::mem::replace(&mut model.problem, Problem::new(0)),
    };
    // the pins already made, by their terms: variables held by one junction in the same way are one row
    let mut pinned: BTreeSet<Vec<(usize, u64)>> = BTreeSet::new();
    let mut added = false;
    for crossing in crossings {
        let (Some(upper), Some(lower)) = (
            place_near(model, net, crossing.at, &crossing.upper),
            place_near(model, net, crossing.at, &crossing.lower),
        ) else {
            continue;
        };
        let (Some(deck), Some(low)) = (height_at(model, &upper), height_at(model, &lower)) else {
            continue;
        };
        let wanted = low.value(solved) + BRIDGE_CLEARANCE_M + crossing.thickness;
        let short = builder.unknown();
        builder.row(
            deck.terms_with(short, 1.0),
            wanted - deck.constant,
            f64::INFINITY,
        );
        builder.row(vec![(short, 1.0)], 0.0, f64::INFINITY);
        builder.problem.add_linear(HARD, short);
        builder.problem.add_square(HARD, &[(short, 1.0)], 0.0);
        for var in [lower.from, lower.to] {
            let Some(height) = model
                .chains
                .get(lower.chain)
                .and_then(|mc| mc.heights.get(var))
                .filter(|height| !height.terms.is_empty())
            else {
                continue;
            };
            let key = height
                .terms
                .iter()
                .map(|&(k, c)| (k, c.to_bits()))
                .collect();
            if !pinned.insert(key) {
                continue;
            }
            // held exactly: the whole shortfall is the slack of the deck
            let keep = height.value(solved) - height.constant;
            builder.row(height.terms.clone(), keep, keep);
        }
        added = true;
    }
    builder.problem.unknowns = builder.next;
    model.problem = builder.problem;
    added
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The rows (N) of `docs/superpowers/specs/2026-11-16-height-solve-design.md`: roads whose footprints share
//! terrain cells share a height, as far as the ground lets them.
//!
//! The spots every [`SAMPLE_M`] and the pairs of close spots are those of the old settling (`spots` and
//! `close_pairs`), found from the positions and the half widths of the roads alone. The pairs of two chains are
//! grouped into runs; a run is tied only if the original ground allows it at every pair of the run, else it gets no
//! rows at all (roads side by side on a hillside stay apart).

use std::collections::{BTreeMap, BTreeSet};

use crate::conform::FILL_SLOPE_MAX;
use crate::heightmap::Heightmap;
use crate::junctions::Junctions;
use crate::roadplace::PlacedNetwork;

use super::{Builder, Draft, Expr, Source};
use crate::profile::{
    KEEP_APART_M, MATCH_M, RoadStrip, SAMPLE_M, Spot, close_pairs, joint_heights, road_on_stretch,
    spots, strip_of,
};

/// Weight of the slack of a pair of spots.
const NEAR: f64 = 10.0;
/// How far the original ground at two spots of a run may differ beyond what the pair may differ by in height, in
/// metres, for the run to be tied.
const TIE_STEP_M: f64 = 1.0;

/// A place where a bridge crosses a road: the roads of the two stretches are not tied near it.
#[derive(Debug, Clone)]
pub(crate) struct Apart {
    /// Where the lines of the two roads cross.
    pub at: (f64, f64),
    /// The id of the stretch of the bridge.
    pub upper: String,
    /// The id of the stretch of the road below.
    pub lower: String,
}

/// The chains that have a road on the stretch `id`.
fn chains_on(net: &PlacedNetwork, drafts: &[Draft], id: &str) -> BTreeSet<usize> {
    drafts
        .iter()
        .enumerate()
        .filter(|(_, d)| {
            d.roads
                .iter()
                .any(|r| net.roads.get(*r).is_some_and(|r| road_on_stretch(r, id)))
        })
        .map(|(i, _)| i)
        .collect()
}

/// The places where two chains are connected, by the pair of chains (the lower index first): the centre of every
/// junction that holds a variable of both, and the position of every end of one that is a meeting on the other.
/// Two spots of the two chains near such a place are not tied: where roads meet they are one surface (the plane of
/// the junction, the height of the meeting) and legitimately differ in height on a slope. A junction that holds
/// two separate stretches of one chain (a loop) is not looked for.
fn connections(
    drafts: &[Draft],
    junctions: &Junctions,
) -> BTreeMap<(usize, usize), Vec<(f64, f64)>> {
    let mut out: BTreeMap<(usize, usize), Vec<(f64, f64)>> = BTreeMap::new();
    let mut holding: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (ci, draft) in drafts.iter().enumerate() {
        for j in draft.holders.iter().flatten() {
            holding.entry(*j).or_default().insert(ci);
        }
        let last = draft.sources.len().saturating_sub(1);
        for end in [0, last] {
            if let (Some(Source::Meets { owner, .. }), Some(at)) =
                (draft.sources.get(end), draft.chain.at.get(end))
                && *owner != ci
            {
                out.entry((ci.min(*owner), ci.max(*owner)))
                    .or_default()
                    .push(*at);
            }
        }
    }
    for (j, chains) in holding {
        let Some(point) = junctions.points.get(j) else {
            continue;
        };
        for &a in &chains {
            for &b in chains.range(a + 1..) {
                out.entry((a, b)).or_default().push(point.at);
            }
        }
    }
    out
}

/// The height of the spot `spot`: linear between the two variables of its segment.
fn spot_height(drafts: &[Draft], heights: &[Vec<Expr>], spot: &Spot) -> Option<Expr> {
    let chain = &drafts.get(spot.strip)?.chain;
    let own = heights.get(spot.strip)?;
    let from = chain.joints.get(spot.segment)?.2;
    let to = chain.joints.get(spot.segment + 1)?.2;
    if from == to {
        return own.get(from).cloned();
    }
    Some(Expr::sum(&[
        (own.get(from)?, 1.0 - spot.t),
        (own.get(to)?, spot.t),
    ]))
}

/// What two spots may differ by in height: [`MATCH_M`] where their footprints overlap, more as they move apart.
fn allowed(distance: f64, gap: f64, cell: f64) -> f64 {
    if distance < gap {
        MATCH_M
    } else {
        MATCH_M + (distance - gap + cell) * FILL_SLOPE_MAX
    }
}

/// Adds the rows (N) and their cost, for the chains `drafts` whose heights are `heights`; no rows for the pairs of
/// a bridge's road and the road below within [`KEEP_APART_M`] of a crossing in `apart`, and none for two spots within
/// [`KEEP_APART_M`] of a place where their roads are connected (see [`connections`]).
pub(super) fn add(
    builder: &mut Builder,
    net: &PlacedNetwork,
    hm: &Heightmap,
    drafts: &[Draft],
    heights: &[Vec<Expr>],
    junctions: &Junctions,
    apart: &[Apart],
) {
    let cell = f64::from(hm.cell_m);
    let Some(strips) = drafts
        .iter()
        .map(|d| {
            let first = net.roads.get(d.first)?;
            Some(strip_of(
                net,
                &d.roads,
                joint_heights(&d.chain, &[]),
                &[],
                first,
            ))
        })
        .collect::<Option<Vec<RoadStrip>>>()
    else {
        return;
    };
    let spots = spots(&strips);
    let pairs = close_pairs(&spots, &strips, cell);
    let crossings: Vec<(&Apart, BTreeSet<usize>, BTreeSet<usize>)> = apart
        .iter()
        .map(|a| {
            (
                a,
                chains_on(net, drafts, &a.upper),
                chains_on(net, drafts, &a.lower),
            )
        })
        .collect();
    let connected = connections(drafts, junctions);
    let near = |s: &Spot, at: (f64, f64)| (s.x - at.0).hypot(s.y - at.1) <= KEEP_APART_M;
    // the runs: the pairs of two chains, in the order along the first, split where the first jumps
    let mut by_chains: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (k, pair) in pairs.iter().enumerate() {
        let (Some(a), Some(b)) = (spots.get(pair.first), spots.get(pair.second)) else {
            continue;
        };
        let kept = crossings.iter().all(|(c, upper, lower)| {
            let across = (upper.contains(&a.strip) && lower.contains(&b.strip))
                || (lower.contains(&a.strip) && upper.contains(&b.strip));
            !(across && (near(a, c.at) || near(b, c.at)))
        });
        // both spots near a place where the two roads are connected
        let joined = connected
            .get(&(a.strip.min(b.strip), a.strip.max(b.strip)))
            .is_some_and(|places| places.iter().any(|at| near(a, *at) && near(b, *at)));
        if kept && !joined {
            by_chains.entry((a.strip, b.strip)).or_default().push(k);
        }
    }
    for mut list in by_chains.into_values() {
        let along = |k: usize, second: bool| {
            pairs
                .get(k)
                .and_then(|p| spots.get(if second { p.second } else { p.first }))
                .map_or(0.0, |s| s.along)
        };
        list.sort_by(|&x, &y| {
            along(x, false)
                .total_cmp(&along(y, false))
                .then(along(x, true).total_cmp(&along(y, true)))
                .then(x.cmp(&y))
        });
        let mut runs: Vec<Vec<usize>> = Vec::new();
        let mut last = f64::NEG_INFINITY;
        for k in list {
            let here = along(k, false);
            match runs.last_mut() {
                Some(run) if here - last <= 2.0 * SAMPLE_M => run.push(k),
                _ => runs.push(vec![k]),
            }
            last = here;
        }
        for run in &runs {
            tie_run(builder, hm, drafts, heights, &spots, &pairs, run);
        }
    }
}

/// The rows of one run, if the ground lets it be tied.
fn tie_run(
    builder: &mut Builder,
    hm: &Heightmap,
    drafts: &[Draft],
    heights: &[Vec<Expr>],
    spots: &[Spot],
    pairs: &[crate::profile::Pair],
    run: &[usize],
) {
    let cell = f64::from(hm.cell_m);
    let ground = |s: &Spot| f64::from(hm.sample(s.x, s.y).height);
    let mut rows: Vec<(Expr, f64)> = Vec::new();
    for &k in run {
        let Some(pair) = pairs.get(k) else { return };
        let (Some(a), Some(b)) = (spots.get(pair.first), spots.get(pair.second)) else {
            return;
        };
        let limit = allowed(pair.distance, pair.gap, cell);
        let step = (ground(a) - ground(b)).abs();
        // not a number is not a tie either
        if step.is_nan() || step > limit + TIE_STEP_M {
            return;
        }
        let (Some(ha), Some(hb)) = (
            spot_height(drafts, heights, a),
            spot_height(drafts, heights, b),
        ) else {
            return;
        };
        let d = Expr::sum(&[(&ha, 1.0), (&hb, -1.0)]);
        if !d.terms.is_empty() {
            rows.push((d, limit));
        }
    }
    for (d, limit) in rows {
        let n = builder.band(&d, limit);
        builder.problem.add_square(NEAR, &[(n, 1.0)], 0.0);
    }
}

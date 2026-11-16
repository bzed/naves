// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The rows (N) of `docs/superpowers/specs/2026-11-16-height-solve-design.md`: roads whose footprints share
//! terrain cells share a height, as far as the ground lets them.
//!
//! The spots every [`SAMPLE_M`] and the pairs of close spots are those of the old settling (`spots` and
//! `close_pairs`), found from the positions and the half widths of the roads alone. Of those pairs only the mutually
//! nearest are kept (a spot's nearest spot of the other chain, seen from both sides), chosen before the exclusions
//! for bridges and for the junction a loop leaves and re-enters. Roads that part at an angle keep about one pair, so
//! they are left untied beyond their first spot; that is intended: rows (N) are for roads side by side. The pairs of
//! two chains are grouped into runs; a run is tied only if the original ground allows it at every pair of the run,
//! else it gets no rows at all (roads side by side on a hillside stay apart).

use std::collections::{BTreeMap, BTreeSet};

use crate::conform::FILL_SLOPE_MAX;
use crate::heightmap::Heightmap;
use crate::junctions::Junctions;
use crate::roadplace::PlacedNetwork;

use super::{Builder, Draft, Expr};
use crate::profile::Pair;
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

/// The places where a chain is connected to itself: the centre of every junction that holds two separate stretches
/// of it (a loop that leaves and re-enters the junction; its held variables are not contiguous, a through road's
/// are), by chain. Two spots of the chain near such a place are not tied: there the legs are one surface (the plane
/// of the junction) and legitimately differ in height on a slope.
fn loops(drafts: &[Draft], junctions: &Junctions) -> BTreeMap<usize, Vec<(f64, f64)>> {
    let mut out: BTreeMap<usize, Vec<(f64, f64)>> = BTreeMap::new();
    for (ci, draft) in drafts.iter().enumerate() {
        let mut runs: BTreeMap<usize, usize> = BTreeMap::new();
        let mut before = None;
        for holder in &draft.holders {
            if let Some(j) = holder
                && before != Some(*j)
            {
                *runs.entry(*j).or_insert(0) += 1;
            }
            before = *holder;
        }
        for (j, count) in runs {
            if count > 1
                && let Some(point) = junctions.points.get(j)
            {
                out.entry(ci).or_default().push(point.at);
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

/// The pairs (by index) that are mutually nearest: the second spot is the nearest of the first's pairs with the
/// second's chain, and the first spot the nearest of the second's pairs with the first's chain (the lower spot
/// index on a tie). Roads share a height abreast, not at other places along a slope, and which of the two roads
/// comes first does not matter. This is chosen before the exclusions.
fn mutual_nearest(pairs: &[Pair], spots: &[Spot]) -> BTreeSet<usize> {
    let mut best: BTreeMap<(usize, usize), (f64, usize, usize)> = BTreeMap::new();
    for (k, pair) in pairs.iter().enumerate() {
        let (Some(a), Some(b)) = (spots.get(pair.first), spots.get(pair.second)) else {
            continue;
        };
        for (spot, chain, partner) in [
            (pair.first, b.strip, pair.second),
            (pair.second, a.strip, pair.first),
        ] {
            let slot = best
                .entry((spot, chain))
                .or_insert((pair.distance, partner, k));
            if pair
                .distance
                .total_cmp(&slot.0)
                .then(partner.cmp(&slot.1))
                .is_lt()
            {
                *slot = (pair.distance, partner, k);
            }
        }
    }
    let chosen = |spot: usize, chain: usize, k: usize| {
        best.get(&(spot, chain)).is_some_and(|slot| slot.2 == k)
    };
    pairs
        .iter()
        .enumerate()
        .filter(|(k, pair)| {
            let (Some(a), Some(b)) = (spots.get(pair.first), spots.get(pair.second)) else {
                return false;
            };
            chosen(pair.first, b.strip, *k) && chosen(pair.second, a.strip, *k)
        })
        .map(|(k, _)| k)
        .collect()
}

/// Adds the rows (N) and their cost, for the chains `drafts` whose heights are `heights`, for the pairs of spots
/// that are mutually nearest (chosen from all close pairs, before the exclusions); no rows for the pairs of
/// a bridge's road and the road below within [`KEEP_APART_M`] of a crossing in `apart`, and none for two spots within
/// [`KEEP_APART_M`] of a junction that their chain leaves and re-enters (see [`loops`]).
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
    // the crossings by the pair of chains they separate (the lower index first)
    let mut crossings: BTreeMap<(usize, usize), Vec<(f64, f64)>> = BTreeMap::new();
    for a in apart {
        let (upper, lower) = (
            chains_on(net, drafts, &a.upper),
            chains_on(net, drafts, &a.lower),
        );
        for &u in &upper {
            for &l in &lower {
                crossings
                    .entry((u.min(l), u.max(l)))
                    .or_default()
                    .push(a.at);
            }
        }
    }
    let looping = loops(drafts, junctions);
    let near = |s: &Spot, at: (f64, f64)| (s.x - at.0).hypot(s.y - at.1) <= KEEP_APART_M;
    let mutual = mutual_nearest(&pairs, &spots);
    // the runs: the pairs of two chains, in the order along the first, split where the first jumps
    let mut kept_pairs: Vec<usize> = Vec::new();
    for (k, pair) in pairs.iter().enumerate() {
        let (Some(a), Some(b)) = (spots.get(pair.first), spots.get(pair.second)) else {
            continue;
        };
        let key = (a.strip.min(b.strip), a.strip.max(b.strip));
        let kept = !crossings
            .get(&key)
            .is_some_and(|places| places.iter().any(|at| near(a, *at) || near(b, *at)));
        // both spots of one chain near a junction that it leaves and re-enters
        let joined = a.strip == b.strip
            && looping
                .get(&a.strip)
                .is_some_and(|places| places.iter().any(|at| near(a, *at) && near(b, *at)));
        if kept && !joined && mutual.contains(&k) {
            kept_pairs.push(k);
        }
    }
    let mut by_chains: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for k in kept_pairs {
        let strips = pairs
            .get(k)
            .and_then(|p| Some((spots.get(p.first)?.strip, spots.get(p.second)?.strip)));
        if let Some(key) = strips {
            by_chains.entry(key).or_default().push(k);
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

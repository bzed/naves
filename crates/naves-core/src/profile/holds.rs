// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which variables of a chain a junction holds, and why (see [`crate::junctions`]).

use std::collections::BTreeSet;

use crate::junctions::{ARM_REACH_M, HOLD_M, Junction, JunctionKind, Junctions};
use crate::roadplace::{OVERLAY_DROP_M, PlacedKind, PlacedNetwork};

use super::{Chain, chain_of, point_segment_distance};

/// Which rule holds a variable of a chain to a junction (for measuring, see [`chain_holds`]).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldRule {
    /// The variable lies in the junction's patch.
    Patch,
    /// A variable of a junction piece.
    Piece,
    /// An end of the chain.
    End,
    /// Up to eight variables inward from a held end, on the other road's level strip.
    Inward,
    /// A variable of the piece that passes the junction within [`ARM_REACH_M`].
    Arm,
}

/// The junction that holds a variable, and why (for measuring, see [`chain_holds`]).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hold {
    /// The rule.
    pub rule: HoldRule,
    /// Index into [`Junctions::points`].
    pub junction: usize,
    /// The height it gives.
    pub height: f64,
    /// Where the variable is held: its position, for a bridge deck the end of the deck that faces the junction.
    pub at: (f64, f64),
}

/// Sets the junction that holds variable `var`, which lies at `at`: it gets the height of the junction's plane
/// there. A road of a chain that lies under the top road of an overlay
/// (`ids` are the stretches of the chain) is held [`OVERLAY_DROP_M`] below the junction's height.
fn hold(
    held: &mut [Option<Hold>],
    var: usize,
    at: (f64, f64),
    junction: usize,
    junctions: &Junctions,
    ids: &BTreeSet<String>,
    rule: HoldRule,
) {
    if let (Some(slot), Some(j)) = (held.get_mut(var), junctions.points.get(junction)) {
        *slot = Some(Hold {
            rule,
            junction,
            height: j.height_at(at) - overlay_drop(j, ids),
            at,
        });
    }
}

/// How far below the junction's plane a chain of the stretches `ids` is held: [`OVERLAY_DROP_M`] for a road
/// under the top road of an overlay, else 0.
pub(super) fn overlay_drop(junction: &Junction, ids: &BTreeSet<String>) -> f64 {
    if junction.kind == JunctionKind::Overlay && !junction.lower.is_disjoint(ids) {
        OVERLAY_DROP_M
    } else {
        0.0
    }
}

/// The holds of the joints of the chain `roads`, one per joint of the strip as the height solve builds it (the
/// joints of a bridge deck share one variable), with the position of each joint.
///
/// For measuring (the `road-analysis` tool and the tests); not part of the library's interface.
#[doc(hidden)]
#[must_use]
pub fn chain_holds(
    net: &PlacedNetwork,
    roads: &[usize],
    cell: f64,
    junctions: &Junctions,
) -> Vec<((f64, f64), Option<Hold>)> {
    holds_of(net, roads, cell, junctions, true)
}

/// [`chain_holds`] without the check that the chain belongs to the junction, to count what the check removes.
#[doc(hidden)]
#[must_use]
pub fn chain_holds_unchecked(
    net: &PlacedNetwork,
    roads: &[usize],
    cell: f64,
    junctions: &Junctions,
) -> Vec<((f64, f64), Option<Hold>)> {
    holds_of(net, roads, cell, junctions, false)
}

fn holds_of(
    net: &PlacedNetwork,
    roads: &[usize],
    cell: f64,
    junctions: &Junctions,
    check: bool,
) -> Vec<((f64, f64), Option<Hold>)> {
    let Some(chain) = chain_of(net, roads, cell, junctions) else {
        return Vec::new();
    };
    let half_width = roads
        .first()
        .and_then(|i| net.roads.get(*i))
        .map_or(0.0, |r| r.width / 2.0);
    let held = junction_heights(net, &chain, roads, half_width, cell, junctions, check);
    chain
        .joints
        .iter()
        .map(|j| ((j.0, j.1), held.get(j.2).copied().flatten()))
        .collect()
}

/// The junction each joint of `chain` is held to, if any.
pub(super) fn joint_junctions(
    net: &PlacedNetwork,
    chain: &Chain,
    roads: &[usize],
    cell: f64,
    junctions: &Junctions,
) -> Vec<Option<usize>> {
    let half_width = roads
        .first()
        .and_then(|i| net.roads.get(*i))
        .map_or(0.0, |r| r.width / 2.0);
    let held = junction_heights(net, chain, roads, half_width, cell, junctions, true);
    chain
        .joints
        .iter()
        .map(|j| held.get(j.2).copied().flatten().map(|h| h.junction))
        .collect()
}

/// The stretch ids of the roads of a chain, junction pieces' `a|b` ids split.
pub(super) fn stretch_ids(net: &PlacedNetwork, roads: &[usize]) -> BTreeSet<String> {
    roads
        .iter()
        .filter_map(|i| net.roads.get(*i))
        .flat_map(|r| r.slot.stretch.split('|').map(str::to_string))
        .collect()
}

/// Where variable `var` of `chain` lies for a junction at `toward`: its position, and for a bridge deck, which is
/// as long as its span, the end of the deck or the middle, whichever is nearest the junction. A deck is held to a
/// junction at the end that faces it, not at its middle.
fn hold_point(chain: &Chain, var: usize, toward: Option<(f64, f64)>) -> (f64, f64) {
    let middle = chain.at.get(var).copied().unwrap_or_default();
    let (Some((start, end)), Some(to)) = (chain.deck.get(var).copied().flatten(), toward) else {
        return middle;
    };
    [start, end]
        .into_iter()
        .chain(std::iter::once(middle))
        .min_by(|a, b| {
            (a.0 - to.0)
                .hypot(a.1 - to.1)
                .total_cmp(&(b.0 - to.0).hypot(b.1 - to.1))
        })
        .unwrap_or(middle)
}

/// Where the end variable `end` of `chain` (`side` 0: the first, else the last) lies for the rule that holds a
/// chain end to a junction: a deck's outer end, the one that is the end of the chain, not its middle.
pub(super) fn outer_point(chain: &Chain, end: usize, side: usize) -> Option<(f64, f64)> {
    match chain.deck.get(end).copied().flatten() {
        Some((start, finish)) => Some(if side == 0 { start } else { finish }),
        None => chain.at.get(end).copied(),
    }
}

/// The runs of consecutive variables held to one junction by `Patch`, cut down to the one variable of each run
/// that lies nearest the junction: a road that passes a junction is held at one joint, not along its length.
fn nearest_of_runs(candidates: &[(usize, usize, f64)]) -> Vec<(usize, usize)> {
    let mut runs: Vec<Vec<(usize, usize, f64)>> = Vec::new();
    for &candidate in candidates {
        match runs.last_mut() {
            Some(run)
                if run
                    .last()
                    .is_some_and(|l| l.1 == candidate.1 && l.0 + 1 == candidate.0) =>
            {
                run.push(candidate);
            }
            _ => runs.push(vec![candidate]),
        }
    }
    runs.iter()
        .filter_map(|run| run.iter().min_by(|a, b| a.2.total_cmp(&b.2)))
        .map(|&(var, junction, _)| (var, junction))
        .collect()
}

/// The height a junction gives each variable of a chain, by the least that holds the junction's height where
/// the road has it:
/// - a junction piece holds its two variables (the joints lie on the piece's plane, which tilts along its
///   through road);
/// - a chain that passes a junction is held at the one joint nearest the junction point (`Patch`), or, where
///   no joint lies in the patch, at the one joint of the nearest piece that lies nearest the junction (`Arm`,
///   within [`ARM_REACH_M`]);
/// - a chain that ends at a junction is held at its end (a side road ends at the port of a piece, beyond the
///   through road's edge), and inward only as far as its joints lie on the other road's level strip, up to eight.
///
/// A chain is held only to the junctions it belongs to (`Junction::stretches`), unless `check` is false (for
/// measuring what the check removes).
#[expect(
    clippy::too_many_lines,
    reason = "the rules that hold a chain to a junction, in the order they apply"
)]
pub(super) fn junction_heights(
    net: &PlacedNetwork,
    chain: &Chain,
    roads: &[usize],
    half_width: f64,
    cell: f64,
    junctions: &Junctions,
    check: bool,
) -> Vec<Option<Hold>> {
    let count = chain.at.len();
    let ids = stretch_ids(net, roads);
    let member = |index: usize| {
        !check
            || junctions
                .points
                .get(index)
                .is_some_and(|j| j.stretches.is_empty() || !j.stretches.is_disjoint(&ids))
    };
    // the nearest junction to a point that this chain belongs to, within the reach beyond its patch
    let nearest = |point: (f64, f64), reach: f64| {
        junctions
            .points
            .iter()
            .enumerate()
            .filter(|(i, _)| member(*i))
            .map(|(i, j)| (i, (j.at.0 - point.0).hypot(j.at.1 - point.1), j.patch_m))
            .filter(|(_, d, patch)| *d <= patch + reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, d, _)| (i, d))
    };
    let give = |held: &mut [Option<Hold>], var: usize, junction: usize, rule: HoldRule| {
        let toward = junctions.points.get(junction).map(|j| j.at);
        let at = hold_point(chain, var, toward);
        hold(held, var, at, junction, junctions, &ids, rule);
    };
    let mut held: Vec<Option<Hold>> = vec![None; count];
    for (k, &road) in roads.iter().enumerate() {
        let Some(road) = net
            .roads
            .get(road)
            .filter(|r| r.kind == PlacedKind::Junction)
        else {
            continue;
        };
        let centre = (
            f64::midpoint(road.start.x, road.end.x),
            f64::midpoint(road.start.y, road.end.y),
        );
        if let (Some((junction, _)), Some(from), Some(to)) = (
            nearest(centre, HOLD_M),
            chain.joints.get(k),
            chain.joints.get(k + 1),
        ) {
            give(&mut held, from.2, junction, HoldRule::Piece);
            give(&mut held, to.2, junction, HoldRule::Piece);
        }
    }
    // a junction that a variable lies in the patch of: the variable nearest the junction of each run
    let candidates: Vec<(usize, usize, f64)> = chain
        .at
        .iter()
        .enumerate()
        .filter(|(var, _)| held.get(*var).is_some_and(Option::is_none))
        .filter_map(|(var, point)| {
            // a deck is as near a junction as the nearest of its ends and its middle
            let ends = chain
                .deck
                .get(var)
                .copied()
                .flatten()
                .into_iter()
                .flat_map(|(a, b)| [a, b]);
            std::iter::once(*point)
                .chain(ends)
                .filter_map(|p| nearest(p, 0.0))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(j, d)| (var, j, d))
        })
        .collect();
    for (var, junction) in nearest_of_runs(&candidates) {
        give(&mut held, var, junction, HoldRule::Patch);
    }
    // the ends, and inward from them while the joints lie on the surface of the other road; the ends of a
    // ring are not ends (it closes there) and its joints lie on its plane, not on a level strip
    let on_ring = junctions
        .rings
        .iter()
        .any(|r| !r.stretches.is_disjoint(&ids));
    for (side, end) in [0, count.saturating_sub(1)].into_iter().enumerate() {
        let Some((junction_index, _)) =
            outer_point(chain, end, side).and_then(|point| nearest(point, HOLD_M))
        else {
            continue;
        };
        let Some(junction) = junctions.points.get(junction_index) else {
            continue;
        };
        // the ground the other road's level strip takes: its half width and 1.5 cells beyond (`conform`), on a
        // junction piece counted from the end of the piece
        let reach = if junction.kind == JunctionKind::Piece {
            junction.patch_m + half_width + 1.5 * cell
        } else {
            junction.patch_m + 1.5 * cell
        };
        let mut var = end;
        for _ in 0..8 {
            let on_surface = chain
                .at
                .get(var)
                .is_some_and(|p| (p.0 - junction.at.0).hypot(p.1 - junction.at.1) <= reach);
            if var != end && (!on_surface || on_ring) {
                break;
            }
            if held.get(var).is_some_and(Option::is_none) {
                let rule = if var == end {
                    HoldRule::End
                } else {
                    HoldRule::Inward
                };
                give(&mut held, var, junction_index, rule);
            }
            let following = if side == 0 {
                var.checked_add(1)
            } else {
                var.checked_sub(1)
            };
            match following {
                Some(step) if step < count => var = step,
                _ => break,
            }
        }
    }
    let members: Vec<usize> = (0..junctions.points.len()).filter(|i| member(*i)).collect();
    hold_beside(chain, junctions, &members, &ids, &mut held);
    held
}

/// A junction beside a piece, where no variable lies in its patch (a road that ends on this one without a
/// crossing, or an arm that passes the crossing at up to [`ARM_REACH_M`]): the nearer variable of the nearest
/// piece holds its height. `members` are the junctions the chain belongs to; a junction that already holds a
/// variable of the chain is done.
fn hold_beside(
    chain: &Chain,
    junctions: &Junctions,
    members: &[usize],
    ids: &BTreeSet<String>,
    held: &mut [Option<Hold>],
) {
    let (mut low, mut high) = (
        (f64::INFINITY, f64::INFINITY),
        (f64::NEG_INFINITY, f64::NEG_INFINITY),
    );
    for p in &chain.at {
        low = (low.0.min(p.0), low.1.min(p.1));
        high = (high.0.max(p.0), high.1.max(p.1));
    }
    let is_deck = |v: usize| chain.deck.get(v).is_some_and(Option::is_some);
    for &junction in members {
        let Some(point) = junctions.points.get(junction).map(|j| j.at) else {
            continue;
        };
        // a box test first: most junctions are nowhere near the chain
        if point.0 < low.0 - ARM_REACH_M
            || point.0 > high.0 + ARM_REACH_M
            || point.1 < low.1 - ARM_REACH_M
            || point.1 > high.1 + ARM_REACH_M
            || held.iter().flatten().any(|h| h.junction == junction)
        {
            continue;
        }
        let free = |v: usize| held.get(v).is_some_and(Option::is_none) && !is_deck(v);
        let beside = chain
            .at
            .windows(2)
            .enumerate()
            .filter_map(|(var, pair)| {
                let [from, to] = pair else { return None };
                (free(var) && free(var + 1))
                    .then(|| (point_segment_distance(point, *from, *to), var, *from, *to))
            })
            .filter(|(distance, ..)| *distance <= ARM_REACH_M)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, var, from, to)) = beside {
            let nearer = if (from.0 - point.0).hypot(from.1 - point.1)
                <= (to.0 - point.0).hypot(to.1 - point.1)
            {
                var
            } else {
                var + 1
            };
            let at = chain.at.get(nearer).copied().unwrap_or_default();
            hold(held, nearer, at, junction, junctions, ids, HoldRule::Arm);
        }
    }
}

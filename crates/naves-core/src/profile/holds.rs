// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which variables of a chain a junction holds, and why (see [`crate::junctions`]).

use std::collections::BTreeSet;

use crate::junctions::{ARM_REACH_M, HOLD_M, Junction, JunctionKind, Junctions};
use crate::limits::RAMP_JOINT_MIN_M;
use crate::roadplace::{OVERLAY_DROP_M, PlacedKind, PlacedNetwork};

use super::{Chain, chain_of, point_segment_distance, ring_of};

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
    let half_width = chain
        .legs
        .first()
        .and_then(|i| net.roads.get(*i))
        .map_or(0.0, |r| r.width / 2.0);
    let held = junction_heights(net, &chain, half_width, cell, junctions, check);
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
    cell: f64,
    junctions: &Junctions,
) -> Vec<Option<usize>> {
    let half_width = chain
        .legs
        .first()
        .and_then(|i| net.roads.get(*i))
        .map_or(0.0, |r| r.width / 2.0);
    let held = junction_heights(net, chain, half_width, cell, junctions, true);
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
///   through road's edge), and inward only as far as its joints lie on the other road's level strip, up to eight;
/// - a joint that was added where the road enters or leaves a junction's patch is held to that junction
///   (`Patch`, see [`ramp_joints`]).
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
    half_width: f64,
    cell: f64,
    junctions: &Junctions,
    check: bool,
) -> Vec<Option<Hold>> {
    let count = chain.at.len();
    let roads = chain.legs.as_slice();
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
    // the joints added where the road enters and leaves the patch of a junction ([`ramp_joints`]): each is held
    // to its junction (a road that passes a junction is held at one joint of its own, but these two are what
    // makes the road lie on the junction's plane inside the patch)
    for (var, junction) in chain.patch.iter().enumerate() {
        if let Some(junction) = junction.filter(|j| member(*j))
            && held.get(var).is_some_and(Option::is_none)
        {
            give(&mut held, var, junction, HoldRule::Patch);
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

/// A junction that a chain passes within its patch without a variable there: the strip between two variables
/// (one of them a bridge deck) runs through the patch, and the junction's plane is to hold it there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Crossing {
    /// Index into [`Junctions::points`].
    pub junction: usize,
    /// The variable before the strip's run through the patch.
    pub from: usize,
    /// The variable after it.
    pub to: usize,
    /// How far along the run, from the end of `from` to the start of `to`, the strip lies nearest the junction.
    pub t: f64,
    /// That place.
    pub at: (f64, f64),
}

/// The [`Crossing`]s of a chain: the junctions it belongs to that hold no variable of it (`held`, from
/// [`junction_heights`]) and whose patch the strip runs through between a bridge deck and the next variable,
/// where no joint lies to be held (the deck is not held by [`HoldRule::Patch`] or [`HoldRule::Arm`] there: it
/// lies outside the patch at its end, and the next joint is a piece further on). A junction within its patch of
/// the end of the deck is left to the rules of the deck. One per junction, in the order
/// of [`Junctions::points`], at the run that passes nearest it.
pub(super) fn crossings(
    chain: &Chain,
    junctions: &Junctions,
    ids: &BTreeSet<String>,
    held: &[Option<Hold>],
) -> Vec<Crossing> {
    let is_deck = |v: usize| chain.deck.get(v).is_some_and(Option::is_some);
    let mut found = Vec::new();
    if !chain.deck.iter().any(Option::is_some) {
        return found;
    }
    for (index, junction) in junctions.points.iter().enumerate() {
        if (!junction.stretches.is_empty() && junction.stretches.is_disjoint(ids))
            || held.iter().flatten().any(|h| h.junction == index)
        {
            continue;
        }
        let nearest = (0..chain.at.len().saturating_sub(1))
            .filter(|&v| is_deck(v) || is_deck(v + 1))
            .filter_map(|v| {
                // a deck runs level to its end: the run starts at the end of the deck that faces the next
                // variable and ends at the start of the one that faces the last
                let from = match chain.deck.get(v).copied().flatten() {
                    Some((_, end)) => end,
                    None => chain.at.get(v).copied()?,
                };
                let to = match chain.deck.get(v + 1).copied().flatten() {
                    Some((start, _)) => start,
                    None => chain.at.get(v + 1).copied()?,
                };
                // a junction in the patch of a deck's end is the deck's own: held by [`HoldRule::Patch`] when it
                // is the nearest, else its step is for the report (a deck is one level height, held to one
                // junction)
                let at_deck_end = |deck: bool, end: (f64, f64)| {
                    deck && (junction.at.0 - end.0).hypot(junction.at.1 - end.1) <= junction.patch_m
                };
                if at_deck_end(is_deck(v), from) || at_deck_end(is_deck(v + 1), to) {
                    return None;
                }
                let (dx, dy) = (to.0 - from.0, to.1 - from.1);
                let length2 = dx * dx + dy * dy;
                if length2 <= 0.0 {
                    return None;
                }
                let t = (((junction.at.0 - from.0) * dx + (junction.at.1 - from.1) * dy) / length2)
                    .clamp(0.0, 1.0);
                // a run's end variable that another junction already holds is not pinned to this one's plane
                let var_held =
                    |var: usize| held.get(var).is_some_and(Option::is_some) && !is_deck(var);
                if (t <= 0.0 && var_held(v)) || (t >= 1.0 && var_held(v + 1)) {
                    return None;
                }
                let at = (from.0 + t * dx, from.1 + t * dy);
                let distance = (junction.at.0 - at.0).hypot(junction.at.1 - at.1);
                (distance <= junction.patch_m).then_some((distance, v, t, at))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, from, t, at)) = nearest {
            found.push(Crossing {
                junction: index,
                from,
                to: from + 1,
                t,
                at,
            });
        }
    }
    found
}

/// How far inside the edge of a patch a joint that is added there lies, in metres, so that the test that finds the
/// variables in a patch finds it whatever the rounding.
const PATCH_INSET_M: f64 = 0.05;

/// A joint to add inside a piece of a chain: where the road enters or leaves the patch of a junction (see
/// [`ramp_joints`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct RampJoint {
    /// The segment of the chain (between joint `segment` and the next) that the joint lies on.
    pub segment: usize,
    /// How far along the segment, from 0 (its first joint) to 1 (its last).
    pub t: f64,
    /// Where it lies.
    pub at: (f64, f64),
    /// The junction whose patch it lies in, an index into [`Junctions::points`].
    pub junction: usize,
    /// For a place too close to a joint the road has (under [`RAMP_JOINT_MIN_M`]), that joint, as an index into
    /// [`Chain::joints`]: no joint is added, the junction holds this one.
    pub existing: Option<usize>,
}

/// The joints to add to `chain` (the chain of the roads `own` as the placement has them, with a joint at the end of
/// each piece) where a road passes through the patch of a junction it belongs to without a joint of its own in
/// the patch: one where it enters the patch and one where it leaves, a little inside the patch's edge. The
/// junction's plane holds them (`Patch`), so the road lies on it inside the patch, and the height difference to
/// the road's other joints is closed by the grades between the joints, a ramp, not by a step or by lifting a
/// joint far away.
///
/// A joint is not added closer than [`RAMP_JOINT_MIN_M`] to a joint the road has: the road's joint is held to the
/// junction instead (`RampJoint::existing`; not the start of the chain, a bridge deck or a joint another junction
/// holds), and not closer to one added before it (the two where the road enters and leaves a patch that short are one,
/// at the nearest approach). Where no joint can be held that way, the junction is held by the older rules (`Arm`,
/// and for a run next to a bridge deck a crossing row, see [`crossings`]). Only the pieces of a plain road are split: not a bridge deck, a junction piece, nor the chain of
/// a roundabout, whose joints lie on its plane. A junction that already holds a variable of the chain by another
/// rule than `Arm` (a junction piece, a chain end, a chain that ends on this road) is not given joints.
pub(super) fn ramp_joints(
    net: &PlacedNetwork,
    own: &[usize],
    chain: &Chain,
    cell: f64,
    junctions: &Junctions,
) -> Vec<RampJoint> {
    if junctions.points.is_empty() || ring_of(net, own, junctions).is_some() {
        return Vec::new();
    }
    let ids = stretch_ids(net, own);
    let (mut low, mut high) = (
        (f64::INFINITY, f64::INFINITY),
        (f64::NEG_INFINITY, f64::NEG_INFINITY),
    );
    for joint in &chain.joints {
        low = (low.0.min(joint.0), low.1.min(joint.1));
        high = (high.0.max(joint.0), high.1.max(joint.1));
    }
    let mut found: Vec<RampJoint> = Vec::new();
    for (index, junction) in junctions.points.iter().enumerate() {
        let (cx, cy) = junction.at;
        let reach = junction.patch_m;
        // a box test first: most junctions are nowhere near the chain
        if cx < low.0 - reach
            || cx > high.0 + reach
            || cy < low.1 - reach
            || cy > high.1 + reach
            || (!junction.stretches.is_empty() && junction.stretches.is_disjoint(&ids))
            || chain
                .joints
                .iter()
                .any(|j| (j.0 - cx).hypot(j.1 - cy) <= reach)
        {
            continue;
        }
        found.extend(ramp_joints_at(net, chain, junction, index, &found));
    }
    if found.is_empty() {
        return found;
    }
    // a junction that another rule than `Arm` holds a variable of this chain to is left to that rule
    let half_width = chain
        .legs
        .first()
        .and_then(|i| net.roads.get(*i))
        .map_or(0.0, |r| r.width / 2.0);
    let held = junction_heights(net, chain, half_width, cell, junctions, true);
    let taken: BTreeSet<usize> = held
        .iter()
        .flatten()
        .filter(|h| h.rule != HoldRule::Arm)
        .map(|h| h.junction)
        .collect();
    found.retain(|f| !taken.contains(&f.junction));
    // a road's own joint that another rule or another junction already holds stays as it is
    found.retain(|f| {
        f.existing
            .and_then(|k| chain.joints.get(k))
            .and_then(|j| held.get(j.2).copied().flatten())
            .is_none_or(|h| h.rule == HoldRule::Arm && h.junction == f.junction)
    });
    found
}

/// The joints [`ramp_joints`] wants for the junction `index` (`found` are those of the junctions before it): where its
/// patch is passed and no joint of the road lies in it.
fn ramp_joints_at(
    net: &PlacedNetwork,
    chain: &Chain,
    junction: &Junction,
    index: usize,
    found: &[RampJoint],
) -> Vec<RampJoint> {
    let (cx, cy) = junction.at;
    let radius = junction.patch_m - PATCH_INSET_M;
    let Some(Pass {
        segment,
        t: (t_in, t_out),
        from: a,
        direction: (dx, dy),
    }) = (radius > 0.0)
        .then(|| through_patch(net, chain, junction, radius))
        .flatten()
    else {
        return Vec::new();
    };
    let at = |t: f64| (a.0 + t * dx, a.1 + t * dy);
    let chord = (t_out - t_in) * dx.hypot(dy);
    let wanted: Vec<f64> = if chord < RAMP_JOINT_MIN_M {
        vec![f64::midpoint(t_in, t_out)]
    } else {
        vec![t_in, t_out]
    };
    // the road's own joints that are too close for another: the junction holds the nearest one, as the older rule
    // did, not both ends of a piece
    let mut own_joints: Vec<usize> = Vec::new();
    let mut added: Vec<RampJoint> = Vec::new();
    for t in wanted {
        let point = at(t);
        let near = |p: (f64, f64)| (p.0 - point.0).hypot(p.1 - point.1);
        let existing = chain
            .joints
            .iter()
            .enumerate()
            .map(|(k, j)| (k, near((j.0, j.1))))
            .filter(|(_, d)| *d < RAMP_JOINT_MIN_M)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((joint, _)) = existing {
            // not the start of the chain (a chain end rule holds that) nor a joint of a bridge deck (one level height)
            let usable = joint > 0
                && chain
                    .joints
                    .get(joint)
                    .is_some_and(|j| chain.deck.get(j.2).is_some_and(Option::is_none));
            if usable && !own_joints.contains(&joint) {
                own_joints.push(joint);
            }
        } else if !found
            .iter()
            .chain(&added)
            .any(|f| near(f.at) < RAMP_JOINT_MIN_M)
        {
            added.push(RampJoint {
                segment,
                t,
                at: point,
                junction: index,
                existing: None,
            });
        }
    }
    let from_junction = |k: usize| {
        chain
            .joints
            .get(k)
            .map_or(f64::INFINITY, |j| (j.0 - cx).hypot(j.1 - cy))
    };
    if let Some(joint) = own_joints
        .into_iter()
        .min_by(|a, b| from_junction(*a).total_cmp(&from_junction(*b)))
    {
        added.push(RampJoint {
            segment,
            t: 0.0,
            at: chain.joints.get(joint).map_or((cx, cy), |j| (j.0, j.1)),
            junction: index,
            existing: Some(joint),
        });
    }
    added
}

/// The piece of `chain` that passes nearest the centre of `junction` through a circle of `radius` around it, with no
/// joint inside: its segment, the two parameters (from 0 at its first joint to 1 at its last) where it enters and
/// leaves the circle, its first joint and its direction (a [`Pass`]). Only a plain piece counts, not a bridge deck or a junction
/// piece.
fn through_patch(
    net: &PlacedNetwork,
    chain: &Chain,
    junction: &Junction,
    radius: f64,
) -> Option<Pass> {
    let (cx, cy) = junction.at;
    chain
        .joints
        .windows(2)
        .enumerate()
        .filter(|(k, _)| {
            chain
                .legs
                .get(*k)
                .and_then(|i| net.roads.get(*i))
                .is_some_and(|r| r.kind == PlacedKind::Piece)
        })
        .filter_map(|(k, pair)| {
            let [a, b] = pair else { return None };
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let (fx, fy) = (a.0 - cx, a.1 - cy);
            let qa = dx * dx + dy * dy;
            if qa <= 0.0 {
                return None;
            }
            let qb = 2.0 * (fx * dx + fy * dy);
            let discriminant = qb * qb - 4.0 * qa * (fx * fx + fy * fy - radius * radius);
            if discriminant <= 0.0 {
                return None;
            }
            let root = discriminant.sqrt();
            let (t_in, t_out) = ((-qb - root) / (2.0 * qa), (-qb + root) / (2.0 * qa));
            if !(t_in > 0.0 && t_out < 1.0) {
                return None;
            }
            let middle = f64::midpoint(t_in, t_out);
            let nearest = (fx + middle * dx).hypot(fy + middle * dy);
            Some((
                nearest,
                Pass {
                    segment: k,
                    t: (t_in, t_out),
                    from: (a.0, a.1),
                    direction: (dx, dy),
                },
            ))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, pass)| pass)
}

/// A piece's run through a circle (see [`through_patch`]).
struct Pass {
    /// The segment of the chain.
    segment: usize,
    /// Where it enters and leaves the circle, from 0 at the segment's first joint to 1 at its last.
    t: (f64, f64),
    /// The segment's first joint.
    from: (f64, f64),
    /// From the first joint to the last.
    direction: (f64, f64),
}

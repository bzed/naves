// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Step 4 of the clean-up: a junction on the end of a bridge deck is moved off it.
//!
//! A bridge end node where three or more stretch ends meet is a junction on the deck. Of the other stretches
//! one may continue the bridge's road (by the chain rule, [`crate::roadnet::chains`]):
//!
//! - **The road goes on** (the stretch `C`): the roads that met the deck end are taken off the end node and joined to
//!   `C` [`APPROACH_M`] along it from the deck end, their last metres re-aimed there. `C` is cut by a new node
//!   there into an approach piece (`<id>/approach<n>`, next to the deck) and the rest, which keeps the id. If `C` is
//!   [`APPROACH_M`] long (within [`SPECK_M`]), the roads are joined to its far end node instead and `C` stays whole.
//!   The new node must be at least [`APPROACH_M`] from the deck end and at least [`JUNCTION_SPACING_M`] from the
//!   far end of `C`, otherwise there is `"no_room"`. So is it if a road that is moved would be joined to a node it
//!   already has, if one of the roads is itself a bridge or a tunnel, or if moving a road makes it cross a road it did
//!   not cross before.
//! - **The road ends** (a T): the last [`APPROACH_M`] of the bridge become a plain stretch of the same family,
//!   named `<id>/approach<n>`, joined to the junction. Only if the bridge stays at least three times [`APPROACH_M`] long
//!   (`"bridge_too_short"` otherwise) and no road passes under the part that is cut off (`"road_below"`).
//!
//! A bridge end with only two stretch ends is not touched and not reported. The ends are handled in the order of
//! the bridge ids (end 0 before end 1) on the graph as the earlier ends left it; new nodes are numbered above all
//! existing nodes in that order.

use std::collections::BTreeSet;

use super::{
    APPROACH_M, Approach, ApproachKind, NoApproach, SPECK_M, line_crossings, polyline_length,
    reaimed,
};
use crate::roadnet::{
    RoadGraph, Stretch, continuation, crossings_of, crossings_under, point_along, slice_points,
};
use crate::roadplace::JUNCTION_SPACING_M;

/// A position in map metres.
type Point = (f64, f64);

/// What one bridge end changed.
struct Change {
    stretches: Vec<Stretch>,
    kind: ApproachKind,
    moved: Vec<String>,
    at: Point,
    nodes_used: i64,
}

/// `s` with its ends in the order of their ids.
fn oriented(mut s: Stretch) -> Stretch {
    if s.start > s.end {
        std::mem::swap(&mut s.start, &mut s.end);
        s.points.reverse();
    }
    s
}

/// The points of `s` seen from its end `node` (the first point is at the node).
fn from_node(s: &Stretch, node: i64) -> Vec<Point> {
    if s.start == node {
        s.points.clone()
    } else {
        s.points.iter().rev().copied().collect()
    }
}

/// The first unused name `<id>/approach<n>`.
fn piece_id(work: &[Stretch], id: &str) -> String {
    let taken: BTreeSet<&str> = work.iter().map(|s| s.id.as_str()).collect();
    (0..=work.len())
        .map(|n| format!("{id}/approach{n}"))
        .find(|name| !taken.contains(name.as_str()))
        .unwrap_or_default()
}

/// The crossings of `line` with the stretches of `work` that share no node with `skip`.
fn crossing_count(work: &[Stretch], line: &[Point], skip: &[i64], own: &str) -> usize {
    work.iter()
        .filter(|o| o.id != own && !skip.contains(&o.start) && !skip.contains(&o.end))
        .map(|o| line_crossings(line, &o.points).len())
        .sum()
}

/// The T case: the last [`APPROACH_M`] of the deck at its end `at_start` become a plain stretch.
fn shorten(
    work: &[Stretch],
    graph: &RoadGraph,
    bi: usize,
    at_start: bool,
    next_node: i64,
) -> Result<Change, &'static str> {
    let b = work.get(bi).ok_or("no_room")?;
    let length = polyline_length(&b.points);
    if length - APPROACH_M < 3.0 * APPROACH_M - 1e-9 {
        return Err("bridge_too_short");
    }
    let (cut_from, cut_to) = if at_start {
        (0.0, APPROACH_M)
    } else {
        (length - APPROACH_M, length)
    };
    if crossings_under(graph, bi)
        .iter()
        .any(|c| c.upper_s >= cut_from && c.upper_s <= cut_to)
    {
        return Err("road_below");
    }
    let (keep, off) = if at_start {
        (
            slice_points(&b.points, APPROACH_M, length),
            slice_points(&b.points, 0.0, APPROACH_M),
        )
    } else {
        (
            slice_points(&b.points, 0.0, length - APPROACH_M),
            slice_points(&b.points, length - APPROACH_M, length),
        )
    };
    let node = if at_start { b.start } else { b.end };
    let cut = if at_start { off.last() } else { off.first() }
        .copied()
        .ok_or("no_room")?;
    let deck = Stretch {
        points: keep,
        start: if at_start { next_node } else { b.start },
        end: if at_start { b.end } else { next_node },
        ..b.clone()
    };
    let piece = Stretch {
        id: piece_id(work, &b.id),
        points: off,
        start: node,
        end: next_node,
        bridge: false,
        bridge_kind: None,
        bridge_offset: 0.0,
        layer: 0,
        ..b.clone()
    };
    // the piece runs from the junction to the cut
    let piece = Stretch {
        points: if at_start {
            piece.points
        } else {
            piece.points.iter().rev().copied().collect()
        },
        ..piece
    };
    let mut stretches: Vec<Stretch> = work
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != bi)
        .map(|(_, s)| s.clone())
        .collect();
    stretches.push(oriented(deck));
    stretches.push(oriented(piece));
    Ok(Change {
        stretches,
        kind: ApproachKind::BridgeShortened,
        moved: Vec::new(),
        at: cut,
        nodes_used: 1,
    })
}

/// The continuing stretch `c` cut at `place`, [`APPROACH_M`] from `ends[0]`, by the node `ends[1]`; its far end is `ends[2]`.
/// The piece that runs on to the far end keeps the id, the other one is the approach.
fn cut_continuation(
    work: &[Stretch],
    c: &Stretch,
    ends: [i64; 3],
    along: &[Point],
    place: Point,
) -> [Stretch; 2] {
    let length = polyline_length(along);
    let mut near = slice_points(along, 0.0, APPROACH_M);
    let mut rest = slice_points(along, APPROACH_M, length);
    if let Some(p) = near.last_mut() {
        *p = place;
    }
    if let Some(p) = rest.first_mut() {
        *p = place;
    }
    let [node, target, far] = ends;
    [
        oriented(Stretch {
            id: piece_id(work, &c.id),
            points: near,
            start: node,
            end: target,
            ..c.clone()
        }),
        oriented(Stretch {
            points: rest,
            start: target,
            end: far,
            ..c.clone()
        }),
    ]
}

/// The road-goes-on case: the roads on the deck end node `node` are joined to the continuing stretch `ci`.
fn move_side_roads(
    work: &[Stretch],
    bi: usize,
    ci: usize,
    node: i64,
    next_node: i64,
) -> Result<Change, &'static str> {
    let Some(c) = work.get(ci) else {
        return Err("no_room");
    };
    if c.start == c.end || c.bridge || c.tunnel {
        return Err("no_room");
    }
    let far = if c.start == node { c.end } else { c.start };
    let along = from_node(c, node);
    let length = polyline_length(&along);
    let far_is_deck = work
        .iter()
        .any(|s| (s.bridge || s.tunnel) && (s.start == far || s.end == far));
    // where the roads are joined: the far node if the stretch is just long enough, else a new node
    let (target, place, split) = if (length - APPROACH_M).abs() <= SPECK_M {
        if far_is_deck {
            return Err("no_room");
        }
        (far, along.last().copied().ok_or("no_room")?, false)
    } else if length >= APPROACH_M + JUNCTION_SPACING_M {
        (next_node, point_along(&along, APPROACH_M), true)
    } else {
        return Err("no_room");
    };
    let sides: Vec<usize> = work
        .iter()
        .enumerate()
        .filter(|&(i, s)| i != bi && i != ci && (s.start == node || s.end == node))
        .map(|(i, _)| i)
        .collect();
    let mut changed: Vec<(usize, Stretch)> = Vec::new();
    for &i in &sides {
        let s = work.get(i).ok_or("no_room")?;
        let other = if s.start == node { s.end } else { s.start };
        if s.start == s.end || s.bridge || s.tunnel || other == target {
            return Err("no_room");
        }
        let mut back = from_node(s, node);
        back = reaimed(&back, place);
        let moved = if s.start == node {
            Stretch {
                start: target,
                points: back,
                ..s.clone()
            }
        } else {
            back.reverse();
            Stretch {
                end: target,
                points: back,
                ..s.clone()
            }
        };
        let skip = [node, other, target, far];
        if crossing_count(work, &moved.points, &skip, &s.id)
            > crossing_count(work, &s.points, &skip, &s.id)
        {
            return Err("no_room");
        }
        changed.push((i, oriented(moved)));
    }
    let mut stretches: Vec<Stretch> = work
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != ci && !changed.iter().any(|(k, _)| k == i))
        .map(|(_, s)| s.clone())
        .collect();
    stretches.extend(changed.iter().map(|(_, s)| s.clone()));
    if split {
        stretches.extend(cut_continuation(
            work,
            c,
            [node, target, far],
            &along,
            place,
        ));
    } else {
        stretches.push(c.clone());
    }
    let moved = sides
        .iter()
        .filter_map(|&i| work.get(i).map(|s| s.id.clone()))
        .collect();
    Ok(Change {
        stretches,
        kind: ApproachKind::SideRoadsMoved,
        moved,
        at: place,
        nodes_used: i64::from(split),
    })
}

/// The step (see the module).
pub(super) fn make_approaches(graph: &RoadGraph) -> (RoadGraph, Vec<Approach>, Vec<NoApproach>) {
    let mut work: Vec<Stretch> = graph.stretches.clone();
    work.sort_by(|a, b| a.id.cmp(&b.id));
    // the ends of the bridges as the graph gives them: a cut makes a new node and may turn a bridge round, so an
    // end is followed by its node
    let mut targets: Vec<(String, u8, i64)> = work
        .iter()
        .filter(|s| s.bridge && !s.tunnel && s.start != s.end)
        .flat_map(|s| [(s.id.clone(), 0_u8, s.start), (s.id.clone(), 1_u8, s.end)])
        .collect();
    targets.sort();
    let mut next_node = work
        .iter()
        .flat_map(|s| [s.start, s.end])
        .max()
        .map_or(1, |m| m.saturating_add(1));
    let mut done = Vec::new();
    let mut refused = Vec::new();
    let mut changed = false;
    for (id, end, node) in targets {
        let Some(bi) = work.iter().position(|s| s.id == id) else {
            continue;
        };
        let Some(b) = work.get(bi) else { continue };
        if b.start != node && b.end != node {
            continue;
        }
        let at_start = b.start == node;
        let Some(at) = (if at_start {
            b.points.first()
        } else {
            b.points.last()
        })
        .copied() else {
            continue;
        };
        let arms: usize = work
            .iter()
            .map(|s| usize::from(s.start == node) + usize::from(s.end == node))
            .sum();
        if arms < 3 {
            continue;
        }
        let now = RoadGraph {
            crossings: crossings_of(&work),
            stretches: work.clone(),
        };
        let verdict = match continuation(&now, bi, at_start) {
            Some((ci, _)) => move_side_roads(&work, bi, ci, node, next_node),
            None => shorten(&work, &now, bi, at_start, next_node),
        };
        match verdict {
            Ok(change) => {
                work = change.stretches;
                work.sort_by(|a, b| a.id.cmp(&b.id));
                next_node = next_node.saturating_add(change.nodes_used);
                changed = true;
                done.push(Approach {
                    bridge: id,
                    end,
                    kind: change.kind,
                    moved: change.moved,
                    at: change.at,
                });
            }
            Err(reason) => refused.push(NoApproach {
                bridge: id,
                end,
                reason,
                at,
            }),
        }
    }
    if !changed {
        return (graph.clone(), done, refused);
    }
    let crossings = crossings_of(&work);
    (
        RoadGraph {
            stretches: work,
            crossings,
        },
        done,
        refused,
    )
}

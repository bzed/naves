// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The clean-up of the road graph before anything is built on it: sidepaths, stubs, close junctions and bridge
//! approaches, in this order (see `docs/superpowers/specs/2026-11-18-network-cleanup-design.md`).
//!
//! Every step is `fn(&RoadGraph, ...) -> (RoadGraph, reports)`, deterministic, and never makes the graph fall
//! apart: the number of its connected components never grows. A step that is switched off (its `..._ENABLED`
//! constant) returns the graph unchanged, bit for bit. The steps are still empty here: they return the graph
//! as it is and report nothing.

use std::collections::BTreeMap;

use crate::catalog::Family;
use crate::roadnet::{Ring, RoadGraph};

/// The furthest a sidepath runs from its road, in map metres.
pub const SIDEPATH_MAX_M: f64 = 15.0;
/// The shortest parallel run that counts as a sidepath without a tag that says so, in map metres.
pub const SIDEPATH_MIN_RUN_M: f64 = 30.0;
/// The longest dead-end stub that is removed, in map metres.
pub const STUB_MAX_M: f64 = 6.0;
/// A piece shorter than this is a speck, in map metres.
pub const SPECK_M: f64 = 0.5;
/// Junctions closer than this are merged into one, in map metres.
pub const JUNCTION_MERGE_M: f64 = 6.0;
/// How far from a bridge's end a junction is moved, in map metres.
pub const APPROACH_M: f64 = 8.0;

/// Whether sidepaths are removed (step 1).
pub const SIDEPATHS_ENABLED: bool = true;
/// Whether stubs are removed (step 2).
pub const STUBS_ENABLED: bool = true;
/// Whether close junctions are merged (step 3).
pub const MERGE_ENABLED: bool = true;
/// Whether bridge approaches are made (step 4).
pub const APPROACHES_ENABLED: bool = true;

/// A part of a stretch that runs beside a road and was removed.
#[derive(Debug, Clone, PartialEq)]
pub struct DroppedSidepath {
    /// The id of the stretch the part was removed from.
    pub stretch: String,
    /// The id of the road it ran beside.
    pub road: String,
    /// Where the removed part starts, in metres along the stretch.
    pub from_m: f64,
    /// Where it ends.
    pub to_m: f64,
    /// The middle of the removed part, in map metres.
    pub at: (f64, f64),
}

/// A short dead end that was removed.
#[derive(Debug, Clone, PartialEq)]
pub struct DroppedStub {
    /// The id of the stretch.
    pub stretch: String,
    /// Its length in metres.
    pub length_m: f64,
    /// Its middle, in map metres.
    pub at: (f64, f64),
}

/// Close junctions that were made one.
#[derive(Debug, Clone, PartialEq)]
pub struct MergedJunction {
    /// The node that stays.
    pub kept: i64,
    /// The nodes that went into it.
    pub merged: Vec<i64>,
    /// The ids of the stretches that were removed on the way.
    pub removed: Vec<String>,
    /// Where the junction is now, in map metres.
    pub at: (f64, f64),
}

/// A junction that was moved off the end of a bridge.
#[derive(Debug, Clone, PartialEq)]
pub struct Approach {
    /// The id of the bridge stretch.
    pub bridge: String,
    /// Which end: 0 for its start, 1 for its end.
    pub end: u8,
    /// Where the approach is, in map metres.
    pub at: (f64, f64),
}

/// What the clean-up did. Nothing is changed that is not reported.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CleanReport {
    /// Parts of stretches removed as sidepaths of a road.
    pub sidepaths: Vec<DroppedSidepath>,
    /// Dead-end stubs removed.
    pub stubs: Vec<DroppedStub>,
    /// Junctions merged.
    pub merged: Vec<MergedJunction>,
    /// Bridge ends that got an approach.
    pub approaches: Vec<Approach>,
    /// Roundabouts that were turned into plain junctions (set by the caller that collapses them).
    pub roundabouts: Vec<Ring>,
    /// The connected components of the graph before the clean-up.
    pub components_before: usize,
    /// The connected components of the graph after it.
    pub components_after: usize,
}

/// The root of `i` in the union-find `parent`.
fn root(parent: &mut [usize], mut i: usize) -> usize {
    while let Some(&p) = parent.get(i) {
        if p == i {
            break;
        }
        // path halving
        let up = parent.get(p).copied().unwrap_or(p);
        if let Some(slot) = parent.get_mut(i) {
            *slot = up;
        }
        i = up;
    }
    i
}

/// The number of connected components of the graph: stretches that share a node are in one. An empty graph has
/// none.
#[must_use]
pub fn components(graph: &RoadGraph) -> usize {
    let mut index: BTreeMap<i64, usize> = BTreeMap::new();
    for s in &graph.stretches {
        for node in [s.start, s.end] {
            let next = index.len();
            index.entry(node).or_insert(next);
        }
    }
    let mut parent: Vec<usize> = (0..index.len()).collect();
    for s in &graph.stretches {
        if let (Some(&a), Some(&b)) = (index.get(&s.start), index.get(&s.end)) {
            let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
            if let Some(slot) = parent.get_mut(ra.max(rb)) {
                *slot = ra.min(rb);
            }
        }
    }
    (0..parent.len())
        .filter(|&i| root(&mut parent, i) == i)
        .count()
}

/// Step 1: the parts of stretches that run beside a road of higher rank and disturb it. Not done yet.
#[must_use]
pub fn drop_sidepaths(
    graph: &RoadGraph,
    _widths: &dyn Fn(Family) -> f64,
    _cell_m: f64,
) -> (RoadGraph, Vec<DroppedSidepath>) {
    (graph.clone(), Vec::new())
}

/// Step 2: short dead ends. Not done yet.
#[must_use]
pub fn drop_stubs(
    graph: &RoadGraph,
    _widths: &dyn Fn(Family) -> f64,
) -> (RoadGraph, Vec<DroppedStub>) {
    (graph.clone(), Vec::new())
}

/// Step 3: junctions closer than [`JUNCTION_MERGE_M`]. Not done yet.
#[must_use]
pub fn merge_junctions(graph: &RoadGraph) -> (RoadGraph, Vec<MergedJunction>) {
    (graph.clone(), Vec::new())
}

/// Step 4: a junction on the end of a bridge is moved [`APPROACH_M`] off the deck. Not done yet.
#[must_use]
pub fn make_approaches(graph: &RoadGraph) -> (RoadGraph, Vec<Approach>) {
    (graph.clone(), Vec::new())
}

/// Cleans the graph: sidepaths, stubs, close junctions and bridge approaches, in this order. `widths` is the
/// road width of a family and `cell_m` the cell of the terrain, both in map metres. The report's `roundabouts`
/// are left empty; whoever collapsed the roundabouts sets them.
#[must_use]
pub fn clean(
    graph: &RoadGraph,
    widths: &dyn Fn(Family) -> f64,
    cell_m: f64,
) -> (RoadGraph, CleanReport) {
    let (graph1, sidepaths) = drop_sidepaths(graph, widths, cell_m);
    let (graph2, stubs) = drop_stubs(&graph1, widths);
    let (graph3, merged) = merge_junctions(&graph2);
    let (graph4, approaches) = make_approaches(&graph3);
    let report = CleanReport {
        sidepaths,
        stubs,
        merged,
        approaches,
        roundabouts: Vec::new(),
        components_before: components(graph),
        components_after: components(&graph4),
    };
    (graph4, report)
}

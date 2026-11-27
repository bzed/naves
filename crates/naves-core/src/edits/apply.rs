// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Applying the way edits to the OSM ways: the edits win (spec 5.2).

use std::collections::{BTreeMap, BTreeSet};

use super::ElementId;
use super::ways::{WayAction, WayEdit, WayEdits, fingerprint};
use crate::osm::{OsmWay, OsmWays};

/// Something about an edit that the user should look at. The edit is applied anyway unless it is `Broken`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditIssue {
    /// A way of the edits uses a node that has no position: no node edit and no way of the extract has it. The
    /// way is left out.
    Broken {
        /// The edited way.
        way: ElementId,
        /// The first node without a position.
        node: ElementId,
    },
    /// The OSM original of a `modify` or `delete` is not what it was when the edit was made.
    OriginalChanged {
        /// The edited way.
        way: ElementId,
    },
    /// The OSM original of a `modify` or `delete` is not in the extract any more.
    OriginalGone {
        /// The edited way.
        way: ElementId,
    },
}

/// What [`apply_way_edits`] did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EditReport {
    /// How many edits took effect: way edits that added a way, modified one, or deleted a way of the extract
    /// (a broken edit and a delete of a way that is gone do not count), plus OSM node edits that moved a node of
    /// a way in the result.
    pub applied: usize,
    /// The issues, in way id order, each with a `(lat, lon)` to show it at: the first known node of the
    /// way. If the way has no known node, the position of its first node edit; if it has none, `(0.0, 0.0)`.
    pub issues: Vec<(EditIssue, (f64, f64))>,
}

/// Applies `edits` to `ways`; the edits win over the extract.
///
/// Node edits replace the position of that node id in every way. `delete` removes a way, `modify` replaces its
/// tags and nodes, `add` inserts a way. The positions of an edited way's nodes come from the node edits, else
/// from any way of the extract that has the node (also one that the edits delete), else the way is
/// [`EditIssue::Broken`] and left out. A `modify` or `delete` whose original is not in the extract is
/// [`EditIssue::OriginalGone`], one whose original differs from its `base` fingerprint is
/// [`EditIssue::OriginalChanged`]; both are applied. The result is in id order (user ways, with raw ids, after
/// the OSM ways), so it does not depend on the order of the files; `skipped` is carried over.
#[must_use]
pub fn apply_way_edits(ways: &OsmWays, edits: &WayEdits) -> (OsmWays, EditReport) {
    let mut known: BTreeMap<i64, (f64, f64)> = BTreeMap::new();
    for way in &ways.ways {
        for (&node, &point) in way.nodes.iter().zip(&way.points) {
            known.entry(node).or_insert(point);
        }
    }
    let moved: BTreeMap<i64, (f64, f64)> = edits
        .nodes
        .iter()
        .filter_map(|n| Some((n.id.raw()?, (n.lat, n.lon))))
        .collect();
    let position = |node: i64| moved.get(&node).or_else(|| known.get(&node)).copied();

    let by_id: BTreeMap<i64, &WayEdit> = edits
        .ways
        .iter()
        .filter_map(|w| Some((w.id.raw()?, w)))
        .collect();
    let originals: BTreeMap<i64, &OsmWay> = ways.ways.iter().map(|w| (w.id, w)).collect();

    let mut out: BTreeMap<i64, OsmWay> = BTreeMap::new();
    for way in &ways.ways {
        if by_id.contains_key(&way.id) {
            continue;
        }
        let points = way
            .nodes
            .iter()
            .zip(&way.points)
            .map(|(node, &point)| moved.get(node).copied().unwrap_or(point))
            .collect();
        out.insert(
            way.id,
            OsmWay {
                points,
                ..way.clone()
            },
        );
    }

    let mut report = EditReport::default();
    for (&raw, &edit) in &by_id {
        let original = originals.get(&raw).copied();
        let place = |edit: &WayEdit| {
            edit.nodes
                .iter()
                .find_map(|n| position(n.raw()?))
                .or_else(|| {
                    edit.nodes
                        .iter()
                        .find_map(|n| moved.get(&n.raw()?).copied())
                })
                .or_else(|| original.and_then(|o| o.points.first().copied()))
                .unwrap_or((0.0, 0.0))
        };
        if edit.action != WayAction::Add
            && let Some(issue) = original_issue(edit, original)
        {
            report.issues.push((issue, place(edit)));
        }
        match edit.action {
            WayAction::Delete => {
                if original.is_some() {
                    report.applied += 1;
                }
            }
            WayAction::Add | WayAction::Modify => match resolve(raw, edit, &position) {
                Err(node) => report
                    .issues
                    .push((EditIssue::Broken { way: edit.id, node }, place(edit))),
                Ok(way) => {
                    report.applied += 1;
                    out.insert(raw, way);
                }
            },
        }
    }

    let used: BTreeSet<i64> = out
        .values()
        .flat_map(|w| w.nodes.iter().copied())
        .filter(|n| *n < super::USER_ID_BASE && moved.contains_key(n))
        .collect();
    report.applied += used.len();

    (
        OsmWays {
            ways: out.into_values().collect(),
            skipped: ways.skipped,
        },
        report,
    )
}

/// The way of `edit` with its positions, or the first node that has none.
fn resolve(
    raw: i64,
    edit: &WayEdit,
    position: &impl Fn(i64) -> Option<(f64, f64)>,
) -> Result<OsmWay, ElementId> {
    let mut way = OsmWay {
        id: raw,
        tags: edit.tags.clone(),
        nodes: Vec::with_capacity(edit.nodes.len()),
        points: Vec::with_capacity(edit.nodes.len()),
    };
    for &id in &edit.nodes {
        let (n, point) = id.raw().and_then(|n| Some((n, position(n)?))).ok_or(id)?;
        way.nodes.push(n);
        way.points.push(point);
    }
    Ok(way)
}

/// The issue with the original of a `modify` or `delete`, if there is one.
fn original_issue(edit: &WayEdit, original: Option<&OsmWay>) -> Option<EditIssue> {
    let way = edit.id;
    match original {
        None => Some(EditIssue::OriginalGone { way }),
        Some(o) => edit
            .base
            .as_ref()
            .is_some_and(|base| *base != fingerprint(&o.tags, &o.nodes))
            .then_some(EditIssue::OriginalChanged { way }),
    }
}

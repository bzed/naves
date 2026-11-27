// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Applying the way edits to the OSM ways: the edits win (spec 5.2).

use std::collections::{BTreeMap, BTreeSet};

use super::ElementId;
use super::ways::{WayAction, WayEdit, WayEdits, fingerprint};
use crate::osm::{LotPolygon, OsmWay, OsmWays, lot_tags_of};

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

/// Whether `edit` is a closed way (at least three corners and the first node again) whose tags make a lot.
fn lot_tags_of_edit(edit: &WayEdit) -> Option<BTreeMap<String, String>> {
    let closed = edit.nodes.len() >= 4 && edit.nodes.first() == edit.nodes.last();
    if edit.action == WayAction::Delete || !closed {
        return None;
    }
    lot_tags_of(&edit.tags)
}

/// Applies the way edits to the lots (car parks, camp sites and similar areas) of the extract; the edits win.
///
/// A `modify` or `add` of a closed way (at least three corners, the first node again) whose tags make a lot
/// ([`lot_tags_of`]) replaces the way lot of that id (`relation == false`, part 0) or adds one, with its outline as
/// drawn and no holes. A `modify` of a lot id whose new tags make no lot, or a `delete`, removes the way lot of that
/// id. Relations are not editable, and a way edit that has nothing to do with a lot leaves the lots alone. The result
/// is in the order of [`OsmLand::lots`](crate::osm::OsmLand::lots): ways in id order (user ways after the OSM ones),
/// then the relations as they were.
///
/// The ring of a [`LotPolygon`] has no node ids, so an edit's nodes are resolved like those of
/// [`apply_way_edits`]: from the node edits first, then from `ways`, the highway and waterway ways of the extract.
/// The nodes of a lot's ring are in none of those, so **a lot edit needs a node edit for every node of its
/// outline** (the editor writes them); a node without a position makes the edit [`EditIssue::Broken`] and it is
/// left out. A node edit does not move the ring of a lot that is not edited.
///
/// The report is for the build to put together with that of [`apply_way_edits`] ([`EditReport::absorb_lots`]), which
/// has already counted every `add` and `modify` and every `delete` of a way of `ways`: so `applied` here counts
/// only the `delete`s of a lot that is not in `ways`, and the issues are the `Broken` ones and
/// [`EditIssue::OriginalGone`] for a `delete` (or a lot `modify`) of an id that is neither a lot nor in `ways`.
/// There is no [`EditIssue::OriginalChanged`] for a lot: a lot has no node ids to fingerprint.
#[must_use]
pub fn apply_lot_edits(
    lots: &[LotPolygon],
    ways: &OsmWays,
    edits: &WayEdits,
) -> (Vec<LotPolygon>, EditReport) {
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
    let in_ways: BTreeSet<i64> = ways.ways.iter().map(|w| w.id).collect();

    let mut way_lots: BTreeMap<i64, LotPolygon> = lots
        .iter()
        .filter(|l| !l.relation)
        .map(|l| (l.id, l.clone()))
        .collect();
    let existing: BTreeSet<i64> = way_lots.keys().copied().collect();
    let mut report = EditReport::default();

    let mut by_id: Vec<(i64, &WayEdit)> = edits
        .ways
        .iter()
        .filter_map(|w| Some((w.id.raw()?, w)))
        .collect();
    by_id.sort_by_key(|(raw, _)| *raw);
    for (raw, edit) in by_id {
        let lot_tags = lot_tags_of_edit(edit);
        let was_lot = existing.contains(&raw);
        let place = |nodes: &[ElementId]| {
            nodes
                .iter()
                .find_map(|n| position(n.raw()?))
                .unwrap_or((0.0, 0.0))
        };
        match (edit.action, lot_tags) {
            (WayAction::Delete, _) | (WayAction::Modify, None) => {
                if way_lots.remove(&raw).is_some() {
                    if edit.action == WayAction::Delete && !in_ways.contains(&raw) {
                        report.applied += 1;
                    }
                } else if !was_lot && !in_ways.contains(&raw) && edit.action == WayAction::Delete {
                    report
                        .issues
                        .push((EditIssue::OriginalGone { way: edit.id }, place(&edit.nodes)));
                }
            }
            (WayAction::Add, None) => {}
            (action, Some(kept)) => {
                if action == WayAction::Modify && !was_lot && !in_ways.contains(&raw) {
                    report
                        .issues
                        .push((EditIssue::OriginalGone { way: edit.id }, place(&edit.nodes)));
                }
                let positions: Vec<Option<(f64, f64)>> = edit
                    .nodes
                    .iter()
                    .map(|id| id.raw().and_then(position))
                    .collect();
                let missing = positions
                    .iter()
                    .position(Option::is_none)
                    .and_then(|at| edit.nodes.get(at).copied());
                let outer: Vec<(f64, f64)> = positions.into_iter().flatten().collect();
                if let Some(node) = missing {
                    report
                        .issues
                        .push((EditIssue::Broken { way: edit.id, node }, place(&edit.nodes)));
                    // a broken edit leaves the way out: the lot it would have replaced is gone too
                    way_lots.remove(&raw);
                } else {
                    way_lots.insert(
                        raw,
                        LotPolygon {
                            id: raw,
                            relation: false,
                            part: 0,
                            tags: kept,
                            outer,
                            holes: Vec::new(),
                        },
                    );
                }
            }
        }
    }

    let mut out: Vec<LotPolygon> = way_lots.into_values().collect();
    out.extend(lots.iter().filter(|l| l.relation).cloned());
    (out, report)
}

impl EditReport {
    /// Puts the report of [`apply_lot_edits`] (`lots`) into this one, the report of [`apply_way_edits`]: the counts
    /// add up, the issues of `lots` are added unless this report has them already, and the
    /// [`EditIssue::OriginalGone`] this report has for an id that is a lot of `lots_before` (the lots as read,
    /// before the edits) is dropped: `apply_way_edits` only knows the highway and waterway ways, so a lot is "gone"
    /// to it. Each way is then reported once.
    pub fn absorb_lots(&mut self, lots: EditReport, lots_before: &[LotPolygon]) {
        let is_lot = |way: &ElementId| {
            way.raw()
                .is_some_and(|raw| lots_before.iter().any(|l| !l.relation && l.id == raw))
        };
        self.issues.retain(|(issue, _)| match issue {
            EditIssue::OriginalGone { way } => !is_lot(way),
            _ => true,
        });
        self.applied += lots.applied;
        for (issue, at) in lots.issues {
            if !self.issues.iter().any(|(have, _)| *have == issue) {
                self.issues.push((issue, at));
            }
        }
    }
}

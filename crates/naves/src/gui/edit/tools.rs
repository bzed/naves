// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The tools of the Roads level: Move, Draw, Cut, Join and Bridge, and Delete. Every tool turns its events into at
//! most one [`NavesApp::edit_way`] call, so one gesture is one undo step; a click that finds nothing to work on
//! changes nothing.

use std::collections::BTreeMap;

use naves_core::edits::{By, EditsError, ElementId, USER_ID_BASE};
use naves_core::osm::OsmWay;

use super::hit::{Target, nodes_at};
use crate::gui::NavesApp;
use crate::gui::levels::roads::ROAD_CLASSES;
use crate::gui::shortcuts::ToolId;
use crate::gui::viewport::{ViewEvent, WorldPoint};

/// The tag of a bridge, and the tag that puts it above the ground.
const BRIDGE: &str = "bridge";
const LAYER: &str = "layer";
/// The tag that keeps a way as drawn; it does not make two ways different.
const KEEP: &str = "naves:keep";
/// The prefix of the id of a road class item.
const CLASS_PREFIX: &str = "roads.class.";

/// The line Draw is drawing: its points in map metres, each with the node it joins, if any.
#[derive(Debug, Default)]
pub struct Drawing {
    /// The points so far.
    pub points: Vec<(WorldPoint, Option<i64>)>,
}

/// A node being moved.
#[derive(Debug, Clone, Copy)]
struct Drag {
    node: i64,
    at: WorldPoint,
}

/// The first click of Join or Bridge.
#[derive(Debug, Clone, Copy)]
struct First {
    tool: ToolId,
    node: i64,
    way: i64,
    at: WorldPoint,
}

/// What a tool holds between two events.
#[derive(Debug, Default)]
pub struct Pending {
    drag: Option<Drag>,
    first: Option<First>,
}

impl Pending {
    /// Where the handles of what is held are, for the overlay.
    #[must_use]
    pub fn marks(&self) -> Vec<WorldPoint> {
        self.drag
            .iter()
            .map(|d| d.at)
            .chain(self.first.iter().map(|f| f.at))
            .collect()
    }

    fn clear(&mut self) {
        self.drag = None;
        self.first = None;
    }
}

/// Forgets what a tool holds when another tool is in use. Called every frame.
pub fn sync(app: &mut NavesApp) {
    if app.tool != ToolId::LINE {
        app.drawing.points.clear();
    }
    if app.tool != ToolId::MOVE {
        app.pending.drag = None;
    }
    if app.pending.first.is_some_and(|f| f.tool != app.tool) {
        app.pending.first = None;
    }
}

/// Esc: drops what is held. `true` when there was something, so that the tool stays.
pub fn cancel(app: &mut NavesApp) -> bool {
    let held =
        !app.drawing.points.is_empty() || app.pending.drag.is_some() || app.pending.first.is_some();
    app.drawing.points.clear();
    app.pending.clear();
    held
}

/// Ctrl+Z or Backspace while drawing: takes the last point back. `false` when nothing is being drawn.
pub fn take_point_back(app: &mut NavesApp) -> bool {
    app.drawing.points.pop().is_some()
}

fn primary(event: &ViewEvent) -> Option<WorldPoint> {
    match *event {
        ViewEvent::Click { at, .. } => Some(at),
        _ => None,
    }
}

/// The ways that have the node a click at `at` hits, as `(node, way)` in id order; empty when no node is hit.
fn nodes_hit(app: &NavesApp, at: WorldPoint) -> Vec<(i64, i64)> {
    app.edit
        .session()
        .map(|s| nodes_at(s, at, app.hit_tolerance_m()))
        .unwrap_or_default()
}

/// The node a click at `at` hits, with the first way that has it.
fn node_at(app: &NavesApp, at: WorldPoint) -> Option<(i64, i64)> {
    nodes_hit(app, at).into_iter().next()
}

/// Whether `node` is the first or the last node of `way`.
fn is_end(app: &NavesApp, way: i64, node: i64) -> bool {
    app.edit
        .session()
        .and_then(|s| s.way(way))
        .is_some_and(|w| w.nodes.first() == Some(&node) || w.nodes.last() == Some(&node))
}

/// A way of the session, with the edits applied, and as the extract has it.
fn way_pair(app: &NavesApp, way: i64) -> Option<(OsmWay, OsmWay)> {
    let session = app.edit.session()?;
    Some((session.way(way)?.clone(), session.original(way)?.clone()))
}

fn ids(nodes: &[i64]) -> Result<Vec<ElementId>, EditsError> {
    nodes
        .iter()
        .map(|&n| {
            ElementId::from_raw(n).ok_or_else(|| EditsError::Invalid {
                what: format!("node {n}"),
                problem: "id cannot be used",
            })
        })
        .collect()
}

/// The position of a node in map metres.
fn node_position(app: &NavesApp, node: i64) -> Option<WorldPoint> {
    let session = app.edit.session()?;
    session.source.ways.ways.iter().find_map(|w| {
        let i = w.nodes.iter().position(|&n| n == node)?;
        let &(lat, lon) = w.points.get(i)?;
        let (x, y) = session.source.frame.to_map(lat, lon).ok()?;
        Some(WorldPoint { x, y })
    })
}

/// The OSM position of a node: where the extract has it.
fn osm_position(app: &NavesApp, node: i64) -> Option<(f64, f64)> {
    if node >= USER_ID_BASE {
        return None;
    }
    let session = app.edit.session()?;
    session.source.extract.ways.iter().find_map(|w| {
        let i = w.nodes.iter().position(|&n| n == node)?;
        w.points.get(i).copied()
    })
}

fn same_class(a: &BTreeMap<String, String>, b: &BTreeMap<String, String>) -> bool {
    let strip = |t: &BTreeMap<String, String>| {
        t.iter()
            .filter(|(k, _)| k.as_str() != KEEP)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    strip(a) == strip(b)
}

/// Move: press on a node, drag, release. Esc during the drag cancels it.
pub fn move_event(app: &mut NavesApp, event: &ViewEvent) {
    match *event {
        ViewEvent::Press { at, .. } => {
            app.pending.drag = node_at(app, at).map(|(node, _)| Drag { node, at });
        }
        ViewEvent::Drag { at, .. } => {
            if let Some(drag) = &mut app.pending.drag {
                drag.at = at;
            }
        }
        ViewEvent::Release { at, .. } => {
            if let Some(drag) = app.pending.drag.take() {
                move_node(app, drag.node, at);
            }
        }
        _ => {}
    }
}

fn move_node(app: &mut NavesApp, node: i64, to: WorldPoint) {
    let Some(from) = node_position(app, node) else {
        return;
    };
    if (from.x - to.x).hypot(from.y - to.y) < 0.01 {
        return; // not moved
    }
    let Some(id) = ElementId::from_raw(node) else {
        return;
    };
    let osm_at = osm_position(app, node);
    let Some(session) = app.edit.session() else {
        return;
    };
    let (lat, lon) = match session.source.frame.to_geo(to.x, to.y) {
        Ok(geo) => geo,
        Err(e) => {
            app.set_edit_problem(format!("The edit cannot be made: {e}"));
            return;
        }
    };
    app.edit_way(|edits, _| edits.move_node(id, lat, lon, osm_at, By::User))
        .ok(); // an error is a problem already
}

/// The tags of a drawn way: the class of the picked item, else a residential street.
fn drawing_tags(item: Option<&str>) -> BTreeMap<String, String> {
    let class = item
        .and_then(|i| i.strip_prefix(CLASS_PREFIX))
        .and_then(|id| ROAD_CLASSES.iter().find(|c| c.id == id))
        .or_else(|| ROAD_CLASSES.iter().find(|c| c.id == "residential"));
    class
        .map(|c| c.tags)
        .unwrap_or_default()
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// Draw: each click adds a point (a node within reach is joined); Space, Enter, a right click or a double click
/// ends the line.
pub fn draw_event(app: &mut NavesApp, event: &ViewEvent) {
    match *event {
        ViewEvent::Click { at, .. } => add_point(app, at),
        ViewEvent::SecondaryClick { .. } | ViewEvent::DoubleClick { .. } => finish_drawing(app),
        _ => {}
    }
}

fn add_point(app: &mut NavesApp, at: WorldPoint) {
    let joined = node_at(app, at).map(|(node, _)| node);
    let point = joined
        .and_then(|node| node_position(app, node))
        .unwrap_or(at);
    if let Some(&(last, last_node)) = app.drawing.points.last() {
        // the second click of a double click, or a click on the node just placed
        let near = (last.x - point.x).hypot(last.y - point.y) <= app.hit_tolerance_m();
        if near || (joined.is_some() && joined == last_node) {
            return;
        }
    }
    app.drawing.points.push((point, joined));
}

/// Ends the line: a node for each new point and the way with the tags of the class, in one step. With fewer than
/// two points nothing happens and the point stays.
pub fn finish_drawing(app: &mut NavesApp) {
    if app.drawing.points.len() < 2 {
        return;
    }
    let Some(session) = app.edit.session() else {
        return;
    };
    let mut geo = Vec::new();
    for (point, node) in &app.drawing.points {
        match (node, session.source.frame.to_geo(point.x, point.y)) {
            (Some(node), _) => geo.push((Some(*node), (0.0, 0.0))),
            (None, Ok(position)) => geo.push((None, position)),
            (None, Err(e)) => {
                app.set_edit_problem(format!("The edit cannot be made: {e}"));
                return;
            }
        }
    }
    let tags = drawing_tags(app.item.as_deref());
    let result = app.edit_way(|edits, _| {
        let mut nodes = Vec::new();
        for &(node, (lat, lon)) in &geo {
            nodes.push(match node {
                Some(node) => ElementId::from_raw(node).ok_or_else(|| EditsError::Invalid {
                    what: format!("node {node}"),
                    problem: "id cannot be used",
                })?,
                None => edits.new_node(lat, lon, By::User)?,
            });
        }
        edits.add_way(tags, nodes, By::User).map(|_| ())
    });
    if result.is_ok() {
        app.drawing.points.clear();
    }
}

/// Cut: a click on an inner node ends the way there and starts a new one with the same tags.
pub fn cut_event(app: &mut NavesApp, event: &ViewEvent) {
    let Some(at) = primary(event) else {
        return;
    };
    // at a junction: a way the node is inside of
    let Some((i, current, original)) = nodes_hit(app, at).into_iter().find_map(|(node, way)| {
        let (current, original) = way_pair(app, way)?;
        let i = current.nodes.iter().position(|&n| n == node)?;
        // an end is no place to cut
        (i > 0 && i + 1 < current.nodes.len()).then_some((i, current, original))
    }) else {
        return;
    };
    app.edit_way(|edits, _| {
        let all = ids(&current.nodes)?;
        edits.modify_way(
            &original,
            current.tags.clone(),
            all.get(..=i).unwrap_or_default().to_vec(),
            By::User,
        )?;
        edits
            .add_way(
                current.tags.clone(),
                all.get(i..).unwrap_or_default().to_vec(),
                By::User,
            )
            .map(|_| ())
    })
    .ok(); // an error is a problem already
}

/// Join: the end of one road, then the end of another.
pub fn join_event(app: &mut NavesApp, event: &ViewEvent) {
    let Some(at) = primary(event) else {
        return;
    };
    // at a junction: the ways the node is an end of; the second click takes another way than the first
    let ends: Vec<(i64, i64)> = nodes_hit(app, at)
        .into_iter()
        .filter(|&(node, way)| is_end(app, way, node))
        .collect();
    let first = app.pending.first.filter(|f| f.tool == ToolId::JOIN);
    let Some(&(node, way)) = ends
        .iter()
        .find(|&&(_, way)| first.is_none_or(|f| f.way != way))
        .or(ends.first())
    else {
        return; // not an end
    };
    let Some(first) = first else {
        app.pending.first = Some(First {
            tool: ToolId::JOIN,
            node,
            way,
            at,
        });
        return;
    };
    if first.way == way {
        // the same road: the new click replaces the first
        app.pending.first = Some(First {
            tool: ToolId::JOIN,
            node,
            way,
            at,
        });
        return;
    }
    app.pending.first = None;
    join(app, (first.node, first.way), (node, way));
}

fn join(app: &mut NavesApp, a: (i64, i64), b: (i64, i64)) {
    let (Some((a_way, a_original)), Some((b_way, b_original))) =
        (way_pair(app, a.1), way_pair(app, b.1))
    else {
        return;
    };
    // A ends at its node, B starts at its node
    let mut a_nodes = a_way.nodes.clone();
    if a_nodes.last() != Some(&a.0) {
        a_nodes.reverse();
    }
    let mut b_nodes = b_way.nodes.clone();
    if b_nodes.first() != Some(&b.0) {
        b_nodes.reverse();
    }
    let same = same_class(&a_way.tags, &b_way.tags);
    if !same && a.0 == b.0 {
        return; // they meet already
    }
    app.edit_way(|edits, _| {
        if same {
            a_nodes.extend(b_nodes.iter().skip(1));
            edits.modify_way(&a_original, a_way.tags.clone(), ids(&a_nodes)?, By::User)?;
            edits.delete_way(&b_original, By::User)
        } else {
            // B's end becomes A's end; the rest of B stays as it is
            let joined: Vec<i64> = b_way
                .nodes
                .iter()
                .map(|&n| if n == b.0 { a.0 } else { n })
                .collect();
            edits.modify_way(&b_original, b_way.tags.clone(), ids(&joined)?, By::User)
        }
    })
    .ok(); // an error is a problem already
}

/// Bridge: two nodes of a road make the part between them a bridge; a click on a bridge makes it a road again.
pub fn bridge_event(app: &mut NavesApp, event: &ViewEvent) {
    let Some(at) = primary(event) else {
        return;
    };
    // at a junction: the way of the first click if it has the node, else the first way that has it
    let hits = nodes_hit(app, at);
    let pending = app.pending.first.filter(|f| f.tool == ToolId::BRIDGE);
    let Some(&(node, way)) = hits
        .iter()
        .find(|&&(_, way)| pending.is_some_and(|f| f.way == way))
        .or(hits.first())
    else {
        return;
    };
    let Some((current, original)) = way_pair(app, way) else {
        return;
    };
    if current.tags.get(BRIDGE).map(String::as_str) == Some("yes") {
        app.pending.first = None;
        let mut tags = current.tags.clone();
        tags.remove(BRIDGE);
        tags.remove(LAYER);
        let nodes = current.nodes.clone();
        app.edit_way(|edits, _| edits.modify_way(&original, tags, ids(&nodes)?, By::User))
            .ok(); // an error is a problem already
        return;
    }
    let here = First {
        tool: ToolId::BRIDGE,
        node,
        way,
        at,
    };
    let Some(first) = app
        .pending
        .first
        .filter(|f| f.tool == ToolId::BRIDGE && f.way == way && f.node != node)
    else {
        app.pending.first = Some(here);
        return;
    };
    app.pending.first = None;
    let positions = (
        current.nodes.iter().position(|&n| n == first.node),
        current.nodes.iter().position(|&n| n == node),
    );
    let (Some(p), Some(q)) = positions else {
        return;
    };
    let (i, j) = (p.min(q), p.max(q));
    make_bridge(app, &current, &original, i, j);
}

/// Splits `current` at the nodes `i` and `j`; the part between is the bridge. The first part keeps the id.
fn make_bridge(app: &mut NavesApp, current: &OsmWay, original: &OsmWay, i: usize, j: usize) {
    let last = current.nodes.len().saturating_sub(1);
    let mut bridge_tags = current.tags.clone();
    bridge_tags.insert(BRIDGE.to_string(), "yes".to_string());
    bridge_tags.insert(LAYER.to_string(), "1".to_string());
    app.edit_way(|edits, _| {
        let all = ids(&current.nodes)?;
        let part = |from: usize, to: usize| all.get(from..=to).unwrap_or_default().to_vec();
        // the pieces in order, with their tags
        let mut pieces = Vec::new();
        if i > 0 {
            pieces.push((current.tags.clone(), part(0, i)));
        }
        pieces.push((bridge_tags.clone(), part(i, j)));
        if j < last {
            pieces.push((current.tags.clone(), part(j, last)));
        }
        let mut pieces = pieces.into_iter();
        if let Some((tags, nodes)) = pieces.next() {
            edits.modify_way(original, tags, nodes, By::User)?;
        }
        for (tags, nodes) in pieces {
            edits.add_way(tags, nodes, By::User)?;
        }
        Ok(())
    })
    .ok(); // an error is a problem already
}

impl NavesApp {
    /// Delete: the selected way, or the selected node of a drawn way. An OpenStreetMap node is only moved.
    pub(in crate::gui) fn delete_selection(&mut self) {
        let Some(target) = self.selection.as_ref().and_then(|s| s.target) else {
            return;
        };
        match target {
            Target::Way(way) => {
                let Some((_, original)) = way_pair(self, way) else {
                    return;
                };
                if self
                    .edit_way(|edits, _| edits.delete_way(&original, By::User))
                    .is_ok()
                {
                    self.selection = None;
                }
            }
            Target::Node { node, way } => {
                if node < USER_ID_BASE {
                    self.set_edit_problem(
                        "An OpenStreetMap node cannot be deleted, only moved.".to_string(),
                    );
                    return;
                }
                self.take_node_out(node, way);
            }
        }
    }

    fn take_node_out(&mut self, node: i64, way: i64) {
        let Some((current, original)) = way_pair(self, way) else {
            return;
        };
        let nodes: Vec<i64> = current
            .nodes
            .iter()
            .copied()
            .filter(|&n| n != node)
            .collect();
        let result = if nodes.len() < 2 {
            self.edit_way(|edits, _| edits.delete_way(&original, By::User))
        } else {
            let tags = current.tags.clone();
            self.edit_way(|edits, _| {
                edits.modify_way(&original, tags, ids(&nodes)?, By::User)?;
                // the node is gone with its way's list, unless another way still has it
                if let Some(id) = ElementId::from_raw(node)
                    && !edits.ways.iter().any(|w| w.nodes.contains(&id))
                {
                    edits.nodes.retain(|n| n.id != id);
                }
                Ok(())
            })
        };
        if result.is_ok() {
            self.selection = self.selection.take().map(|mut s| {
                s.target = (nodes.len() >= 2).then_some(Target::Way(way));
                s
            });
        }
    }
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Lines of points in map metres turned into node ids of the road edit set.
//!
//! A caller (the MCP road tools) draws a road as points. A point that lies on a node of the OSM ways is that
//! node, so crossings and junctions stay joined; any other point becomes a new user node. No OSM node is moved.
//! Every operation leaves the set as it was when it returns an error.

use std::collections::BTreeMap;

use super::ways::invalid;
use super::{By, EditsError, ElementId, MAX_WAY_NODES, WayEdits};
use crate::heightmap::MapGeo;
use crate::osm::{OsmWay, OsmWays};

/// A point this close (metres) to a node is that node.
pub const SNAP_M: f64 = 0.05;

/// How many ways a listing of the road tools returns at most.
pub const MAX_LISTED_WAYS: usize = 1_000;

/// The nodes of the OSM ways by map position, to find the node a point lies on.
#[derive(Debug, Clone, Default)]
pub struct NodeIndex {
    /// `(x, y, raw id)` sorted by `x`, then `y`, then id.
    nodes: Vec<(f64, f64, i64)>,
}

impl NodeIndex {
    /// Indexes every node of every way, projected once. A node shared by ways appears once; a node that cannot
    /// be projected is left out.
    #[must_use]
    pub fn new(ways: &OsmWays, frame: &MapGeo) -> NodeIndex {
        let mut seen: BTreeMap<i64, (f64, f64)> = BTreeMap::new();
        for way in &ways.ways {
            for (&id, &(lat, lon)) in way.nodes.iter().zip(&way.points) {
                if seen.contains_key(&id) {
                    continue;
                }
                if let Ok((x, y)) = frame.to_map(lat, lon)
                    && x.is_finite()
                    && y.is_finite()
                {
                    seen.insert(id, (x, y));
                }
            }
        }
        let mut nodes: Vec<(f64, f64, i64)> =
            seen.into_iter().map(|(id, (x, y))| (x, y, id)).collect();
        nodes.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
        });
        NodeIndex { nodes }
    }

    /// The raw id of the node nearest to the map position within [`SNAP_M`]; the smaller id on a tie.
    #[must_use]
    pub fn near(&self, x: f64, y: f64) -> Option<i64> {
        let start = self.nodes.partition_point(|n| n.0 < x - SNAP_M);
        let mut best: Option<(f64, i64)> = None;
        for &(nx, ny, id) in self.nodes.iter().skip(start) {
            if nx > x + SNAP_M {
                break;
            }
            let d = (nx - x).hypot(ny - y);
            if d <= SNAP_M {
                let better =
                    best.is_none_or(|(bd, bid)| d < bd || (d.total_cmp(&bd).is_eq() && id < bid));
                if better {
                    best = Some((d, id));
                }
            }
        }
        best.map(|(_, id)| id)
    }
}

/// Checks a line of map positions: 2 to [`MAX_WAY_NODES`] points, every coordinate finite and inside
/// `0.0..=map_size_m`.
///
/// # Errors
/// [`EditsError::Invalid`] (named `what`) for the first problem found.
pub fn check_line(what: &str, points: &[(f64, f64)], map_size_m: f64) -> Result<(), EditsError> {
    if points.len() < 2 {
        return Err(invalid(what, "a way needs at least 2 nodes"));
    }
    check_points(what, points, map_size_m)
}

/// The checks of [`check_line`] without the lower limit: a lone point is a valid position.
fn check_points(what: &str, points: &[(f64, f64)], map_size_m: f64) -> Result<(), EditsError> {
    if points.len() > MAX_WAY_NODES {
        return Err(invalid(what, "too many nodes"));
    }
    let inside = 0.0..=map_size_m;
    for &(x, y) in points {
        if !x.is_finite() || !y.is_finite() {
            return Err(invalid(what, "a point is not a finite number"));
        }
        if !inside.contains(&x) || !inside.contains(&y) {
            return Err(invalid(what, "a point is outside the map"));
        }
    }
    Ok(())
}

fn check_highway(what: &str, tags: &BTreeMap<String, String>) -> Result<(), EditsError> {
    if tags.contains_key("highway") {
        Ok(())
    } else {
        Err(invalid(what, "a road needs a highway tag"))
    }
}

impl WayEdits {
    /// The node ids for a line of map positions, in order: the OSM node a point lies on, else a node made by
    /// this call within [`SNAP_M`] (so a closed line reuses its first node), else a new user node by `by`.
    ///
    /// # Errors
    /// [`EditsError::Invalid`] if the line has too many points, or a point is not finite or outside the map (as [`check_line`]), a point cannot be projected, or a node cannot be
    /// added. The set is unchanged then.
    pub fn line_nodes(
        &mut self,
        index: &NodeIndex,
        frame: &MapGeo,
        points: &[(f64, f64)],
        by: By,
    ) -> Result<Vec<ElementId>, EditsError> {
        check_points("line", points, frame.map_size_m())?;
        let (node_count, next_id) = (self.nodes.len(), self.next_id);
        let result = self.line_nodes_unchecked(index, frame, points, by);
        if result.is_err() {
            // new_node only appends, so cutting back restores the set.
            self.nodes.truncate(node_count);
            self.next_id = next_id;
        }
        result
    }

    fn line_nodes_unchecked(
        &mut self,
        index: &NodeIndex,
        frame: &MapGeo,
        points: &[(f64, f64)],
        by: By,
    ) -> Result<Vec<ElementId>, EditsError> {
        let mut made: Vec<(f64, f64, ElementId)> = Vec::new();
        let mut ids = Vec::with_capacity(points.len());
        for &(x, y) in points {
            if let Some(id) = index.near(x, y).and_then(ElementId::from_raw) {
                ids.push(id);
                continue;
            }
            if let Some(&(_, _, id)) = made
                .iter()
                .find(|&&(mx, my, _)| (mx - x).hypot(my - y) <= SNAP_M)
            {
                ids.push(id);
                continue;
            }
            let (lat, lon) = frame
                .to_geo(x, y)
                .map_err(|_| invalid("new node", "a point cannot be projected"))?;
            let id = self.new_node(lat, lon, by)?;
            made.push((x, y, id));
            ids.push(id);
        }
        Ok(ids)
    }

    /// Adds a way drawn as points in map metres, and returns its user id. The tags need a `highway` key.
    ///
    /// # Errors
    /// [`EditsError::Invalid`] if the line or tags cannot be used or the way cannot be added. The set is
    /// unchanged then.
    pub fn add_line(
        &mut self,
        index: &NodeIndex,
        frame: &MapGeo,
        tags: BTreeMap<String, String>,
        points: &[(f64, f64)],
        by: By,
    ) -> Result<ElementId, EditsError> {
        let what = "new way";
        check_line(what, points, frame.map_size_m())?;
        check_highway(what, &tags)?;
        let mut work = self.clone();
        let nodes = work.line_nodes(index, frame, points, by)?;
        let id = work.add_way(tags, nodes, by)?;
        *self = work;
        Ok(id)
    }

    /// Changes the tags and/or the line of a way. `original` is the way as OSM has it, `current` the way as it
    /// is now (with the edits). What is `None` stays as in `current`. User nodes that `current` listed and that
    /// no way edit lists any more are dropped.
    ///
    /// # Errors
    /// [`EditsError::Invalid`] if both are `None` (`nothing to change`), the line or tags cannot be used, or the
    /// way cannot be changed. The set is unchanged then.
    #[expect(
        clippy::too_many_arguments,
        reason = "the original and current way, the new tags and line, and who: all are needed"
    )]
    pub fn modify_line(
        &mut self,
        index: &NodeIndex,
        frame: &MapGeo,
        original: &OsmWay,
        current: &OsmWay,
        tags: Option<BTreeMap<String, String>>,
        points: Option<&[(f64, f64)]>,
        by: By,
    ) -> Result<(), EditsError> {
        let what = format!("way {}", current.id);
        if tags.is_none() && points.is_none() {
            return Err(invalid(&what, "nothing to change"));
        }
        let current_nodes = current
            .nodes
            .iter()
            .map(|&raw| ElementId::from_raw(raw))
            .collect::<Option<Vec<ElementId>>>()
            .ok_or_else(|| invalid(&what, "a node id cannot be used"))?;
        if let Some(tags) = &tags {
            check_highway(&what, tags)?;
        }
        if let Some(points) = points {
            check_line(&what, points, frame.map_size_m())?;
        }
        let tags = tags.unwrap_or_else(|| current.tags.clone());
        let mut work = self.clone();
        let nodes = match points {
            Some(points) => work.line_nodes(index, frame, points, by)?,
            None => current_nodes.clone(),
        };
        work.modify_way(original, tags, nodes, by)?;
        work.drop_unused_user_nodes(&current_nodes);
        *self = work;
        Ok(())
    }
}

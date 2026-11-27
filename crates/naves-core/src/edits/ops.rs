// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Edit operations on [`WayEdits`]: what the GUI and the MCP server call.
//!
//! Every operation checks its input against the limits of [`WayEdits::validate`] before it changes anything, so
//! on an error the set is unchanged and a set that was valid stays valid.

use std::collections::{BTreeMap, BTreeSet};

use super::ways::{check_position, invalid};
use super::{
    By, EditsError, ElementId, MAX_NODE_EDITS, MAX_WAY_EDITS, MAX_WAY_NODES, NodeEdit, WayAction,
    WayEdit, WayEdits, fingerprint,
};
use crate::osm::OsmWay;

/// The tag that keeps a way as drawn.
const KEEP: &str = "naves:keep";

fn with_keep(mut tags: BTreeMap<String, String>) -> BTreeMap<String, String> {
    tags.entry(KEEP.to_string())
        .or_insert_with(|| "yes".to_string());
    tags
}

fn check_way(
    what: &str,
    tags: &BTreeMap<String, String>,
    nodes: &[ElementId],
) -> Result<(), EditsError> {
    WayEdits::check_tags(what, tags)?;
    if nodes.len() > MAX_WAY_NODES {
        return Err(invalid(what, "too many nodes"));
    }
    if nodes.iter().any(|id| id.raw().is_none()) {
        return Err(invalid(what, "a node id cannot be used"));
    }
    WayEdits::check_nodes(what, nodes)
}

impl WayEdits {
    /// Adds a node the user drew and returns its new user id.
    ///
    /// # Errors
    /// [`EditsError::Invalid`] for a position that is not on Earth, too many node edits, or no id left.
    pub fn new_node(&mut self, lat: f64, lon: f64, by: By) -> Result<ElementId, EditsError> {
        let what = "new node";
        check_position(what, lat, lon)?;
        if self.nodes.len() >= MAX_NODE_EDITS {
            return Err(invalid(what, "too many node edits"));
        }
        let id = ElementId::User(self.next_id);
        if id.raw().is_none() {
            return Err(invalid(what, "no user id left"));
        }
        let next = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| invalid(what, "no user id left"))?;
        self.next_id = next;
        self.nodes.push(NodeEdit {
            id,
            lat,
            lon,
            base: None,
            by,
        });
        Ok(id)
    }

    /// Moves a node. A user node must exist; an OSM node needs its OSM position `osm_at` the first time (it is
    /// kept as `base`; later calls keep the first one).
    ///
    /// # Errors
    /// [`EditsError::Invalid`] for an unusable id or position, an unknown user node, a missing `osm_at`, or too
    /// many node edits.
    pub fn move_node(
        &mut self,
        id: ElementId,
        lat: f64,
        lon: f64,
        osm_at: Option<(f64, f64)>,
        by: By,
    ) -> Result<(), EditsError> {
        let what = format!("node {id}");
        if id.raw().is_none() {
            return Err(invalid(&what, "id cannot be used"));
        }
        check_position(&what, lat, lon)?;
        let existing = self.nodes.iter().position(|n| n.id == id);
        if id.is_user() {
            let index = existing.ok_or_else(|| invalid(&what, "unknown user node"))?;
            if let Some(node) = self.nodes.get_mut(index) {
                node.lat = lat;
                node.lon = lon;
                node.by = by;
            }
            return Ok(());
        }
        if let Some(index) = existing {
            if let Some(node) = self.nodes.get_mut(index) {
                node.lat = lat;
                node.lon = lon;
                node.by = by;
            }
        } else {
            let (base_lat, base_lon) =
                osm_at.ok_or_else(|| invalid(&what, "the OSM position is needed"))?;
            check_position(&what, base_lat, base_lon)?;
            if self.nodes.len() >= MAX_NODE_EDITS {
                return Err(invalid(&what, "too many node edits"));
            }
            self.nodes.push(NodeEdit {
                id,
                lat,
                lon,
                base: Some((base_lat, base_lon)),
                by,
            });
        }
        Ok(())
    }

    /// Adds a way and returns its new user id. `naves:keep=yes` is added unless `tags` has `naves:keep`.
    ///
    /// # Errors
    /// [`EditsError::Invalid`] if the tags or nodes cannot be used, there are too many way edits, or no id is left.
    pub fn add_way(
        &mut self,
        tags: BTreeMap<String, String>,
        nodes: Vec<ElementId>,
        by: By,
    ) -> Result<ElementId, EditsError> {
        let what = "new way";
        let tags = with_keep(tags);
        check_way(what, &tags, &nodes)?;
        if self.ways.len() >= MAX_WAY_EDITS {
            return Err(invalid(what, "too many way edits"));
        }
        let id = ElementId::User(self.next_id);
        if id.raw().is_none() {
            return Err(invalid(what, "no user id left"));
        }
        let next = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| invalid(what, "no user id left"))?;
        self.next_id = next;
        self.ways.push(WayEdit {
            id,
            action: WayAction::Add,
            tags,
            nodes,
            base: None,
            by,
        });
        Ok(id)
    }

    /// Replaces the tags and nodes of a way. An OSM way gets a `modify` edit with the fingerprint of `original`
    /// as `base` (the first one is kept); a user way (an `original.id` of `USER_ID_BASE` or more) has its `add`
    /// edit updated. `naves:keep=yes` is added unless `tags` has `naves:keep`.
    ///
    /// # Errors
    /// [`EditsError::Invalid`] if the id, tags or nodes cannot be used, a user way does not exist, or there are
    /// too many way edits.
    pub fn modify_way(
        &mut self,
        original: &OsmWay,
        tags: BTreeMap<String, String>,
        nodes: Vec<ElementId>,
        by: By,
    ) -> Result<(), EditsError> {
        let id = ElementId::from_raw(original.id)
            .ok_or_else(|| invalid(&format!("way {}", original.id), "id cannot be used"))?;
        let what = format!("way {id}");
        let tags = with_keep(tags);
        check_way(&what, &tags, &nodes)?;
        let existing = self.ways.iter().position(|w| w.id == id);
        if id.is_user() {
            let index = existing.ok_or_else(|| invalid(&what, "unknown user way"))?;
            if let Some(edit) = self.ways.get_mut(index) {
                edit.tags = tags;
                edit.nodes = nodes;
                edit.by = by;
            }
            return Ok(());
        }
        if let Some(edit) = existing.and_then(|i| self.ways.get_mut(i)) {
            if edit.base.is_none() {
                edit.base = Some(fingerprint(&original.tags, &original.nodes));
            }
            edit.action = WayAction::Modify;
            edit.tags = tags;
            edit.nodes = nodes;
            edit.by = by;
        } else {
            if self.ways.len() >= MAX_WAY_EDITS {
                return Err(invalid(&what, "too many way edits"));
            }
            self.ways.push(WayEdit {
                id,
                action: WayAction::Modify,
                tags,
                nodes,
                base: Some(fingerprint(&original.tags, &original.nodes)),
                by,
            });
        }
        Ok(())
    }

    /// Deletes a way. An OSM way gets a `delete` edit (replacing an earlier `modify`, keeping its `base`); a user
    /// way's edit is removed together with its user nodes that no other way edit lists.
    ///
    /// # Errors
    /// [`EditsError::Invalid`] if the id cannot be used, a user way does not exist, or there are too many way
    /// edits.
    pub fn delete_way(&mut self, original: &OsmWay, by: By) -> Result<(), EditsError> {
        let id = ElementId::from_raw(original.id)
            .ok_or_else(|| invalid(&format!("way {}", original.id), "id cannot be used"))?;
        let what = format!("way {id}");
        let existing = self.ways.iter().position(|w| w.id == id);
        if id.is_user() {
            let index = existing.ok_or_else(|| invalid(&what, "unknown user way"))?;
            self.remove_way_at(index);
            return Ok(());
        }
        if let Some(index) = existing {
            let dropped = self.ways.get(index).map(|w| w.nodes.clone());
            if let Some(edit) = self.ways.get_mut(index) {
                if edit.base.is_none() {
                    edit.base = Some(fingerprint(&original.tags, &original.nodes));
                }
                edit.action = WayAction::Delete;
                edit.tags.clear();
                edit.nodes.clear();
                edit.by = by;
            }
            if let Some(nodes) = dropped {
                self.drop_unused_user_nodes(&nodes);
            }
        } else {
            if self.ways.len() >= MAX_WAY_EDITS {
                return Err(invalid(&what, "too many way edits"));
            }
            self.ways.push(WayEdit {
                id,
                action: WayAction::Delete,
                tags: BTreeMap::new(),
                nodes: Vec::new(),
                base: Some(fingerprint(&original.tags, &original.nodes)),
                by,
            });
        }
        Ok(())
    }

    /// Drops the way's edit (an OSM way is as OSM has it again; a user way is gone). User nodes that no other
    /// way edit lists go too. Does nothing if the way has no edit.
    pub fn reset_way(&mut self, id: ElementId) {
        if let Some(index) = self.ways.iter().position(|w| w.id == id) {
            self.remove_way_at(index);
        }
    }

    fn remove_way_at(&mut self, index: usize) {
        let removed = self.ways.remove(index);
        self.drop_unused_user_nodes(&removed.nodes);
    }

    fn drop_unused_user_nodes(&mut self, candidates: &[ElementId]) {
        let used: BTreeSet<ElementId> = self
            .ways
            .iter()
            .flat_map(|w| w.nodes.iter().copied())
            .collect();
        let gone: BTreeSet<ElementId> = candidates
            .iter()
            .copied()
            .filter(|id| id.is_user() && !used.contains(id))
            .collect();
        self.nodes.retain(|n| !gone.contains(&n.id));
    }
}

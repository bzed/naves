// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The edit session: the ways of the project with the hand-made edits, saved at once and undone step by step.

#[cfg(test)]
pub mod fixture;
pub mod hit;
pub mod overlay;
pub mod properties;
pub mod tools;

use std::collections::BTreeSet;
use std::path::Path;

use naves_core::edits::{EditsError, USER_ID_BASE, WayEdits};
use naves_core::osm::OsmWay;
use naves_export::generate::Source;

use crate::gui::viewport::WorldPoint;

/// What the editor holds.
#[derive(Debug, Default)]
pub enum EditState {
    /// No session: there is no project data to edit yet.
    #[default]
    None,
    /// The source layer is being read.
    Loading,
    /// Ready to edit.
    Ready(Box<EditSession>),
    /// The source layer could not be read; the text says why.
    Failed(String),
}

impl EditState {
    /// The session, once it is there.
    #[must_use]
    pub fn session(&self) -> Option<&EditSession> {
        match self {
            EditState::Ready(session) => Some(session),
            _ => None,
        }
    }

    /// The session, once it is there, to change it.
    pub fn session_mut(&mut self) -> Option<&mut EditSession> {
        match self {
            EditState::Ready(session) => Some(session),
            _ => None,
        }
    }
}

/// Where a way comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// An OpenStreetMap way as it is.
    Osm,
    /// An OpenStreetMap way that has an edit, or one of whose nodes was moved.
    EditedOsm,
    /// A way drawn by the user.
    Drawn,
}

/// A road way of the session projected to map metres, for hit-testing and the overlay.
#[derive(Debug, Clone)]
pub struct ProjectedWay {
    /// The raw id of the way.
    pub id: i64,
    /// Its nodes with their raw ids; a node that cannot be projected is left out.
    pub points: Vec<(i64, WorldPoint)>,
    /// Where the way comes from.
    pub origin: Origin,
}

/// The source layer with its edits, and the steps to undo and redo.
#[derive(Debug)]
pub struct EditSession {
    /// The extract, the edits and the ways with the edits applied.
    pub source: Source,
    undo: Vec<WayEdits>,
    redo: Vec<WayEdits>,
    /// Edits were made since the roads were built.
    pub dirty: bool,
    /// The road ways in map metres, rebuilt only when the edits change.
    projected: Vec<ProjectedWay>,
    /// The raw ids of the OpenStreetMap ways that have an edit.
    edited: BTreeSet<i64>,
    /// The raw ids of the nodes that were moved.
    moved: BTreeSet<i64>,
    /// How often `projected` was built (for the tests).
    builds: usize,
}

impl EditSession {
    /// How many steps can be undone.
    pub const UNDO_DEPTH: usize = 100;

    /// A session on `source`, with nothing to undo.
    #[must_use]
    pub fn new(source: Source) -> Self {
        let mut session = Self {
            source,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
            projected: Vec::new(),
            edited: BTreeSet::new(),
            moved: BTreeSet::new(),
            builds: 0,
        };
        session.project();
        session
    }

    /// The road ways in map metres, in id order.
    #[must_use]
    pub fn projected(&self) -> &[ProjectedWay] {
        &self.projected
    }

    /// How often the road ways were projected: once when the session starts and once per change of the edits.
    #[cfg(test)]
    #[must_use]
    pub fn projection_builds(&self) -> usize {
        self.builds
    }

    fn project(&mut self) {
        self.edited = self
            .source
            .edits
            .ways
            .iter()
            .filter_map(|w| w.id.raw())
            .collect();
        self.moved = self
            .source
            .edits
            .nodes
            .iter()
            .filter_map(|n| n.id.raw())
            .collect();
        let mut projected = Vec::new();
        for way in &self.source.ways.ways {
            if !way.tags.contains_key("highway") {
                continue;
            }
            let points = way
                .nodes
                .iter()
                .zip(&way.points)
                .filter_map(|(node, &(lat, lon))| {
                    let (x, y) = self.source.frame.to_map(lat, lon).ok()?;
                    Some((*node, WorldPoint { x, y }))
                })
                .collect();
            projected.push(ProjectedWay {
                id: way.id,
                points,
                origin: self.origin(way.id),
            });
        }
        self.projected = projected;
        self.builds += 1;
    }

    /// Whether there is a step to undo.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether there is a step to redo.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Makes an edit: `op` runs on a copy of the edits, the copy is saved to `dir`, and only then does it replace
    /// the edits. Any error leaves the session and the file as they were.
    ///
    /// # Errors
    ///
    /// The text of what went wrong, starting with "The edit cannot be made: " or "Could not save the edit: ".
    pub fn apply(
        &mut self,
        dir: &Path,
        op: impl FnOnce(&mut WayEdits, &Source) -> Result<(), EditsError>,
    ) -> Result<(), String> {
        let mut edits = self.source.edits.clone();
        op(&mut edits, &self.source).map_err(|e| format!("The edit cannot be made: {e}"))?;
        save(&edits, dir)?;
        let before = std::mem::replace(&mut self.source.edits, edits);
        push_capped(&mut self.undo, before);
        self.redo.clear();
        self.changed();
        Ok(())
    }

    /// Takes back the last edit. `false` when there is none.
    ///
    /// # Errors
    ///
    /// "Could not save the edit: ..." when the file cannot be written; nothing changes then.
    pub fn undo(&mut self, dir: &Path) -> Result<bool, String> {
        let Some(previous) = self.undo.last() else {
            return Ok(false);
        };
        save(previous, dir)?;
        let Some(previous) = self.undo.pop() else {
            return Ok(false); // checked above
        };
        let current = std::mem::replace(&mut self.source.edits, previous);
        self.redo.push(current);
        self.changed();
        Ok(true)
    }

    /// Does the last undone edit again. `false` when there is none.
    ///
    /// # Errors
    ///
    /// "Could not save the edit: ..." when the file cannot be written; nothing changes then.
    pub fn redo(&mut self, dir: &Path) -> Result<bool, String> {
        let Some(next) = self.redo.last() else {
            return Ok(false);
        };
        save(next, dir)?;
        let Some(next) = self.redo.pop() else {
            return Ok(false); // checked above
        };
        let current = std::mem::replace(&mut self.source.edits, next);
        push_capped(&mut self.undo, current);
        self.changed();
        Ok(true)
    }

    fn changed(&mut self) {
        self.source.reapply();
        self.project();
        self.dirty = true;
    }

    /// The way with this raw id, with the edits applied.
    #[must_use]
    pub fn way(&self, raw: i64) -> Option<&OsmWay> {
        let ways = &self.source.ways.ways;
        ways.binary_search_by_key(&raw, |w| w.id)
            .ok()
            .and_then(|i| ways.get(i))
    }

    /// The way as the extract has it (for an OSM id), or as drawn (for a user id).
    #[must_use]
    pub fn original(&self, raw: i64) -> Option<&OsmWay> {
        if raw >= USER_ID_BASE {
            self.way(raw)
        } else {
            let ways = &self.source.extract.ways;
            ways.binary_search_by_key(&raw, |w| w.id)
                .ok()
                .and_then(|i| ways.get(i))
        }
    }

    /// Where the way comes from.
    #[must_use]
    pub fn origin(&self, raw: i64) -> Origin {
        if raw >= USER_ID_BASE {
            Origin::Drawn
        } else if self.edited.contains(&raw)
            || self
                .way(raw)
                .is_some_and(|w| w.nodes.iter().any(|n| self.moved.contains(n)))
        {
            Origin::EditedOsm
        } else {
            Origin::Osm
        }
    }
}

fn save(edits: &WayEdits, dir: &Path) -> Result<(), String> {
    edits
        .save(dir)
        .map_err(|e| format!("Could not save the edit: {e}"))
}

fn push_capped(stack: &mut Vec<WayEdits>, edits: WayEdits) {
    stack.push(edits);
    if stack.len() > EditSession::UNDO_DEPTH {
        stack.remove(0);
    }
}

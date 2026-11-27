// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `edits/ways.toml`: the nodes and ways the user drew or changed.
//!
//! The file lists node edits (a new node, or an OSM node that was moved) and way edits (a new way, a changed OSM
//! way, or a deleted one). Positions are WGS84 degrees. The file is untrusted input: [`WayEdits::parse`] checks
//! every count, length and number before anything uses it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{EditsError, ElementId};

/// Most way edits in one file.
pub const MAX_WAY_EDITS: usize = 100_000;
/// Most node edits in one file.
pub const MAX_NODE_EDITS: usize = 1_000_000;
/// Most nodes in one way (OSM's own limit).
pub const MAX_WAY_NODES: usize = 2_000;
/// Most tags on one way.
pub const MAX_TAGS: usize = 64;
/// Longest tag key or value, in characters (OSM's limit).
pub const MAX_TAG_LEN: usize = 255;

const EDITS_DIR: &str = "edits";
const WAYS_FILE: &str = "ways.toml";

/// Who made an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum By {
    /// A person, in the GUI or by hand.
    #[default]
    User,
    /// An AI client through the MCP server.
    Ai,
}

/// A node the user added, or an OSM node the user moved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeEdit {
    /// The node. A user node for a new one, the OSM id for a moved one.
    pub id: ElementId,
    /// Latitude in degrees.
    pub lat: f64,
    /// Longitude in degrees.
    pub lon: f64,
    /// The OSM position `(lat, lon)` of a moved OSM node; `None` for a user node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<(f64, f64)>,
    /// Who made the edit.
    #[serde(default)]
    pub by: By,
}

/// What a way edit does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WayAction {
    /// A new way (a user id).
    Add,
    /// A changed OSM way: its tags and nodes are replaced.
    Modify,
    /// A removed OSM way.
    Delete,
}

/// A way the user added, changed or removed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WayEdit {
    /// The way. A user id for `add`, the OSM id for `modify` and `delete`.
    pub id: ElementId,
    /// What the edit does.
    pub action: WayAction,
    /// The tags of the way after the edit (none for `delete`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<String, String>,
    /// The nodes of the way after the edit (none for `delete`). A closed way repeats its first node at the end.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<ElementId>,
    /// [`fingerprint`] of the OSM original, for `modify` and `delete`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Who made the edit.
    #[serde(default)]
    pub by: By,
}

/// The content of `edits/ways.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct WayEdits {
    /// File format version; 1.
    pub format: u32,
    /// The number the next user element gets; greater than every user id in the file.
    pub next_id: u64,
    /// Node edits.
    pub nodes: Vec<NodeEdit>,
    /// Way edits.
    pub ways: Vec<WayEdit>,
}

impl Default for WayEdits {
    fn default() -> Self {
        WayEdits {
            format: 1,
            next_id: 1,
            nodes: Vec::new(),
            ways: Vec::new(),
        }
    }
}

fn invalid(what: &str, problem: &'static str) -> EditsError {
    EditsError::Invalid {
        what: what.to_string(),
        problem,
    }
}

fn check_position(what: &str, lat: f64, lon: f64) -> Result<(), EditsError> {
    if !lat.is_finite() || !lon.is_finite() {
        return Err(invalid(what, "position is not a finite number"));
    }
    if !(-90.0..=90.0).contains(&lat) {
        return Err(invalid(what, "latitude is outside -90..90"));
    }
    if !(-180.0..=180.0).contains(&lon) {
        return Err(invalid(what, "longitude is outside -180..180"));
    }
    Ok(())
}

impl WayEdits {
    /// Reads the file's text and checks it.
    ///
    /// # Errors
    /// [`EditsError::Parse`] for damaged TOML, [`EditsError::Invalid`] for content that cannot be used.
    pub fn parse(text: &str) -> Result<WayEdits, EditsError> {
        let edits: WayEdits = toml::from_str(text)?;
        edits.validate()?;
        Ok(edits)
    }

    /// Checks everything [`WayEdits::parse`] checks.
    ///
    /// # Errors
    /// [`EditsError::Invalid`] naming the element and the problem.
    pub fn validate(&self) -> Result<(), EditsError> {
        if self.format != 1 {
            return Err(invalid("file", "format must be 1"));
        }
        if self.nodes.len() > MAX_NODE_EDITS {
            return Err(invalid("file", "too many node edits"));
        }
        if self.ways.len() > MAX_WAY_EDITS {
            return Err(invalid("file", "too many way edits"));
        }
        let mut max_user = 0_u64;
        let mut note = |id: ElementId| {
            if let ElementId::User(n) = id {
                max_user = max_user.max(n);
            }
        };

        let mut seen_nodes = BTreeSet::new();
        for node in &self.nodes {
            let what = format!("node {}", node.id);
            note(node.id);
            if node.id.raw().is_none() {
                return Err(invalid(&what, "id cannot be used"));
            }
            if !seen_nodes.insert(node.id) {
                return Err(invalid(&what, "node edited twice"));
            }
            check_position(&what, node.lat, node.lon)?;
            if let Some((lat, lon)) = node.base {
                if node.id.is_user() {
                    return Err(invalid(&what, "a user node has no base position"));
                }
                check_position(&what, lat, lon)?;
            }
        }

        let mut seen_ways = BTreeSet::new();
        for way in &self.ways {
            let what = format!("way {}", way.id);
            note(way.id);
            if way.id.raw().is_none() {
                return Err(invalid(&what, "id cannot be used"));
            }
            if !seen_ways.insert(way.id) {
                return Err(invalid(&what, "way edited twice"));
            }
            match (way.action, way.id.is_user()) {
                (WayAction::Add, false) => {
                    return Err(invalid(&what, "an added way needs a user id"));
                }
                (WayAction::Modify | WayAction::Delete, true) => {
                    return Err(invalid(&what, "a modified or deleted way needs an OSM id"));
                }
                _ => {}
            }
            Self::check_tags(&what, &way.tags)?;
            // The length is checked before any per-node work.
            if way.nodes.len() > MAX_WAY_NODES {
                return Err(invalid(&what, "too many nodes"));
            }
            for &id in &way.nodes {
                note(id);
                if id.raw().is_none() {
                    return Err(invalid(&what, "a node id cannot be used"));
                }
            }
            if way.action == WayAction::Delete {
                if !way.nodes.is_empty() || !way.tags.is_empty() {
                    return Err(invalid(&what, "a deleted way has no nodes and no tags"));
                }
            } else {
                Self::check_nodes(&what, &way.nodes)?;
            }
        }

        if max_user >= self.next_id {
            return Err(invalid("file", "next_id is not greater than every user id"));
        }
        Ok(())
    }

    fn check_tags(what: &str, tags: &BTreeMap<String, String>) -> Result<(), EditsError> {
        if tags.len() > MAX_TAGS {
            return Err(invalid(what, "too many tags"));
        }
        for (key, value) in tags {
            if key.is_empty() {
                return Err(invalid(what, "a tag key is empty"));
            }
            if key.chars().count() > MAX_TAG_LEN || value.chars().count() > MAX_TAG_LEN {
                return Err(invalid(what, "a tag key or value is too long"));
            }
        }
        Ok(())
    }

    fn check_nodes(what: &str, nodes: &[ElementId]) -> Result<(), EditsError> {
        if nodes.len() < 2 {
            return Err(invalid(what, "a way needs at least 2 nodes"));
        }
        // The only repeat allowed is the first node again at the end (a closed way).
        let Some((last, body)) = nodes.split_last() else {
            return Err(invalid(what, "a way needs at least 2 nodes"));
        };
        let mut seen = BTreeSet::new();
        for &id in body {
            if !seen.insert(id) {
                return Err(invalid(what, "a node is repeated"));
            }
        }
        if seen.contains(last) && nodes.first() != Some(last) {
            return Err(invalid(what, "a node is repeated"));
        }
        if nodes.first() == Some(last) && body.len() < 2 {
            return Err(invalid(what, "a closed way needs at least 3 nodes"));
        }
        Ok(())
    }

    fn path(project_dir: &Path) -> PathBuf {
        project_dir.join(EDITS_DIR).join(WAYS_FILE)
    }

    /// Reads `edits/ways.toml` below `project_dir`; a missing file is the empty edit set.
    ///
    /// # Errors
    /// [`EditsError`] if the file exists but cannot be read, is damaged, or cannot be used.
    pub fn load(project_dir: &Path) -> Result<WayEdits, EditsError> {
        let path = Self::path(project_dir);
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(WayEdits::default()),
            Err(source) => Err(EditsError::Io { path, source }),
        }
    }

    /// Checks and writes `edits/ways.toml` below `project_dir`, atomically.
    ///
    /// # Errors
    /// [`EditsError`] if an edit cannot be used or the file cannot be written.
    pub fn save(&self, project_dir: &Path) -> Result<(), EditsError> {
        self.validate()?;
        let path = Self::path(project_dir);
        let folder = project_dir.join(EDITS_DIR);
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source| EditsError::Io { path, source }
        };
        std::fs::create_dir_all(&folder).map_err(io(&folder))?;
        let text = toml::to_string_pretty(self)?;
        let temp = tempfile::NamedTempFile::new_in(&folder).map_err(io(&folder))?;
        std::fs::write(temp.path(), text).map_err(io(temp.path()))?;
        temp.persist(&path).map_err(|e| io(&path)(e.error))?;
        Ok(())
    }
}

/// A stable 64-bit FNV-1a fingerprint, as 16 hex digits, of an OSM way's tags and nodes.
///
/// The input is the `key=value` lines in key order, then the node ids as decimal lines. It tells whether the
/// OSM original of an edited way has changed since the edit was made.
#[must_use]
pub fn fingerprint(tags: &BTreeMap<String, String>, nodes: &[i64]) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0100_0000_01b3;
    let mut hash = OFFSET;
    let mut feed = |bytes: &[u8]| {
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    };
    for (key, value) in tags {
        feed(format!("{key}={value}\n").as_bytes());
    }
    for node in nodes {
        feed(format!("{node}\n").as_bytes());
    }
    format!("{hash:016x}")
}

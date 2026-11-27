// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Hand-made edits to the roads, terrain and placed models of a map (spec 5.2).
//!
//! An edit set is a TOML file of the project. Every element it names is identified by an [`ElementId`]: an OSM
//! id for what came from OpenStreetMap, or `u<n>` for what the user drew.

pub mod apply;
pub mod ids;
pub mod ops;
pub mod ways;

use std::path::PathBuf;

pub use apply::{EditIssue, EditReport, apply_way_edits};
pub use ids::{ElementId, ElementIdError, USER_ID_BASE};
pub use ways::{
    By, MAX_NODE_EDITS, MAX_TAG_LEN, MAX_TAGS, MAX_WAY_EDITS, MAX_WAY_NODES, NodeEdit, WayAction,
    WayEdit, WayEdits, fingerprint,
};

/// What can go wrong with an edit file.
#[derive(Debug, thiserror::Error)]
pub enum EditsError {
    /// The file cannot be read or written.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file or folder.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The file is not valid TOML of the expected shape.
    #[error("edit file is damaged: {0}")]
    Parse(#[from] toml::de::Error),
    /// The edits cannot be written as TOML.
    #[error("edit file cannot be written: {0}")]
    Write(#[from] toml::ser::Error),
    /// An element of the file cannot be used.
    #[error("{what}: {problem}")]
    Invalid {
        /// The element: `way u3`, `node 812`, or `file`.
        what: String,
        /// What is wrong with it.
        problem: &'static str,
    },
}

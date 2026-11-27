// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Hand-made edits to the roads, terrain and placed models of a map (spec 5.2).
//!
//! An edit set is a TOML file of the project. Every element it names is identified by an [`ElementId`]: an OSM
//! id for what came from OpenStreetMap, or `u<n>` for what the user drew.

pub mod ids;

pub use ids::{ElementId, ElementIdError, USER_ID_BASE};

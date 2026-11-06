// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Problems: what is wrong with the project or the last build, with where it is on the map.

use super::levels::Level;
use super::viewport::WorldPoint;

/// One problem.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    /// What is wrong, in words, and what to do about it where that is known.
    pub message: String,
    /// Where it is on the map, if it is anywhere.
    pub at: Option<WorldPoint>,
    /// The level it belongs to; `None` for a problem that belongs to no level.
    pub level: Option<Level>,
}

impl Problem {
    /// A problem without a position that belongs to no level.
    #[must_use]
    pub fn note(message: String) -> Self {
        Self {
            message,
            at: None,
            level: None,
        }
    }
}

/// The problems, which of them are shown and where the cursor of F8 is.
#[derive(Debug, Clone, PartialEq)]
pub struct ProblemList {
    /// All problems, from all sources.
    pub rows: Vec<Problem>,
    /// Show the problems of every level, not only of the active one.
    pub all: bool,
    /// The row F8 went to last.
    pub cursor: Option<usize>,
    /// Draw a marker on the map for each problem that has a position.
    pub markers: bool,
}

impl Default for ProblemList {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            all: false,
            cursor: None,
            markers: true,
        }
    }
}

impl ProblemList {
    /// The rows to show while `active` is the active level: its own and those of no level, or all.
    #[must_use]
    pub fn shown(&self, active: Level) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, p)| self.all || p.level.is_none_or(|l| l == active))
            .map(|(i, _)| i)
            .collect()
    }

    /// Moves the cursor to the next shown problem, wrapping, and returns it.
    pub fn next(&mut self, active: Level) -> Option<&Problem> {
        self.step(active, true)
    }

    /// Moves the cursor to the previous shown problem, wrapping, and returns it.
    pub fn previous(&mut self, active: Level) -> Option<&Problem> {
        self.step(active, false)
    }

    fn step(&mut self, active: Level, forward: bool) -> Option<&Problem> {
        let shown = self.shown(active);
        let last = shown.len().checked_sub(1)?;
        let here = self.cursor.and_then(|c| shown.iter().position(|i| *i == c));
        let target = match (here, forward) {
            (None, true) => 0,
            (None, false) => last,
            (Some(p), true) => (p + 1) % shown.len(),
            (Some(p), false) => p.checked_sub(1).unwrap_or(last),
        };
        self.cursor = shown.get(target).copied();
        self.cursor.and_then(|i| self.rows.get(i))
    }
}

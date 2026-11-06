// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The per-user settings as the program uses them: where the file is, and tools from it.

use std::path::PathBuf;

use naves_core::prefs::{Preferences, config_dir, default_path};
use naves_export::tools::{ToolError, Tools, locate_preferring};

use crate::output;

/// The settings file, if the configuration folder is known.
pub fn path() -> Option<PathBuf> {
    config_dir().map(|dir| default_path(&dir))
}

/// The settings of this person, and what is wrong with the file if it is damaged. A damaged file gives the
/// defaults and is left as it is.
pub fn load() -> (Preferences, Option<String>) {
    let Some(path) = path() else {
        return (Preferences::default(), None);
    };
    match Preferences::load(&path) {
        Ok(prefs) => (prefs, None),
        Err(error) => (Preferences::default(), Some(error.to_string())),
    }
}

/// Finds the external tools: environment variables, then the settings, then the search. A damaged settings file
/// is a warning, and the settings are then left out.
///
/// # Errors
///
/// As [`locate_preferring`].
pub fn locate_tools() -> Result<Tools, ToolError> {
    let (prefs, damaged) = load();
    if let Some(problem) = damaged {
        output::note(&format!(
            "warning: the settings file is damaged and is ignored: {problem}"
        ));
    }
    locate_preferring(&prefs.tools)
}

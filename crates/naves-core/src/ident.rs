// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! ASCII identifiers for config class names, PBO prefixes and file names.

use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

/// Turns a project name into an ASCII identifier matching `^[a-z][a-z0-9_]*$`.
///
/// The name is decomposed (NFKD), combining marks are dropped, letters are lowercased, every run of
/// other characters becomes one `_` and leading and trailing `_` are removed. A result that does
/// not start with a letter gets a `w` prefix (so `"3 Gipfel"` becomes `"w3_gipfel"`).
#[must_use]
pub fn world_ident(name: &str) -> String {
    let mut out = String::new();
    let mut pending_sep = false;
    for c in name.nfkd().filter(|c| !is_combining_mark(*c)) {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if pending_sep && !out.is_empty() {
                out.push('_');
            }
            pending_sep = false;
            out.push(c);
        } else {
            pending_sep = true;
        }
    }
    if out.chars().next().is_none_or(|c| !c.is_ascii_lowercase()) {
        out.insert(0, 'w');
    }
    out
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! ASCII identifiers for config class names, PBO prefixes and file names.

use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

/// A printable ASCII version of a display name for places that only take ASCII (config strings): accents
/// are removed (NFKD), quotes and other characters outside `[A-Za-z0-9 ._-]` are dropped, and runs of
/// whitespace become one space. Names without any ASCII letter or digit give an empty string.
#[must_use]
pub fn ascii_name(name: &str) -> String {
    let kept: String = name
        .nfkd()
        .filter(|c| !is_combining_mark(*c))
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

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

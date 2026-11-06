// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Runs the fuzz seed corpora as ordinary tests: every seed must give `Ok` or `Err`, never a panic.

#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helper code; failing loudly is the point"
)]

use std::path::PathBuf;

use naves_formats::paa::{ColourTags, with_colour_tags};
use naves_formats::wrp8::EditableWorld;

fn seeds(target: &str) -> Vec<Vec<u8>> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/corpus")
        .join(target);
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no seeds in {}", dir.display());
    files.iter().map(|p| std::fs::read(p).unwrap()).collect()
}

#[test]
fn wrp8_read_seeds() {
    let mut ok = 0;
    for data in seeds("wrp8_read") {
        if EditableWorld::read(&mut &data[..]).is_ok() {
            ok += 1;
        }
    }
    assert_eq!(ok, 1, "only the roundtrip seed is a valid file");
}

#[test]
fn paa_tags_seeds() {
    let tags = ColourTags {
        average: [1, 2, 3, 4],
        flag: Some(1),
    };
    let ok = seeds("paa_tags")
        .iter()
        .filter(|data| with_colour_tags(data, &tags).is_ok())
        .count();
    assert_eq!(ok, 1, "only the dxt1 seed is a valid file");
}

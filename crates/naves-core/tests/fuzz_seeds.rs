// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Runs the fuzz seed corpora as ordinary tests: every seed must give `Ok` or `Err`, never a panic.

#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helper code; failing loudly is the point"
)]

use std::path::PathBuf;

use naves_core::project::Project;

fn seeds(target: &str) -> Vec<(String, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/corpus")
        .join(target);
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no seeds in {}", dir.display());
    files
        .iter()
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read(p).unwrap())
        })
        .collect()
}

#[test]
fn project_parse_seeds() {
    for (name, data) in seeds("project_parse") {
        let result = Project::parse(&String::from_utf8_lossy(&data));
        assert_eq!(result.is_ok(), name == "roundtrip", "seed {name}");
    }
}

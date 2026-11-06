// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The seeds of the fuzz targets of this crate run as ordinary tests.

use std::path::PathBuf;

use naves_core::imagery::catalog::catalog;
use naves_export::imagery_fetch::search_scene_count;

#[test]
fn stac_response_seeds() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/stac_response");
    let source = catalog(None)
        .unwrap()
        .into_iter()
        .find(|s| s.id == "sentinel2-scenes")
        .unwrap();
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let result = search_scene_count(&source, "visual", &std::fs::read(&path).unwrap());
        match name.as_str() {
            "valid" => assert_eq!(result.unwrap(), 2, "two scenes in the real answer"),
            _ => assert!(result.is_err(), "seed {name}"),
        }
        seen += 1;
    }
    assert!(seen >= 3, "seeds in {}", dir.display());
}

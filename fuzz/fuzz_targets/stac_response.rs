// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

#![no_main]

use libfuzzer_sys::fuzz_target;
use naves_core::imagery::catalog::catalog;
use naves_export::imagery_fetch::search_scene_count;

fuzz_target!(|data: &[u8]| {
    let Ok(sources) = catalog(None) else {
        return;
    };
    if let Some(source) = sources.iter().find(|s| s.id == "sentinel2-scenes") {
        let _ = search_scene_count(source, "visual", data);
    }
});

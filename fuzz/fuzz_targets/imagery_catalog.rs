// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

#![no_main]

use libfuzzer_sys::fuzz_target;
use naves_core::imagery::catalog::{options_for, parse_sources};
use naves_core::landcover::GeoWindow;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(sources) = parse_sources(text) {
        let window = GeoWindow {
            south: 47.5,
            west: 12.9,
            north: 47.6,
            east: 13.0,
        };
        let _ = options_for(&sources, window);
    }
});

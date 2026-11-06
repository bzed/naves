// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

#![no_main]

use libfuzzer_sys::fuzz_target;
use naves_core::landcover::GeoWindow;
use naves_core::osm::read_land_from_bytes;

fuzz_target!(|data: &[u8]| {
    let window = GeoWindow {
        south: 47.54,
        west: 12.96,
        north: 47.56,
        east: 12.99,
    };
    let _ = read_land_from_bytes(data, window);
});

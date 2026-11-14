// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

#![no_main]

use std::sync::Once;

use libfuzzer_sys::fuzz_target;
use naves_core::landcover::GeoWindow;
use naves_core::osm::read_ways_from_bytes;

static QUIET_PANICS: Once = Once::new();

fuzz_target!(|data: &[u8]| {
    // libFuzzer's panic hook aborts before the guard around the decoder can catch a panic, and the guard is
    // part of what is being tested: a panic inside `osmpbf` has to come out as an error, nothing else.
    QUIET_PANICS.call_once(|| std::panic::set_hook(Box::new(|_| {})));
    let window = GeoWindow {
        south: 47.596,
        west: 12.978,
        north: 47.606,
        east: 12.992,
    };
    let _ = read_ways_from_bytes(data, window);
});

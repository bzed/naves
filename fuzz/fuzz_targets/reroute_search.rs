// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

#![no_main]

use libfuzzer_sys::fuzz_target;

include!("../case/reroute_search.rs");

fuzz_target!(|data: &[u8]| {
    let _ = reroute_search_case::run(data);
});

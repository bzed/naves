// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

#![no_main]

use libfuzzer_sys::fuzz_target;

include!("../case/road_line.rs");

fuzz_target!(|data: &[u8]| {
    let _ = road_line_case::run(data);
});

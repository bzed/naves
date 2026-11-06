// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

#![no_main]

use libfuzzer_sys::fuzz_target;
use naves_formats::odol::read_odol;

fuzz_target!(|data: &[u8]| {
    let _ = read_odol(data);
});

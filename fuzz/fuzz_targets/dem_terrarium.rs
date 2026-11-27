// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

#![no_main]

use libfuzzer_sys::fuzz_target;
use naves_export::dem_fetch::{TilePyramid, terrarium};

fuzz_target!(|data: &[u8]| {
    if terrarium(data).is_ok() {
        let mut pyramid = TilePyramid::default();
        if pyramid.insert(16, 35130, 22901, data).is_ok() {
            let _ = pyramid.sample(47.557, 12.974, 16);
        }
    }
});

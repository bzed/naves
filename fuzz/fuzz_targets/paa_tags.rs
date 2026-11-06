// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

#![no_main]

use libfuzzer_sys::fuzz_target;
use naves_formats::paa::{ColourTags, with_colour_tags};

fuzz_target!(|data: &[u8]| {
    let tags = ColourTags {
        average: [1, 2, 3, 4],
        flag: Some(1),
    };
    let _ = with_colour_tags(data, &tags);
});

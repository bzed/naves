// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use naves_core::dem::DemTile;

fuzz_target!(|data: &[u8]| {
    let _ = DemTile::from_reader(Cursor::new(data));
});

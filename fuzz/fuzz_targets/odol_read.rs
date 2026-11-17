// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

#![no_main]

use std::collections::BTreeMap;

use libfuzzer_sys::fuzz_target;
use naves_formats::odol::read_odol;

fuzz_target!(|data: &[u8]| {
    // a model that reads is also measured as a bridge, a panel and the pipe
    if let Ok(model) = read_odol(data) {
        let models: BTreeMap<String, _> = [
            "dz\\structures\\roads\\bridges\\bridge_metal_25.p3d",
            "dz\\structures\\roads\\panels\\panel_concrete_1.p3d",
            "dz\\structures\\industrial\\misc\\misc_concretepipe.p3d",
        ]
        .into_iter()
        .map(|k| (k.to_string(), model.clone()))
        .collect();
        // no straights: an error after the parts are measured, never a panic
        let _ = naves_core::catalog::PieceCatalog::from_models(&models);
    }
});

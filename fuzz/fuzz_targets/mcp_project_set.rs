// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

#![no_main]

use libfuzzer_sys::fuzz_target;
use naves_core::project::Project;

// Arbitrary `{"path": ..., "value": ...}` requests applied to a sample project.
fuzz_target!(|data: &[u8]| {
    let Ok(request) = serde_json::from_slice::<serde_json::Value>(data) else {
        return;
    };
    let (Some(path), Some(value)) = (request.get("path").and_then(|p| p.as_str()), request.get("value")) else {
        return;
    };
    let project = Project::new("Fuzz", 47.0, 12.0, 4000.0);
    let _ = project.with_value(path, value.clone());
});

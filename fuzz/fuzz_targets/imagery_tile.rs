// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

#![no_main]

use libfuzzer_sys::fuzz_target;
use naves_core::imagery::ImageryRaster;
use naves_core::project::Project;

fuzz_target!(|data: &[u8]| {
    let project = Project::new("Fuzz", 47.557, 12.974, 800.0);
    if let Ok(Some(raster)) = ImageryRaster::from_bytes(data.to_vec(), &project, 2.0) {
        if let Ok(sampler) = raster.sampler() {
            let _ = sampler.rgb_at(47.557, 12.974);
        }
    }
});

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

#![no_main]

use libfuzzer_sys::fuzz_target;
use naves_core::smooth::{Sample, SmoothParams, resample, smooth_line};

/// Records of 17 bytes: x and y as little-endian `f64`, one flag byte (bit 0 fixed, bit 1 stiff).
fn decode(data: &[u8]) -> Vec<Sample> {
    data.as_chunks::<17>().0.iter()
        .filter_map(|c| {
            let x = f64::from_le_bytes(c.get(0..8)?.try_into().ok()?);
            let y = f64::from_le_bytes(c.get(8..16)?.try_into().ok()?);
            let flags = *c.get(16)?;
            Some(Sample {
                at: (x, y),
                fixed: flags & 1 != 0,
                stiffness: if flags & 2 != 0 { 10.0 } else { 1.0 },
            })
        })
        .collect()
}

fuzz_target!(|data: &[u8]| {
    let samples = decode(data);
    let _ = smooth_line(&samples, &SmoothParams::default());
    let line: Vec<(f64, f64)> = samples.iter().map(|s| s.at).collect();
    let _ = resample(&line, 2.0);
});

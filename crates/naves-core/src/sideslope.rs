// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The side slope of the original ground at the roads.

use crate::heightmap::Heightmap;
use crate::roadplace::PlacedNetwork;

/// Half the width over which the side slope is measured.
pub const SIDE_M: f64 = 10.0;
/// Pieces within this distance along the road share a median.
pub const SMOOTH_M: f64 = 25.0;

/// The side slope of the original ground at each road of `net` (rise over run, not negative), smoothed
/// along its chain; in the order of `net.roads`.
///
/// Raw value of a road: at its middle, the absolute difference of the ground height at [`SIDE_M`] to its left and
/// to its right, perpendicular to the line from its start to its end, over `2 * SIDE_M`. Smoothed: the median
/// of the raw values of the roads of its chain whose middles are within [`SMOOTH_M`] along the chain, so that a
/// noisy sample of the elevation model does not switch a rule on and off from piece to piece.
#[must_use]
pub fn side_slopes(net: &PlacedNetwork, base: &Heightmap) -> Vec<f64> {
    let raw: Vec<f64> = net
        .roads
        .iter()
        .map(|road| raw_slope(base, (road.start.x, road.start.y), (road.end.x, road.end.y)))
        .collect();
    let mut smooth = raw.clone();
    for chain in &net.chains {
        // the middle of each road along the chain
        let mut stations: Vec<(usize, f64)> = Vec::new();
        let mut run = 0.0;
        for &index in chain {
            let Some(road) = net.roads.get(index) else {
                continue;
            };
            let length = (road.end.x - road.start.x).hypot(road.end.y - road.start.y);
            stations.push((index, run + length / 2.0));
            run += length;
        }
        for &(index, station) in &stations {
            let mut near: Vec<f64> = stations
                .iter()
                .filter(|(_, other)| (other - station).abs() <= SMOOTH_M)
                .filter_map(|(other, _)| raw.get(*other).copied())
                .collect();
            if let Some(value) = median(&mut near)
                && let Some(slot) = smooth.get_mut(index)
            {
                *slot = value;
            }
        }
    }
    smooth
}

/// The slope across the line `from`-`to` at its middle.
fn raw_slope(base: &Heightmap, from: (f64, f64), to: (f64, f64)) -> f64 {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length = dx.hypot(dy);
    if length <= 0.0 {
        return 0.0;
    }
    let middle = (f64::midpoint(from.0, to.0), f64::midpoint(from.1, to.1));
    let (nx, ny) = (-dy / length, dx / length);
    let height = |side: f64| {
        f64::from(
            base.sample(middle.0 + side * SIDE_M * nx, middle.1 + side * SIDE_M * ny)
                .height,
        )
    };
    (height(1.0) - height(-1.0)).abs() / (2.0 * SIDE_M)
}

/// The median of `values` (the mean of the two in the middle for an even count), or `None` if there are none.
fn median(values: &mut [f64]) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n == 0 {
        return None;
    }
    let upper = values.get(n / 2).copied()?;
    if n % 2 == 1 {
        Some(upper)
    } else {
        let lower = values.get(n / 2 - 1).copied()?;
        Some(f64::midpoint(lower, upper))
    }
}

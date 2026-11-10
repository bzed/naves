// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Checks on the final terrain: the profile is a plan, this measures what came out, on the same heightmap
//! the game gets.
//!
//! * roads and drivable tracks have to be **drivable**: not steeper than the vehicles manage, not tilted
//!   across, and lying on the ground (not in the air, not buried);
//! * everything smaller (footpaths, and tracks that OSM calls very bad) only has to be **walkable**.

use crate::catalog::Family;
use crate::features::{Feature, FeatureKind};
use crate::heightmap::Heightmap;
use crate::limits::{CROSS_FALL_AIM, CROSS_FALL_LIMIT, drive_grade, grade_aim};
use crate::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad};

/// The steepest ground a player walks up, in degrees. **Provisional**: the roads plan measures it in the game
/// (Task 1, walk ramps); Livonia's navigation mesh allows agents 60°, which is an upper bound and not the
/// player's limit. It must stay above the cutting slope, so that a player can always leave a road.
pub const WALK_LIMIT_DEG: f64 = 50.0;
/// The most a road piece may be above or below the terrain inside its footprint.
pub const MAX_MISS_M: f64 = 0.15;

/// Length of the stretch of road over which a grade is measured.
const GRADE_WINDOW_M: f64 = 12.5;
/// Distance between the samples along a path.
const PATH_STEP_M: f64 = 2.0;
/// Steep ground shorter than this is a scramble, not an obstacle.
const MIN_STEEP_PATH_M: f64 = 6.0;
/// Steep sections closer than this are one.
const MERGE_STEEP_PATH_M: f64 = 10.0;

/// What a vehicle or a walker can do on a piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveState {
    /// Within the aim of its family.
    Fine,
    /// Steeper or more tilted across than the aim of its family, but not beyond what vehicles manage.
    Steep,
    /// A road that vehicles cannot climb or that tilts too much across.
    NotDrivable,
    /// A track that only has to be walkable, and is.
    WalkOnly,
    /// A track that only has to be walkable, and is too steep for that.
    TooSteepToWalk,
}

/// The measurements of one road piece on the final terrain.
#[derive(Debug, Clone, PartialEq)]
pub struct PieceCheck {
    /// The piece, as an index into [`PlacedNetwork::roads`].
    pub slot: usize,
    /// The surface.
    pub family: Family,
    /// Where it starts.
    pub start: (f64, f64),
    /// Where it ends.
    pub end: (f64, f64),
    /// Width of the road.
    pub width: f64,
    /// The steepest grade over any 12.5 m of it.
    pub grade: f64,
    /// The largest fall across it (rise over width).
    pub cross_fall: f64,
    /// The difference between its grade and the next piece's.
    pub grade_change: f64,
    /// The most it is above or below the terrain inside its footprint, with either way of splitting a cell.
    pub miss_m: f64,
    /// What follows from all that.
    pub state: DriveState,
}

/// A stretch of a path that is too steep to walk.
#[derive(Debug, Clone, PartialEq)]
pub struct SteepPathSection {
    /// The path (feature id).
    pub feature: String,
    /// Where the section starts, in map metres.
    pub from: (f64, f64),
    /// Where it ends.
    pub to: (f64, f64),
    /// Its length.
    pub length_m: f64,
    /// The steepest ground in it, in degrees.
    pub slope_deg: f64,
}

fn count_of(value: f64) -> usize {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a sample count from a length"
    )]
    let count = value.max(0.0).ceil() as usize;
    count
}

fn height(hm: &Heightmap, at: (f64, f64)) -> f64 {
    f64::from(hm.sample(at.0, at.1).height)
}

/// The height of the terrain at `(x, y)` when each grid cell is split into two triangles along one of its
/// diagonals: `rising` is the one from south-west to north-east.
fn triangle_height(hm: &Heightmap, x: f64, y: f64, rising: bool) -> f64 {
    let cell = f64::from(hm.cell_m).max(f64::MIN_POSITIVE);
    let last = f64::from(hm.size.saturating_sub(1));
    let (gx, gy) = ((x / cell).clamp(0.0, last), (y / cell).clamp(0.0, last));
    let (col, row) = (gx.floor(), gy.floor());
    let (fx, fy) = (gx - col, gy - row);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "grid indices from a clamped position"
    )]
    let (col, row) = (col as u32, row as u32);
    let at = |c: u32, r: u32| f64::from(hm.at(c, r));
    let (h00, h10, h01, h11) = (
        at(col, row),
        at(col + 1, row),
        at(col, row + 1),
        at(col + 1, row + 1),
    );
    if rising {
        if fx >= fy {
            h00 + fx * (h10 - h00) + fy * (h11 - h10)
        } else {
            h00 + fx * (h11 - h01) + fy * (h01 - h00)
        }
    } else if fx + fy <= 1.0 {
        h00 + fx * (h10 - h00) + fy * (h01 - h00)
    } else {
        h11 + (1.0 - fx) * (h01 - h11) + (1.0 - fy) * (h10 - h11)
    }
}

/// What is measured on one piece.
struct Measured {
    grade: f64,
    signed_grade: f64,
    cross_fall: f64,
    miss: f64,
    steepest_ground_deg: f64,
}

/// A piece's own axes: where it starts, the direction along it and the direction to its right.
struct Axes {
    origin: (f64, f64),
    along: (f64, f64),
    right: (f64, f64),
}

impl Axes {
    /// The map position `along` metres along the piece and `across` metres to its right.
    fn at(&self, along: f64, across: f64) -> (f64, f64) {
        (
            self.origin.0 + self.along.0 * along + self.right.0 * across,
            self.origin.1 + self.along.1 * along + self.right.1 * across,
        )
    }
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    f64::from(u32::try_from(numerator).unwrap_or(0))
        / f64::from(u32::try_from(denominator).unwrap_or(1).max(1))
}

/// The most the piece, as a quad through the terrain heights at its four corners, is above or below the
/// terrain inside its footprint, whichever way the cells are split.
fn quad_miss(hm: &Heightmap, axes: &Axes, length: f64, width: f64) -> f64 {
    let half = width / 2.0;
    let corner = |along: f64, across: f64| height(hm, axes.at(along, across));
    let (near_left, near_right) = (corner(0.0, -half), corner(0.0, half));
    let (far_left, far_right) = (corner(length, -half), corner(length, half));
    let step = (f64::from(hm.cell_m) / 2.0).max(0.25);
    let (along_count, across_count) = (
        count_of(length / step).max(1),
        count_of(width / step).max(1),
    );
    let mut miss = 0.0_f64;
    for i in 0..=along_count {
        for j in 0..=across_count {
            let (u, v) = (ratio(i, along_count), ratio(j, across_count));
            let surface = near_left * (1.0 - u) * (1.0 - v)
                + near_right * (1.0 - u) * v
                + far_left * u * (1.0 - v)
                + far_right * u * v;
            let (x, y) = axes.at(u * length, (v - 0.5) * width);
            for rising in [true, false] {
                miss = miss.max((surface - triangle_height(hm, x, y, rising)).abs());
            }
        }
    }
    miss
}

fn measure(road: &PlacedRoad, hm: &Heightmap) -> Option<Measured> {
    let (from, to) = ((road.start.x, road.start.y), (road.end.x, road.end.y));
    let length = (to.0 - from.0).hypot(to.1 - from.1);
    if length < 0.5 {
        return None;
    }
    let axes = Axes {
        origin: from,
        along: ((to.0 - from.0) / length, (to.1 - from.1) / length),
        right: ((to.1 - from.1) / length, -(to.0 - from.0) / length),
    };
    let count = count_of(length).max(1);
    let spacing = length / ratio(count, 1);
    let samples: Vec<(f64, f64)> = (0..=count)
        .map(|i| axes.at(spacing * ratio(i, 1), 0.0))
        .collect();
    let heights: Vec<f64> = samples.iter().map(|p| height(hm, *p)).collect();
    // over full windows only (a window cut short at the end of the piece would exaggerate a ramp that goes
    // on into the next piece); a piece shorter than a window is measured whole
    let window = count_of(GRADE_WINDOW_M / spacing).max(1).min(count);
    let mut grade = 0.0_f64;
    for (i, start) in heights.iter().enumerate().take(count - window + 1) {
        let run = spacing * ratio(window, 1);
        grade = grade.max((heights.get(i + window).copied().unwrap_or(*start) - start).abs() / run);
    }
    let half = road.width / 2.0;
    let cross_fall = samples
        .iter()
        .map(|p| {
            let left = height(hm, (p.0 - axes.right.0 * half, p.1 - axes.right.1 * half));
            let right = height(hm, (p.0 + axes.right.0 * half, p.1 + axes.right.1 * half));
            (left - right).abs() / road.width.max(0.1)
        })
        .fold(0.0, f64::max);
    let steepest_ground_deg = samples
        .iter()
        .map(|p| f64::from(hm.sample(p.0, p.1).slope_deg()))
        .fold(0.0, f64::max);
    let rise = heights.last().copied().unwrap_or(0.0) - heights.first().copied().unwrap_or(0.0);
    Some(Measured {
        grade,
        signed_grade: rise / length,
        cross_fall,
        miss: quad_miss(hm, &axes, length, road.width),
        steepest_ground_deg,
    })
}

/// Measures every road piece and junction of `net` on the terrain `hm`; bridges and end caps are not checked
/// (nothing under a bridge has to fit).
#[must_use]
pub fn check_roads_on_terrain(net: &PlacedNetwork, hm: &Heightmap) -> Vec<PieceCheck> {
    let mut checks: Vec<Option<PieceCheck>> = Vec::new();
    let mut signed: Vec<f64> = Vec::new();
    for (slot, road) in net.roads.iter().enumerate() {
        let measured = matches!(road.kind, PlacedKind::Piece | PlacedKind::Junction)
            .then(|| measure(road, hm))
            .flatten();
        signed.push(measured.as_ref().map_or(0.0, |m| m.signed_grade));
        checks.push(measured.map(|m| {
            let state = if road.drivable {
                if m.grade > drive_grade(road.family) || m.cross_fall > CROSS_FALL_LIMIT {
                    DriveState::NotDrivable
                } else if m.grade > grade_aim(road.family) || m.cross_fall > CROSS_FALL_AIM {
                    DriveState::Steep
                } else {
                    DriveState::Fine
                }
            } else if m.steepest_ground_deg > WALK_LIMIT_DEG {
                DriveState::TooSteepToWalk
            } else {
                DriveState::WalkOnly
            };
            PieceCheck {
                slot,
                family: road.family,
                start: (road.start.x, road.start.y),
                end: (road.end.x, road.end.y),
                width: road.width,
                grade: m.grade,
                cross_fall: m.cross_fall,
                grade_change: 0.0,
                miss_m: m.miss,
                state,
            }
        }));
    }
    for chain in &net.chains {
        for pair in chain.windows(2) {
            let [earlier, later] = pair else { continue };
            let after = signed.get(*later).copied().unwrap_or(0.0);
            let before = signed.get(*earlier).copied().unwrap_or(0.0);
            let both = checks.get(*later).is_some_and(Option::is_some);
            if let (true, Some(Some(check))) = (both, checks.get_mut(*earlier)) {
                check.grade_change = (after - before).abs();
            }
        }
    }
    checks.into_iter().flatten().collect()
}

/// A run of steep samples along a path: the arclength of its first and last sample.
struct Steep {
    from: f64,
    to: f64,
    from_at: (f64, f64),
    to_at: (f64, f64),
    slope: f64,
}

/// The stretches of paths, and of tracks that need not be drivable, where the ground is too steep to walk.
///
/// The ground is read every 2 m. A stretch counts when it is steep over at least 6 m; stretches less than
/// 10 m apart are one.
#[must_use]
pub fn check_walkable(features: &[Feature], hm: &Heightmap) -> Vec<SteepPathSection> {
    let mut out = Vec::new();
    for feature in features {
        let walk_only = match feature.kind {
            FeatureKind::Path { .. } => true,
            FeatureKind::Road { drivable, .. } => !drivable,
        };
        if !walk_only {
            continue;
        }
        let mut runs: Vec<Steep> = Vec::new();
        let mut open = false;
        let mut travelled = 0.0;
        let mut next_sample = 0.0;
        for pair in feature.points.windows(2) {
            let [a, b] = pair else { continue };
            let length = (b.0 - a.0).hypot(b.1 - a.1);
            while next_sample <= travelled + length + 1e-9 {
                let t = if length > 0.0 {
                    (next_sample - travelled) / length
                } else {
                    0.0
                };
                let at = (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
                let slope = f64::from(hm.sample(at.0, at.1).slope_deg());
                if slope > WALK_LIMIT_DEG {
                    match runs.last_mut().filter(|_| open) {
                        Some(run) => {
                            run.to = next_sample;
                            run.to_at = at;
                            run.slope = run.slope.max(slope);
                        }
                        None => runs.push(Steep {
                            from: next_sample,
                            to: next_sample,
                            from_at: at,
                            to_at: at,
                            slope,
                        }),
                    }
                    open = true;
                } else {
                    open = false;
                }
                next_sample += PATH_STEP_M;
            }
            travelled += length;
        }
        runs.retain(|r| r.to - r.from + PATH_STEP_M >= MIN_STEEP_PATH_M);
        let mut merged: Vec<Steep> = Vec::new();
        for run in runs {
            match merged.last_mut() {
                Some(last) if run.from - last.to < MERGE_STEEP_PATH_M => {
                    last.to = run.to;
                    last.to_at = run.to_at;
                    last.slope = last.slope.max(run.slope);
                }
                _ => merged.push(run),
            }
        }
        out.extend(merged.into_iter().map(|run| SteepPathSection {
            feature: feature.id.clone(),
            from: run.from_at,
            to: run.to_at,
            length_m: run.to - run.from + PATH_STEP_M,
            slope_deg: run.slope,
        }));
    }
    out
}

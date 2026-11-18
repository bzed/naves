// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a real car manages on each road surface: the grade it is aimed at, the steepest grade it can
//! climb, the grade a road may reach when holding its aim would take it off the ground, and the smallest
//! vertical radius.
//!
//! Every number here is an estimate from real cars, **not measured in the game**:
//! `docs/research/2026-11-10-road-alignment.md`, section 3. Task 1 of the roads plan (drive ramps) replaces
//! them when it has been measured.

use crate::catalog::Family;

/// The grade a road of this family is aimed at (rise over run).
///
/// Forest roads are built at 8 to 12 %, 15 to 18 % over short lengths. Gravel and dirt tracks keep their
/// steeper aims, and they stay below the ceilings of the tracks.
///
/// **Decided by the maintainer (2026-11-13):** tracks in the mountains are not expected to be drivable by car;
/// a passenger car manages 30 to 40 %, a 4WD 45 to 60 %. Gravel tracks are therefore held to a car at the top
/// of its range, dirt tracks to a 4WD (see [`drive_grade`]). These are estimates from real vehicles, not
/// driven in the game: `docs/research/2026-11-10-road-alignment.md`, section 3.
#[must_use]
pub fn grade_aim(family: Family) -> f64 {
    match family {
        Family::Asf1 => 0.08,
        Family::Asf2 => 0.10,
        Family::Asf3 => 0.12,
        Family::City => 0.14,
        Family::Grav => 0.18,
        Family::Mud => 0.25,
    }
}

/// The steepest grade a vehicle climbs on this surface (rise over run).
///
/// Asphalt and streets are held to what a car manages, 30 %.
///
/// **Decided by the maintainer (2026-11-13):** tracks in the mountains are not expected to be drivable by car;
/// a passenger car manages 30 to 40 %, a 4WD 45 to 60 %; gravel tracks therefore get a hard limit of 40 % and
/// dirt tracks 50 % (ceilings 36 % and 45 %), replacing the 30 % and 28 % of the earlier decision (gravel and
/// dirt kept the limits of tractors). These are estimates from real vehicles, not driven in the game:
/// `docs/research/2026-11-10-road-alignment.md`, section 3.
#[must_use]
pub fn drive_grade(family: Family) -> f64 {
    match family {
        Family::Asf1 | Family::Asf2 | Family::Asf3 | Family::City => 0.30,
        Family::Grav => 0.40,
        Family::Mud => 0.50,
    }
}

/// The steepest grade a road may take when holding its aim would leave the ground: nine tenths of what a
/// vehicle manages.
///
/// This is an estimate from real cars, not measured in the game:
/// `docs/research/2026-11-10-road-alignment.md`, section 3.
#[must_use]
pub fn grade_ceiling(family: Family) -> f64 {
    0.9 * drive_grade(family)
}

/// The smallest vertical radius a road of this family may have, in metres.
///
/// A car leaves the ground over a crest of radius `v² / g`, 79 m at 100 km/h. This is an estimate from real
/// cars, not measured in the game: `docs/research/2026-11-10-road-alignment.md`, section 3.
#[must_use]
pub fn min_vertical_radius(family: Family) -> f64 {
    match family {
        Family::Asf1 => 600.0,
        Family::Asf2 => 400.0,
        Family::Asf3 => 250.0,
        Family::City => 150.0,
        Family::Grav => 100.0,
        Family::Mud => 60.0,
    }
}

/// The fall across a road that it is aimed at (rise over width).
///
/// Roads are built with 2.5 % across on a straight and up to 7 or 8 % in a bend. This is an estimate from
/// real cars, not measured in the game: `docs/research/2026-11-10-road-alignment.md`, section 3.
pub const CROSS_FALL_AIM: f64 = 0.08;

/// The largest fall across a road that a vehicle can take (rise over width).
///
/// This is an estimate from real cars, not measured in the game:
/// `docs/research/2026-11-10-road-alignment.md`, section 3.
pub const CROSS_FALL_LIMIT: f64 = 0.20;

/// How far from the original ground a road may be planned, in metres, on ground whose side slope is
/// `side_slope` (rise over run, across the road).
///
/// 30 m on gentle ground (up to 0.20), growing linearly to 100 m at 0.60 and staying there on steeper
/// ground, where a road is cut into or built out from the mountainside. A NaN slope (no ground data) gets the
/// 30 m. This is a design choice, not measured in the game: `docs/superpowers/specs/2026-11-13-road-heights-design.md`,
/// section 5.1.
#[must_use]
pub fn earthwork_allowance_m(side_slope: f64) -> f64 {
    if side_slope.is_nan() {
        return 30.0;
    }
    30.0 + (side_slope.clamp(0.20, 0.60) - 0.20) / 0.40 * 70.0
}

/// How far from a junction's patch or from the end of a bridge deck, measured along the road, a short ramp is
/// allowed to bend more sharply than the comfort radius of the family says, in metres: one long road piece.
///
/// An estimate, **not driven in the game**: `docs/superpowers/specs/2026-11-18-ramps-design.md`.
pub const RAMP_ZONE_M: f64 = 25.0;

/// The largest change of grade at one joint inside a ramp zone (see [`RAMP_ZONE_M`]), rise over run: 0.10, about
/// 5.7 degrees. A car with ordinary ground clearance is expected to take such a break at walking speed without
/// grounding; two of them [`RAMP_JOINT_MIN_M`] apart make a 20 % ramp.
///
/// The limit used in a ramp zone is the larger of this and what the family's vertical radius allows over the run.
/// An estimate, **not driven in the game**: `docs/superpowers/specs/2026-11-18-ramps-design.md`.
pub const GRADE_BREAK_MAX: f64 = 0.10;

/// The change of grade at one joint inside a ramp zone (see [`RAMP_ZONE_M`]) that the height solve gives up only for
/// the grade ceiling: 0.20, twice [`GRADE_BREAK_MAX`], about 11.3 degrees. Between the two a break is a finding that a
/// hold or a clearance may cost; above it a break costs as much as a hold that is given up, so that a road is not
/// bent this sharply to buy a clearance or a hold (a bump a car does not take at any speed, a "sudden very short
/// change of steepness").
///
/// An estimate, **not driven in the game**: `docs/superpowers/specs/2026-11-18-ramps-design.md`.
pub const GRADE_BREAK_HARD: f64 = 0.20;

/// A joint that the height solve adds where a road enters a junction's patch is not added closer than this to a
/// joint the road already has, in metres (one terrain cell at terrain size 2048).
pub const RAMP_JOINT_MIN_M: f64 = 4.0;

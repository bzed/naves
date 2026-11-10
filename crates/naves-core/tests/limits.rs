// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The vehicle limits of the road families.

#![expect(clippy::float_cmp, reason = "the documented limits are exact values")]

use naves_core::catalog::Family;
use naves_core::limits::{
    CROSS_FALL_AIM, CROSS_FALL_LIMIT, drive_grade, grade_aim, grade_ceiling, min_vertical_radius,
};

#[test]
fn every_aim_is_below_its_ceiling_and_the_ceiling_below_the_limit() {
    for family in [
        Family::Asf1,
        Family::Asf2,
        Family::Asf3,
        Family::City,
        Family::Grav,
        Family::Mud,
    ] {
        assert!(grade_aim(family) < grade_ceiling(family), "{family:?}");
        assert!(grade_ceiling(family) < drive_grade(family), "{family:?}");
        assert!(min_vertical_radius(family) >= 60.0, "{family:?}");
    }
    const { assert!(CROSS_FALL_AIM < CROSS_FALL_LIMIT) };
}

#[test]
fn the_limits_are_the_documented_ones() {
    // docs/research/2026-11-10-road-alignment.md, section 3
    assert_eq!(
        [
            drive_grade(Family::Asf3),
            drive_grade(Family::Grav),
            drive_grade(Family::Mud)
        ]
        .map(|v| (v * 100.0).round()),
        [30.0, 30.0, 28.0]
    );
    assert_eq!(
        [grade_aim(Family::Grav), grade_aim(Family::Mud)].map(|v| (v * 100.0).round()),
        [18.0, 25.0]
    );
}

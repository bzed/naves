// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The vehicle limits of the road families.

#![expect(clippy::float_cmp, reason = "the documented limits are exact values")]

use naves_core::catalog::Family;
use naves_core::limits::{
    CROSS_FALL_AIM, CROSS_FALL_LIMIT, drive_grade, earthwork_allowance_m, grade_aim, grade_ceiling,
    min_vertical_radius,
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
        [30.0, 40.0, 50.0]
    );
    assert_eq!(
        [grade_aim(Family::Grav), grade_aim(Family::Mud)].map(|v| (v * 100.0).round()),
        [18.0, 25.0]
    );
}

#[test]
fn tracks_have_the_limits_of_a_car_at_its_best_and_of_a_4wd() {
    assert!((drive_grade(Family::Grav) - 0.40).abs() < 1e-12);
    assert!((drive_grade(Family::Mud) - 0.50).abs() < 1e-12);
    assert!((grade_ceiling(Family::Grav) - 0.36).abs() < 1e-12);
    assert!((grade_ceiling(Family::Mud) - 0.45).abs() < 1e-12);
    for family in [Family::Asf1, Family::Asf2, Family::Asf3, Family::City] {
        assert!((drive_grade(family) - 0.30).abs() < 1e-12);
    }
    for family in Family::ALL {
        assert!(grade_aim(family) < grade_ceiling(family));
    }
}

#[test]
fn the_allowance_grows_with_the_side_slope() {
    for (slope, want) in [
        (0.0, 30.0),
        (0.20, 30.0),
        (0.40, 65.0),
        (0.60, 100.0),
        (2.0, 100.0),
    ] {
        assert!(
            (earthwork_allowance_m(slope) - want).abs() < 1e-9,
            "{slope}"
        );
    }
    assert!((earthwork_allowance_m(f64::NAN) - 30.0).abs() < 1e-9);
}

#[test]
fn the_allowance_is_30_m_at_a_negative_slope_and_100_m_at_an_infinite_one() {
    assert!((earthwork_allowance_m(-0.5) - 30.0).abs() < 1e-9);
    assert!((earthwork_allowance_m(f64::NEG_INFINITY) - 30.0).abs() < 1e-9);
    assert!((earthwork_allowance_m(f64::INFINITY) - 100.0).abs() < 1e-9);
}

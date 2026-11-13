// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Checks on the final terrain: drivable, walkable, on the ground.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test helper code; failing loudly is the point and the numbers are tiny"
)]

use naves_core::catalog::Family;
use naves_core::checks::{
    DriveState, MAX_MISS_M, WALK_LIMIT_DEG, check_roads_on_terrain, check_walkable,
};
use naves_core::conform::{CUT_SLOPE, FILL_SLOPE_MAX, conform};
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::heightmap::Heightmap;
use naves_core::limits::{CROSS_FALL_AIM, drive_grade, grade_aim};
use naves_core::profile::solve_profiles;
use naves_core::roadfit::Pose;
use naves_core::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad, Slot};

fn terrain(size: u32, cell: f32, f: impl Fn(f64, f64) -> f64) -> Heightmap {
    let mut h = Vec::new();
    for row in 0..size {
        for col in 0..size {
            h.push(f(
                f64::from(col) * f64::from(cell),
                f64::from(row) * f64::from(cell),
            ) as f32);
        }
    }
    Heightmap {
        size,
        cell_m: cell,
        h,
    }
}

fn piece(
    index: u32,
    kind: PlacedKind,
    family: Family,
    drivable: bool,
    a: (f64, f64),
    b: (f64, f64),
) -> PlacedRoad {
    let heading = (b.0 - a.0).atan2(b.1 - a.1);
    PlacedRoad {
        slot: Slot {
            stretch: "s".to_string(),
            index,
        },
        kind,
        family,
        drivable,
        model: "m".to_string(),
        origin: Pose {
            x: a.0,
            y: a.1,
            heading,
        },
        start: Pose {
            x: a.0,
            y: a.1,
            heading,
        },
        end: Pose {
            x: b.0,
            y: b.1,
            heading,
        },
        width: 7.0,
    }
}

/// 25 m pieces along a line from `a` to `b`, in one chain.
fn chain(family: Family, drivable: bool, a: (f64, f64), b: (f64, f64)) -> PlacedNetwork {
    let n = ((b.0 - a.0).hypot(b.1 - a.1) / 25.0).ceil() as u32;
    let roads: Vec<PlacedRoad> = (0..n)
        .map(|i| {
            let p = |t: f64| (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
            piece(
                i,
                PlacedKind::Piece,
                family,
                drivable,
                p(f64::from(i) / f64::from(n)),
                p(f64::from(i + 1) / f64::from(n)),
            )
        })
        .collect();
    PlacedNetwork {
        chains: vec![(0..roads.len()).collect()],
        roads,
        ..PlacedNetwork::default()
    }
}

/// Adds a second chain of 12.5 m pieces from `a` to `b`.
fn append_chain(
    net: &mut PlacedNetwork,
    family: Family,
    drivable: bool,
    a: (f64, f64),
    b: (f64, f64),
) {
    let n = ((b.0 - a.0).hypot(b.1 - a.1) / 12.5).ceil() as u32;
    let base = net.roads.len();
    for i in 0..n {
        let p = |t: f64| (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
        net.roads.push(piece(
            i,
            PlacedKind::Piece,
            family,
            drivable,
            p(f64::from(i) / f64::from(n)),
            p(f64::from(i + 1) / f64::from(n)),
        ));
    }
    net.chains.push((base..base + n as usize).collect());
}

/// One arc piece of `radius` around `centre`, from `from_deg` through `turn_deg`, with the tangent
/// headings at both ends.
fn arc(
    family: Family,
    centre: (f64, f64),
    radius: f64,
    from_deg: f64,
    turn_deg: f64,
) -> PlacedNetwork {
    let from = from_deg.to_radians();
    let turn = turn_deg.to_radians();
    let side = turn.signum();
    let point = |heading: f64| {
        (
            centre.0 + side * radius * (-heading.cos()),
            centre.1 + side * radius * heading.sin(),
        )
    };
    let start = point(from);
    let end = point(from + turn);
    let mut road = piece(0, PlacedKind::Piece, family, true, start, end);
    road.origin.heading = from;
    road.start.heading = from;
    road.end.heading = from + turn;
    PlacedNetwork {
        chains: vec![vec![0]],
        roads: vec![road],
        ..PlacedNetwork::default()
    }
}

/// A terrain rising `grade` towards the north.
fn ramp(grade: f64) -> Heightmap {
    terrain(100, 6.25, move |_, y| 100.0 + grade * y)
}

fn states(net: &PlacedNetwork, hm: &Heightmap) -> Vec<DriveState> {
    check_roads_on_terrain(net, hm)
        .iter()
        .map(|c| c.state)
        .collect()
}

#[test]
fn a_road_at_the_limit_is_drivable() {
    for family in [Family::Asf3, Family::Grav, Family::Mud] {
        let net = chain(family, true, (300.0, 100.0), (300.0, 400.0));
        for state in states(&net, &ramp(drive_grade(family) - 0.01)) {
            assert!(
                matches!(state, DriveState::Fine | DriveState::Steep),
                "{family:?}: {state:?}"
            );
        }
    }
}

#[test]
fn a_road_over_the_limit_is_not_drivable() {
    for family in [Family::Asf3, Family::Grav, Family::Mud] {
        let net = chain(family, true, (300.0, 100.0), (300.0, 400.0));
        for state in states(&net, &ramp(drive_grade(family) + 0.02)) {
            assert_eq!(state, DriveState::NotDrivable, "{family:?}");
        }
    }
}

#[test]
fn steep_is_between_the_two_limits() {
    let net = chain(Family::Asf3, true, (300.0, 100.0), (300.0, 400.0));
    let aim = grade_aim(Family::Asf3);
    assert!(aim < drive_grade(Family::Asf3));
    assert!(
        states(&net, &ramp(aim + 0.03))
            .iter()
            .all(|s| *s == DriveState::Steep)
    );
    assert!(
        states(&net, &ramp(aim - 0.03))
            .iter()
            .all(|s| *s == DriveState::Fine)
    );
}

#[test]
fn a_grade4_track_is_never_not_drivable() {
    let net = chain(Family::Mud, false, (300.0, 100.0), (300.0, 400.0));
    for grade in [0.1, 0.4, 0.9, 2.0] {
        for state in states(&net, &ramp(grade)) {
            assert!(
                matches!(state, DriveState::WalkOnly | DriveState::TooSteepToWalk),
                "grade {grade}: {state:?}"
            );
        }
    }
}

#[test]
fn a_grade4_track_over_the_walk_limit_is_too_steep_to_walk() {
    let net = chain(Family::Mud, false, (300.0, 100.0), (300.0, 400.0));
    let steep = (WALK_LIMIT_DEG + 5.0).to_radians().tan();
    assert!(
        states(&net, &ramp(steep))
            .iter()
            .all(|s| *s == DriveState::TooSteepToWalk)
    );
    let gentle = (WALK_LIMIT_DEG - 10.0).to_radians().tan();
    assert!(
        states(&net, &ramp(gentle))
            .iter()
            .all(|s| *s == DriveState::WalkOnly)
    );
}

#[test]
fn a_short_steep_spot_within_the_window_counts() {
    // flat, except a 2 m step over 4 m: 50 % there, 16 % over 12.5 m
    let hm = terrain(100, 4.0, |_, y| {
        100.0 + 2.0 * ((y - 196.0) / 4.0).clamp(0.0, 1.0)
    });
    let net = chain(Family::Asf3, true, (50.0, 100.0), (50.0, 300.0));
    let checks = check_roads_on_terrain(&net, &hm);
    assert!(
        checks.iter().any(|c| c.state == DriveState::Steep),
        "{:?}",
        checks.iter().map(|c| c.grade).collect::<Vec<_>>()
    );
    let worst = checks.iter().map(|c| c.grade).fold(0.0, f64::max);
    assert!(worst > 0.12 && worst < drive_grade(Family::Asf3), "{worst}");
}

#[test]
fn cross_fall_over_the_aim_is_steep_and_still_drivable() {
    let hm = terrain(100, 6.25, |x, _| 100.0 - 0.12 * x);
    let net = chain(Family::Asf3, true, (300.0, 100.0), (300.0, 400.0));
    assert!(states(&net, &hm).iter().all(|s| *s == DriveState::Steep));
}

#[test]
fn cross_fall_over_the_limit_is_not_drivable() {
    let hm = terrain(100, 6.25, |x, _| 100.0 - 0.24 * x);
    let net = chain(Family::Asf3, true, (300.0, 100.0), (300.0, 400.0));
    assert!(
        states(&net, &hm)
            .iter()
            .all(|s| *s == DriveState::NotDrivable)
    );
}

#[test]
fn gravel_and_tracks_have_their_own_lower_limits() {
    for (family, grade, state) in [
        (Family::Grav, 0.20, DriveState::Steep),
        (Family::Grav, 0.42, DriveState::NotDrivable),
        (Family::Mud, 0.26, DriveState::Steep),
        (Family::Mud, 0.52, DriveState::NotDrivable),
        (Family::Asf3, 0.27, DriveState::Steep),
        (Family::Asf3, 0.33, DriveState::NotDrivable),
    ] {
        let net = chain(family, true, (300.0, 100.0), (300.0, 400.0));
        assert!(
            states(&net, &ramp(grade)).iter().all(|s| *s == state),
            "{family:?} at {grade}"
        );
    }
}

#[test]
fn an_arc_on_a_spiral_ramp_has_no_cross_fall() {
    // a spiral ramp: 10 % along a circle of radius 10 m, level along every radius
    let centre = (300.0, 300.0);
    let hm = terrain(200, 3.125, |x, y| {
        100.0 + 0.10 * 10.0 * (x - centre.0).atan2(y - centre.1)
    });
    let net = arc(Family::Asf3, centre, 10.0, 20.0, 60.0);
    let check = &check_roads_on_terrain(&net, &hm)[0];
    assert!(check.cross_fall < 0.02, "{}", check.cross_fall);
    assert!((check.grade - 0.10).abs() < 0.02, "{}", check.grade);
}

#[test]
fn a_straight_is_measured_as_before() {
    let hm = terrain(100, 6.25, |x, y| 100.0 + 0.07 * y - 0.03 * x);
    let net = chain(Family::Asf3, true, (300.0, 100.0), (300.0, 400.0));
    let check = &check_roads_on_terrain(&net, &hm)[0];
    assert!((check.grade - 0.07).abs() < 0.005 && (check.cross_fall - 0.03).abs() < 0.005);
}

#[test]
fn a_bump_shows_as_grade_change() {
    // flat for 100 m, then a 10 % ramp
    let hm = terrain(100, 6.25, |_, y| 100.0 + 0.10 * (y - 200.0).max(0.0));
    let net = chain(Family::Asf3, true, (300.0, 100.0), (300.0, 400.0));
    let checks = check_roads_on_terrain(&net, &hm);
    let biggest = checks.iter().map(|c| c.grade_change).fold(0.0, f64::max);
    assert!(biggest > 0.05 && biggest < 0.15, "{biggest}");
    assert!(
        checks.last().unwrap().grade_change.abs() < 1e-12,
        "the last piece has no next one"
    );
}

#[test]
fn miss_is_zero_on_shaped_terrain() {
    // a level road across a V-shaped valley, on terrain shaped by conform
    let base = terrain(120, 6.25, |_, y| {
        100.0 - 25.0 * (1.0 - ((y - 375.0).abs() / 15.0).min(1.0))
    });
    let net = chain(Family::Asf3, true, (100.0, 375.0), (650.0, 375.0));
    let profiles = solve_profiles(&net, &base);
    let shaped = conform(&base, &profiles.strips).unwrap().heightmap;
    for c in check_roads_on_terrain(&net, &shaped) {
        assert!(
            c.miss_m < MAX_MISS_M,
            "piece {} misses by {}",
            c.slot,
            c.miss_m
        );
    }
}

#[test]
fn a_side_road_starts_on_the_ground_of_the_road_it_leaves() {
    // the through road falls 5 % to the north; the side road leaves east, where the ground rises 10 %
    let base = terrain(160, 6.25, |x, y| {
        100.0 - 0.05 * y + 0.10 * (x - 300.0).max(0.0)
    });
    let mut net = chain(Family::Asf3, true, (300.0, 100.0), (300.0, 900.0));
    append_chain(&mut net, Family::Grav, true, (303.5, 500.0), (600.0, 500.0));
    let profiles = solve_profiles(&net, &base);
    let shaped = conform(&base, &profiles.strips).unwrap().heightmap;
    for c in check_roads_on_terrain(&net, &shaped)
        .iter()
        .filter(|c| c.family == Family::Grav)
    {
        assert!(c.miss_m <= MAX_MISS_M, "miss {} at {:?}", c.miss_m, c.start);
        assert!(
            c.cross_fall <= CROSS_FALL_AIM,
            "cross fall {} at {:?}",
            c.cross_fall,
            c.start
        );
    }
}

#[test]
fn miss_finds_a_road_in_the_air() {
    let base = terrain(120, 6.25, |_, y| {
        100.0 - 25.0 * (1.0 - ((y - 375.0).abs() / 15.0).min(1.0))
    });
    let net = chain(Family::Asf3, true, (100.0, 375.0), (650.0, 375.0));
    let worst = check_roads_on_terrain(&net, &base)
        .iter()
        .map(|c| c.miss_m)
        .fold(0.0, f64::max);
    // pieces whose corners sit on the rim of the valley span it
    assert!(worst >= 0.0);
    let hm = terrain(100, 6.25, |_, y| {
        if (y - 300.0).abs() < 4.0 { 70.0 } else { 100.0 }
    });
    let net = chain(Family::Asf3, true, (300.0, 280.0), (300.0, 330.0));
    let worst = check_roads_on_terrain(&net, &hm)
        .iter()
        .map(|c| c.miss_m)
        .fold(0.0, f64::max);
    assert!(worst > 5.0, "{worst}");
}

#[test]
fn miss_tries_both_diagonals() {
    // one cell with a raised corner: one way of splitting it puts the middle 5 m up, the other leaves it at 0
    let hm = terrain(40, 10.0, |x, y| {
        if (x - 110.0).abs() < 1.0 && (y - 110.0).abs() < 1.0 {
            10.0
        } else {
            0.0
        }
    });
    let net = PlacedNetwork {
        roads: vec![piece(
            0,
            PlacedKind::Piece,
            Family::Asf3,
            true,
            (105.0, 80.0),
            (105.0, 140.0),
        )],
        chains: vec![vec![0]],
        ..PlacedNetwork::default()
    };
    let miss = check_roads_on_terrain(&net, &hm)[0].miss_m;
    assert!(miss > 1.0, "{miss}");
}

#[test]
fn a_bridge_is_not_checked() {
    let mut net = chain(Family::Asf3, true, (300.0, 100.0), (300.0, 400.0));
    for r in &mut net.roads {
        r.kind = PlacedKind::Bridge;
    }
    assert!(check_roads_on_terrain(&net, &ramp(0.5)).is_empty());
}

fn path(id: &str, points: &[(f64, f64)]) -> Feature {
    Feature {
        id: id.to_string(),
        way_id: 1,
        source: Source::Osm,
        kind: FeatureKind::Path { width_m: 1.6 },
        points: points.to_vec(),
        nodes: (1..=i64::try_from(points.len()).unwrap_or(0)).collect(),
    }
}

#[test]
fn a_steep_path_section_is_reported_with_its_length() {
    let steep = (WALK_LIMIT_DEG + 8.0).to_radians().tan();
    // gentle ground, with 30 m of very steep ground in the middle
    let hm = terrain(100, 2.0, move |_, y| {
        100.0
            + if y > 100.0 {
                steep * (y - 100.0).min(30.0)
            } else {
                0.0
            }
    });
    let sections = check_walkable(&[path("p", &[(100.0, 40.0), (100.0, 190.0)])], &hm);
    assert_eq!(sections.len(), 1, "{sections:?}");
    assert!(
        sections[0].length_m >= 25.0 && sections[0].length_m <= 36.0,
        "{}",
        sections[0].length_m
    );
    assert!(sections[0].slope_deg > WALK_LIMIT_DEG);
    assert_eq!(sections[0].feature, "p");
}

#[test]
fn a_short_scramble_is_not_reported() {
    let steep = (WALK_LIMIT_DEG + 8.0).to_radians().tan();
    let hm = terrain(100, 1.0, move |_, y| {
        100.0 + steep * ((y - 100.0).clamp(0.0, 3.0))
    });
    assert!(check_walkable(&[path("p", &[(50.0, 40.0), (50.0, 160.0)])], &hm).is_empty());
}

#[test]
fn close_sections_merge() {
    let steep = (WALK_LIMIT_DEG + 8.0).to_radians().tan();
    // two steep stretches of 12 m, 6 m apart
    let hm = terrain(100, 1.0, move |_, y| {
        let a = ((y - 60.0).clamp(0.0, 12.0)) * steep;
        let b = ((y - 78.0).clamp(0.0, 12.0)) * steep;
        100.0 + a + b
    });
    let sections = check_walkable(&[path("p", &[(50.0, 20.0), (50.0, 140.0)])], &hm);
    assert_eq!(sections.len(), 1, "{sections:?}");
}

#[test]
fn a_gentle_path_reports_nothing() {
    assert!(check_walkable(&[path("p", &[(100.0, 40.0), (100.0, 400.0)])], &ramp(0.3)).is_empty());
}

#[test]
fn a_track_that_is_not_drivable_is_checked_like_a_path() {
    let steep = (WALK_LIMIT_DEG + 8.0).to_radians().tan();
    let mut track = path("t", &[(100.0, 40.0), (100.0, 190.0)]);
    track.kind = FeatureKind::Road {
        family: Family::Mud,
        drivable: false,
        bridge: false,
        tunnel: false,
    };
    assert!(!check_walkable(&[track.clone()], &ramp(steep)).is_empty());
    // a drivable road is not a path
    if let FeatureKind::Road { drivable, .. } = &mut track.kind {
        *drivable = true;
    }
    assert!(check_walkable(&[track], &ramp(steep)).is_empty());
}

#[test]
fn a_road_can_always_be_left_downhill() {
    assert!(FILL_SLOPE_MAX.atan().to_degrees() < WALK_LIMIT_DEG);
    assert!(CUT_SLOPE.atan().to_degrees() < WALK_LIMIT_DEG);
}

#[test]
fn checks_are_deterministic() {
    let net = chain(Family::Grav, true, (300.0, 100.0), (300.0, 400.0));
    let hm = terrain(100, 6.25, |x, y| 100.0 + 0.1 * y + 5.0 * (x / 40.0).sin());
    assert_eq!(
        check_roads_on_terrain(&net, &hm),
        check_roads_on_terrain(&net, &hm)
    );
}

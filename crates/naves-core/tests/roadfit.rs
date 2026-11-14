// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fitting road pieces to a line.

#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]

use std::f64::consts::PI;

use naves_core::catalog::testing::synthetic_catalog;
use naves_core::catalog::{Family, RoadPiece};
use naves_core::roadfit::{EndGoal, FitError, FitPiece, GAP_CLOSED_M, Pose, StretchFit, fit_line};
use proptest::prelude::*;

fn pieces() -> Vec<RoadPiece> {
    synthetic_catalog().pieces(Family::Asf3).to_vec()
}

fn at(x: f64, y: f64, heading: f64) -> Pose {
    Pose { x, y, heading }
}

fn wrap(a: f64) -> f64 {
    let r = a.rem_euclid(2.0 * PI);
    if r > PI { r - 2.0 * PI } else { r }
}

/// Each piece starts where the previous one ends, or, for an overlapping straight, on its axis before
/// its end with the same heading.
fn assert_joined(fit: &StretchFit, start: Pose) {
    let mut prev_end = start;
    for (i, p) in fit.pieces.iter().enumerate() {
        let gap = (p.start.x - prev_end.x).hypot(p.start.y - prev_end.y);
        if gap > 1e-3 {
            // an overlap: behind the previous end, on its line, same direction
            let back = (prev_end.x - p.start.x) * prev_end.heading.sin()
                + (prev_end.y - p.start.y) * prev_end.heading.cos();
            let side = (p.start.x - prev_end.x) * prev_end.heading.cos()
                - (p.start.y - prev_end.y) * prev_end.heading.sin();
            assert!(
                back > 0.0 && back <= 6.25 + 1e-6 && side.abs() < 1e-3,
                "piece {i} starts {gap:.4} m from the previous end (back {back:.3}, side {side:.4})"
            );
            assert!(
                wrap(p.start.heading - prev_end.heading).abs() < 1e-6,
                "piece {i}"
            );
        } else {
            assert!(
                wrap(p.start.heading - prev_end.heading).abs() < 1e-5,
                "piece {i}"
            );
        }
        prev_end = p.end;
    }
}

fn straight_line(length: f64) -> Vec<(f64, f64)> {
    vec![(0.0, 0.0), (0.0, length)]
}

fn fit_straight(length: f64) -> StretchFit {
    let fit = fit_line(
        &straight_line(length),
        at(0.0, 0.0, 0.0),
        EndGoal::Fixed(at(0.0, length, 0.0)),
        &pieces(),
    )
    .unwrap();
    assert_joined(&fit, at(0.0, 0.0, 0.0));
    fit
}

fn is_arc(p: &FitPiece) -> bool {
    p.model.contains(' ')
}

#[test]
fn a_straight_line_uses_straights() {
    let fit = fit_straight(1000.0);
    assert!(fit.pieces.iter().all(|p| !is_arc(p)));
    assert!(fit.gap_m < 1e-6, "{}", fit.gap_m);
    assert!(fit.pieces.len() <= 45, "{} pieces", fit.pieces.len());
    assert!(fit.max_deviation_m < 1e-6);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]
    #[test]
    fn any_length_closes_with_straights(length in 6.25_f64..1500.0) {
        let fit = fit_straight(length);
        prop_assert!(fit.gap_m < 1e-3, "length {length}: gap {}", fit.gap_m);
        prop_assert!(fit.heading_error_deg < 1e-6);
    }
}

fn radius(p: &FitPiece) -> f64 {
    let chord = (p.end.x - p.start.x).hypot(p.end.y - p.start.y);
    let turn = wrap(p.end.heading - p.start.heading).abs();
    chord / (2.0 * (turn / 2.0).sin())
}

#[test]
fn a_circle_uses_the_matching_radius() {
    // a quarter circle of 100 m radius, turning right, centre at (100, 0)
    let path: Vec<(f64, f64)> = (0..=90)
        .map(|d| {
            let a = f64::from(d).to_radians();
            (100.0 - 100.0 * a.cos(), 100.0 * a.sin())
        })
        .collect();
    let fit = fit_line(&path, at(0.0, 0.0, 0.0), EndGoal::Free, &pieces()).unwrap();
    assert_joined(&fit, at(0.0, 0.0, 0.0));
    let arcs: Vec<&FitPiece> = fit.pieces.iter().filter(|p| is_arc(p)).collect();
    assert_ne!(arcs.len(), 0);
    for arc in arcs {
        assert!(radius(arc) >= 49.9, "an arc of radius {:.1}", radius(arc));
    }
    assert!(fit.median_deviation_m < 1.0, "{}", fit.median_deviation_m);
    assert!(fit.gap_m.abs() < 1e-9, "a free end has no gap");
}

#[test]
fn an_s_curve_closes() {
    let smooth = |t: f64| t * t * (3.0 - 2.0 * t);
    let path: Vec<(f64, f64)> = (0..=150)
        .map(|y| (60.0 * smooth(f64::from(y) / 150.0), f64::from(y)))
        .collect();
    let goal = at(60.0, 150.0, 0.0);
    let fit = fit_line(&path, at(0.0, 0.0, 0.0), EndGoal::Fixed(goal), &pieces()).unwrap();
    assert_joined(&fit, at(0.0, 0.0, 0.0));
    assert!(fit.gap_m < GAP_CLOSED_M, "gap {}", fit.gap_m);
    assert!(
        fit.heading_error_deg < 1.0,
        "heading {}",
        fit.heading_error_deg
    );
    let end = fit.pieces.last().unwrap().end;
    assert!((end.x - 60.0).hypot(end.y - 150.0) < GAP_CLOSED_M + 1e-6);
}

#[test]
fn the_free_end_has_no_gap() {
    let path = vec![(0.0, 0.0), (20.0, 90.0), (60.0, 180.0)];
    let fit = fit_line(&path, at(0.0, 0.0, 0.2), EndGoal::Free, &pieces()).unwrap();
    assert!(fit.gap_m.abs() < 1e-12);
    assert!(fit.heading_error_deg.abs() < 1e-12);
    assert_joined(&fit, at(0.0, 0.0, 0.2));
}

#[test]
fn a_hairpin_tighter_than_the_tightest_arc_does_not_loop() {
    // 90 m up, a 180 degree turn of radius 5 m, 90 m back down: 200 m of road
    let mut path: Vec<(f64, f64)> = vec![(0.0, 0.0), (0.0, 90.0)];
    for d in 1..180 {
        let a = f64::from(d).to_radians();
        path.push((5.0 - 5.0 * a.cos(), 90.0 + 5.0 * a.sin()));
    }
    path.push((10.0, 90.0));
    path.push((10.0, 0.0));
    let fit = fit_line(&path, at(0.0, 0.0, 0.0), EndGoal::Free, &pieces()).unwrap();
    assert!(fit.pieces.len() <= 40, "{} pieces", fit.pieces.len());
    assert!(fit.max_deviation_m > 0.0);
    assert_joined(&fit, at(0.0, 0.0, 0.0));
}

#[test]
fn progress_does_not_jump_across_a_hairpin() {
    // two parallel legs 15 m apart
    let path = vec![(0.0, 0.0), (0.0, 100.0), (15.0, 100.0), (15.0, 0.0)];
    let fit = fit_line(&path, at(0.0, 0.0, 0.0), EndGoal::Free, &pieces()).unwrap();
    // the fit follows the first leg for 75 m before it turns (the U-turn is tighter than any piece, so it
    // cuts the corner), and does not jump across to the other leg on the way
    let turn = fit
        .pieces
        .iter()
        .position(|p| p.end.y >= 75.0)
        .expect("never got along the first leg");
    for p in &fit.pieces[..turn] {
        assert!(p.end.x.abs() < 4.0, "left the first leg early: {:?}", p.end);
    }
}

#[test]
fn the_same_input_gives_the_same_pieces() {
    let path = vec![(0.0, 0.0), (30.0, 100.0), (80.0, 160.0), (150.0, 180.0)];
    let a = fit_line(&path, at(0.0, 0.0, 0.3), EndGoal::Free, &pieces()).unwrap();
    let b = fit_line(&path, at(0.0, 0.0, 0.3), EndGoal::Free, &pieces()).unwrap();
    assert_eq!(a, b);
}

#[test]
fn input_that_cannot_be_fitted_is_an_error() {
    let p = pieces();
    assert!(matches!(
        fit_line(&[(0.0, 0.0)], at(0.0, 0.0, 0.0), EndGoal::Free, &p),
        Err(FitError::TooFewPoints)
    ));
    assert!(matches!(
        fit_line(
            &[(0.0, 0.0), (f64::NAN, 1.0)],
            at(0.0, 0.0, 0.0),
            EndGoal::Free,
            &p
        ),
        Err(FitError::NotFinite)
    ));
    assert!(matches!(
        fit_line(
            &[(0.0, 0.0), (0.0, 10.0)],
            at(0.0, 0.0, 0.0),
            EndGoal::Free,
            &[]
        ),
        Err(FitError::NoPieces)
    ));
}

/// The stretches of the village fixture, at both map scales, fitted with the pieces of their family.
#[test]
fn closure_rate_on_the_stretches_of_the_fixture() {
    use naves_core::features::road_features;
    use naves_core::heightmap::MapGeo;
    use naves_core::osm::read_ways;
    use naves_core::project::Project;
    use naves_core::roadnet::build_graph;

    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/osm/schoenau.osm.pbf");
    let catalog = synthetic_catalog();
    for scale in [1.0, 0.5] {
        let mut project = Project::new("Fit", 47.601, 12.985, 900.0 / scale);
        project.scale.horizontal = scale;
        let frame = MapGeo::new(&project).unwrap();
        let ways = read_ways(std::slice::from_ref(&fixture), frame.window(100.0).unwrap()).unwrap();
        let size = project.map_size_m();
        let graph = build_graph(&road_features(&ways, &frame, size));
        let mut gaps = Vec::new();
        let mut deviations = Vec::new();
        for stretch in &graph.stretches {
            let (Some(p0), Some(p1)) = (stretch.points.first(), stretch.points.get(1)) else {
                continue;
            };
            let count = stretch.points.len();
            let (Some(q0), Some(q1)) = (stretch.points.get(count - 2), stretch.points.last())
            else {
                continue;
            };
            let length: f64 = stretch
                .points
                .windows(2)
                .map(|p| (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1))
                .sum();
            if length < 40.0 {
                continue;
            }
            let start = at(p0.0, p0.1, (p1.0 - p0.0).atan2(p1.1 - p0.1));
            let goal = at(q1.0, q1.1, (q1.0 - q0.0).atan2(q1.1 - q0.1));
            let fit = fit_line(
                &stretch.points,
                start,
                EndGoal::Fixed(goal),
                catalog.pieces(stretch.family),
            )
            .unwrap();
            gaps.push(fit.gap_m);
            deviations.push(fit.median_deviation_m);
        }
        assert!(
            gaps.len() >= 10,
            "scale {scale}: only {} stretches",
            gaps.len()
        );
        let closed = gaps
            .iter()
            .filter(|g| **g < naves_core::roadfit::GAP_TOLERATED_M)
            .count();
        deviations.sort_by(f64::total_cmp);
        let median = deviations[deviations.len() / 2];
        #[expect(clippy::cast_precision_loss, reason = "small counts")]
        let rate = closed as f64 / gaps.len() as f64;
        assert!(
            rate >= 0.9,
            "scale {scale}: only {:.0} % closed",
            rate * 100.0
        );
        assert!(median < 6.0, "scale {scale}: median deviation {median}");
    }
}

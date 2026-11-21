// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tuning round 1 of the smoothing stage: collinear points are dropped, a zone that cannot be widened goes back to
//! the original line, a widened line does not enter water, and the approach of a bridge is held.

#![expect(
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test code; failing loudly is the point"
)]

use naves_core::catalog::Family;
use naves_core::roadnet::{Crossing, RoadGraph, Stretch};
use naves_core::smooth::{
    SAMPLE_M, SIMPLIFY_TOL_M, SmoothParams, SmoothReport, WIDEN_BRIDGE_HOLD_M,
    WIDEN_REVERT_BLEND_M, radius_at, resample, simplify_line, smooth_graph, smooth_graph_fit_only,
    smooth_graph_in_water,
};

type P = (f64, f64);

fn stretch(id: &str, start: i64, end: i64, points: &[P]) -> Stretch {
    Stretch {
        id: id.to_string(),
        family: Family::Asf3,
        drivable: true,
        bridge: false,
        tunnel: false,
        roundabout: false,
        layer: 0,
        points: points.to_vec(),
        start,
        end,
        bridge_kind: None,
        bridge_offset: 0.0,
        use_sidepath: false,
    }
}

fn graph(stretches: Vec<Stretch>) -> RoadGraph {
    RoadGraph {
        stretches,
        crossings: Vec::new(),
    }
}

fn ten(_: &Stretch) -> f64 {
    10.0
}

fn bits(p: P) -> (u64, u64) {
    (p.0.to_bits(), p.1.to_bits())
}

/// Every sample kept (the stage as it was before the drop).
fn keep_all() -> SmoothParams {
    SmoothParams {
        simplify: 0.0,
        ..SmoothParams::default()
    }
}

fn dist_to_segment(p: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let along = if len2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - a.0 - along * dx).hypot(p.1 - a.1 - along * dy)
}

fn dist_to_line(p: P, line: &[P]) -> f64 {
    line.windows(2)
        .map(|w| dist_to_segment(p, w[0], w[1]))
        .fold(f64::INFINITY, f64::min)
}

fn hairpin_of(gap: f64) -> Vec<P> {
    let r = gap / 2.0;
    let mut points = vec![(0.0, 0.0), (80.0, 0.0)];
    for k in 1..18 {
        let a = (-90.0 + 10.0 * f64::from(k)).to_radians();
        points.push((80.0 + r * a.cos(), r + r * a.sin()));
    }
    points.push((80.0, gap));
    points.push((0.0, gap));
    points
}

/// The hairpin as two stretches that meet at arc sample `at`, with a third that leaves that node.
fn split_hairpin(gap: f64, at: usize) -> RoadGraph {
    let points = hairpin_of(gap);
    let node = points[at];
    RoadGraph {
        stretches: vec![
            stretch("a", 1, 2, &points[..=at]),
            stretch("b", 2, 3, &points[at..]),
            stretch("c", 2, 4, &[node, (node.0 + 40.0, node.1)]),
        ],
        crossings: vec![Crossing {
            node: 2,
            point: node,
            stretches: vec![0, 1, 2],
        }],
    }
}

#[test]
fn collinear_samples_are_dropped() {
    // a straight road drawn with a point every 5 m
    let line: Vec<P> = (0..=20).map(|i| (f64::from(i) * 5.0, 3.0)).collect();
    let g = graph(vec![stretch("s", 1, 2, &line)]);
    let (out, _) = smooth_graph(&g, &ten, &SmoothParams::default());
    assert_eq!(
        out.stretches[0].points.len(),
        2,
        "{:?}",
        out.stretches[0].points
    );
    assert_eq!(bits(out.stretches[0].points[0]), bits(line[0]));
    assert_eq!(bits(out.stretches[0].points[1]), bits(line[20]));
}

#[test]
fn a_curve_keeps_the_points_that_hold_its_shape() {
    // a quarter circle of 30 m radius, between two straights
    let radius = 30.0_f64;
    let mut points = vec![(-40.0, 0.0), (0.0, 0.0)];
    for k in 1..=30 {
        let a = f64::from(k) * 3.0_f64.to_radians();
        points.push((radius * a.sin(), radius * (1.0 - a.cos())));
    }
    points.push((radius, radius + 40.0));
    let g = graph(vec![stretch("c", 1, 2, &points)]);
    let (all, _) = smooth_graph(&g, &ten, &keep_all());
    let (out, _) = smooth_graph(&g, &ten, &SmoothParams::default());
    let (all, dropped) = (&all.stretches[0].points, &out.stretches[0].points);
    assert!(dropped.len() < all.len(), "nothing was dropped");
    assert!(
        dropped.len() > 8,
        "the curve lost its shape: {}",
        dropped.len()
    );
    // the line has moved nowhere by more than the tolerance
    for p in all {
        assert!(
            dist_to_line(*p, dropped) <= SIMPLIFY_TOL_M + 1e-9,
            "{p:?} is {} m off",
            dist_to_line(*p, dropped)
        );
    }
    // the ends stay
    assert_eq!(bits(dropped[0]), bits(all[0]));
    assert_eq!(bits(*dropped.last().unwrap()), bits(*all.last().unwrap()));
}

#[test]
fn a_kink_is_not_dropped() {
    let line = [(0.0, 0.0), (50.0, 0.0), (50.0, 0.2), (100.0, 0.2)];
    let out = simplify_line(&line, SIMPLIFY_TOL_M);
    assert_eq!(out.len(), 4);
    let flat = simplify_line(&[(0.0, 0.0), (50.0, 0.01), (100.0, 0.0)], SIMPLIFY_TOL_M);
    assert_eq!(flat.len(), 2);
    // nothing to do for a tolerance of zero or a line that is not finite
    let nan = [(0.0, 0.0), (f64::NAN, 1.0), (2.0, 2.0)];
    assert_eq!(simplify_line(&nan, SIMPLIFY_TOL_M).len(), 3);
    assert_eq!(simplify_line(&line, 0.0).len(), 4);
}

#[test]
fn the_figures_are_computed_before_the_drop() {
    let g = split_hairpin(14.0, 10);
    let (_, all): (_, SmoothReport) = smooth_graph(&g, &ten, &keep_all());
    let (out, dropped) = smooth_graph(&g, &ten, &SmoothParams::default());
    assert_eq!(all.zones, dropped.zones);
    assert_eq!(all.per_stretch, dropped.per_stretch);
    assert!((all.tight_before_m - dropped.tight_before_m).abs() < 1e-12);
    assert!((all.tight_after_m - dropped.tight_after_m).abs() < 1e-12);
    assert!((all.max_move_m - dropped.max_move_m).abs() < 1e-12);
    // the counts are in the report
    assert_eq!(dropped.points_fitted, all.points_fitted);
    assert!(dropped.points_after < dropped.points_fitted, "{dropped:?}");
    let kept: usize = out.stretches.iter().map(|s| s.points.len()).sum();
    assert_eq!(kept, dropped.points_after);
}

#[test]
fn a_zone_that_cannot_be_widened_goes_back_to_the_original_line() {
    // the junction sits on the arc: the legs have no room to spread (see the widening tests)
    let g = split_hairpin(8.0, 3);
    let (out, report) = smooth_graph(&g, &ten, &keep_all());
    let (fit, _) = smooth_graph_fit_only(&g, &ten, &keep_all());
    assert!(
        report.zones.iter().all(|z| z.plate_candidate),
        "{:?}",
        report.zones
    );
    // the original, resampled: the stretches' own lines
    let original: Vec<P> = hairpin_of(8.0)[3..].to_vec();
    let resampled = resample(&original, SAMPLE_M);
    let b = &out.stretches[1].points;
    let fit_b = &fit.stretches[1].points;
    assert_eq!(b.len(), resampled.len());
    // the zone is where the line is not the fit's bitwise, plus the blend at its two ends
    let changed: Vec<usize> = (0..b.len())
        .filter(|&k| bits(b[k]) != bits(fit_b[k]))
        .collect();
    let (lo, hi) = (*changed.first().unwrap(), *changed.last().unwrap());
    let blend = (WIDEN_REVERT_BLEND_M / SAMPLE_M).ceil() as usize + 1;
    assert!(hi - lo > 2 * blend + 4, "zone {lo}..{hi}");
    for k in lo + blend..=hi - blend {
        assert!(
            dist_to_line(b[k], &original) < 1e-9,
            "sample {k} is {} m off the original",
            dist_to_line(b[k], &original)
        );
        assert!(
            radius_at(b[k - 1], b[k], b[k + 1])
                >= radius_at(resampled[k - 1], resampled[k], resampled[k + 1]) - 1e-9
        );
    }
    // no step, anywhere
    for s in &out.stretches {
        for w in s.points.windows(2) {
            let step = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
            assert!(step <= SAMPLE_M * 1.5 + 1e-9, "a step of {step} m");
        }
    }
    // and the fit pulled the bend tighter than the original, which is what the revert undoes
    let worst = |line: &[P]| {
        line.windows(3)
            .map(|w| radius_at(w[0], w[1], w[2]))
            .fold(f64::INFINITY, f64::min)
    };
    assert!(
        worst(b) >= worst(fit_b) - 1e-9,
        "{} against {}",
        worst(b),
        worst(fit_b)
    );
}

#[test]
fn a_widened_line_that_enters_water_goes_back() {
    // a hairpin with room that is widened outward (see the widening tests); water lies beyond the original arc
    let points = hairpin_of(8.0);
    let g = graph(vec![stretch("h", 1, 2, &points)]);
    let (dry, dry_report) = smooth_graph(&g, &ten, &keep_all());
    assert!(
        dry_report
            .zones
            .iter()
            .all(|z| z.reached && !z.blocked_by_water)
    );
    // widened, the legs spread: the lower one lies below y = 0
    let lowest = dry.stretches[0]
        .points
        .iter()
        .map(|p| p.1)
        .fold(f64::MAX, f64::min);
    assert!(lowest < -0.5, "the widened lower leg is at y = {lowest}");
    let water = |p: P| p.1 < -0.5;
    let (out, report) = smooth_graph_in_water(&g, &ten, &water, &keep_all());
    assert!(
        report.zones.iter().any(|z| z.blocked_by_water),
        "{:?}",
        report.zones
    );
    for z in report.zones.iter().filter(|z| z.blocked_by_water) {
        assert!(!z.plate_candidate, "{z:?}");
        assert!(!z.reached, "{z:?}");
        assert!(z.max_outward_m.abs() < 1e-9, "{z:?}");
    }
    // nothing of the result is in water, and the line is the original where the zone is
    assert!(out.stretches[0].points.iter().all(|p| !water(*p)));
    let lowest = out.stretches[0]
        .points
        .iter()
        .map(|p| p.1)
        .fold(f64::MAX, f64::min);
    assert!(lowest > -0.5, "{lowest}");
    // where the original line is in water already, a sample is not blocked for it
    let wet = |p: P| p.1 < 0.5;
    let (_, wet_report) = smooth_graph_in_water(&g, &ten, &wet, &keep_all());
    assert!(
        wet_report
            .zones
            .iter()
            .all(|z| !z.blocked_by_water || z.max_outward_m.abs() < 1e-9)
    );
}

#[test]
fn the_approach_of_a_bridge_is_held() {
    // a hairpin with room whose zone reaches to within 10 m of the end of a bridge on the return leg: the samples
    // within WIDEN_BRIDGE_HOLD_M of the bridge are what the fit gives
    let mut points = hairpin_of(14.0);
    points.pop();
    points.push((30.0, 14.0));
    let n1 = (30.0, 14.0);
    let n2 = (-10.0, 14.0);
    let mut deck = stretch("br", 2, 3, &[n1, n2]);
    deck.bridge = true;
    let g = graph(vec![
        stretch("p1", 1, 2, &points),
        deck,
        stretch("p2", 3, 4, &[n2, (-50.0, 14.0)]),
    ]);
    let (out, report) = smooth_graph(&g, &ten, &keep_all());
    let (fit, _) = smooth_graph_fit_only(&g, &ten, &keep_all());
    assert_eq!(report.zones.len(), 1, "{:?}", report.zones);
    assert!(
        report.zones[0].reached && !report.zones[0].plate_candidate,
        "{:?}",
        report.zones[0]
    );
    assert!(report.zones[0].max_outward_m > 1.0);
    // the widening resamples the samples it may move, so the held ones are looked for, not matched by index
    let mut held = 0;
    for fit_point in &fit.stretches[0].points {
        let along = (fit_point.0 - n1.0).abs();
        if along <= WIDEN_BRIDGE_HOLD_M && fit_point.1 > 7.0 {
            held += 1;
            assert!(
                out.stretches[0]
                    .points
                    .iter()
                    .any(|p| bits(*p) == bits(*fit_point)),
                "{fit_point:?} moved by the widening"
            );
        }
    }
    // and no sample between them is on a widened line: they all lie on the fit's return leg
    for p in &out.stretches[0].points {
        if (p.0 - n1.0).abs() <= WIDEN_BRIDGE_HOLD_M && p.1 > 7.0 {
            assert!(
                fit.stretches[0].points.iter().any(|q| bits(*q) == bits(*p)),
                "{p:?} is not a sample of the fit"
            );
        }
    }
    assert!(held > 5, "only {held} samples near the bridge");
    // the zone itself did widen further out
    assert!(out.stretches[0].points.iter().any(|a| a.1 < -0.5));
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The curve fit for one line: `smooth_line`, `resample` and `radius_at`.

#![expect(
    clippy::indexing_slicing,
    reason = "test code; failing loudly is the point"
)]

use naves_core::smooth::{CORRIDOR_M, Sample, SmoothParams, radius_at, resample, smooth_line};

type P = (f64, f64);

fn samples(points: &[P], fixed: &[usize]) -> Vec<Sample> {
    points
        .iter()
        .enumerate()
        .map(|(i, &at)| Sample {
            at,
            fixed: fixed.contains(&i),
            stiffness: 1.0,
        })
        .collect()
}

fn dist(a: P, b: P) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// The signed turn at every interior point, in degrees (left positive).
fn turns(line: &[P]) -> Vec<f64> {
    line.windows(3)
        .map(|w| {
            let a = (w[1].0 - w[0].0, w[1].1 - w[0].1);
            let b = (w[2].0 - w[1].0, w[2].1 - w[1].1);
            (a.0 * b.1 - a.1 * b.0)
                .atan2(a.0 * b.0 + a.1 * b.1)
                .to_degrees()
        })
        .collect()
}

fn an_l() -> Vec<P> {
    resample(&[(0.0, 0.0), (40.0, 0.0), (40.0, 40.0)], 2.0)
}

fn within_corridor(input: &[P], output: &[P]) {
    assert_eq!(input.len(), output.len());
    for (i, (a, b)) in input.iter().zip(output).enumerate() {
        assert!(
            dist(*a, *b) <= CORRIDOR_M + 1e-6,
            "sample {i} moved {}",
            dist(*a, *b)
        );
    }
}

#[test]
fn a_straight_line_stays_where_it_is() {
    let line: Vec<P> = (0..50).map(|i| (f64::from(i) * 2.0, 3.0)).collect();
    let out = smooth_line(&samples(&line, &[]), &SmoothParams::default()).unwrap();
    for (a, b) in line.iter().zip(&out) {
        assert!(dist(*a, *b) < 1e-6, "{a:?} -> {b:?}");
    }
}

#[test]
fn a_corner_is_rounded_inside_the_corridor() {
    let line = an_l();
    let last = line.len() - 1;
    let out = smooth_line(&samples(&line, &[0, last]), &SmoothParams::default()).unwrap();
    within_corridor(&line, &out);
    // A fillet of a 90 degree corner inside a 2 m corridor has a radius of at most 2 / (sqrt 2 - 1) = 4.8 m,
    // which is about 24 degrees per 2 m sample; W_STAY pulls it a little below that.
    let worst = turns(&out).iter().fold(0.0_f64, |m, t| m.max(t.abs()));
    assert!(worst < 30.0, "largest turn {worst}");
    let corner = line.iter().position(|p| *p == (40.0, 0.0)).unwrap();
    let r = radius_at(out[corner - 1], out[corner], out[corner + 1]);
    assert!(r >= 3.5, "radius {r}");
}

#[test]
fn fixed_samples_do_not_move() {
    let line = an_l();
    let last = line.len() - 1;
    let out = smooth_line(&samples(&line, &[0, 1, last]), &SmoothParams::default()).unwrap();
    for i in [0, 1, last] {
        assert_eq!(out[i].0.to_bits(), line[i].0.to_bits());
        assert_eq!(out[i].1.to_bits(), line[i].1.to_bits());
    }
    // and the rest still moved
    assert!(dist(out[line.len() / 2], line[line.len() / 2]) > 0.1);
}

#[test]
fn a_curve_drawn_as_short_segments_becomes_even() {
    let radius = 30.0_f64;
    let total = radius * std::f64::consts::FRAC_PI_2;
    let at = |s: f64| {
        let a = s / radius;
        (radius * a.sin(), radius * (1.0 - a.cos()))
    };
    let mut points = vec![at(0.0)];
    let mut s = 0.0;
    for step in [2.0, 9.0, 3.0, 11.0, 4.0].iter().cycle() {
        s += step;
        if s >= total {
            break;
        }
        points.push(at(s));
    }
    points.push(at(total));
    let line = resample(&points, 2.0);
    let last = line.len() - 1;
    let out = smooth_line(&samples(&line, &[0, last]), &SmoothParams::default()).unwrap();
    let unevenness = |l: &[P]| {
        let t = turns(l);
        t.windows(2)
            .map(|w| (w[0] - w[1]).abs())
            .fold(0.0_f64, f64::max)
    };
    let before = unevenness(&line);
    let after = unevenness(&out);
    assert!(after <= before / 2.0, "unevenness {before} -> {after}");
}

#[test]
fn a_switchback_stays_a_switchback() {
    let mut points: Vec<P> = vec![(0.0, 0.0), (60.0, 0.0)];
    for k in 1..24 {
        let a = std::f64::consts::PI * f64::from(k) / 24.0;
        points.push((60.0 + 4.0 * a.sin(), 4.0 - 4.0 * a.cos()));
    }
    points.push((60.0, 8.0));
    points.push((0.0, 8.0));
    let line = resample(&points, 2.0);
    let last = line.len() - 1;
    let out = smooth_line(&samples(&line, &[0, last]), &SmoothParams::default()).unwrap();
    within_corridor(&line, &out);
    let total: f64 = turns(&out).iter().sum();
    assert!((total - 180.0).abs() < 5.0, "summed turn {total}");
    let lower: Vec<P> = out
        .iter()
        .copied()
        .filter(|p| p.0 < 55.0 && p.1 < 4.0)
        .collect();
    let upper: Vec<P> = out
        .iter()
        .copied()
        .filter(|p| p.0 < 55.0 && p.1 >= 4.0)
        .collect();
    assert!(!lower.is_empty() && !upper.is_empty());
    for a in &lower {
        for b in &upper {
            assert!(dist(*a, *b) >= 4.0 - 1e-6, "{a:?} {b:?}");
        }
    }
}

#[test]
fn stiff_samples_stay_straighter() {
    // an S: a left arc then a right arc of radius 20 m
    let r = 20.0_f64;
    let mut points: Vec<P> = Vec::new();
    for k in 0..=20 {
        let a = std::f64::consts::FRAC_PI_3 * f64::from(k) / 20.0;
        points.push((r * a.sin(), r * (1.0 - a.cos())));
    }
    let (ex, ey) = *points.last().unwrap();
    let a0 = std::f64::consts::FRAC_PI_3;
    for k in 1..=20 {
        let a = std::f64::consts::FRAC_PI_3 * f64::from(k) / 20.0;
        points.push((
            ex + r * (a0.sin() - (a0 - a).sin()),
            ey + r * ((a0 - a).cos() - a0.cos()),
        ));
    }
    let line = resample(&points, 2.0);
    let n = line.len();
    let last = n - 1;
    let (from, to) = (n / 3, 2 * n / 3);
    let middle_turn = |stiffness: f64| {
        let mut s = samples(&line, &[0, last]);
        for sample in &mut s[from..to] {
            sample.stiffness = stiffness;
        }
        let out = smooth_line(&s, &SmoothParams::default()).unwrap();
        // turns() entry k belongs to point k + 1
        turns(&out)[from - 1..to - 1]
            .iter()
            .map(|t| t.abs())
            .sum::<f64>()
    };
    let soft = middle_turn(1.0);
    let stiff = middle_turn(10.0);
    assert!(stiff < soft, "stiff {stiff} soft {soft}");
}

#[test]
fn degenerate_input_is_an_error_not_a_panic() {
    let p = SmoothParams::default();
    for n in 0..3 {
        let line: Vec<P> = (0..n).map(|i| (f64::from(i), 0.0)).collect();
        assert!(
            smooth_line(&samples(&line, &[]), &p).is_err(),
            "{n} samples"
        );
    }
    let mut line: Vec<P> = (0..10).map(|i| (f64::from(i), 0.0)).collect();
    line[4].0 = f64::NAN;
    assert!(smooth_line(&samples(&line, &[]), &p).is_err());
    let same = vec![(1.0, 1.0); 10];
    assert!(smooth_line(&samples(&same, &[]), &p).is_err());
    let line: Vec<P> = (0..10).map(|i| (f64::from(i), 0.0)).collect();
    let mut s = samples(&line, &[]);
    s[3].stiffness = f64::INFINITY;
    assert!(smooth_line(&s, &p).is_err());
    let bad = SmoothParams {
        corridor: -1.0,
        stay: 0.02,
    };
    assert!(smooth_line(&samples(&line, &[]), &bad).is_err());
}

#[test]
fn resample_keeps_the_ends_and_spaces_evenly() {
    let r = resample(&[(0.0, 0.0), (10.5, 0.0)], 2.0);
    assert_eq!(r.len(), 6);
    assert_eq!(r[0], (0.0, 0.0));
    assert_eq!(r[5], (10.5, 0.0));
    for w in r.windows(2) {
        assert!((dist(w[0], w[1]) - 2.1).abs() < 1e-9);
    }
    assert_eq!(
        resample(&[(0.0, 0.0), (1.0, 0.0)], 2.0),
        vec![(0.0, 0.0), (1.0, 0.0)]
    );
    let r = resample(
        &[(0.0, 0.0), (0.0, 0.0), (4.0, 0.0), (4.0, 0.0), (4.0, 4.0)],
        2.0,
    );
    assert_eq!(r.len(), 5);
    assert_eq!(r[0], (0.0, 0.0));
    assert_eq!(r[4], (4.0, 4.0));
    assert_eq!(resample(&[], 2.0), Vec::<(f64, f64)>::new());
    assert_eq!(resample(&[(1.0, 1.0)], 2.0), vec![(1.0, 1.0)]);
}

#[test]
fn the_radius_of_a_circle_and_of_a_line() {
    let r = radius_at((-1.0, 0.0), (0.0, 1.0), (1.0, 0.0));
    assert!((r - 1.0).abs() < 1e-9);
    assert!(radius_at((0.0, 0.0), (1.0, 0.0), (2.0, 0.0)).is_infinite());
    assert!(radius_at((0.0, 0.0), (0.0, 0.0), (2.0, 0.0)).is_infinite());
}

#[test]
fn the_same_input_gives_the_same_bytes() {
    let line = an_l();
    let last = line.len() - 1;
    let s = samples(&line, &[0, last]);
    let a = smooth_line(&s, &SmoothParams::default()).unwrap();
    let b = smooth_line(&s, &SmoothParams::default()).unwrap();
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x.0.to_bits(), y.0.to_bits());
        assert_eq!(x.1.to_bits(), y.1.to_bits());
    }
}

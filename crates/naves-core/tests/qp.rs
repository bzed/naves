// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The sparse quadratic program solver behind `qp::solve`.

use naves_core::qp::{Problem, QpError, Row, solve};

fn row(terms: &[(usize, f64)], lo: f64, hi: f64) -> Row {
    Row {
        terms: terms.to_vec(),
        lo,
        hi,
    }
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() < tol
}

#[test]
fn a_least_squares_problem_without_rows() {
    let mut p = Problem::new(2);
    p.add_square(1.0, &[(0, 1.0)], -3.0);
    p.add_square(2.0, &[(1, 1.0)], 4.0);
    let x = solve(&p).unwrap();
    assert_eq!(x.len(), 2);
    assert!(close(x[0], 3.0, 1e-6), "{x:?}");
    assert!(close(x[1], -4.0, 1e-6), "{x:?}");
    assert!(p.cost(&x).abs() < 1e-9, "{}", p.cost(&x));
}

#[test]
fn a_bound_cuts_the_minimum() {
    let mut p = Problem::new(1);
    p.add_square(1.0, &[(0, 1.0)], -3.0);
    p.rows.push(row(&[(0, 1.0)], f64::NEG_INFINITY, 1.0));
    let x = solve(&p).unwrap();
    assert!(close(x[0], 1.0, 1e-6), "{x:?}");
    assert!(close(p.cost(&x), 4.0, 1e-5), "{}", p.cost(&x));
}

/// `(x − 3)²` plus an exact-penalty slack `s` on `x − s ≤ hi`, cost `1e4·s + 1e4·s²`.
fn slack_problem(hi: f64) -> Problem {
    let mut p = Problem::new(2);
    p.add_square(1.0, &[(0, 1.0)], -3.0);
    p.add_linear(1e4, 1);
    p.add_square(1e4, &[(1, 1.0)], 0.0);
    p.rows
        .push(row(&[(0, 1.0), (1, -1.0)], f64::NEG_INFINITY, hi));
    p.rows.push(row(&[(1, 1.0)], 0.0, f64::INFINITY));
    p
}

#[test]
fn an_exact_penalty_slack_is_zero_when_the_row_can_be_kept() {
    let x = solve(&slack_problem(5.0)).unwrap();
    assert!(x[1] < 1e-6, "{x:?}");
    assert!(close(x[0], 3.0, 1e-6), "{x:?}");

    let mut p = slack_problem(1.0);
    p.rows.push(row(&[(0, 1.0)], 2.0, f64::INFINITY));
    let x = solve(&p).unwrap();
    assert!(close(x[1], 1.0, 1e-5), "{x:?}");
    assert!(close(x[0], 2.0, 1e-5), "{x:?}");
}

#[test]
fn an_equality_row_is_kept() {
    let mut p = Problem::new(2);
    p.add_square(1.0, &[(0, 1.0)], 0.0);
    p.add_square(1.0, &[(1, 1.0)], 0.0);
    p.rows.push(row(&[(0, 1.0), (1, 1.0)], 2.0, 2.0));
    let x = solve(&p).unwrap();
    assert!(close(x[0], 1.0, 1e-6), "{x:?}");
    assert!(close(x[1], 1.0, 1e-6), "{x:?}");
}

#[test]
fn contradicting_rows_without_slack_are_not_solved() {
    let mut p = Problem::new(1);
    p.add_square(1.0, &[(0, 1.0)], 0.0);
    p.rows.push(row(&[(0, 1.0)], f64::NEG_INFINITY, 0.0));
    p.rows.push(row(&[(0, 1.0)], 1.0, f64::INFINITY));
    let err = solve(&p).unwrap_err();
    assert!(matches!(err, QpError::NotSolved { .. }), "{err:?}");
}

#[test]
fn invalid_input_is_an_error() {
    let invalid = |p: &Problem| matches!(solve(p), Err(QpError::Invalid { .. }));

    let mut p = Problem::new(1);
    p.add_square(1.0, &[(1, 1.0)], 0.0);
    assert!(invalid(&p), "square term out of range");

    let mut p = Problem::new(1);
    p.add_linear(1.0, 3);
    assert!(invalid(&p), "linear term out of range");

    let mut p = Problem::new(1);
    p.rows.push(row(&[(2, 1.0)], 0.0, 1.0));
    assert!(invalid(&p), "row term out of range");

    let mut p = Problem::new(1);
    p.add_square(1.0, &[(0, f64::NAN)], 0.0);
    assert!(invalid(&p), "NaN coefficient");

    let mut p = Problem::new(1);
    p.add_square(1.0, &[(0, 1.0)], f64::INFINITY);
    assert!(invalid(&p), "infinite constant");

    let mut p = Problem::new(1);
    p.rows.push(row(&[(0, f64::NAN)], 0.0, 1.0));
    assert!(invalid(&p), "NaN row coefficient");

    let mut p = Problem::new(1);
    p.rows.push(row(&[(0, 1.0)], 2.0, 1.0));
    assert!(invalid(&p), "lo > hi");

    let mut p = Problem::new(1);
    p.rows.push(row(&[(0, 1.0)], f64::NAN, 1.0));
    assert!(invalid(&p), "NaN bound");

    let mut p = Problem::new(1);
    p.rows.push(row(&[(0, 1.0)], f64::INFINITY, f64::INFINITY));
    assert!(invalid(&p), "lo = +inf");

    let mut p = Problem::new(1);
    p.add_square(-1.0, &[(0, 1.0)], 0.0);
    assert!(invalid(&p), "negative weight");

    let mut p = Problem::new(1);
    p.add_linear(f64::NAN, 0);
    assert!(invalid(&p), "NaN linear weight");

    let mut p = Problem::new(1);
    p.add_square(1e300, &[(0, 1e300)], 0.0);
    assert!(invalid(&p), "the cost overflows");
}

/// A chain of `n` heights pulled to a wavy ground, with a second-difference cost and grade rows
/// `|x_{i+1} − x_i| ≤ grade`.
fn chain(n: usize, grade: f64) -> Problem {
    let mut p = Problem::new(n);
    for i in 0..n {
        #[expect(clippy::cast_precision_loss, reason = "small test index")]
        let t = i as f64;
        let ground = 5.0 * (t / 37.0).sin() + 0.3 * (t / 3.0).cos();
        p.add_square(1.0, &[(i, 1.0)], -ground);
    }
    for i in 1..n - 1 {
        p.add_square(30.0, &[(i - 1, 1.0), (i, -2.0), (i + 1, 1.0)], 0.0);
    }
    for i in 0..n - 1 {
        p.rows.push(row(&[(i + 1, 1.0), (i, -1.0)], -grade, grade));
    }
    p
}

#[test]
fn two_solves_are_bit_identical() {
    let p = chain(2000, 0.05);
    let a = solve(&p).unwrap();
    let b = solve(&p).unwrap();
    assert_eq!(a.len(), 2000);
    let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&a), bits(&b));
    for w in a.windows(2) {
        assert!((w[1] - w[0]).abs() <= 0.05 + 1e-6, "{w:?}");
    }
}

#[test]
fn no_unknowns() {
    assert_eq!(solve(&Problem::new(0)).unwrap(), Vec::<f64>::new());
}

#[test]
fn an_unbounded_problem_is_not_solved() {
    let mut p = Problem::new(1);
    p.add_linear(1.0, 0);
    let err = solve(&p).unwrap_err();
    assert!(matches!(err, QpError::NotSolved { .. }), "{err:?}");
    assert!(err.to_string().contains("not solved"), "{err}");
}

#[test]
fn the_cost_adds_squares_and_linear_terms() {
    let mut p = Problem::new(2);
    p.add_square(2.0, &[(0, 1.0), (1, -1.0)], 1.0);
    p.add_linear(3.0, 1);
    // 2·(1 − 2 + 1)² + 3·2 = 6; 2·(0 − 0 + 1)² + 0 = 2
    assert!(close(p.cost(&[1.0, 2.0]), 6.0, 1e-12));
    assert!(close(p.cost(&[0.0, 0.0]), 2.0, 1e-12));
}

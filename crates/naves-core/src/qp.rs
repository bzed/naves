// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A sparse convex quadratic program behind one function.
//!
//! The problem is `min ½ xᵀP x + qᵀx + constant` subject to `lo ≤ A x ≤ hi`, with `P` positive
//! semidefinite and both `P` and `A` sparse. The cost is built from weighted squares of affine
//! expressions ([`Problem::add_square`]) and weighted linear terms ([`Problem::add_linear`]), so `P` is
//! positive semidefinite by construction. [`solve`] checks the input first and hands the problem to the
//! interior point solver of the `clarabel` crate (`QDLDL` factorisation, one thread, no time limit, a fixed
//! iteration limit), so the same problem always gives the same result, bit for bit.

use std::collections::BTreeMap;

use clarabel::algebra::CscMatrix;
use clarabel::solver::{
    DefaultSettingsBuilder, DefaultSolver, IPSolver, SolverStatus, SupportedConeT,
};

/// The tolerance of the solver: absolute and relative, on the duality gap and on feasibility.
const TOLERANCE: f64 = 1e-7;
/// The iteration limit of the solver.
const MAX_ITER: u32 = 200;
/// The largest finite bound a row may have: the solver silently drops bounds from `1e20` on, so a bound
/// this large is rejected rather than lost.
const MAX_BOUND: f64 = 1e15;

/// One linear row `lo ≤ Σ c_k x_k ≤ hi`.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The terms `(unknown, coefficient)`; an unknown may appear more than once, its coefficients add up.
    pub terms: Vec<(usize, f64)>,
    /// The lower bound, `f64::NEG_INFINITY` for a row without one.
    pub lo: f64,
    /// The upper bound, `f64::INFINITY` for a row without one.
    pub hi: f64,
}

/// A weighted square `weight · (Σ c_k x_k + constant)²` of the cost.
#[derive(Debug, Clone, PartialEq)]
struct Square {
    weight: f64,
    terms: Vec<(usize, f64)>,
    constant: f64,
}

/// A quadratic program: the unknowns, the rows and the cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    /// The number of unknowns `x_0 … x_{n−1}`.
    pub unknowns: usize,
    /// The linear rows the solution keeps.
    pub rows: Vec<Row>,
    squares: Vec<Square>,
    linear: Vec<(usize, f64)>,
}

/// Why a problem has no solution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QpError {
    /// The solver ended without a solution (infeasible, unbounded, iteration limit or a numerical
    /// failure).
    #[error("the quadratic program is not solved: {status}")]
    NotSolved {
        /// The solver's status.
        status: String,
    },
    /// The problem is malformed.
    #[error("invalid quadratic program: {what}")]
    Invalid {
        /// What is wrong, and where.
        what: String,
    },
}

impl Problem {
    /// A problem with `unknowns` unknowns, no rows and no cost.
    #[must_use]
    pub fn new(unknowns: usize) -> Problem {
        Problem {
            unknowns,
            rows: Vec::new(),
            squares: Vec::new(),
            linear: Vec::new(),
        }
    }

    /// Adds `weight * (Σ c_k x_k + constant)²` to the cost.
    pub fn add_square(&mut self, weight: f64, terms: &[(usize, f64)], constant: f64) {
        self.squares.push(Square {
            weight,
            terms: terms.to_vec(),
            constant,
        });
    }

    /// Adds `weight * x_k` to the cost.
    pub fn add_linear(&mut self, weight: f64, unknown: usize) {
        self.linear.push((unknown, weight));
    }

    /// The cost at `x` (for tests); NaN when a term names an unknown that `x` does not have.
    #[must_use]
    pub fn cost(&self, x: &[f64]) -> f64 {
        let value = |k: usize| x.get(k).copied().unwrap_or(f64::NAN);
        let squares: f64 = self
            .squares
            .iter()
            .map(|s| {
                let e = s.terms.iter().map(|&(k, c)| c * value(k)).sum::<f64>() + s.constant;
                s.weight * e * e
            })
            .sum();
        let linear: f64 = self.linear.iter().map(|&(k, w)| w * value(k)).sum();
        squares + linear
    }

    /// Checks every index, coefficient, bound and weight.
    fn validate(&self) -> Result<(), QpError> {
        let invalid = |what: String| Err(QpError::Invalid { what });
        let terms_ok =
            |terms: &[(usize, f64)], place: &dyn Fn() -> String| -> Result<(), QpError> {
                for &(k, c) in terms {
                    if k >= self.unknowns {
                        return invalid(format!(
                            "{}: unknown {k} out of range (unknowns {})",
                            place(),
                            self.unknowns
                        ));
                    }
                    if !c.is_finite() {
                        return invalid(format!("{}: coefficient {c} of unknown {k}", place()));
                    }
                }
                Ok(())
            };
        for (i, s) in self.squares.iter().enumerate() {
            let place = || format!("square {i}");
            if !s.weight.is_finite() || s.weight < 0.0 {
                return invalid(format!("{}: weight {}", place(), s.weight));
            }
            if !s.constant.is_finite() {
                return invalid(format!("{}: constant {}", place(), s.constant));
            }
            terms_ok(&s.terms, &place)?;
        }
        for (i, &(k, w)) in self.linear.iter().enumerate() {
            let place = || format!("linear term {i}");
            if !w.is_finite() {
                return invalid(format!("{}: weight {w}", place()));
            }
            terms_ok(&[(k, 1.0)], &place)?;
        }
        for (i, r) in self.rows.iter().enumerate() {
            let place = || format!("row {i}");
            if r.lo.is_nan() || r.hi.is_nan() || r.lo > r.hi {
                return invalid(format!("{}: bounds {} ..= {}", place(), r.lo, r.hi));
            }
            if (r.lo.is_infinite() && r.lo > 0.0) || (r.hi.is_infinite() && r.hi < 0.0) {
                return invalid(format!(
                    "{}: bounds {} ..= {} keep no value",
                    place(),
                    r.lo,
                    r.hi
                ));
            }
            if [r.lo, r.hi]
                .iter()
                .any(|b| b.is_finite() && b.abs() >= MAX_BOUND)
            {
                return invalid(format!(
                    "{}: bounds {} ..= {}: a finite bound must be below {MAX_BOUND:e} in size",
                    place(),
                    r.lo,
                    r.hi
                ));
            }
            terms_ok(&r.terms, &place)?;
        }
        Ok(())
    }
}

/// The terms of `terms` with each unknown once, coefficients added, in the order of the unknowns.
fn merged(terms: &[(usize, f64)]) -> Vec<(usize, f64)> {
    let mut map = BTreeMap::new();
    for &(k, c) in terms {
        *map.entry(k).or_insert(0.0) += c;
    }
    map.into_iter().collect()
}

/// Triplets `(rows, cols, values)` of a sparse matrix.
#[derive(Default)]
struct Triplets {
    rows: Vec<usize>,
    cols: Vec<usize>,
    values: Vec<f64>,
}

impl Triplets {
    fn push(&mut self, row: usize, col: usize, value: f64) {
        self.rows.push(row);
        self.cols.push(col);
        self.values.push(value);
    }

    fn all_finite(&self) -> bool {
        self.values.iter().all(|v| v.is_finite())
    }

    /// The matrix, checked to be in the canonical form the solver relies on.
    fn into_csc(self, m: usize, n: usize, what: &str) -> Result<CscMatrix<f64>, QpError> {
        let matrix = CscMatrix::new_from_triplets(m, n, self.rows, self.cols, self.values);
        matrix.check_format().map_err(|e| QpError::Invalid {
            what: format!("{what}: {e}"),
        })?;
        Ok(matrix)
    }
}

/// `P` (upper triangle), `q` and the constant of the cost.
fn cost_matrices(problem: &Problem) -> (Triplets, Vec<f64>) {
    let mut p = Triplets::default();
    let mut q = vec![0.0; problem.unknowns];
    for s in &problem.squares {
        // w (cᵀx + k)² = ½ xᵀ (2w c cᵀ) x + (2w k c)ᵀ x + w k²
        let terms = merged(&s.terms);
        for (a, &(i, ci)) in terms.iter().enumerate() {
            for &(j, cj) in terms.iter().skip(a) {
                // `terms` is sorted, so i ≤ j: the upper triangle.
                p.push(i, j, 2.0 * s.weight * ci * cj);
            }
            if let Some(qi) = q.get_mut(i) {
                *qi += 2.0 * s.weight * s.constant * ci;
            }
        }
    }
    for &(k, w) in &problem.linear {
        if let Some(qk) = q.get_mut(k) {
            *qk += w;
        }
    }
    (p, q)
}

/// `A`, `b` and the cones of the rows: the equality rows first (zero cone), then every finite bound
/// as one row of the nonnegative cone (`A x ≤ b`).
fn row_matrices(problem: &Problem) -> (Triplets, Vec<f64>, Vec<SupportedConeT<f64>>) {
    let mut a = Triplets::default();
    let mut b = Vec::new();
    let push =
        |a: &mut Triplets, b: &mut Vec<f64>, terms: &[(usize, f64)], sign: f64, bound: f64| {
            let row = b.len();
            for &(k, c) in terms {
                a.push(row, k, sign * c);
            }
            b.push(sign * bound);
        };
    let (equal, inequal): (Vec<&Row>, Vec<&Row>) = problem.rows.iter().partition(|r| r.lo >= r.hi);
    for r in &equal {
        push(&mut a, &mut b, &r.terms, 1.0, r.hi);
    }
    let zero = b.len();
    for r in &inequal {
        if r.hi.is_finite() {
            push(&mut a, &mut b, &r.terms, 1.0, r.hi);
        }
        if r.lo.is_finite() {
            push(&mut a, &mut b, &r.terms, -1.0, r.lo);
        }
    }
    let nonneg = b.len() - zero;
    let mut cones = Vec::new();
    if zero > 0 {
        cones.push(SupportedConeT::ZeroConeT(zero));
    }
    if nonneg > 0 {
        cones.push(SupportedConeT::NonnegativeConeT(nonneg));
    }
    (a, b, cones)
}

/// Solves `problem`: the unknowns at the minimum of the cost that keep every row.
///
/// The solution keeps the rows only within the solver's tolerance (`1e-7`, absolute and relative): a value
/// can lie slightly outside a bound, and a slack bounded by `≥ 0` can come back a few `1e-9` below zero.
/// Callers clamp such values and never test a row for exact equality.
///
/// # Errors
///
/// [`QpError::Invalid`] for an unknown index out of range, a non-finite coefficient, constant, weight or
/// NaN bound, a finite bound of `1e15` or more in size (the solver would drop it), `lo > hi`, a row that
/// keeps no value (`lo = +∞` or `hi = −∞`) or a negative weight;
/// [`QpError::NotSolved`] when the solver does not report a solution (rows that contradict each other,
/// an unbounded cost, the iteration limit or a numerical failure).
pub fn solve(problem: &Problem) -> Result<Vec<f64>, QpError> {
    solve_reported(problem).map(|(x, _)| x)
}

/// What the solver reported for a solution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolveReport {
    /// The solver's status.
    pub status: String,
    /// The iterations it took.
    pub iterations: u32,
}

/// [`solve`], with what the solver reported (for measuring).
///
/// # Errors
///
/// As [`solve`].
pub fn solve_reported(problem: &Problem) -> Result<(Vec<f64>, SolveReport), QpError> {
    problem.validate()?;
    let unknowns = problem.unknowns;
    if unknowns == 0 {
        let report = SolveReport {
            status: format!("{:?}", SolverStatus::Solved),
            iterations: 0,
        };
        return Ok((Vec::new(), report));
    }
    let (quadratic, linear) = cost_matrices(problem);
    let (rows, bounds, cones) = row_matrices(problem);
    if !quadratic.all_finite() || !linear.iter().all(|v| v.is_finite()) || !rows.all_finite() {
        return Err(QpError::Invalid {
            what: "the cost or the rows overflow".to_string(),
        });
    }
    let quadratic = quadratic.into_csc(unknowns, unknowns, "the cost matrix")?;
    let rows = rows.into_csc(bounds.len(), unknowns, "the row matrix")?;

    let settings = DefaultSettingsBuilder::default()
        .max_iter(MAX_ITER)
        .time_limit(f64::INFINITY)
        .verbose(false)
        .tol_gap_abs(TOLERANCE)
        .tol_gap_rel(TOLERANCE)
        .tol_feas(TOLERANCE)
        .tol_infeas_abs(TOLERANCE)
        .tol_infeas_rel(TOLERANCE)
        .max_threads(1)
        .direct_solve_method("qdldl".to_string())
        .build()
        .map_err(|e| QpError::Invalid {
            what: format!("solver settings: {e}"),
        })?;
    let mut solver = DefaultSolver::new(&quadratic, &linear, &rows, &bounds, &cones, settings)
        .map_err(|e| QpError::Invalid {
            what: format!("solver input: {e}"),
        })?;
    solver.solve();
    match solver.solution.status {
        status @ (SolverStatus::Solved | SolverStatus::AlmostSolved) => {
            let report = SolveReport {
                status: format!("{status:?}"),
                iterations: solver.solution.iterations,
            };
            Ok((solver.solution.x, report))
        }
        status => Err(QpError::NotSolved {
            status: format!("{status:?}"),
        }),
    }
}

<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# One height solve for the road network: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** All road and junction heights come out of one quadratic program whose slacks are the findings; it replaces the junction repair, the per-chain solve, settling and the clearance rounds.

**Architecture:** A small `qp` module wraps a sparse QP solver behind one function. `profile::model` turns the placed network into affine height expressions, a cost and rows (grade, bend, tilt, clearance, nearby roads); `profile::solve_network` solves it in two phases and writes strips and junctions back. The old steps stay until Task 6 switches the pipeline and deletes them.

**Tech Stack:** Rust 1.99, `naves-core`; candidate crate `clarabel` 0.11 (`default-features = false`), fallback our own ADMM.

**Spec:** `docs/superpowers/specs/2026-11-16-height-solve-design.md` (binding: every symbol, constant and row below is defined there).

## Global Constraints

- AGENTS.md in full. Test first (RED captured, then GREEN). `make check` before every commit, coverage ≥ 85 %.
- Library code never panics on input: no `unwrap`, `expect`, indexing; `#[expect(lint, reason)]`, never `#[allow]`.
- Deterministic: same input, bit-identical heights. No `HashMap` order, no threads in the solve, no time limits.
- No game data in tests. Temp files under `/var/tmp`, never `$HOME`.
- Every long process runs under `systemd-run --user --scope --quiet -p MemoryMax=<N>G -p MemorySwapMax=1G`
  (cargo and `make check` 12G, `road-analysis` 16G, a map build's sway 16G). Never raise a cap; report an OOM kill.
- Wait on a PID or a marker file, never a `pgrep -f` loop. No DayZ or Wine unless the task says so. No push.
- Docs (`docs/manual`) and a `CHANGELOG.md` line in the commit that changes behaviour. Conventional commits.
- Constants, verbatim from the spec: `SMOOTHNESS = 30.0`, `REFERENCE_M = 12.5`, `MIN_STENCIL_M = 1.0`,
  `JUNCTION_GROUND = 2.0`, `JUNCTION_TILT = 2.0`, tilt lever `25.0`, `AIM = 1.0`, `NEAR = 10.0`, `HARD = 1.0e4`,
  `TIE_STEP_M = 1.0`, `KEEP_APART_M = 50.0`, `MATCH_M = 0.1`, `SAMPLE_M = 3.0`, `BRIDGE_CLEARANCE_M = 4.5`,
  `CLEARANCE_TOLERANCE_M = 0.05`, `EXCESS_TOLERANCE = 1.0e-4`, solver tolerances `1e-7`, iteration limit 200.
- Figures are quoted for Berchtesgaden at `terrain_size = 2048`. Baseline (e84cb2f): `road_grade` 1079 (751 bends /
  328 grades), `road_misses_terrain` 4625, `road_conflict` 1025, `road_not_drivable` 208, `road_gap` 343,
  `join_without_junction` 65, `junction_too_steep` 8; placement and analysis about 1775 s.

## Review Focus

- A network with no junction, no crossing and one chain of one piece (the smallest problem; empty row sets).
- A chain whose every variable is a constant (a ring): no unknowns of its own, cost and rows must not index them.
- Rows that contradict each other (two rings at fixed heights joined by a short road): the solve ends, slack > 0.
- NaN or infinite ground heights (no-data in the DEM under a road): an error or a finite target, never NaN in `P`.
- A meeting whose owner chain's variables are themselves expressions (held or a meeting): substitution, not a panic.

---

### Task 1: The QP seam and its solver (Opus)

**Files:** Create `crates/naves-core/src/qp.rs`; Modify `crates/naves-core/src/lib.rs`, `crates/naves-core/Cargo.toml`,
`deny.toml` if needed; Test `crates/naves-core/tests/qp.rs`

**Interfaces (produced):**
```rust
pub struct Row { pub terms: Vec<(usize, f64)>, pub lo: f64, pub hi: f64 }   // ±INFINITY for a one-sided row
pub struct Problem { pub unknowns: usize, pub rows: Vec<Row>, /* private: P triplets, q, constant */ }
impl Problem {
    pub fn new(unknowns: usize) -> Problem;
    /// Adds `weight * (Σ c_k x_k + constant)²` to the cost.
    pub fn add_square(&mut self, weight: f64, terms: &[(usize, f64)], constant: f64);
    /// Adds `weight * x_k` to the cost.
    pub fn add_linear(&mut self, weight: f64, unknown: usize);
    /// The cost at `x` (for tests).
    pub fn cost(&self, x: &[f64]) -> f64;
}
#[derive(thiserror::Error)] pub enum QpError { NotSolved { status: String }, Invalid { what: String } }
pub fn solve(problem: &Problem) -> Result<Vec<f64>, QpError>;
```
`Invalid`: an index ≥ `unknowns`, a non-finite coefficient or bound, `lo > hi`, a negative weight.

- [ ] **Step 1: The dependency review (AGENTS.md "Dependencies", strict).** Candidates: `clarabel` (0.11.1,
  Apache-2.0), `osqp` (C binding), `osqp-rust` (c2rust), others found with `cargo search`. For `clarabel` with
  `default-features = false`: read the source (not the README): every `unsafe` block, panics on bad input,
  allocation, the build script, the dependency tree (`cargo tree`), RustSec, releases and maintainers, tests,
  licence against `deny.toml`, builds on Rust 1.99 and Windows targets, determinism (single thread, QDLDL, no
  time-based stop). Write the result to the report. **If it fails, stop and report (BLOCKED) with the reasons**; the
  controller then orders the ADMM fallback behind the same interface.
- [ ] **Step 2: Failing tests** in `tests/qp.rs`:
  - `a_least_squares_problem_without_rows`: `add_square(1, [(0,1)], -3)`, `add_square(2, [(1,1)], 4)` → x = (3, −4)
    within 1e-6.
  - `a_bound_cuts_the_minimum`: minimise `(x−3)²` with row `x ≤ 1` → 1 within 1e-6.
  - `an_exact_penalty_slack_is_zero_when_the_row_can_be_kept`: `(x−3)²`, row `x − s ≤ 5`, `s ≥ 0`, cost
    `1e4·s + 1e4·s²` → s < 1e-6; and with row `x − s ≤ 1` and a second row `x ≥ 2` → s = 1 within 1e-5.
  - `contradicting_rows_without_slack_are_not_solved`: `x ≤ 0`, `x ≥ 1` → `Err(NotSolved)`.
  - `invalid_input_is_an_error`: an index out of range, NaN coefficient, `lo > hi` → `Err(Invalid)`.
  - `two_solves_are_bit_identical`: a chain of 2000 unknowns with second-difference cost and grade rows, solved
    twice → `==` on the vectors' bits.
  - `no_unknowns` → `Ok(vec![])`.
- [ ] **Step 3: Implement** with `cargo add clarabel --no-default-features` (plus only features the review
  shows are needed). Settings: tolerances 1e-7, `max_iter` 200, no time limit, verbose off, single thread. Map
  `Solved` and `AlmostSolved` to `Ok`, everything else to `NotSolved { status }`.
- [ ] **Step 4:** PASS, `make deny`, `make check`.
- [ ] **Step 5: Commit** `feat: a sparse quadratic program solver behind one function`, with the review of every
  candidate in the message.

### Task 2: The height model and the phase A solve (Opus)

**Files:** Create `crates/naves-core/src/profile/model.rs`; Modify `crates/naves-core/src/profile.rs` (module,
`excesses_of`), `crates/naves-core/src/junctions.rs` (`Junction` fields); Test `crates/naves-core/tests/heightsolve.rs`

**Interfaces:**
- Consumes: `qp::{Problem, Row, solve, QpError}`; from the existing code `chain_of`, `holds::fixed_heights` /
  `junction_heights` (which junction holds which variable), `end_joins` (meetings), `segment_grades`, `ring_of`,
  `strip_of`, `joint_heights`, `excesses_of`, `limits::*`.
- Produces:
```rust
// junctions.rs, set by find_junctions next to `gradient`:
pub tilt_axis: Option<(f64, f64)>,   // unit vector along the through road; None: level (join, ring, no through road)
pub tilt_cap: f64,                   // min(JUNCTION_TILT_SHARE * ceiling, CROSS_FALL_AIM); 0 where no axis
// profile.rs
#[derive(thiserror::Error)] pub enum ProfileError { NotSolved { status: String }, Invalid { what: String } }
pub fn solve_network(net: &PlacedNetwork, hm: &Heightmap, junctions: &Junctions, side: &[f64])
    -> Result<(Profiles, Junctions), ProfileError>;
#[doc(hidden)] pub fn solve_network_with(/* same */, solver: &dyn Fn(&Problem) -> Result<Vec<f64>, QpError>)
    -> Result<(Profiles, Junctions), ProfileError>;
// model.rs (pub(crate))
pub(crate) struct Expr { pub terms: Vec<(usize, f64)>, pub constant: f64 }
pub(crate) struct Model { /* per chain: Chain, Expr per variable, limits; junction unknowns; Problem */ }
```
`solve_network` implements the spec's joints and unknowns, the whole cost except `NEAR`, and rows (G), (B) with
the two deck-end rows, and (T): phase A only. The returned `Junctions` is a clone with `height = z_j` and
`gradient = s_j · u_j` for every junction that holds a variable; `unresolved` as the spec says. `excesses_of` is
changed to the spec's measure (ceiling limits, `EXCESS_TOLERANCE`, deck ends) **for the new path only**; the old
path keeps its own until Task 6. A non-finite ground target is `ProfileError::Invalid`.

The old `solve_profiles`, `solve_clearing`, `settle` and `repair` are not touched in this task. `find_junctions`
still calls `repair`; `solve_network` ignores the repaired heights (it starts from `Junction::ground`).

- [ ] **Step 1: Failing tests** in `tests/heightsolve.rs` (synthetic networks on planes, with the helpers the
  existing `tests/profile.rs` and `tests/junctions.rs` use; `Asf3`: aim 0.12, ceiling 0.27, radius from `limits`):
  - `a_road_under_its_aim_lies_on_the_ground`: one chain, 200 m, plane rising 5 % → every joint within 1e-4 m of
    the ground; no excess.
  - `a_road_steeper_than_its_ceiling_keeps_the_ceiling`: plane 40 % → every segment grade ≤ 0.27 + 1e-4; no
    excess; the mean of `h − ground` over the joints within 0.5 m of 0 (it does not drift off).
  - `between_aim_and_ceiling_the_road_stays_near_the_ground`: plane 20 % → every grade in (0.12, 0.20 + 1e-4];
    every joint within 2 m of the ground.
  - `two_junctions_too_steep_for_each_other_both_move`: two T-junctions 20 m apart on a through road, ground at
    them 8 m apart (40 %) → the grade between them ≤ ceiling + 1e-4, both `|z − ground| ≥ 0.5`, no excess,
    `unresolved` empty.
  - `a_junction_on_a_slope_tilts_with_its_through_road`: plane 6 % along the through road → `|s − 0.06| ≤ 1e-3`,
    held joints within 0.01 m of the ground.
  - `a_junction_tilt_stays_within_its_cap`: plane 20 % → `|s| ≤ tilt_cap + 1e-6`.
  - `a_deck_bends_at_both_ends`: approaches of 12.5 m pieces on ground rising 10 % to a 25 m deck and falling
    10 % after it → at each deck end the change of grade ≤ `run / R + 1e-4` with `run = (12.5 + 25) / 2`; no
    `VerticalBend` excess at the deck. And `a_sharp_deck_end_is_reported`: both abutments' approaches pinned by rings
    so that the approach must be at 20 % → a `VerticalBend` excess at each deck end (two, not one).
  - `fixed_heights_that_cannot_be_joined_give_a_finding`: two rings (planes 10 m apart in height) joined by a 10 m
    road → `Ok`, a `Grade` excess on it.
  - `a_chain_end_on_another_road_takes_its_height`: a side road ending on a through road without a junction →
    its end is within 1e-6 of the through road's height there.
  - `a_road_under_an_overlay_lies_5_cm_below`: as the existing overlay test of `tests/profile.rs`, on the new path.
  - `a_single_piece_network_solves`, `a_ring_alone_solves` (every variable a constant).
  - `no_data_under_a_road_is_an_error`: a NaN in the heightmap under a joint → `Err(Invalid)`.
  - `a_solver_that_fails_is_an_error`: `solve_network_with` and a solver returning `NotSolved` → `Err(NotSolved)`.
  - `the_solve_is_bit_identical`: a network with a junction, solved twice.
- [ ] **Step 2–4:** RED captured, implement, PASS, `make check`.
- [ ] **Step 5:** One run of the new path on Berchtesgaden 2048 through a temporary, uncommitted hook in
  `road-analysis` (or a scratch example under `/var/tmp`): report unknowns, rows, solve time, solver status and
  iterations, and the number of `Grade` and `VerticalBend` excesses. Not a gate; the numbers go into the report.
- [ ] **Step 6: Commit** `feat: road heights as one quadratic program (grades, bends, junction planes)`.

### Task 3: Clearance under bridges, phase B (Sonnet)

**Files:** Modify `crates/naves-core/src/profile/model.rs`, `crates/naves-core/src/profile.rs`; Test
`crates/naves-core/tests/heightsolve.rs`

**Interfaces:**
- Consumes: Task 2's `Model`, `solve_network_with`; `roadnet::over_crossings`, the existing `clearance_at`,
  `lower_surface`, `measure_clearances`, `deck_road`.
- Produces:
```rust
pub fn solve_network_clearing(net: &PlacedNetwork, hm: &Heightmap, junctions: &Junctions, side: &[f64],
    graph: &RoadGraph, thickness: &dyn Fn(&Stretch) -> Option<f64>)
    -> Result<(Profiles, Junctions), ProfileError>;
```
Phase A is `solve_network`'s problem; phase B adds the spec's rows (C) (the deck bound with slack `q_c`, the two
pins of the road below with slacks `p_w`) and solves again. No crossing with a known thickness: phase A's result.
`Profiles::clearances` is `measure_clearances` on the result.

- [ ] **Step 1: Failing tests** (the scenarios of road-look Task 10 in `tests/profile.rs`, on the new function):
  - `a_bridge_over_a_road_clears_it`: a bridge crossing a road on flat ground, thickness 1.0 → deck surface ≥
    road + 5.5 − 0.05; `clearances[0].reached ≥ 4.45`; the approaches' grades ≤ ceiling + 1e-4 (no wedge: no
    `Grade` excess on the bridge's chain).
  - `the_road_below_keeps_its_height`: the lower road's joints within 50 m of the crossing differ from phase A
    (`solve_network`) by less than 0.05 m.
  - `a_junction_near_the_bridge_rises_with_it`: a junction on the bridge's road 30 m from the deck end → its
    height is above its ground, and the grade from it to the deck ≤ ceiling + 1e-4.
  - `a_deck_and_the_road_below_on_one_junction_is_a_finding`: both held by one junction → `Ok`, `reached < 4.45`.
  - `an_unknown_thickness_is_not_checked`: `thickness` returns `None` → the result equals `solve_network`'s.
- [ ] **Step 2–4:** RED, implement, PASS, `make check`.
- [ ] **Step 5: Commit** `feat: bridge clearance as rows of the height solve`.

### Task 4: Roads that share terrain cells (Sonnet)

**Files:** Modify `crates/naves-core/src/profile/model.rs`, `crates/naves-core/src/profile.rs` (make `spots`,
`close_pairs` usable before heights exist); Test `crates/naves-core/tests/heightsolve.rs`

**Interfaces:** Consumes Task 2 and 3. Produces no new public item: `solve_network` and
`solve_network_clearing` add the spec's rows (N) and the `NEAR` cost in both phases. `spots` and `close_pairs`
work on joint positions and half widths (heights unused); runs, the ground test with `TIE_STEP_M`, and the
exclusion near an over-crossing are as the spec says.

- [ ] **Step 1: Failing tests:**
  - `two_legs_of_a_hairpin_share_one_height`: two parallel roads 4 m apart (footprints overlap) on ground that
    differs by 0.6 m between them → at every pair `|h_a − h_b| ≤ 0.1 + 0.05`.
  - `roads_side_by_side_on_a_hillside_stay_apart`: the same with the ground 4 m apart in height over the whole
    run → both roads within 0.5 m of their own ground (no rows).
  - `one_untied_pair_unties_the_whole_run`: ground difference 0.6 m except 3 m in the middle third → no rows for
    the run (both roads within 0.5 m of their ground at the run's ends too).
  - `a_bridge_and_the_road_below_are_not_tied`: Task 3's crossing → the clearance is still reached.
  - `roads_further_apart_may_differ_by_the_fill_slope`: 10 m apart, ground 5 m apart in height → heights differ
    by at most `allowed` of the spec + 0.05.
- [ ] **Step 2–4:** RED, implement, PASS, `make check`.
- [ ] **Step 5: Commit** `feat: roads that share terrain cells share a height in the height solve`.

### Task 5: Switch the pipeline and remove the old steps (Sonnet)

**Files:** Modify `crates/naves-export/src/generate.rs` (`place_once`), `crates/naves-export/examples/road-analysis.rs`,
`crates/naves-core/src/profile.rs`, `crates/naves-core/src/profile/holds.rs`, `crates/naves-core/src/junctions.rs`,
every test that uses the removed items, `docs/manual/src/` (the profile and junction sections of getting-started),
`docs/research/2026-11-10-road-alignment.md` (a new section: the height solve, with the figures), `CHANGELOG.md`

**Interfaces (final names):**
```rust
pub fn solve_profiles(net, hm, junctions, side) -> Result<Profiles, ProfileError>;          // = solve_network(..).0
pub fn solve_clearing(net, hm, junctions, side, graph, thickness)
    -> Result<(Profiles, Junctions), ProfileError>;                                          // = solve_network_clearing
```
`solve_network*` are renamed to these; `solve_profiles_with` and `DeckFloor` go. `BuildError` gets a variant for
`ProfileError`. Everything in the spec's "What is removed" is deleted; `find_junctions` no longer repairs.

- [ ] **Step 1:** Switch `place_once` and `road-analysis`; delete the old code; make the test suites compile.
  Per old test: keep it if its scenario and assertion still describe wanted behaviour (adapt the call), port it if
  only the mechanism changed, delete it if it pins a removed mechanism (`repair` rounds, `raise_floors`,
  `bound_spans`, settle rounds). List every deleted and every changed test with one line of reason in the report.
- [ ] **Step 2:** `make check`. Coverage must not fall below the base's.
- [ ] **Step 3:** `road-analysis` at 2048 once; report every count against the baseline in Global Constraints, the
  clearances, the placement and analysis time, the solver's sizes and iterations.
- [ ] **Step 4:** Manual and research note describe the solve as it is now (what is solved, what a finding means,
  the weights). CHANGELOG under Changed.
- [ ] **Step 5: Commit** `feat!: the road network's heights are solved as one problem` (the public API of
  `naves-core::profile` changes).

### Checkpoint A (controller)

Compare the counts with the spec's "How it is judged". Game check with tools/ingame-check on sites 23, 50, 51, 7,
11, 28, 29, 37, 1, 2 (memory caps; fresh `naves` binary, HEAD and its mtime in the build log). If a judged number
or a site is worse, amend the plan with a tuning or fix task before the README.

### Task 6: Figures (Sonnet)

`examples/berchtesgaden/README.md`: a dated section with the new counts, times and the in-game results; the
build-time table. Commit `docs: the height solve on the Berchtesgaden example`.

### Final review

Whole-branch review on the most capable model, then superpowers:finishing-a-development-branch.

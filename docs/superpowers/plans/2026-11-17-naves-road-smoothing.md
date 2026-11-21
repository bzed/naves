<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Road Smoothing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Smooth every road's centre line in the graph, before bridges are fitted, so that all later stages share
one smooth line.

**Architecture:** A new module `crates/naves-core/src/smooth.rs` fits a smooth curve to each chain of stretches
inside a corridor around the OpenStreetMap line (a small convex problem per chain, solved with `clarabel`, already a
dependency of `naves-core`). `generate::fitted_graph` calls it between `drop_duplicates` and `fit_bridges`. Nothing
else changes how it reads the graph.

**Tech Stack:** Rust 1.99, `clarabel` (existing), `rayon` (existing). No new dependency is planned; if one seems
needed, AGENTS.md's crate review applies and the commit message carries it.

**Spec:** `docs/superpowers/specs/2026-11-17-road-smoothing-design.md`

**Implementers:** Sonnet. **Reviews:** Opus (maintainer, 2026-11-17).

## Global Constraints

- AGENTS.md binds: test first (RED then GREEN), `make check` green before every commit, coverage not below the value
  on the base commit, no panics on input, `#[expect(lint, reason)]` never `#[allow]`, no truncating casts, sort with
  `total_cmp`, deterministic output, docs and CHANGELOG in the behaviour commit, no game data in the repo.
- Every process gets a hard memory limit: `systemd-run --user --scope --quiet -p MemoryMax=12G -p MemorySwapMax=1G`
  for cargo and `make check`, 16G for `road-analysis`, server 8G, client 14G, `ingame-check` 24G. Never raise a cap.
- Temp and build folders under `/var/tmp` (never `/tmp`, never `$HOME`). Shared target dir `/var/tmp/naves-target`.
- Never start the game outside the checkpoint task. Wine/Proton only inside headless sway. Never touch the user's
  DayZ profile folder. Remove `@naves*` symlinks from the DayZ install afterwards.
- Figures are quoted for Berchtesgaden at terrain size 2048.
- Constants, verbatim from the spec: `SAMPLE_M = 2.0`, `CORRIDOR_M = 2.0`, `W_STAY = 0.02`,
  `BRIDGE_STRAIGHT_WEIGHT = 10.0`, `CHAIN_TURN_MAX_DEG = 150.0`, `SIMPLIFY_TOL_M = 0.05`.
- Junction nodes, bridge end nodes, stretch ids and node ids do not change. No road moves more than `CORRIDOR_M`
  sideways.
- Geometry first: the pieces contribute only `min_radius(family)`.

## Review Focus

- A chain that is a closed ring (roundabout): it must be cut deterministically and still be smoothed, not skipped or
  looped forever.
- A stretch shorter than `SAMPLE_M`, a zero-length segment, duplicate points, NaN coordinates: no panic, the line is
  kept, the report counts it.
- A real switchback: after smoothing it is still a switchback inside the corridor, not a shortcut.
- A solver failure on one chain must not fail the build or change other chains.
- The same input must give byte-identical graphs on repeated runs and with a different rayon thread count.

---

### Task 1: Baseline on main (Sonnet)

**Files:** none in the repo. Output under `/var/tmp/naves-smooth/baseline/`.

- [ ] **Step 1:** On `main` after the bridges merge (log `git rev-parse HEAD` and the `naves` binary's mtime), run
  `road-analysis` on Berchtesgaden at 2048 once (16G cap), log to `baseline/ra-2048.log`.
- [ ] **Step 2:** Apply `/var/tmp/naves-smooth/recon.patch` in a scratch worktree under `/var/tmp` (own target dir
  `/var/tmp/naves-smooth-target`), re-run it, and record: bridge-end turns (p50, p95, max), placed-joint distance
  from the line (median, p95, share over 1 m), corner statistics. Do not commit the patch.
- [ ] **Step 3:** Write `baseline/baseline.md` with every figure of the spec's "How it is judged" section and the
  run time. Remove the worktree.

No commit. The controller copies the figures into the later task dispatches.

### Task 2: The curve fit for one line (Sonnet)

**Files:** Create `crates/naves-core/src/smooth.rs`, `crates/naves-core/tests/smooth.rs`; modify
`crates/naves-core/src/lib.rs` (module), `fuzz/` (a target for `smooth_line`, seeds run in `make test`).

**Interfaces — Produces:**

```rust
pub const SAMPLE_M: f64 = 2.0;
pub const CORRIDOR_M: f64 = 2.0;
pub const W_STAY: f64 = 0.02;
pub const BRIDGE_STRAIGHT_WEIGHT: f64 = 10.0;

/// One sample of a line to smooth.
pub struct Sample { pub at: (f64, f64), pub fixed: bool, pub stiffness: f64 }

pub struct SmoothParams { pub corridor: f64, pub stay: f64 }
impl Default for SmoothParams { /* CORRIDOR_M, W_STAY */ }

#[derive(Debug, thiserror::Error)]
pub enum SmoothError { /* TooShort, NotFinite, Solver(String) */ }

/// Points at equal steps of about `step` along `line`; first and last are kept exactly.
pub fn resample(line: &[(f64, f64)], step: f64) -> Vec<(f64, f64)>;

/// The smoothed positions of `samples`, same length and order. Fixed samples do not move; no sample moves
/// more than `params.corridor` along its normal.
pub fn smooth_line(samples: &[Sample], params: &SmoothParams) -> Result<Vec<(f64, f64)>, SmoothError>;

/// The radius of the circle through three consecutive points (infinite on a straight line).
pub fn radius_at(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64;
```

The normal at a sample is the left normal of the direction from its previous to its next sample (one-sided at the
ends). The problem is the one in the spec ("The curve fit"); build it for `clarabel` the way `profile.rs`'s height
solve builds its problem (read that first and reuse its conventions and tolerances).

- [ ] **Step 1: Failing tests** (`tests/smooth.rs`):
  - `a_straight_line_stays_where_it_is`: 50 collinear samples → every output within 1e-6 of its input.
  - `a_corner_is_rounded_inside_the_corridor`: an L of two 40 m legs at 90°, resampled at 2 m, ends fixed → the
    largest turn between consecutive output segments is under 15° (it is 90° in the input); every output is within
    `CORRIDOR_M + 1e-6` of its input; `radius_at` at the corner sample is at least 4 m.
  - `fixed_samples_do_not_move`: the same L with samples 0, 1 and the last fixed → those three equal their inputs
    exactly.
  - `a_curve_drawn_as_short_segments_becomes_even`: a 90° arc of radius 30 m drawn with segments of 2, 9, 3, 11, 4 m
    (repeat to cover it), resampled → the largest `|turn_i − turn_{i+1}|` between consecutive output corners is
    smaller than in the resampled input by at least half.
  - `a_switchback_stays_a_switchback`: two parallel 60 m legs 8 m apart joined by a half circle of radius 4 m →
    every output within the corridor; the output's summed signed turn is within 5° of 180°; the two legs' outputs
    stay at least 4 m apart.
  - `stiff_samples_stay_straighter`: an S-bend whose middle third has `stiffness` 10 → the summed absolute turn
    inside the middle third is smaller than with stiffness 1.
  - `degenerate_input_is_an_error_not_a_panic`: 0, 1, 2 samples; a NaN coordinate; all samples identical →
    `Err`, no panic.
  - `resample_keeps_the_ends_and_spaces_evenly`: a 10.5 m line at step 2 → 6 points (5 equal steps of 2.1 m), first
    and last exact; a 1 m line → its two ends; zero-length segments inside are skipped.
  - `the_same_input_gives_the_same_bytes`: two calls → `to_bits()` equal for every coordinate.
- [ ] **Step 2:** Run `cargo test -p naves-core --test smooth` → fails to compile (module missing).
- [ ] **Step 3:** Implement the three functions and the types.
- [ ] **Step 4:** Tests pass; add the fuzz target (arbitrary f64 pairs and flags → no panic) and its seeds.
- [ ] **Step 5:** `make check`; commit `feat: a smooth curve can be fitted to a line inside a corridor`.

### Task 3: Chains and the whole graph (Sonnet)

**Files:** Modify `crates/naves-core/src/smooth.rs`, `crates/naves-core/tests/smooth.rs`; read `roadnet.rs`
(`RoadGraph`, `Stretch`, nodes, how the chain pairing finds a road's continuation) and `catalog.rs` (curve pieces).

**Interfaces — Consumes:** Task 2. **Produces:**

```rust
pub const CHAIN_TURN_MAX_DEG: f64 = 150.0;

/// What smoothing did, for the build report and the measurement.
pub struct SmoothReport {
    pub chains: usize,
    pub kept: Vec<(StretchId, String)>,      // stretches left as they were, with the reason, in stretch order
    pub max_move_m: f64,
    pub tight_before_m: f64,                 // road length tighter than its family's minimum radius
    pub tight_after_m: f64,
    pub per_stretch: Vec<StretchSmooth>,     // in stretch order: id, max_move_m, tight_before_m, tight_after_m
}

/// The graph with every stretch's line smoothed. Node positions, node ids and stretch ids are unchanged.
pub fn smooth_graph(graph: &RoadGraph, min_radius: &dyn Fn(&Stretch) -> f64, params: &SmoothParams)
    -> (RoadGraph, SmoothReport);

/// The tightest curve piece's radius of a road family in the catalog (None if the family has no curve piece).
pub fn min_radius(catalog: &PieceCatalog, family: /* the family type roadplace uses */) -> Option<f64>;
```

Use the real type names found in `roadnet.rs`/`catalog.rs`; if a name above does not exist, take the existing one
and say so in the report. Chains, fixed samples and stiffness are exactly as in the spec ("Chains", "The curve
fit"). Chains are solved independently (rayon `par_iter` over a chain list in deterministic order, results collected
in that order).

- [ ] **Step 1: Failing tests** (synthetic graphs built through the public graph API, as `tests/bridgekind.rs` does):
  - `a_road_is_smooth_across_a_two_stretch_node`: two stretches meeting at 30° at a node with no other stretch →
    the node's position is unchanged, and the turn between the last segment of the first and the first segment of
    the second is under 5°.
  - `a_junction_keeps_its_node_and_its_directions`: three stretches at a node, one arriving at 40° to its own next
    segment → the node and each stretch's second sample are unchanged (bitwise).
  - `a_bridge_keeps_its_ends_and_gets_straighter`: plain – bridge – plain, the bridge bent 20° in its middle, the
    approaches straight for 40 m → both bridge end nodes unchanged; the bridge's summed absolute turn is smaller
    than before; the approaches moved by no more than `CORRIDOR_M`.
  - `a_ring_is_cut_and_smoothed`: a closed ring of four stretches with no junction → returns, `chains == 1`, every
    node unchanged, `kept` is empty.
  - `a_sharp_return_ends_the_chain`: two stretches meeting at 170° → two chains.
  - `a_failed_chain_keeps_its_line_and_is_reported`: a stretch with a NaN point beside a sound stretch → the sound
    one is smoothed, the other is identical to its input and listed in `kept`.
  - `nothing_moves_further_than_the_corridor`: a random-walk graph with a fixed seed (20 stretches) →
    `report.max_move_m <= CORRIDOR_M + 1e-6`, and it equals the largest distance measured in the test.
  - `tight_length_is_reported`: an L corner on a family whose minimum radius is 10 m → `tight_before_m > 0` and
    `tight_after_m < tight_before_m`.
  - `the_result_does_not_depend_on_the_thread_count`: run in rayon pools of 1 and 4 threads → equal graphs (bitwise)
    and equal reports.
  - `min_radius_is_the_tightest_curve_piece`: a synthetic catalog with arcs of radius 25 and 10 → `Some(10.0)`; no
    arcs → `None`.
- [ ] **Step 2:** RED. **Step 3:** implement. **Step 4:** GREEN.
- [ ] **Step 5:** `make check`; commit `feat: the road graph can be smoothed chain by chain`.

### Task 3b: Tight curves are widened outward (Sonnet; may use one Opus helper)

Added 2026-11-21 (maintainer: tight curves from scaling are smoothed outwards; earthwork and the outward move are
fine). Spec section "Tight curves are widened outward"; its constants are used verbatim.

**Files:** Modify `crates/naves-core/src/smooth.rs` (split into `smooth.rs` and `smooth/widen.rs` if it helps),
`crates/naves-core/tests/smooth.rs`; read `roadnet.rs` (`crossings_of`).

**Interfaces — Consumes:** Tasks 2 and 3. **Produces:**

```rust
pub const WIDEN_LEAD: f64 = 4.0;
pub const WIDEN_REACH: f64 = 2.0;
pub const WIDEN_DIRECTIONS: usize = 8;
pub const WIDEN_SLACK_COST: f64 = 1e4;
pub const WIDEN_PASSES: usize = 3;
/// Switches the widening pass; off returns exactly what the fit alone gives.
pub const WIDEN_ENABLED: bool = true;

/// One tight zone of a chain and what widening did to it.
pub struct WidenedZone {
    pub at: (f64, f64),            // the tightest sample before widening
    pub stretches: Vec<StretchId>, // in chain order
    pub radius_before_m: f64,
    pub radius_after_m: f64,
    pub radius_wanted_m: f64,
    pub max_outward_m: f64,
    pub reached: bool,             // no sample of the zone is tighter than wanted afterwards
}
// SmoothReport gains: `zones: Vec<WidenedZone>` (chain order, then along the chain), `max_outward_m: f64`;
// StretchSmooth gains `max_outward_m: f64`.
```

`smooth_graph` keeps its signature. `max_move_m` keeps meaning the sideways move inside the normal corridor;
outward moves in zones are reported in `max_outward_m` and are not bound by `CORRIDOR_M`.

- [ ] **Step 1: Failing tests** (synthetic graphs, as in Task 3):
  - `a_hairpin_is_widened_to_the_minimum_radius`: two parallel 80 m legs 8 m apart joined by a half circle of
    radius 4 m, one stretch between two dead ends, family minimum 10 m → one zone, `reached`, every `radius_at`
    along the output at least 10 m · 0.92, `radius_after_m > radius_before_m`, the summed signed turn within 5° of
    180°, no sample moved inward of the original bend (each sample of the zone lies on the outside of the original
    line or on it, to 1e-6), `tight_after_m < tight_before_m`.
  - `a_right_angle_is_widened`: an L of two 60 m legs, minimum 10 m → `reached`, the radius at every sample at
    least 9.2 m, `max_outward_m > 0`.
  - `a_curve_that_is_wide_enough_is_left_to_the_fit`: a 90° arc of radius 30 m, minimum 10 m → no zone, the output
    is bitwise what `WIDEN_ENABLED = false` gives (compare with a run through the fit alone).
  - `a_way_split_inside_a_bend_moves_with_the_road`: the hairpin as two stretches meeting at its apex (a
    two-stretch node) → `reached`; both stretches share the moved node's position bitwise; the graph's crossing
    for that node id reports the moved position; node and stretch ids are unchanged.
  - `a_junction_in_the_bend_holds_it_and_the_zone_is_reported`: the hairpin with a third stretch joining at its
    apex → the node is unchanged bitwise, the zone is listed with `reached == false` and `radius_wanted_m == 10.0`,
    nothing panics, the other stretches of the graph are as the fit alone leaves them.
  - `a_bridge_in_a_zone_keeps_its_ends`: plain – bridge – plain with a tight bend on one approach 10 m from the
    bridge end → both bridge end nodes unchanged bitwise.
  - `the_outside_is_the_side_away_from_the_bend`: the hairpin mirrored (left-turning and right-turning) → the two
    outputs are mirror images to 1e-6.
  - `nothing_outside_a_zone_moves_further_than_the_corridor`: the random-walk graph of Task 3 →
    `max_move_m <= CORRIDOR_M + 1e-6` still holds for samples outside zones.
  - `widening_does_not_depend_on_the_thread_count` and `the_same_input_gives_the_same_bytes` extended to a graph
    with a zone.
  - `a_zone_with_a_nan_or_zero_minimum_is_skipped`: `min_radius` returning NaN, 0 or infinity → no zone, no panic.
- [ ] **Step 2:** RED. **Step 3:** implement. **Step 4:** GREEN.
- [ ] **Step 5:** `make check`; commit `feat: curves tighter than the road pieces are widened outward`.

### Task 4: Smoothing in the build (Sonnet)

**Files:** Modify `crates/naves-core/src/generate.rs` (`fitted_graph`, `FittedGraph`, `Generated`),
`crates/naves-export/src/build.rs` (report), the `road-analysis` example, tests that pin coordinates and now move,
`docs/manual/` (how roads follow OSM), `CHANGELOG.md`, the bridges spec's lead section (one sentence: smoothing runs
first).

**Interfaces — Consumes:** Task 3. **Produces:** `FittedGraph::smooth: SmoothReport`, `Generated::smooth`,
`build/report.json` key `roads.smoothing` (`chains`, `max_move_m`, `tight_before_m`, `tight_after_m`,
`kept` as a list of `{stretch, reason}`), a line in the build summary, and in `road-analysis` output the lines the
baseline has (so before and after compare directly).

- `fitted_graph`: `build_graph` → `collapse_small_rings` → `drop_duplicates` → **`smooth_graph`** → `fit_bridges`.
  A family without a curve piece uses `f64::INFINITY` as its minimum radius (nothing counts as tight).
- Tests that pin coordinates after `fitted_graph`: update the expectation only where the move is explained by
  smoothing, and list each changed test with old and new value in the report. A test that fails for another reason
  is a finding, not an expectation to change.

- [ ] **Step 1: Failing tests:** a generate-level test with a synthetic project whose road has a 60° corner 30 m
  before a bridge: `Generated::smooth.max_move_m > 0`; the bridge's `BridgeFit` records no lead at that end ("none
  needed") where the unsmoothed graph needed one; `report.json` has `roads.smoothing` with the five keys; two builds
  give identical `report.json` bytes.
- [ ] **Step 2:** RED. **Step 3:** implement. **Step 4:** GREEN, `make check`.
- [ ] **Step 5:** Run `road-analysis` at 2048 (16G cap, log `/var/tmp/naves-smooth/after/ra-2048.log`) and write
  `/var/tmp/naves-smooth/after/after.md`: every figure of the baseline beside its new value, the run time, the
  largest move, the ten stretches with the largest remaining tight length. Do not tune.
- [ ] **Step 6:** Commit `feat!: road lines are smoothed before bridges are fitted` (the `!` because edits keyed by
  slot can be orphaned; say so in the CHANGELOG).

### Task 5: Judge and tune once (Opus review of the figures, Sonnet for changes)

The controller reads `after.md` against the baseline with the reviewer.

- If the placement became more than 20 % slower: Sonnet adds the collinear-sample drop (`SIMPLIFY_TOL_M`) in
  `smooth_graph`, test first (`collinear_samples_are_dropped`: a straight 100 m stretch has two points after
  smoothing; a curve keeps every point whose removal would move the line by more than the tolerance). AGENTS.md's
  crate rule applies: check for a crate with line simplification first and justify the choice in the commit.
- If a finding got worse (`road_gap`, `road_conflict`, `road_not_drivable`, `road_misses_terrain`, joins over 0.5 m)
  by more than 2 %: the controller picks one constant to change (`CORRIDOR_M`, `W_STAY`, `BRIDGE_STRAIGHT_WEIGHT`),
  Sonnet changes it with its tests, one more run. At most two such rounds; what remains is recorded as a ruling.
- Commit per change: `fix: …`; final figures into the spec's "How it is judged" as a results table and into the
  manual/README where road figures are quoted.

### Checkpoint: the game (controller with a Sonnet agent)

Build the map from this branch (rebuild the `naves` binary first; log HEAD and the binary's mtime), then
`tools/ingame-check` on about ten sites picked from `after.md`: four bridge ends that kinked before, two
switchbacks, two city corners, one fork of bridges, one straight road as control. Same sites from a build of the
base commit for comparison where shots do not exist yet. All memory caps apply; one game at a time; non-default
ports; remove the mod symlinks afterwards. Send the pairs to the maintainer.

### Final review (Opus)

Whole-branch review against the spec, the deferred minors in the ledger, and the Review Focus list. One fix wave
(Sonnet). Then superpowers:finishing-a-development-branch.

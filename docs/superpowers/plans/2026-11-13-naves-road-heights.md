<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Road Heights, Junctions, Earthworks and Bridges Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Roads meet at one height taken from the ground, tracks get track limits, a road keeps its own ground next to another road, dips are crossed by bridges, and the default grid is 2048.

**Architecture:** A new `junctions` module gives every junction one height (ground median, then a repair pass for junctions too close together); `profile::solve_profiles` takes those heights as fixed points, so chains no longer inherit each other's drift. `conform` gets level patches at junctions, faces on steep ground and a fair split of the ground between roads close together. Bridges are decided at the graph level: a bridge is a stretch with `bridge = true`, so widening a short OSM bridge and bridging a dip both re-cut stretches and place again.

**Tech Stack:** Rust 1.95, edition 2024; crates `naves-core` (logic), `naves-formats` (world file), `naves-export` (generate, build), `naves` (CLI, GUI, MCP). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-11-13-road-heights-design.md`. Read it first; section numbers below refer to it. Measurements: `docs/research/2026-11-10-road-alignment.md`, section 6.

## Global Constraints

- `AGENTS.md` applies to every task: test first (red, then green), integration tests in `crates/*/tests/`, deterministic, `make check` before every commit, user docs and the `CHANGELOG.md` line in the same commit as a behaviour change. Where this plan and `AGENTS.md` disagree, `AGENTS.md` wins.
- No game data in the repo. Tests use `naves_core::catalog::testing::synthetic_catalog()` (feature `test-util`) and terrains built in the test (`terrain(size, cell, f)` as in `crates/naves-core/tests/profile.rs`).
- Same input, same output: no `HashMap`/`HashSet` in anything that reaches the output, fixed round counts, fixed orders, sums in a fixed order.
- Library code never panics on data: no `unwrap`, `expect`, indexing; checked arithmetic.
- Every Wine or Proton program runs in a headless sway session. **Never start the DayZ client or server**; Task 10 runs binarize only. Large data and scratch builds go under `/var/tmp`, never `/tmp` or `$HOME`.
- Limits, verbatim from the spec: `grav` hard limit 0.40 (ceiling 0.36), `mud` hard limit 0.50 (ceiling 0.45), public roads unchanged at 0.30; aims unchanged. Junction height: median of the original ground on a 1 m grid inside 10 m. Junctions 3 m or less apart are one. Allowance: 30 m up to a side slope of 20 %, linear to 100 m at 60 %. Steep for a face: side slope above 35 %. Dip: more than 3 m above the original ground. Bridge scale at most 1.5. Default `terrain_size` 2048.
- **Measure every task:** after its commit, run `cargo run --release -p naves-export --example road-analysis 1024` and `... 2048` (about 3 minutes each; reads `examples/berchtesgaden/` and the DayZ install read-only, starts no Wine) and add the violation counts per rule and the figures the task names to a new section 7 of `docs/research/2026-11-10-road-alignment.md`, one sub-section per task, with the previous task's figures beside them. A figure that got worse is said so in that sub-section. This measurement commit is `docs:`.
- What counts as done on the example (spec section 9), checked in Task 3 and again in Task 10: no piece with a planned grade above its hard limit that is not reported; no planned grade above 100 % anywhere; every junction within 3 m of its ground median.

## Review Focus

1. **A junction at the edge of the map**, where part of the 10 m circle lies outside: the median is taken over the samples inside, and a junction with none inside takes the height at its point. Test in Task 2.
2. **Two junctions at the same place or a link of zero length:** they are one junction; nothing divides by zero. Test in Task 2.
3. **A ring road whose two ends are the same junction, and a network with no junction at all:** both solve; the ring meets itself at one height. Test in Task 3.
4. **A dip that touches a junction or the end of a dead-end road:** no bridge reaches into a junction or past the end; the dip is filled when dry and reported when wet. Test in Task 9.
5. **An existing project file that names `terrain_size = 1024`** keeps it, and one that names `terrain_size` without `land_size` still gets a valid pair. Test in Task 10.

---

### Task 0: The tree as a committed base

The working tree holds uncommitted work that this plan builds on: the fixes of the code review (seven findings), the measuring tool, the research note and the spec. Nothing new is written in this task.

**Files:** as in `git status`.

- [ ] **Step 1:** Run `make check` (with `TMPDIR=/var/tmp`). Expected: passes.
- [ ] **Step 2:** Commit the review fixes, one commit per finding, each with its tests, docs and changelog line. The groups (use `git add -p` where a file holds two):
  - `fix(core): the ground under a bridge keeps its height` (`conform.rs`, `tests/conform.rs`)
  - `fix(core): a bridge deck takes its height from its abutments` (`profile.rs`, `tests/profile.rs`)
  - `fix(core): overrides reject numbers that are not finite and bad model names` (`objects.rs` validation, `tests/objects.rs`, `fuzz/corpus/overrides_parse/*`, `project-file.md`)
  - `fix(core): roads cut at the map edge keep their identity` (`features.rs`, `tests/features.rs`, `generated_yaw` in `objects.rs`, `tests/roadplace.rs`)
  - `fix(core): a crest or dip that is too sharp is reported as one` (`GradeExcessKind`, `validate.rs`, `tests/validate.rs`, `troubleshooting.md`)
  - `fix(export): the road bed lies under the placed pieces` (`generate.rs` `paint_roads`, `tests/road_bed.rs`, `getting-started.md`)
  - `refactor: index the passes that scanned every road` (the rest of `generate.rs`, `roadplace.rs`, `validate.rs`, `profile.rs`, `roadnet.rs`)
- [ ] **Step 3:** Commit `crates/naves-export/examples/road-analysis.rs` as `test: measure grids, junction heights and track limits`, and the research note and the spec as `docs: road heights design and its measurements`. Leave `crates/naves-export/examples/render-crossfall.rs` untracked; it is not part of this work.
- [ ] **Step 4:** `make check`. Expected: passes.

### Task 1: Track limits

**Files:**
- Modify: `crates/naves-core/src/limits.rs`, `crates/naves-core/tests/limits.rs`
- Modify: `docs/manual/src/getting-started.md` or wherever the limits are described (`grep -rn "30 %" docs/manual/src`), `docs/research/2026-11-10-road-alignment.md` (section 3: a "decided" paragraph like the existing one), `CHANGELOG.md`

**Interfaces:**
- Produces: `drive_grade(Family::Grav) == 0.40`, `drive_grade(Family::Mud) == 0.50`; `grade_ceiling` stays `0.9 * drive_grade`.

- [ ] **Step 1: Failing test** in `tests/limits.rs`:

```rust
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
```

- [ ] **Step 2:** `cargo test -p naves-core --test limits`. Expected: FAIL (0.30 and 0.28).
- [ ] **Step 3:** Change the two values in `drive_grade`; rewrite its doc comment and that of `grade_aim`: gravel tracks are held to a car at the top of its range, dirt tracks to a 4WD (maintainer's decision, 2026-11-13; estimates, not driven in the game).
- [ ] **Step 4:** `cargo test --workspace --all-targets --locked`. Tests that pinned the old values through behaviour (a track that was "not drivable" at 29 %) get the new numbers; say which in the commit message.
- [ ] **Step 5:** Docs, changelog line, `make check`, commit `feat(core): track limits for tracks`. Then the measurement (Global Constraints). Expected on the example at 2048, from the trial: `mud` length cut deeper than 10 m about 2 %, the Klingerweg (`158289388:259090646-278531300`) without a grade above 45 %.

### Task 2: Junction heights from the ground

**Files:**
- Create: `crates/naves-core/src/junctions.rs`, `crates/naves-core/tests/junctions.rs`
- Modify: `crates/naves-core/src/lib.rs` (`pub mod junctions;`), `crates/naves-core/src/profile.rs` (make `rank` `pub(crate)`)

**Interfaces:**
- Consumes: `PlacedNetwork`, `PlacedRoad`, `PlacedKind` (`roadplace`), `RoadGraph` (`roadnet`), `Heightmap`, `limits::grade_ceiling`, `profile::rank`.
- Produces:

```rust
/// Radius of the circle the ground is sampled in.
pub const GROUND_RADIUS_M: f64 = 10.0;
/// Junctions this close along a road are one.
pub const MERGE_M: f64 = 3.0;
/// Rounds of [`repair`].
pub const REPAIR_ROUNDS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JunctionKind { Piece, Join, EndOnRoad }

#[derive(Debug, Clone, PartialEq)]
pub struct Junction {
    pub at: (f64, f64),
    pub kind: JunctionKind,
    /// Median of the original ground.
    pub ground: f64,
    /// The height the roads get; `ground` until `repair` moves it.
    pub height: f64,
    /// Rank of the most important road meeting here.
    pub rank: u8,
    /// Radius of the level patch: half the length of the junction piece, or half the width of the widest
    /// road meeting here.
    pub patch_m: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub a: usize,
    pub b: usize,
    /// Road length from junction edge to junction edge, at least 1 m.
    pub length: f64,
    /// `grade_ceiling` of the road's family.
    pub ceiling: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Junctions {
    pub points: Vec<Junction>,
    pub links: Vec<Link>,
    /// Links still over their ceiling after `repair`, by index into `links`.
    pub unresolved: Vec<usize>,
}

pub fn ground_median(base: &Heightmap, at: (f64, f64), radius: f64) -> f64;
pub fn find_junctions(net: &PlacedNetwork, graph: &RoadGraph, base: &Heightmap) -> Junctions;
pub fn repair(junctions: &mut Junctions);
impl Junctions {
    /// The junction whose patch holds `point`, the nearest if several.
    pub fn at(&self, point: (f64, f64)) -> Option<&Junction>;
}
```

`find_junctions` returns the junctions sorted by position (x, then y, `total_cmp`) and the links sorted by (`a`, `b`, `length`); it calls `repair`. A working reference for finding the three kinds of junction and the links, and for `ground_median` and the repair, is the `#### junction study` block of `crates/naves-export/examples/road-analysis.rs` (functions `dump_graph`, `load_graph` and the study): port it, do not call it. On the example it finds 1269 junctions and 1535 links.

- [ ] **Step 1: Failing tests** in `tests/junctions.rs` (helpers `terrain` and `piece` as in `tests/profile.rs`; build a `PlacedNetwork` and the matching `RoadGraph` by hand, or with `build_graph` + `place_network` on `synthetic_catalog()` as `tests/roadplace.rs` does):

```rust
#[test]
fn the_median_ignores_one_wild_sample() {
    // flat at 100 with one vertex at 500 inside the circle
    let hm = terrain(64, 2.0, |x, y| if (x, y) == (60.0, 60.0) { 500.0 } else { 100.0 });
    assert!((ground_median(&hm, (60.0, 60.0), 10.0) - 100.0).abs() < 0.5);
}

#[test]
fn a_junction_at_the_map_edge_uses_the_samples_inside() {
    let hm = terrain(64, 2.0, |x, _| 100.0 + 0.1 * x);
    let h = ground_median(&hm, (1.0, 60.0), 10.0);
    assert!(h.is_finite() && (100.0..=101.2).contains(&h));
    // nothing inside: the height at the point, clamped to the map
    assert!(ground_median(&hm, (-50.0, 60.0), 10.0).is_finite());
}

#[test]
fn every_junction_starts_on_its_ground() {
    let (net, graph) = t_junction_network(); // an asf3 through road with a city side road
    let hm = terrain(128, 3.125, |x, y| 100.0 + 0.05 * x + 0.02 * y);
    let j = find_junctions(&net, &graph, &hm);
    assert_eq!(j.points.len(), 1);
    assert!((j.points[0].height - j.points[0].ground).abs() < 1e-9);
    assert!(j.unresolved.is_empty());
}

#[test]
fn two_junctions_too_close_for_their_ground_move_toward_each_other() {
    // two junctions 10 m of city road apart (ceiling 0.27), ground 100 and 108
    let mut j = two_junctions(10.0, 0.27, 100.0, 108.0, /* ranks */ 4, 4);
    repair(&mut j);
    let rise = (j.points[1].height - j.points[0].height).abs();
    assert!(rise <= 0.27 * 10.0 + 0.01);
    assert!(j.unresolved.is_empty());
    // equal rank: both moved by the same amount
    assert!(((j.points[0].height - 100.0) - (108.0 - j.points[1].height)).abs() < 0.01);
}

#[test]
fn the_junction_on_the_more_important_road_moves_less() {
    let mut j = two_junctions(10.0, 0.27, 100.0, 108.0, /* asf1 */ 6, /* mud */ 1);
    repair(&mut j);
    assert!((j.points[0].height - 100.0).abs() < (108.0 - j.points[1].height).abs() / 16.0);
}

#[test]
fn junctions_three_metres_apart_are_one() {
    let (net, graph) = two_side_roads_meeting_a_road_at(/* stations */ 50.0, 52.5);
    let hm = terrain(128, 3.125, |_, _| 100.0);
    assert_eq!(find_junctions(&net, &graph, &hm).points.len(), 1);
}

#[test]
fn the_result_does_not_depend_on_the_order_of_the_roads() {
    let (net, graph) = grid_of_roads(); // 3 x 3 roads on a tilted plane
    let hm = terrain(256, 3.125, |x, y| 100.0 + 0.2 * x - 0.1 * y);
    let a = find_junctions(&net, &graph, &hm);
    let b = find_junctions(&reordered(&net), &graph, &hm); // chains and roads in reverse
    assert_eq!(a, b);
}

#[test]
fn a_link_that_cannot_be_repaired_is_reported() {
    // three junctions in a ring of 1 m links with grounds 100, 120, 140: cannot converge to within the ceiling
    // on all three unless they meet; after 50 rounds either all are legal or the rest is listed
    let mut j = ring_of_three(1.0, 0.27, [100.0, 120.0, 140.0]);
    repair(&mut j);
    for (i, link) in j.links.iter().enumerate() {
        let rise = (j.points[link.a].height - j.points[link.b].height).abs();
        assert!(rise <= link.ceiling * link.length + 0.01 || j.unresolved.contains(&i));
    }
}
```

- [ ] **Step 2:** `cargo test -p naves-core --test junctions`. Expected: FAIL (module missing).
- [ ] **Step 3: Implement.** `ground_median`: samples on a 1 m grid inside the circle that lie inside the map, `Heightmap::sample(x, y).height`, sorted with `total_cmp`, the middle one (mean of the two middle ones for an even count). `repair`, the algorithm the tests do not fully determine:

```text
repeat REPAIR_ROUNDS times, links in their stored order:
    excess = |h[a] - h[b]| - ceiling * length
    if excess > 0:
        move = excess + 0.001
        wa = 2^(-rank[a]), wb = 2^(-rank[b])        // mobility: a higher rank moves less
        the higher junction goes down by move * w / (wa + wb), the lower goes up by the other share
unresolved = links with excess > 0.01 after the last round
```

`patch_m`: for `Piece`, half the length of the junction piece (start to end); for `Join` and `EndOnRoad`, half the width of the widest road meeting there. Merging (`MERGE_M`): junction points connected by a link of 3 m or less become one junction at the mean of their points, with the highest rank and the largest patch; their link disappears.
- [ ] **Step 4:** Tests pass; `cargo clippy -p naves-core --all-targets -- -D warnings`.
- [ ] **Step 5:** Commit `feat(core): one height per junction, taken from the ground`. No user-visible change yet (nothing calls it): no changelog line. No measurement.

### Task 3: Roads are tied to the junction heights

**Files:**
- Modify: `crates/naves-core/src/profile.rs`, `crates/naves-core/tests/profile.rs`
- Modify: `crates/naves-core/src/validate.rs`, `crates/naves-core/tests/validate.rs`
- Modify: `crates/naves-export/src/generate.rs` (`place_roads`, `Roads`), `crates/naves-export/examples/road-analysis.rs` (its call of `solve_profiles`)
- Modify: `docs/manual/src/troubleshooting.md`, `docs/manual/src/getting-started.md` (how road heights are decided), `CHANGELOG.md`

**Interfaces:**
- Consumes: `junctions::{Junctions, find_junctions}` (Task 2).
- Produces:
  - `pub fn solve_profiles(net: &PlacedNetwork, hm: &Heightmap, junctions: &Junctions) -> Profiles`
  - `Rule::JunctionTooSteep` with id `"junction_too_steep"` (add to `Rule::ALL`, now 12), message `"Junctions too close for their heights: {rise:.1} m over {length:.0} m"`, one per `junctions.unresolved`, at the midpoint of the two junctions.
  - `check_roads` gets `junctions: &Junctions` after `profiles`.
  - `Roads` in `generate.rs` gets `junctions: Junctions`.

How the heights enter the solver: in `profile_chain`, every variable of the chain whose position lies in a junction's patch (`Junctions::at`) is fixed to that junction's `height`. That covers chain ends (replacing the heights from `end_joins`/`meeting_height`) **and** the junction pieces in the middle of a through chain, so a through chain becomes a sequence of spans between fixed variables; `solve_chain` already takes `fixed_height: &[Option<f64>]` per variable. A chain end that meets another road where `find_junctions` found no junction keeps today's behaviour (tied to the owner's height). `settle` stays, but never moves a variable fixed by a junction. The meeting grade (`MEETING_GRADE`) stays for the asphalt families and `city`; for `grav` and `mud` it is not applied (spec 4.3: it holds the lower end of the Klingerweg 9 m in the ground).

- [ ] **Step 1: Failing tests** in `tests/profile.rs` (update the existing helper calls to pass `&find_junctions(...)` or `&Junctions::default()`):

```rust
#[test]
fn all_roads_at_a_junction_have_its_height() {
    let (net, graph) = t_junction_network();
    let hm = terrain(128, 3.125, |x, y| 100.0 + 0.08 * x + 0.03 * y);
    let j = find_junctions(&net, &graph, &hm);
    let p = solve_profiles(&net, &hm, &j);
    let want = j.points[0].height;
    for strip in &p.strips {
        for joint in strip.joints.iter().filter(|q| near(&j.points[0], (q.0, q.1))) {
            assert!((joint.2 - want).abs() < 0.01, "{} vs {want}", joint.2);
        }
    }
}

#[test]
fn the_through_road_does_not_drag_the_side_road_off_the_ground() {
    // through road along a valley floor that its own limits lift 6 m off the ground at the junction today
    let (net, graph, hm) = side_road_on_a_drifting_through_road();
    let j = find_junctions(&net, &graph, &hm);
    let p = solve_profiles(&net, &hm, &j);
    let side_end = end_of_side_road(&p);
    assert!((side_end.2 - f64::from(hm.sample(side_end.0, side_end.1).height)).abs() < 1.0);
}

#[test]
fn the_order_of_the_chains_does_not_change_the_heights() {
    let (net, graph) = grid_of_roads();
    let hm = terrain(256, 3.125, |x, y| 100.0 + 0.2 * x - 0.1 * y);
    let j = find_junctions(&net, &graph, &hm);
    let a = solve_profiles(&net, &hm, &j);
    let b = solve_profiles(&reordered(&net), &hm, &j);
    assert_eq!(heights_by_slot(&net, &a), heights_by_slot(&reordered(&net), &b)); // within 1e-9
}

#[test]
fn a_ring_meets_itself_at_one_height_and_a_lone_road_still_solves() {
    let (ring, graph) = ring_with_one_junction();
    let hm = terrain(128, 3.125, |x, _| 100.0 + 0.1 * x);
    let j = find_junctions(&ring, &graph, &hm);
    let p = solve_profiles(&ring, &hm, &j);
    let (first, last) = ends_of(&p.strips[0]);
    assert!((first.2 - last.2).abs() < 0.01);
    let lone = lone_road();
    assert_eq!(solve_profiles(&lone, &hm, &Junctions::default()).strips.len(), 1);
}

#[test]
fn no_piece_carries_a_step() {
    // the Klingerweg in small: a mud road climbing 30 % between two junctions on the ground
    let (net, graph, hm) = steep_track_between_two_junctions(225.0, 0.30);
    let p = solve_profiles(&net, &hm, &find_junctions(&net, &graph, &hm));
    for w in p.strips[0].joints.windows(2) {
        let run = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        assert!(((w[1].2 - w[0].2) / run).abs() <= 0.45 + 1e-6);
    }
}
```

and in `tests/validate.rs`: a `Junctions` with one unresolved link gives one violation with rule id `junction_too_steep` and the message format above.
- [ ] **Step 2:** Run them. Expected: FAIL to compile (signature), then fail on heights.
- [ ] **Step 3:** Implement as described under Interfaces. In `generate.rs`: `let junctions = find_junctions(&net, &graph, base);` before `solve_profiles`.
- [ ] **Step 4:** `cargo test --workspace --all-targets --locked`. Existing profile tests that asserted "the side road ends at the through road's height" now assert the junction height; keep their intent, say which changed in the commit message.
- [ ] **Step 5:** Docs (the new rule in `troubleshooting.md`; one paragraph on junction heights in the manual), changelog, `make check`, commit `feat(core): roads meet at the height of their junction`. Measurement, with these figures added: the three "done" criteria of the Global Constraints (add them to `road-analysis` as a `#### done` section: count of pieces with a planned grade above 100 %, count above the hard limit that have no `road_grade` or `road_not_drivable` violation, largest distance of a junction from its ground median), the distance of the planned junction height from the ground median (was p50 1.64 m, p90 6.80 m), and the largest `road_grade` (was 235 % at 2048).

### Task 4: Level patches, faces and roads close together

**Files:**
- Create: `crates/naves-core/src/sideslope.rs`
- Modify: `crates/naves-core/src/lib.rs`, `crates/naves-core/src/profile.rs` (`RoadStrip`), `crates/naves-core/src/conform.rs`, `crates/naves-core/tests/conform.rs`
- Modify: `crates/naves-export/src/generate.rs`, `crates/naves-export/examples/road-analysis.rs`, `docs/manual/src/getting-started.md`, `CHANGELOG.md`

**Interfaces:**
- Consumes: `Junctions` (Task 2).
- Produces:

```rust
// sideslope.rs
/// Half the width over which the side slope is measured.
pub const SIDE_M: f64 = 10.0;
/// Pieces within this distance along the road share a median.
pub const SMOOTH_M: f64 = 25.0;
/// The side slope of the original ground at each road of `net` (rise over run, not negative), smoothed
/// along its chain; in the order of `net.roads`.
pub fn side_slopes(net: &PlacedNetwork, base: &Heightmap) -> Vec<f64>;

// profile.rs, RoadStrip gains
/// For each segment between two joints, the side slope of the original ground there.
pub side_slope: Vec<f64>,

// conform.rs
/// Side slope above which the uphill side of a road may be a face.
pub const FACE_SIDE_SLOPE: f64 = 0.35;
/// Rise over run of a face: steeper than any grid cell can show.
pub const FACE_SLOPE: f64 = 50.0;
pub struct LevelPatch { pub at: (f64, f64), pub radius: f64, pub height: f64, pub rank: u8 }
pub fn conform(base: &Heightmap, strips: &[RoadStrip], patches: &[LevelPatch])
    -> Result<Conformed, ConformError>;
```

`solve_profiles` fills `side_slope` from `side_slopes` (it gets them as a fourth parameter `side: &[f64]`, by road index). `generate.rs` builds the patches from `junctions.points` (`Join` and `EndOnRoad` only; a `Piece` junction is already a level segment of its strip) with `radius = patch_m + one cell`.

Rules for `conform` (spec 5.2, 5.3, 4.3):
1. A patch claims the vertices within its radius at its height, like a core.
2. Cut slope at a core vertex: `FACE_SLOPE` where the strip segment's `side_slope` is above `FACE_SIDE_SLOPE`, otherwise as today. Fill slopes unchanged.
3. A vertex within reach of two cores (two strips, two legs of one strip, or a strip and a patch) whose heights differ by more than `CONFLICT_M`: it goes to the one whose **edge** is nearer (distance to the centre line less `half_width`; for a patch, distance to its point less `radius`), ties to the higher rank. It is a conflict (reported as today) only if it lies within the width of both. Between two such cores the slope is `FACE_SLOPE`.

- [ ] **Step 1: Failing tests** in `tests/conform.rs`:

```rust
#[test]
fn two_roads_side_by_side_at_different_heights_each_keep_their_ground() {
    // two 6 m wide roads, centre lines 12 m apart, at 100 and 105 m, on 3.125 m cells
    let base = terrain(128, 3.125, |_, _| 102.0);
    let strips = [straight_strip(y = 194.0, h = 100.0, half = 3.0, rank = 3),
                  straight_strip(y = 206.0, h = 105.0, half = 3.0, rank = 6)];
    let out = conform(&base, &strips, &[]).unwrap();
    for x in (60..340).step_by(5) {
        for (y, h) in [(194.0, 100.0), (206.0, 105.0)] {
            for dy in [-3.0, 0.0, 3.0] {
                assert!((sample(&out.heightmap, f64::from(x), y + dy) - h).abs() < 0.2);
            }
        }
    }
    assert!(out.conflicts.is_empty());
}

#[test]
fn roads_that_overlap_at_different_heights_are_still_a_conflict() {
    let strips = [straight_strip(y = 200.0, h = 100.0, half = 3.0, rank = 3),
                  straight_strip(y = 202.0, h = 105.0, half = 3.0, rank = 6)];
    assert!(!conform(&flat(102.0), &strips, &[]).unwrap().conflicts.is_empty());
}

#[test]
fn on_a_steep_hillside_the_cut_is_a_face_one_cell_wide() {
    // hillside rising 60 % to the north, road along x at the height of the ground under its centre
    let base = terrain(128, 3.125, |_, y| 100.0 + 0.6 * y);
    let strip = straight_strip_with_side_slope(y = 200.0, h = 220.0, half = 3.0, side = 0.6);
    let out = conform(&base, &[strip], &[]).unwrap();
    // uphill: the original ground is back within two cells of the edge of the level strip
    let edge = 200.0 + 3.0 + 1.5 * 3.125;
    assert!((sample(&out.heightmap, 200.0, edge + 2.0 * 3.125) - (100.0 + 0.6 * (edge + 2.0 * 3.125))).abs() < 0.5);
}

#[test]
fn on_gentle_ground_the_cut_slope_is_as_before() {
    let base = terrain(128, 3.125, |_, y| 100.0 + 0.1 * y);
    let strip = straight_strip_with_side_slope(y = 200.0, h = 115.0, half = 3.0, side = 0.1);
    let out = conform(&base, &[strip], &[]).unwrap();
    assert!(max_slope_beside(&out.heightmap, 200.0) <= CUT_SLOPE_MAX + 0.05);
}

#[test]
fn a_patch_levels_the_ground_at_a_join() {
    let base = terrain(128, 3.125, |x, _| 100.0 + 0.3 * x);
    let patch = LevelPatch { at: (200.0, 200.0), radius: 8.0, height: 160.0, rank: 3 };
    let out = conform(&base, &[], &[patch]).unwrap();
    for (dx, dy) in [(0.0, 0.0), (6.0, 0.0), (-6.0, 0.0), (0.0, 6.0)] {
        assert!((sample(&out.heightmap, 200.0 + dx, 200.0 + dy) - 160.0).abs() < 0.2);
    }
}
```

and in a new `tests/sideslope.rs`: on `100 + 0.4 * y` a road along x has side slope 0.4 and a road along y has 0.0 (within 0.01); one vertex raised by 30 m beside one piece of a ten-piece road changes that piece's smoothed value by less than 0.05.
- [ ] **Step 2:** Run. Expected: FAIL to compile.
- [ ] **Step 3:** Implement. `side_slopes`: at the midpoint of each road, `|h(left 10 m) − h(right 10 m)| / 20`, perpendicular to `start → end`; then for each road the median over the roads of its chain whose midpoints are within `SMOOTH_M` along the chain.
- [ ] **Step 4:** All tests pass, including the bridge-ground tests of Task 0's first commit.
- [ ] **Step 5:** Docs (the manual's sentence that a player can always walk off a road no longer holds on steep ground; say so), changelog, `make check`, commit `feat(core): faces on steep ground, and roads close together keep their ground`. Measurement, with: `road_misses_terrain` above 0.5 m by cause (was 735 at 2048, 483 of them "another road at a different height"), `road_conflict`, `earthworks_capped`, and the share of pieces on side slopes above 35 % with fill over 2 m at the downhill edge (was 38 %).

### Task 5: Reporting a road far from the ground

**Files:**
- Modify: `crates/naves-core/src/limits.rs`, `crates/naves-core/tests/limits.rs`
- Modify: `crates/naves-core/src/checks.rs` (`PieceCheck`, `check_roads_on_terrain`), `crates/naves-core/tests/checks.rs`
- Modify: `crates/naves-core/src/validate.rs`, `crates/naves-core/tests/validate.rs`
- Modify: callers in `crates/naves-export/src/generate.rs`, `crates/naves/src` (GUI and MCP, if they show `PieceCheck`), `road-analysis.rs`
- Modify: `docs/manual/src/troubleshooting.md`, `docs/manual/src/mcp.md` (the list of rules), `CHANGELOG.md`

**Interfaces:**
- Consumes: `sideslope::side_slopes` (Task 4).
- Produces:
  - `pub fn earthwork_allowance_m(side_slope: f64) -> f64` in `limits.rs`: 30.0 up to 0.20, linear to 100.0 at 0.60, 100.0 above; a NaN slope gives 30.0.
  - `PieceCheck` gains `pub off_ground_m: f64` (planned height at the piece's middle less the original ground there; negative is a cut) and `pub side_slope: f64`.
  - `pub fn check_roads_on_terrain(net: &PlacedNetwork, hm: &Heightmap, base: &Heightmap, side: &[f64]) -> Vec<PieceCheck>`
  - `Rule::RoadFarFromGround`, id `"road_far_from_ground"`, messages `"Cut {depth:.0} m deep; {allowance:.0} m fit this ground"` and `"Embankment {height:.0} m high; {allowance:.0} m fit this ground"`.

- [ ] **Step 1: Failing tests:**

```rust
// tests/limits.rs
#[test]
fn the_allowance_grows_with_the_side_slope() {
    for (slope, want) in [(0.0, 30.0), (0.20, 30.0), (0.40, 65.0), (0.60, 100.0), (2.0, 100.0)] {
        assert!((earthwork_allowance_m(slope) - want).abs() < 1e-9);
    }
    assert!((earthwork_allowance_m(f64::NAN) - 30.0).abs() < 1e-9);
}
// tests/validate.rs
#[test]
fn a_deep_cut_on_flat_ground_is_reported_and_the_same_cut_on_a_mountainside_is_not() {
    let flat = piece_check(off_ground_m = -40.0, side_slope = 0.05);
    let steep = piece_check(off_ground_m = -40.0, side_slope = 0.60);
    let v = violations_of(&[flat, steep]);
    let far: Vec<_> = v.iter().filter(|x| x.rule == Rule::RoadFarFromGround).collect();
    assert_eq!(far.len(), 1);
    assert_eq!(far[0].message, "Cut 40 m deep; 30 m fit this ground");
}
```

and in `tests/checks.rs`: a road 5 m below a flat base reports `off_ground_m` within 0.1 of −5.0; a bridge piece reports 0.0 (a deck is not measured against the ground).
- [ ] **Step 2 to 4:** red, implement, green.
- [ ] **Step 5:** Docs, changelog, `make check`, commit `feat(core): report a road that is far from the ground for its terrain`. Measurement, with the count of the new rule (expected from the trial: none on tracks; four cuts over 30 m on public roads before Task 3, to be seen after it).

### Task 6: Bridge choice and scale

**Files:**
- Modify: `crates/naves-core/src/roadplace.rs` (`PlacedRoad`, `choose_bridge`, `place_bridge`), `crates/naves-core/tests/roadplace.rs`
- Modify: `crates/naves-core/src/objects.rs` (`GeneratedObject`, `PlacedObject`, `road_objects`, `apply_overrides`, `piece_transform`, `bridge_object_height`), `crates/naves-core/tests/objects.rs`
- Modify: `crates/naves-core/src/catalog.rs` (`testing`: a 12 m paved model and a wooden model in the synthetic catalog, if missing)
- Modify: `crates/naves-export/src/generate.rs` (`object_heights`, `deck_levels`), `crates/naves-export/src/build.rs` (`world_objects`), `crates/naves-export/tests/bridge.rs`
- Modify: `docs/formats/road-objects.md`, `docs/manual/src/getting-started.md`, `CHANGELOG.md`

**Interfaces:**
- Produces:
  - `pub const MAX_BRIDGE_SCALE: f64 = 1.5;` in `roadplace.rs`.
  - `PlacedRoad` gains `pub scale: f64` (1.0 for everything but a scaled bridge); `GeneratedObject` and `PlacedObject` gain `pub scale: f64`.
  - `pub fn piece_transform(object: &PlacedObject, ground_height: f32) -> [f32; 12]` multiplies the nine rotation entries by `object.scale`.
  - `pub fn bridge_object_height(deck_level: f64, model: &BridgeModel, scale: f64) -> f64` = `deck_level − scale * model.deck_height`.
  - Choice: wooden models (model path contains `wood`, as the existing code tells them apart) for `Grav` and `Mud`, the others for `Asf1`..`City`; among them the narrowest with `width >= road width`; if none, the widest with `scale = min(road width / width, MAX_BRIDGE_SCALE)` and the existing warning only when the scale was capped. The scaled length (`scale * length`) is what the bridge run is laid with.
  - An orphaned or replaced object has scale 1.0; a moved bridge keeps its scale.

- [ ] **Step 1: Failing tests:**

```rust
// tests/roadplace.rs
#[test]
fn a_paved_road_gets_the_narrowest_bridge_that_is_wide_enough() {
    let net = place(bridge_stretch(Family::Asf1 /* 12 m */, 60.0));
    let b = bridges(&net);
    assert!(b.iter().all(|r| r.width >= 12.0 && (r.scale - 1.0).abs() < 1e-12));
    assert!(net.warnings.is_empty());
}
#[test]
fn a_bridge_that_is_too_narrow_is_scaled_up_and_not_down() {
    // catalog with only a 9 m paved model; a 12 m road, then a 6 m road
    let wide = place_with(only_stone_9m(), bridge_stretch(Family::Asf1, 60.0));
    assert!(bridges(&wide).iter().all(|r| (r.scale - 12.0 / 9.0).abs() < 1e-9));
    let narrow = place_with(only_stone_9m(), bridge_stretch(Family::Asf3, 60.0));
    assert!(bridges(&narrow).iter().all(|r| (r.scale - 1.0).abs() < 1e-12));
}
#[test]
fn the_scale_stops_at_one_and_a_half_and_says_so() {
    let net = place_with(only_wood_5m(), bridge_stretch(Family::Grav /* 8 m */, 60.0));
    assert!(bridges(&net).iter().all(|r| (r.scale - 1.5).abs() < 1e-12));
    assert_eq!(net.warnings.len(), 1);
}
#[test]
fn tracks_get_wooden_bridges_and_paved_roads_do_not() { /* model names by family */ }
#[test]
fn scaled_bridges_still_meet_end_to_end() {
    // consecutive bridge pieces: end of one is the start of the next within 1 cm
}
// tests/objects.rs
#[test]
fn the_transform_carries_the_scale() {
    let o = PlacedObject { id: None, model: "b".into(), x: 10.0, y: 20.0, yaw: 0.0, scale: 1.25 };
    let t = piece_transform(&o, 5.0);
    assert_eq!([t[0], t[4], t[8]], [1.25, 1.25, 1.25]);
    assert_eq!([t[9], t[10], t[11]], [10.0, 5.0, 20.0]);
}
#[test]
fn a_scaled_bridge_stands_lower_by_its_scaled_deck_height() {
    assert!((bridge_object_height(100.0, &model_with_deck(8.0), 1.25) - 90.0).abs() < 1e-9);
}
```

- [ ] **Step 2 to 4:** red, implement, green. `overrides.toml` does not store the scale (it is derived); `tests/objects.rs` gets a test that a moved bridge keeps the generated scale.
- [ ] **Step 5:** Docs: `road-objects.md` gets "scale is written into the rotation; **not tested in the game**" under Part 2; changelog; `make check`; commit `feat: bridges by surface, scaled up to the road's width`. Measurement, with the count of bridge objects, how many are scaled and the count of width warnings (was 15).

### Task 7: Every OSM bridge is a bridge

**Files:**
- Modify: `crates/naves-core/src/roadnet.rs`, `crates/naves-core/tests/roadnet.rs`
- Modify: `crates/naves-core/src/roadplace.rs` (remove `MIN_BRIDGE_M`), `crates/naves-core/tests/roadplace.rs`
- Modify: `crates/naves-export/src/generate.rs` (`place_roads`), `docs/manual/src/getting-started.md`, `CHANGELOG.md`

**Interfaces:**
- Consumes: the bridge choice of Task 6 (the scaled length of the model a stretch will get). Move the choice into a function both can call: `pub fn bridge_for(catalog: &PieceCatalog, family: Family) -> Option<(&BridgeModel, f64 /* scale */, bool /* capped */)>` in `roadplace.rs`.
- Produces, in `roadnet.rs`:

```rust
/// A part of a stretch that is to be a bridge, in metres along the stretch from its start.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeSpan { pub stretch: usize, pub from_m: f64, pub to_m: f64 }

/// The graph with the spans cut out of their stretches as bridge stretches. The part before the first
/// span keeps the stretch's id; a span is `<id>/bridge<n>` and the road after it `<id>/after<n>`, `n`
/// counted from the stretch's start. A span that reaches a crossing with three or more stretches, or the
/// end of a dead end, is shortened to stop 1 m before it. Overlapping spans are merged.
pub fn with_bridge_spans(graph: &RoadGraph, spans: &[BridgeSpan]) -> RoadGraph;

/// For each bridge stretch shorter than `needed(stretch)`, the same stretch grown evenly at both ends into
/// the stretches that continue it (crossings of exactly two stretches), as far as they allow.
pub fn widen_bridges(graph: &RoadGraph, needed: &dyn Fn(&Stretch) -> f64) -> RoadGraph;
```

`place_roads` calls `widen_bridges(&graph, &|s| scaled model length for s.family)` before `place_network`. A bridge that cannot be widened to one model length (a junction on each side) stays an ordinary road and gets one line in `net.warnings`: `"bridge on {id}: no room for a bridge model between the junctions"`.

- [ ] **Step 1: Failing tests:**

```rust
// tests/roadnet.rs
#[test]
fn a_span_becomes_a_bridge_stretch_with_a_stable_id() {
    let g = graph_of(&[road("1", &[(0.0, 0.0), (200.0, 0.0)])]);
    let id = g.stretches[0].id.clone();
    let out = with_bridge_spans(&g, &[BridgeSpan { stretch: 0, from_m: 80.0, to_m: 110.0 }]);
    let ids: Vec<_> = out.stretches.iter().map(|s| (s.id.as_str(), s.bridge)).collect();
    assert!(ids.contains(&(id.as_str(), false)));
    assert!(ids.contains(&(format!("{id}/bridge0").as_str(), true)));
    assert!(ids.contains(&(format!("{id}/after0").as_str(), false)));
    assert!((length_of(&out, &format!("{id}/bridge0")) - 30.0).abs() < 1e-6);
}
#[test]
fn a_second_span_does_not_rename_the_first() { /* add a span at 150..170: bridge0 and after0 keep their ids and points up to 150 */ }
#[test]
fn a_span_stops_short_of_a_junction_and_of_a_dead_end() { /* from_m = -5, to_m beyond a 3-way crossing */ }
#[test]
fn a_short_osm_bridge_grows_into_its_neighbours() {
    // road 100 m, bridge 8 m, road 100 m, in one line; needed 25 m
    let out = widen_bridges(&g, &|_| 25.0);
    assert!((length_of_bridge(&out) - 25.0).abs() < 0.01);
    assert!((total_length(&out) - 208.0).abs() < 0.01);
    assert_eq!(ids(&out), ids(&g)); // the same stretches, the same ids
}
#[test]
fn a_bridge_between_two_junctions_grows_only_as_far_as_they_allow() {}
// tests/roadplace.rs
#[test]
fn an_eight_metre_osm_bridge_is_built_as_a_bridge() {
    let net = place(widen(road_bridge_road(100.0, 8.0, 100.0)));
    assert_eq!(count(&net, PlacedKind::Bridge), 1);
}
```

- [ ] **Step 2 to 4:** red, implement, green. Stretch ids contain node ids; `widen_bridges` moves points, not nodes, so ids do not change.
- [ ] **Step 5:** Docs, changelog, `make check`, commit `feat(core): every bridge in OpenStreetMap is a bridge in the game`. Measurement, with: OSM bridge stretches, how many are built as bridges (before and after), how many have no room.

### Task 8: Streams and rivers

**Files:**
- Modify: `crates/naves-core/src/osm.rs` (`WAY_TAGS`, the filter that keeps ways, `OsmWays`), `crates/naves-core/tests/osm.rs`
- Modify: `crates/naves-core/src/features.rs`, `crates/naves-core/tests/features.rs`
- Modify: `fuzz/corpus/osm_ways/` (a seed with a waterway), `tests/fixtures/README.md` if a fixture is added (prefer building the bytes in the test, as the existing synthetic OSM tests do)

**Interfaces:**
- Produces:
  - `read_ways` and `read_ways_from_bytes` also keep ways tagged `waterway=stream`, `river` or `canal` (not `ditch`, not `drain`); `"waterway"` joins `WAY_TAGS`.
  - `pub fn water_lines(ways: &OsmWays, frame: &MapGeo, map_size_m: f64) -> Vec<Vec<(f64, f64)>>` in `features.rs`: those ways in map metres, clipped to the map like roads, sorted by way id. A way that is a tunnel (`tunnel=*` other than `no`, a culvert) is left out: the water is not at the surface.
  - `classify` is unchanged: a way without `highway` is not a road.

- [ ] **Step 1: Failing tests:** a PBF with one `highway=track`, one `waterway=stream`, one `waterway=ditch`, one `waterway=stream` + `tunnel=culvert`: `road_features` returns one feature, `water_lines` returns one line with the stream's points in map metres; a way with both `highway` and `waterway` is a road and not a water line.
- [ ] **Step 2 to 4:** red, implement, green; the fuzz seed runs in `tests/fuzz_seeds.rs`.
- [ ] **Step 5:** `make check`, commit `feat(core): read streams and rivers from OpenStreetMap`. No user-visible change yet: no changelog line, no measurement.

### Task 9: Dips

**Files:**
- Create: `crates/naves-core/src/dips.rs`, `crates/naves-core/tests/dips.rs`
- Modify: `crates/naves-core/src/lib.rs`, `crates/naves-core/src/validate.rs`, `crates/naves-core/tests/validate.rs`
- Modify: `crates/naves-export/src/generate.rs` (`place_roads`, `read_features` to keep the water lines), `crates/naves-export/tests/build.rs` or a new `crates/naves-export/tests/dips.rs`
- Modify: `docs/manual/src/getting-started.md`, `docs/manual/src/troubleshooting.md`, `docs/manual/src/mcp.md`, `CHANGELOG.md`

**Interfaces:**
- Consumes: `BridgeSpan`, `with_bridge_spans` (Task 7), `bridge_for` (Task 7), `water_lines` (Task 8), `Junctions` (Task 2), `Profiles`.
- Produces:

```rust
/// A road this far above the original ground is crossing a dip.
pub const DIP_M: f64 = 3.0;
/// The road may leave the straight line of a bridge model by this much.
pub const BRIDGE_STRAIGHT_M: f64 = 2.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Dip {
    /// Index into `RoadGraph::stretches`.
    pub stretch: usize,
    pub from_m: f64,
    pub to_m: f64,
    /// The most the road stands above the ground in the run.
    pub depth_m: f64,
    pub over_water: bool,
    /// Middle of the run.
    pub at: (f64, f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DipAction { Bridge, Fill, Report }

pub fn find_dips(graph: &RoadGraph, net: &PlacedNetwork, profiles: &Profiles, base: &Heightmap,
                 water: &[Vec<(f64, f64)>], lakes: &[LandPolygon], frame: &MapGeo) -> Vec<Dip>;
/// `model_length` is the scaled length of the bridge model the road would get, if any.
pub fn decide(dip: &Dip, graph: &RoadGraph, model_length: Option<f64>) -> (DipAction, Option<BridgeSpan>);
```

  - `Rule::RoadOverWater`, id `"road_over_water"`, message `"Road crosses water {depth:.0} m below it without a bridge; look at it"`.
  - `check_roads` gets `dips: &[(Dip, DipAction)]`.

`decide` (spec 6.3): `Bridge` when a model exists, the run is at least half `model_length` long, the span grown to a whole number of models (evenly at both ends) stays 1 m clear of every crossing of three or more stretches and of a dead end, and within every model length of the span the stretch's points are within `BRIDGE_STRAIGHT_M` of the chord. Otherwise `Fill` when not `over_water`, otherwise `Report`. Stretches that are already bridges or tunnels have no dips. In `place_roads`, the fixed order of spec 6.3:

```text
graph0 = widen_bridges(build_graph(features))
net0 = place_network(graph0); junctions0; profiles0
dips = find_dips(...); actions = decide(each)
if any Bridge:
    graph = with_bridge_spans(graph0, spans); net = place_network(graph); junctions; profiles   // once, no further rounds
dips still standing in the air over water after that are the reports
```

Placing takes most of the build's time (about 170 s on the example), and this doubles it when a dip gets a bridge. That is accepted for now; say so in the changelog line. Do not add a cache in this task.

- [ ] **Step 1: Failing tests** in `tests/dips.rs`:

```rust
#[test]
fn a_road_over_a_hollow_has_a_dip_as_long_as_the_hollow() {
    // flat at 100 with a 40 m wide, 8 m deep hollow across the road at x = 200
    let d = dips_on(hollow(200.0, 40.0, 8.0), straight_road(Family::Asf3, 400.0));
    assert_eq!(d.len(), 1);
    assert!((d[0].depth_m - 8.0).abs() < 1.0 && !d[0].over_water);
    assert!((d[0].to_m - d[0].from_m) > 20.0 && (d[0].to_m - d[0].from_m) < 45.0);
}
#[test]
fn a_hollow_of_two_metres_is_not_a_dip() {}
#[test]
fn a_dip_with_room_gets_a_bridge_of_whole_models() {
    let (action, span) = decide(&dip(180.0, 220.0, false), &graph, Some(25.0));
    assert_eq!(action, DipAction::Bridge);
    let span = span.unwrap();
    assert!(((span.to_m - span.from_m) / 25.0 - 2.0).abs() < 1e-9); // 40 m needs two models
    assert!(((span.from_m + span.to_m) / 2.0 - 200.0).abs() < 1e-9);
}
#[test]
fn a_long_dip_gets_a_chain() { /* 80 m run, 25 m model: four models */ }
#[test]
fn a_short_dry_dip_is_filled_and_a_short_wet_one_is_reported() {
    assert_eq!(decide(&dip(195.0, 205.0, false), &graph, Some(25.0)).0, DipAction::Fill);
    assert_eq!(decide(&dip(195.0, 205.0, true), &graph, Some(25.0)).0, DipAction::Report);
}
#[test]
fn a_dip_at_a_junction_or_at_the_end_of_a_dead_end_gets_no_bridge() {
    // run from 2 m to 30 m of a stretch that starts at a three-way crossing; and the last 20 m of a dead end
    assert_eq!(decide(&dip(2.0, 30.0, false), &graph_starting_at_a_junction, Some(25.0)).0, DipAction::Fill);
    assert_eq!(decide(&dip(180.0, 200.0, true), &dead_end_200m, Some(25.0)).0, DipAction::Report);
}
#[test]
fn a_dip_in_a_bend_gets_no_bridge() { /* arc of radius 30 m: more than 2 m off the chord of a 25 m model */ }
#[test]
fn a_stream_under_the_run_makes_it_wet_and_one_beside_it_does_not() {}
#[test]
fn no_model_means_fill_or_report() { /* model_length = None */ }
```

and one end-to-end test in `naves-export` with the synthetic catalog: a road over a 40 m hollow yields bridge objects whose ids contain `/bridge0`, the ground under them is untouched, and a second `generate`-level run gives byte-identical objects and heights.
- [ ] **Step 2 to 4:** red, implement, green.
- [ ] **Step 5:** Docs (how dips are handled; the new rule; that the build takes longer when it bridges a dip), changelog, `make check`, commit `feat: bridges over dips, fill where dry, a report over water`. Measurement, with: dips found, by action; bridge objects; `road_over_water`; share of pieces with fill over 2 m and over 5 m (were 25 to 38 % and 7 to 19 % by slope band); `earthworks_capped`.

### Task 10: The grid: 2048 by default, and the example through binarize

**Files:**
- Modify: `crates/naves-core/src/project.rs` (the default `Grid`), `crates/naves-core/tests/project.rs`
- Modify: `examples/berchtesgaden/naves.toml` (`terrain_size = 2048`, `land_size = 256`), `examples/berchtesgaden/README.md`
- Modify: `docs/manual/src/project-file.md` (the example at line 28 and the table row at line 69), `docs/manual/src/getting-started.md`, `docs/manual/src/troubleshooting.md`, `CHANGELOG.md`
- Modify: any test or CLI text that names 1024 as the default (`grep -rn 1024 crates/naves/src crates/*/tests docs/manual/src`)

**Interfaces:**
- Produces: a new project has `terrain_size = 2048` and `land_size = 2048 / LAND_RATIO`; a project file that names a size keeps it.

- [ ] **Step 1: Failing tests** in `tests/project.rs`:

```rust
#[test]
fn a_new_project_has_a_grid_of_2048() {
    let p = Project::new("x", 47.585, 12.965, 12800.0);
    assert_eq!((p.grid.terrain_size, p.grid.land_size), (2048, 256));
}
#[test]
fn a_project_file_that_names_1024_keeps_it() {
    let p = Project::parse(&toml_with_grid("terrain_size = 1024\nland_size = 128")).unwrap();
    assert_eq!((p.grid.terrain_size, p.grid.land_size), (1024, 128));
}
#[test]
fn a_project_file_that_names_only_the_terrain_size_gets_a_fitting_land_size_or_a_clear_error() {
    // whichever the parser does today for a missing land_size: pin it; it must not pair 1024 with 256
}
```

- [ ] **Step 2 to 4:** red, change the default, green.
- [ ] **Step 5: The example through binarize, at 2048.** No game. Copy the project to `/var/tmp/naves-2048.XXXXXX/project` (copy `naves.toml`, symlink `dem`, `landcover`, `osm`, `imagery`, `navmesh`, `secrets`), and run `naves build <copy>` with the release binary **inside a headless sway session** (a sway config whose `exec` line runs the build with output to a log and ends with `swaymsg exit`; started with `env -u WAYLAND_DISPLAY -u DISPLAY -u SWAYSOCK WLR_BACKENDS=headless sway -c <config>`), under `timeout` a little above `NAVES_BINARIZE_TIMEOUT` and `systemd-run --user --scope -p MemoryMax=32G -p MemorySwapMax=0`. Record: exit status, elapsed time, peak memory, PBO size, the counts per rule from `build/report.json`, binarize's "Cannot open object" count, and whether the output world holds the road and bridge model names. Afterwards `pgrep` shows no Wine, Proton or binarize process of the run.
  - If it passes: write the figures into `examples/berchtesgaden/README.md` (the cost table and a "results with roads" section) and the research note.
  - If binarize fails or does not finish at 2048: **stop and report to the maintainer**; revert the default and the example to 1024 in this task (spec section 7) and record why.
- [ ] **Step 6:** Check the "done" criteria of the Global Constraints on the 2048 report. Manual: the default, that existing projects keep their size, that roads have **not been tested in the game** (binarize finishes without the road models; draping, scaled bridges and vehicle limits are unverified). Changelog. `make check`. Commit `feat: a terrain grid of 2048 by default`.

---

## After the plan

Open until the maintainer can supervise a game run (spec section 9), not part of any task: the road objects in the game, a scaled bridge under a vehicle, the vehicles' grades, a face beside a road. The plan-view faults of placing (332 stretch ends, 737 crossings without a junction piece) and rocks over streams need their own designs.

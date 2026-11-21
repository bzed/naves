<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Road Planning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Re-route every curve the road pieces cannot build as a chain of arcs and straights no tighter than the
family's minimum radius, between two anchors on the same road, across the terrain, clear of water and other roads;
keep and report every bend that cannot be planned.

**Architecture:** A new module `crates/naves-core/src/reroute.rs` with sub-modules `raster` (obstacles and distance
maps), `band` (feasibility band), `field` (cost-to-go), `dubins` and `search` (the arc search for one leg). The top
module finds the tight zones on the smoothing's chains, makes anchors and legs, searches each leg in parallel, writes
the routes back, cross-checks them and reports. `naves-export`'s `generate::fitted_graph_with` calls it between
`netclean::clean` and the smoothing, behind a switch, with the four stage combinations selectable per call.

**Tech Stack:** Rust 1.99; `rayon` (already a dependency) for zones in parallel. No new dependency is planned. Task 2
checks crates first, as AGENTS.md demands: `pathfinding` (Dijkstra for the field), `dubins_paths` (Dubins words),
`line_drawing` (supercover grid traversal), the distance transform of `imageproc` (too heavy for one function is the
likely verdict). What the repo already has comes first (`smooth::resample`, `smooth::radius_at`,
`roadnet::polyline_length`, `validate::water_test`, `conform::core_reach_m`).

**Spec:** `docs/superpowers/specs/2026-11-21-road-planning-design.md`

**Method source:** `docs/research/2026-11-21-road-tools-clean-room-notes.md`, sections 3.2 to 3.6. Implementers read the
notes and the spec, never third-party code.

**Implementers:** Sonnet. One Opus helper is allowed for the search: it may prototype the search in Task 1 and is
continued (same agent) for Task 5. **Reviews:** Opus, after every task. Steps marked **(DeepSeek)** are simple and
fully specified; the controller may delegate them to a cheaper model.

## Global Constraints

- AGENTS.md binds: test first (RED then GREEN), `make check` green before every commit, coverage not below the value
  on the base commit, no panics on input (`unwrap`, `expect`, indexing on untrusted data are denied in library code),
  `#[expect(lint, reason)]` never `#[allow]`, no truncating or sign-changing casts, checked arithmetic on sizes, sort
  and compare floats with `total_cmp`, no `==` on floats, deterministic output (no `HashMap` or `HashSet` iteration
  order), `thiserror` errors in the library, `tracing` not `println!` outside the binary and examples, docs and
  CHANGELOG in the behaviour commit, no game data in the repo (tests use synthetic graphs and heightmaps).
- Every process gets a hard memory limit: `systemd-run --user --scope --quiet -p MemoryMax=12G -p MemorySwapMax=1G`
  for cargo and `make check`, 16G for `road-analysis` and the spike, server 8G, client 14G, `ingame-check` 24G.
  Never raise a cap; report an OOM kill. The sum of what parallel agents run must fit the machine (24 threads,
  60 GB): check `nproc`, `free`, `uptime` before starting a long run.
- Temp and build folders under `/var/tmp` (never `/tmp`, never `$HOME`). Shared target dir `/var/tmp/naves-target`,
  never deleted; after a worktree agent, touch the sources before `make check`.
- Never start the game outside the checkpoint task. Wine/Proton only inside headless sway. Never touch the user's
  DayZ profile folder or client settings. Non-default server ports. Remove `@naves*` symlinks from the DayZ install
  afterwards. One game at a time.
- Figures are quoted for Berchtesgaden at half scale (`scale.horizontal = 0.5`), terrain size 2048 (4 m cells).
- Constants, verbatim from the spec (all `pub const` in `reroute.rs`, collected in `RerouteParams::default()`; the
  ones marked **(spike)** are replaced by Task 1's answer before Task 2 starts, and the spec's table is updated in
  the same commit):
  `REROUTE_ENABLED = false`, `PLAN_LEAD = 4.0` (minimum radii, **spike**), `PLAN_RADIUS_MARGIN = 1.05`,
  `PLAN_REACHED_FRACTION = 0.99`, `PLAN_STEP_M = 4.0` (**spike**), `PLAN_MAX_TURN_DEG = 45.0`,
  `PLAN_HEADING_BINS_MIN = 32`, `PLAN_HEADING_BINS_MAX = 180`, `PLAN_ARRIVE_REACH = max(3·s, 4·R)` (**spike**),
  `PLAN_JUNCTION_STRAIGHT_M = 6.25` (**spike**), `PLAN_BRIDGE_HOLD_M = 25.0`, `PLAN_STRAY_MAX = 3.0` (minimum radii,
  **spike**), `PLAN_CUT_M = 6.0`, `PLAN_FILL_M = 6.0` (**spike**), `W_SLOPE = 4.0`, `W_CURVE = 0.1`,
  `W_EARTH = 0.5` (**spike**), `W_STRAY = 0.1` (**spike**), `PLAN_BUDGETS = [(1.0, 20_000), (1.5, 40_000),
  (3.0, 80_000)]` (**spike**), `PLAN_MAX_ZONE_M = 400.0` (**spike**), `PLAN_RASTER_MAX_CELLS = 4_000_000`,
  `PLAN_FIELD_MAX_CELLS = 1_000_000`, `CHAMFER_ERROR = 1.0824`. Reused, not copied: `smooth::SAMPLE_M` (2.0),
  `smooth::CHAIN_TURN_MAX_DEG` (150.0), `limits::grade_ceiling`, `conform::core_reach_m`.
- Stretch ids and node ids never change. No stretch is added or removed. Junction nodes (and their arm directions),
  dead ends, bridge and tunnel end nodes, bridge stretches and the 25 m beside each bridge or tunnel end never move.
- A zone that is not planned keeps its original line bit for bit; one zone's failure changes nothing else.
- The stage switched off returns the graph unchanged, bit for bit. Byte-identical graphs across runs and thread
  counts. Budgets are counts of states, never wall time.
- Every planned or kept zone is reported; nothing moves silently.

## Review Focus

These are the inputs most likely to bite. Each has its test in the task named.

1. **Anchors that coincide or lie within each other's radius.** A tight kink on a 6 m stretch between two junctions;
   two anchors 5 m apart with opposite headings; a junction straight that reaches past the other anchor. Expected:
   `held` or a valid route, never a panic, never a route that violates the radius. Tests in Task 5 (`Degenerate`,
   5 m straight) and Task 6 (6 m stretch between junctions).
2. **A chain that is a ring.** A closed road with a tight bend at its lowest node (the cut) and one elsewhere.
   Expected: the bend at the cut is `held`; the other is planned and the ring stays closed (first point equals last
   point, bit for bit). Test in Task 6.
3. **Non-finite or huge coordinates.** NaN in one stretch, a stretch at 10⁹ m. Expected: the bad chain is skipped with
   a reason, every other zone is planned as without it, raster and field sizes stay under their caps. Tests in Task 2
   (raster cap, NaN segment), Task 5 (`NotFinite`), Task 6 (NaN stretch, 10⁹ m stretch).
4. **A zone at the map edge.** A corner 5 m from the edge whose bulge would cross it. Expected: no written point within
   `core_reach_m(own half)` of the edge or outside the map; the zone is planned inward or kept. Tests in Task 2 (edge
   cells) and Task 6.
5. **A search that exhausts its budget on every zone of a town.** Fifty tight town corners with a tiny budget.
   Expected: all kept as `no_route` with `budget_exhausted`, the total of expanded states bounded by
   `zones · (Σ budgets + 2 · last budget)`, the same report on every run. Test in Task 7.
6. **A switchback whose legs are the same chain.** The route of one zone must not run into its own road outside the
   zone (the other leg). Test in Task 6.

---

### Task 1: Spike (Sonnet; the Opus helper may prototype the search; no production code)

Throwaway code that answers the questions below on real zones. It decides the step set, the weights and the budgets
that the later tasks write in.

**Files:** none in the repo. A scratch worktree under `/var/tmp/naves-plan/spike/` (its own target dir
`/var/tmp/naves-plan/spike-target`), an example `crates/naves-export/examples/reroute-spike.rs` there only. Output
under `/var/tmp/naves-plan/spike/out/`. Nothing is committed; the spike's code is deleted when Task 2 starts.

- [ ] **Step 1: Zones.** On `main` (log `git rev-parse HEAD`), build the cleaned graph of the example at 2048 as
  `fitted_graph_with` does up to `netclean::clean`. Find the zones by the spec's rule (smoothing chains, 2 m resample,
  tight runs, `PLAN_LEAD` 4 radii). Write `zones.tsv`: id, position, family, length, smallest radius, summed turn,
  anchor kinds, distance to the nearest junction, to water, to another road. Count them (expected near, not equal to,
  873).
- [ ] **Step 2: Pick six sites** from `zones.tsv`, with their coordinates in `sites.md`: two switchbacks (summed turn
  over 150°, no junction within the lead, open country), two town corners between junctions (City, turn 60 to 120°,
  junction within the lead at both ends), one hairpin held by a junction (tight run within 10 m of a junction), one
  bend beside water (water within 15 m of the tight run).
- [ ] **Step 3: Prototype** the spec's search (raster, band, field, five-arc steps, Dubins arrival, cost, key,
  budgets) in the example, as simply as possible.
- [ ] **Step 4: Measure** each site for every combination of `PLAN_STEP_M` ∈ {2, 4, 6}, `W_STRAY` ∈
  {0.05, 0.1, 0.25, 0.5}, `W_EARTH` ∈ {0.25, 0.5, 1.0}, cut = fill ∈ {3, 6, 12}, `PLAN_STRAY_MAX` ∈ {2, 3, 4}: route
  found (and at which weight), radius reached, length before and after, largest distance from the original line,
  earthwork (m²), climb, states expanded to the first arrival at weight 1, states in total, time. Also: whether the
  Dubins arrival succeeds near junction anchors; how many routes cross a stream the original line does not cross
  (`read.water`).
- [ ] **Step 5: All zones once** with the chosen values: route found per class (open country, town, junction-held,
  water), states per zone (median, p95, max), time on 1 and on 16 threads, extrapolated to the map.
- [ ] **Step 6: Write `answers.md`**, one line per question, each with the number it rests on:
  1. Step length and the five-arc step set: enough, or are ±θ/4 steps needed?
  2. `W_STRAY` by the spec's rule (largest weight that keeps every route found at 0.05, length within +10 %).
  3. `W_EARTH`, cut and fill, `PLAN_STRAY_MAX`: the smallest values with which the six sites are as good as with the
     largest.
  4. `PLAN_BUDGETS`: four times the largest count to the first arrival at weight 1, rounded up to a round number, and
     the two weighted pairs at twice and four times that.
  5. `PLAN_LEAD`, `PLAN_ARRIVE_REACH`, `PLAN_JUNCTION_STRAIGHT_M`, `PLAN_MAX_ZONE_M`: kept or changed.
  6. Streams: how many routes cross a new stream; recommend the open point's answer.
  7. Projected planning time for the map at the machine's threads.
- [ ] **Step 7: Decide what happens to the plan** (controller, with the Opus reviewer), by these rules:
  - **Town corners between junctions cannot be planned** (no route at any setting within `PLAN_STRAY_MAX` 4): v1
    covers bends whose anchors are not both junction anchors. Add `PLAN_TOWN_CORNERS = false`; such zones are not
    searched, kept as `held`, candidate `plate`. Task 6 gets the test for it. The rest of the plan is unchanged.
  - **Switchbacks need more than 4 minimum radii of stray:** they stay plate candidates; the cap stays at most 4.
  - **The median zone needs more than 50 000 states at weight 1, or the projected time is over 120 s on 16
    threads:** try a coarser key (cells of `s`, θ/2 bins) and three successors; if still over, v1 covers zones
    shorter than the value that brings it under, the rest kept as `no_route` with detail `too_long`.
  - **Dubins arrivals fail at most junction anchors:** the junction straight is dropped (0 m) and the arm direction
    alone is held; if they still fail, the junction-held class stays plate candidates.
  - **Fewer than half of the open-country sites are planned at any setting:** stop; report to the maintainer before
    writing production code.
- [ ] **Step 8:** Update the spec's constants table and this plan's Global Constraints with the answers (the only
  repo change of this task). Commit `docs: road planning constants from the spike`.

### Task 2: Obstacle raster and distance maps (Sonnet)

**Files:** create `crates/naves-core/src/reroute.rs` (module doc, constants, `RerouteParams` with `Default`, empty
`reroute_graph` is not yet public), `crates/naves-core/src/reroute/raster.rs`, `crates/naves-core/tests/reroute.rs`;
modify `crates/naves-core/src/lib.rs`. A fuzz target `fuzz/fuzz_targets/reroute_raster.rs` (segments and polygons
from bytes) with seeds that run in `make test`.

**Interfaces — Consumes:** `smooth::WaterTest<'a>` (`&'a (dyn Fn((f64, f64)) -> bool + Sync)`),
`conform::core_reach_m(half_width: f64, cell: f64) -> f64`.

**Interfaces — Produces:**

- `reroute::raster::Blocked { Water, Road, Edge }` (`Debug, Clone, Copy, PartialEq, Eq, Ord`).
- `reroute::raster::Clearance { pub water_m: f64, pub road_m: f64 }`.
- `reroute::raster::Obstacles` with `Obstacles::new(min: (f64, f64), max: (f64, f64), cell_m: f64, map_extent_m: f64,
  edge_margin_m: f64, max_cells: usize) -> Obstacles`, `stamp_water(&mut self, in_water: WaterTest<'_>)`,
  `stamp_road(&mut self, line: &[(f64, f64)], reach_m: f64)`, `set_original(&mut self, line: &[(f64, f64)])`,
  `finish(&mut self)` (the chamfer transforms), `segment(&self, a: (f64, f64), b: (f64, f64), clear: Clearance) ->
  Result<(), Blocked>`, `stray_m(&self, p: (f64, f64)) -> f64`, `water_distance_m(&self, p: (f64, f64)) -> f64`,
  `resolution_m(&self) -> f64`, `cells(&self) -> usize`.
- `reroute::raster::chamfer(obstacle: &[bool], cols: usize, rows: usize, res_m: f64) -> Vec<u16>` (decimetres,
  saturating).

- [ ] **Step 0: Crate check** (AGENTS.md): read the source of `line_drawing` (supercover), `imageproc`'s
  `distance_transform` and any chamfer crate found; record the verdict for the commit message.
- [ ] **Step 1: Failing tests** (box (0, 0)–(200, 200), cell 4 m, so resolution 2 m and 100 × 100 cells):
  - a square lake (100..120, 100..120) stamped: `segment((110, 0), (110, 200), 0-clearance)` is `Err(Water)`;
    `water_distance_m((130, 110))` is in [9.0, 11.0];
  - `segment((130, 0), (130, 200))` with water clearance 0 is `Ok`, with 5 m is `Err(Water)` (threshold
    `(5 + 2) · 1.0824 + 2 · √2 = 10.4` m), at x = 140 with 5 m is `Ok`;
  - a road line y = 50 stamped with reach 6 m: a segment along y = 60 with road clearance 0 is `Ok`, along y = 55 is
    `Err(Road)`;
  - map extent 200, edge margin 5: a segment along x = 3 is `Err(Edge)`; a segment to (−10, 50) is `Err(Edge)`;
  - a segment with a NaN end is `Err(Edge)`, no panic;
  - `set_original` with the line y = 100: `stray_m((50, 107))` is in [7.0, 7.0 · 1.0824 + 2.83];
  - a box of 100 km × 100 km with `max_cells` 4 000 000: `cells() ≤ 4_000_000`, and `resolution_m()` is
    `2 · 1.25^k` for the smallest such k;
  - a one-cell lake in a 10 km box: the distance far away is 6553.5 m (u16 saturates), no overflow;
  - `chamfer` of an empty mask is all `u16::MAX`; of a full mask all 0.
- [ ] **Step 2:** RED. **Step 3:** implement. **Step 4:** GREEN, `make check` (12G cap).
- [ ] **Step 5:** Commit `feat: an obstacle raster with distance maps for road planning` (with the crate review).

### Task 3: Feasibility band (Sonnet)

**Files:** `crates/naves-core/src/reroute/band.rs`, `tests/reroute.rs`.

**Interfaces — Consumes:** `raster::{Obstacles, Clearance, Blocked}`, `heightmap::Heightmap` (`size`, `cell_m: f32`,
`sample(x, y) -> Sample { height: f32, .. }`), `limits::grade_ceiling(Family) -> f64`.

**Interfaces — Produces:**

- `reroute::band::Band { pub cut_m: f64, pub fill_m: f64, pub grade: f64, pub spacing_m: f64 }`.
- `reroute::band::Reach { pub lo: f64, pub hi: f64 }`.
- `reroute::band::BandStep { pub reach: Reach, pub climb_m: f64, pub earthwork_m2: f64 }`.
- `reroute::band::BandFail { Blocked(Blocked), NoGround, Grade }`.
- `reroute::band::ground_at(map: &Heightmap, p: (f64, f64)) -> Option<f64>` (None outside the map or non-finite).
- `reroute::band::band_step(line: &[(f64, f64)], ground: &dyn Fn((f64, f64)) -> Option<f64>, band: &Band,
  start: Option<Reach>, obstacles: &Obstacles, clear: Clearance) -> Result<BandStep, BandFail>`.

- [ ] **Step 1: Failing tests** (obstacle-free raster; spacing 2 m):
  - flat ground at 100 m, grade 0.1, cut = fill = 0, a 100 m straight: `Ok`, climb 0, earthwork 0, reach [100, 100];
  - ground rising 0.2 per metre over 100 m, grade 0.1, cut = fill = 6: `Ok`, climb 16.0 ± 0.1, earthwork 80 ± 4 m²,
    reach [14, 16] ± 10⁻⁶ above the start ground;
  - the same over 130 m: `Err(Grade)` (the interval empties after 120 m);
  - flat at 100 with cut = fill = 6 and a start reach [50, 50]: `Err(Grade)` at the first sample;
  - ground returning NaN at x = 40: `Err(NoGround)`; a point outside the map: `ground_at` is `None`;
  - a step through a stamped lake: `Err(Blocked(Water))`;
  - two consecutive steps carry the reach: the second step's start interval is the first's end interval (pinned by a
    ramp that passes as one 100 m step and as two 50 m steps with equal climb).
- [ ] **Step 2:** RED. **Step 3:** implement. **Step 4:** GREEN, `make check`.
- [ ] **Step 5:** Commit `feat: the feasibility band of road planning`.

### Task 4: Cost-to-go field (Sonnet)

**Files:** `crates/naves-core/src/reroute/field.rs`, `tests/reroute.rs`.

**Interfaces — Consumes:** `raster::Obstacles`, `band::{Band, band_step}`.

**Interfaces — Produces:** `reroute::field::Field` with `Field::build(min: (f64, f64), max: (f64, f64), target:
(f64, f64), step_m: f64, cell_m: f64, band: &Band, ground: &dyn Fn((f64, f64)) -> Option<f64>, obstacles: &Obstacles,
clear: Clearance, stray_cap_m: f64, max_cells: usize) -> Field` and `Field::estimate(&self, p: (f64, f64)) -> f64`,
`Field::cells(&self) -> usize`. Dijkstra with an ordered heap (`total_cmp`, then cell index); a crate from Task 2's
check may replace it.

- [ ] **Step 1: Failing tests** (box (0, 0)–(200, 200), flat, field cell 4 m, target (100, 100)):
  - `estimate((140, 100))` is `40 / 1.0824 − 4` ± 0.5;
  - a water wall x = 120..124, y = 0..180: `estimate((160, 100)) ≥ 150` (the way round), and the estimate on the
    target's side of the wall is unchanged;
  - a cell enclosed by water: `estimate` is 0;
  - a 100 km box: `cells() ≤ 1_000_000`;
  - the field ignores the grade (a 50 % slope between the point and the target gives the same estimate as flat ground
    when cut and fill are 6 m);
  - two builds give the same estimates bit for bit.
- [ ] **Step 2:** RED. **Step 3:** implement. **Step 4:** GREEN, `make check`.
- [ ] **Step 5:** Commit `feat: the cost-to-go field of road planning`.

### Task 5: The arc search for one leg (Sonnet with the Opus helper)

**Files:** `crates/naves-core/src/reroute/dubins.rs`, `crates/naves-core/src/reroute/search.rs`, `tests/reroute.rs`.
A fuzz target `reroute_search` (poses and a small obstacle set from bytes, tiny budgets) with seeds.

**Interfaces — Consumes:** `raster`, `band`, `field`; `smooth::radius_at`.

**Interfaces — Produces:**

- `reroute::Pose { pub at: (f64, f64), pub heading: f64 }` (radians, counter-clockwise from +x).
- `reroute::Anchor { pub at: (f64, f64), pub heading: Option<f64>, pub straight_m: f64 }` (`None`: free heading, a
  dead end).
- `reroute::dubins::DubinsPath { pub turns: [Turn; 3], pub lengths: [f64; 3] }`, `Turn { Left, Straight, Right }`,
  `dubins_paths(from: Pose, to: Pose, radius: f64) -> Vec<DubinsPath>` (sorted by length, then by word),
  `DubinsPath::length(&self) -> f64`, `DubinsPath::points(&self, from: Pose, radius: f64, step_m: f64) ->
  Vec<(f64, f64)>`.
- `reroute::search::Weights { pub slope: f64, pub curve: f64, pub earth: f64, pub stray: f64 }`.
- `reroute::search::LegInput<'a> { pub start: Anchor, pub end: Anchor, pub radius: f64, pub step_m: f64, pub band:
  &'a Band, pub ground: &'a dyn Fn((f64, f64)) -> Option<f64>, pub obstacles: &'a Obstacles, pub clear: Clearance,
  pub field: &'a Field, pub weights: Weights, pub stray_cap_m: f64, pub arrive_reach_m: f64, pub budgets: &'a
  [(f64, usize)] }`.
- `reroute::search::Route { pub points: Vec<(f64, f64)>, pub length_m: f64, pub climb_m: f64, pub earthwork_m2: f64,
  pub max_stray_m: f64, pub cost: f64, pub weight: f64, pub states: usize }`.
- `reroute::search::LegOutcome { Found(Route), NoRoute { budget_exhausted: bool, states: usize, closest_m: f64 } }`.
- `reroute::RerouteError { NotFinite, Degenerate }` (`thiserror`).
- `reroute::search::search_leg(input: &LegInput<'_>) -> Result<LegOutcome, RerouteError>`.

- [ ] **Step 0: Crate check** of `dubins_paths` (source, panics on degenerate input, licence, maintenance); record
  the verdict.
- [ ] **Step 1: Failing tests (Dubins)**, radius 10.5: (0, 0, east) to (0, 21, west) shortest length `π · 10.5` ±
  10⁻⁹; (0, 0, east) to (50, 0, east) length 50; (0, 0, east) to (10.5, 10.5, north) length `π / 2 · 10.5`; every
  returned path ends at the target within 10⁻⁹ m and 10⁻⁹ rad; equal poses give one path of length 0; NaN input gives
  no path, no panic.
- [ ] **Step 2: Failing tests (search)**, flat ground at 100 m, no obstacles, R = 10.5, the spike's step and weights:
  - straight: (0, 0, east) to (100, 0, east): found, length 100 ± 10⁻⁶, `max_stray_m` ≤ 0.01, `states` ≤ 500;
  - 90° corner, original line (0, 0) → (60, 0) → (60, 60), anchors (18, 0, east) and (60, 42, north): found; every
    three-point radius of `points` ≥ 9.9; the last point equals (60, 42) within 10⁻⁶ m and the last segment heads
    north within 10⁻⁶ rad; `max_stray_m` ≤ 6.0; length in [79.0, 84.0];
  - hairpin, legs x = 0 and x = 8 from y = 0 to 60, apex radius 4: anchors (0, 20, north) and (8, 20, south):
    found, every radius ≥ 9.9, `max_stray_m` ≤ `3 · 10.5`, length > 92.6;
  - the same hairpin with water at y ≥ 30: `NoRoute { budget_exhausted: false, .. }`;
  - grade: anchors (0, 0, east) and (40, 0, east), ground rising 0.75 per metre, grade 0.27, cut = fill = 0: either
    `NoRoute` or a route of length ≥ 111.1; with the stray cap 0.5 m it is `NoRoute { budget_exhausted: false }`;
  - free end: (0, 0, east) to the dead end (50, 10): found, last point exact;
  - junction start: `straight_m` 6.25: the first 6.25 m of the route lie on the start heading within 10⁻⁶ m;
  - budgets `[(1.0, 10)]` on the hairpin: `NoRoute { budget_exhausted: true, states: 10, .. }`;
  - anchors 5 m apart, both east: found, length 5 ± 10⁻⁶; anchors equal: `Err(Degenerate)`; a NaN anchor:
    `Err(NotFinite)`;
  - two runs of the corner give equal `Route`s bit for bit.
- [ ] **Step 3:** RED. **Step 4:** implement. **Step 5:** GREEN, `make check`, the fuzz target 10 s.
- [ ] **Step 6:** Commit `feat: the arc search of road planning for one leg`.

### Task 6: Zones, anchors and legs on the graph, and the write-back (Sonnet)

**Files:** `crates/naves-core/src/reroute.rs` (top level), `crates/naves-core/src/smooth.rs` and
`smooth/widen.rs` only to make `plan_chains`, `ChainPlan`, `Link` and `tight_runs` `pub(crate)` (no behaviour change;
the smoothing tests stay green untouched), `crates/naves-core/tests/reroute.rs`.

**Interfaces — Consumes:** `roadnet::{RoadGraph, Stretch, Crossing}`, `roadnet::crossings_of(&[Stretch]) ->
Vec<Crossing>` (`pub(crate)`), `roadnet::polyline_length`, `roadnet::over_crossings(&RoadGraph) ->
Vec<OverCrossing>`, `smooth::{resample, radius_at, SAMPLE_M, CHAIN_TURN_MAX_DEG, WaterTest}`,
`catalog::Family::width_m`, `heightmap::Heightmap`.

**Interfaces — Produces:**

- `reroute::RerouteParams { pub lead: f64, pub radius_margin: f64, pub step_m: f64, pub stray_max: f64, pub cut_m:
  f64, pub fill_m: f64, pub weights: Weights, pub budgets: Vec<(f64, usize)>, pub max_zone_m: f64, pub
  junction_straight_m: f64, pub bridge_hold_m: f64, pub raster_max_cells: usize, pub field_max_cells: usize }`,
  `Default` from the constants.
- `reroute::AnchorKind { Line, Junction(i64), DeadEnd(i64), BridgeHold, TunnelHold, RingCut(i64) }`.
- `reroute::KeptReason { NoRoute, BlockedByWater, BlockedByRoad, Held }` with `id(self) -> &'static str`
  (`"no_route"`, `"blocked_by_water"`, `"blocked_by_road"`, `"held"`).
- `reroute::ZoneOutcome { Planned, Kept { reason: KeptReason, detail: String } }`, `reroute::Candidate { Plate,
  CurvedBridge }`.
- `reroute::PlannedZone { pub id: String, pub at: (f64, f64), pub stretches: Vec<String>, pub legs: usize, pub
  anchors: (AnchorKind, AnchorKind), pub junction: Option<i64>, pub partner: Option<String>, pub outcome:
  ZoneOutcome, pub radius_before_m: f64, pub radius_wanted_m: f64, pub radius_after_m: f64, pub length_before_m: f64,
  pub length_after_m: f64, pub max_stray_m: f64, pub climb_m: f64, pub earthwork_m2: f64, pub states: usize, pub
  weight: Option<f64>, pub budget_exhausted: bool, pub original_feasible: bool, pub candidate: Option<Candidate> }`.
- `reroute::RerouteReport { pub zones: Vec<PlannedZone>, pub skipped: Vec<(String, String)>, pub tight_before_m: f64,
  pub tight_after_m: f64, pub max_stray_m: f64, pub states: usize }`.
- `reroute::reroute_graph(graph: &RoadGraph, ground: &Heightmap, min_radius: &dyn Fn(&Stretch) -> f64, in_water:
  WaterTest<'_>, params: &RerouteParams) -> (RoadGraph, RerouteReport)` (the minimum radius is collected per stretch
  before the parallel part, as the smoothing does). In this task a failed leg is `Kept { NoRoute }` or
  `Kept { Held }`; the other reasons come in Task 7.

All fixtures: a flat heightmap of 513 × 513 vertices at 4 m (2048 m), City roads (tightest arc 10 m, R = 10.5), a
corner at (500, 500) unless said otherwise.

- [ ] **Step 1: Failing tests (zones and anchors):**
  - a graph without a tight run: the graph comes back equal, the report has no zone;
  - town corner: J1 (400, 500) → corner (500, 500) → J2 (500, 600), both J with a side road: one zone, planned, both
    anchors `Line`; J1 and J2 at their positions bit for bit; the sets of stretch ids and node ids equal; tight length
    after 0; `radius_wanted_m` 10.0;
  - corner 20 m after J1: anchor `Junction(J1)`; the direction of the arm at J1 unchanged within 10⁻⁹ rad; the route's
    first 6.25 m on y = 500 within 10⁻⁶ m; planned;
  - corner 15 m before a dead end: anchor `DeadEnd`; the dead end's position bit for bit;
  - a 30 m bridge 10 m after the corner: the bridge stretch's points bit for bit, every point within 25 m of its ends
    unchanged; the zone has two legs;
  - a tight corner 12 m before a bridge, so the hold leaves no room: `Kept { Held }`, candidate `Plate`, line bit for
    bit;
  - a tight kink on a 6 m stretch between two junctions: `Kept { Held }`, no panic (Review Focus 1);
  - a two-stretch node at (480, 500) inside the zone: it moves; the crossing of that node has the new point, equal to
    the end points of both its stretches; the stretch count is unchanged;
  - a zone whose original line passes under a bridge deck: `Kept { Held }` with detail `crosses_deck`.
- [ ] **Step 2: Failing tests (obstacles on the graph):**
  - the hairpin of Task 5 with a road across its top 35 m beyond the apex, not touched by the original line (the
    clearance is `core_reach_m(5, 4) + core_reach_m(5, 4) + 4 = 26` m): the route's smallest distance to that road's
    centre line ≥ 26 m;
  - a side road at an anchor (exempt): the route crosses it as often as the original line does;
  - switchback: two legs of one chain 15 m apart beyond the zone; the route keeps the road clearance from the own
    chain outside the zone (Review Focus 6);
  - ring: a closed chain with a tight bend at its lowest node: that zone `Kept { Held }` with anchor `RingCut`; a tight
    bend elsewhere on it: planned, the ring's first and last point equal bit for bit (Review Focus 2);
  - map edge: the corner at (8, 500) with the bulge towards x < 0: no written point with x < `core_reach_m(5, 4)`;
    planned or kept, no panic (Review Focus 4);
  - one stretch with a NaN point: listed in `skipped`, the town corner elsewhere planned exactly as without it; a
    stretch at 10⁹ m: skipped or kept, `states` bounded by the budgets, no panic (Review Focus 3).
- [ ] **Step 3: Failing tests (determinism):** the features given in reverse order give an equal graph and report;
  rayon pools of 1 and 4 threads give equal graphs and reports bit for bit.
- [ ] **Step 4:** RED. **Step 5:** implement. **Step 6:** GREEN, `make check`.
- [ ] **Step 7:** Commit `feat: road planning re-routes tight zones of the road graph`.

### Task 7: Failure reasons, routes against each other, the report (Sonnet)

**Files:** `crates/naves-core/src/reroute.rs`, `tests/reroute.rs`.

**Interfaces — Produces:** the reasons `BlockedByWater` and `BlockedByRoad` (diagnostic searches without the water
or road obstacle, the last budget pair only); the sequential cross-check in zone order;
`RerouteReport::summary(&self) -> String`; `RerouteReport::kept(&self, reason: KeptReason) -> usize`.

- [ ] **Step 1: Failing tests:**
  - hairpin with water outward and inward of the legs from y ≥ 30: `Kept { BlockedByWater }`, candidate
    `CurvedBridge`, line bit for bit;
  - the same with a road instead of the water: `Kept { BlockedByRoad }`, candidate `None`;
  - two hairpins on parallel roads whose apexes face each other 30 m apart (original lines more than 26 m apart, so
    not touching) and whose routes would each bulge into the gap: the zone of the lower chain id is planned, the other
    `Kept { BlockedByRoad }` with detail `route_of <id>`; its line bit for bit;
  - fifty town corners with budgets `[(1.0, 50)]`: fifty `Kept { NoRoute }` with `budget_exhausted`, report `states ≤
    50 · (50 + 2 · 50)`, and the same report on a second run (Review Focus 5);
  - a route that the final check rejects (forced in a unit test by a check radius above the planning radius):
    `Kept { NoRoute }` with detail `check_failed`;
  - `summary()` of a report with 3 zones (2 planned, 1 held) is exactly
    `Road planning: 3 zones, 2 planned, 1 kept (no_route 0, held 1, blocked_by_water 0, blocked_by_road 0), tight
    length <before> m -> <after> m, largest move <m> m` with the fixture's numbers.
- [ ] **Step 2:** RED. **Step 3:** implement. **Step 4:** GREEN, `make check`.
- [ ] **Step 5:** Commit `feat: road planning says why a bend was kept`.

### Task 8: Road planning in the build (Sonnet; DeepSeek for the marked steps)

**Files:** `crates/naves-export/src/generate.rs`, `crates/naves-core/src/roadplace.rs` (`PlacedNetwork`),
`crates/naves-core/src/validate.rs`, `crates/naves-core/tests/validate.rs`, `crates/naves-export/src/build.rs`,
`crates/naves-export/tests/smoothing.rs` and `tests/build.rs` (callers of `fitted_graph_with`), a new
`crates/naves-export/tests/planning.rs`, `crates/naves/src/mcp.rs`, the binary's summary print, the `road-analysis`
and `road-sites` examples, `docs/manual/src/getting-started.md`, `troubleshooting.md`, `mcp.md`, `CHANGELOG.md`.

**Interfaces — Consumes:** `reroute::{reroute_graph, RerouteParams, RerouteReport, REROUTE_ENABLED}`,
`smooth::SMOOTH_ENABLED`.

**Interfaces — Produces:**

- `generate::Stages { pub reroute: bool, pub smooth: bool }` with `Stages::DEFAULT` (`REROUTE_ENABLED`,
  `SMOOTH_ENABLED`).
- `generate::fitted_graph_with(features, catalog, base, stages: Stages, in_water)` (the `smooth: bool` parameter
  becomes `stages`); `FittedGraph::reroute: Option<RerouteReport>`, `FittedGraph::reroute_time: Duration`.
- `generate::plan_roads_with(…, stages: Stages)`; `plan_roads` calls it with `Stages::DEFAULT`.
- `PlacedNetwork::reroute: Option<RerouteReport>`, `Generated::reroute`, `BuildOutput::planning: Option<String>`.
- `Rule::RoadCurvePlanned` (`road_curve_planned`, information) and `Rule::RoadCurveNotPlanned`
  (`road_curve_not_planned`, warning); `Rule::ALL: [Rule; 33]`; `validate::reroute_violations`.
- `build.rs::planning_json` for `roads.planning`.

- [ ] **Step 1: Failing tests:**
  - `fitted_graph_with` with `Stages { reroute: false, smooth: false }` on the smoothing test's fixture equals the
    graph of today bit for bit, `reroute` is `None`;
  - with `reroute: true` on a fixture with one tight corner: `reroute` is `Some`, one zone planned, and `graph`
    differs only on that chain;
  - `fitted_graph(..).reroute.is_some() == REROUTE_ENABLED`;
  - `Rule::RoadCurvePlanned.id() == "road_curve_planned"`, `Rule::RoadCurveNotPlanned.id() ==
    "road_curve_not_planned"`; a report with one planned and one held zone gives one finding of each at the zones'
    `at`, with the radii in the message;
  - `planning_json(None)["enabled"] == false`; with a report: `enabled`, `zones`, `tight_before_m`,
    `tight_after_m`, every zone with `outcome`, `reason`, `candidate`;
  - the MCP tool description lists both new rule ids (the existing list test).
- [ ] **Step 2:** RED. **Step 3:** implement the wiring, the stage's progress line
  `Planning the tight curves (<s> s)` and the summary line.
- [ ] **Step 4 (DeepSeek):** `road-analysis`: read `STAGES` from the environment (`off`, `plan`, `smooth`,
  `plan+smooth`; default `Stages::DEFAULT`), print `STAGES reroute=<bool> smooth=<bool>`, `TIMING reroute <s> s`,
  `PLAN <summary>`, `PLAN_REASON <reason> <count>` for each reason, `PLAN_KEPT <id> <reason> <x> <y> <radius>` for
  each kept zone, `PLAN_FIXED junctions moved <n>, dead ends moved <n>, bridge ends moved <n>, tunnel ends moved <n>
  (all must be 0)`.
- [ ] **Step 5 (DeepSeek):** the manual: getting-started "Roads" (what planning does, that it is off), troubleshooting
  (the two rules, the four reasons, the candidates), mcp.md (the rule list); `CHANGELOG.md` under Unreleased, Added:
  "Road planning re-routes curves tighter than the road pieces ... switched off in this release (`REROUTE_ENABLED`)
  ... with it on, roads inside planned curves move by up to three minimum radii, so edits keyed by slot can be
  orphaned (`orphaned_edit`) ... `roads.planning` in `build/report.json`, findings `road_curve_planned` and
  `road_curve_not_planned`".
- [ ] **Step 6:** GREEN, `make check` (with `make docs`).
- [ ] **Step 7:** Commit `feat: road planning in the build, switched off`.

### Task 9: Judging runs (controller with the Opus reviewer; Sonnet runs them)

**Files:** none in the repo. Output under `/var/tmp/naves-plan/judge/`.

- [ ] **Step 1:** Rebuild the release example once (log HEAD and the binary's mtime). Check the machine (`nproc`,
  `free`, `uptime`); one run at a time, 16G cap each.
- [ ] **Step 2:** Four runs of `road-analysis` on Berchtesgaden at 2048 with the same binary: `STAGES=off`, `plan`,
  `smooth`, `plan+smooth`. Logs `judge/<stages>/ra-2048.log`, `RULES_OUT=judge/<stages>/rules.tsv`.
- [ ] **Step 3:** Write `judge/compare.md`, one table, four columns:
  - tight length (baseline 16 143 m; smoothing alone 4088 m) and zones (873 on the fitted line; the spike's count on
    the drawn line);
  - zones by outcome and reason; the full list of kept zones with reason, candidate, position and radius;
  - largest and p95 distance of a planned route from the original line;
  - `road_gap` count, total length, largest (389, 1618 m, 131 m); `road_not_drivable` (141), `road_conflict` (797),
    `road_misses_terrain` (3752), `road_grade`, `road_in_water`, `earthworks_capped`, `join_without_junction`,
    `orphaned_edit`, every other rule;
  - the `PLAN_FIXED` line (all must be 0);
  - placement time (about 550 s) and `TIMING reroute` separately.
- [ ] **Step 4:** Judge against the spec's bar: tight length with `plan` at most 8072 m (half the baseline); none of
  `road_gap`, `road_not_drivable`, `road_conflict`, `road_misses_terrain` worse than `off` by more than 5 % without a
  reason that can be named; planning under 60 s; placement not more than 10 % slower. A miss is reported to the
  maintainer with the numbers; nothing is tuned until the numbers are understood (one round of tuning at most, each
  change its own commit with a test).
- [ ] **Step 5:** From `plan+smooth` against `plan`: did the smoothing tighten planned arcs (tight length inside
  planned zones after smoothing)? Record it for the spec's open point.
- [ ] **Step 6:** Pick ten sites for the game: two switchbacks, two town corners, the junction hairpin, the bend beside
  water, one plate candidate, one curved-bridge candidate, one zone with a moved two-stretch node, one straight road as
  a control. Write them as a `sites.json` for `tools/ingame-check`.

No commit, unless Step 4's tuning changes a constant (then the spec's table changes in the same commit).

### Task 10: Checkpoint, the game (controller with a Sonnet agent)

- [ ] **Step 1:** Build the map twice from this branch: once `STAGES=off`, once `STAGES=plan` (log HEAD and the binary's
  mtime each time). Keep both `build/` folders under `/var/tmp/naves-plan/game/`.
- [ ] **Step 2:** `tools/ingame-check/run.sh` on each build with the same `sites.json` (24G cap, non-default ports, one
  game at a time, headless sway only). Stop the game when done; remove the `@naves*` symlinks from the DayZ install.
- [ ] **Step 3:** Send the pairs of contact sheets to the maintainer with one line per site: what planning changed,
  whether the road is drivable and usable, anything that looks wrong.
- [ ] **Step 4:** The maintainer decides whether `REROUTE_ENABLED` becomes `true`. If yes: a commit `feat!: road
  planning is switched on` that flips the constant, updates the manual ("off" becomes "on") and adds the
  **Breaking** note for orphaned edits to `CHANGELOG.md`; `make check`.

### Task 11: Final review (Opus)

- [ ] Review the whole branch against the spec and this plan: every spec rule has a test; the Review Focus tests
  exist and assert what is listed; the constants in the code equal the spec's table; no fixed node can move (read the
  write-back, not only the tests); determinism (no hash-map iteration, budgets by count, `total_cmp`); no panics on
  input; coverage not below the base; the manual and the CLI help agree.
- [ ] Update the spec's status line with the result (planned, judged, on or off) and its open points with what the
  judging showed.
- [ ] Hand the list of kept zones (from Task 9) to the controller as the start of the plate and curved-bridge tasks.

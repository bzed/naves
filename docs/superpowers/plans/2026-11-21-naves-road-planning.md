<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Road Planning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Re-route every curve the road pieces cannot build as a chain of arcs and straights no tighter than the
family's minimum radius and no steeper than the road was meant to be, between two anchors on the same road, across the
terrain, clear of water, streams, other roads and its own road; keep and report every bend that cannot be planned.

**Architecture:** A new module `crates/naves-core/src/reroute.rs` with sub-modules `raster` (obstacles, distance maps,
wall lines), `band` (feasibility band, the two-pass profile, the leg's grade limit), `field` (cost-to-go), `dubins`
and `search` (the arc search for one leg). The top module finds the tight zones on the smoothing's chains, makes
anchors and legs, searches each leg in parallel, writes the routes back, cross-checks them and reports.
`naves-export`'s `generate::fitted_graph_with` calls it between `netclean::clean` and the smoothing, behind a switch,
with the four stage combinations selectable per call.

**Tech Stack:** Rust 1.99; `rayon` (already a dependency) for legs in parallel. No new dependency is planned. Tasks 2
and 5 check crates first, as AGENTS.md demands: `pathfinding` (Dijkstra for the field), `dubins_paths` (Dubins words),
`line_drawing` (supercover grid traversal), the distance transform of `imageproc`. What the repo already has comes
first (`smooth::resample`, `smooth::radius_at`, `roadnet::polyline_length`, `validate::water_test`,
`conform::core_reach_m`, `features::water_lines`, `limits::{grade_aim, grade_ceiling}`).

**Spec:** `docs/superpowers/specs/2026-11-21-road-planning-design.md` (amended after the spike).

**Method source:** `docs/research/2026-11-21-road-tools-clean-room-notes.md`, sections 3.2 to 3.6 and 5.1.
Implementers read the notes and the spec, never third-party code. The spike's source
(`.superpowers/sdd/2026-11-21-naves-road-planning/spike/spike-src/`) is ours and may be read as a reference for what
was measured; production code is written fresh, test first, and nothing is pasted from it.

**Implementers:** Sonnet. One Opus helper is allowed for the search (Task 5). **Reviews:** Opus, after every task.
Steps marked **(DeepSeek)** are simple and fully specified; the controller may delegate them to a cheaper model.

## Global Constraints

- AGENTS.md binds: test first (RED then GREEN), `make check` green before every commit, coverage not below the value
  on the base commit, no panics on input (`unwrap`, `expect`, indexing on untrusted data are denied in library code),
  `#[expect(lint, reason)]` never `#[allow]`, no truncating or sign-changing casts, checked arithmetic on sizes, sort
  and compare floats with `total_cmp`, no `==` on floats, deterministic output (no `HashMap` or `HashSet` iteration
  order), `thiserror` errors in the library, `tracing` not `println!` outside the binary and examples, docs and
  CHANGELOG in the behaviour commit, no game data in the repo (tests use synthetic graphs and heightmaps).
- Every process gets a hard memory limit: `systemd-run --user --scope --quiet -p MemoryMax=12G -p MemorySwapMax=1G`
  for cargo and `make check`, 16G for `road-analysis` and the re-check, server 8G, client 14G, `ingame-check` 24G.
  Never raise a cap; report an OOM kill. The sum of what parallel agents run must fit the machine (24 threads,
  60 GB): check `nproc`, `free`, `uptime` before starting a long run.
- Temp and build folders under `/var/tmp` (never `/tmp`, never `$HOME`). Shared target dir `/var/tmp/naves-target`,
  never deleted; after a worktree agent, touch the sources before `make check`.
- Never start the game outside the checkpoint task. Wine/Proton only inside headless sway. Never touch the user's
  DayZ profile folder or client settings. Non-default server ports. Remove `@naves*` symlinks from the DayZ install
  afterwards. One game at a time.
- Figures are quoted for Berchtesgaden at half scale (`scale.horizontal = 0.5`), terrain size 2048 (4 m cells).
- Constants, verbatim from the spec (all `pub const` in `reroute.rs`, collected in `RerouteParams::default()`; the
  ones marked **(re-check)** are confirmed or changed by Task 7b, and the spec's table changes in the same commit):
  `REROUTE_ENABLED = false`, `PLAN_LEAD = 2.0` (minimum radii, **re-check**), `PLAN_RADIUS_MARGIN = 1.05`,
  `PLAN_REACHED_FRACTION = 0.99`, `PLAN_STEP_M = 6.0`, `PLAN_MAX_TURN_DEG = 45.0`, `PLAN_HEADING_BINS_MIN = 32`,
  `PLAN_HEADING_BINS_MAX = 180`, `PLAN_ARRIVE_REACH = max(3·s, 4·R)`, `PLAN_JUNCTION_STRAIGHT_M = 6.25`,
  `PLAN_BRIDGE_HOLD_M = 25.0`, `PLAN_DECK_ANCHOR_M = 12.0`, `PLAN_STRAY_MAX = 3.0` (minimum radii, **re-check**),
  `PLAN_CUT_M = 6.0`, `PLAN_FILL_M = 6.0` (**re-check**), `PLAN_ANCHOR_TOL_M = 0.5` (**re-check**),
  `PLAN_GRADE_RELAX_STEPS = 3` (**re-check**), `PLAN_STEEPER_MARGIN = 0.02`, `PLAN_DRAWN_WINDOW_M = 20.0`,
  `W_SLOPE = 4.0`, `W_CURVE = 0.1`, `W_EARTH = 1.0` (**re-check**), `W_STRAY = 0.25` (**re-check**),
  `PLAN_BUDGETS = [(1.0, 10_000), (1.5, 20_000), (3.0, 40_000)]` (**re-check**), `PLAN_MAX_ZONE_M = 400.0`,
  `PLAN_EXEMPT_EXTEND_M = 10.0` (**re-check**), `PLAN_CROSSING_END_M = 10.0`, `PLAN_RASTER_MAX_CELLS = 4_000_000`,
  `PLAN_FIELD_MAX_CELLS = 1_000_000`, `CHAMFER_ERROR = 1.0824`. Derived per leg: the grade limit
  `g_leg = min(grade_ceiling, max(grade_aim, drawn mean grade))`, the test threshold
  `thr(c) = (c + res) · CHAMFER_ERROR + res · √2`, `PLAN_SELF_FREE = thr(road clearance) + own core reach`,
  `PLAN_SELF_ALONG = π · R + 2 · road clearance`. Reused, not copied: `smooth::SAMPLE_M` (2.0),
  `smooth::CHAIN_TURN_MAX_DEG` (150.0), `limits::grade_aim`, `limits::grade_ceiling`, `conform::core_reach_m`.
- Stretch ids and node ids never change. No stretch is added or removed. Junction nodes (and their arm directions),
  dead ends, bridge and tunnel end nodes, bridge stretches and the 25 m beside each bridge or tunnel end never move.
- A planned leg is never steeper than its grade limit at any sample of its profile; a relaxed limit is reported.
- A zone that is not planned keeps its original line bit for bit; one zone's failure changes nothing else.
- The stage switched off returns the graph unchanged, bit for bit. Byte-identical graphs across runs and thread
  counts. Budgets are counts of states, never wall time.
- Every planned or kept zone is reported; nothing moves silently.

## Review Focus

These are the inputs most likely to bite. Each has its test in the task named.

1. **A switchback cut short.** The cost prefers length; between two fixed anchor heights a shorter route is cheaper
   as long as the band holds. Expected: no planned leg shorter than `(|Δz| − 2 · PLAN_ANCHOR_TOL_M) / g_leg`, and no
   profile sample steeper than `g_leg`. Tests in Task 3 (the switchback's profile numbers) and Task 5 (the synthetic
   switchback at 15 % and at 25 %).
2. **The own road.** A planned leg must keep the road clearance from its own chain outside the zone (the other legs of
   a switchback stack) and must not loop back onto itself. The spike never tested this. Tests in Task 2 (stamping the
   own chain beyond `PLAN_SELF_FREE`), Task 5 (a loop that would come back onto itself) and Task 6 (a switchback
   stack).
3. **Routes against each other.** Two legs searched in parallel can each be clear of the other's drawn line and still
   touch each other's route. Expected: the later one (zone order) reverts to its line, `blocked_by_road`, the same on
   every run. Test in Task 7.
4. **Anchors that coincide, lie within each other's radius, or sit on a deck.** A tight kink on a 6 m stretch between
   two junctions; a junction 8 m from a bridge end. Expected: `held` or a valid route, never a panic, never a route
   that violates the radius, never an anchor blocked by its own deck. Tests in Task 2 (deck within 12 m of an anchor),
   Task 5 (`Degenerate`), Task 6 (6 m stretch).
5. **A chain that is a ring, a zone at the map edge, non-finite or huge coordinates.** Expected: the ring cut is
   `held`, the ring stays closed; no point within the edge margin unless the drawn line already was; a bad chain is
   skipped and nothing else changes; raster and field sizes stay under their caps. Tests in Tasks 2, 5 and 6.
6. **A search that exhausts its budget on every zone of a town**, now with the grade relaxation on top. Expected: all
   kept as `no_route` with `budget_exhausted`, states bounded by
   `zones · ((PLAN_GRADE_RELAX_STEPS + 1) · Σ budgets + 2 · last budget)`, the same report on every run. Test in
   Task 7.

---

### Task 1: Spike (done, 2026-11-21)

Done; nothing in the repo changed. Answers, sites, pictures, tables and source:
`.superpowers/sdd/2026-11-21-naves-road-planning/spike/` (`answers.md`, `sites.md`, `svg/`, `out/`, `spike-src/`; working
copies under `/var/tmp/naves-plan/spike/`). The spec's constants table carries the answers with the measurements behind
them. Headline: 2261 zones at lead 2 (1700 at lead 4), 77 % of the lead-4 zones with a junction at a zone end; 2046 of
2227 searched zones planned, 167 held, 11 blocked by a road, 2 by water, 1 failed its check; tight length 16 135 m →
1547 m; 5.3 s on 16 threads, 53.5 s on one; output identical at 1, 10 and 16 threads; none of the "if bad" rules
triggered.

Checked afterwards by the plan's author (scratch copy `/var/tmp/naves-plan/check/`, not in the repo): the spike's
`climb` is the sum of rises and falls of the terrain clamped into the reachable interval, not the net height
difference; its earthwork is a lower bound that is 0 while the terrain stays inside the interval; the anchors' road
height floated ±6 m. Switchback-1's route (90.8 m, net 21.4 m, mean 23.6 %, gravel ceiling 36 %, aim 18 %) needs
46 m² of earthwork, a cut of 1.6 m and a fill of 2.3 m with its anchors pinned (two-pass profile), and cannot be built
at 18 % at that length; the terrain along it is steeper than the ceiling on 18.2 m. On the map, 239 of 2046 planned legs
are steeper on their mean than `max(aim, drawn grade)` (half of them by less than 1.7 m of missing road, the worst
80 m), 15 above the ceiling. The spec's grade rule, pinned anchors and real earthwork come from this; Task 7b measures
their effect before the constants are frozen.

### Task 2: Obstacle raster, distance maps and wall lines (Sonnet)

**Files:** create `crates/naves-core/src/reroute.rs` (module doc, constants, `RerouteParams` with `Default`; nothing
public yet but the sub-modules), `crates/naves-core/src/reroute/raster.rs`, `crates/naves-core/tests/reroute.rs`;
modify `crates/naves-core/src/lib.rs`. A fuzz target `fuzz/fuzz_targets/reroute_raster.rs` (segments, polygons and
wall lines from bytes) with seeds that run in `make test`.

**Interfaces — Consumes:** `smooth::WaterTest<'a>` (`&'a (dyn Fn((f64, f64)) -> bool + Sync)`),
`conform::core_reach_m(half_width: f64, cell: f64) -> f64`.

**Interfaces — Produces:**

- `reroute::raster::Blocked { Water, Road, Edge, Wall }` (`Debug, Clone, Copy, PartialEq, Eq, Ord`).
- `reroute::raster::Clearance { pub water_m: f64, pub road_m: f64 }`.
- `reroute::raster::Obstacles` with `Obstacles::new(min: (f64, f64), max: (f64, f64), cell_m: f64, map_extent_m: f64,
  edge_margin_m: f64, max_cells: usize) -> Obstacles`, `stamp_water(&mut self, in_water: WaterTest<'_>)`,
  `stamp_road(&mut self, line: &[(f64, f64)], reach_m: f64, exempt: &dyn Fn((f64, f64)) -> bool)` (a sample of the
  line for which `exempt` is true is not stamped), `set_original(&mut self, line: &[(f64, f64)])`, `finish(&mut self)`,
  `threshold(&self, clearance_m: f64) -> f64` (`thr(c)`, 0 for `c ≤ 0`), `segment(&self, a, b, clear: Clearance) ->
  Result<(), Blocked>`, `stray_m(&self, p) -> f64`, `water_distance_m(&self, p) -> f64`,
  `road_distance_m(&self, p) -> f64`, `water_cap_m(&self, original: &[(f64, f64)]) -> f64` (the largest water
  clearance the original line passes), `resolution_m(&self) -> f64`, `cells(&self) -> usize`.
- `reroute::raster::Wall { pub line: Vec<(f64, f64)>, pub allowed: usize, pub kind: WallKind }`,
  `WallKind { Road, Stream }`; `reroute::raster::crossings_away(route: &[(f64, f64)], line: &[(f64, f64)],
  anchors: [(f64, f64); 2], end_m: f64) -> usize`; `Wall::blocks(&self, a, b) -> bool` (true when `allowed` is 0 and
  the segment crosses the line).
- `reroute::raster::chamfer(obstacle: &[bool], cols: usize, rows: usize, res_m: f64) -> Vec<u16>`.
- `reroute::raster::exempt_samples(other: &[(f64, f64)], original: &[(f64, f64)], near_m: f64, extend_m: f64) ->
  Vec<bool>` (per 2 m sample of `other`: within `near_m` of the original line, grown by `extend_m` along `other`).

- [ ] **Step 0: Crate check** (AGENTS.md): read the source of `line_drawing` (supercover), `imageproc`'s
  `distance_transform` and any chamfer crate found; record the verdict for the commit message.
- [ ] **Step 1: Failing tests** (box (0, 0)–(200, 200), cell 4 m, so resolution 2 m and 100 × 100 cells):
  - a square lake (100..120, 100..120): `segment((110, 0), (110, 200), 0-clearance)` is `Err(Water)`;
    `water_distance_m((130, 110))` in [9.0, 11.0];
  - `threshold(5.0)` is `(5 + 2) · 1.0824 + 2 · √2` = 10.40 ± 10⁻⁹; `segment` along x = 130 with water clearance 5
    is `Err(Water)`, along x = 140 `Ok`;
  - **one threshold** (spike 8.1): an original line along x = 135; `water_cap_m` of it is the clearance whose threshold
    equals its distance to the lake; with that clearance the original line passes `segment` (it failed in the spike
    when the cap was the bare distance);
  - a road y = 50 stamped with reach 6: along y = 60 with road clearance 0 `Ok`, along y = 55 `Err(Road)`; with an
    `exempt` predicate true for x < 100, a segment along y = 52 from x = 10 to 90 is `Ok` and from 110 to 190 is
    `Err(Road)` (per-place exemption);
  - `exempt_samples` of a 200 m road whose middle 20 m lies 8 m from the original, `near_m` 20, `extend_m` 10: exactly
    the samples of the middle 40 m are true;
  - **map edge** (spike 8.3): extent 200, margin 5: a segment along x = 3 is `Err(Edge)`, a segment to (−10, 50) is
    `Err(Edge)`; a segment along x = 7 is `Ok` with road clearance 15 (the edge has no clearance on top of its margin);
    with margin 0 (a road that runs to the edge) a segment along x = 1 is `Ok`;
  - **wall lines**: a stream line x = 100; `Wall { allowed: 0 }` blocks a segment from (90, 50) to (110, 50); with
    `allowed: 1` it does not block; `crossings_away` of a route crossing the stream at (100, 50) and at (100, 195) with
    anchors (90, 50) and (110, 195): 0 with `end_m` 10 (both within 10 m of an anchor), 2 with `end_m` 0 (spike 8.4);
  - a segment with a NaN end is `Err(Edge)`, no panic;
  - `set_original` with y = 100: `stray_m((50, 107))` in [7.0, 7.0 · 1.0824 + 2.83];
  - a 100 km box with `max_cells` 4 000 000: `cells() ≤ 4_000_000`, `resolution_m()` is `2 · 1.25^k` for the smallest
    such k; a one-cell lake in a 10 km box: far distance 6553.5 m, no overflow; `chamfer` of an empty mask is all
    `u16::MAX`, of a full mask all 0.
- [ ] **Step 2:** RED. **Step 3:** implement. **Step 4:** GREEN, `make check` (12G cap).
- [ ] **Step 5:** Commit `feat: an obstacle raster with distance maps and wall lines for road planning` (with the crate
  review).

### Task 3: Feasibility band, profile and grade limit (Sonnet)

**Files:** `crates/naves-core/src/reroute/band.rs`, `tests/reroute.rs`.

**Interfaces — Consumes:** `raster::{Obstacles, Clearance, Blocked}`, `heightmap::Heightmap` (`size`, `cell_m: f32`,
`sample(x, y) -> Sample { height: f32, .. }`), `limits::{grade_aim, grade_ceiling}(Family) -> f64`,
`catalog::Family`.

**Interfaces — Produces:**

- `reroute::band::Band { pub cut_m: f64, pub fill_m: f64, pub grade: f64, pub spacing_m: f64, pub anchor_tol_m: f64 }`.
- `reroute::band::Reach { pub lo: f64, pub hi: f64 }`; `Reach::pinned(z: f64, tol: f64) -> Reach`.
- `reroute::band::BandStep { pub reach: Reach, pub climb_m: f64, pub earthwork_low_m2: f64 }` (the lower bound).
- `reroute::band::BandFail { Blocked(Blocked), NoGround, Grade, Arrival }`.
- `reroute::band::ground_at(map: &Heightmap, p: (f64, f64)) -> Option<f64>`.
- `reroute::band::band_step(line: &[(f64, f64)], ground: &dyn Fn((f64, f64)) -> Option<f64>, band: &Band,
  start: Reach, obstacles: &Obstacles, clear: Clearance) -> Result<BandStep, BandFail>`.
- `reroute::band::arrives(reach: Reach, z_end: f64, tol: f64) -> bool`.
- `reroute::band::Profile { pub heights: Vec<f64>, pub earthwork_m2: f64, pub cut_max_m: f64, pub fill_max_m: f64,
  pub max_grade: f64 }`; `reroute::band::profile(line: &[(f64, f64)], ground: &dyn Fn((f64, f64)) -> Option<f64>,
  band: &Band) -> Result<Profile, BandFail>` (notes 5.1 steps 1 to 4: both ends pinned within `anchor_tol_m`,
  forward and backward passes, the profile closest to the terrain).
- `reroute::band::DrawnGrade { pub delta_z: f64, pub mean: f64, pub window: f64 }`;
  `reroute::band::drawn_grade(line: &[(f64, f64)], ground: &dyn Fn((f64, f64)) -> Option<f64>, window_m: f64) ->
  Option<DrawnGrade>`; `reroute::band::leg_limit(families: &[Family], drawn: &DrawnGrade) -> f64`
  (`min(ceiling, max(aim, mean))`, smallest aim and ceiling over the families); `reroute::band::relaxed_limits(limit:
  f64, ceiling: f64, steps: usize) -> Vec<f64>` (`[limit, limit + gap/3, limit + 2·gap/3, ceiling]` for 3 steps;
  only `[limit]` when the limit is the ceiling).

- [ ] **Step 1: Failing tests** (obstacle-free raster; spacing 2 m; tolerance 0.5 m):
  - flat ground at 100 m, grade 0.1, a 100 m straight from `Reach::pinned(100, 0.5)`: `Ok`, climb 0, lower-bound
    earthwork 0, reach [99.5, 100.5]; `profile` earthwork 0;
  - ground rising 0.2 per metre over 60 m, grade 0.1, cut = fill 6, start pinned: `band_step` is `Ok` with reach
    [6.0, 6.5] above the start ground (the band allows it), but `arrives(reach, 12.0, 0.5)` is false: the end anchor is
    12 m up and the road reaches at most 6.5 m (pinned anchors, spec "The feasibility band"); over 70 m `Err(Grade)`
    (the interval empties after 65 m);
  - the same 60 m with grade 0.2: arrives; `profile` max_grade ≤ 0.2 + 10⁻⁹;
  - **the switchback-1 numbers** (a synthetic line with the terrain of the real route: 90.8 m, net rise 21.4 m, a 20 m
    stretch at 54.9 %, the rest gentler, gravel): at the ceiling 0.36 the forward band passes, `band_step`'s lower-bound
    earthwork is 0, and `profile` gives earthwork > 0 with a fill or cut over 1 m (the spike reported 0; the true value
    on the real terrain is 46 m², cut 1.6 m, fill 2.3 m); at 0.18 `profile` fails (`Arrival`): a 90.8 m road cannot
    climb 21.4 − 1.0 m at 18 %;
  - `drawn_grade` of a 145.7 m line on a uniform slope rising 21.9 m: mean 0.150 ± 0.001; `leg_limit([Grav], ..)` is
    0.18 (aim); with a rise of 36.4 m (mean 0.25) the limit is 0.25; with a rise of 60 m (0.41) it is the ceiling 0.36;
    for narrow asphalt with mean 0.074 and a 20 m window of 0.264 the limit is 0.12 (the window is reported, not used);
  - `relaxed_limits(0.18, 0.36, 3)` is [0.18, 0.24, 0.30, 0.36]; `relaxed_limits(0.36, 0.36, 3)` is [0.36];
  - NaN ground: `Err(NoGround)`; a point outside the map: `ground_at` is `None`; a step through a stamped lake:
    `Err(Blocked(Water))`;
  - two consecutive steps carry the reach (a ramp passes as one 100 m step and as two 50 m steps with equal climb);
  - the lower bound never exceeds `profile`'s earthwork (a property test over seeded random terrains, fixed seed,
    regression file in the repo).
- [ ] **Step 2:** RED. **Step 3:** implement. **Step 4:** GREEN, `make check`.
- [ ] **Step 5:** Commit `feat: the feasibility band, profile and grade limit of road planning`.

### Task 4: Cost-to-go field (Sonnet)

**Files:** `crates/naves-core/src/reroute/field.rs`, `tests/reroute.rs`.

**Interfaces — Consumes:** `raster::{Obstacles, Wall}`, `band::{Band, band_step}`.

**Interfaces — Produces:** `reroute::field::Field` with `Field::build(min: (f64, f64), max: (f64, f64), target:
(f64, f64), step_m: f64, cell_m: f64, band: &Band, ground: &dyn Fn((f64, f64)) -> Option<f64>, obstacles: &Obstacles,
walls: &[Wall], clear: Clearance, stray_cap_m: f64, max_cells: usize) -> Field`, `Field::estimate(&self, p) -> f64`,
`Field::cells(&self) -> usize`. Dijkstra with an ordered heap (`total_cmp`, then cell index); a crate from Task 2's
check may replace it.

- [ ] **Step 1: Failing tests** (box (0, 0)–(200, 200), flat, step 6 so field cell 6 m, target (100, 100)):
  - `estimate((142, 100))` is `42 / 1.0824 − 6` ± 0.5;
  - a water wall x = 120..124, y = 0..180: `estimate((160, 100)) ≥ 150`; a stream wall (allowed 0) on the same line
    gives the same;
  - a cell enclosed by water: `estimate` 0;
  - a 100 km box: `cells() ≤ 1_000_000`;
  - the field ignores the grade (a 50 % slope gives the same estimate as flat ground with cut and fill 6 m);
  - two builds give the same estimates bit for bit.
- [ ] **Step 2:** RED. **Step 3:** implement. **Step 4:** GREEN, `make check`.
- [ ] **Step 5:** Commit `feat: the cost-to-go field of road planning`.

### Task 5: The arc search for one leg (Sonnet with the Opus helper)

**Files:** `crates/naves-core/src/reroute/dubins.rs`, `crates/naves-core/src/reroute/search.rs`, `tests/reroute.rs`.
A fuzz target `reroute_search` (poses and a small obstacle set from bytes, tiny budgets) with seeds.

**Interfaces — Consumes:** `raster`, `band`, `field`; `smooth::radius_at`.

**Interfaces — Produces:**

- `reroute::Pose { pub at: (f64, f64), pub heading: f64 }` (radians, counter-clockwise from +x).
- `reroute::Anchor { pub at: (f64, f64), pub heading: Option<f64>, pub straight_m: f64 }`.
- `reroute::dubins::{DubinsPath, Turn, dubins_paths}` as before: `dubins_paths(from: Pose, to: Pose, radius: f64) ->
  Vec<DubinsPath>` (sorted by length, then word), `DubinsPath::length`, `DubinsPath::points(&self, from, radius,
  step_m)`.
- `reroute::search::Weights { pub slope: f64, pub curve: f64, pub earth: f64, pub stray: f64 }`.
- `reroute::search::LegInput<'a> { pub start: Anchor, pub end: Anchor, pub radius: f64, pub step_m: f64, pub band:
  &'a Band, pub ground: &'a dyn Fn((f64, f64)) -> Option<f64>, pub obstacles: &'a Obstacles, pub walls: &'a [Wall],
  pub clear: Clearance, pub field: &'a Field, pub weights: Weights, pub stray_cap_m: f64, pub arrive_reach_m: f64,
  pub self_along_m: f64, pub budgets: &'a [(f64, usize)] }`.
- `reroute::search::Route { pub points: Vec<(f64, f64)>, pub length_m: f64, pub climb_m: f64, pub earthwork_m2: f64,
  pub cut_max_m: f64, pub fill_max_m: f64, pub max_stray_m: f64, pub planned_grade: f64, pub grade_limit: f64, pub
  relax_steps: usize, pub cost: f64, pub weight: f64, pub states: usize }` (`earthwork_m2` from `band::profile`).
- `reroute::search::LegOutcome { Found(Route), NoRoute { budget_exhausted: bool, states: usize, closest_m: f64 } }`.
- `reroute::RerouteError { NotFinite, Degenerate }` (`thiserror`).
- `reroute::search::search_leg(input: &LegInput<'_>) -> Result<LegOutcome, RerouteError>` (one grade limit, the one in
  `input.band`) and `reroute::search::search_leg_relaxed(input: &LegInput<'_>, limits: &[f64]) ->
  Result<LegOutcome, RerouteError>` (tries the limits in order; `relax_steps` is the index of the one that worked).

Rules from the spec: arrival only if the band's final interval meets the end anchor's tolerance (`band::arrives`);
at arrival `band::profile` of the whole candidate route replaces the lower-bound earthwork in its cost; the
self-clearance rule (two route samples more than `self_along_m` apart along the route at least the road clearance
threshold apart) is part of every step and arrival test.

- [ ] **Step 0: Crate check** of `dubins_paths` (source, panics on degenerate input, licence, maintenance).
- [ ] **Step 1: Failing tests (Dubins)**, radius 10.5: (0, 0, east) to (0, 21, west) shortest `π · 10.5` ± 10⁻⁹;
  (0, 0, east) to (50, 0, east) 50; (0, 0, east) to (10.5, 10.5, north) `π / 2 · 10.5`; equal poses one path of
  length 0; NaN input no path, no panic; **10 000 seeded random pose pairs** (fixed seed, positions in ±200 m, all
  headings): every returned path ends at the target within 10⁻⁹ m and 10⁻⁹ rad (the spike: 90 816 paths, worst
  2.5·10⁻¹³).
- [ ] **Step 2: Failing tests (search)**, flat ground at 100 m unless said, no obstacles, R = 10.5, step 6, weights
  from the constants:
  - straight: (0, 0, east) to (100, 0, east): found, length 100 ± 10⁻⁶, `max_stray_m` ≤ 0.01, `states` ≤ 500;
  - 90° corner, original line (0, 0) → (60, 0) → (60, 60), anchors (18, 0, east) and (60, 42, north): every
    three-point radius ≥ 9.9; the end pose exact within 10⁻⁶ m and rad; `max_stray_m` ≤ 6.0; length in [79.0, 84.0];
  - hairpin, legs x = 0 and x = 8 from y = 0 to 60, apex radius 4, anchors (0, 20, north) and (8, 20, south): found,
    every radius ≥ 9.9, `max_stray_m` ≤ 31.5, length > 92.6; with water at y ≥ 30: `NoRoute { budget_exhausted:
    false }`;
  - **switchback within the grade** (Review Focus 1; the controller's test with switchback-1's numbers): a gravel
    hairpin of drawn length 145.7 m on a uniform slope, anchors 21.9 m apart in height at the bottom of both legs, the
    limit from `leg_limit` (0.18, the drawn mean being 0.150): found; length ≥ `(21.9 − 1.0) / 0.18` = 116.1 m; the
    route's `band::profile` max_grade ≤ 0.18 + 10⁻⁹; `planned_grade` ≤ 0.189; every radius ≥ 9.9; the route's
    furthest point along the hairpin's axis is at least as far out as the drawn apex (it lies on the outside of the
    drawn loop); `relax_steps` 0;
  - the same with a rise of 36.4 m (drawn 0.25): `grade_limit` 0.25 and length ≥ `(36.4 − 1.0) / 0.25` = 141.6 m;
  - **relaxation**: the same hairpin with water beyond the apex so that only a 100 m route fits: not found at 0.18,
    found at the first relaxed limit 0.24 with `relax_steps` 1 and `planned_grade` ≤ 0.24 + 1.0 / length;
  - **pinned anchors**: anchors 40 m apart on a slope that rises 12 m between them, limit 0.27: every found route is
    at least `(12 − 1) / 0.27` = 40.7 m long; with the stray cap 0.5 m `NoRoute` (12 m in 40 m is 30 %), where the
    spike's anchors, free by ±6 m, would have passed;
  - **self-clearance** (Review Focus 2): a leg whose cheapest route would loop back across its own start (a 300°
    turn between anchors 12 m apart): the found route keeps every pair of samples more than `self_along_m` apart along
    it at least the road clearance threshold apart, or `NoRoute`; never a self-crossing route;
  - grade property: anchors (0, 0, east) and (40, 0, east), ground rising 0.75 per metre, limit 0.27: `NoRoute` or a
    route of length ≥ `(30 − 1) / 0.27` = 107.4 m; with the stray cap 0.5 m `NoRoute { budget_exhausted: false }`;
  - free end: (0, 0, east) to the dead end (50, 10): found, last point exact; junction start with `straight_m` 6.25:
    the first 6.25 m on the start heading within 10⁻⁶ m;
  - budgets `[(1.0, 10)]` on the hairpin: `NoRoute { budget_exhausted: true, states: 10, .. }`;
  - anchors 5 m apart both east: found, length 5 ± 10⁻⁶; anchors equal: `Err(Degenerate)`; a NaN anchor:
    `Err(NotFinite)`;
  - two runs of the corner give equal `Route`s bit for bit; the corner searched inside rayon pools of 1 and 4 threads
    gives equal routes.
- [ ] **Step 3:** RED. **Step 4:** implement. **Step 5:** GREEN, `make check`, the fuzz target 10 s.
- [ ] **Step 6: Check on the two switchbacks** (throwaway, not committed; under `/var/tmp/naves-plan/t5/`, 16G cap).
  Export the two switchback legs of `sites.md` from the spike's scratch copy (`/var/tmp/naves-plan/check/`, mode
  `site`: anchors, original line, the terrain along a 3-radius box as a small grid, the stamped roads) to JSON, and run
  `search_leg_relaxed` on them with the production code. Measure, for `PLAN_STRAY_MAX` 3, 4 and 5 and `W_STRAY` 0.25
  and 0.05: found, relaxation steps, length, planned grade against the drawn grade and the aim, largest move, real
  earthwork, deepest cut and fill, states. Write `t5/switchbacks.md`. Expected from the spec: switchback-1 needs at
  least 114 m at 18 %; switchback-2 stays near its 65.7 m. If switchback-1 is not found at 18 % within cap 3, record the
  smallest cap that finds it; Task 7b decides the cap on the whole map.
- [ ] **Step 7:** Commit `feat: the arc search of road planning for one leg`.

### Task 6: Zones, anchors and legs on the graph, and the write-back (Sonnet)

**Files:** `crates/naves-core/src/reroute.rs` (top level), `crates/naves-core/src/smooth.rs` and
`smooth/widen.rs` only to make `plan_chains`, `ChainPlan`, `Link` and `tight_runs` `pub(crate)` (no behaviour change;
the smoothing tests stay green untouched), `crates/naves-core/tests/reroute.rs`.

**Interfaces — Consumes:** `roadnet::{RoadGraph, Stretch, Crossing}`, `roadnet::crossings_of(&[Stretch]) ->
Vec<Crossing>` (`pub(crate)`), `roadnet::polyline_length`, `roadnet::over_crossings(&RoadGraph) ->
Vec<OverCrossing>`, `smooth::{resample, radius_at, SAMPLE_M, CHAIN_TURN_MAX_DEG, WaterTest}`,
`catalog::Family::width_m`, `heightmap::Heightmap`, `band::{drawn_grade, leg_limit, relaxed_limits}`,
`raster::exempt_samples`.

**Interfaces — Produces:**

- `reroute::RerouteParams { pub lead: f64, pub radius_margin: f64, pub step_m: f64, pub stray_max: f64, pub cut_m:
  f64, pub fill_m: f64, pub anchor_tol_m: f64, pub relax_steps: usize, pub weights: Weights, pub budgets:
  Vec<(f64, usize)>, pub max_zone_m: f64, pub junction_straight_m: f64, pub bridge_hold_m: f64, pub deck_anchor_m: f64,
  pub exempt_extend_m: f64, pub crossing_end_m: f64, pub raster_max_cells: usize, pub field_max_cells: usize }`,
  `Default` from the constants.
- `reroute::AnchorKind { Line, Junction(i64), DeadEnd(i64), BridgeHold, TunnelHold, RingCut(i64) }`.
- `reroute::KeptReason { NoRoute, BlockedByWater, BlockedByRoad, Held }` with `id(self) -> &'static str`.
- `reroute::ZoneOutcome { Planned, Kept { reason: KeptReason, detail: String } }`, `reroute::Candidate { Plate,
  CurvedBridge }`.
- `reroute::PlannedZone { pub id: String, pub at: (f64, f64), pub stretches: Vec<String>, pub legs: usize, pub
  anchors: (AnchorKind, AnchorKind), pub junction: Option<i64>, pub partner: Option<String>, pub outcome:
  ZoneOutcome, pub radius_before_m: f64, pub radius_wanted_m: f64, pub radius_after_m: f64, pub length_before_m: f64,
  pub length_after_m: f64, pub max_stray_m: f64, pub drawn_grade: f64, pub drawn_grade_window: f64, pub
  planned_grade: f64, pub grade_limit: f64, pub relax_steps: usize, pub climb_m: f64, pub earthwork_m2: f64, pub
  cut_max_m: f64, pub fill_max_m: f64, pub states: usize, pub weight: Option<f64>, pub budget_exhausted: bool, pub
  original_feasible: bool, pub candidate: Option<Candidate> }`.
- `reroute::RerouteReport { pub zones: Vec<PlannedZone>, pub skipped: Vec<(String, String)>, pub tight_before_m: f64,
  pub tight_after_m: f64, pub max_stray_m: f64, pub steeper_than_drawn: usize, pub reverted: usize, pub states: usize }`.
- `reroute::reroute_graph(graph: &RoadGraph, ground: &Heightmap, min_radius: &dyn Fn(&Stretch) -> f64, in_water:
  WaterTest<'_>, streams: &[Vec<(f64, f64)>], params: &RerouteParams) -> (RoadGraph, RerouteReport)`. In this task a
  failed leg is `Kept { NoRoute }` or `Kept { Held }`; the other reasons and the cross-check come in Task 7.

What the obstacles of a leg are, from the spec: water and streams (streams the original does not cross as walls with
`allowed` 0, the others with their count); every other stretch stamped at its reach except its `exempt_samples`
(near the original by `thr(road clearance)` plus its reach, extended by `PLAN_EXEMPT_EXTEND_M`), its centre line a
wall with the original's crossing count; the own chain outside the leg stamped except its first `PLAN_SELF_FREE` from
each anchor; decks and tunnels stamped, never exempt, unless within `PLAN_DECK_ANCHOR_M` of a fixed anchor; a deck
crossing of the original line is `held` (`crosses_deck`). A leg whose original line fails the test is still searched
(`original_feasible: false`).

All fixtures: a flat heightmap of 513 × 513 vertices at 4 m (2048 m) unless said, City roads (tightest arc 10 m,
R = 10.5), a corner at (500, 500) unless said otherwise.

- [ ] **Step 1: Failing tests (zones and anchors):**
  - a graph without a tight run: the graph comes back equal, the report has no zone;
  - town corner: J1 (400, 500) → corner (500, 500) → J2 (500, 600), both J with a side road: one zone, planned, both
    anchors `Line` (lead 2: the anchors are 21 m from the tight run); J1 and J2 bit for bit; the sets of stretch ids and
    node ids equal; tight length after 0; `radius_wanted_m` 10.0;
  - corner 15 m after J1: anchor `Junction(J1)`; the arm's direction at J1 unchanged within 10⁻⁹ rad; the route's
    first 6.25 m on y = 500 within 10⁻⁶ m; planned;
  - corner 15 m before a dead end: anchor `DeadEnd`; the dead end bit for bit;
  - a 30 m bridge 10 m after the corner: the bridge's points bit for bit, every point within 25 m of its ends unchanged;
  - a junction 8 m from a bridge end, the corner 15 m past the junction: the deck is part of the anchor (within 12 m),
    the leg is searched and planned (spike 8.2);
  - a tight corner 12 m before a bridge (the hold leaves no room): `Kept { Held }`, candidate `Plate`, bit for bit;
  - a tight kink on a 6 m stretch between two junctions: `Kept { Held }` with detail `anchors_close`, no panic;
  - a two-stretch node inside the zone moves; its crossing has the new point, equal to both stretches' end points;
  - a zone whose original line passes under a bridge deck: `Kept { Held }`, detail `crosses_deck`;
  - **the grade of a leg**: a corner on a slope of 10 % along the road (City, aim 14 %): `drawn_grade` 0.10 ± 0.005,
    `grade_limit` 0.14, `planned_grade` ≤ 0.14 + 1.0 / length.
- [ ] **Step 2: Failing tests (obstacles on the graph):**
  - the hairpin of Task 5 with a road across its top 35 m beyond the apex (clearance threshold not reached by the
    original): the route's smallest distance to that road ≥ its reach plus `thr(road clearance)`;
  - **per-place exemption**: a side road that the original line passes 8 m from at one place, and that runs 20 m
    outside the corner 80 m further on: the route may pass the first place as near as the original, but keeps the
    clearance threshold from the far part; it crosses the side road as often as the original, counted away from the
    anchors;
  - **switchback stack** (Review Focus 2): a chain with three hairpins whose legs are 25 m apart; each planned leg
    keeps at least the clearance threshold from the own chain outside its leg beyond `PLAN_SELF_FREE`, or is kept
    `blocked_by_road`; no planned route crosses the chain's other legs;
  - **streams**: a stream 15 m outside the corner, not crossed by the original: no planned route crosses it; a stream
    the original crosses once: the route crosses it once;
  - ring: tight bend at its lowest node `Kept { Held }`, anchor `RingCut`; elsewhere planned, first and last point
    equal bit for bit;
  - map edge: the corner at (8, 500) with the bulge towards x < 0: no written point with x < `core_reach_m(5, 4)`
    unless the original line has one; a road that runs to the edge: planned, its end on the edge unchanged;
  - an original line inside another road's earthwork (`original_feasible: false`) whose route can leave it: planned;
  - one stretch with a NaN point: listed in `skipped`, the town corner elsewhere planned exactly as without it; a
    stretch at 10⁹ m: skipped or kept, `states` bounded by the budgets, no panic.
- [ ] **Step 3: Failing tests (determinism):** the features in reverse order give an equal graph and report; rayon
  pools of 1, 4 and 16 threads give equal graphs and reports bit for bit (the spike: identical at 1, 10, 16).
- [ ] **Step 4:** RED. **Step 5:** implement. **Step 6:** GREEN, `make check`.
- [ ] **Step 7:** Commit `feat: road planning re-routes tight zones of the road graph`.

### Task 7: Failure reasons, routes against each other, the report (Sonnet)

**Files:** `crates/naves-core/src/reroute.rs`, `tests/reroute.rs`.

**Interfaces — Produces:** the reasons `BlockedByWater` (diagnostic without water areas and streams) and
`BlockedByRoad` (without other roads), each at the ceiling grade with the last budget pair only; the sequential
cross-check in zone order; `RerouteReport::summary(&self) -> String`; `RerouteReport::kept(&self, reason:
KeptReason) -> usize`.

- [ ] **Step 1: Failing tests:**
  - hairpin with water outward and inward of the legs from y ≥ 30: `Kept { BlockedByWater }`, candidate
    `CurvedBridge`, line bit for bit; the same with a stream wall: `BlockedByWater`;
  - the same with a road instead: `Kept { BlockedByRoad }`, candidate `None`;
  - **routes against each other** (Review Focus 3): two hairpins on parallel roads whose apexes face each other 30 m
    apart (original lines not within the threshold) and whose routes each bulge into the gap: the zone of the lower
    chain id is planned, the other `Kept { BlockedByRoad }` with detail `route_of <id>`, its line bit for bit,
    `reverted` 1; swapping the stretch order of the input changes nothing;
  - a leg planned only at a relaxed limit: `relax_steps` > 0, `steeper_than_drawn` counts it when its planned grade is
    more than 0.02 above the drawn grade, and its `road_curve_planned` message (Task 8) can say by how much;
  - fifty town corners with budgets `[(1.0, 50)]`: fifty `Kept { NoRoute }` with `budget_exhausted`, report
    `states ≤ 50 · (4 · 50 + 2 · 50)`, the same report on a second run (Review Focus 6);
  - a route that the final check rejects (forced by a check radius above the planning radius): `Kept { NoRoute }`,
    detail `check_failed`;
  - `summary()` of a report with 3 zones (2 planned, 1 held, 1 planned steeper than drawn) is exactly
    `Road planning: 3 zones, 2 planned, 1 kept (no_route 0, held 1, blocked_by_water 0, blocked_by_road 0), tight
    length <before> m -> <after> m, largest move <m> m, 1 steeper than drawn` with the fixture's numbers.
- [ ] **Step 2:** RED. **Step 3:** implement. **Step 4:** GREEN, `make check`.
- [ ] **Step 5:** Commit `feat: road planning says why a bend was kept`.

### Task 7b: Re-check on the map, then freeze the constants (Sonnet runs, controller with the Opus reviewer decides)

The spike measured without the grade rule, the pinned anchors, the real earthwork, the per-place exemption, the
self-clearance and the cross-check. This task measures the production stage with all of them before Task 8 wires it
into the build.

**Files:** none in the repo except the spec's constants table and this plan's Global Constraints. A throwaway example
under `/var/tmp/naves-plan/t7b/` (worktree with its own target dir `/var/tmp/naves-plan/t7b-target`) that builds the
cleaned graph of the example at 2048 as `fitted_graph_with` does and calls `reroute_graph`.

- [ ] **Step 1:** The six sites of `sites.md` (by position; the zone ids are lead-4 ids) and the whole map, with the
  constants of the spec, on 1 and 16 threads (16G cap). Record per site and in total: zones by outcome and reason,
  tight length after, largest and p95 move, legs steeper than drawn (count, worst), relaxation steps used, real
  earthwork (median, p95, max; deepest cut and fill), routes reverted against earlier routes, states (median, p95,
  max), time. Check the outputs at 1 and 16 threads are equal.
- [ ] **Step 2:** One-at-a-time variations on the whole map: `PLAN_STRAY_MAX` 3, 4, 5; `W_STRAY` 0.25, 0.05; `W_EARTH`
  0.5, 1.0, 2.0; cut = fill 3, 6, 12; `PLAN_ANCHOR_TOL_M` 0.25, 0.5, 1.0; `PLAN_LEAD` 2, 3; the test threshold `thr(c)`
  against `c + res / 2`; `PLAN_EXEMPT_EXTEND_M` 0, 10, 20; budgets as given and doubled. And the 56 City corners of
  60 to 120° between junctions of the spike (by position): planned share at lead 2 and lead 3.
- [ ] **Step 3: Decide** (controller with the Opus reviewer), by these rules, and write `t7b/answers.md`:
  - the stray cap is the smallest of 3, 4, 5 at which both switchbacks are planned without relaxation and the map
    plans no fewer legs than at the next larger value; above 5 the switchbacks are kept and reported;
  - `W_EARTH` and cut/fill: the smallest values that give the same planned legs as the largest, now on real earthwork;
  - the anchor tolerance: the largest value with no planned leg above its limit on the profile;
  - the budgets: four times the largest count to a first arrival at weight 1 on the six sites, as before;
  - the test threshold: the bare clearance plus half a cell only if it plans more legs and no planned route comes within
    the clearance of another road's earthwork on an exact distance check;
  - if more than 5 % of the planned legs need a relaxation step, report to the maintainer before going on.
- [ ] **Step 4:** Update the spec's constants table (drop the **(re-check)** marks, add the measurements) and the
  Global Constraints; the constants in `reroute.rs` with their tests if any value changed. `make check`.
- [ ] **Step 5:** Commit `docs: road planning constants after the re-check` (with the code change, if any, as
  `fix:`).

### Task 8: Road planning in the build (Sonnet; DeepSeek for the marked steps)

**Files:** `crates/naves-export/src/generate.rs`, `crates/naves-core/src/roadplace.rs` (`PlacedNetwork`),
`crates/naves-core/src/validate.rs`, `crates/naves-core/tests/validate.rs`, `crates/naves-export/src/build.rs`,
`crates/naves-export/tests/smoothing.rs` and `tests/build.rs` (callers of `fitted_graph_with`), a new
`crates/naves-export/tests/planning.rs`, `crates/naves/src/mcp.rs`, the binary's summary print, the `road-analysis`
and `road-sites` examples, `docs/manual/src/getting-started.md`, `troubleshooting.md`, `mcp.md`, `CHANGELOG.md`.

**Interfaces — Consumes:** `reroute::{reroute_graph, RerouteParams, RerouteReport, REROUTE_ENABLED}`,
`smooth::SMOOTH_ENABLED`, `features::water_lines` (already read into `Read::water`).

**Interfaces — Produces:**

- `generate::Stages { pub reroute: bool, pub smooth: bool }` with `Stages::DEFAULT`.
- `generate::fitted_graph_with(features, catalog, base, stages: Stages, in_water, streams: &[Vec<(f64, f64)>])`
  (the `smooth: bool` parameter becomes `stages`; the stream lines are passed through to `reroute_graph`);
  `fitted_graph(features, catalog, base, in_water, streams)`; `FittedGraph::reroute: Option<RerouteReport>`,
  `FittedGraph::reroute_time: Duration`.
- `generate::plan_roads_with(…, stages: Stages)`; `plan_roads` calls it with `Stages::DEFAULT` and passes its `water`
  (the stream lines) on.
- `PlacedNetwork::reroute: Option<RerouteReport>`, `Generated::reroute`, `BuildOutput::planning: Option<String>`.
- `Rule::RoadCurvePlanned` (`road_curve_planned`, information) and `Rule::RoadCurveNotPlanned`
  (`road_curve_not_planned`, warning); `Rule::ALL: [Rule; 33]`; `validate::reroute_violations`.
- `build.rs::planning_json` for `roads.planning`.

- [ ] **Step 1: Failing tests:**
  - `fitted_graph_with` with `Stages { reroute: false, smooth: false }` on the smoothing test's fixture equals today's
    graph bit for bit, `reroute` `None`;
  - with `reroute: true` on a fixture with one tight corner: `reroute` `Some`, one zone planned, `graph` differs only on
    that chain; with a stream beside the corner passed in, no planned route crosses it;
  - `fitted_graph(..).reroute.is_some() == REROUTE_ENABLED`;
  - `Rule::RoadCurvePlanned.id() == "road_curve_planned"`, `Rule::RoadCurveNotPlanned.id() ==
    "road_curve_not_planned"`; a report with one planned, one planned at a relaxed limit and one held zone gives two
    `road_curve_planned` (the relaxed one's message says "<x> points steeper than drawn") and one
    `road_curve_not_planned`, at the zones' `at`;
  - `planning_json(None)["enabled"] == false`; with a report: `enabled`, `zones`, `tight_before_m`, `tight_after_m`,
    `steeper_than_drawn`, `reverted`, every zone with `outcome`, `reason`, `candidate`, `drawn_grade`,
    `planned_grade`, `grade_limit`, `relax_steps`, `earthwork_m2`;
  - the MCP tool description lists both new rule ids.
- [ ] **Step 2:** RED. **Step 3:** implement the wiring, the progress line `Planning the tight curves (<s> s)` and the
  summary line.
- [ ] **Step 4 (DeepSeek):** `road-analysis`: read `STAGES` from the environment (`off`, `plan`, `smooth`,
  `plan+smooth`; default `Stages::DEFAULT`), print `STAGES reroute=<bool> smooth=<bool>`, `TIMING reroute <s> s`,
  `PLAN <summary>`, `PLAN_REASON <reason> <count>`, `PLAN_KEPT <id> <reason> <x> <y> <radius>` for each kept zone,
  `PLAN_STEEP <id> <x> <y> drawn <g> planned <g> limit <g> relax <n>` for each leg planned steeper than drawn by more
  than `PLAN_STEEPER_MARGIN`, `PLAN_FIXED junctions moved <n>, dead ends moved <n>, bridge ends moved <n>, tunnel ends
  moved <n> (all must be 0)`.
- [ ] **Step 5 (DeepSeek):** the manual: getting-started "Roads" (what planning does: radius, not steeper than drawn or
  the family's aim, streams, other roads; that it is off), troubleshooting (the two rules, the four reasons, the
  candidates, a relaxed grade), mcp.md (the rule list); `CHANGELOG.md` under Unreleased, Added: "Road planning
  re-routes curves tighter than the road pieces ... no steeper than the road as drawn or its type's aim ... switched off
  in this release (`REROUTE_ENABLED`) ... with it on, roads inside planned curves move by up to three minimum radii, so
  edits keyed by slot can be orphaned (`orphaned_edit`) ... `roads.planning` in `build/report.json`, findings
  `road_curve_planned` and `road_curve_not_planned`".
- [ ] **Step 6:** GREEN, `make check` (with `make docs`).
- [ ] **Step 7:** Commit `feat: road planning in the build, switched off`.

### Task 9: Judging runs (controller with the Opus reviewer; Sonnet runs them)

**Files:** none in the repo. Output under `/var/tmp/naves-plan/judge/`.

- [ ] **Step 1:** Rebuild the release example once (log HEAD and the binary's mtime). Check the machine; one run at a
  time, 16G cap each.
- [ ] **Step 2:** Four runs of `road-analysis` at 2048 with the same binary: `STAGES=off`, `plan`, `smooth`,
  `plan+smooth`. Logs `judge/<stages>/ra-2048.log`, `RULES_OUT=judge/<stages>/rules.tsv`.
- [ ] **Step 3:** Write `judge/compare.md`, one table, four columns:
  - tight length (baseline 16 135 m; smoothing alone 4088 m; spike 1547 m) and zones (2261 on the drawn line);
  - zones by outcome and reason; the full list of kept zones with reason, candidate, position and radius;
  - largest and p95 move (spike 28.5 m, 16.6 m);
  - planned legs steeper than drawn: count, how many needed a relaxation, the worst with drawn and planned grade
    (the `PLAN_STEEP` lines); routes reverted against earlier routes (spike: 26 would be);
  - drivability on planned legs: `road_not_drivable`, `road_grade`, `road_gap`, `road_misses_terrain` inside planned
    zones in `plan` against the same places in `off`;
  - `road_gap` count, total length, largest (389, 1618 m, 131 m); `road_not_drivable` (141), `road_conflict` (797),
    `road_misses_terrain` (3752), `road_grade`, `road_in_water`, `earthworks_capped`, `join_without_junction`,
    `orphaned_edit`, every other rule;
  - the `PLAN_FIXED` line (all must be 0);
  - placement time (about 550 s) and `TIMING reroute` (spike 5.3 s on 16 threads) separately.
- [ ] **Step 4:** Judge against the spec's bar (a proposal; the maintainer decides): tight length with `plan` at most
  8068 m (the spike reached 1547 m); none of `road_gap`, `road_not_drivable`, `road_conflict`, `road_misses_terrain`
  worse than `off` by more than 5 % without a reason that can be named; planned legs steeper than drawn by more than 2
  points at most 1 % and none above the ceiling; planning under 60 s; placement not more than 10 % slower. A miss is
  reported with the numbers; nothing is tuned until the numbers are understood (one round at most, each change its own
  commit with a test).
- [ ] **Step 5:** From `plan+smooth` against `plan`: did the smoothing tighten planned arcs? Record it for the open
  point.
- [ ] **Step 6:** Pick ten sites for the game: the two switchbacks, two town corners, the junction hairpin, the bend
  beside water, one plate candidate, one curved-bridge candidate, one zone with a moved two-stretch node, one straight
  road as a control. Write them as a `sites.json` for `tools/ingame-check`.

No commit, unless Step 4's tuning changes a constant (then the spec's table changes in the same commit).

### Task 10: Checkpoint, the game (controller with a Sonnet agent)

- [ ] **Step 1:** Build the map twice from this branch: `STAGES=off` and `STAGES=plan` (log HEAD and the binary's mtime
  each time), under `/var/tmp/naves-plan/game/`.
- [ ] **Step 2:** `tools/ingame-check/run.sh` on each build with the same `sites.json` (24G cap, non-default ports, one
  game at a time, headless sway only). Drive up both switchbacks, not only photograph them. Stop the game when done;
  remove the `@naves*` symlinks.
- [ ] **Step 3:** Send the pairs to the maintainer with one line per site: what planning changed, whether the road is
  drivable and usable, whether it feels steeper than before, anything that looks wrong.
- [ ] **Step 4:** The maintainer decides whether `REROUTE_ENABLED` becomes `true`. If yes: a commit `feat!: road
  planning is switched on` that flips the constant, updates the manual and adds the **Breaking** note for orphaned
  edits to `CHANGELOG.md`; `make check`.

### Task 11: Final review (Opus)

- [ ] Review the whole branch against the spec and this plan: every spec rule has a test; the Review Focus tests exist
  and assert what is listed; the constants in the code equal the spec's table; no fixed node can move and no planned
  leg can exceed its grade limit (read the write-back and the band, not only the tests); determinism (no hash-map
  iteration, budgets by count, `total_cmp`); no panics on input; coverage not below the base; the manual and the CLI
  help agree.
- [ ] Update the spec's status line with the result and its open points with what the judging showed.
- [ ] Hand the list of kept zones (from Task 9) to the controller as the start of the plate and curved-bridge tasks.

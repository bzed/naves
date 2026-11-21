<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Lots (Car Parks, Camp Sites and Similar) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Read the lot areas (`amenity=parking`, `amenity=tourist_bus_parking`, `highway=rest_area`,
`tourism=camp_site`, `tourism=caravan_site`); take every road wholly inside a lot out of the network and do not paint
it; cut access roads back to just inside the edge; paint every lot paved or grass after its own `surface` tag or the
roads that serve it; replace its ground by a plane or a curvature-capped quadratic with no interior hump or hollow,
blended into the terrain; tie access roads and through roads to that surface in the height solve; report everything.

**Architecture:** `osm::read_land` also keeps the lot areas (`OsmLand::lots`). `features` and `roadnet` carry the
roads' `surface` tag as metres paved and unpaved. A new module `crates/naves-core/src/lot.rs` turns the areas into map
polygons (`areas`), groups the ones that touch, fits a smooth surface per group and decides each outcome (`assess`) and
the surface class (`surfaces`). A new clean-up step `crates/naves-core/src/netclean/lot.rs` removes the roads inside
and cuts the access roads; it runs first in `netclean::clean_with`. `conform` gets a polygon patch (`AreaPatch`),
`junctions` a fixed-height entry junction (`JunctionKind::LotEntry`), `profile::model` the lot surface as the ground
target of through roads. `naves-export`'s `generate` wires it behind `LOTS_ENABLED` and a per-call stage flag, paints,
and reports.

**Tech Stack:** Rust 1.99; no new dependency planned. AGENTS.md's crate review applies if one seems needed: `geo`
(polygon contains, area, distance) and `nalgebra` (a 6 × 6 least-squares solve, a 2 × 2 eigen decomposition) are
the first to check, against a few lines of our own (the repo's even-odd tests in `osm.rs` and `surface.rs`, Cramer's
rule in `junctions::ring_plane`); a 6 × 6 system does not justify `nalgebra`'s tree unless the review says otherwise.

**Spec:** `docs/superpowers/specs/2026-11-21-car-parks-design.md`

**Implementers:** Sonnet. **Reviews:** Opus, after every task. Steps marked **(DeepSeek)** are simple and fully
specified; the controller may delegate them to a cheaper model.

## Global Constraints

- AGENTS.md binds: test first (RED then GREEN), `make check` green before every commit, coverage not below the value
  on the base commit (and never below 85 %), no panics on input (`unwrap`, `expect`, indexing on untrusted data are
  denied in library code), `#[expect(lint, reason)]` never `#[allow]`, no truncating or sign-changing casts, checked
  arithmetic on sizes, sort and compare floats with `total_cmp`, no `==` on floats, deterministic output (no `HashMap`
  or `HashSet` iteration order), `thiserror` errors in the library, `tracing` not `println!` outside the binary and
  examples, rustdoc on public items, docs and CHANGELOG in the behaviour commit, SPDX headers on new files, no game
  data in the repo (tests use synthetic polygons, graphs and heightmaps; the slot-4 colour and the texture names are
  facts written into the docs, not game files). Property tests use `proptest` with a fixed seed and keep their
  regression files in the repo.
- The lot reading runs inside `read_land`, so the `osm_read` fuzz target covers it; add a seed with a car park way, a
  camp site way and a lot multipolygon. The `surface` tag on roads runs inside `classify`, covered by `osm_ways`; add a
  seed with a tagged service road. Seeds run in `make test`.
- Every process gets a hard memory limit: `systemd-run --user --scope --quiet -p MemoryMax=12G -p MemorySwapMax=1G`
  for cargo and `make check`, 16G for `road-analysis` and the measurement programs, server 8G, client 14G,
  `ingame-check` 24G. Never raise a cap; report an OOM kill. The sum of what parallel agents run must fit the machine
  (24 threads, 60 GB): check `nproc`, `free`, `uptime` before a long run; one heavy job at a time.
- Temp and build folders under `/var/tmp` (never `/tmp`, never `$HOME`); this plan's scratch is
  `/var/tmp/naves-lots/`. Shared target dir `/var/tmp/naves-target`, never deleted; after a worktree agent, touch the
  sources before `make check`.
- Never start the game outside the checkpoint task. Wine/Proton only inside headless sway. Never touch the user's
  DayZ profile folder or client settings. Non-default server ports. Remove `@naves*` symlinks from the DayZ install
  afterwards. One game at a time. Reading game files (configs, PAAs, rvmats) with dayzmake is allowed; never edit
  them.
- Figures are quoted for Berchtesgaden at half scale (`scale.horizontal = 0.5`, `scale.height = 0.5`), terrain size
  2048 (4 m cells).
- Constants, verbatim from the spec (all `pub const` in `lot.rs`): `LOTS_ENABLED = false`, `LOT_INSIDE_TOL_M = 1.0`,
  `LOT_INSIDE_SHARE = 0.5`, `LOT_ENTRY_M = 4.0`, `LOT_PLANE_FIT_M = 0.5`, `LOT_MIN_RADIUS_M = 600.0`
  (`limits::min_vertical_radius(Family::Asf1)`), `LOT_RELIEF_LENGTH_M = 24.0`, the relief target
  `LOT_RELIEF_LENGTH_M² / (8 · LOT_MIN_RADIUS_M)` (0.12 m), `LOT_MAX_MOVE_M = 3.0`, `LOT_STEEP =
  limits::CROSS_FALL_LIMIT` (0.20), `LOT_MIN_VERTICES = 3`, `LOT_WATER_MAX = 0.5` (samples are on a fixed 1 m grid, not a constant); the lot patch
  has rank 0; reused, not copied: `conform::SHOULDER_CELLS`, `conform::core_reach_m`, `limits::RAMP_JOINT_MIN_M`.
  Paved families `Asf1`, `Asf2`, `Asf3`, `City`; grass `Grav`, `Mud`. The paved and unpaved `surface` values are the
  spec's two lists, verbatim.
- With the stage off (the constant, or the per-call flag) every output is the same as on the base commit, bit for
  bit: graph (also its stretch ids: the `surface` metres do not cut stretches), heightmap, surface grid, tile masks
  and materials, painted paths, report (apart from `roads.lots: {"enabled": false}`).
- Only roads wholly inside a lot are removed; no access road is ever removed; nothing is removed or cut silently.
- New node ids are above all existing ids, handed out in sorted order; a cut access road keeps its stretch id.

## Review Focus

The inputs most likely to bite. Each has its test in the task named.

1. **A polygon with a hole or a self-intersection.** A 40 × 40 m lot with a 10 × 10 m hole; a bow-tie outline.
   Expected: a point in the hole is not inside, a road wholly in the hole stays, the hole's ground is not used for the
   fit and keeps its height beyond the shoulder; the bow-tie's area is counted from samples and nothing panics. Tests
   in Task 2 (inside, area), Task 4 (fit), Task 7 (shaping).
2. **A road that enters, leaves and re-enters.** A dead-end stretch whose line crosses the edge three times before it
   ends inside. Expected: cut only after its last entry, the earlier parts unchanged, one entry. Test in Task 5.
3. **Two lots sharing an edge.** Expected: one group, one surface (equal heights on the shared edge after the
   shaping), a road across the shared edge wholly inside the union removed, each lot reported on its own. Tests in
   Task 4, Task 5, Task 7.
4. **A lot whose only access is through another lot.** Expected: it is smoothed with its group, takes the group's
   surface class, and the connectivity check (a group is one node) finds no split. Tests in Task 5 and Task 6.
5. **A lot at the map edge.** A lot that reaches past the map edge, with a road cut there (negative node id) ending
   inside it. Expected: smoothed on its part on the map, no sample read outside it, the cut end untouched. Tests in
   Task 4 and Task 5.
6. **A road drawn on the outline.** Expected: within the tolerance everywhere but not inside
   (`LOT_INSIDE_SHARE`); it stays. Test in Task 5.
7. **A lot of one kind inside or overlapping a lot of another kind** (a car park inside a camp site). Expected: one
   group and one surface over the union; the roads inside the car park and inside the camp site removed once; each
   lot keeps its own surface class and the car park is painted on top (largest first); both reported. Tests in Task 4
   (group), Task 5 (removal), Task 6 (classes and paint order).
8. **A surface that would hump.** Bowl-shaped ground whose best quadratic has its lowest point inside the lot.
   Expected: the plane is used, no interior extremum. Test in Task 4.

---

### Task 1: Baseline, the code's own measurement, and the tarmac layer facts (Sonnet; no production code)

**Files:** none in the repo. Output under `/var/tmp/naves-lots/baseline/`. The design's scratch programs are in
`/var/tmp/naves-parking/` (`scan/`: every area tag, the plane and capped-quadratic fits, the local relief; `measure/`:
the first version; both are cargo packages with path dependencies on the repo's crates).

- [ ] **Step 1:** On the base commit (log `git rev-parse HEAD` and the `naves` binary's mtime), run `road-analysis`
  on Berchtesgaden at 2048 once (16G cap, `RULES_OUT=…/rules.tsv`), log `baseline/ra-2048.log`. Record every rule
  count, in particular `road_gap` (count, total, largest), `road_conflict`, `road_not_drivable`,
  `road_misses_terrain`, `join_without_junction`.
- [ ] **Step 2:** Copy `scan/` to `baseline/scan/`, restrict it to the spec's lot tags, include areas under 20 m²,
  use the code's serving reach (`core_reach_m(half width, 4.0) + 4.0`), the stretch's own paved and unpaved metres
  (every way of the stretch, not only its lowest id), and the spec's form rule (plane under 0.5 m, else the capped
  quadratic, plane on an interior extremum, the λ guard at 3 m). Run it (16G cap). Write `baseline/lots.tsv` (one row
  per lot: id, kind, position, polygon as WKT, area, outcome, form, move p95 and largest, slope, local relief before,
  surface class and its source, roads inside, access) and `baseline/baseline.md` with the spec's tables re-counted.
  If a count moves by more than five lots, or any named lot changes its outcome, update the spec's tables in Task 2's
  commit.
- [ ] **Step 3:** From `baseline/lots.tsv` and `rules.tsv`, count every rule within 50 m of a smoothed lot, for Task 9.
- [ ] **Step 4: the tarmac layer.** With dayzmake (read only) on the user's install: read the exact file names of
  the `en_tarmac` and `en_tarmacold` textures (`_nopx` and `_ca`) from `surfaces_bliss` (as the four existing layers'
  names were read; do not assume them), and the average colour of each `_ca`. List Livonia's tile materials, find one
  whose name lists five or more layers, read its mask PAA (`paa_to_image`) and material, and note the colour (RGBA)
  that selects slot 4. Write `baseline/slot4.md`. If slot 4 cannot be pinned down, say so: Task 6 then paints paved
  lots soil and the controller tells the maintainer.

No commit.

### Task 2: Reading the lot areas (Sonnet)

**Files:** Modify `crates/naves-core/src/osm.rs`, `crates/naves-core/src/lib.rs`; create
`crates/naves-core/src/lot.rs`, `crates/naves-core/tests/lot.rs`; extend `crates/naves-core/tests/osm.rs`; seeds under
`fuzz/corpus/osm_read/`.

**Interfaces — Consumes:** `osm::{read_land, OsmLand, assemble, RawWay}`, `heightmap::MapGeo::to_map`.

**Interfaces — Produces:**

- `osm::LotPolygon { pub id: i64, pub relation: bool, pub part: usize, pub tags: BTreeMap<String, String>,
  pub outer: Vec<(f64, f64)>, pub holes: Vec<Vec<(f64, f64)>> }` (lat, lon, closed rings); kept tags `amenity`,
  `tourism`, `highway`, `parking`, `surface`, `layer`, `building`, `location`, `access`, `caravan_site:type`.
- `OsmLand::lots: Vec<LotPolygon>`: ways first, then relations, each in id order, parts in ring order. A way or a
  `type=multipolygon` relation with `amenity=parking`, `amenity=tourist_bus_parking`, `highway=rest_area`,
  `tourism=camp_site` or `tourism=caravan_site`; `amenity=parking_space` and `tourism=camp_pitch` are not kept. The
  water, rock and built-up polygons are unchanged.
- `lot::LotKind { CarPark, BusParking, RestArea, CampSite, CaravanSite }` and `lot::Placement { Ground, StreetSide,
  NotAtGround }`.
- `lot::LotArea { pub id: String, pub kind: LotKind, pub placement: Placement, pub outer: Vec<(f64, f64)>,
  pub holes: Vec<Vec<(f64, f64)>>, pub area_m2: f64, pub surface_tag: Option<Paving> }` in map metres; `id` is
  `osm/way/<id>` or `osm/relation/<id>/<part>`; `Paving` comes from Task 3 (in this task a local stub enum that
  Task 3 replaces, or Task 3 first: the controller orders them; both orders work).
- `lot::areas(land: &OsmLand, frame: &MapGeo) -> (Vec<LotArea>, usize)`: the areas in id order and the number
  skipped (fewer than three distinct corners, a point that cannot be projected or is not finite).
- `lot::contains(area: &LotArea, p: (f64, f64)) -> bool` (even-odd, outer less holes) and
  `lot::edge_distance(area: &LotArea, p: (f64, f64)) -> f64`.

- [ ] **Step 1: Failing tests (osm):** a PBF (built the way `tests/osm.rs` builds its input) with one way tagged
  `amenity=parking, parking=surface, surface=asphalt` gives one `LotPolygon` with those tags; ways tagged
  `tourism=camp_site`, `tourism=caravan_site`, `highway=rest_area`, `amenity=tourist_bus_parking` give one each; a
  multipolygon relation with two outer rings and one inner gives two polygons, parts 0 and 1, the hole on the ring
  that contains it; `amenity=parking_space` and `tourism=camp_pitch` give none; a file with water and lots gives the
  same `polygons` as without the lots; the result is the same with the files in the other order.
- [ ] **Step 2: Failing tests (areas):** kinds from the tags; placement: `parking=street_side`, `lane`, `layby` →
  `StreetSide`; `parking=underground`, `multi-storey`, `rooftop`, `building=yes`, `location=underground`, `layer=-1`,
  `layer=1` → `NotAtGround`; else `Ground`. A 40 × 40 m square has `area_m2` 1600 ± 1; with a 10 × 10 m hole 1500 ± 1,
  and `contains` is false at the hole's centre and true 5 m inside the outer edge; a bow-tie of two 10 × 10 m right
  triangles meeting at a point has `area_m2` 100 ± 3 and `contains` is true in each triangle (Review Focus 1); a ring
  with a NaN point or only two distinct corners is skipped and counted; `edge_distance` of the square's centre is 20;
  `surface_tag` `Paved` for `asphalt`, `Unpaved` for `grass`, `None` for `foo`; ids `osm/way/7` and
  `osm/relation/9/1`; the same output for reversed input.
- [ ] **Step 3:** RED. **Step 4:** implement; the area is counted on a fixed 1 m grid of even-odd samples.
  **Step 5:** GREEN, the fuzz seeds, `make check`.
- [ ] **Step 6:** Commit `feat: the lot areas of OpenStreetMap are read` (nothing consumes them yet; no CHANGELOG
  line).

### Task 3: The roads' surface tag (Sonnet; DeepSeek allowed for Steps 1 to 3, the change is mechanical)

**Files:** `crates/naves-core/src/features.rs`, `crates/naves-core/src/roadnet.rs`, `crates/naves-core/src/lot.rs`
(the `Paving` lists), tests `crates/naves-core/tests/features.rs`, `tests/roadnet.rs`; every place that builds a
`Stretch` or a `FeatureKind::Road` / `Path` literal (tests and examples: let the compiler list them).

**Interfaces — Produces:**

- `lot::Paving { Paved, Unpaved }` and `lot::paving(value: &str) -> Option<Paving>` with the spec's two lists.
- `FeatureKind::Road::paving: Option<Paving>` (`#[serde(default)]`), from the way's `surface` tag;
  `FeatureKind::Path::street: bool` (`#[serde(default)]`), true for `service=driveway` and `parking_aisle`.
- `Stretch::paved_m: f64`, `Stretch::unpaved_m: f64`: the metres of the stretch's line whose way is tagged paved or
  unpaved. Not part of the attributes that cut a stretch (like `use_sidepath`); a cut piece takes them in proportion to
  its length; `drop_duplicates`, `collapse_small_rings`, the clean-up steps, smoothing and `with_bridge_spans` keep them
  consistent (sum over pieces equals the original within 1e-9).

- [ ] **Step 1: Failing tests:** `classify` of `highway=service, surface=fine_gravel` gives a `City` road with
  `paving: Some(Unpaved)`; `surface=asphalt` → `Some(Paved)`; no tag or `surface=foo` → `None`; the family is
  unchanged in every case (a track's family still follows its tags as today); `service=parking_aisle` gives
  `Path { width_m: 3.0, street: true }`, `highway=footway` `street: false`.
- [ ] **Step 2: Failing tests:** two ways joined into one stretch, 30 m `asphalt` and 20 m `fine_gravel`: one stretch
  (same id as today), `paved_m` 30 ± 1e-6, `unpaved_m` 20 ± 1e-6; the graph of a fixture with the tags equals the graph
  without them in everything but these two fields; a stretch cut in half by `with_bridge_spans` gives halves whose
  sums equal the original.
- [ ] **Step 3:** RED. **Step 4:** implement. **Step 5:** GREEN, the `osm_ways` seed, `make check`.
- [ ] **Step 6:** Commit `feat: roads keep their surface tag as metres paved and unpaved` (no behaviour change; no
  CHANGELOG line).

### Task 4: Groups, exclusions and the smooth surface (Sonnet; Opus reviews the fit with extra care)

**Files:** `crates/naves-core/src/lot.rs`, `crates/naves-core/tests/lot.rs`, a `proptest` regression file under
`crates/naves-core/tests/`.

**Interfaces — Consumes:** `LotArea`, `heightmap::Heightmap::{at, sample}`, `smooth::WaterTest`,
`limits::{min_vertical_radius, CROSS_FALL_LIMIT}`.

**Interfaces — Produces:**

- `lot::Form { Plane, Quadratic }`.
- `lot::Surface { pub at: (f64, f64), pub coeffs: [f64; 6], pub form: Form, pub lambda: f64 }`: the height at a
  point is `coeffs[0] + coeffs[1]·dx + coeffs[2]·dy + coeffs[3]·dx² + coeffs[4]·dx·dy + coeffs[5]·dy²` about `at`;
  `lambda` is 1 unless the move guard scaled the lot. `Surface::height_at(&self, p) -> f64` (the fitted surface) and
  `Surface::gradient_at(&self, p) -> (f64, f64)`.
- `lot::Outcome { Smoothed { form: Form }, SurfaceOnly { reason: &'static str }, Excluded { reason: &'static str } }`;
  reasons `"street_side"`, `"no_vertex"`, `"would_split"` (set in Task 5); `"not_at_ground"`, `"in_water"`.
- `lot::Assessed { pub area: LotArea, pub group: usize, pub outcome: Outcome, pub steep: bool, pub move_p95_m: f64,
  pub move_max_m: f64, pub slope_p95: f64, pub relief_before_m: f64, pub relief_after_m: f64,
  pub range_before: (f64, f64), pub range_after: (f64, f64), pub water_share: f64 }` (NaN where not smoothed).
- `lot::LotSurface { pub group: usize, pub members: Vec<String>, pub polygons: Vec<(Vec<(f64, f64)>,
  Vec<Vec<(f64, f64)>>)>, pub surface: Surface }` with `contains(&self, p) -> bool` and `target(&self, p, ground: f64)
  -> f64` (`ground + lambda · (height_at(p) − ground)`).
- `lot::assess(areas: &[LotArea], base: &Heightmap, in_water: WaterTest<'_>) -> (Vec<Assessed>, Vec<LotSurface>)`.
- `lot::local_relief(heights: &dyn Fn(i64, i64) -> Option<f64>, vertices: &[(i64, i64)], cell_m: f64) -> (f64, f64)`
  (95th percentile and largest; public for the judging and the property test).

- [ ] **Step 1: Failing tests (form):**
  - flat ground at 100, a 40 × 40 lot: `Plane`, height 100 ± 1e-6, every move 0, relief after 0;
  - ground rising 0.05 to the east with ±0.3 m bumps (fixed seed): `Plane`, gradient (0.05, 0) ± 0.01, relief after
    under 1e-6, `move_max_m` under 0.35;
  - ground `z = 100 + (x − 30)² / 600` (curvature 1/300) on a lot spanning x = 50 to 110 (its minimum outside): a
    quadratic with its x-curvature capped to 1/600 (`coeffs[3]` 1/1200 ± 1e-9), every second difference along a row at
    most 16/600 + 1e-9, every row monotonic;
  - bowl-shaped ground `z = 100 + (x² + y²) / 400` on a 60 × 60 lot centred at the bowl's bottom: `Plane` (the capped
    quadratic would have its minimum inside), no vertex inside lower or higher than all eight neighbours (Review
    Focus 8);
  - a 100 × 100 lot with a 5 m ridge across it: `lambda` = 3 / (largest move of the unscaled surface) ± 1e-9, the
    largest move 3.0 ± 1e-6, reported move-limited;
  - a plane at 0.25: smoothed and `steep`; at 0.15 not steep;
  - a 10 × 40 lot on flat ground with one 3 m bump: plane, the bump removed (move at the bump about 3 m, under the
    guard);
  - a hole in a 40 × 40 lot whose ground is 5 m higher: the fit unchanged (height 100 ± 1e-6) (Review Focus 1);
  - a 6 × 6 lot lying between terrain vertices: `SurfaceOnly { "no_vertex" }`; street side: `SurfaceOnly
    { "street_side" }`; 60 % water: `Excluded { "in_water" }`; `NotAtGround`: `Excluded { "not_at_ground" }`;
  - a lot that reaches past the map edge: smoothed on its part on the map, and a heightmap whose `sample` panics
    outside the map in a test double is never asked outside it (Review Focus 5).
- [ ] **Step 2: Failing tests (property):** `proptest` (fixed seed, 256 cases): random convex lots of 20 to 120 m on
  random ground (a random plane, a random quadratic, random bumps up to 2 m); for every smoothed result: every row,
  column and diagonal monotonic or with one turning point, no interior local extremum, every second difference at most
  `cell² / LOT_MIN_RADIUS_M` (scaled by λ where λ < 1 the bound is on the surface, the result's relief at most
  `(1 − λ)·before + 0.12`), and `relief_after_m` ≤ 0.12 + 1e-9 where λ = 1.
- [ ] **Step 3: Failing tests (groups):** two 20 × 20 lots sharing an edge on ground rising 0.05 to the north: one
  group, one surface, `height_at` equal on the shared edge (Review Focus 3); 0.5 m apart one group, 2 m apart two; a
  car park inside a camp site: one group over the union (the camp site's area), two `Assessed` with their own ids
  (Review Focus 7); results equal for reversed input.
- [ ] **Step 4:** RED. **Step 5:** implement: least squares on the terrain vertices inside the members, Gaussian
  elimination with partial pivoting and a fixed order; the curvature cap by clipping the eigenvalues of the 2 × 2
  Hessian and re-fitting the plane part; the plane on an interior extremum; λ. **Step 6:** GREEN, `make check`.
- [ ] **Step 7:** Commit `feat: lots are grouped and get a smooth surface fitted to their ground` (nothing consumes it
  yet; no CHANGELOG line).

### Task 5: No streets inside a lot (Sonnet; may read `netclean/sidepath.rs` for `splits`)

**Files:** create `crates/naves-core/src/netclean/lot.rs`; modify `crates/naves-core/src/netclean.rs`
(`clean_with`, `CleanReport`, the stub step's protected nodes), `crates/naves-core/tests/netclean.rs`.

**Interfaces — Consumes:** `lot::{LotArea, Assessed, LotSurface}` (the step works on every lot that is not excluded
and not a street strip, smoothed or not), `roadnet::{RoadGraph, Stretch}`, the crate's `polyline_length`,
`slice_points`, `first_free_node`, `crossings_of`; the sidepath module's `splits` (made `pub(super)`, no behaviour
change).

**Interfaces — Produces:**

- `netclean::LotGroup { pub group: usize, pub members: Vec<String>, pub polygons: Vec<(Vec<(f64, f64)>,
  Vec<Vec<(f64, f64)>>)> }` (built by `lot::groups(&[Assessed]) -> Vec<LotGroup>` in `lot.rs`).
- `netclean::DroppedInLot { pub stretch: String, pub lot: String, pub length_m: f64, pub at: (f64, f64) }`.
- `netclean::CutAccess { pub stretch: String, pub lot: String, pub cut_m: f64, pub entry: (f64, f64), pub node: i64 }`.
- `netclean::drop_lot_roads(graph: &RoadGraph, groups: &[LotGroup]) -> LotStep` with `LotStep { graph, dropped:
  Vec<DroppedInLot>, access: Vec<CutAccess>, unchanged: Vec<usize> (groups left unchanged, `would_split`), through:
  Vec<String> (inside stretches kept as roads through a lot), entries: BTreeSet<i64> (every node the step leaves with
  one stretch: the stub step leaves them alone) }`. `LotGroup` also has `areas_m2: Vec<f64>` (parallel to `members`).
  `CleanReport::lot_through_kept` counts `through`.
- `netclean::components_with(graph: &RoadGraph, groups: &[LotGroup]) -> usize`.
- `netclean::clean_with(graph, widths, cell_m, groups: &[LotGroup]) -> (RoadGraph, CleanReport)`; `clean` calls it with
  `&[]`, unchanged in behaviour. The lot step runs first; the stub step treats every `CutAccess::node` as not a free
  end.
- `CleanReport::lot_roads`, `lot_access`, `lots_split`, `junctions_in_lots: usize`; components counted with
  `components_with`; `components_by_step` gets the lot step first.

- [ ] **Step 1: Failing tests:**
  - a 40 × 40 lot (south edge y = 0) with a T of two service stretches wholly inside (junction node at (20, 20)) and an
    access road from a street at y = −30: both removed and reported with their lengths, the junction counted; the
    access road keeps its id and ends at (20, 4.0) ± 1e-6 at a new node one above the largest id, `cut_m` 16 ± 1e-6;
    `components_with` equal before and after;
  - the same lot marked surface only (`no_vertex` is impossible at 40 m; use a test seam that passes a group for a
    surface-only lot): the roads go all the same (no streets inside, in every case);
  - the street at y = −6 (a 10 m wide `City` access road left 10 m long): `clean_with` keeps it, `clean` without
    groups on the same graph drops it (control);
  - a road along the outline sharing its nodes stays (Review Focus 6); a road crossing west to east stays whole; the
    same road with an aisle branching off inside: the aisle goes, the road's two stretches and their node stay;
  - a dead-end stretch crossing the edge three times before it ends inside: cut only after its last entry (Review
    Focus 2);
  - two lots sharing an edge with a road across it inside the union: removed; a lot B reached only through lot A:
    `components_with` unchanged (Review Focus 3, 4); a car park inside a camp site, with one road in the car park and
    two in the camp site: three removed, each reported once with the lot it lies in (the smallest containing lot)
    (Review Focus 7);
  - a road cut at the map edge (negative end node) ending inside a lot: not cut (Review Focus 5);
  - a removal that would split a component (forced through a private seam that replaces `splits`): nothing changes for
    that group;
  - a bridge stretch inside is never removed; a stretch with a NaN point is left as it is;
  - no groups: the graph equal bit for bit, the report's lot fields empty; the same result for reversed input.
- [ ] **Step 2:** RED. **Step 3:** implement. **Step 4:** GREEN, `make check`.
- [ ] **Step 5:** Commit `feat: roads inside a lot are taken out of the road graph` (nothing calls `clean_with` with
  groups yet; no CHANGELOG line).

### Task 6: The surface class and the painting (Sonnet; DeepSeek for Step 6)

**Files:** `crates/naves-core/src/surface.rs`, `crates/naves-core/src/lot.rs`, `crates/naves-export/src/layers.rs`,
`crates/naves-export/src/rvmat.rs`, `crates/naves-export/src/generate.rs` (`paint_lots`, `paint_paths_with`, not
called yet), tests `crates/naves-core/tests/surface.rs`, `tests/lot.rs`, `crates/naves-export/tests/layers.rs`,
`tests/road_bed.rs`, `docs/formats/terrain-layers.md`.

**Interfaces — Consumes:** Task 1's `slot4.md`; `Assessed`, `CleanReport::{lot_roads, lot_access}`,
`Stretch::{paved_m, unpaved_m}`, `conform::core_reach_m`.

**Interfaces — Produces:**

- `SurfaceClass::Tarmac` (slot 4, name `"Tarmac"`, `rgb()` the average colour of the chosen texture from Task 1);
  `SurfaceClass::ALL: [SurfaceClass; 5]`. If Task 1 found no slot-4 colour: no new class, paved is `SurfaceClass::Soil`,
  and the tarmac steps of this task are skipped.
- `SurfaceGrid::fill_polygon(&mut self, outer: &[(f64, f64)], holes: &[Vec<(f64, f64)>], class: SurfaceClass)`.
- `layers::SURFACES` gets the tarmac entry with the file names from Task 1 (`en_tarmacold`, texture `en_tarmac_old`: the game's `en_tarmac` has no texture files);
  `mask_rgba(4)` is (0, 0, 0, 128), Task 1's value, not yet seen in the game.
- `rvmat::tile_material` writes slot 4's textures only for a tile whose mask uses slot 4 (an argument with the tile's
  used slots; documented).
- `lot::surfaces(assessed: &[Assessed], graph: &RoadGraph, clean: &CleanReport, cell_m: f64)
  -> Vec<(String, SurfaceClass)>`: one per lot that is not excluded, in id order (`graph` the graph before the lot
  step, for the streets beside).
- `generate::paint_lots(surfaces: &mut SurfaceGrid, areas: &[LotArea], decided: &[(String, SurfaceClass)])`: largest
  first, then id; `generate::paint_paths_with(surfaces, features, lots: &[LotArea])`: a path with `street: true`
  inside a lot is not painted; every other path as today.

- [ ] **Step 1: Failing tests (grid):** `SurfaceClass::Tarmac.slot() == 4`; a 10 × 10 m square fills 100 cells, with a
  4 × 4 m hole 84; a bow-tie both triangles; half off the map only the cells on it; NaN nothing.
- [ ] **Step 2: Failing tests (decision):** a lot tagged `surface=grass` whose removed roads are 100 m asphalt → grass
  (the lot's tag wins); removed roads 404 m tagged `fine_gravel`, 34 m `asphalt`, 132 m untagged `City` → grass (the
  Grafenlehen numbers); 84 m untagged `City` → paved (family); 25 m `Asf3` untagged and 25 m `Mud` untagged → paved
  (tie); no roads inside or entering but an `Asf3` street 3 m from the edge → paved; a `Mud` track there → grass;
  nothing within `core_reach_m + cell` → grass; lot B without roads in a group with paved lot A → paved (Review Focus
  4); a car park with paved roads inside a camp site with gravel roads: car park paved, camp site grass (Review Focus 7);
  a street strip beside a `City` road → paved.
- [ ] **Step 3: Failing tests (painting and layers):** a guard test written first, on the base code, storing today's
  `tile_material` text for one tile and today's mask bytes for a fixture grid (it passes before and after: a tile
  without tarmac is written byte for byte as before); then: a tile with tarmac lists the tarmac textures in `Stage11`
  and `Stage12` and its mask uses Task 1's colour there; `paint_lots` paints a camp site and the car park inside it so
  that the car park's cells have the car park's class; a `parking_aisle` path inside a lot is not painted, a footway
  through it is, on top of the lot.
- [ ] **Step 4:** RED. **Step 5:** implement. **Step 6 (DeepSeek):** `docs/formats/terrain-layers.md`: the slot-4 row
  (tarmac, its mask value alpha 128, its texture `en_tarmac_old`) and the sentence that slot 4 is written only on
  tiles that use it. **Step 7:** GREEN, `make check`. The GUI's ground legend reads `SurfaceClass::ALL`
  (`crates/naves/src/gui/levels/ground.rs`): if the GUI shows anything new, invoke `/frontend-design:frontend-design`
  first; map items keep the DayZ look.
- [ ] **Step 8:** Commit `feat: a tarmac ground layer, and paved or grass lots` (nothing paints lots yet; no CHANGELOG
  line).

### Task 7: The smooth surface in the terrain, and the roads that meet it (Sonnet)

**Files:** `crates/naves-core/src/conform.rs`, `crates/naves-core/src/junctions.rs`,
`crates/naves-core/src/profile/model.rs` (`ground_targets`, the constant plane of entries; `profile/holds.rs` only if
`Inward` needs the lot's area as its reach), `crates/naves-export/src/generate.rs` (`place_once_with`,
`area_patches`), tests `crates/naves-core/tests/conform.rs`, `tests/heightsolve.rs`, `tests/junctions.rs`,
`crates/naves-export/tests/level_patches.rs`.

**Interfaces — Consumes:** `LotSurface`, `CutAccess`, `conform::{LevelPatch, conform_bridges, SHOULDER_CELLS}`,
`junctions::{Junctions, Junction, JunctionKind, find_junctions}`, the ring path in `profile::model`
(`ring_of_junction`, `Source::Constant`).

**Interfaces — Produces:**

- `conform::AreaPatch { pub polygons: Vec<(Vec<(f64, f64)>, Vec<Vec<(f64, f64)>>)>, pub height: Box<dyn Fn((f64, f64),
  f64) -> f64 + Sync>, pub rank: u8 }` or, if a closure field is unwelcome, `pub surface: lot::LotSurface` (the
  implementer's choice; `naves-core` owns both); the height of a claimed vertex is `LotSurface::target(p, ground)`.
- `conform::conform_areas(base: &Heightmap, strips: &[RoadStrip], patches: &[LevelPatch], areas: &[AreaPatch],
  bridges: &[BridgeSpan]) -> Result<Conformed, ConformError>`; `conform_bridges` calls it with `&[]`, unchanged.
- `JunctionKind::LotEntry`; `Junctions::lots: Vec<LotSurface>`; `junctions::add_lot_entries(junctions: &mut
  Junctions, net: &PlacedNetwork, lots: &[LotSurface], entries: &[CutAccess], base: &Heightmap)`: one junction per entry
  of a smoothed lot at the placed end of its access road, height `target` there, gradient `gradient_at` (times λ),
  `patch_m` half the road's width, `stretches` the access stretch, rank 0. Entries of surface-only lots get none.
- In `profile::model`, a `LotEntry` junction is a constant plane like a ring junction; `ground_targets` uses
  `LotSurface::target` for a joint over a smoothed lot instead of the original ground.
- `generate::place_once_with(graph, catalog, base, lots: &[LotSurface], entries: &[CutAccess]) -> Result<Placed,
  BuildError>`; `place_once` calls it with `&[]`; `generate::area_patches(lots: &[LotSurface]) -> Vec<AreaPatch>`.

- [ ] **Step 1: Failing tests (shaping):** flat ground at 100, one lot 20 × 20 whose surface is a plane at 101:
  every vertex inside at 101 ± 1e-4, every vertex within 1.5 cells beyond the edge at 101, the ground beyond falls back
  to 100 on the fill slope (no vertex below 100); a lot whose surface is a capped quadratic: every claimed vertex at
  `target` ± 1e-4, and `local_relief` of the shaped vertices inside ≤ 0.12 + 1e-6; a road strip at 100.5 through the
  lot: vertices inside the road's half width at 100.5; a 6 × 6 hole whose centre vertex keeps 100 (Review Focus 1); a
  vertex under a bridge span not claimed; two lots of one group sharing an edge: equal vertices on the shared edge
  (Review Focus 3); `conform_areas` with no areas equals `conform_bridges` bit for bit on the `tests/bridge_ground.rs`
  fixture.
- [ ] **Step 2: Failing tests (roads):** a straight 100 m `City` road on flat ground at 100 ending at an entry of a lot
  whose surface is 101: after `solve_clearing` the end joint at 101 ± 1e-6, the joints inward over the lot at the
  surface there, no grade above `grade_ceiling(City)`; the same at 130: the solve returns and the link is in
  `Junctions::unresolved`; a through road across a lot whose surface lies 0.5 m above the ground: every joint over the
  lot within 0.05 m of the surface, the shaped vertices beside the road's ring within `CONFLICT_M` of the road, no
  `RoadConflict` there; a through road across a lot whose surface slopes 0.35 along it: the road keeps its ceiling and
  a conflict is reported where they part (no panic); an access road of a surface-only lot: no entry junction, its
  profile as today; with no lots, `place_once_with` equals `place_once` bit for bit on the
  `tests/bridge_clearance.rs` fixture.
- [ ] **Step 3:** RED. **Step 4:** implement. **Step 5:** GREEN, `make check`.
- [ ] **Step 6:** Commit `feat: a lot's ground is smoothed and its roads meet it` (nothing calls it with lots yet; no
  CHANGELOG line).

### Task 8: Lots in the build, behind the switch; findings, report, manual (Sonnet; DeepSeek for Steps 4 and 5)

**Files:** `crates/naves-core/src/lot.rs` (`LotReport`), `crates/naves-export/src/generate.rs`,
`crates/naves-core/src/roadplace.rs` (`PlacedNetwork`), `crates/naves-core/src/validate.rs`,
`crates/naves-core/tests/validate.rs`, `crates/naves-export/src/build.rs`, `crates/naves-export/tests/build.rs`, a new
`crates/naves-export/tests/lots.rs`, the callers of `fitted_graph_with`, `plan_roads` and `paint_paths`
(`tests/smoothing.rs`, the examples), `crates/naves/src/mcp.rs`, the binary's summary print, the `road-analysis`
example, `docs/manual/src/getting-started.md`, `troubleshooting.md`, `mcp.md`, `CHANGELOG.md`.

**Interfaces — Consumes:** everything above; `generate::Stages` from the road-planning plan if it has landed, else
this task creates `Stages { pub smooth: bool, pub lots: bool }` with `Stages::DEFAULT` (`SMOOTH_ENABLED`,
`LOTS_ENABLED`) and replaces `fitted_graph_with`'s `smooth: bool` with it (road planning then adds `reroute`).

**Interfaces — Produces:**

- `lot::LotReport { pub assessed: Vec<Assessed>, pub surfaces: Vec<LotSurface>, pub classes: Vec<(String,
  SurfaceClass)> }` with `summary(&self, clean: &CleanReport) -> String`; the removals stay in `CleanReport`.
- `Stages::lots`; `generate::fitted_graph_with(features, catalog, base, stages, in_water, lots: &[LotArea])`;
  `FittedGraph::lots: Option<LotReport>` (`None` when off).
- `generate::plan_roads(…, lots: &[LotArea], …)` passes surfaces and entries to `place_once_with`; `place_roads`
  gives `area_patches` to `conform_areas`; `generate` paints the lots, then the paths with `paint_paths_with`, then the
  road beds.
- `PlacedNetwork::lots: Option<LotReport>`, `Generated::lots`, `BuildOutput::lots: Option<String>`.
- Rules: `Rule::LotSmoothed` (`lot_smoothed`, information: kind, form, largest move, surface class),
  `Rule::LotSurfaceOnly` (`lot_surface_only`, information, with the reason), `Rule::LotExcluded` (`lot_excluded`,
  information), `Rule::LotSteep` (`lot_steep`, warning: its slope against 0.20), `Rule::LotMoveLimited`
  (`lot_move_limited`, warning: λ and the move it would have needed against 3 m), `Rule::RoadInLotDropped`
  (`road_in_lot_dropped`, information, for a removed stretch and for a cut access road); `Rule::ALL` grows by six;
  `validate::lot_violations(report: &LotReport, clean: &CleanReport) -> Vec<Violation>`.
- `build.rs::lots_json` for `roads.lots` in `build/report.json`: `{"enabled": false}` when off; else `enabled`, the
  counts by kind, outcome, form and reason, paved and grass counts, `steep`, `move_limited`, `roads_removed`,
  `roads_removed_m`, `access_cut`, `junctions_removed`, and every lot (`id`, `kind`, `x`, `y`, `area_m2`, `outcome`,
  `reason`, `form`, `surface`, `surface_source` (`tag`, `roads`, `street`, `none`), `move_p95_m`, `move_max_m`,
  `lambda`, `slope_p95`, `relief_before_m`, `relief_after_m`, `range_before`, `range_after`).

- [ ] **Step 1: Failing tests:**
  - `fitted_graph_with` with `lots: false` on the smoothing test's fixture plus one lot area equals today's graph bit
    for bit, `lots` is `None`;
  - with `lots: true` on a fixture with one camp site holding a T of service roads: both stretches gone, one access
    road cut, `lots` is `Some` with one `Smoothed`;
  - `fitted_graph(..).lots.is_some() == LOTS_ENABLED`;
  - a synthetic project build (the faked tools of `tests/build.rs`) with the stage on: the conformed heightmap equals
    the lot's target inside it (± 1e-3), the surface grid is the decided class there, `report.json`'s
    `roads.lots.smoothed == 1`; with the stage off the heightmap, surface grid and every written layer file equal the
    build without the lot area;
  - rule ids as above; a report with one smoothed, one steep, one move-limited, one street strip, one excluded and one
    removed stretch gives one finding of each rule at the lot's centre (the stretch's middle for the road), steep and
    move-limited naming their numbers and limits;
  - `summary()` of a fixture report is exactly `Lots: 5 (3 car parks, 2 camp sites), 4 smoothed (3 plane, 1 quadratic;
    3 paved, 1 grass; 1 steep, 0 move limited), 1 surface only, 0 excluded; 3 roads (536 m) removed, 1 access road
    cut` with the fixture's numbers;
  - the MCP rule list test (`every_rule_is_documented_where_rules_are_listed`) passes with the six new ids.
- [ ] **Step 2:** RED. **Step 3:** implement the wiring, the progress line `Shaping the lots` and the summary line.
- [ ] **Step 4 (DeepSeek):** `road-analysis`: read `LOTS` from the environment (`off`, `on`; default
  `LOTS_ENABLED`), print `STAGES lots=<bool>`, `LOT <summary>`, `LOT_OUTCOME <kind> <outcome> <form or reason>
  <count>`, `LOT_ROADS removed <n> <m> m, access cut <n>, junctions removed <n>`, `LOT_RELIEF <kind> before <p95>
  <max> after <p95> <max>`, `LOT_MOVE <kind> <p95> <max>`, `LOT_ENTRY <stretch> <step m>`, `LOT_THROUGH <stretch>
  <largest difference m>`, `LOT_FLAG <id> <x> <y> steep|move_limited <value>` and `LOT_NEAR <rule> <count>` for every
  rule within 50 m of a smoothed lot.
- [ ] **Step 5 (DeepSeek):** the manual: getting-started "Roads" (lots: which tags, no streets inside, access roads end
  just inside, through roads stay; that it is off) and "Ground from real data" (paved or grass lots, the tarmac layer,
  smooth ground: a plane or a gentle curve, no local bumps, steep lots stay steep); troubleshooting (the six rules,
  every reason, what steep and move-limited mean and their limits); mcp.md (the rule list); `CHANGELOG.md` under
  Unreleased, Added: "Lots — car parks, camp sites, caravan sites, rest areas and coach parks — get a full paved or
  grass surface and smooth ground; the roads inside them are no longer built, and their access roads end just inside
  ... switched off in this release (`LOTS_ENABLED`) ... with it on, edits on removed or cut roads are orphaned
  (`orphaned_edit`): on the example 85 roads are removed and 21 cut ... `roads.lots` in `build/report.json`, findings
  `lot_smoothed`, `lot_surface_only`, `lot_excluded`, `lot_steep`, `lot_move_limited`, `road_in_lot_dropped`".
- [ ] **Step 6:** GREEN, `make check` (with `make docs`).
- [ ] **Step 7:** Commit `feat: lots in the build, switched off`.

### Task 9: Judging runs (controller with the Opus reviewer; Sonnet runs them)

**Files:** none in the repo. Output under `/var/tmp/naves-lots/judge/`.

- [ ] **Step 1:** Rebuild the release example once (log HEAD and the binary's mtime). Check the machine; one run at a
  time, 16G cap each.
- [ ] **Step 2:** Two runs of `road-analysis` on Berchtesgaden at 2048 with the same binary: `LOTS=off` and `LOTS=on`.
  Logs `judge/<off|on>/ra-2048.log`, `RULES_OUT=judge/<off|on>/rules.tsv`.
- [ ] **Step 3:** Write `judge/compare.md`, one table, two columns, beside Task 1's baseline:
  - lots by kind and outcome, plane and quadratic, paved and grass, steep (spec: 44), move limited (spec: 3);
  - stretches and metres removed (spec: 85, 2064 m), access roads (32 dead ends, 21 cut), junction nodes removed (47);
  - components with groups as nodes (must not grow); drivable length (must fall by no more than the removed and cut
    metres);
  - per lot kind, the local relief before and after (`LOT_RELIEF`; after at most 0.12 m outside the move-limited
    lots), the moves (`LOT_MOVE`) and the slope of the result;
  - `road_gap` (count, total, largest), `road_conflict`, `road_not_drivable`, `road_misses_terrain`,
    `join_without_junction` map-wide and `LOT_NEAR` within 50 m; `road_grade`, `junction_too_steep`,
    `earthworks_capped`, `orphaned_edit` and every other rule;
  - `LOT_ENTRY` (all under 0.05 m), `LOT_THROUGH` (largest), the steepest access link;
  - the stage's time and the build's time.
- [ ] **Step 4:** Judge against the spec's bar: no rule worse map-wide by more than 5 % without a reason that can be
  named; within 50 m of lots `road_gap` and `road_conflict` not worse; no entry step; the relief target met. A miss
  goes to the maintainer with the numbers; nothing is tuned until it is understood (one round at most, each change its
  own commit with a test, the spec's tables in the same commit).
- [ ] **Step 5:** Write the game sites as a `sites.json` for `tools/ingame-check`: Grafenlehen (5638, 5412) — must be
  one; Königssee P1 (5741, 5171) with its through road (large, gentle); Camping Allweglehen, a point of it on the map
  (steep camp site), or Campingplatz Simonhof (1330, 7247) if Allweglehen's part on the map is too small; the caravan
  site P2 Königssee (5601, 5200) (steep, grass); P2 Hirschbichlstraße (265, 5714) (roads removed, a plane on a slope);
  Parkplatz Salzbergwerk (6772, 7741) (flat but bumpy).

No commit, unless Step 4's tuning changes a constant.

### Task 10: Checkpoint, the game (controller with a Sonnet agent)

- [ ] **Step 1:** Build the map twice: once from this branch as it is (stage off), once from a scratch worktree under
  `/var/tmp/naves-lots/game/` with `LOTS_ENABLED = true` (not committed; its own target dir under the same folder).
  Log HEAD, the worktree's diff and the binary's mtime each time. Keep the `build/` folders under
  `/var/tmp/naves-lots/game/`.
- [ ] **Step 2:** `tools/ingame-check/run.sh` on each build with the same `sites.json` (24G cap, server 8G, client
  14G, non-default ports, one game at a time, headless sway only). Do a control run against vanilla Livonia first if
  anything looks broken. Stop the game when done; remove the `@naves*` symlinks from the DayZ install.
- [ ] **Step 3:** Send the contact sheets to the maintainer with one line per site: surface (tarmac shows, not dark red
  or flat colour; that mask alpha 128 selects the fifth layer), the ground (smooth, no bumps, no hole or dam, the shoulder blended; whether a
  large lot looks artificially even), the access road and the through road (no step, drivable onto the lot), anything
  that looks wrong.
- [ ] **Step 4:** The maintainer decides whether `LOTS_ENABLED` becomes `true` and that the fifth layer shows. If on: a commit
  `feat!: lots are switched on` that flips the constant, updates the manual ("off" becomes "on") and adds the
  **Breaking** note for orphaned edits (85 roads removed and 21 cut on the example) to `CHANGELOG.md`; `make check`.

### Task 11: Final review (Opus)

- [ ] Review the whole branch against the spec and this plan: every spec rule has a test; the Review Focus tests exist
  and assert what is listed; the constants in the code equal the spec's table; no access road can be removed (read the
  step, not only the tests); the smooth surface cannot have an interior extremum (read the fit); with the stage off
  nothing changes (read the call sites: every new path is behind the flag; the `surface` metres never cut a stretch);
  determinism; no panics on input; coverage not below the base; the manual and the CLI help agree.
- [ ] Update the spec's status line with the result (built, judged, on or off) and its open points with what the
  judging and the game showed.
- [ ] Hand the lists of steep and move-limited lots (from Task 9) to the controller as the start of the terrace and
  facet work, and the open tags to the maintainer.

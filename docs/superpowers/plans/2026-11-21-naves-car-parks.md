<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Car Parks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Read the `amenity=parking` areas; for every car park that lies on the ground and one plane can hold, take
the roads that lie wholly inside it out of the network, cut its access roads back to just inside its edge, paint it
paved or grass after the roads that serve it, flatten it to one plane blended into the terrain, and tie the access
roads to that plane in the height solve; paint the others and report everything.

**Architecture:** `osm::read_land` also keeps the car park areas (`OsmLand::car_parks`). A new module
`crates/naves-core/src/carpark.rs` turns them into map polygons (`areas`), groups the ones that touch, fits a plane per
group and decides each outcome (`assess`) and the surface (`surfaces`). A new clean-up step
`crates/naves-core/src/netclean/carpark.rs` removes the roads inside and cuts the access roads; it runs first in
`netclean::clean_with`. `conform` gets a polygon patch (`AreaPatch`), `junctions` a fixed-height entry junction
(`JunctionKind::CarParkEntry`) whose height the solve takes as a constant, as for a ring. `naves-export`'s
`generate` wires it behind `CARPARKS_ENABLED` and a per-call stage flag, paints, and reports.

**Tech Stack:** Rust 1.99; no new dependency planned. AGENTS.md's crate review applies if one seems needed: `geo`
(polygon contains, area, distance) is the first to check for the polygon tests, against the repo's own even-odd
tests in `osm.rs` and `surface.rs`, which come first.

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
  data in the repo (tests use synthetic polygons, graphs and heightmaps; the slot-4 colour and texture names are
  facts written into the docs, not game files).
- Every new parser path gets fuzz coverage: the car park reading runs inside `read_land`, so the `osm_read` target
  covers it; add a seed with a car park way and a car park multipolygon, which also runs in `make test`.
- Every process gets a hard memory limit: `systemd-run --user --scope --quiet -p MemoryMax=12G -p MemorySwapMax=1G`
  for cargo and `make check`, 16G for `road-analysis` and the measurement program, server 8G, client 14G,
  `ingame-check` 24G. Never raise a cap; report an OOM kill. The sum of what parallel agents run must fit the machine
  (24 threads, 60 GB): check `nproc`, `free`, `uptime` before a long run; one heavy job at a time.
- Temp and build folders under `/var/tmp` (never `/tmp`, never `$HOME`); this plan's scratch is
  `/var/tmp/naves-carparks/`. Shared target dir `/var/tmp/naves-target`, never deleted; after a worktree agent,
  touch the sources before `make check`.
- Never start the game outside the checkpoint task. Wine/Proton only inside headless sway. Never touch the user's
  DayZ profile folder or client settings. Non-default server ports. Remove `@naves*` symlinks from the DayZ install
  afterwards. One game at a time. Reading game files (configs, PAAs, rvmats) with dayzmake is allowed; never edit
  them.
- Figures are quoted for Berchtesgaden at half scale (`scale.horizontal = 0.5`, `scale.height = 0.5`), terrain size
  2048 (4 m cells).
- Constants, verbatim from the spec (all `pub const` in `carpark.rs`): `CARPARKS_ENABLED = false`,
  `CARPARK_INSIDE_TOL_M = 1.0`, `CARPARK_INSIDE_SHARE = 0.5`, `CARPARK_ENTRY_M = 4.0`,
  `CARPARK_MAX_GRADE = limits::CROSS_FALL_AIM` (0.08), `CARPARK_MAX_DEPTH_M = 1.5`, `CARPARK_DEPTH_QUANTILE = 0.95`,
  `CARPARK_MIN_AREA_M2 = 64.0`, `CARPARK_WATER_MAX = 0.5`, `CARPARK_SAMPLE_M = 1.0`; the car park patch has rank 0;
  reused, not copied: `conform::SHOULDER_CELLS`, `conform::core_reach_m`, `features::CLIP_INSET_M`,
  `limits::RAMP_JOINT_MIN_M`. Paved families `Asf1`, `Asf2`, `Asf3`, `City`; gravel and dirt `Grav`, `Mud`.
- With the stage off (the constant, or the per-call flag) every output is the same as on the base commit, bit for
  bit: graph, heightmap, surface grid, tile masks and materials, report (apart from `car_parks: {"enabled": false}`).
- Only roads wholly inside a built car park are removed; no access road is ever removed; nothing is removed or cut
  silently.
- New node ids are above all existing ids, handed out in sorted order; a cut access road keeps its stretch id.

## Review Focus

The inputs most likely to bite. Each has its test in the task named.

1. **A polygon with a hole or a self-intersection.** A 40 × 40 m car park with a 10 × 10 m hole; a bow-tie outline.
   Expected: a point in the hole is not inside, a road wholly in the hole stays, the hole's ground is not used for the
   plane and keeps its height beyond the shoulder; the bow-tie's area is counted from samples (two triangles, not
   the shoelace's cancellation) and nothing panics. Tests in Task 2 (inside, area), Task 3 (plane), Task 6 (shaping).
2. **A road that enters, leaves and re-enters.** A dead-end stretch whose line crosses the edge three times before it
   ends inside. Expected: cut only after its last entry, the earlier parts unchanged, one entry. Test in Task 4.
3. **Two car parks sharing an edge.** Expected: one group, one plane (equal heights on the shared edge after the
   shaping), a road that crosses the shared edge wholly inside the union is removed, each car park reported on its
   own. Tests in Task 3 (group and plane), Task 4 (removal), Task 6 (shaping).
4. **A car park whose only access is through another car park.** Expected: it is built with its group, takes the
   group's surface, and the connectivity check (a built group is one node) finds no split. Tests in Task 4
   (components) and Task 5 (surface).
5. **A polygon at the map edge.** A car park that reaches within `CLIP_INSET_M` of the edge, with a road cut there
   (negative node id) ending inside it. Expected: surface only (`map_edge`), no plane, no road removed or cut, the cut
   end untouched; samples outside the map never read. Tests in Task 3 and Task 4.
6. **A road drawn on the outline.** OpenStreetMap often shares nodes between a car park's outline and the street.
   Expected: such a road lies within the tolerance everywhere but is not inside (`CARPARK_INSIDE_SHARE`) and stays.
   Test in Task 4.

---

### Task 1: Baseline, the code's own measurement, and the slot-4 mask colour (Sonnet; no production code)

**Files:** none in the repo. Output under `/var/tmp/naves-carparks/baseline/`. The design's scratch measurement is in
`/var/tmp/naves-parking/` (`measure/` is a cargo package with path dependencies on the repo's crates; its tables are
`out3.tsv` and `outcome.tsv`).

- [ ] **Step 1:** On the base commit (log `git rev-parse HEAD` and the `naves` binary's mtime), run `road-analysis`
  on Berchtesgaden at 2048 once (16G cap, `RULES_OUT=…/rules.tsv`), log `baseline/ra-2048.log`. Record every rule
  count, in particular `road_gap` (count, total, largest), `road_conflict`, `road_not_drivable`,
  `road_misses_terrain`, `join_without_junction`.
- [ ] **Step 2:** Copy the measurement program to `baseline/measure/` and change the frontage reach to the code's
  `core_reach_m(half width, 4.0) + 4.0`; run it (16G cap). Write `baseline/carparks.tsv` (one row per car park: id,
  position, polygon as WKT, area, fall, depth, the spec's outcome and surface) and `baseline/baseline.md` with the
  spec's tables re-counted. If a count moves by more than five car parks from the spec, update the spec's table in
  Task 2's commit.
- [ ] **Step 3:** From `baseline/carparks.tsv` and `rules.tsv`, count every rule within 50 m of a car park that the
  spec calls built (inside or within 50 m of its outline), for the comparison in Task 8.
- [ ] **Step 4: the slot-4 mask colour.** With dayzmake (read only) list Livonia's tile materials and find one whose
  name lists five or more layers and whose layers include `en_tarmac` (or any fifth layer); read its mask PAA
  (`paa_to_image`) and the material, and note the colour (RGBA) that selects slot 4. Check the two texture paths
  `dz\surfaces_bliss\data\terrain\en_tarmac_nopx.paa` and `en_tarmac_ca.paa` exist, and compute the average colour of
  `en_tarmac_ca`. Write the findings to `baseline/slot4.md`. If slot 4 cannot be pinned down, say so: Task 5 then
  paints paved car parks soil and the controller tells the maintainer.

No commit.

### Task 2: Reading the car park areas (Sonnet)

**Files:** Modify `crates/naves-core/src/osm.rs`, `crates/naves-core/src/lib.rs`; create
`crates/naves-core/src/carpark.rs`, `crates/naves-core/tests/carpark.rs`; extend `crates/naves-core/tests/osm.rs`;
a seed under `fuzz/corpus/osm_read/`.

**Interfaces — Consumes:** `osm::{read_land, OsmLand, assemble, RawWay}`, `heightmap::MapGeo::to_map`.

**Interfaces — Produces:**

- `osm::CarParkPolygon { pub id: i64, pub relation: bool, pub part: usize, pub tags: BTreeMap<String, String>,
  pub outer: Vec<(f64, f64)>, pub holes: Vec<Vec<(f64, f64)>> }` (lat, lon, closed rings); the kept tags are
  `parking`, `surface`, `layer`, `building`, `location`, `access`.
- `OsmLand::car_parks: Vec<CarParkPolygon>`: ways first, then relations, each in id order, parts in ring order. A way
  or a `type=multipolygon` relation with `amenity=parking`; `amenity=parking_space` is not kept. The water, rock and
  built-up polygons are unchanged.
- `carpark::AreaKind { Surface, StreetSide, NotAtGround }`.
- `carpark::CarParkArea { pub id: String, pub kind: AreaKind, pub outer: Vec<(f64, f64)>,
  pub holes: Vec<Vec<(f64, f64)>>, pub area_m2: f64, pub surface_tag: Option<String> }` in map metres; `id` is
  `osm/way/<id>` or `osm/relation/<id>/<part>`.
- `carpark::areas(land: &OsmLand, frame: &MapGeo) -> (Vec<CarParkArea>, usize)`: the areas in id order and the
  number skipped (fewer than three distinct corners, a point that cannot be projected or is not finite).
- `carpark::contains(area: &CarParkArea, p: (f64, f64)) -> bool` (even-odd, outer less holes) and
  `carpark::edge_distance(area: &CarParkArea, p: (f64, f64)) -> f64` (to the nearest edge of any ring).

- [ ] **Step 1: Failing tests (osm):** a PBF (built the way `tests/osm.rs` builds its input) with one way tagged
  `amenity=parking, parking=surface, surface=asphalt` gives one `CarParkPolygon` with those tags and `relation:
  false`; a multipolygon relation with two outer rings and one inner gives two polygons, parts 0 and 1, the hole on
  the ring that contains it; an `amenity=parking_space` way gives none; a file with water and a car park gives the
  same `polygons` as without the car park; the result is the same with the two files given in the other order.
- [ ] **Step 2: Failing tests (areas):** kinds: `parking=street_side`, `lane`, `layby` → `StreetSide`;
  `parking=underground`, `multi-storey`, `rooftop`, `building=yes`, `location=underground`, `layer=-1`, `layer=1` →
  `NotAtGround`; no `parking` tag, `surface`, `carports` → `Surface`. A 40 × 40 m square has `area_m2` 1600 ± 1; with
  a 10 × 10 m hole 1500 ± 1, and `contains` is false at the hole's centre and true 5 m inside the outer edge; a
  bow-tie of two 10 × 10 m right triangles meeting at a point has `area_m2` 100 ± 3 (from samples) and `contains` is
  true in each triangle; a ring with a NaN point or only two distinct corners is skipped and counted; `edge_distance`
  of the square's centre is 20; ids `osm/way/7` and `osm/relation/9/1`; the output order is the same for reversed
  input.
- [ ] **Step 3:** RED. **Step 4:** implement; the area is counted on a `CARPARK_SAMPLE_M` grid of even-odd samples.
  **Step 5:** GREEN, the fuzz seed, `make check`.
- [ ] **Step 6:** Commit `feat: the car park areas of OpenStreetMap are read` (no behaviour change yet: nothing
  consumes them; no CHANGELOG line).

### Task 3: Groups, exclusions and the plane (Sonnet)

**Files:** `crates/naves-core/src/carpark.rs`, `crates/naves-core/tests/carpark.rs`.

**Interfaces — Consumes:** `CarParkArea`, `roadnet::RoadGraph` (bridge and tunnel stretches),
`heightmap::Heightmap::sample`, `smooth::WaterTest`, `features::CLIP_INSET_M`, `limits::CROSS_FALL_AIM`.

**Interfaces — Produces:**

- `carpark::Outcome { Built, NotFlattened { reason: &'static str }, SurfaceOnly { reason: &'static str },
  Excluded { reason: &'static str } }`; reasons: `"too_uneven"`, `"would_split"` (set in Task 4); `"street_side"`,
  `"tiny"`, `"map_edge"`, `"under_bridge"`, `"over_tunnel"`; `"not_at_ground"`, `"in_water"`.
- `carpark::Assessed { pub area: CarParkArea, pub group: usize, pub outcome: Outcome, pub fall_free: f64,
  pub depth_m: f64, pub water_share: f64 }` (`fall_free` the free fit's steepest fall, `depth_m` the 95 % depth
  against the capped plane; NaN where no plane was fitted).
- `carpark::CarParkPlane { pub group: usize, pub members: Vec<String>, pub polygons: Vec<(Vec<(f64, f64)>,
  Vec<Vec<(f64, f64)>>)>, pub at: (f64, f64), pub height: f64, pub gradient: (f64, f64) }` with
  `height_at(&self, p: (f64, f64)) -> f64` and `contains(&self, p) -> bool` (inside any member).
- `carpark::assess(areas: &[CarParkArea], graph: &RoadGraph, base: &Heightmap, in_water: WaterTest<'_>,
  map_size_m: f64) -> (Vec<Assessed>, Vec<CarParkPlane>)`: one plane per group whose members are all `Built`, groups
  numbered by their lowest member id.

- [ ] **Step 1: Failing tests (plane):** on flat ground at 100 m a 40 × 40 car park is `Built`, plane height
  100 ± 1e-6, gradient (0, 0); on ground rising 0.05 to the east the gradient is (0.05, 0) ± 1e-6 and
  `fall_free` 0.05; on ground rising 0.15 to the east the plane's gradient is (0.08, 0) ± 1e-9, `fall_free` 0.15 ±
  1e-6, `depth_m` in 1.25..1.42 (0.07 times the 95th percentile of the distance from the centre line, about 19 m),
  outcome `Built`; the same on a 60 × 60 car park gives `depth_m` in 1.9..2.1 and `NotFlattened { "too_uneven" }`;
  on flat ground with one 5 m bump of 3 × 3 m the outcome is `Built` (the percentile ignores it); a hole in a 40 × 40
  car park whose ground is 5 m higher does not change the plane (height 100 ± 1e-6) (Review Focus 1); fewer than
  three samples on the map: level at the median.
- [ ] **Step 2: Failing tests (outcomes):** street side → `SurfaceOnly { "street_side" }` with no plane; 50 m² →
  `SurfaceOnly { "tiny" }`; a car park reaching to 10 m from the map edge → `SurfaceOnly { "map_edge" }`, and a car
  park wholly outside the map the same, without reading a sample outside it (Review Focus 5); a bridge stretch
  passing over a corner → `SurfaceOnly { "under_bridge" }`, a tunnel under it → `"over_tunnel"`; 60 % in water →
  `Excluded { "in_water" }`; `NotAtGround` → `Excluded { "not_at_ground" }`.
- [ ] **Step 3: Failing tests (groups):** two 20 × 20 squares sharing an edge on ground rising 0.05 to the north
  are one group with one plane, and `height_at` of a point on the shared edge is the same for both (Review Focus 3);
  two squares 0.5 m apart are one group, 2 m apart two; a group with one member over the depth limit makes every
  member `NotFlattened` with the group's numbers; results equal for reversed input.
- [ ] **Step 4:** RED. **Step 5:** implement (least squares as `junctions::ring_plane`; the tilt capped with its
  direction kept and the height re-fitted as the mean). **Step 6:** GREEN, `make check`.
- [ ] **Step 7:** Commit `feat: car parks are grouped and get a plane fitted to their ground` (nothing consumes it
  yet; no CHANGELOG line).

### Task 4: Roads inside a car park leave the network (Sonnet; may read `netclean/sidepath.rs` for `splits`)

**Files:** create `crates/naves-core/src/netclean/carpark.rs`; modify `crates/naves-core/src/netclean.rs`
(`clean_with`, `CleanReport`, the stub step's protected nodes), `crates/naves-core/tests/netclean.rs`.

**Interfaces — Consumes:** `carpark::CarParkPlane`, `roadnet::{RoadGraph, Stretch}`, the crate's
`polyline_length`, `slice_points`, `first_free_node`, `crossings_of`; the sidepath module's `splits` (made
`pub(super)`, no behaviour change).

**Interfaces — Produces:**

- `netclean::DroppedInCarPark { pub stretch: String, pub car_park: String, pub length_m: f64, pub at: (f64, f64) }`.
- `netclean::CutAccess { pub stretch: String, pub car_park: String, pub cut_m: f64, pub entry: (f64, f64),
  pub node: i64 }` (`node` the node at the entry, new or the old end node when nothing was cut).
- `netclean::drop_car_park_roads(graph: &RoadGraph, planes: &[CarParkPlane]) -> (RoadGraph, Vec<DroppedInCarPark>,
  Vec<CutAccess>, Vec<usize>)`: the last list holds the groups left unchanged because the removal would split a
  component (reason `would_split`).
- `netclean::components_with(graph: &RoadGraph, planes: &[CarParkPlane]) -> usize`: every plane is one node joined
  to each stretch that has a sample inside it or within the tolerance of its edge.
- `netclean::clean_with(graph: &RoadGraph, widths: &dyn Fn(Family) -> f64, cell_m: f64, planes: &[CarParkPlane])
  -> (RoadGraph, CleanReport)`; `clean` calls it with `&[]` and is unchanged in behaviour. The car-park step runs
  first; the stub step treats every `CutAccess::node` as not a free end.
- `CleanReport::car_park_roads: Vec<DroppedInCarPark>`, `CleanReport::car_park_access: Vec<CutAccess>`,
  `CleanReport::car_parks_split: Vec<usize>`, `CleanReport::junctions_in_car_parks: usize`; `components_before` and
  `components_after` count with `components_with`; `components_by_step` gets the car-park step first.

- [ ] **Step 1: Failing tests:**
  - a 40 × 40 car park (south edge y = 0) with a T of two service stretches wholly inside (a junction node at
    (20, 20)) and an access road from a street at y = −30 up to the T: both inside stretches removed and reported
    with their lengths, the junction counted; the access road keeps its id, now ends at (20, 4.0) ± 1e-6 at a new node
    whose id is one above the largest, `CutAccess::cut_m` 16 ± 1e-6; `components_with` equal before and after;
  - the same with the street at y = −6 (a 10 m wide `City` access road left 10 m long): `clean_with` keeps it (the
    stub step does not drop it), while `clean` on the same graph without planes would (control);
  - a road drawn along the outline, sharing the outline's nodes, stays whole (Review Focus 6);
  - a road crossing the car park from west to east stays whole; the same road with an aisle branching off it inside:
    the aisle is removed, the road's two stretches and their node stay;
  - a dead-end stretch that crosses the edge three times before ending inside is cut only after its last entry, its
    first inside part unchanged (Review Focus 2);
  - two car parks sharing an edge, a road wholly inside their union across the shared edge: removed; a car park B
    whose only road reaches it through car park A: `components_with` unchanged (Review Focus 3, 4);
  - a road cut at the map edge (negative end node) ending inside a plane: not cut (Review Focus 5);
  - a removal that would split a component (forced in a unit test through a private seam that replaces `splits`):
    nothing changes for that group, its index is returned;
  - a bridge stretch inside is never removed; a stretch with a NaN point is left as it is;
  - no planes: the graph is equal bit for bit and the report's car-park fields are empty; the result is the same for
    the stretches given in reverse order.
- [ ] **Step 2:** RED. **Step 3:** implement. **Step 4:** GREEN, `make check`.
- [ ] **Step 5:** Commit `feat: roads inside a car park are taken out of the road graph` (nothing calls
  `clean_with` with planes yet; no CHANGELOG line).

### Task 5: The surface (Sonnet; DeepSeek for Step 6)

**Files:** `crates/naves-core/src/surface.rs`, `crates/naves-core/src/carpark.rs`,
`crates/naves-export/src/layers.rs`, `crates/naves-export/src/rvmat.rs`, `crates/naves-export/src/generate.rs`
(`paint_car_parks`, not called yet), tests `crates/naves-core/tests/surface.rs`, `tests/carpark.rs`,
`crates/naves-export/tests/layers.rs`, `docs/formats/terrain-layers.md`.

**Interfaces — Consumes:** Task 1's `slot4.md`; `Assessed`, `CleanReport::{car_park_roads, car_park_access}`,
`conform::core_reach_m`.

**Interfaces — Produces:**

- `SurfaceClass::Tarmac` (slot 4, name `"Tarmac"`, `rgb()` the average colour of `en_tarmac_ca` from Task 1);
  `SurfaceClass::ALL: [SurfaceClass; 5]`. If Task 1 found no slot-4 colour, no new class: paved is
  `SurfaceClass::Soil` and the rest of this task's tarmac steps are skipped.
- `SurfaceGrid::fill_polygon(&mut self, outer: &[(f64, f64)], holes: &[Vec<(f64, f64)>], class: SurfaceClass)`:
  every cell whose centre is even-odd inside, clipped to the map; non-finite input paints nothing.
- `layers::SURFACES` gets the tarmac entry; `mask_rgba(4)` is Task 1's colour.
- `rvmat::tile_material` writes slot 4's textures only for a tile whose mask uses slot 4; it gets that from the
  caller (a `uses_slot4: bool` argument or the tile's slot set; implementer's choice, documented).
- `carpark::surfaces(assessed: &[Assessed], graph: &RoadGraph, clean: &CleanReport, cell_m: f64)
  -> Vec<(String, SurfaceClass)>`: one per car park that is not excluded, in id order (`graph` is the graph before the
  car-park step, for the frontage roads).
- `generate::paint_car_parks(surfaces: &mut SurfaceGrid, areas: &[CarParkArea], decided: &[(String, SurfaceClass)],
  class: SurfaceClass)`: paints the areas decided as `class`.

- [ ] **Step 1: Failing tests (surface):** `SurfaceClass::Tarmac.slot() == 4`; a 10 × 10 m square on a 1 m grid
  fills 100 cells, with a 4 × 4 m hole 84; a bow-tie fills both triangles; a polygon half off the map fills only the
  cells on it; NaN paints nothing.
- [ ] **Step 2: Failing tests (decision):** a car park with an 84 m `City` stretch removed → paved; 30 m `Grav`
  and 20 m `City` inside → grass; 25 m `Asf3` and 25 m `Mud` → paved (tie); none inside or entering but an `Asf3`
  street 3 m from its edge → paved; a `Mud` track 3 m away → grass; nothing within `core_reach_m + cell` → grass;
  car park B without roads in a group with paved A → paved (Review Focus 4); a street-side car park beside a `City`
  road → paved.
- [ ] **Step 3: Failing tests (layers):** first, before any change, a guard test that stores today's
  `tile_material` text for one tile and today's mask bytes for a fixture grid as expected values (it passes on the
  base commit and must still pass after: a tile without tarmac is written byte for byte as before); then the failing
  test: a tile with tarmac lists `en_tarmac_nopx` in `Stage11` and `en_tarmac_ca` in `Stage12`, and its mask uses Task
  1's colour for those pixels.
- [ ] **Step 4:** RED. **Step 5:** implement. **Step 6 (DeepSeek):** `docs/formats/terrain-layers.md`: the slot-4
  row (tarmac, its colour, its texture) and the sentence that slot 4 is written only on tiles that use it. **Step 7:**
  GREEN, `make check`. Check the GUI's ground legend (`crates/naves/src/gui/levels/ground.rs` reads
  `SurfaceClass::ALL`): if the GUI shows anything new, invoke `/frontend-design:frontend-design` first and keep the
  map item's DayZ look.
- [ ] **Step 8:** Commit `feat: a tarmac ground layer and the surface of a car park` (nothing paints car parks yet;
  no CHANGELOG line).

### Task 6: The plane in the terrain and the tie to access roads (Sonnet)

**Files:** `crates/naves-core/src/conform.rs`, `crates/naves-core/src/junctions.rs`,
`crates/naves-core/src/profile/model.rs` (and `profile/holds.rs` only if the `Inward` rule needs the plane's area as
its reach), `crates/naves-export/src/generate.rs` (`place_once_with`, `area_patches`), tests
`crates/naves-core/tests/conform.rs`, `tests/heightsolve.rs`, `tests/junctions.rs`,
`crates/naves-export/tests/level_patches.rs`.

**Interfaces — Consumes:** `CarParkPlane`, `CutAccess`, `conform::{LevelPatch, conform_bridges, SHOULDER_CELLS}`,
`junctions::{Junctions, Junction, JunctionKind, find_junctions}`, the ring path in `profile::model`
(`ring_of_junction`, `Source::Constant`).

**Interfaces — Produces:**

- `conform::AreaPatch { pub polygons: Vec<(Vec<(f64, f64)>, Vec<Vec<(f64, f64)>>)>, pub at: (f64, f64),
  pub height: f64, pub gradient: (f64, f64), pub rank: u8 }`.
- `conform::conform_areas(base: &Heightmap, strips: &[RoadStrip], patches: &[LevelPatch], areas: &[AreaPatch],
  bridges: &[BridgeSpan]) -> Result<Conformed, ConformError>`; `conform_bridges` calls it with `&[]`, unchanged.
- `JunctionKind::CarParkEntry`; `Junctions::car_parks: Vec<CarParkPlane>`;
  `junctions::add_car_park_entries(junctions: &mut Junctions, net: &PlacedNetwork, planes: &[CarParkPlane],
  entries: &[CutAccess])`: one junction per entry at the placed end of its access road, its height and gradient the
  plane's, `patch_m` half the road's width, `stretches` the access stretch, rank 0, kept in the `points` order.
- In `profile::model`, a `CarParkEntry` junction is a constant plane like a ring junction (`Source::Constant`, no
  height unknown).
- `generate::place_once_with(graph: &RoadGraph, catalog: &PieceCatalog, base: &Heightmap, planes: &[CarParkPlane],
  entries: &[CutAccess]) -> Result<Placed, BuildError>`; `place_once` calls it with `&[]`.
- `generate::area_patches(planes: &[CarParkPlane]) -> Vec<AreaPatch>` (rank 0).

- [ ] **Step 1: Failing tests (shaping):** flat ground at 100, one `AreaPatch` 20 × 20 at 101 level: every vertex
  inside at 101 ± 1e-4, every vertex within 1.5 cells beyond the edge at 101, the ground beyond falls back to 100 on
  the fill slope (no vertex below 100, none further than 1.5 m / `FILL_SLOPE` + 1 cell beyond the shoulder above
  100.01); a road strip at 100.5 through the patch: vertices inside the road's half width at 100.5; a 6 × 6 hole whose
  centre vertex keeps 100 (Review Focus 1); a vertex under a bridge span not claimed; two patches sharing an edge with
  one plane: the vertices on the shared edge equal on both sides (Review Focus 3); `conform_areas` with no areas equals
  `conform_bridges` bit for bit on the `tests/bridge_ground.rs` fixture.
- [ ] **Step 2: Failing tests (tie):** a straight 100 m `City` road on flat ground at 100, ending at an entry of a
  plane at 101: after `solve_clearing` the end joint is at 101 ± 1e-6, the joints inward that lie over the plane are
  at the plane's height there, and no grade on the road is above `grade_ceiling(City)`; the same with a plane at
  130 (out of reach in 100 m): the solve still returns, and the link is in `Junctions::unresolved` (a finding, not a
  panic); with no planes, `place_once_with` equals `place_once` bit for bit on the `tests/bridge_clearance.rs`
  fixture.
- [ ] **Step 3:** RED. **Step 4:** implement. **Step 5:** GREEN, `make check`.
- [ ] **Step 6:** Commit `feat: a car park is shaped to its plane and its access roads meet it` (nothing calls it
  with planes yet; no CHANGELOG line).

### Task 7: Car parks in the build, behind the switch; findings, report, manual (Sonnet; DeepSeek for Steps 4 and 5)

**Files:** `crates/naves-core/src/carpark.rs` (`CarParkReport`), `crates/naves-export/src/generate.rs`,
`crates/naves-core/src/roadplace.rs` (`PlacedNetwork`), `crates/naves-core/src/validate.rs`,
`crates/naves-core/tests/validate.rs`, `crates/naves-export/src/build.rs`, `crates/naves-export/tests/build.rs`, a new
`crates/naves-export/tests/car_parks.rs`, the callers of `fitted_graph_with` and `plan_roads`
(`tests/smoothing.rs`, the examples), `crates/naves/src/mcp.rs`, the binary's summary print, the `road-analysis`
example, `docs/manual/src/getting-started.md`, `troubleshooting.md`, `mcp.md`, `CHANGELOG.md`.

**Interfaces — Consumes:** everything above; `generate::Stages` from the road-planning plan if it has landed, else
this task creates `Stages { pub smooth: bool, pub car_parks: bool }` with `Stages::DEFAULT` (`SMOOTH_ENABLED`,
`CARPARKS_ENABLED`) and replaces `fitted_graph_with`'s `smooth: bool` with it (road planning then adds `reroute`).

**Interfaces — Produces:**

- `carpark::CarParkReport { pub assessed: Vec<Assessed>, pub planes: Vec<CarParkPlane>,
  pub surfaces: Vec<(String, SurfaceClass)> }` with `summary(&self, clean: &CleanReport) -> String`; the
  removals stay in `CleanReport`.
- `Stages::car_parks`; `generate::fitted_graph_with(features, catalog, base, stages, in_water, car_parks:
  &[CarParkArea])`; `FittedGraph::car_parks: Option<CarParkReport>` (`None` when off).
- `generate::plan_roads(…, car_parks: &[CarParkArea], …)` passes planes and entries to `place_once_with`;
  `place_roads` gives `area_patches` to `conform_areas`; `generate` paints grass car parks before the paths and paved
  ones after them, before the road beds.
- `PlacedNetwork::car_parks: Option<CarParkReport>`, `Generated::car_parks`, `BuildOutput::car_parks:
  Option<String>`.
- Rules: `Rule::CarParkBuilt` (`car_park_built`, information), `Rule::CarParkNotFlattened`
  (`car_park_not_flattened`, warning), `Rule::CarParkSurfaceOnly` (`car_park_surface_only`, information),
  `Rule::CarParkExcluded` (`car_park_excluded`, information), `Rule::RoadInCarParkDropped`
  (`road_in_car_park_dropped`, information, for a removed stretch and for a cut access road); `Rule::ALL` grows by
  five; `validate::car_park_violations(report: &CarParkReport, clean: &CleanReport) -> Vec<Violation>`.
- `build.rs::car_parks_json` for `roads.car_parks` in `build/report.json`: `{"enabled": false}` when off; else
  `enabled`, the counts by outcome and reason, paved and grass counts, `roads_removed`, `roads_removed_m`,
  `access_cut`, `junctions_removed`, and every car park (`id`, `x`, `y`, `area_m2`, `outcome`, `reason`, `surface`,
  `fall`, `depth_m`, `plane_height`, `plane_gradient`).

- [ ] **Step 1: Failing tests:**
  - `fitted_graph_with` with `car_parks: false` on the smoothing test's fixture plus one car park area equals the
    graph of today bit for bit, `car_parks` is `None`;
  - with `car_parks: true` on a fixture with one built car park holding a T of service roads: the two stretches are
    gone, one access road cut, `car_parks` is `Some` with one `Built`;
  - `fitted_graph(..).car_parks.is_some() == CARPARKS_ENABLED`;
  - a synthetic project build (the faked tools of `tests/build.rs`) with the stage on: the conformed heightmap is at
    the plane's height inside the car park (± 1e-3), the surface grid is tarmac there, `report.json`'s
    `roads.car_parks.built == 1`; with the stage off the heightmap, surface grid and every written layer file equal
    the build without the car park area;
  - rule ids as above; a report with one built, one not flattened (fall 0.17, depth 3.13), one tiny and one removed
    stretch gives one finding of each rule at the car park's centre (the stretch's middle for the road), the not
    flattened one naming both numbers and both limits;
  - `summary()` of that report is exactly `Car parks: 4, 1 built (1 paved, 0 grass), 1 not flattened, 2 surface only,
    0 excluded; 1 road (84 m) removed, 1 access road cut` with the fixture's numbers;
  - the MCP rule list test (`every_rule_is_documented_where_rules_are_listed`) passes with the five new ids.
- [ ] **Step 2:** RED. **Step 3:** implement the wiring, the progress line `Shaping the car parks` and the summary
  line.
- [ ] **Step 4 (DeepSeek):** `road-analysis`: read `CARPARKS` from the environment (`off`, `on`; default
  `CARPARKS_ENABLED`), print `STAGES car_parks=<bool>`, `CARPARK <summary>`, `CARPARK_OUTCOME <outcome> <reason>
  <count>`, `CARPARK_ROADS removed <n> <m> m, access cut <n>, junctions removed <n>`, `CARPARK_ENTRY <stretch>
  <step m>` (the height difference between each entry joint and the plane), `CARPARK_LEFT <id> <x> <y> <fall>
  <depth>` for each not flattened, and `CARPARK_NEAR <rule> <count>` for every rule within 50 m of a built car park.
- [ ] **Step 5 (DeepSeek):** the manual: getting-started "Roads" (car parks: what is removed, how access roads end,
  that it is off) and "Ground from real data" (paved and grass car parks, the tarmac layer); troubleshooting (the five
  rules, every reason, what "not flattened" means and its two limits); mcp.md (the rule list); `CHANGELOG.md` under
  Unreleased, Added: "Car parks (`amenity=parking`) get a paved or grass surface and are flattened to one plane;
  roads inside them are taken out of the road network and their access roads end just inside them ... switched off
  in this release (`CARPARKS_ENABLED`) ... with it on, edits on removed or cut roads are orphaned (`orphaned_edit`)
  ... `roads.car_parks` in `build/report.json`, findings `car_park_built`, `car_park_not_flattened`,
  `car_park_surface_only`, `car_park_excluded`, `road_in_car_park_dropped`".
- [ ] **Step 6:** GREEN, `make check` (with `make docs`).
- [ ] **Step 7:** Commit `feat: car parks in the build, switched off`.

### Task 8: Judging runs (controller with the Opus reviewer; Sonnet runs them)

**Files:** none in the repo. Output under `/var/tmp/naves-carparks/judge/`.

- [ ] **Step 1:** Rebuild the release example once (log HEAD and the binary's mtime). Check the machine (`nproc`,
  `free`, `uptime`); one run at a time, 16G cap each.
- [ ] **Step 2:** Two runs of `road-analysis` on Berchtesgaden at 2048 with the same binary: `CARPARKS=off` and
  `CARPARKS=on`. Logs `judge/<off|on>/ra-2048.log`, `RULES_OUT=judge/<off|on>/rules.tsv`.
- [ ] **Step 3:** Write `judge/compare.md`, one table, two columns, beside Task 1's baseline:
  - car parks by outcome and reason, paved and grass (spec: 126 built, 28 not flattened, 144 surface only, 0
    excluded; 109 of the built paved);
  - stretches and metres removed (spec: 9, 351 m), access roads cut (43), junction nodes removed (9);
  - components with built groups as nodes (must not grow); drivable length (must fall by no more than the removed and
    cut metres);
  - `road_gap` (count, total, largest), `road_conflict`, `road_not_drivable`, `road_misses_terrain`,
    `join_without_junction` map-wide and `CARPARK_NEAR` within 50 m; `road_grade`, `junction_too_steep`,
    `earthworks_capped`, `orphaned_edit` and every other rule;
  - the `CARPARK_ENTRY` steps (all must be under 0.05 m) and the steepest access link;
  - the stage's time and the build's time.
- [ ] **Step 4:** Judge against the spec's bar: no rule worse map-wide by more than 5 % without a reason that can be
  named; within 50 m of built car parks `road_gap` and `road_conflict` not worse; no entry step. A miss goes to the
  maintainer with the numbers; nothing is tuned until it is understood (one round at most, each change its own commit
  with a test, the spec's table in the same commit).
- [ ] **Step 5:** Write the game sites as a `sites.json` for `tools/ingame-check`: Königssee P1 (5741, 5171) and its
  through road, P2 and P3 Hirschbichlstraße (265, 5714) and (216, 5723), way 255488300 (2351, 6180), Parkplatz
  Salzbergwerk (6772, 7741), Hinterbrand (6967, 5309) as the not-flattened control, and Grafenlehen (5638, 5412), which
  v1 leaves unchanged.

No commit, unless Step 4's tuning changes a constant.

### Task 9: Checkpoint, the game (controller with a Sonnet agent)

- [ ] **Step 1:** Build the map twice: once from this branch as it is (stage off), once from a scratch worktree under
  `/var/tmp/naves-carparks/game/` with `CARPARKS_ENABLED = true` (not committed; its own target dir under the same
  folder). Log HEAD, the diff of the worktree and the binary's mtime each time. Keep both `build/` folders under
  `/var/tmp/naves-carparks/game/`.
- [ ] **Step 2:** `tools/ingame-check/run.sh` on each build with the same `sites.json` (24G cap, server 8G, client
  14G, non-default ports, one game at a time, headless sway only). Do a control run against vanilla Livonia first if
  anything looks broken. Stop the game when done; remove the `@naves*` symlinks from the DayZ install.
- [ ] **Step 3:** Send the pairs of contact sheets to the maintainer with one line per site: surface (tarmac shows,
  not dark red or flat colour), the plane (flat, no hole or dam, shoulder blended), the access road (no step,
  drivable onto the car park), anything that looks wrong.
- [ ] **Step 4:** The maintainer decides whether `CARPARKS_ENABLED` becomes `true`. If yes: a commit `feat!: car parks
  are switched on` that flips the constant, updates the manual ("off" becomes "on") and adds the **Breaking** note for
  orphaned edits to `CHANGELOG.md`; `make check`.

### Task 10: Final review (Opus)

- [ ] Review the whole branch against the spec and this plan: every spec rule has a test; the Review Focus tests
  exist and assert what is listed; the constants in the code equal the spec's table; no access road can be removed
  (read the step, not only the tests); with the stage off nothing changes (read the call sites: every new path is
  behind the flag); determinism (no hash-map iteration, `total_cmp`, sorted ids); no panics on input; coverage not
  below the base; the manual and the CLI help agree.
- [ ] Update the spec's status line with the result (built, judged, on or off) and its open points with what the
  judging and the game showed.
- [ ] Hand the list of not-flattened car parks (from Task 8) to the controller as the start of the terrace task, and
  the camp-site open point (Grafenlehen) to the maintainer.

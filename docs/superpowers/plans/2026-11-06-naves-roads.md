<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Náves M1, plan 3: roads — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `naves build` turns the OSM road network into fitted vanilla road pieces on terrain shaped for them, with junctions and bridges, and footpaths as ground texture, and proves it: roads and drivable tracks are checked to be drivable on the final terrain, everything smaller to be walkable, and all of it can be switched on and off as layers of the map, in the first part of the final editor's interface, with changeable shortcuts and its own settings.

**Architecture:** OSM ways become `Feature`s in map coordinates. A graph of stretches and chains is fitted piece by piece in plan view (major roads first, junctions sit where the through road passes). A height profile per chain then reshapes the terrain, which is held to a slope limit around the road at the level of the terrain grid, so a road on a valley side gets a real bench. The pieces become objects with a stable identity in the 8WVR, and checks measure the result on the final terrain. `naves-core` holds all of this without knowing about tools; `naves-export` reads the piece catalog from the user's install and runs the pipeline. The GUI and MCP draw the same layers from one renderer.

**Tech Stack:** Rust 1.95, existing `osmpbf`, `proj4rs`, `rayon`, `eframe`/`egui`; bzed's armake2 fork to read PBOs; DayZ Tools under Proton.

**Spec:** `docs/superpowers/specs/2026-11-04-naves-design.md`, sections 2 (goal 2), 3, 4, 5.1, 5.2, 5.3 (stages 3 to 5 and 9), 5.7 (last point), 5.8, 8 (step 3). Background: `docs/research/2026-11-03-dayz-osm-terrain-research.md` sections 3 and 5. Interface: `docs/superpowers/specs/2026-11-06-naves-editor-ui.md`. Evidence: `docs/research/2026-11-06-road-pieces.md`. Read that note first: it contradicts the spec's research on one point (see below).

**Starts from:** `main` after plan 2 (`docs/superpowers/plans/2026-11-06-naves-land-cover.md`, branch `m1-land-cover-plan`) is merged. This plan uses its `Sources.osm`, `naves_core::osm`, `GeoWindow`, `SurfaceGrid`, `build_surfaces` and `BuildOutput.warnings` and the `surface` base map.

**The finding that shapes this plan.** The research note assumed rigid pieces pitched per object. In Bohemia's sample all 549 road pieces and in Livonia 97.9 % of 35 730 road records have a vertical up axis, on a hilly map. So the engine very probably fits road pieces to the terrain itself. If Task 1 confirms that, a piece's transform has a yaw and a height and nothing else, and **the road's vertical alignment is the terrain under it**: smooth profile, flat cross-section, and straight between piece joints, because a 25 m piece is probably one quad. If Task 1 refutes it, only `piece_transform` (Task 9) changes.

**Added by the maintainer on 2026-11-06:** terrain must be drivable on gravel and paved roads and on drivable tracks, and everything smaller only must not be too steep to walk on; roads going into and out of valleys often miss their underlying terrain because its resolution is not fine enough, so the terrain has to change to suit them; these things go into switchable layers; the UI has to fit the final editor, with items to add, tools and shortcuts per level; and a settings window has to change the default key bindings and similar things (Tasks 1, 4, 8, 10, 12 and 13).

Not in this plan: reshaping the terrain for paths (a path that is too steep is reported, not repaired), generated road meshes for junctions and closures (spec section 7), trees and buildings, water planes, railways, thinning for scale, the cached layer graph, rule and preset files, editing roads by hand, tunnels as real tunnels (they need terrain holes, spec section 6).

## Global Constraints

- **`AGENTS.md` applies to every task**: tests first, coverage at or above 85 %, strict lints, `make check` before every commit, user docs and the `CHANGELOG.md` line in the same commit. Where this plan and `AGENTS.md` disagree, `AGENTS.md` wins.
- Spec section 4, verbatim: "**Milestone 1 uses vanilla assets only**, roads included: road pieces only from vanilla, with a small gap or overlap accepted within a tolerance where a stretch cannot close". "**Road pieces may overlap** (especially straights). Road fitting is an optimisation per stretch from crossing to crossing". "**Footpaths/trails** (`sac_scale` etc.) never get road pieces."
- Spec section 3, anti-goals: "Losing a manual edit when something is regenerated." "Railways or roads running through a lake or river." "Bumpy, kinked or floating roads."
- **No game data in the repo**: no ODOL, no extracted PBO, in fixtures neither. Tests build synthetic ODOL bytes and a synthetic catalog. Game files are read from the user's install at run time and never end up in the mod (Task 11 tests this).
- **Same input, same output:** no `HashMap` iteration in output. Floating-point sums over a parallel iterator depend on how rayon splits the work, so anything that feeds the output sums sequentially or over fixed chunks. Solvers run single-threaded unless a test shows two runs are byte-identical.
- Every parser of untrusted input gets a fuzz target whose seeds also run in `make test` (Tasks 2, 4, 9 and 13). No network in tests.
- Every Wine or Proton program runs in a headless sway session. One game at a time; remove the DayZ install's mod symlink afterwards. Large data goes under `/var/tmp`; run `make check` with `TMPDIR=/var/tmp`.
- Headings are radians clockwise from north (map +y). Map coordinates: x east, y north, metres, origin south-west. World files write x, height, y.
- Road pieces keep their real size at any `scale.horizontal` (spec 5.5). OSM geometry shrinks, the pieces do not.
- **What counts as a failure of this plan:** a road or drivable track a vehicle cannot climb, without a violation saying so; a road hanging in the air or buried in the slope anywhere, above all on a valley side.

## Review Focus

1. **Roads cut at the map edge or the edge of the extract:** every way is clipped, no piece reaches past the map, a cut end needs no closure and gets no end cap. Test in Task 4 and Task 7.
2. **Two crossings closer than a junction piece (12.5 m), which `scale.horizontal = 0.5` makes common in villages:** the second crossing falls back to a join without a junction piece, and nothing overlaps wrongly or panics. Test in Task 7.
3. **A bend tighter than the tightest piece (radius 10 m) after scaling, such as a hairpin:** the fit stays on the road's side of the bend as far as it can and reports its deviation. It neither loops nor fails the build. Test in Task 6.
4. **Parallel roads squeezed together by scaling, with different heights:** the higher class wins and the build reports a `road_conflict` violation, with no terrain spike. Test in Task 8.
5. **A catalog that no longer matches the game after a DayZ update, or a model that has gone:** the cache is rebuilt when the PBO changes, and a missing model names the file. Test in Task 3.

---

### Task 1: Gate: road objects from our world in the game

Answers the questions the rest of the plan depends on, on the real tools, before anything is built on them. Same form as plan 1's bridge experiment. Time-boxed to two days. **Stop and report to the maintainer** if the world with road objects cannot be made to load.

**Files:**
- Create: `crates/naves-formats/examples/road_probe.rs`, `docs/formats/road-objects.md`
- Modify: `crates/naves-formats/src/wrp8.rs` (the doc comment of `WorldObject::transform`), `crates/naves-formats/tests/wrp8.rs`, `docs/research/2026-11-03-dayz-osm-terrain-research.md` (a pointer at section 3.2)

**Interfaces:**
- Produces: `road_probe` reads an 8WVR and writes it back with a road layout added. `docs/formats/road-objects.md` records the answers for later tasks.

The probe layout, at the map's centre on the Berchtesgaden example terrain, on a real slope: a 200 m chain of `asf3_25` following the fall line, a curve of six `asf3_10 50`, a chain across a 20 % cross-slope, two `asf3_25` overlapping by 10 m, one piece 2 m above the ground, one piece with a 5° pitched transform, `kr_t_asf3_asf3` with a side road. Then the same chain on a stretch where the terrain has a sharp crest. And, on flat ground patched into the 8WVR: **drive ramps** of 60 m at 10, 15, 20, 25, 30, 35, 40 and 50 % grade, one set each under `asf3`, `grav` and `mud`; **walk ramps** of bare terrain at 30, 35, 40, 45, 50, 55, 60 and 65°.

- [ ] **Step 1: Write the test and fix the doc comment.** `transform_is_stored_column_by_column`: a hand-computed rotation in `WorldObject`, with `[0..3]` the image of the model's x axis. Doc comment: "3×3 rotation column by column (images of the model's x, y, z axes), then position x, height, y". Commit `docs(formats): 8WVR rotations are stored by column`.
- [ ] **Step 2: Build the example, run the probe on its staged 8WVR, and binarize it** (`tools/bridge-experiment.sh` shows the layout). Answer: does binarize need the models below `P:`? Add `"DZ_Structures_Roads"` to `requiredAddons` in the probe's config. If binarize drops or rejects the objects, extract `structures_roads.pbo` to `P:\dz\structures\roads` in the sandbox (read-only copy, in `/var/tmp`) and repeat.
- [ ] **Step 3: Read the result without the game.** Scan the OPRW for the probe's models. Answer: are they in the road net with connection positions? Is the height in the output the one we wrote or one binarize computed? Does the pitched piece keep its pitch?
- [ ] **Step 4: Look at it in the game.** Test server with the patched mission (`AGENTS.md`, *Comparable screenshots*), a client, screenshots of each element from close up, and a vehicle spawned at the chain in the mission's `init.c`. Answer:
  1. Does a piece with a vertical up axis follow the slope, or stay level and float? This decides the transform.
  2. Does the 2 m high piece stay high or sit on the ground? This decides what the height means.
  3. What happens on the crest: does the terrain poke through the 25 m quad, or does the piece bend at cell lines?
  4. Do overlapping pieces flicker? (A Steam comment reports road flicker after 1.27.)
  5. What surface do the wheels report, and does a vehicle drive over the joins without bumps?
  6. **How steep can a vehicle climb?** From a standstill, on each drive ramp, with a small car, a large car and the truck (for example Ada, Sarka, M3S). The steepest grade all three manage, per surface, times 0.8 gives `DRIVE_GRADE_PAVED`, `DRIVE_GRADE_GRAVEL` and `DRIVE_GRADE_TRACK`.
  7. **How steep can a player walk?** Straight up each walk ramp and diagonally. "Can walk" means the player keeps moving up at no less than half their speed on flat ground, with no sliding back. The steepest such slope, minus 5°, is `WALK_LIMIT_DEG`. The maintainer does not know this limit, and nor does the data: Livonia's navmesh config allows AI agents `maxSlope = 60`, which is an upper bound and not the player's limit. Record the speed per slope, so that the choice of threshold can be revisited.
  **Tools:** dayz-mcp (`~/workspace/mcp/dayz-mcp`, now ported to Linux and Proton): `player_teleport`, `player_move` and `query_player_state` for the walk ramps; `vehicle_control` at a fixed full throttle and `vehicle_telemetry` for the drive ramps, which are straight and need no steering; `scene_raycast` and `surface_query` for heights and surfaces. The server-side `vehicle_control` is known not to move PHYSICS cars: it runs from the owning client (its README, B3). If something in dayz-mcp does not work, it can be fixed, in that repo, as its own commits; tell the maintainer first if it takes more than half a day. **Driving that needs steering is the maintainer's:** it takes fast inputs. For those runs, have `vehicle_trace` record them at 20 Hz, so that bumps come out as numbers and not as an impression.
- [ ] **Step 5: Write `docs/formats/road-objects.md`** with each answer, the evidence and the date. Include the three limits and how they were measured. State the result as one of: **draped** (height and yaw only, terrain decides) or **rigid** (pitched transforms needed). Add the pointer in the research doc. Remove the mod symlink.
- [ ] **Step 6: Commit** `docs: how road objects behave from 8WVR to the game`. If the result is **rigid**, tell the maintainer before Task 9: the plan stays valid, and `piece_transform` takes pitch and roll from the profile.

### Task 2: ODOL reader

**Files:**
- Create: `crates/naves-formats/src/odol.rs`, `crates/naves-formats/tests/odol.rs`, `fuzz/fuzz_targets/odol_read.rs`, `fuzz/corpus/odol_read/*` (from the synthetic builder only)
- Modify: `crates/naves-formats/src/lib.rs`, `crates/naves-formats/tests/fuzz_seeds.rs`, `fuzz/Cargo.toml`, `docs/formats/road-objects.md`

**Interfaces:**
- Produces, in `naves_formats::odol`:
  - `pub struct OdolModel { pub version: u32, pub bbox_min: [f32; 3], pub bbox_max: [f32; 3], pub memory: BTreeMap<String, Vec<[f32; 3]>>, pub properties: BTreeMap<String, String> }`, with selection names lower-cased.
  - `pub fn read_odol(bytes: &[u8]) -> Result<OdolModel, OdolError>`. It accepts version 56 and rejects others with an error naming the version.

Layout: `ODOL` plus version, LOD count, resolutions, ModelInfo and the LOD address table, after bis-file-formats (MIT, reading it is allowed), with the v56 change from research section 2: 4 zero bytes in ModelInfo. The memory LOD has the resolution `1e15` and is reached through the address table, so the visual LODs are never parsed. Its arrays are short enough to be uncompressed in all pieces seen. If a real model has a compressed array in that LOD, return `OdolError::Compressed` and raise it with the maintainer: no LZO dependency for a case that may not exist.

- [ ] **Step 1: Write the failing tests** with a builder in the test file that writes the observed layout. `memory_points_are_read` (lb, le, pb, pe at known positions, including vertex indices out of order); `properties_are_read`; `other_versions_are_rejected`; `every_truncation_is_an_error` (each prefix of a valid file gives `Err`, never a panic); `lengths_do_not_allocate` (a count of `u32::MAX` is an error at once); `addresses_outside_the_file_are_errors`. The real-file test `#[ignore = "needs DayZ"] road_pieces_match_the_research_note` reads `structures_roads.pbo` through armake2 from the install and asserts: `asf1_25` lb (-6, 0, -12.5) and pe (6, 0, 12.5); `asf3_10 25` width 7, chord 4.358 and heading change 10° within 0.01; the six families' widths from the note; the two pieces the note could not read (`asf1_10 25`, `city_10 100`).
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.** Checked arithmetic for all offsets and no `Vec::with_capacity` from a file's length.
- [ ] **Step 4: Run** `cargo test -p naves-formats`, the ignored test with `-- --ignored`, and `make fuzz`. Expected: PASS. Add the bridge models' memory points and bounding boxes to `docs/formats/road-objects.md` from the same run: the scan could not read them.
- [ ] **Step 5: Commit** `feat(formats): read memory points and properties from ODOL models`.

### Task 3: Piece catalog

**Files:**
- Create: `crates/naves-core/src/catalog.rs`, `crates/naves-export/src/catalog_read.rs`
- Modify: `crates/naves-core/src/lib.rs`, `crates/naves-export/src/lib.rs`
- Test: `crates/naves-core/tests/catalog.rs`, `crates/naves-export/tests/catalog_read.rs`

**Interfaces:**
- Consumes: `read_odol`, `Tools.armake2`, `Tools.dayz_server`.
- Produces, in `naves_core::catalog`:
  - `pub enum Family { Asf1, Asf2, Asf3, City, Grav, Mud }`.
  - `pub struct Port { pub x: f64, pub z: f64, pub heading: f64 }`: a join point in model coordinates, heading along the road.
  - `pub enum PieceKind { Straight, Arc, EndCap }`.
  - `pub struct RoadPiece { pub model: String, pub family: Family, pub kind: PieceKind, pub width: f64, pub start: Port, pub end: Port }`.
  - `pub struct Junction { pub model: String, pub through: Family, pub side: Family, pub start: Port, pub end: Port, pub side_ports: Vec<Port> }`: one side port for a T, two for an X.
  - `pub struct BridgeModel { pub model: String, pub length: f64, pub width: f64, pub deck_height: f64, pub start: Port, pub end: Port }`.
  - `pub struct PieceCatalog`, with `pub fn from_models(models: &BTreeMap<String, OdolModel>) -> Result<PieceCatalog, CatalogError>`, `pieces(&self, Family) -> &[RoadPiece]`, `t_junction(&self, through: Family, side: Family) -> Option<&Junction>`, `x_junction(&self, through: Family, cross: Family) -> Option<&Junction>`, `bridges(&self) -> &[BridgeModel]`.
  - Feature `testing`: `pub fn synthetic_catalog() -> PieceCatalog`: true circular arcs built from the angle and radius table in the research note, all six families. Used by every test below, and contains no game data.
- Produces, in `naves_export::catalog_read`: `pub fn road_catalog(tools: &Tools, cache_dir: &Path) -> Result<PieceCatalog, CatalogError>`.

Skipped at this stage: `_crosswalk` pieces (they carry a marking, and a fit should not place them by accident), `runway_*`, `taxiway_*`, `centerline_*`, `path_*`, and Livonia's `*enoch` sets, which have no junctions. `kr_x_city_city_asf3` is skipped too (a different shape).

- [ ] **Step 1: Write the failing tests.**
  - `catalog.rs`: `arc_angles_come_from_the_models` (`_10 25` gives 10°, chord 4.358 within 0.001); `used_backwards_a_piece_turns_the_other_way` (via `RoadPiece::reversed()`, which swaps the ports and turns both headings by π); `junction_ports_are_found` (T has `ld`, `lh`; X has `ld`, `lh`, `pd`, `ph`, each pair giving one side port pointing away from the through road); `a_t_is_found_by_its_families`; `an_x_for_a_pair_that_does_not_exist_is_none`; `a_family_without_straights_names_the_missing_model`.
  - `catalog_read.rs` (a fake armake2 on `PATH`, synthetic ODOL): `the_catalog_is_cached` (second call runs no tool, measured by the fake's call count); `a_changed_pbo_rebuilds_the_cache` (size or mtime changes); `a_missing_pbo_names_the_file`; `a_damaged_model_names_the_model`; `extraction_happens_outside_the_project` (no file below the project dir except the cache).
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.** Extract with `armake2 unpack` into a `tempfile` directory; keep only the parsed result as JSON in the cache (`$XDG_CACHE_HOME/naves/catalog/`, spec 5.11: the catalog is per user and never distributed).
- [ ] **Step 4: Run** `cargo test -p naves-core -p naves-export`, and once against the real install: `#[ignore = "needs DayZ"] catalog_of_the_installed_game` (six families with 14 or more pieces each, at least 15 junctions). Expected: PASS.
- [ ] **Step 5: Commit** `feat: catalog of vanilla road pieces read from the install`.

### Task 4: Road features from OSM

**Files:**
- Create: `crates/naves-core/src/features.rs`, `tests/fixtures/osm/schoenau.osm.pbf` (crop made once with `osmium extract` from the Geofabrik `oberbayern` extract; attribution in `tests/fixtures/README.md`), `fuzz/fuzz_targets/osm_ways.rs`, `fuzz/corpus/osm_ways/*`
- Modify: `crates/naves-core/src/osm.rs`, `crates/naves-core/src/heightmap.rs`, `crates/naves-core/src/lib.rs`, `fuzz/Cargo.toml`
- Test: `crates/naves-core/tests/features.rs`, `crates/naves-core/tests/osm.rs`

The existing fixture `koenigssee.osm.pbf` has no highways. The new crop is a village with a main road, a side road, a crossing, a bridge, a track and a path. Keep it below 200 KB.

**Interfaces:**
- Produces, in `naves_core::osm`: `pub struct OsmWay { pub id: i64, pub tags: BTreeMap<String, String>, pub nodes: Vec<i64>, pub points: Vec<(f64, f64)> }`; `pub struct OsmWays { pub ways: Vec<OsmWay>, pub skipped: usize }`; `pub fn read_ways(paths: &[PathBuf], window: GeoWindow) -> Result<OsmWays, OsmError>`, and `read_ways_from_bytes` for the fuzz target. Same passes and panic guard as `read_land`; only ways with `highway=*` and only the tags the table below reads.
- Produces, in `naves_core::heightmap`: `pub struct MapFrame`; `MapFrame::new(&Project) -> Result<MapFrame, ProjectionError>`; `to_map(&self, lat: f64, lon: f64) -> Result<(f64, f64), ProjectionError>`; `to_geo(&self, x: f64, y: f64) -> Result<(f64, f64), ProjectionError>`. `build_heightmap` and `build_surfaces` use it (spec: one formula for map and real positions).
- Produces, in `naves_core::features`:
  - `pub enum Source { Osm, User, Ai }` (spec 5.2; only `Osm` is produced in this plan).
  - `pub enum FeatureKind { Road { family: Family, drivable: bool, bridge: bool, tunnel: bool }, Path { width_m: f64 } }`. `drivable` decides which check a road gets in Task 10.
  - `pub struct Feature { pub id: String, pub source: Source, pub kind: FeatureKind, pub points: Vec<(f64, f64)>, pub nodes: Vec<i64> }`: `points` and `nodes` have equal length, in map metres. Clipping creates end nodes with negative ids.
  - `pub fn classify(tags: &BTreeMap<String, String>) -> Option<FeatureKind>`.
  - `pub fn road_features(ways: &OsmWays, frame: &MapFrame, map_size_m: f64) -> Vec<Feature>`.

Classification (**[my call]**, from the spike; `*_link` as its parent; every other road is drivable). `motor_vehicle=no` and `access=*` do not count: they say what is allowed in the real world, and a forestry road in the national park is still physically a road.

| OSM | Result |
|---|---|
| `motorway`, `trunk`, `primary` | Asf1 |
| `secondary` | Asf2 |
| `tertiary`, `unclassified` | Asf3 |
| `residential`, `living_street` | City |
| `service` (no `service=driveway` or `parking_aisle`) | City |
| `track` with `tracktype=grade1`, `grade2` | Grav |
| `track` with no `tracktype` and `surface=asphalt`, `paved`, `concrete`, `gravel`, `fine_gravel`, `compacted` | Grav |
| `track` with `tracktype=grade3`, or with no `tracktype` | Mud, drivable |
| `track` with `tracktype=grade4`, `grade5`, or `smoothness=horrible` or worse | Mud, not drivable: walk check only |
| `service=driveway`, `parking_aisle` | Path, 3 m |
| `path`, `footway`, `cycleway`, `bridleway`, `steps`, `pedestrian` | Path, 1.6 m |
| any `highway` with `sac_scale=hiking`, `mountain_hiking` | Path, 1.6 m |
| any `highway` with `sac_scale=demanding_mountain_hiking` | Path, 1 m |
| any `highway` with `sac_scale=alpine_hiking` or harder, or `trail_visibility=bad` or worse | none: no trace on the ground |
| `construction`, `proposed`, `raceway`, `platform`, `area=yes`, `abandoned`, `razed` | none |

- [ ] **Step 1: Write the failing tests.** `classification_table` (a row per line above; `tracktype` wins over `surface`; `smoothness=horrible` beats `tracktype=grade1`); `access_tags_do_not_change_drivable`; `sac_scale_never_gives_a_road` (every `highway` value with every `sac_scale` value gives `Path` or `None`, even `track` and `primary`, `proptest`); `clipped_way_ends_at_the_inset_border` (a way leaving the map ends on the line 13 m inside it, half the longest straight, on an end node with a negative id; a way touching only outside gives nothing; a way that leaves and comes back gives two features with distinct ids); `to_map_inverts_to_geo` and `to_map_uses_the_map_scale` (at `scale.horizontal` 0.5); `fixture_has_the_main_road`; `tags_are_filtered`; `damaged_files_are_errors` (seeds, also run from `fuzz_seeds.rs`); `missing_nodes_count_as_skipped`.
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.** A way with `bridge=*` other than `no` is `bridge: true`; the same rule for `tunnel`.
- [ ] **Step 4: Run** `cargo test -p naves-core`, `make fuzz`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): road and path features from OSM`.

### Task 5: Road graph

**Files:**
- Create: `crates/naves-core/src/roadnet.rs`
- Modify: `crates/naves-core/src/lib.rs`
- Test: `crates/naves-core/tests/roadnet.rs`

**Interfaces:**
- Consumes: `Feature`.
- Produces, in `naves_core::roadnet`:
  - `pub struct Stretch { pub id: String, pub family: Family, pub drivable: bool, pub bridge: bool, pub tunnel: bool, pub points: Vec<(f64, f64)>, pub start: i64, pub end: i64 }`. `id` is `<lowest way id>:<start node>-<end node>`, the stable part of an object's identity, independent of OSM's way order.
  - `pub struct Crossing { pub node: i64, pub point: (f64, f64), pub stretches: Vec<usize> }`.
  - `pub struct RoadGraph { pub stretches: Vec<Stretch>, pub crossings: Vec<Crossing> }`; `pub fn build_graph(features: &[Feature]) -> RoadGraph`.
  - `pub struct Chain { pub links: Vec<ChainLink> }`; `pub struct ChainLink { pub stretch: usize, pub reversed: bool }`; `pub fn chains(graph: &RoadGraph) -> Vec<Chain>`.

**Splitting:** a stretch runs from crossing to crossing and joins ways through degree-2 nodes (the spike's biggest single improvement). It also splits where family, `drivable`, `bridge` or `tunnel` changes.

**Chains:** at each crossing, the two arms with the smallest heading change, if below 45° and of the same family, are one through road. Everything else starts or ends there. A chain is a maximal path through such pairs. Chains are ordered: family rank (Asf1, Asf2, City, Asf3, Grav, Mud), then length descending, then id. This order is the placement order of Task 7.

- [ ] **Step 1: Write the failing tests.** `degree_two_nodes_are_joined`; `two_ways_between_the_same_crossings_stay_two_stretches` (ids differ); `family_change_splits`; `bridge_splits`; `a_t_has_one_through_road` (straight road with a side road gives two chains, the straight one with two links first); `the_straighter_pair_is_the_through_road` (a Y at 30° and 60°); `no_through_road_above_45_degrees`; `different_families_are_not_through`; `a_ring_is_one_chain_that_ends_where_it_starts` (terminates); `ids_do_not_depend_on_input_order` (`proptest`, features shuffled, same ids and same chains).
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p naves-core --test roadnet`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): road graph with stretches and through chains`.

### Task 6: Fitting pieces to a line

The core algorithm: one line in, a closed sequence of pieces out.

**Files:**
- Create: `crates/naves-core/src/roadfit.rs`
- Modify: `crates/naves-core/src/lib.rs`
- Test: `crates/naves-core/tests/roadfit.rs`

**Interfaces:**
- Consumes: `RoadPiece` (with `reversed()`), `synthetic_catalog`.
- Produces, in `naves_core::roadfit`:
  - `pub struct Pose { pub x: f64, pub y: f64, pub heading: f64 }`.
  - `pub enum EndGoal { Free, Fixed(Pose) }`.
  - `pub struct FitPiece { pub model: String, pub origin: Pose, pub start: Pose, pub end: Pose }`.
  - `pub struct StretchFit { pub pieces: Vec<FitPiece>, pub gap_m: f64, pub heading_error_deg: f64, pub median_deviation_m: f64, pub max_deviation_m: f64 }`.
  - `pub fn fit_line(path: &[(f64, f64)], start: Pose, goal: EndGoal, pieces: &[RoadPiece]) -> Result<StretchFit, FitError>`. `FitError` only for input that cannot be fitted at all (fewer than two points, non-finite values). A poor fit is a result, not an error.
  - `pub const GAP_CLOSED_M: f64 = 0.10; pub const GAP_TOLERATED_M: f64 = 0.50; pub const KINK_TOLERATED_DEG: f64 = 1.0;` (**[my call]**. Bohemia's joins have a p90 near 3 cm, and the spike reached 0.5 m on half of the stretches with a greedy search).

Algorithm (from research section 3.1; it replaces the spike's greedy heading tracker):

1. **Beam search over arc sequences.** State: pose, progress along the line, cost, the sequence so far. Moves: each arc in both directions, and a straight of 6.25, 12.5 or 25 m. Progress advances along the line by projecting onto a window ahead of the current progress, never over the whole line, so that a hairpin does not jump across. Cost per move: length × (squared lateral deviation + heading term) plus a small penalty per piece and per unit of curvature, so that a straight beats a chain of tiny arcs. Keep the best 200 states by cost at each progress bucket; ties by the sequence order, so the result is deterministic. Stop when progress reaches the end; a guard on piece count (line length ÷ 2 + 16) ends runaway searches.
2. **Local improvement.** Swap arcs of the same angle with a different radius (10° at R 25, 50, 75, 100 differ only in length) and insert ±1° S-bends into straights above 40 m. Keep a change only if the closure or the cost gets better.
3. **Closure, for `EndGoal::Fixed`.** For a fixed arc sequence the end position is linear in the straight run lengths, so solve the 2 × 2 minimum-norm system for the length changes. Run lengths are bounded: 0 or at least 6.25. Runs that hit a bound leave the active set and the system is solved again, at most 8 rounds. No dependency: it is a 2 × 2 solve.
4. **Realising a run.** A run of length L becomes 25 m pieces, then 12.5 and 6.25, with the last piece overlapping the one before it so that the run ends exactly at L (spec: overlap is allowed for straights). The overlap never exceeds half a piece.

- [ ] **Step 1: Write the failing tests** with `synthetic_catalog`.
  - `a_straight_line_uses_straights` (a 1000 m line: no arcs, gap 0 at a `Fixed` end, at most 45 pieces);
  - `any_length_closes_with_straights` (`proptest` on lengths from 6.25 to 3000: gap below 1 mm);
  - `a_circle_uses_the_matching_radius` (R 100 quarter circle: no arc tighter than R 50, median deviation below 1 m);
  - `an_s_curve_closes` (end position and heading fixed: gap below `GAP_CLOSED_M`, heading error below 1°);
  - `pieces_join` (each piece's `start` equals the previous `end` within 1 mm, in every test here);
  - `the_free_end_has_no_gap`;
  - `a_hairpin_tighter_than_the_tightest_arc_does_not_loop` (radius 5 m at 180°: returns, at most 40 pieces for 200 m, deviation reported above 0);
  - `progress_does_not_jump_across_a_hairpin` (two parallel legs 15 m apart: the pieces follow the first leg to its end);
  - `the_same_input_gives_the_same_pieces` (twice, byte equal);
  - a corpus test `closure_rate_on_the_stretches_of_the_fixture` (all closed stretches of `schoenau.osm.pbf`, at scale 1.0 and 0.5: at least 90 % below `GAP_TOLERATED_M`, median deviation below 6 m).
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.** Write the search as plain functions over a slice of pieces so that the tests can drive them. Note the time per kilometre in the commit message.
- [ ] **Step 4: Run** `cargo test -p naves-core --test roadfit --release`. Expected: PASS. If the corpus test falls short after honest tuning, write down the numbers and raise it with the maintainer rather than lower the bar: spec section 7's generated road models exist for exactly this case.
- [ ] **Step 5: Commit** `feat(core): fit road pieces to a line`, naming the solvers considered (research section 3.1 lists MILP and CP-SAT) and why they were not used.

### Task 7: Placing the network

**Files:**
- Create: `crates/naves-core/src/roadplace.rs`
- Modify: `crates/naves-core/src/roadfit.rs`, `crates/naves-core/src/lib.rs`
- Test: `crates/naves-core/tests/roadplace.rs`

**Interfaces:**
- Consumes: `RoadGraph`, `chains`, `fit_line`, `PieceCatalog`.
- Produces, in `naves_core::roadplace`:
  - `pub struct Slot { pub stretch: String, pub index: u32 }`.
  - `pub enum PlacedKind { Piece, Junction, Bridge, EndCap }`.
  - `pub struct PlacedRoad { pub slot: Slot, pub kind: PlacedKind, pub family: Family, pub drivable: bool, pub model: String, pub origin: Pose, pub start: Pose, pub end: Pose, pub width: f64 }`.
  - `pub struct RoadGap { pub stretch: String, pub at: (f64, f64), pub gap_m: f64, pub heading_error_deg: f64 }`.
  - `pub struct PlacedNetwork { pub roads: Vec<PlacedRoad>, pub gaps: Vec<RoadGap>, pub joins_without_junction: Vec<(f64, f64)>, pub tunnels_omitted: Vec<(String, (f64, f64))>, pub chains: Vec<Vec<usize>> }`: `chains` holds indices into `roads`, in driving order, for Task 8.
  - `pub fn place_network(graph: &RoadGraph, catalog: &PieceCatalog) -> Result<PlacedNetwork, PlaceError>`.

Rules, hierarchical instead of the joint solve the research note discusses (**[my call]**: it needs closure only where a road meets something already placed, which removes most of the spike's closures, and the through road decides where a junction sits, which is the "floating" the spec asks for):

- Chains in the order of Task 5. A chain's ends are `Free` unless they meet a port or a piece already placed.
- **Junctions in a through chain:** when the fit passes a crossing, it emits the `kr_t` or `kr_x` piece for (through family, family of the side road), as a 12.5 m straight in the sequence. The fit has to pass within 3 m of the crossing, or the junction drops to a join without a junction piece. Its side ports become the fixed start of the side roads.
- **Side roads start at a right angle:** vanilla junctions have nothing else. The fit bends towards the OSM course from there, and its deviation near the crossing is expected.
- **No model for the pair**, or an end with five or more arms, or a second crossing within 12.5 m along the road of one already placed: no junction piece. The side road ends on the through road's edge, overlapping by up to 2 m, and `joins_without_junction` records its position.
- **Dead ends and map edges:** dead ends get the `_6konec` end cap. Ends with a negative node id (clipped, Task 4) get neither cap nor closure.
- **Tunnels:** a `tunnel` stretch gets no pieces, its neighbours end in end caps, and it goes into `tunnels_omitted` with its start position (real tunnels need terrain holes, milestone 2).
- **Bridges** are Task 9's addition: here a `bridge` stretch is an ordinary stretch.
- **Slots** count along the stretch in its stored direction, whichever way the chain ran through it, so that an object's identity does not change when a chain changes.

- [ ] **Step 1: Write the failing tests.** On small synthetic graphs: `a_t_places_a_junction_and_a_side_road`; `a_side_road_starts_at_the_port` (within 1 mm, heading included); `a_crossing_places_an_x`; `a_pair_without_a_model_joins_without_a_junction` (`grav` onto `asf3`); `two_crossings_close_together_use_one_junction` (6 m apart: one junction, one join, no piece overlaps the junction by more than 2 m); `dead_ends_have_end_caps`; `a_clipped_end_has_no_cap_and_stays_inside_the_map`; `a_side_road_between_two_through_roads_closes_or_reports_a_gap`; `a_tunnel_leaves_a_hole_and_is_reported`; `a_ring_closes_on_itself`; `slots_do_not_change_when_a_stretch_is_walked_backwards`; `a_changed_side_road_does_not_move_the_through_road` (edit a side road's points: the through road's pieces stay byte equal); `the_same_input_gives_the_same_network`.
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement,** adding to `fit_line` a list of forced elements at given progress values (junctions now, bridges later).
- [ ] **Step 4: Run** `cargo test -p naves-core`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): place the road network, junctions and end caps`.

### Task 8: Height profiles and shaping the terrain

This is where roads stop missing their terrain. The cause on a valley side: the DEM has a 30 m grid, the map's vertices are 6.25 m apart, and a road 5 to 12 m wide crosses a slope that the grid can only show as a ramp. So the terrain does not get smoothed towards the road; it is **held to a slope limit around the road, at every grid vertex**, and whatever sticks out is cut, whatever is too low is filled.

This replaces the least-squares solve of research section 5 (spec section 10 leaves the terrain solver to the builder). A least-squares solve trades smoothness against closeness and promises no slope limit, which is the property the valley case needs. The method below guarantees one, gives each side of a road its own width of earthworks, shows a conflict between two roads as an empty range, and needs no sparse solver and no new dependency.

**Files:**
- Create: `crates/naves-core/src/profile.rs`, `crates/naves-core/src/conform.rs`
- Modify: `crates/naves-core/src/lib.rs`
- Test: `crates/naves-core/tests/profile.rs`, `crates/naves-core/tests/conform.rs`

**Interfaces:**
- Consumes: `PlacedNetwork`, `Heightmap`.
- Produces, in `naves_core::profile`:
  - `pub fn max_grade(family: Family) -> f64` and `pub fn max_grade_change(family: Family) -> f64`.
  - `pub struct RoadStrip { pub slots: Vec<usize>, pub joints: Vec<(f64, f64, f64)>, pub half_width: f64, pub rank: u8 }`: heights at the piece joints, linear between them.
  - `pub struct GradeExcess { pub at: (f64, f64), pub grade: f64, pub limit: f64 }`.
  - `pub struct Profiles { pub strips: Vec<RoadStrip>, pub excesses: Vec<GradeExcess> }`.
  - `pub fn solve_profiles(net: &PlacedNetwork, hm: &Heightmap) -> Profiles`.
- Produces, in `naves_core::conform`:
  - `pub const CUT_SLOPE: f64 = 1.0; pub const FILL_SLOPE: f64 = 0.667;` rise over run (45° and 34°): steeper than natural ground is fine in a cutting, an embankment should look like one. **The maintainer does not know the right values and nor do I:** these are starting points, chosen so that an embankment looks like one. Task 14 compares them on the real map. Both must stay below `WALK_LIMIT_DEG`, so that a player can always leave the road.
  - `pub struct RoadConflict { pub at: (f64, f64), pub height_difference_m: f64 }`.
  - `pub struct Conformed { pub heightmap: Heightmap, pub max_cut_m: f32, pub max_fill_m: f32, pub conflicts: Vec<RoadConflict>, pub capped: Vec<(f64, f64)> }`.
  - `pub const MAX_EARTHWORK_M: f64 = 60.0;`, how far earthworks reach from a road's core. A starting point too, for Task 14 to compare.
  - `pub fn conform(base: &Heightmap, strips: &[RoadStrip]) -> Result<Conformed, ConformError>`.

Limits on the map's slopes (**[my call]**; `max_grade_change` is per joint, the difference between neighbouring pieces' grades, and it is the number that makes a road feel bumpy). These are what the profile aims for; Task 10 checks the hard limits Task 1 measured.

| Family | `max_grade` | `max_grade_change` |
|---|---|---|
| Asf1 | 0.08 | 0.02 |
| Asf2 | 0.10 | 0.025 |
| Asf3 | 0.12 | 0.03 |
| City | 0.14 | 0.04 |
| Grav | 0.18 | 0.05 |
| Mud | 0.25 | 0.08 |

**Profile.** Per chain in placement order: minimise the squared distance to the terrain under the joints plus the squared second difference, with the grade limits. A band system solved directly (no crate) and alternating with clipping passes, a fixed 30 rounds so that the result is deterministic. Ends on an already placed road take its height and stay fixed. Where fixed ends make the limit impossible, keep the ends and report a `GradeExcess`. A junction piece has one height along its length. **Switchbacks:** a third clipping pass holds joints of different parts of the network to `|height difference| ≤ distance × FILL_SLOPE`, when they are closer than `half_width × 2 + 3 cells` and more than 50 m apart along the road. Without it, two legs squeezed together by scaling ask for a wall between them.

**Shaping.** `base` stays as it is, because the spec's stack keeps layers apart, and the result is a new heightmap.

1. **Core:** every vertex within `half_width + 1.5` cells of a strip's centre line takes the strip's height at its closest point. Half a diagonal more than the road's edge means every terrain triangle under the road has all three corners on the road, whatever way the engine splits a cell. That is the guarantee against a road missing its terrain, and the reason a coarse grid gives a wider bench. Where cores differ by more than 0.3 m with no junction, the higher rank wins and a `RoadConflict` is reported.
2. **Limits:** two whole-grid fields, `upper` (core height + `CUT_SLOPE` × distance to the core) and `lower` (core height − `FILL_SLOPE` × distance), as two-pass 8-neighbour chamfer sweeps from all cores together. This is linear in the grid and deterministic.
3. **Clamp where it is connected to the road:** a flood from the core through neighbouring vertices whose base lies outside `lower..=upper`, stopping at the first vertex inside the range (where the earthwork meets natural ground) and at `MAX_EARTHWORK_M`. Only flooded vertices take the base clamped to the range. **Without the flood, a whole-grid clamp would shave every mountain that rises steeply from a road, even 300 m away; the Königssee's walls are exactly that.** A cliff beyond the meeting line stays a cliff. Where the flood stops at the cap, a real rock wall meets the road: the step stays and goes into `capped`. A vertex where `lower > upper` is between two roads that cannot both be met: it takes the bound of the higher rank's road, and a `RoadConflict` is reported.
4. **Round the creases:** four passes of neighbour averaging on the vertices that changed, excluding the core, clamped back into the limits after each pass, in a fixed order.

Outside the earthworks the terrain is byte for byte the base. Inside the cap they are as wide as the slope demands: a metre of cut costs a metre on the map.

- [ ] **Step 1: Write the failing tests.**
  - `profile.rs`: `a_flat_road_stays_on_the_terrain`; `grade_limit_holds` and `grade_change_limit_holds` (`proptest` on random terrain profiles, seeds kept); `a_steep_ramp_between_fixed_ends_reports_an_excess`; `side_road_starts_at_the_through_road_height`; `a_junction_is_level_along_its_length`; `switchback_legs_leave_room_for_a_slope` (legs 30 m apart on a 35° flank: height difference at most 30 × `FILL_SLOPE`); `the_profile_is_deterministic`.
  - `conform.rs`, on synthetic terrain:
    - `the_road_core_is_at_the_profile` (all core vertices within 1 mm); `the_cross_section_is_flat`; `the_terrain_is_straight_between_joints`.
    - `every_triangle_under_the_road_has_its_corners_on_the_road` (a road at 0°, 17°, 45° to the grid, on cells of 6.25 and 12.5 m, each cell's corners under the road's footprint, both diagonals).
    - **The valley cases**, each asserting no terrain above or below the road surface within the footprint, slope limits held and daylight on both sides: `road_along_a_35_degree_valley_side_gets_a_bench` (cut uphill, fill downhill, wider than the road); `road_down_into_a_v_valley` (a 12 % road crossing the fall line into a V-shaped valley narrower than two DEM cells, as a bilinear 30 m DEM shows it); `road_across_a_valley_floor_gets_an_embankment`; `road_through_a_ridge_gets_a_cutting`; `a_switchback_on_a_steep_flank`.
    - `slope_never_exceeds_the_limits_next_to_the_core` (`proptest`: no two neighbouring vertices near a road, where the base was gentler than the limits, differ by more than the limit × cell × 1.1, after rounding too; the 1.1 is the chamfer's error); `earthworks_end_where_they_meet_the_ground` (on a uniform slope the changed area ends at the distance worked out by hand, within two cells); `a_cliff_beyond_the_meeting_line_stays` (a 70° wall 100 m from a road on gentle ground: bytes equal); `a_wall_next_to_the_road_stops_at_the_cap_and_is_reported` (a 70° wall rising directly from the road: no change beyond `MAX_EARTHWORK_M`, one `capped` entry or more); `nothing_changes_outside_the_earthworks` (bytes equal); `parallel_roads_at_different_heights_report_a_conflict` (no vertex outside both input heights' range); `the_base_is_untouched`; `a_road_along_the_map_edge_does_not_panic`; `conforming_is_deterministic` (two runs byte equal).
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p naves-core --test profile --test conform`. Expected: FAIL.
- [ ] **Step 3: Implement.** Measure on a 2048² map with 20 km of road and note the time in the commit message.
- [ ] **Step 4: Run** `cargo test -p naves-core`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): road height profiles and terrain shaped to fit them`, saying why the least-squares solve of research section 5 was not used.

### Task 9: Bridges, objects with identity, and overrides

**Files:**
- Create: `crates/naves-core/src/objects.rs`, `fuzz/fuzz_targets/overrides_parse.rs`, `fuzz/corpus/overrides_parse/*`
- Modify: `crates/naves-core/src/roadplace.rs`, `crates/naves-core/src/profile.rs`, `crates/naves-core/src/conform.rs`, `crates/naves-core/src/lib.rs`, `fuzz/Cargo.toml`, `docs/manual/src/project-file.md`
- Test: `crates/naves-core/tests/objects.rs`, `crates/naves-core/tests/roadplace.rs`, `crates/naves-core/tests/fuzz_seeds.rs`

**Interfaces:**
- Consumes: `PlacedNetwork`, `Conformed`.
- Produces, in `naves_core::objects`:
  - `pub struct ObjectId { pub feature: String, pub preset: String, pub slot: u32 }` with `Display` and `FromStr` (`road/<stretch>/<slot>`). Spec 5.2: identified by source, not by position.
  - `pub struct GeneratedObject { pub id: ObjectId, pub model: String, pub x: f64, pub y: f64, pub yaw: f64 }`.
  - `pub enum Override { Moved { x: f64, y: f64, yaw: f64 }, Deleted, Replaced { model: String } }`.
  - `pub struct PinnedEdit { pub id: ObjectId, pub generated_model: String, pub generated_at: (f64, f64), pub edit: Override }`.
  - `pub struct Overrides { pub edits: Vec<PinnedEdit> }`; `Overrides::load(dir: &Path)` reads `edits/overrides.toml`, a missing file gives an empty list; `save` writes atomically.
  - `pub struct PlacedObject { pub id: Option<ObjectId>, pub model: String, pub x: f64, pub y: f64, pub yaw: f64 }`: `None` for an orphaned edit, which is now a placed object.
  - `pub struct OrphanedEdit { pub id: ObjectId, pub at: (f64, f64) }`.
  - `pub fn apply_overrides(generated: &[GeneratedObject], overrides: &Overrides) -> (Vec<PlacedObject>, Vec<OrphanedEdit>)`.
  - `pub fn road_objects(net: &PlacedNetwork) -> Vec<GeneratedObject>`.
  - `pub fn piece_transform(object: &PlacedObject, ground_height: f32) -> [f32; 12]`.

`piece_transform` for yaw θ, per the research note and Task 1's **draped** result: `[cos θ, 0, -sin θ, 0, 1, 0, sin θ, 0, cos θ, x, h, y]`.

**Orphans (spec 5.2):** an edit whose generated object is gone, or whose object now has a different model or lies more than 25 m from `generated_at`, was made for something else. The edit "is kept exactly as it is and becomes a placed object, and a validation warning points at it". A `Deleted` edit with no object left has nothing to keep and is dropped silently, since the deletion already holds. Slots can shift when a stretch refits, and this comparison is what stops an edit moving onto a different piece.

**Bridges** (from the measured models in `docs/formats/road-objects.md`): a `bridge` stretch at least 10 m long becomes a straight run of bridge models along its chord (`Stone` for Asf and City, `Wood` for Grav and Mud; the widest fit if none is wide enough, with a warning; **[my call]**, check it in Task 14). The chord is a forced element of the fit, as junctions are. Shorter bridges stay ordinary road. The bridge is level: its deck height is the profile's height at both ends, and the terrain keeps the base height below the span, since a road's core and earthworks stop at the bridge's ends. Deck height comes from `BridgeModel.deck_height`. Bridges are rigid objects whose height comes from `deck_height`: Task 1 found out what the height of an object means, and `piece_transform` writes it for bridges accordingly.

- [ ] **Step 1: Write the failing tests.** `objects.rs`: `the_id_round_trips` (`Display` and `FromStr`, with a stretch id holding `:` and `-`); `piece_transform_matches_the_sample` (θ = 120° gives the hand-worked `[-0.5, 0, -0.866, 0, 1, 0, 0.866, 0, -0.5, ...]`); `moved_edit_is_kept`; `deleted_edit_is_kept`; `replaced_edit_is_kept`; `regeneration_keeps_edits` (generate, edit, generate from the same input, apply: edit present); `object_gone_makes_an_orphan`; `changed_model_makes_an_orphan`; `moved_far_makes_an_orphan`; `orphan_stays_exactly_where_it_was`; `a_deleted_edit_with_no_object_is_dropped_silently`; `overrides_file_round_trips`; `missing_overrides_file_is_empty`; `damaged_overrides_are_errors` (seeds). `roadplace.rs`: `a_bridge_is_a_straight_run_of_bridge_models`; `a_short_bridge_stays_road`; `the_ground_under_a_bridge_keeps_its_height`; `the_road_meets_the_bridge_at_deck_height`; `an_edit_keeps_its_object_when_a_neighbouring_stretch_changes`.
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.** Document `edits/overrides.toml` in the manual as a file the editor will write. In this milestone it is edited by hand.
- [ ] **Step 4: Run** `cargo test -p naves-core`, `make fuzz`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): bridges, stable object identity and overrides`.

### Task 10: Checks on the final terrain: drivable, walkable, on the ground

The profile is a plan; this measures what came out, on the same heightmap the game gets.

**Files:**
- Create: `crates/naves-core/src/checks.rs`
- Modify: `crates/naves-core/src/lib.rs`
- Test: `crates/naves-core/tests/checks.rs`

**Interfaces:**
- Consumes: `PlacedNetwork`, `Conformed`, `Feature`.
- Produces, in `naves_core::checks`:
  - `pub const DRIVE_GRADE_PAVED: f64; pub const DRIVE_GRADE_GRAVEL: f64; pub const DRIVE_GRADE_TRACK: f64; pub const WALK_LIMIT_DEG: f64;`, the values Task 1 measured, with a comment pointing to `docs/formats/road-objects.md`.
  - `pub const MAX_CROSS_FALL: f64 = 0.05; pub const MAX_MISS_M: f64 = 0.15;` (**[my call]**).
  - `pub enum DriveState { Fine, Steep, NotDrivable, WalkOnly, TooSteepToWalk }`: `Steep` is over `max_grade` but under the hard limit; the last two are for roads with `drivable: false`.
  - `pub struct PieceCheck { pub slot: usize, pub family: Family, pub start: (f64, f64), pub end: (f64, f64), pub width: f64, pub grade: f64, pub cross_fall: f64, pub grade_change: f64, pub miss_m: f64, pub state: DriveState }`.
  - `pub fn check_roads_on_terrain(net: &PlacedNetwork, hm: &Heightmap) -> Vec<PieceCheck>`.
  - `pub struct SteepPathSection { pub feature: String, pub from: (f64, f64), pub to: (f64, f64), pub length_m: f64, pub slope_deg: f64 }`.
  - `pub fn check_walkable(features: &[Feature], hm: &Heightmap) -> Vec<SteepPathSection>`.

**Roads.** Read the terrain every metre along the piece, on its centre line and both edges. `grade` is the steepest over a 12.5 m window along the road. `cross_fall` is the largest fall across it. `grade_change` is against the next piece. `miss_m` is the largest vertical distance between the piece's plane and the terrain inside its footprint, with **both diagonals** of each grid cell tried, as we do not know how the engine splits them: this is the direct test of a road hanging in the air or buried. `NotDrivable`: a road with `drivable: true` over its family's hard limit (paved for Asf and City, gravel for Grav, track for Mud), or a `cross_fall` above `MAX_CROSS_FALL`. A road with `drivable: false` is never `NotDrivable` and gets the walking check below instead. Bridges are skipped: nothing under them has to fit.

**Everything smaller** (paths, and tracks with `drivable: false`). Only one thing is checked, per the maintainer: too steep to walk. Read the ground every 2 m along the path. A section counts when the slope of the ground exceeds `WALK_LIMIT_DEG` over at least 6 m; sections less than 10 m apart merge into one. Scrambles under 6 m are walkable in spirit. A path that is reported stays as it is. Ways the table in Task 4 leaves without a trace (`alpine_hiking` and harder) are not checked at all.

- [ ] **Step 1: Write the failing tests.** `a_road_at_the_limit_is_drivable`; `a_road_over_the_limit_is_not_drivable`, for Asf3, Grav and a drivable Mud with their own limits; `a_grade4_track_is_never_not_drivable`; `a_grade4_track_over_the_walk_limit_is_too_steep_to_walk`; `steep_is_between_the_two_limits`; `a_short_steep_spot_within_the_window_counts`; `cross_fall_makes_a_road_not_drivable`; `a_bump_shows_as_grade_change`; `miss_is_zero_on_shaped_terrain` (the valley cases of Task 8 through `conform`: below `MAX_MISS_M` on every piece); `miss_finds_a_road_in_the_air` (an unshaped heightmap with a valley under the road); `miss_tries_both_diagonals`; `a_bridge_is_not_checked`; `a_steep_path_section_is_reported_with_its_length`; `a_short_scramble_is_not_reported`; `close_sections_merge`; `a_gentle_path_reports_nothing`; `batter_slopes_are_walkable` (`CUT_SLOPE` and `FILL_SLOPE` below `WALK_LIMIT_DEG`); `checks_are_deterministic`.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p naves-core --test checks`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p naves-core`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): check that roads are drivable and paths walkable`.

### Task 11: Generation in the build, paths on the ground

**Files:**
- Create: `crates/naves-export/src/generate.rs`, `crates/naves-core/src/validate.rs`
- Modify: `crates/naves-core/src/surface.rs`, `crates/naves-core/src/lib.rs`, `crates/naves-export/src/build.rs`, `crates/naves-export/src/config.rs`, `crates/naves-export/src/bridge.rs` if the sandbox needs the models (Task 1), `crates/naves-export/src/lib.rs`, `docs/manual/src/getting-started.md`, `docs/manual/src/navmesh.md`, `docs/manual/src/troubleshooting.md`, `CHANGELOG.md`
- Test: `crates/naves-export/tests/build.rs`, `crates/naves-export/tests/config.rs`, `crates/naves-core/tests/surface.rs`, `crates/naves-core/tests/validate.rs`

**Interfaces:**
- Consumes: everything above.
- Produces:
  - In `naves_core::surface`: `SurfaceGrid::stroke(&mut self, line: &[(f64, f64)], width_m: f64, class: SurfaceClass)`.
  - In `naves_core::validate`: `pub enum Rule { RoadNotDrivable, RoadMissesTerrain, PathTooSteep, RoadGrade, RoadGap, RoadInWater, RoadConflict, EarthworksCapped, OrphanedEdit, JoinWithoutJunction, TunnelOmitted }` with `id(self) -> &'static str`; `pub struct Violation { pub rule: Rule, pub at: (f64, f64), pub message: String }`; `pub fn check_roads(net: &PlacedNetwork, profiles: &Profiles, conformed: &Conformed, pieces: &[PieceCheck], steep_paths: &[SteepPathSection], orphans: &[OrphanedEdit], water: &[LandPolygon], frame: &MapFrame) -> Vec<Violation>`, sorted by rule, then position. Neighbouring `NotDrivable` pieces make one violation, with the section's length and steepest grade.
  - In `naves_export::generate`: `pub struct RoadStats { pub km_by_family: BTreeMap<String, f64>, pub stretches: usize, pub closed: usize, pub tolerated: usize, pub open: usize, pub median_deviation_m: f64, pub max_grade: f64, pub max_cut_m: f32, pub max_fill_m: f32, pub pieces: usize, pub not_drivable_m: f64, pub largest_miss_m: f64, pub paths_too_steep_m: f64 }`; `pub struct Generated { pub base: Heightmap, pub heightmap: Heightmap, pub surfaces: SurfaceGrid, pub objects: Vec<PlacedObject>, pub features: Vec<Feature>, pub pieces: Vec<PieceCheck>, pub violations: Vec<Violation>, pub stats: RoadStats }`; `pub fn generate(project_dir: &Path, project: &Project, tools: &Tools, progress: &dyn Fn(&str)) -> Result<Generated, BuildError>`.
  - In `naves_export`: `stage_world` takes `objects: &[WorldObject]`; `world_config` gets `"DZ_Structures_Roads"` in `requiredAddons` when the world has road objects. If Task 1 found that binarize needs the models, `BinarizeBridge::binarize_world` gets `extra_addons: &[PathBuf]`, the PBOs it unpacks into the sandbox's `P:`.

Order in `generate`: DEM → base heightmap → OSM ways → features → graph → catalog → placing → profiles → conforming → objects and overrides → surfaces on the conformed terrain → strokes → checks → validation. Ground under a road: a Soil stroke of road width + 2 m, so that grass does not grow through it. Paths: a Soil stroke of their width (`FeatureKind::Path`). `road_in_water` takes OSM's `natural=water` polygons (priority 3): any non-bridge road piece with its origin inside one is a violation (spec 5.7).

The file order of objects in the 8WVR is sorted by `ObjectId`; the object's `id` field counts up from 1 in that order. Violations go into `BuildOutput.warnings` in one line each, and in full into `build/report.json`.

- [ ] **Step 1: Write the failing tests.**
  - `surface.rs`: `a_stroke_paints_its_width` (a known cell inside, one outside, at a diagonal); `a_stroke_outside_the_map_is_clipped`; `an_empty_line_does_nothing`.
  - `validate.rs`: a test per rule; `a_section_of_undrivable_pieces_is_one_violation`; `road_in_water_ignores_bridges`; `violations_are_sorted`.
  - `build.rs` (fake tools, synthetic catalog behind the fake armake2): `game_models_are_not_in_the_mod` (the fake binarize's sandbox holds a `dz\\...` file, and nothing below `dz` is in the packed folder); `required_addons_follow_the_objects` (with and without road objects, `tests/config.rs`); `a_project_without_osm_builds_as_before` (surfaces and elevation byte equal to the land-cover result, no objects, `structures_roads` never read); `osm_without_roads_builds_with_no_objects` (the koenigssee fixture); `roads_reach_the_world_file` (read the 8WVR back: pieces at their positions with `transform[4] == 1`); `the_terrain_under_a_road_is_conformed`; `paths_are_in_the_mask_and_have_no_objects` (a `path` way gives the blue mask colour and no object); `two_builds_are_byte_identical`; `the_report_has_the_violations`; `a_stale_navmesh_is_warned_about` (a project with `navmesh/<world>.nm` gets a warning that the terrain changed and the mesh should be regenerated, whenever the world has road objects).
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.** `build` calls `generate`, and everything else in it stays. Read sources before `layout.clean()`, as plan 2 does. A bad OSM file costs nothing.
- [ ] **Step 4: Run** `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5: Commit** `feat: roads and paths in the build`. The manual says what is generated, what is not (tunnels, junction angles), and that the navigation mesh has to be regenerated after a terrain change.

### Task 12: The editor's shell, with switchable layers

Start with the `/frontend-design:frontend-design` skill (`AGENTS.md`, *User interface*), then read `docs/superpowers/specs/2026-11-06-naves-editor-ui.md`: this task builds its first part (its section 7). **Build what the final layout needs to be in its place: levels, the stack, the left panel by level, the shortcut table, the bottom dock.** The tool rail and the item panel come with milestone 2, when there is something to place.

Layers of this plan, in the design's stack:

| Level | Layer | Id (MCP) | Drawn as | Colours |
|---|---|---|---|---|
| 4 Roads and paths | Roads | `roads` | true width, min. 1.5 px, with a 1 px white casing so it reads on any base | ink `1d2924` |
| 4 Roads and paths | Paths | `paths` | dashed, min. 1 px | brown `8c6a45` |
| 4 Roads and paths | Drivable | `drivable` | over the road's line. Not drivable is twice as wide with cross ticks | fine `2e6a4c`, steep `d08a1e`, not drivable `b03a2e`, walk only `5a6660` |
| 1 Terrain | Too steep to walk | `too_steep_to_walk` | diagonal hatching, ink at 55 %, 6 px apart in screen space, over ground above `WALK_LIMIT_DEG` | ink |
| 1 Terrain | Terrain changes | `terrain_changes` | tint by `new − base`, full (50 %) at ±5 m, none below 0.25 m | cut `8c6a45`, fill `3f6f9e` |
| above the stack | Problems | `problems` | diamond with a white edge, constant size at any zoom | error `b03a2e` |

Switching a layer on unfolds its key under its row, with the symbol as drawn on the map, from the same constants. Base maps above the stack: Relief, Slope, Height, Ground ("Hillshade" becomes "Relief" in the GUI; MCP keeps `hillshade`). Defaults: Relief, level 0 active, Roads and Problems on. Level state is per session: toggling or switching must not change `naves.toml`.

Left panel by level: 0 the project settings as today; 1 largest cut and fill, source files; 2 land-cover and OSM files; 4 `RoadStats` as a table. Clicking with Select (Q, the only tool so far, so no rail) picks a road piece in the active level, Alt+click in any: the properties under the stack show model, family, grade, cross fall, miss and where it came from, read-only (spec 5.9, "inspect objects and features"). A 3 px band in the level's colour on top of the map, and a status line at its bottom.

Copy: a tooltip says what a row means, with the number from the constant: "Ground steeper than 52°. Players cannot walk up it." Rows that have nothing to draw say how to fix that, in the row: "Add an .osm.pbf file to sources.osm to see roads." If tools are missing: "Roads need armake2: <the error>". While generating: a spinner and "Placing roads", and the road layers are hidden because the terrain under them has changed. Problems say what is wrong and where: "Not drivable: 22 % over 80 m".

Shortcuts of this plan, a subset of the design's table with the same keys: 0, 1, 2, 4 go to a level; H and Shift+H; Q and Esc; F and Shift+F; Ctrl+B; F8 and Shift+F8; F1. Task 13 makes them changeable and adds F2.

**Files:**
- Create: `crates/naves/src/gui/levels.rs`, `crates/naves/src/gui/layers.rs`, `crates/naves/src/gui/shortcuts.rs`, `docs/manual/src/shortcuts.md`, `docs/manual/src/images/map-layers.png`
- Modify: `crates/naves-core/src/render.rs`, `crates/naves/src/mcp.rs`, `crates/naves/src/gui/mod.rs`, `crates/naves/src/gui/worker.rs`, `crates/naves/src/gui/tests.rs`, `docs/manual/src/SUMMARY.md`, `docs/manual/src/gui.md`, `docs/manual/src/mcp.md`, `CHANGELOG.md`
- Test: `crates/naves-core/tests/render.rs`, `crates/naves/tests/mcp.rs`, `crates/naves/src/gui/tests.rs`

**Interfaces:**
- Consumes: `generate`, `Generated`, `PieceCheck`, `Violation`.
- Produces, in `naves_core::render`:
  - `pub enum MapLayer { Roads, Paths, Drivable, TooSteepToWalk, TerrainChanges, Problems }`, with `pub const ALL` (the drawing order, bottom first: terrain changes, too steep to walk, paths, roads, drivable, problems) and `pub fn id(self) -> &'static str`.
  - `pub struct LayerData<'a> { pub base: &'a Heightmap, pub heightmap: &'a Heightmap, pub pieces: &'a [PieceCheck], pub features: &'a [Feature], pub violations: &'a [Violation] }`.
  - `pub fn render_layer(layer: MapLayer, data: &LayerData<'_>, req: &RenderRequest) -> Result<RgbaImage, RenderError>`: transparent everywhere it draws nothing.
  - `pub fn composite(under: &mut RgbaImage, over: &RgbaImage)`.
  - `pub fn fade(image: &mut RgbaImage, factor: f32)`, for the levels that are not active. Lines as coverage from the distance to each segment, no new dependency.
- Produces, in `naves::gui`:
  - `levels::Level` with all ten of the design: `Project`, `Terrain`, `Ground`, `Water`, `Roads`, `Vegetation`, `Buildings`, `Objects`, `Life`, `Economy`; `digit(self) -> u8`, `name(self) -> &'static str`, `colour(self) -> Color32`, `layers(self) -> &'static [MapLayer]`, `is_built(self) -> bool`.
  - `layers::LayerPanel { pub base: Overlay, pub active: Level, pub on: BTreeSet<MapLayer>, pub hidden: BTreeSet<Level>, pub show_others_in_full: bool }`, with `Default`; `pub fn available(layer: MapLayer, state: &GenerationState) -> Result<(), String>`, the text in the row when it is not.
  - `shortcuts::Shortcut { key: Key, modifiers: Modifiers, action: Action, label: &'static str, scope: Scope }`; `SHORTCUTS: &[Shortcut]`, the one table of defaults; `pub fn action_for(input: &egui::InputState, level: Level, typing: bool) -> Option<Action>`.
- MCP (spec 5.10: read and see): `render_topdown` gets an optional `layers` list of the ids above, so the AI sees what the user sees, composited by the same code; without it, no generation runs and the result is as today. `roads_stats` returns `RoadStats`. `validation_list` returns violations with rule, position and message, with an optional `rule` filter.
- GUI: each active layer is its own texture, keyed by `(RenderRequest, generation)`, painted in `ALL` order, so that **switching a layer or a level renders nothing**. Generation runs in a job like `HeightmapJob`, once per input change. The bottom dock has the tabs Build and Problems.

- [ ] **Step 1: Write the failing tests.**
  - `render.rs`: `each_layer_is_transparent_where_it_draws_nothing`; `a_road_is_drawn_at_its_width` (a known pixel on it, one off it, at two zooms); `not_drivable_is_wider_than_fine`; `walk_only_is_not_coloured_as_a_problem`; `hatching_marks_only_ground_above_the_limit`; `terrain_changes_tint_cut_and_fill_differently`, and none on unchanged ground; `a_problem_marker_has_the_same_size_at_any_zoom`; `composite_keeps_what_is_below_transparent_pixels`; `fade_changes_only_alpha`; `drawing_order_is_stable`.
  - MCP: `render_topdown_without_layers_does_not_generate`, `render_topdown_with_layers_matches_composited_layers`; `an_unknown_layer_is_a_tool_error`; `roads_stats_counts_the_network`, `validation_list_filters_by_rule`; `a_project_without_osm_has_empty_stats` (not an error).
  - GUI, through the accessibility tree:
    - Levels: `only_built_levels_are_shown`, and in the stack's order, bottom up; `digits_never_change` (the table for all ten is fixed; a test pins it to the design); `a_digit_makes_a_level_active`; `a_level_that_is_not_built_ignores_its_digit`; `making_a_hidden_level_active_shows_it`; `the_left_panel_follows_the_level`; `level_0_keeps_the_settings_working`.
    - Shortcuts: `no_key_is_assigned_twice_in_a_scope` (over the whole table, and no level's key repeats a global one); `no_shortcut_needs_shift_or_altgr_on_qwertz` (not `[`, `]`, `/`, `?`, and no `Y` or `Z` as a tool); `typing_in_a_field_fires_no_shortcut` (typing "4" in the Name field keeps the level and changes the name); `f1_shows_every_shortcut_of_the_level`; `the_manual_lists_every_shortcut` (reads `docs/manual/src/shortcuts.md`); `tooltips_show_the_key`.
    - Layers: `base_maps_are_exclusive`; `layers_are_checkboxes_with_their_names` (each exposes its name and state, and Space toggles it); `defaults_are_level_0_relief_roads_and_problems`; `switching_renders_nothing` (the render counter stays put, for a layer and for a level); `switching_does_not_touch_the_project_file`; `key_entries_show_only_when_the_layer_is_on`; `without_osm_the_row_says_what_to_add`; `missing_tools_are_named_in_the_row`; `road_layers_hide_while_generating`.
    - Selection and problems: `a_click_selects_only_in_the_active_level`; `alt_click_switches_to_the_level_it_hit`; `the_properties_show_where_a_piece_came_from`; `esc_clears_the_selection`; `f8_goes_to_the_next_problem_and_centres_the_map`; `the_problem_count_matches_the_list`.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p naves -p naves-core`. Expected: FAIL.
- [ ] **Step 3: Implement.** `gui/mod.rs` stays thin, as its header says: logic goes in `levels.rs`, `layers.rs` and `shortcuts.rs`.
- [ ] **Step 4: Run** `cargo test -p naves -p naves-core`. Expected: PASS.
- [ ] **Step 5: Look at it.** Run `naves gui` on the example in a headless sway with `grim`: each layer alone and all on, over each base map, each level active. Fix what does not read: roads on Slope, hatching on Ground, a problem marker on a road, the column at the smallest window size. Save a view as `docs/manual/src/images/map-layers.png`. If an element says nothing, remove it.
- [ ] **Step 6: Commit** `feat: the editor's levels, switchable map layers and shortcuts`.

### Task 13: Settings: keyboard, map and external tools

Start with the `/frontend-design:frontend-design` skill, then read section 8 of `docs/superpowers/specs/2026-11-06-naves-editor-ui.md`, which fixes the window, the Keyboard tab and the file.

**Files:**
- Create: `crates/naves-core/src/prefs.rs`, `crates/naves/src/gui/prefs_window.rs`, `fuzz/fuzz_targets/prefs_parse.rs`, `fuzz/corpus/prefs_parse/*`, `docs/manual/src/settings.md`
- Modify: `crates/naves-core/src/lib.rs`, `crates/naves-export/src/tools.rs`, `crates/naves/src/cli.rs`, `crates/naves/src/mcp.rs` (they call `Tools::locate`), `crates/naves/src/gui/mod.rs`, `crates/naves/src/gui/shortcuts.rs`, `crates/naves/src/gui/layers.rs`, `crates/naves/src/gui/map_view.rs`, `fuzz/Cargo.toml`, `docs/manual/src/SUMMARY.md`, `docs/manual/src/shortcuts.md`, `docs/manual/src/installation.md`, `docs/manual/src/troubleshooting.md`, `CHANGELOG.md`
- Test: `crates/naves-core/tests/prefs.rs`, `crates/naves-core/tests/fuzz_seeds.rs`, `crates/naves-export/tests/` (tool lookup), `crates/naves/src/gui/tests.rs`, `crates/naves/tests/cli.rs`

The code calls them `Preferences`, because `gui/settings.rs` is already the project's form. The window says "Settings".

**Interfaces:**
- Consumes: `SHORTCUTS`, `Action`, `action_for`, `LayerPanel` (Task 12); `Tools::locate_with`.
- Produces, in `naves_core::prefs` (no egui, so that it can be fuzzed; the crate `naves` has no library):
  - `pub struct Preferences { pub shortcuts: BTreeMap<String, String>, pub map: MapPrefs, pub tools: ToolPaths }`: `shortcuts` maps an action id to a key such as `"Ctrl+B"`, or `""` for no key, only where it differs from the default.
  - `pub struct MapPrefs { pub invert_zoom: bool, pub fade_percent: u8, pub interface_scale: f32, pub start_base: String, pub start_layers: Vec<String> }`; `pub struct ToolPaths { pub armake2: Option<PathBuf>, pub steam_root: Option<PathBuf>, pub dayz_tools: Option<PathBuf>, pub dayz: Option<PathBuf>, pub proton: Option<PathBuf> }`.
  - `pub fn default_path(config_home: &Path) -> PathBuf`; `Preferences::load(path: &Path) -> Result<Preferences, PrefsError>`: a missing file gives the defaults; `Preferences::parse(text: &str)`; `save(&self, path: &Path)` atomically, and keeping the file's unknown keys.
- Produces, in `naves_export::tools`: `pub fn locate_preferring(paths: &ToolPaths) -> Result<Tools, ToolError>`, `Tools::locate` with a settings layer in the `env` closure: environment, then settings, then the search. Also `pub fn tool_origins(paths: &ToolPaths) -> Vec<ToolOrigin>`, for the window: tool, path in use and "Environment", "Settings" or "Found".
- Changes in `naves::gui::shortcuts`: `Action::id(self) -> &'static str`, stable forever, pinned by a test; `pub struct Bindings`, built from `SHORTCUTS` and the preferences; `Bindings::key_for(Action)`, `Bindings::conflict(new: Binding, action: Action) -> Option<Action>`, which knows the scopes; `action_for(input, bindings: &Bindings, level, typing)`. The tooltips and the F1 sheet read `Bindings`.
- Produces, in `naves::gui::prefs_window`: `pub struct PrefsWindow`, a modal with the tabs Keyboard, Map, External tools. A capture state that stops `action_for` while the window is open. `NavesApp::open` gets the preferences' path as a parameter, so that tests never touch the user's config (`AGENTS.md`, *Tests*).

- [ ] **Step 1: Write the failing tests.**
  - `prefs.rs`: `a_missing_file_gives_the_defaults`; `values_round_trip`; `unknown_keys_survive_a_save`; `a_damaged_file_is_an_error_and_is_not_overwritten`; `saving_is_atomic`; `out_of_range_values_are_clamped` (fade above 100, a scale of 0 or NaN). Seeds for `prefs_parse`, also run from `fuzz_seeds.rs`.
  - Tool lookup: `the_environment_beats_the_settings`; `the_settings_beat_the_search`; `a_path_in_the_settings_that_does_not_exist_names_the_setting`; `origins_say_where_each_path_came_from`.
  - CLI: `naves_build_uses_the_settings` (`XDG_CONFIG_HOME` in a temp dir, a fake armake2 named only there).
  - GUI, through the accessibility tree:
    - `the_button_and_f2_open_the_window`; `esc_closes_it`; `shortcuts_do_not_fire_while_it_is_open`; `no_setting_changes_the_project_file`.
    - `pressing_a_key_rebinds_and_saves`; `the_new_key_works_and_the_old_one_does_not`; `esc_cancels_a_capture`; `esc_enter_and_f1_cannot_be_changed`; `a_used_key_asks_before_replacing`; `cancel_changes_nothing`; `replace_leaves_the_other_action_with_no_key`; `the_same_key_in_two_levels_is_no_conflict`; `a_global_key_conflicts_with_every_level`; `reset_restores_one_and_reset_all_restores_all`; `search_filters_the_rows`; `tooltips_and_f1_show_the_new_key`; `action_ids_are_pinned`; `a_new_default_reaches_a_file_that_does_not_mention_it`.
    - `invert_zoom_inverts_the_wheel` (on `MapView`, as a plain-number test); `fade_percent_changes_the_other_levels`; `start_layers_are_used_when_a_project_opens`; `a_damaged_file_shows_its_error_in_the_window`.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p naves -p naves-core -p naves-export`. Expected: FAIL.
- [ ] **Step 3: Implement.** The row "Roads need armake2: <the error>" of Task 12 gets a button "Open settings", which opens External tools. `docs/manual/src/shortcuts.md` keeps listing the defaults and links to `settings.md`.
- [ ] **Step 4: Run** `cargo test -p naves -p naves-core -p naves-export`, `make fuzz`. Expected: PASS.
- [ ] **Step 5: Look at it.** Run `naves gui` in a headless sway with `grim`, with a rebound key, a conflict showing and a damaged file. Fix what does not read.
- [ ] **Step 6: Commit** `feat: settings for shortcuts, the map and external tools`.

### Task 14: Driving through Berchtesgaden

**Files:**
- Modify: `examples/berchtesgaden/README.md`, `docs/research/2026-11-06-road-pieces.md`, `docs/manual/src/getting-started.md`, `CHANGELOG.md`

- [ ] **Step 1: Build the example** with the sources from plan 2's Task 8. Check `render_topdown` with the `roads` layer against OSM by eye for the whole map, and read `validation_list`. Expected: the B305 and the road along the Königssee recognisable, no road in the lake without a violation, no road off the map.
- [ ] **Step 2: Check the numbers,** from `roads_stats`. Targets: at least 90 % of closed stretches within 0.5 m and 99 % within 3 m; median deviation from OSM below 6 m; no `road_in_water` without a real cause; `largest_miss_m` below `MAX_MISS_M`; `not_drivable_m` zero, or each section explained; no track with `drivable: false` and no path too steep to walk without a look at why. Look at the example with the `drivable`, `too_steep_to_walk` and `terrain_changes` layers on: are the earthworks where you would expect them, and no wider? Compare a grid of 2048 with 1024 on the worst valley side and write down what the bench width and the build time do.
- [ ] **Step 3: Choose the earthwork numbers.** Nobody knows the right values for `CUT_SLOPE`, `FILL_SLOPE` and `MAX_EARTHWORK_M` yet, so they are chosen by looking. Build the example with three settings: 45° and 34° (the start), 35° and 27° (gentler, wider), and reach 60 and 120 m. On the three worst valley sides, compare screenshots from across the valley, the width of `terrain_changes`, `capped` entries and whether a player can walk off the road on either side. Show the maintainer the comparison, and take the values they choose. Write them and the reason in the research note. The test `batter_slopes_are_walkable` holds whatever is chosen. Write the numbers in the README next to the build's time, peak memory and PBO size. If a number misses, fix the cause or write down why, and tell the maintainer: don't move the target quietly.
- [ ] **Step 4: Regenerate the navigation mesh** (`docs/manual/src/navmesh.md`), because the terrain has changed.
- [ ] **Step 5: Drive it.** Test server with the patched mission (`AGENTS.md`, *Comparable screenshots*), a vehicle spawned in the mission's `init.c` on the road; document admin spawning in the manual (spec section 8: "a few vehicle spawns (or documented admin spawning)"). Client, with dayz-mcp (see Task 1).
  - **Without steering, by the agent:** `scene_raycast` along every road for the height of road and ground; compare with `miss_m` from the checks. Screenshots from fixed spots (`camera_set`, `capture_screenshot`; dayz-mcp's README says what capture needs): a straight on a slope, a hairpin, a junction, a bridge, a village street, a mountain path, **and the three valley sides with the largest cut and fill in `roads_stats`, from the road and from across the valley.** `vehicle_control` at full throttle from a standstill on the steepest straight of a paved road, a gravel road and a drivable track. `player_move` up a path section the checks call fine and one they call too steep: it gets up the first and fails on the second.
  - **By the maintainer:** drive the main road and a gravel road end to end, with `vehicle_trace` recording. Expected: no bumps that throw the car off, and no point where the car has to stop.
  - **Expected overall:** player connects, no crash, roads visible and on the ground along their whole length, valley sides included, no gaps you could fall in. Note every bump from the trace with its position, and compare it with the checks there.
- [ ] **Step 6: Tune what the drive shows,** with a test first if it is a bug: grade limits, the measured limits if the real map disagrees with the ramps, the bridge choice of Task 9, stroke widths. Record the before and after in the research note.
- [ ] **Step 7: Run** `TMPDIR=/var/tmp make check`. Expected: PASS. **Commit** `docs: driving through Berchtesgaden`. Push; CI, CodeQL and Codecov green. Remove the mod symlink.

---

## After this plan

Plan 1's remaining order stands, and each adds its level to the interface (`2026-11-06-naves-editor-ui.md`, section 7): houses (which take over the object model, and add the undo history and the 3D view), water, railways and bridges, scaling and thinning, satellite imagery. Plan 2 proposed vegetation objects as a next step too: Tasks 9 and 11 are all of the object plumbing it needs, so it can come at any point from here on. The cached layer graph (spec 5.3) comes when a real project shows that generation is too slow to iterate on; Task 14's measurements say whether that is already the case.

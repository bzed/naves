<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Náves M1, plan 2: ground surfaces from land cover — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `naves build` paints the ground from real data: ESA WorldCover as the complete base, OSM on top for water, rock and built-up areas, and today's slope and altitude rules wherever neither source says anything.

**Architecture:** `naves-core` turns the sources into one `SurfaceGrid` (a class per map cell); `naves-export` only draws that grid into masks, satellite images and tile materials. Sources are local files listed in `naves.toml`, like the DEM. Náves itself does not download anything.

**Tech Stack:** Rust 1.95, `geotiff-reader` (`read_band_window`), `osmpbf` (after review, Task 4), polygon rasterizing (crate or own, Task 4), existing `proj4rs` projection.

**Spec:** `docs/superpowers/specs/2026-11-04-naves-design.md`, sections 5.1, 5.2 (surface layers), 5.3 (stages 1, 3, 8) and 9. Evidence: `docs/research/2026-11-05-reference-terrains.md` (land-cover sources, mask colours, PAA fixes), `docs/formats/terrain-layers.md`.

This plan moves the land-use half of "Later plans" item 7 of plan 1 forward, at the maintainer's request. Not in this plan: trees and other vegetation objects (spec 5.3 stage 6), water planes and lake beds (the water plan; here a lake is only soil-coloured ground), real satellite imagery, downloading sources from Náves, painting surfaces by hand.

## Global Constraints

- **`AGENTS.md` applies to every task**: tests first, coverage at or above 85 %, strict lints, `make check` before every commit, user docs and the `CHANGELOG.md` line in the same commit. Where this plan and `AGENTS.md` disagree, `AGENTS.md` wins.
- **Same input, same output:** no `HashMap`/`HashSet` iteration in output; OSM elements are processed in id order.
- Every parser of untrusted input gets a fuzz target whose seeds also run in `make test` (Tasks 3 and 4). No network in tests.
- Large data goes under `/var/tmp` on the development machine: `/tmp` is a tmpfs. Run `make check` with `TMPDIR=/var/tmp`.
- Every Wine or Proton program runs in a headless sway session. One game at a time; remove the DayZ install's mod symlink afterwards.
- Spec section 9, verbatim: "Generated maps contain OSM-derived data: ODbL attribution (and share-alike for the derived database) must be written into the mod; same for Copernicus and imagery sources." Formats come from public documentation and from observing files, with "no disassembly of BI binaries".
- Layer slots verified so far are 0 to 3 only (black, red, green, blue at alpha 255). This plan stays within them.
- Map coordinates: x east, y north, metres, origin south-west; rows south first.

## Review Focus

1. **An area across two WorldCover tiles** (3° boundary): classes are continuous across the seam, as for the DEM. Test in Task 3.
2. **OSM extracts cut polygons at their border:** an unclosed ring, or a way whose nodes are missing, is skipped and counted as a warning, never a panic and never a polygon closed across the map. Test in Task 4.
3. **Sources that do not cover the area:** the uncovered part falls back to the slope and altitude rules and the build reports a warning with the share. It does not fail and does not paint it one class. Test in Task 5.
4. **Very large inputs:** a 36000 × 36000 WorldCover tile or a country-sized `.osm.pbf` must not be loaded whole. Only the window of the map is read, and only nodes of relevant ways are kept. Tests in Tasks 3 and 4.
5. **A `naves.toml` from before this plan** (no `land_cover`, no `osm`) loads, and it builds the same surfaces as before. Test in Task 1 and Task 2.

---

### Task 1: Project sources

**Files:**
- Modify: `crates/naves-core/src/project.rs`, `docs/manual/src/project-file.md`, `CHANGELOG.md`
- Test: `crates/naves-core/tests/project.rs`, seeds in `fuzz/corpus/project_parse/`

**Interfaces:**
- Produces: `Sources { dem: Vec<PathBuf>, land_cover: Vec<PathBuf>, osm: Vec<PathBuf> }`; the two new fields use `#[serde(default, skip_serializing_if = "Vec::is_empty")]`.

- [ ] **Step 1: Write the failing tests**

`old_project_files_still_load` (the seed `roundtrip` loads, both lists are empty, and saving writes neither key); `land_cover_and_osm_paths_are_kept` (parse, save, load round trip); the new lists get the same path validation as `dem`, tested with a rooted path if `dem` rejects those.

- [ ] **Step 2: Run them to see them fail** — `cargo test -p naves-core --test project`. Expected: FAIL (unknown field or missing field).
- [ ] **Step 3: Add the fields and document them** in the `[sources]` table of the manual, with the fetch commands (see Task 8 for the URLs).
- [ ] **Step 4: Run** `cargo test -p naves-core`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): land_cover and osm sources in the project file`.

### Task 2: `SurfaceGrid` and four ground layers

Separates deciding from drawing. Without sources the result must match today's map, apart from the two new layers being available.

**Files:**
- Create: `crates/naves-core/src/surface.rs`
- Modify: `crates/naves-core/src/lib.rs`, `crates/naves-export/src/layers.rs`, `crates/naves-export/src/rvmat.rs`, `crates/naves-export/src/build.rs`, `docs/formats/terrain-layers.md`
- Test: `crates/naves-core/tests/surface.rs`, `crates/naves-export/tests/layers.rs`

**Interfaces:**
- Produces, in `naves_core::surface`:
  - `pub enum SurfaceClass { Grass, Rock, Forest, Soil }` with `pub fn slot(self) -> usize` returning 0, 1, 2, 3.
  - `pub struct SurfaceGrid { pub size: u32, pub cell_m: f64, /* south row first */ }` with `pub fn at(&self, x: f64, y: f64) -> SurfaceClass` (nearest cell, clamped to the map) and `pub fn from_fn(size: u32, cell_m: f64, f: impl Fn(u32, u32) -> SurfaceClass + Sync) -> SurfaceGrid`.
  - `pub fn surface_grid_size(terrain_size: u32) -> u32`: `min(terrain_size * 4, 8192)`.
  - `pub fn grass_limit_m(height_scale: f64) -> f32` (moved from `layers`, value unchanged) and `pub fn procedural_class(sample: &Sample, grass_limit: f32) -> SurfaceClass`.
  - `pub fn procedural_grid(hm: &Heightmap, grass_limit: f32) -> SurfaceGrid`.
- Produces, in `naves_export::layers`: `build_layers(hm: &Heightmap, land_size: u32, prefix: &str, surfaces: &SurfaceGrid) -> LayerSet`. `classify` is removed. `SURFACES` is indexed by `SurfaceClass::slot()`.

| Slot | Class | Mask (RGBA) | Textures below `dz\surfaces_bliss\data\terrain\` | Satellite colour |
|---|---|---|---|---|
| 0 | Grass | 0, 0, 0, 255 | `en_grass1` | `4a 7a 3a` |
| 1 | Rock | 255, 0, 0, 255 | `en_stones` | `8a 87 80` |
| 2 | Forest | 0, 255, 0, 255 | `en_forest_con` | `2e 4d 2a` |
| 3 | Soil | 0, 0, 255, 255 | `en_soil` | `7a 68 4e` |

- [ ] **Step 1: Write the failing tests**

`surface.rs`: `grid_size_is_capped` (1024 → 4096, 2048 → 8192, 8192 → 8192); `at_clamps_to_the_map`; `procedural_rule_is_unchanged` (flat below the limit is Grass, flat above is Rock, a 60° wall is Rock). `layers.rs`: move the current mask tests to `procedural_grid`, and add `every_class_has_its_mask_colour` (a `from_fn` grid with one class per tile quadrant, asserting the four colours in the table) and `rvmat_lists_the_four_layers` (the `_nopx` and `_ca` paths in stages 3 to 10; stages 11 to 14 stay empty).

- [ ] **Step 2: Run them to see them fail.** Expected: compile errors for the new API.
- [ ] **Step 3: Implement.** Lower the baked shading in `render_tile` from `0.35 + 0.65 * hillshade` to `0.7 + 0.3 * hillshade`: the game lights the terrain itself, and the baked shadows made rock faces nearly black. Test it with `baked_shading_stays_light`: no satellite pixel of a steep shaded wall is darker than 70 % of its class colour.
- [ ] **Step 4: Check the game.** Build the example, join, screenshot from the fixed test view (x 3962, z 1400, yaw 270). Expected: no crash, grass and rock as before, lighter rock faces, no tile seams. Check the mask slots 2 and 3 with a temporary test build in which a whole tile is forest and another soil: the right textures appear near the player. Slots 2 and 3 are read from Bohemia's sample but have not been seen in the game yet.
- [ ] **Step 5: Run** `cargo test --workspace`. Expected: PASS.
- [ ] **Step 6: Commit** `refactor: SurfaceGrid decides the ground, layers only draw it` (with the shading line in `CHANGELOG.md`).

### Task 3: WorldCover reader

**Files:**
- Create: `crates/naves-core/src/landcover.rs`, `fuzz/fuzz_targets/landcover_tile.rs`, `fuzz/corpus/landcover_tile/*`, `tests/fixtures/landcover/*` (crops made with `gdal_translate`; attribution in `tests/fixtures/README.md`)
- Modify: `crates/naves-core/src/lib.rs`, `fuzz/Cargo.toml`
- Test: `crates/naves-core/tests/landcover.rs`, `crates/naves-core/tests/fuzz_seeds.rs`

**Interfaces:**
- Produces, in `naves_core::landcover`:
  - `pub struct LandCover`; `pub fn open(paths: &[PathBuf], window: GeoWindow) -> Result<LandCover, LandCoverError>` reads only the pixels inside `window` with `read_band_window::<u8>`; a file that does not touch the window is skipped.
  - `pub struct GeoWindow { pub south: f64, pub west: f64, pub north: f64, pub east: f64 }`.
  - `pub fn class_code(&self, lat: f64, lon: f64) -> Option<u8>`: nearest pixel, `None` outside every file and for code 0.
  - `pub fn pixel_count(&self) -> usize`: pixels held in memory.
  - `pub fn surface_of(code: u8) -> Option<SurfaceClass>`.

| WorldCover code | Meaning | `SurfaceClass` |
|---|---|---|
| 10, 95 | tree cover, mangroves | Forest |
| 20, 30, 90 | shrubland, grassland, wetland | Grass |
| 40, 50, 80 | cropland, built-up, water | Soil |
| 60, 70, 100 | bare, snow, moss and lichen | Rock (no snow texture exists among the vanilla surfaces) |
| anything else | | `None` |

- [ ] **Step 1: Write the failing tests**

`classes_of_the_crop_match_gdal` (values read with `gdallocationinfo` for three positions of a committed crop: tree, grass, water); `seam_between_two_tiles_is_continuous` (two synthetic adjacent tiles, positions on each side and on the line); `only_the_window_is_read` (a synthetic 4000 × 4000 tile, a window of 10 × 10 pixels: `pixel_count()` is 100 or the window rounded up by at most one pixel per side); `outside_and_no_data_are_none`; `wrong_crs_is_an_error` (not EPSG:4326, naming the file); `surface_table` (every row above). Fuzz seeds: a valid crop and two damaged ones, run in `fuzz_seeds.rs`.

- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.** Guard the window size before allocating (`AGENTS.md`: no allocation from file-derived sizes without a limit): reject a window above 16 000 × 16 000 pixels with an error naming the file.
- [ ] **Step 4: Run** `cargo test -p naves-core` and `make fuzz`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): read ESA WorldCover land cover`, and add the licence text to `docs/manual/src/credits.md`.

### Task 4: OSM land polygons

**Files:**
- Create: `crates/naves-core/src/osm.rs`, `fuzz/fuzz_targets/osm_read.rs`, `fuzz/corpus/osm_read/*`, `tests/fixtures/osm/koenigssee.osm.pbf` (a crop made once with `osmium extract`; attribution in `tests/fixtures/README.md`)
- Modify: `crates/naves-core/Cargo.toml`, `crates/naves-core/src/lib.rs`, `deny.toml` if a licence needs it
- Test: `crates/naves-core/tests/osm.rs`

**Interfaces:**
- Produces, in `naves_core::osm`:
  - `pub struct LandPolygon { pub class: SurfaceClass, pub priority: u8, pub outer: Vec<(f64, f64)>, pub holes: Vec<Vec<(f64, f64)>> }`, coordinates as (lat, lon).
  - `pub struct OsmLand { pub polygons: Vec<LandPolygon>, pub skipped_rings: usize }`.
  - `pub fn read_land(paths: &[PathBuf], window: GeoWindow) -> Result<OsmLand, OsmError>`.
  - `pub struct RawWay { pub nodes: Vec<i64>, pub inner: bool }` (`inner` is the relation member role).
  - `pub struct Rings { pub polygons: Vec<(Vec<(f64, f64)>, Vec<Vec<(f64, f64)>>)>, pub skipped: usize }`: outer ring and holes per polygon.
  - `pub fn assemble(ways: &[RawWay], nodes: &BTreeMap<i64, (f64, f64)>) -> Rings`, public for tests: joins member ways end to end into rings, and pairs holes with the outer ring that contains them.

| OSM tag | `SurfaceClass` | Priority (higher wins) |
|---|---|---|
| `natural=water`, `landuse=reservoir` | Soil | 3 |
| `natural=glacier`, `natural=bare_rock`, `natural=scree`, `natural=shingle` | Rock | 2 |
| `landuse=residential`, `industrial`, `commercial`, `farmyard`, `military` | Soil | 1 |

Forest and grass tags are deliberately not read: WorldCover draws those edges better (10 m, every pixel), and `landuse=forest` polygons are large and coarse. Task 8 checks that decision on the real map.

- [ ] **Step 1: Review the crates.** `osmpbf` for reading; for filling polygons `geo-rasterize` against an own even-odd scanline fill in `surface.rs`. Follow *Dependencies* in `AGENTS.md`: read the source, check advisories, maintenance, panics on bad input and the licence. `osmpbf` failing the review stops the task: ask the maintainer. `geo-rasterize` failing means the own scanline, with the reason in the commit message.
- [ ] **Step 2: Write the failing tests**

`assemble`: `way_rings_close`, `split_outer_ring_is_joined` (three ways, some reversed), `inner_ring_becomes_a_hole`, `unclosed_ring_is_skipped_and_counted`, `missing_node_is_skipped_and_counted`. `read_land`: `fixture_has_the_koenigssee` (one Soil polygon containing 47.55, 12.975), `irrelevant_tags_are_ignored`, `elements_outside_the_window_are_dropped`, `result_does_not_depend_on_order` (the same fixture twice gives equal output). Fuzz seeds: the fixture, cut and damaged.

- [ ] **Step 3: Run them to see them fail.**
- [ ] **Step 4: Implement in two passes,** so that a large extract stays small in memory: the first pass keeps relevant ways and relations with their node ids, the second keeps only those nodes. Multipolygon relations use the `type=multipolygon` tag and member roles `outer` and `inner`.
- [ ] **Step 5: Run** `cargo test -p naves-core`, `make fuzz`, `make deny`. Expected: PASS. Measure a build on the 260 MB `oberbayern-latest.osm.pbf` in `/var/tmp` and note time and peak memory (`/usr/bin/time -v`) in the commit message.
- [ ] **Step 6: Commit** `feat(core): read land polygons from OSM`, with the crate review in the message.

### Task 5: Deciding the ground from the sources

**Files:**
- Modify: `crates/naves-core/src/surface.rs`
- Test: `crates/naves-core/tests/surface.rs`

**Interfaces:**
- Consumes: `LandCover` and `OsmLand` (Tasks 3 and 4), `procedural_class` (Task 2), `LocalProjection`.
- Produces:
  - `pub struct SurfaceSources<'a> { pub land_cover: Option<&'a LandCover>, pub osm: Option<&'a OsmLand> }`.
  - `pub struct SurfaceBuild { pub grid: SurfaceGrid, pub uncovered_share: f64 }`.
  - `pub fn build_surfaces(project: &Project, hm: &Heightmap, sources: &SurfaceSources<'_>) -> Result<SurfaceBuild, SurfaceError>`.

The order of decision for each cell, first answer wins:

1. An OSM polygon, highest priority first (in a polygon's hole, the polygon does not count).
2. The WorldCover class at the cell.
3. `procedural_class`: slope above 35° or above 2100 m real altitude is rock, otherwise grass. A cell decided here counts in `uncovered_share`, but only when a land-cover source was given.

Slope and altitude do not override real data: WorldCover has grass on a steep slope and that stays grass.

- [ ] **Step 1: Write the failing tests**

With `from_fn` heightmaps and small synthetic sources: `osm_beats_land_cover`, `higher_priority_polygon_wins`, `a_hole_lets_the_land_cover_through` (an island in a lake), `land_cover_beats_the_rules` (grass on a 60° wall stays grass), `without_any_source_it_is_the_procedural_grid` (equal to `procedural_grid`), `uncovered_part_uses_the_rules_and_is_reported` (a source covering half the map: `uncovered_share` near 0.5, the other half decided by the rules), `position_uses_the_map_scale` (at `scale.horizontal` 0.5, a polygon at a known lat and lon lands on the map position `build_heightmap` uses for it; compare against the same projection), `surfaces_are_deterministic`.

- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.** Rasterize polygons once into a per-cell array instead of testing each cell against each polygon. Create one `LocalProjection` per row, as `build_heightmap` does, and share `to_geo` logic with it rather than copying the map-to-real formula.
- [ ] **Step 4: Run** `cargo test -p naves-core`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): decide the ground from OSM, land cover and rules`.

### Task 6: Build pipeline and attribution

**Files:**
- Modify: `crates/naves-export/src/build.rs`, `crates/naves-export/src/testing.rs` if fixtures need it, `docs/manual/src/credits.md`, `docs/manual/src/getting-started.md`, `CHANGELOG.md`
- Test: `crates/naves-export/tests/build.rs`

**Interfaces:**
- Consumes: `build_surfaces`, `read_land`, `LandCover::open`, `build_layers`.
- Produces: `BuildOutput.warnings` gets a line for `uncovered_share` above 1 % and for `skipped_rings` above 0; the mod folder gets `ATTRIBUTION.txt`, listing only the sources this build used.

Attribution texts: Copernicus as in `credits.md`; OSM `© OpenStreetMap contributors, ODbL 1.0, https://www.openstreetmap.org/copyright`; WorldCover with the exact wording from the licence section of the product page, copied when this task is done, and CC BY 4.0.

- [ ] **Step 1: Write the failing tests**

`land_cover_reaches_the_mask` (a project with the fixture crop: a position known to be forest gives the green mask colour in its tile); `osm_reaches_the_mask` (the fixture's lake gives the blue mask colour); `attribution_names_the_sources_used` (DEM only: Copernicus and no OSM line; all sources: all three); `missing_source_file_names_the_file` (before any tool runs); `partial_coverage_warns` (a warning with the share in `BuildOutput.warnings`).

- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.** Read the sources next to the DEM at the start of `build`, using the window of the whole map (`LocalProjection::to_geo` of the map's corners plus a 100 m margin), before `layout.clean()`, so that a bad file costs nothing.
- [ ] **Step 4: Run** `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(export): build the ground from land cover and OSM`. The manual says plainly that a published map contains OSM-derived data under ODbL, including share-alike for the derived database (spec section 9).

### Task 7: See the surfaces in the GUI and over MCP

Start with the `/frontend-design:frontend-design` skill (`AGENTS.md`, *User interface*).

**Files:**
- Modify: `crates/naves-core/src/render.rs`, `crates/naves/src/mcp.rs`, `crates/naves/src/gui/mod.rs`, `crates/naves/src/gui/worker.rs`, `docs/manual/src/gui.md`, `docs/manual/src/mcp.md`, `CHANGELOG.md`
- Test: `crates/naves-core/tests/render.rs`, `crates/naves/tests/mcp.rs`, `crates/naves/src/gui/tests.rs`

**Interfaces:**
- Produces: `Overlay::Surface` in `render_topdown`, which takes `surfaces: Option<&SurfaceGrid>` and draws the class colours of Task 2's table; `OverlayArg::Surface` (`"surface"`) in MCP; a surface choice in the GUI's overlay selector; the source lists shown under "Elevation files".

- [ ] **Step 1: Write the failing tests**

`surface_overlay_uses_the_class_colours`; MCP `render_topdown_shows_surfaces` (with the fixture sources); GUI `land_cover_files_are_listed`. Changing a source list reloads the map: extend `heightmap_inputs` in the GUI, and the MCP cache comparison already covers it.

- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.** Compute the grid in the same background job as the heightmap, once per job.
- [ ] **Step 4: Run** `cargo test -p naves -p naves-core`. Expected: PASS.
- [ ] **Step 5: Commit** `feat: surface overlay in the GUI and over MCP`.

### Task 8: Berchtesgaden with real ground

**Files:**
- Modify: `examples/berchtesgaden/naves.toml`, `examples/berchtesgaden/README.md`, `examples/berchtesgaden/.gitignore`, `docs/research/2026-11-05-reference-terrains.md`

Sources for the example, which stay out of git:

```sh
# examples/berchtesgaden/landcover/
curl -fO https://esa-worldcover.s3.eu-central-1.amazonaws.com/v200/2021/map/ESA_WorldCover_10m_2021_v200_N45E012_Map.tif
# examples/berchtesgaden/osm/ (260 MB)
curl -fLO https://download.geofabrik.de/europe/germany/bayern/oberbayern-latest.osm.pbf
```

The Austrian part of the map is not in that extract. Check whether the map's corners reach Austria (Salzburg border east of the lake); if they do, also list the `austria` extract or an extract covering both, and note its size.

- [ ] **Step 1: Build with both sources.** Render `render_topdown` with `surface` over MCP and compare by eye with WorldCover and OSM for the whole map. Expected: the Königssee soil-coloured and sharply outlined, forest on the lower slopes, rock on the high massifs, Berchtesgaden town soil.
- [ ] **Step 2: Decide what the two sources get to say.** They disagreed on rock for this area (WorldCover 5.6 %, OSM up to about 24 %). Look at the Watzmann east face in the overlay with OSM rock on and off. If OSM's rock is clearly too coarse, drop `bare_rock` from Task 4's table or restrict it, and record the decision with a before and after in the research note.
- [ ] **Step 3: Check the game** with the patched test mission (noon, no fog, fixed spawn; `AGENTS.md`, *Comparable screenshots*): east shore looking west (x 3962, z 1400, yaw 270), the Watzmann from the north, and over the Königssee basin. Expected: player connects, no crash, navmesh loads (its geometry is unchanged, so it is not regenerated), and the ground visibly matches the overlay.
- [ ] **Step 4: Measure the build:** time, peak memory, PBO size. Record them with the results in the README.
- [ ] **Step 5: Run** `TMPDIR=/var/tmp make check`. Expected: PASS. **Commit** `docs: Berchtesgaden with land cover`. Push; CI, CodeQL and Codecov green.

### Task 9: `areaflags.map` spike

The generated mission still carries Livonia's 75 MB `areaflags.map`, and `AGENTS.md` records that it belongs to the map size it was made for. Time-boxed to half a day. The outcome is a note, and code only if the format turns out to be simple.

**Files:**
- Create: `docs/formats/areaflags.md`

- [ ] **Step 1: Observe the file only:** size against map size across the installed missions (Chernarus, Livonia, Sakhal if present), header, repeating structure, and what the server logs when the file is missing, wrong or empty on a test server. Per spec section 9, no disassembly.
- [ ] **Step 2: Write down** what is known, what the server does in each case, and a recommendation: leave it, empty it, or generate it from `SurfaceGrid`.
- [ ] **Step 3: If an empty or absent file is accepted and harmless,** make `write_mission` do that, test first in `crates/naves-export/tests/mission.rs`. Otherwise stop here and report back to the maintainer.
- [ ] **Step 4: Commit** `docs: what is known about areaflags.map`.

---

## After this plan

Vegetation objects (trees on `Forest` cells up to the measured tree line of about 1900 m real altitude) are the natural next step, because forest without trees is only coloured ground. The remaining order of plan 1's "Later plans" stands.

<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Náves M1: satellite imagery as the base of the terrain — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** for any area on Earth, the user is shown which imagery sources exist there and what each one allows, Náves fetches the chosen ones into the project, and `naves build` merges them, overlaps included, into the satellite images of the terrain, and says so in the finished map when a source is non-commercial.

**Architecture:** retrieving and building stay apart. `naves imagery list` and `naves imagery fetch` (also in the GUI and over MCP) turn catalogue entries into ordinary GeoTIFF files in the project, listed in `naves.toml` with their licence. `naves build` never touches the network: `naves_core::imagery` reads the listed files in the window of the map, measures their colours against each other where they overlap, and paints one composite. `naves-export` draws it into the tile images and writes the notices. Without imagery the map is built exactly as today.

**Tech Stack:** Rust 1.95, existing `geotiff-reader`, `proj4rs`, `rayon`, `image`, `eframe`/`egui`; `image` gains the `jpeg` feature; `tiff` (the encoder, today a dev-dependency) and an HTTP client (`ureq`, after review, Task 7) are new in normal dependencies.

**Spec:** `docs/superpowers/specs/2026-11-04-naves-design.md`, sections 5.1, 5.2 (surface layers), 5.3 (stages 1 and 8), **5.12 (added with this plan)** and 9. Interface: `docs/superpowers/specs/2026-11-06-naves-editor-ui.md`. Geometry: `docs/formats/terrain-layers.md`.

**Starts from:** `main` (after plan 2, land cover). Uses `MapGeo`, `GeoWindow`, `SurfaceGrid`, `build_surfaces`, `decide_ground`, `attribution`, `BuildOutput.warnings`. It is independent of the roads plan. When both are done, a path stroke from the roads plan must still show on the satellite image: whichever lands second adds that test.

**Decisions of the maintainer (2026-11-06):**

- *Overlaps:* areas can have images from different sources and flights. Use a source that covers the whole area where there is one, otherwise analyse the images and merge their colours and resolution (Task 3).
- *Non-commercial data is allowed*, because DayZ maps usually are non-commercial, but the resulting map has to say so (Task 4).
- *More than Berchtesgaden:* the user is presented with several options for retrieving data (Tasks 6 to 8). This lifts, for imagery only, plan 2's "Náves itself does not download anything".

## Findings this plan rests on (checked on 2026-11-06)

| Option | Coverage, resolution | Licence | What was observed |
|---|---|---|---|
| Sentinel-2 L2A scenes, `earth-search.aws.element84.com/v1`, collection `sentinel-2-c1-l2a`, asset `visual` | world, 10 m | Copernicus free and open; commercial use allowed | STAC search works without an account. Scene `S2C_T33TUN_20250919T101514_L2A` has 0.0006 % cloud and covers the whole Berchtesgaden window (band minimum 1 over the crop, no no-data). 335 MB; DEFLATE with predictor 2, pixel-interleaved, EPSG:32633, `NoData 0`, 4 overviews |
| EOxCloudless, WMS `tiles.maps.eox.at/wms`, layers `s2cloudless-2017` to `-2025` | world, 10 m, already a mosaic | **2018 to 2025: CC BY-NC-SA 4.0**; 2016: CC BY 4.0 | Answers **JPEG even when asked for PNG**. Attribution must be visible or "under credits, data sources, or as a part of metadata". Modified imagery may be redistributed as part of a work with its licence stated. Sub-licensing under your own terms is not allowed |
| Bayern DOP40, WMS `geoservices.bayern.de/od/wms/dop/v1/dop40`, layer `by_dop40c` | Bavaria, 0.4 m | CC BY 4.0 (per OSM wiki, confirm in Task 6) | RGBA PNG, **transparent outside Bavaria** |
| NRW `wms_nw_dop` (`nw_dop_rgb`), swisstopo `ch.swisstopo.swissimage`, IGN `ORTHOIMAGERY.ORTHOPHOTOS` | region, sub-metre | confirm in Task 6 | PNG. Maximum request sizes 5000, 10000 and 5010 px where the service states one |
| PDOK `luchtfotorgb` (`Actueel_orthoHR`) | Netherlands | confirm in Task 6 | Answers JPEG when asked for PNG; maximum 2500 px |
| USGS NAIP | USA | public domain (confirm) | **Failed:** our request got an XML error. Listed only if Task 6 gets it to work |

All of these services state EPSG:4326. Google, Bing, Mapbox and Esri imagery is not offered: as far as we know their terms do not allow it to end up in a redistributed map. Task 6 records the terms with their sources; this was not checked yet.

`geotiff-reader` 0.11 with today's features opens the real scene: `read_window::<u8>` gives `[rows, cols, 3]`, `read_band_window` and `read_overview_window` work, `epsg()` and `nodata()` are right, 14 ms for 1300 × 1300.

At the default scale one image pixel is 1.67 real metres (`px_m` 0.83 ÷ `scale.horizontal` 0.5), so a 10 m source is enlarged about 6 × and 0.4 m aerial photos are reduced about 4 ×. The scene is dark for a terrain: mean RGB 44, 53, 34 over the map, against 63, 68, 37 for a Livonia tile's `AVGC`. Its forests are nearly black, and it holds real cast shadows, which is why a brightness setting and a look in the game are part of this plan (Task 4).

Not in this plan: WMTS and tile-pyramid services (basemap.at is one; such data can be supplied as a file), cloud or shadow removal, painting on the satellite image, rendering objects into it (spec section 6), a download inside `naves build`.

## Global Constraints

- **`AGENTS.md` applies to every task**: tests first, coverage at or above 85 %, strict lints, `make check` before every commit, user docs and the `CHANGELOG.md` line in the same commit. Where this plan and `AGENTS.md` disagree, `AGENTS.md` wins.
- **Same input, same output:** a build reads only local files. Floating-point sums over a parallel iterator depend on how rayon splits the work, so sums run per cell or per row in a fixed order and are combined sequentially. Two builds give identical images (test in Task 3).
- **No network in tests.** Fetching goes through a trait; tests use a fake. Real services are only touched by `#[ignore = "needs network"]` tests and by hand.
- **Every parser of untrusted input gets a fuzz target whose seeds also run in `make test`:** imagery GeoTIFF (Task 2), catalogue (Task 6), STAC response (Task 7). Server responses are untrusted: size limits, no allocation from reported sizes, no panics.
- Spec section 9, verbatim: "Generated maps contain OSM-derived data: ODbL attribution (and share-alike for the derived database) must be written into the mod; same for Copernicus and imagery sources."
- Large data goes under `/var/tmp` on the development machine; run `make check` with `TMPDIR=/var/tmp`. Fixtures are small crops with attribution in `tests/fixtures/README.md`. Every Wine or Proton program runs in a headless sway session. One game at a time; remove the DayZ install's mod symlink afterwards.
- Map coordinates: x east, y north, metres, origin south-west. Tile images are north-up. Real position to map position only through `MapGeo`.
- Every HTTP request sends the generic user agent `naves/<version>`, one request at a time, and no personal data.
- **What counts as a failure of this plan:** a visible seam or colour step where two sources meet that the merge should have removed; a map using non-commercial data without saying so; a build that needs the network.

## Review Focus

1. **Real change between two images, not exposure** (a mown field, snow in one flight, a new building estate): the difference must not be stamped into the corrected image as a blotch around that spot. Test in Task 3: `changed_content_leaves_no_stamp`.
2. **A service that answers blank outside its area** (transparent as Bayern does, or an opaque colour): blank is no-data, never imagery, or the merge would measure against white. Test in Task 7: `blank_pixels_are_no_data`.
3. **A map across a UTM zone border**, or scenes from two zones: files in different zones line up to within a pixel. Test in Task 2: `file_in_the_neighbouring_zone_lines_up`.
4. **A project copied to another machine, or offline:** the build works from the files alone. A missing file is named before any tool runs, with the fetch command when the entry came from the catalogue. Test in Task 4: `missing_imagery_names_the_file`.
5. **The notice follows actual use:** a non-commercial file that does not touch the map gives no notice and no credit, and one that touches it gives both, even if a sharper file covers most of it. A licence containing `NC` with `non_commercial = false` is rejected. Tests in Tasks 1 and 4.

---

### Task 1: Imagery in the project file

**Files:**
- Modify: `crates/naves-core/src/project.rs`, `docs/manual/src/project-file.md`, `CHANGELOG.md`
- Test: `crates/naves-core/tests/project.rs`

**Interfaces:**
- Produces, in `naves_core::project`:
  - `pub struct ImageryFile { pub path: PathBuf, pub attribution: String, pub licence: String, #[serde(default)] pub non_commercial: bool }`
  - `Sources.imagery: Vec<ImageryFile>` (`serde(default)`, skipped when empty)
  - `pub struct ImagerySettings { pub brightness: f64 }` and `Project.imagery: ImagerySettings` (`serde(default)`, skipped when default; `DEFAULT_BRIGHTNESS` is 1.0 until Task 4 sets it)

```toml
[[sources.imagery]]
path = "imagery/sentinel2-S2C_T33TUN_20250919T101514_L2A.tif"
attribution = "Contains modified Copernicus Sentinel data 2025"
licence = "Copernicus Sentinel data, free and open"

[[sources.imagery]]
path = "imagery/eox-cloudless-2024.tif"
attribution = "EOxCloudless https://cloudless.eox.at by EOX IT Services GmbH (Contains modified Copernicus Sentinel data 2024)"
licence = "CC BY-NC-SA 4.0"
non_commercial = true
```

- [ ] **Step 1: Write the failing tests.** `old_project_has_no_imagery` (a file without the keys loads, and saving it does not add them); `imagery_round_trips`; `imagery_paths_stay_inside_the_project` (same rules and message style as `sources.dem`); `attribution_and_licence_are_required` (empty is an error naming the field); `attribution_is_one_short_line` (control characters or more than 500 characters are errors; it ends up in `mod.cpp`); `nc_licence_must_be_marked` (`CC BY-NC-SA 4.0` with `non_commercial = false` is an error naming the field, split into tokens on non-alphanumerics and compared case-insensitively, so `Licence` does not match); `brightness_range` (0.25 to 4, finite); `imagery_can_be_set_over_project_set` (`with_value("sources.imagery", [...])`).
- [ ] **Step 2: Run them to see them fail:** `cargo test -p naves-core --test project`. Expected: compile errors.
- [ ] **Step 3: Implement,** with the path check of `validate` shared rather than copied. Document the fields in `project-file.md`.
- [ ] **Step 4: Run** `cargo test -p naves-core && make fuzz`. Expected: PASS; `project_parse` still clean.
- [ ] **Step 5: Commit** `feat(core): imagery sources in the project file`.

### Task 2: Reading imagery files

**Files:**
- Create: `crates/naves-core/src/imagery.rs`, `fuzz/fuzz_targets/imagery_tile.rs`, `fuzz/corpus/imagery_tile/*`, `tests/fixtures/imagery/*` (a Sentinel-2 crop of about 64 × 64 px made with `gdal_translate`; attribution in `tests/fixtures/README.md`)
- Modify: `crates/naves-core/src/lib.rs`, `fuzz/Cargo.toml`, `crates/naves-core/tests/fuzz_seeds.rs`
- Test: `crates/naves-core/tests/imagery.rs`

**Interfaces:**
- Produces, in `naves_core::imagery`:
  - `pub struct ImageryRaster`, with `pub fn open(path: &Path, project: &Project, target_m: f64) -> Result<Option<ImageryRaster>, ImageryError>` (`None` if the file does not touch the map), `pub fn from_bytes(bytes: Vec<u8>, project: &Project, target_m: f64) -> Result<Option<ImageryRaster>, ImageryError>`, and for tests `pub fn from_fn(west, south, east, north, pixel_deg, f: impl Fn(f64, f64) -> Option<[u8; 3]>) -> ImageryRaster` behind `test-util`.
  - `pub fn pixel_m(&self) -> f64`: real metres per pixel of what was read. `pub fn pixel_count(&self) -> usize`.
  - `pub fn sampler(&self) -> Result<RasterSampler<'_>, ImageryError>`, one per thread because a `Proj` is not shared. `RasterSampler::rgb_at(&self, lat: f64, lon: f64) -> Option<[f32; 3]>`: bilinear; `None` outside the raster or if any of the four pixels is no-data.
  - `pub fn real_px_m(project: &Project) -> f64`: the map's image pixel in real metres, the `target_m` to pass.

Rules:

| Input | Behaviour |
|---|---|
| EPSG:4326, 326xx, 327xx, 3857 | read |
| EPSG:258xx (ETRS89 UTM) | read, as UTM on GRS80 |
| any other CRS | error naming the file and the code, with `gdalwarp -t_srs EPSG:<the map's UTM code>` |
| not 8 bit, fewer than 3 bands | error naming the file, with `gdal_translate -ot Byte -scale` (the Copernicus mosaics are 16-bit and separate bands: `gdal_merge` first) |
| compression the decoder rejects (JPEG in TIFF) | error naming the file, with `gdal_translate -co COMPRESS=DEFLATE` |
| rotated transform | error, as in `landcover.rs` |
| 4th band | alpha: 0 is no-data |
| all three bands equal to the `NoData` value | no-data |

Overviews: use the coarsest level whose pixels are still at most `target_m`, otherwise the base image. A window above `MAX_WINDOW_SIDE` (16 000, as in `landcover.rs`) at the chosen level is an error naming the file, with `gdaladdo` if the file has no usable overview. A file too sharp and without overviews is never decimated pixel by pixel, because that aliases. The window is the map's corners in the file's CRS plus a 100 m margin; check its size before allocating.

- [ ] **Step 1: Write the failing tests.** `crop_matches_gdal` (three positions of the committed crop, values read with `gdallocationinfo`, nearest to a pixel centre, within 1); `north_is_up` and `east_is_right` (a synthetic file with one marked pixel, found at the map position `MapGeo::to_map` gives, within one pixel, also at `scale.horizontal = 0.5`); `file_in_the_neighbouring_zone_lines_up` (the same synthetic content in EPSG:32632 and 32633 near 12° E gives the same colour within 1 at ten positions); `only_the_window_is_read` (`pixel_count` of a 4000 × 4000 file against a small map); `outside_is_none`; `no_data_and_alpha_are_none`; `overview_is_chosen` (a file with overviews and a coarse target reads the right level, `pixel_m` says so); `too_large_without_overview_names_gdaladdo`; `wrong_crs_names_gdalwarp`; `sixteen_bit_names_gdal_translate`. Fuzz seeds: the crop and two damaged copies.
- [ ] **Step 2: Run them to see them fail:** `cargo test -p naves-core --test imagery`.
- [ ] **Step 3: Implement.** Follow `read_window` in `landcover.rs` for the window arithmetic, but with a CRS from the table.
- [ ] **Step 4: Run** `cargo test -p naves-core` and `make fuzz`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): read imagery GeoTIFFs in the window of the map`, with the Copernicus Sentinel text in `credits.md`.

### Task 3: Merging overlapping images

The heart of the plan. Pure computation on rasters, no I/O.

**Files:**
- Create: `crates/naves-core/src/imagery/merge.rs` (a submodule of `imagery.rs`)
- Test: `crates/naves-core/tests/imagery_merge.rs`

**Interfaces:**
- Consumes: `ImageryRaster`, `RasterSampler`.
- Produces:
  - `pub struct Composite`; `pub fn merge(rasters: Vec<ImageryRaster>, project: &Project) -> Result<(Composite, MergeReport), ImageryError>`, with `rasters` in the order of `sources.imagery`.
  - `pub struct MapRect { pub west: f64, pub north: f64, pub step_x: f64, pub step_y: f64, pub width: u32, pub height: u32 }`; `Composite::paint(&self, project: &Project, rect: &MapRect) -> Result<Vec<Option<[u8; 3]>>, ImageryError>`: pixel centres, row 0 at the north. One call per tile or render, safe to run in parallel.
  - `pub struct TileImagery<'a> { pub composite: &'a Composite, pub project: &'a Project }`: what `build_layers` and `render_topdown` take.
  - `pub struct MergeReport { pub reference: Option<usize>, pub sources: Vec<SourceReport>, pub warnings: Vec<String> }`; `pub struct SourceReport { pub coverage: f64, pub overlap: f64, pub median_gain: [f32; 3], pub changed_share: f64 }`.

**The method.** Everything is measured on a coarse grid of one cell per land cell (`grid.land_size` per side, 100 real metres at the defaults): fine enough for the fall-off across an aerial photo, coarse enough that a car or a roof does not count, and small enough for any map.

1. **Coarse means.** For every raster, the mean colour per cell from 4 × 4 samples. A cell with any no-data sample has no mean.
2. **Reference:** the raster with the most cells. Ties go to the first listed. Its colours are kept as they are. So one source covering the whole area sets the colours for all others, which is the maintainer's first choice, and a Sentinel-2 base under aerial photos needs no configuration.
3. **Colour matching.** Take the remaining rasters by largest overlap with the coarse composite so far (ties: first listed), so that a flight touching only another flight still gets corrected through it. For each, per channel and cell where both have a mean above `DARK_FLOOR` (8): `gain = composite / source`. Then:
   - `global` = median of the gains (sort with `total_cmp`), clamped to `GAIN_LIMITS` (0.4 to 2.5);
   - a cell gain further than `CONTENT_CHANGE` (25 %) from `global` is real change and not exposure, and is replaced by `global` (its share is `changed_share`);
   - cells without overlap take `global`;
   - two passes of a box blur of radius 4 cells;
   - fewer than `MIN_OVERLAP_CELLS` (16) cells: gain 1 and a warning naming the file ("overlaps no other image, its colours are not matched").
   Add the corrected raster to the coarse composite. Because the gain field is smooth, it evens out colour and leaves the detail alone.
4. **Who wins.** Effective pixel size = `max(pixel_m, target_m)`, so two sources both sharper than the map are equal. Paint from the coarsest upwards; among equals, the first listed goes on top.
5. **Feathering.** A raster's alpha per cell is `min(1, distance to its nearest cell without a mean / FEATHER_CELLS)`, with `FEATHER_CELLS` = 4 and a chamfer distance transform. The map's own edge does not count as an edge. At a pixel: `out = lerp(out, corrected, alpha)`, with alpha bilinear between cells. If nothing is underneath, or the pixel below is no-data, the raster's own valid pixel is used as is, so coverage never shrinks. No-data pixels in the raster leave what is below.
6. **One raster:** no analysis, gain 1, alpha 1: its pixels pass straight through.

- [ ] **Step 1: Write the failing tests,** on `from_fn` rasters with a fixed seed and a textured colour field. A helper paints a row across a seam.
  - `one_source_passes_through`: output equals `rgb_at`, rounded.
  - `whole_area_source_is_the_reference`: a full 10 m raster and a partial 1 m one with gain 1.4: the report names the full one, and the mean of the merged image over the partial area is within 2 levels of the full one.
  - `exposure_step_disappears`: two flights, 1.0 and 1.4, half overlapping, no third source: along a row across the seam no step between neighbouring pixels above 3 levels beyond the texture's own.
  - `falloff_inside_a_photo_is_evened_out`: gain 1.0 at the centre to 1.3 at the edge against a full reference: within 4 levels of the reference at every coarse cell.
  - `detail_survives`: standard deviation of a 1 m texture after correction within 5 % of the expected value.
  - `sharpest_wins`: over a coarse raster with no high frequency, a fine raster's pixel-to-pixel variation stays at the centre of the fine raster.
  - `edge_is_feathered`: going from fine to coarse takes `FEATHER_CELLS` cells, monotonically in the mix.
  - `equals_first_listed_on_top`, with the order swapped too.
  - `sharper_than_the_map_counts_as_equal`: 0.2 m and 0.4 m at a 1.67 m target follow the list order.
  - `changed_content_leaves_no_stamp`: a block bright only in the second flight: the area around the block, 3 cells beyond it, stays within 3 levels of the image without the block; `changed_share` above 0.
  - `chain_of_flights_is_matched`: A overlaps B overlaps C, C does not touch A: C's mean is within 3 levels of A's on a flat scene.
  - `no_overlap_warns_and_keeps_colours`.
  - `hole_in_a_flight_falls_through`: a no-data pocket shows the source below.
  - `dark_water_does_not_drive_gains`: a dark lake with a different colour in both images, with means below `DARK_FLOOR`, leaves `median_gain` unchanged.
  - `two_runs_are_identical`, with 1 and 8 rayon threads.
- [ ] **Step 2: Run them to see them fail:** `cargo test -p naves-core --test imagery_merge`.
- [ ] **Step 3: Implement.** Keep the constants named and documented with why.
- [ ] **Step 4: Run** `cargo test -p naves-core`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): merge overlapping imagery by colour and resolution`.

### Task 4: Into the map, with its notices

**Files:**
- Modify: `crates/naves-export/src/layers.rs`, `crates/naves-export/src/build.rs`, `crates/naves-core/src/project.rs` (`DEFAULT_BRIGHTNESS`), `docs/manual/src/credits.md`, `docs/manual/src/getting-started.md`, `docs/manual/src/troubleshooting.md`, `docs/formats/terrain-layers.md`, `examples/berchtesgaden/{naves.toml,README.md,.gitignore}`, `CHANGELOG.md`
- Test: `crates/naves-export/tests/layers.rs`, `crates/naves-export/tests/build.rs`

**Interfaces:**
- Consumes: `merge`, `Composite::paint`, `Sources.imagery`.
- Produces: `build_layers(hm, land_size, prefix, surfaces, imagery: Option<&TileImagery<'_>>) -> Result<LayerSet, ImageryError>`, plus `LayerSet.imagery_share: f64`. `attribution(...)` takes the imagery files actually used. The mod folder gets `mod.cpp`; `ATTRIBUTION.txt` goes into the mod folder **and** the mission folder, because server owners install the mission separately.

Drawing, in `render_tile`: a pixel the composite covers is its colour × `project.imagery.brightness`, clamped, with **no baked hillshade**, because the image holds its own shadows. A pixel it does not cover keeps today's class colour and hillshade. The mask is untouched: near the player the ground is still drawn by the surface layers.

The notice, as the first thing in `ATTRIBUTION.txt` when a used file has `non_commercial = true`:

```text
NON-COMMERCIAL USE ONLY
The satellite images of this map come from sources that do not allow commercial use (marked below).
Do not use this map commercially without a licence from those sources.
```

If a used licence has the token `SA`, add under it: `Images derived from a share-alike source stay under that licence (marked below).` Below come one entry per used file: its attribution, its licence, and `non-commercial` where it applies. `mod.cpp` holds `name` (the project's name) and `overview` (`Made with Náves. Non-commercial use only: see ATTRIBUTION.txt.`, or without the second sentence when no such file is used), with `"` doubled. `BuildOutput.warnings` gets the same message, and the CLI prints it.

The credit is written once per distinct attribution, so five Sentinel-2 scenes of one year are one line.

- [ ] **Step 1: Write the failing tests.** `layers.rs`: `no_imagery_is_todays_image` (byte-identical to `None` with the current drawing); `imagery_replaces_class_colour_without_shading` (a steep shaded wall under imagery has exactly the imagery colour); `uncovered_pixels_fall_back` ; `brightness_is_applied_and_clamped`; `marked_pixel_lands_on_the_tile_position` (a marker at a known map position in the right tile and pixel, including the overlap: the same point in two neighbouring tiles' overlaps has the same colour). `build.rs`, with the fake tools: `imagery_reaches_the_png`; `missing_imagery_names_the_file` (before any tool runs); `bad_imagery_costs_nothing` (the previous output stays); `partial_coverage_warns` (above 1 %, with the share); `merge_warnings_reach_the_build`; `non_commercial_is_stated_everywhere` (`ATTRIBUTION.txt` starts with the notice in mod and mission, `mod.cpp` says it, `warnings` says it); `commercial_sources_state_no_restriction`; `unused_source_gets_no_notice_and_no_credit` (a non-commercial file outside the map); `covered_source_still_counts` (non-commercial, touching the map, covered by a sharper file: notice stays); `share_alike_line`; `same_attribution_once`; `quote_in_attribution_is_escaped_in_mod_cpp`; `build_is_reproducible_with_imagery`; `old_project_builds_the_same` (no imagery keys: same PNGs as before this plan).
- [ ] **Step 2: Run them to see them fail:** `cargo test -p naves-export`.
- [ ] **Step 3: Implement.** Read and merge next to `decide_ground`, before `layout.clean()`. Only a raster that `open` returned counts as used.
- [ ] **Step 4: Run** `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5: Look at it in the game.** Build `examples/berchtesgaden` with this scene, listed by hand (`curl -fO` into `examples/berchtesgaden/imagery/`):
  `https://e84-earth-search-sentinel-data.s3.us-west-2.amazonaws.com/sentinel-2-c1-l2a/33/T/UN/2025/9/S2C_T33TUN_20250919T101514_L2A/TCI.tif`
  Test server with the patched mission (`AGENTS.md`, *Comparable screenshots*), screenshots from the land-cover views (x 3962, z 1400, yaw 270; the Watzmann from the north; over the Königssee basin), from the ground and from a height. Expected: no crash, no tile seams, the lake and town where they should be, and the land-cover mask agreeing with the image.
- [ ] **Step 6: Set `DEFAULT_BRIGHTNESS`.** Compare `brightness` 1.0, 1.3 and 1.6 against a Livonia view at the same time of day, and record the choice and screenshots' findings in `docs/research/2026-11-05-reference-terrains.md`. A brighter default must not change a project that sets its own. If the cast shadows or the shift from the far image to the near ground look wrong at every brightness, **stop and report to the maintainer**: fixing that (a blend towards class colours, or a different season) is a design decision.
- [ ] **Step 7: Run** `TMPDIR=/var/tmp make check`. Expected: PASS.
- [ ] **Step 8: Commit** `feat(export): build the satellite images from imagery, state its licences in the map`. The manual explains what the notice means and that it is the map maker's responsibility, not legal advice.

### Task 5: See it in the GUI and over MCP

Start with the `/frontend-design:frontend-design` skill (`AGENTS.md`, *User interface*).

**Files:**
- Modify: `crates/naves-core/src/render.rs`, `crates/naves/src/mcp.rs`, `crates/naves/src/gui/mod.rs`, `crates/naves/src/gui/worker.rs`, `docs/manual/src/gui.md`, `docs/manual/src/mcp.md`, `docs/superpowers/specs/2026-11-06-naves-editor-ui.md` (Imagery joins Relief, Slope, Height, Ground as base maps), `CHANGELOG.md`
- Test: `crates/naves-core/tests/render.rs`, `crates/naves/tests/mcp.rs`, `crates/naves/src/gui/tests.rs`

**Interfaces:**
- Produces: `Overlay::Imagery`; `render_topdown(hm, req, surfaces, imagery: Option<&TileImagery<'_>>)`; `RenderError::NoImagery`; `OverlayArg::Imagery` (`"imagery"`) in MCP; "Imagery" in the GUI's overlay choice; the listed files with licence under "Land cover files"; a line "Non-commercial map" in the project panel when a used file is non-commercial; `MergeReport` warnings under Problems.

In this view uncovered ground is drawn checkered, not filled with class colours, so a gap in the imagery can be seen.

- [ ] **Step 1: Write the failing tests.** `imagery_overlay_shows_the_composite`; `imagery_overlay_marks_gaps`; `imagery_overlay_without_sources_is_an_error`; MCP `render_topdown_shows_imagery`, with the fixture; MCP cache follows a changed file list; GUI `imagery_files_are_listed` and `non_commercial_is_shown`; extend `heightmap_inputs` so a changed list reloads the map.
- [ ] **Step 2: Run them to see them fail:** `cargo test -p naves -p naves-core`.
- [ ] **Step 3: Implement.** Merge in the same background job as the heightmap, once per job. MCP makes it only when asked, as with surfaces.
- [ ] **Step 4: Run** `cargo test -p naves -p naves-core`. Expected: PASS.
- [ ] **Step 5: Commit** `feat: imagery base map in the GUI and over MCP`.

### Task 6: The catalogue: what can be had for this area

**Files:**
- Create: `crates/naves-core/src/imagery/catalog.rs`, `crates/naves-core/data/imagery-sources.toml`, `fuzz/fuzz_targets/imagery_catalog.rs`, `fuzz/corpus/imagery_catalog/*`, `docs/research/2026-11-06-imagery-sources.md`, `docs/manual/src/imagery.md`
- Modify: `crates/naves/src/cli.rs`, `crates/naves/src/output.rs`, `crates/naves/src/mcp.rs`, `docs/manual/src/SUMMARY.md`, `docs/manual/src/cli.md`, `docs/manual/src/mcp.md`, `CHANGELOG.md`, `crates/naves-core/tests/fuzz_seeds.rs`, `fuzz/Cargo.toml`
- Test: `crates/naves-core/tests/imagery_catalog.rs`, `crates/naves/tests/cli.rs`, `crates/naves/tests/mcp.rs`

**Interfaces:**
- Produces, in `naves_core::imagery::catalog`:
  - `pub struct CatalogSource { pub id: String, pub name: String, pub kind: SourceKind, pub coverage: Option<[f64; 4]> /* west, south, east, north; None is the world */, pub native_m: f64, pub licence: String, pub attribution: String /* may hold {year} */, pub non_commercial: bool, pub note: String }`
  - `pub enum SourceKind { Stac { url: String, collection: String, asset: String, asset_hosts: Vec<String> }, Wms { url: String, layer: String, max_px: u32, blank: Option<[u8; 3]> } }`
  - `pub fn catalog(user_file: Option<&Path>) -> Result<Vec<CatalogSource>, CatalogError>`: the built-in list, then entries from the user's file; a user id replaces a built-in one.
  - `pub struct ImageryOption { pub source: CatalogSource, pub coverage_share: f64 }`; `pub fn options_for(sources: &[CatalogSource], window: GeoWindow) -> Vec<ImageryOption>`: no source without overlap. Order: whole map first; then commercial use allowed; then sharper; then id.
- Produces: `naves imagery list [DIR]`; MCP tool `imagery_options`, with the same data as JSON.
- The user's file is `$XDG_CONFIG_HOME/naves/imagery-sources.toml`, deliberately **not** in the project: opening someone's project must never make Náves fetch from a URL that project chose. Only `https://` URLs validate.

Output, for a project (values illustrative):

```text
Imagery for Berchtesgaden (12.88..13.05 E, 47.53..47.64 N)

ID                 COVERS      PIXEL   LICENCE                                NOTES
sentinel2-scenes   whole map   10 m    Copernicus, free and open              single dates: pick a clear one
eox-cloudless-2025 whole map   10 m    CC BY-NC-SA 4.0  NON-COMMERCIAL        cloud-free mosaic
bayern-dop40       part (~70%) 0.4 m   CC BY 4.0                              aerial photos of Bavaria

Fetch with: naves imagery fetch <ID> [DIR]
Several can be combined: Náves evens out their colours and uses the sharpest where they overlap.
Or use your own files: see the manual, "Imagery".
```

- [ ] **Step 1: Check every entry by hand,** one request each, and write the evidence into `docs/research/2026-11-06-imagery-sources.md`: endpoint, layer, what it answers (format, alpha, blank colour), EPSG:4326 at 2000 px, coverage box, licence and exact attribution **copied from the publisher's page with the URL and date**, whether fetching for an offline product is allowed. Candidates: the rows of *Findings*, EOxCloudless 2016 (the layer of the CC BY year; the capabilities list starts at 2017), and one more each for at least two regions outside Europe if they pass. **An entry that fails any of these does not ship.** Sentinel-2 and EOX must ship; if either fails, stop and report to the maintainer.
- [ ] **Step 2: Write the failing tests.** `builtin_catalogue_parses_and_is_valid` (ids unique, https only, `native_m` positive, an `NC` licence is marked); `options_are_filtered_by_area` (Bayern for Berchtesgaden, not for a map in Peru; Sentinel-2 for both); `options_are_ordered`; `coverage_share_is_right` for a box halfway across the window; `user_file_adds_and_replaces`; `http_url_is_rejected`; `every_area_has_an_option` (proptest with a checked-in regressions file: any valid window gets at least one option covering it whole). CLI `imagery_list_shows_licences_and_the_non_commercial_mark`, with `XDG_CONFIG_HOME` in a temp dir. MCP `imagery_options_lists_the_same`. Fuzz seeds: the built-in file and two damaged copies.
- [ ] **Step 3: Run them to see them fail:** `cargo test -p naves-core -p naves`.
- [ ] **Step 4: Implement,** and write `docs/manual/src/imagery.md`: how to choose, a table of the options, what non-commercial means for a published map, how the merge decides, why commercial basemaps are not offered, and how to use your own files with their `gdal_translate`/`gdalwarp` commands and a `[[sources.imagery]]` example.
- [ ] **Step 5: Run** `cargo test --workspace && make fuzz && make docs`. Expected: PASS.
- [ ] **Step 6: Commit** `feat: show which imagery sources exist for the area of a project`. The CLI help and the manual agree.

### Task 7: Fetching

**Files:**
- Create: `crates/naves-export/src/imagery_fetch.rs`, `crates/naves-export/src/geotiff_write.rs`, `fuzz/fuzz_targets/stac_response.rs`, `fuzz/corpus/stac_response/*`
- Modify: `crates/naves-export/Cargo.toml`, `crates/naves-export/src/lib.rs`, `crates/naves-export/src/testing.rs`, `crates/naves/src/cli.rs`, `crates/naves/src/mcp.rs`, `Cargo.lock`, `docs/manual/src/{imagery,cli,mcp,troubleshooting}.md`, `docs/research/2026-11-04-dependency-reviews.md`, `CHANGELOG.md`, `fuzz/Cargo.toml`
- Test: `crates/naves-export/tests/imagery_fetch.rs`, `crates/naves/tests/cli.rs`, `crates/naves/tests/mcp.rs`

**Interfaces:**
- Consumes: `CatalogSource`, `SourceKind`, `Project::save`, `ImageryRaster::open`.
- Produces, in `naves_export::imagery_fetch`:
  - `pub trait Http: Sync { fn get(&self, url: &str, max_bytes: u64, out: &mut dyn Write) -> Result<(), FetchError>; fn post_json(&self, url: &str, body: &str, max_bytes: u64) -> Result<Vec<u8>, FetchError>; }`; `pub struct RealHttp` (timeouts, three tries with growing delay, the user agent of *Global Constraints*).
  - `pub struct FetchRequest { pub from: Option<String>, pub to: Option<String> }`: ISO dates, for STAC only.
  - `pub fn fetch(project_dir: &Path, source: &CatalogSource, request: &FetchRequest, http: &dyn Http, progress: &dyn Fn(&str)) -> Result<Vec<ImageryFile>, FetchError>`: writes the files, adds them to `naves.toml`, and returns what was added.
- Produces, in `geotiff_write`: `pub fn write_rgb(path: &Path, rgb: &RgbMosaic, bounds: GeoWindow) -> Result<(), FetchError>`: EPSG:4326, 8 bit, DEFLATE, `NoData` 0, written atomically; `pub struct RgbMosaic { pub width: u32, pub height: u32, pub data: Vec<u8> }`.
- Produces: `naves imagery fetch <ID> [DIR] [--from DATE] [--to DATE]`, printing size estimates, the licence and the non-commercial mark first. MCP tool `imagery_fetch { id, from?, to? }`: ids from the catalogue only, so it takes no URL. Runs in `spawn_blocking`. The MCP write lock of `project_set` also covers the project update here.

**WMS.** Everything is requested in EPSG:4326 (WMS 1.3.0: the bounding box is lat, lon), as every service states it. Pixel size is `max(native_m, real_px_m(project))`, over the map's window plus 100 m. Tiles of `max_px` or 2048 px, whichever is smaller, one request at a time, each decoded by its content (PNG or JPEG, see *Findings*), never by the header it claims. No-data: alpha 0, or exactly the entry's `blank`. A real `[0, 0, 0]` pixel becomes `[1, 1, 1]` so it is not taken for no-data. A file with no valid pixel at all is an error ("this source has nothing for this area"). Output: `imagery/<id>.tif`. Above 16 000 pixels per side, error, with a suggestion to use a coarser source.

**STAC.** Search with the map window's box as `intersects`, in the date range; default: the last two complete years, in the local summer (May to September north of the equator, November to March south). Sort by `eo:cloud_cover`, and add `s2:snow_ice_percentage` if Task 6's check shows the catalogue provides it. Greedily take scenes until a lattice of 33 × 33 points over the map lies inside their `geometry`, at most `MAX_SCENES` (6), skipping scenes above 20 % cloud. If the area is still not covered, keep what there is and warn with the share. Each scene: the asset's URL must have a host in `asset_hosts` and be https, or the scene is rejected. Download to a temp file in `imagery/`, crop to the map's window with `ImageryRaster`'s reader, write `imagery/<id>-<scene id>.tif`, and delete the download. Scene ids go through the same check as archive entry names before they are part of a path. `{year}` in the attribution comes from the scene's date. Scenes of several dates are normal: Task 3 merges them.

Re-fetching an entry overwrites its file and does not add a second entry to `naves.toml`. Nothing replaces an existing file until the new one is whole.

- [ ] **Step 1: Choose the HTTP crate.** Review `ureq` 3 per `AGENTS.md` (*Dependencies*): source, RustSec, TLS stack and its licences against `deny.toml`, proxy handling, response size limits, Windows. Alternatives to weigh: `reqwest` (blocking; much larger tree), `attohttpc`. Do the same for `tiff`'s encoder and for `image`'s `jpeg` feature. Record the results in `docs/research/2026-11-04-dependency-reviews.md` and the commit message; `cargo add`, `make deny`. If nothing passes, ask the maintainer before going on.
- [ ] **Step 2: Write the failing tests,** with a fake `Http` that serves canned bytes and records URLs.
  - GeoTIFF: `written_file_reads_back_with_geotiff_reader` (position and colours of a marker, with `ImageryRaster::open`).
  - WMS: `request_uses_lat_lon_order`; `tiles_cover_the_window_without_gap_or_overlap`; `max_px_is_respected`; `jpeg_answer_to_a_png_request_is_read`; `blank_pixels_are_no_data` (transparent, and a configured white); `black_stays_valid`; `service_error_xml_is_an_error_naming_the_source` (the NAIP failure in *Findings*); `empty_area_is_an_error`; `oversized_answer_is_cut_off`.
  - STAC: `search_uses_area_dates_and_order`; `clearest_scenes_are_taken_until_covered`; `stops_at_max_scenes_and_warns`; `southern_map_defaults_to_southern_summer`; `foreign_asset_host_is_rejected`; `hostile_scene_id_is_rejected` (`../../x`); `damaged_json_is_an_error`; `attribution_gets_the_year_of_the_scene`.
  - Project: `fetched_files_are_listed_with_licence` (including `non_commercial`); `fetching_twice_lists_once`; `failed_fetch_leaves_the_project_and_old_file_alone`; `project_stays_portable` (paths relative).
  - Then: `fetched_project_builds_offline`: a fake fetch, then a build with the fake tools. `build` takes no `Http`, so its signature proves it cannot fetch.
  - CLI `imagery_fetch_unknown_id_lists_the_ids`. MCP `imagery_fetch_takes_no_url`. Fuzz seeds: a real STAC answer reduced to two features, and two damaged copies.
  - `#[ignore = "needs network"]`: `real_sentinel_search`, `real_wms_tile`.
- [ ] **Step 3: Run them to see them fail:** `cargo test -p naves-export --test imagery_fetch`.
- [ ] **Step 4: Implement.**
- [ ] **Step 5: Run** `cargo test --workspace && make fuzz`, and `cargo test -p naves-export -- --ignored real_` once by hand. Expected: PASS.
- [ ] **Step 6: Commit** `feat: fetch imagery into the project`, with the dependency reviews in the message.

### Task 8: Choosing in the GUI

Start with the `/frontend-design:frontend-design` skill (`AGENTS.md`, *User interface*).

**Files:**
- Create: `crates/naves/src/gui/imagery.rs`
- Modify: `crates/naves/src/gui/mod.rs`, `crates/naves/src/gui/worker.rs`, `crates/naves/src/gui/tests.rs`, `docs/manual/src/gui.md`, `CHANGELOG.md`

**Interfaces:**
- Consumes: `catalog`, `options_for`, `fetch`, `RealHttp`, `MergeReport`.
- Produces: an "Imagery" section in the project panel (level 0 of the editor spec: area, scales, sources).

The section shows, for the project's area, one row per option: name, how much of the map it covers, pixel size, licence, and the non-commercial mark in words, not only as a colour. Fetch runs in a background job like `BuildJob`, with progress and cancel; cancelling leaves no partial file. Below it, the project's files in their order (the order decides among equally sharp images) with up and down, remove, and "add own file…", where a file needs attribution and licence before it is accepted. After each change the Imagery base map updates and the merge's report is shown per file: coverage, overlap, the colour correction applied, and its warnings. Removing an entry from the list does not delete the file, and the GUI says so.

- [ ] **Step 1: Write the failing tests** on the view-model. `options_follow_the_area`; `non_commercial_mark_is_text`; `fetch_job_reports_then_lists_the_file` (fake `Http`); `cancel_leaves_no_file`; `order_buttons_change_the_project`; `own_file_needs_licence_and_attribution`; `report_is_shown_per_file`; `one_job_at_a_time`.
- [ ] **Step 2: Run them to see them fail:** `cargo test -p naves`.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p naves`. Expected: PASS. Run the GUI on the example, as the maintainer would: list, fetch, reorder.
- [ ] **Step 5: Commit** `feat(gui): choose and fetch imagery for the project`.

### Task 9: Real areas

**Files:**
- Modify: `examples/berchtesgaden/naves.toml`, `examples/berchtesgaden/README.md`, `docs/research/2026-11-05-reference-terrains.md`, `crates/naves-core/src/imagery/merge.rs` and tests if a value changes, `docs/superpowers/plans/2026-11-04-naves-m1-walkable-terrain.md` (item 7 is done)

- [ ] **Step 1: One source, whole area.** Fresh copy of the example: `naves imagery list`, `naves imagery fetch sentinel2-scenes`, build. Compare `render_topdown` `imagery` with `surface` over the whole map. Expected: lake, town and rock line up with the land cover and OSM within a pixel or two.
- [ ] **Step 2: Overlap, the case the merge is for.** Add `bayern-dop40` over the Sentinel-2 base: aerial photos on the German side, Sentinel-2 on the Austrian side. Render the border at full size. Expected: no visible colour step or line across the border and between flights at the distance a player sees from a mountain. The sharpness change itself remains visible up close. Record `MergeReport`. If a step is visible, tune with a failing test first (cell size, `CONTENT_CHANGE`, `FEATHER_CELLS`), and record the before and after.
- [ ] **Step 3: Non-commercial.** Build with `eox-cloudless-2025` instead. Expected: `ATTRIBUTION.txt` in mod and mission opens with the notice, `mod.cpp` says it, the CLI warns, the GUI shows it, and the DayZ launcher shows the mod's `overview`.
- [ ] **Step 4: Another part of the world.** One small project each outside Europe and across a UTM zone border, for example the Andes and around 6° E or 12° E: list, fetch, build without the game, render. Expected: at least the worldwide options offered, and no seam at the zone border. Record what was offered.
- [ ] **Step 5: In the game.** Patched test mission, the fixed views of Task 4, for the merged map of Step 2. Expected: no crash; player connects; the border and the flight seams do not stand out from the air; tiles have no seams. Record screenshots' findings and the control run.
- [ ] **Step 6: Measure.** Build time, peak memory, PBO size against the land-cover build, at `terrain_size` 1024 and 2048. Record them in the README. If the build is more than twice as slow or the PBO too large to be practical, report to the maintainer before tuning further.
- [ ] **Step 7: Run** `TMPDIR=/var/tmp make check`. Expected: PASS. **Commit** `docs: Berchtesgaden from real imagery`. Push; CI, CodeQL and Codecov green. Remove the mod symlink.

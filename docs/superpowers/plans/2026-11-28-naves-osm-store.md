<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The project's own OSM copy and the feature table — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `naves osm import` cuts the map's area out of the extracts into `osm/copy.osm.pbf`; every build reads only that copy; one table classifies OSM elements into feature kinds; place names reach the in-game map.

**Architecture:** `naves-formats` gets the OSM element model and a PBF writer (pure format code). `naves-core` reads a PBF whole into `OsmData`, selects the elements of an import window from extracts by streaming passes, holds the feature table (`osmkind`), and derives today's `OsmWays` and `OsmLand` from `OsmData` instead of from files. `naves-export` owns the project-level import (`osm/copy.toml`, extend, refresh), the build's coverage check, the report and the world config's names. The binary adds the CLI command, MCP tools and a GUI button.

**Tech Stack:** Rust 1.99, edition 2024; `osmpbf` 0.3.8 (reading, in use); a PBF writer crate after review or our own writer on `flate2` (already in `Cargo.lock`); `sha2` (already in `Cargo.lock`) for source hashes; `toml`, `serde`.

**Spec:** `docs/superpowers/specs/2026-11-28-osm-store-design.md` (read it with this plan; section numbers below are the spec's). Related: `docs/superpowers/specs/2026-11-27-editing-design.md` (edits and fingerprints).

## Coordination with the agent chat (`agent-chat` branch, being implemented)

The agent chat (`docs/superpowers/specs/2026-11-28-agent-chat-design.md`, plan
`docs/superpowers/plans/2026-11-28-naves-agent-chat-phase1.md`, worktree `/var/tmp/naves-agent-chat/wt`) changes code this
plan touches. Rules so the two do not collide:

- **Start this plan's branch from `main` after `agent-chat` phase 1 is merged.** Phase 1 turns `crates/naves/src/mcp.rs`
  into `crates/naves/src/mcp/` (`mod.rs` tool table, `host.rs` with the `Host` seam and `StandaloneHost`, `roads.rs`).
  Tasks 1 to 8 do not touch `crates/naves/`; if phase 1 is not merged when Task 9 starts, Task 9 waits.
- **Signatures the agent chat calls stay as they are:** `naves_export::generate::read_source(project_dir, project)`
  (the host's source cache, `host.rs`), `naves_core::surface::project_surfaces(..)` and `LoadedSources::open(..)`
  (the host's map data and the GUI), `naves_export::generate::Source` and its fields, `naves_core::osm::{OsmWay,
  OsmWays}` (used by `mcp/roads.rs` and `naves_core::edits::lines`). They load the copy **inside**; the callers do not
  change. Their errors gain the two copy errors of Task 7, which reach MCP clients as the tool error text.
- **New MCP tools go through the host**, as the agent-chat design requires ("A later feature adds its tools here …
  with a `Host` method each"): Task 9 adds `Host` methods, implements them in `StandaloneHost`, and in `EditorHost` if
  agent-chat phase 2 has landed by then; if phase 2 lands later, its plan must implement them (note this in the
  agent-chat ledger when Task 9 merges).
- **Cache invalidation:** `StandaloneHost` re-reads the source only when the project differs. An OSM import changes the
  copy, not `naves.toml`, so the import tool must clear the host's source and map caches, and the cache key includes
  `osm/copy.toml`'s content (an import from the CLI while the server runs must be seen too).
- **The GUI button (Task 9, Step 4) comes last** and is rebased onto whatever agent-chat phases 2 and 4 changed in
  `crates/naves/src/gui/` (request queue, settings tab).
- Shared text files (`CHANGELOG.md`, `docs/manual/src/mcp.md`, `fuzz/Cargo.toml`, `crates/naves-core/tests/fuzz_seeds.rs`,
  `crates/naves/tests/mcp.rs` with its `lists_tools` list) get merge conflicts only in lists; resolve by keeping both.

## Global Constraints

- `AGENTS.md` applies to every task: tests first (red, then green), coverage ≥ 85 %, `make check` before every commit, `clippy -D warnings` with pedantic, `#[expect(..., reason = "...")]` not `#[allow]`, no `unwrap`/`expect`/`panic!`/indexing on untrusted data in library code, checked arithmetic and `TryFrom` for sizes, no allocation from a length field before checking it, `thiserror` in libraries, `anyhow` only in the binary, `tracing` not `println!` in libraries, atomic writes, `BTreeMap`/sorted output, user docs and the `CHANGELOG.md` line in the same commit, SPDX header on every new file.
- **Look for a crate first, review it before use**; the commit that adds a crate (or decides against all of them) names the crates considered and the review result (`AGENTS.md`, *Dependencies*). Run `make deny` after any `Cargo.toml` change.
- Every parser of untrusted input gets a fuzz target whose seeds run in `make test` (`crates/naves-core/tests/fuzz_seeds.rs`).
- No game data and no real extract in the repo; fixtures are the openly licensed crops in `tests/fixtures/osm/` (attribution in `tests/fixtures/README.md`) or synthetic data written by the writer of Task 1.
- `osmpbf` can panic on damaged input: every read goes through `osm::guarded` (`catch_unwind`), as today.
- Large data under `/var/tmp`, never `/tmp` (a tmpfs) or `$HOME`; tests write only under a `tempfile::TempDir`; CLI tests set `HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `XDG_DATA_HOME` to temp dirs. Builds: `CARGO_BUILD_JOBS=12`, wrapped in `systemd-run --user --scope -p MemoryMax=24G -p CPUQuota=1200%`.
- Verbatim from the spec: "An element already in the copy is never replaced, even if the extract has a newer version." "The build stops … It does not quietly read the PBFs." Changeset, user and timestamp of elements are dropped.
- The copy is byte-for-byte reproducible: same extracts and windows → same bytes.
- Ids: OSM ids stay plain integers below `USER_ID_BASE = 2^40` (editing design); the copy keeps ids, tags and node lists exactly.
- The margin is **1 km real**, converted to map metres with `scale.horizontal` and passed to `MapGeo::window` (which takes map metres).
- Duplicate ids across extracts: resolved **independent of file order** (sort by id, then content, keep the first), as `osm::read_sources` does today.
- UI work (Task 9's GUI button) starts by invoking the `/frontend-design:frontend-design` skill (`AGENTS.md`, *User interface*); the GUI is dark by default and colourblind-safe.

## Review Focus

1. **A country-sized extract** (Austria, 815 MB) must not be held in memory during import: only ids and the kept elements are. Task 3 test `import_keeps_only_selected_elements` checks the memory bound by counting what is held (no element outside the selection is stored), and the Task 10 run reports peak RSS.
2. **A damaged or truncated copy** (`osm/copy.osm.pbf` half written, or edited by hand) must give an error that names the file, never a panic and never a silent empty map. Task 2 test `damaged_copy_is_an_error_naming_the_file`.
3. **`copy.toml` that disagrees with the PBF** (a crash between the two renames, or a hand edit) must stop the build with a clear message. Task 4 test `manifest_without_pbf_is_an_error` and `pbf_without_manifest_is_an_error`.
4. **A project moved so that the map lies half outside the imported windows**: the build stops and names `naves osm import`; the GUI shows the same message instead of an empty map. Task 7 test `build_stops_when_area_leaves_the_copy`.
5. **A map without OSM** (`[sources] osm = []`) and a project from before this change (extracts listed, no copy): the first builds as today, the second stops with the import hint. Task 7 tests `map_without_osm_builds_without_copy`, `old_project_needs_one_import`.

---

### Task 1: OSM element model and PBF writer (`naves-formats`)

**Files:**
- Create: `crates/naves-formats/src/osm.rs` (model), `crates/naves-formats/src/osm_pbf.rs` (writer)
- Modify: `crates/naves-formats/src/lib.rs`, `crates/naves-formats/Cargo.toml`
- Test: `crates/naves-formats/tests/osm_pbf.rs`; fuzz target `fuzz/fuzz_targets/osm_copy_roundtrip.rs`

**Interfaces:**
- Produces:
  - `pub struct Node { pub id: i64, pub lat: f64, pub lon: f64, pub tags: BTreeMap<String, String> }`
  - `pub struct Way { pub id: i64, pub nodes: Vec<i64>, pub tags: BTreeMap<String, String> }`
  - `pub enum MemberType { Node, Way, Relation }`; `pub struct Member { pub kind: MemberType, pub id: i64, pub role: String }`
  - `pub struct Relation { pub id: i64, pub members: Vec<Member>, pub tags: BTreeMap<String, String> }`
  - `pub struct OsmData { pub nodes: BTreeMap<i64, Node>, pub ways: BTreeMap<i64, Way>, pub relations: BTreeMap<i64, Relation> }` with `OsmData::merge_missing(&mut self, other: OsmData)` (adds only ids not present; returns `(added_nodes, added_ways, added_relations)` as `(usize, usize, usize)`)
  - `pub struct PbfHeader { pub replication_timestamp: Option<i64>, pub bbox: Option<(f64, f64, f64, f64)> }` (bbox as south, west, north, east)
  - `pub fn write_pbf<W: Write>(data: &OsmData, header: &PbfHeader, out: W) -> Result<(), PbfWriteError>`
  - `PbfWriteError` (`thiserror`): `Io`, `TooLarge { what: &'static str }` (string table or block over the format's limits)

- [ ] **Step 1: Crate review.** Review `osm-io`, `pbf-craft`, `osm-pbf`, `vadeen_osm` (and anything newer on crates.io/lib.rs) against `AGENTS.md` *Dependencies*: source read, RustSec, maintenance, panics on input, deterministic output (block order, no timestamps), Windows build, licence in `deny.toml`, dependency tree. Write the result into the commit message of Step 6. If one passes, `write_pbf` wraps it; if none does, write our own encoder for exactly what we write: `OSMHeader` blob, `OSMData` blobs with `PrimitiveGroup`s of `DenseNodes`, `Way`s, `Relation`s, a `StringTable` per block, zlib via `flate2` (fixed level 6), blocks of at most 8000 elements, element types in separate groups, nodes → ways → relations, each by id. Format sources: OSM wiki "PBF Format", `fileformat.proto`, `osmformat.proto`.

- [ ] **Step 2: Write the failing tests** in `crates/naves-formats/tests/osm_pbf.rs`. Read back with `osmpbf` (dev-dependency of `naves-formats`, same version as `naves-core`):
  - `round_trip_keeps_every_element`: an `OsmData` with 3 nodes (one at lat −33.9, lon 18.4; tags with UTF-8 `name = "Schönau am Königssee"`), 2 ways, 1 relation with a node, a way and a relation member (roles `outer`, `inner`, `""`); written and read back: same ids, coordinates within 1e-7°, same tags, node lists, members and roles.
  - `output_is_reproducible`: two writes of the same data give identical bytes.
  - `header_carries_the_replication_timestamp`: `PbfHeader { replication_timestamp: Some(1_793_391_663), .. }` is read back by `osmpbf::BlobReader` as the `HeaderBlock`'s `osmosis_replication_timestamp`.
  - `empty_data_is_a_valid_file`: an empty `OsmData` writes a file `osmpbf` reads without error and without elements.
  - `large_input_spans_blocks`: 20 000 nodes → read back complete and in id order.
- [ ] **Step 3: Run them to see them fail** — `cargo test -p naves-formats --test osm_pbf`. Expected: FAIL (unresolved module `osm`).
- [ ] **Step 4: Implement** the model and `write_pbf` (crate wrapper or own encoder per Step 1). Coordinates are stored at the default granularity (100 nanodegrees); a coordinate that does not fit is `PbfWriteError::TooLarge`.
- [ ] **Step 5: Add the fuzz target** `osm_copy_roundtrip`: build an `OsmData` from the fuzz bytes (bounded: at most 1000 elements, tags of at most 64 bytes), write it, read it back with `osmpbf`, assert equality. Add a seed to `fuzz/corpus/osm_copy_roundtrip/` and the seed run to `fuzz_seeds.rs`. Run `cargo test -p naves-formats` and `cargo test -p naves-core --test fuzz_seeds`. Expected: PASS.
- [ ] **Step 6: Commit** `feat(formats): OSM element model and a reproducible PBF writer` (message: crates considered, review result, why this choice).

### Task 2: Read a whole PBF into `OsmData` (`naves-core`)

**Files:**
- Create: `crates/naves-core/src/osmdata.rs`
- Modify: `crates/naves-core/src/lib.rs`, `crates/naves-core/src/osm.rs` (make `guarded` `pub(crate)`)
- Test: `crates/naves-core/tests/osmdata.rs`; fuzz target `fuzz/fuzz_targets/osm_copy_read.rs`

**Interfaces:**
- Consumes: Task 1 model and `write_pbf` (tests write their inputs with it).
- Produces:
  - `pub fn read_copy(path: &Path) -> Result<(OsmData, PbfHeader), OsmError>` (errors wrapped in `OsmError::File { path, .. }`)
  - `pub fn read_copy_from_bytes(bytes: &[u8]) -> Result<(OsmData, PbfHeader), OsmError>`
  - `pub fn read_header(path: &Path) -> Result<PbfHeader, OsmError>` (one blob read: replication timestamp, bbox)

- [ ] **Step 1: Write the failing tests**: `copy_round_trips_through_osmdata` (write with Task 1, read with `read_copy`, equal); `koenigssee_fixture_reads_whole` (`tests/fixtures/osm/koenigssee.osm.pbf`: node, way and relation counts equal those `osmium fileinfo -e` prints, written into the test as constants by the implementer from one run); `damaged_copy_is_an_error_naming_the_file` (the fixture truncated to half in a `TempDir`: `Err(OsmError::File { path, .. })` with that path, no panic); `not_a_pbf_is_an_error`; `header_of_an_extract_without_timestamp_is_none`.
- [ ] **Step 2: Run** `cargo test -p naves-core --test osmdata`. Expected: FAIL.
- [ ] **Step 3: Implement** with `osmpbf::ElementReader` under `osm::guarded`; element changeset/user/timestamp are not read. A node referenced by a way but absent is not an error here (ways keep their node lists exactly).
- [ ] **Step 4: Fuzz target** `osm_copy_read` (as `osm_read`: silence the panic hook, call `read_copy_from_bytes`), seeds in `fuzz/corpus/osm_copy_read/` (two fixtures), seed run in `fuzz_seeds.rs`. Run `cargo test -p naves-core`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): read a PBF whole into OsmData`.

### Task 3: Select the elements of an import window from extracts

**Files:**
- Create: `crates/naves-core/src/osmcopy.rs`
- Modify: `crates/naves-core/src/lib.rs`
- Test: `crates/naves-core/tests/osmcopy.rs`

**Interfaces:**
- Consumes: Tasks 1, 2; `landcover::GeoWindow`.
- Produces:
  - `pub struct Selection { pub data: OsmData, pub stats: SelectStats }`; `pub struct SelectStats { pub nodes: usize, pub ways: usize, pub relations: usize, pub passes: usize }`
  - `pub fn select(paths: &[PathBuf], window: GeoWindow, skip: &OsmData) -> Result<Selection, OsmError>`: streams the files; returns every element the spec's rules 1–5 (5.1) keep for `window`, **except ids already in `skip`** (the extend case; pass `&OsmData::default()` for a first import or a refresh).

- [ ] **Step 1: Write the failing tests** (inputs written with Task 1's writer into a `TempDir`; window 47.0–47.1 N, 12.0–12.1 E):
  - `node_inside_is_kept`, `node_outside_unreferenced_is_dropped`;
  - `way_with_one_node_inside_keeps_all_its_nodes` (complete ways: the outside nodes are present);
  - `multipolygon_with_one_member_inside_is_complete` (all member ways and their nodes);
  - `area_enclosing_the_window_is_kept` (a closed `landuse=forest` way whose four nodes all lie outside but whose bbox contains the window; and the same as a `type=multipolygon` relation) — rule 4;
  - `route_relation_keeps_member_list_but_not_far_members` (`type=route` with one member way inside and one far away: relation kept with both members listed, far way absent);
  - `boundary_relation_is_not_completed` (`type=boundary` treated like a route);
  - `skip_excludes_known_ids_and_never_replaces` (an id in `skip` is absent from the selection even when the extract's version differs);
  - `duplicate_ids_across_files_resolve_independent_of_order` (same id, different tags in two files: `select([a,b])` == `select([b,a])`);
  - `import_keeps_only_selected_elements` (an extract with 10 000 far nodes and ways: `stats` and `data` contain none of them).
- [ ] **Step 2: Run** `cargo test -p naves-core --test osmcopy`. Expected: FAIL.
- [ ] **Step 3: Implement `select`.** Passes (each over every file, `par_map_reduce` as in `osm.rs`, ids collected into `BTreeSet`s): (1) ids of nodes inside the window; (2) ways using those nodes, plus area ways (closed ways with an area tag, as `ways_of` decides today) whose node bbox overlaps the window → needs node positions: do pass (2a) area-way node lists, (2b) positions of those nodes, then keep the overlapping ones; (3) relations with a kept member, plus `type=multipolygon` relations whose member ways' bbox overlaps the window; (4) member ways of kept multipolygon relations; (5) every node of every kept way. Elements are deduplicated per id by the order-independent rule. Progress goes through `tracing::info!` per pass.
- [ ] **Step 4: Run** `cargo test -p naves-core`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): select an import window from OSM extracts`.

### Task 4: The project's copy: `osm/copy.osm.pbf`, `osm/copy.toml`, import, extend, refresh

The reading side (manifest, `load_copy`, `covers`, `CopyError`) lives in `naves-core::osmcopy`, because
`naves-core::surface` must load the copy (Task 6); the writing side (`import`) lives in `naves-export`.

**Files:**
- Modify: `crates/naves-core/src/osmcopy.rs` (manifest types, `load_copy`, `covers`, `CopyError`)
- Create: `crates/naves-export/src/osm_import.rs` (`import`, `import_window`, `OsmImportError`)
- Modify: `crates/naves-export/src/lib.rs`, `crates/naves-export/Cargo.toml` (`sha2` with `cargo add`, review noted in the commit), `docs/manual/src/project-file.md` (the `osm/` folder, `copy.toml` fields)
- Test: `crates/naves-export/tests/osm_import.rs`; fuzz target `fuzz/fuzz_targets/osm_manifest_parse.rs`

**Interfaces:**
- Consumes: Task 3 `select`; Task 2 `read_copy`, `read_header`; Task 1 `write_pbf`; `MapGeo::window(margin_map_m)`.
- Produces:
  - `pub const COPY_PBF: &str = "osm/copy.osm.pbf"`; `pub const COPY_MANIFEST: &str = "osm/copy.toml"`
  - `pub struct Manifest { pub format: u32, pub import: Vec<ImportRecord> }`; `pub struct ImportRecord { pub window: GeoWindow, pub sources: Vec<SourceRecord>, pub added: Added, pub refresh: bool }`; `pub struct SourceRecord { pub path: PathBuf, pub replication_timestamp: Option<String>, pub modified: Option<String>, pub sha256: String }`; `pub struct Added { pub nodes: usize, pub ways: usize, pub relations: usize }` (serde, field names as in spec 5.2; `format = 1`)
  - `pub fn import_window(project: &Project) -> Result<GeoWindow, OsmImportError>` (map window + 1 km real margin: `MapGeo::window(1000.0 * project.scale.horizontal)`)
  - in `naves_core::osmcopy`: `pub fn load_copy(project_dir: &Path) -> Result<Option<(Arc<OsmData>, Manifest)>, CopyError>` (`None` when neither file exists); `CopyError` (`thiserror`): `Damaged { path, reason }` (manifest without PBF or the reverse, unparsable TOML), `Osm(OsmError)`, `Io { path, source }`; later (Task 7) `Missing` and `NotCovering { missing: GeoWindow }`
  - `pub fn import(project_dir: &Path, project: &Project, refresh: bool) -> Result<ImportOutcome, OsmImportError>`; `pub enum ImportOutcome { Created(Added), Extended(Added), UpToDate, Refreshed { added: Added, changed_edits: Vec<String> } }`
  - in `naves_core::osmcopy`: `pub fn covers(manifest: &Manifest, window: GeoWindow) -> bool` (the union of import windows contains `window`; windows are axis-aligned boxes; containment of a box in a union of boxes, checked exactly by splitting on the boxes' edges)
  - in `naves_export::osm_import`: `OsmImportError` (`thiserror`): `NoSources`, `SourceMissing { path }`, `SourceChanged { path }` (sha256 differs from an earlier import, message names `--refresh`), `Copy(CopyError)`, `Osm(OsmError)`, `Projection(ProjectionError)`, `Io { path, source }`

- [ ] **Step 1: Write the failing tests** (a project in a `TempDir` with synthetic extracts from Task 1; `HOME` etc. not touched):
  - `first_import_writes_both_files` (`Created`, both files exist, manifest has one record with the source's sha256);
  - `second_import_of_same_area_is_up_to_date`;
  - `moved_area_extends_and_keeps_old_elements_byte_identical` (move the centre by 2 km; the extract now has a newer version of a way already in the copy: after `Extended`, that way's tags are the old ones; new elements present; two records in the manifest);
  - `extend_stops_when_a_source_changed` (`SourceChanged` naming the file, copy untouched);
  - `refresh_replaces_and_lists_changed_edits` (an `edits/ways.toml` `modify` of a way whose tags changed: its id is in `changed_edits`; fingerprint via `edits::ways::fingerprint` over the filtered tags, see Task 6);
  - `copy_is_reproducible` (two projects, same inputs → identical `copy.osm.pbf` bytes and identical `copy.toml`);
  - `manifest_without_pbf_is_an_error`, `pbf_without_manifest_is_an_error` (`CopyError::Damaged` from `load_copy`);
  - `replication_timestamp_is_recorded` (header with timestamp → RFC 3339 string; without → `modified` from the file's mtime and `replication_timestamp = None`);
  - `covers_union_of_windows` (two adjacent windows cover a window across their seam; a gap is not covered).
- [ ] **Step 2: Run** `cargo test -p naves-export --test osm_import`. Expected: FAIL.
- [ ] **Step 3: Implement.** Write order: PBF to a temp file in `osm/`, rename; then manifest to a temp file, rename (spec 5.2). Extend: `select(paths, window, &existing)` then `existing.merge_missing(selection.data)`; only if the new window is not covered. Refresh: `select(.., &OsmData::default())`, replace. Sources are the `[sources] osm` paths, resolved against the project dir; a missing file is `SourceMissing`. Document `osm/` in `project-file.md` in the same commit.
- [ ] **Step 4: Fuzz target** `osm_manifest_parse` (TOML into `Manifest`), seeds, seed run. Run `cargo test -p naves-export` and `cargo test -p naves-core --test fuzz_seeds`. Expected: PASS. Run `make deny`.
- [ ] **Step 5: Commit** `feat(export): the project's own OSM copy with import, extend and refresh`.

### Task 5: The feature table (`osmkind`)

**Files:**
- Create: `crates/naves-core/src/osmkind.rs`
- Modify: `crates/naves-core/src/lib.rs`
- Test: `crates/naves-core/tests/osmkind.rs`

**Interfaces:**
- Consumes: Task 1 model.
- Produces:
  - `pub enum Shape { Node, Way, ClosedWay, AreaRelation, OtherRelation }`
  - `pub enum Kind { Road, Path, Lot, Railway, RailPoint, Platform, Waterway, WaterPoint, WaterArea, Ground, Piste, Woodland, TreeLine, Tree, Building, Settlement, Place, Barrier, Power, Aerialway, PointObject }` (`Ord`, `Copy`, with `pub fn id(self) -> &'static str` giving snake_case ids for the report: `road`, `rail_point`, …)
  - `pub fn kinds(tags: &BTreeMap<String, String>, shape: Shape) -> Vec<Kind>` (sorted, deduplicated)
  - `pub fn index(data: &OsmData) -> BTreeMap<Kind, Vec<ElementRef>>`; `pub enum ElementRef { Node(i64), Way(i64), Relation(i64) }` (ids in order)

- [ ] **Step 1: Write the failing tests:** one test per row of the spec's table (5.4) with its main tag(s) and shape, asserting the kind — for example `railway_rail_way_is_railway`, `level_crossing_node_is_rail_point`, `platform_way_is_platform`, `stream_way_is_waterway`, `ford_node_is_water_point`, `lake_area_is_water_area`, `meadow_area_is_ground`, `leisure_park_is_ground`, `piste_way_is_piste`, `forest_area_is_woodland`, `hedge_way_is_tree_line`, `tree_node_is_tree`, `building_yes_is_building`, `building_no_is_nothing`, `military_area_is_settlement`, `village_node_with_name_is_place`, `village_node_without_name_is_nothing`, `fence_is_barrier`, `power_tower_is_power`, `aerialway_line_is_aerialway`, `bench_node_is_point_object`; plus `highway_kinds_match_features_classify` (for every highway value the existing `features::classify` accepts, `kinds` returns `Road` or `Path` exactly as `classify` decides) and `lot_kinds_match_lot_tags` (agrees with `osm::lot_tags_of`); `one_element_can_have_two_kinds` (a closed way `landuse=meadow` + `piste:type=downhill` → `[Ground, Piste]`; a node `railway=level_crossing` → `[RailPoint]`; a closed way `natural=wood` + `leaf_type=mixed` → `[Woodland]`, one kind); `unknown_tags_have_no_kind`.
- [ ] **Step 2: Run** `cargo test -p naves-core --test osmkind`. Expected: FAIL.
- [ ] **Step 3: Implement** as a `match` per key (the style of `features::classify`); `Road`/`Path` delegate to the existing classification so there is one source of truth. Attributes are not copied: generators read tags from `OsmData`.
- [ ] **Step 4: Run** `cargo test -p naves-core`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): the OSM feature table`.

### Task 6: Derive ways and land from `OsmData`; switch the readers

**Files:**
- Modify: `crates/naves-core/src/osm.rs` (new `ways_from`, `land_from`; the path readers are removed), `crates/naves-core/src/surface.rs` (`LoadedSources::open` loads the copy itself; signature unchanged), `crates/naves-core/src/edits/ways.rs` (`pub const FINGERPRINT_KEYS: [&str; 14] = osm::WAY_TAGS`, doc says why), `crates/naves-export/src/generate.rs` (`read_source` loads the copy itself; signature unchanged), `crates/naves-export/examples/road-sites.rs`, `road-analysis.rs`, the fuzz targets `osm_read.rs`, `osm_ways.rs` (feed `read_copy_from_bytes` → `land_from`/`ways_from`)
- Test: `crates/naves-core/tests/osm.rs`, `features.rs`, `roadfit.rs`, `fuzz_seeds.rs` (moved onto the new functions)

**Interfaces:**
- Consumes: Task 2 `read_copy`; Task 4 `load_copy`.
- Produces:
  - `pub fn ways_from(data: &OsmData, window: GeoWindow) -> OsmWays` (same filtering as `read_ways` today: highway and `WATERWAYS` ways touching the window, tags filtered to `WAY_TAGS`, `skipped` counted the same way)
  - `pub fn land_from(data: &OsmData, window: GeoWindow) -> OsmLand` (same as `read_land` today)
  - `pub(crate) const WAY_TAGS` becomes `pub const WAY_TAGS`

- [ ] **Step 1: Write the failing equivalence tests** in `crates/naves-core/tests/osm.rs`: for every fixture in `tests/fixtures/osm/` (`koenigssee*`, `schoenau*`, `lots`, `lot_broken`, `water_only`, `water_broken`, `waterways`) and the windows the existing tests use: `ways_from(&read_copy(f)?.0, w) == read_ways(&[f], w)?` and `land_from(..) == read_land(..)`; and for the multi-file cases (`koenigssee_north` + `koenigssee_south`, `schoenau` + `schoenau_cut`): `ways_from(select([a,b], w')) == read_ways([a,b], w)` where `w'` is `w` grown by the margin. Also `edits_fingerprints_unchanged` (a `ways.toml` `modify` made against `read_ways` output still matches against `ways_from` output).
- [ ] **Step 2: Run** `cargo test -p naves-core --test osm`. Expected: FAIL (functions missing).
- [ ] **Step 3: Implement `ways_from` and `land_from`** on the in-memory maps (no passes); keep `assemble` and the classification code shared.
- [ ] **Step 4: Run** the equivalence tests. Expected: PASS.
- [ ] **Step 5: Switch the readers inside, not the callers.** `generate::read_source(project_dir, project)` and `surface::LoadedSources::open(..)` keep their signatures (the agent chat's host and the GUI call them, see *Coordination*). Inside, they get the data from `osmcopy::project_data(project_dir, project) -> Result<Option<Arc<OsmData>>, CopyError>` (in `naves-core::osmcopy`: `None` when `[sources] osm` is empty, else `load_copy`). **Only for this commit**, when there is no copy and `[sources] osm` is not empty, it returns `select(extracts, window, &OsmData::default())` (so everything behaves as before); Task 7 Step 3 removes that fallback. The build reads the copy once and passes it on internally (a `pub(crate)` variant `read_source_with(project_dir, project, data: &OsmData)`), so a build does not read the file twice. Move the existing tests in `osm.rs`, `features.rs`, `roadfit.rs` and both fuzz targets onto `read_copy` + `ways_from`/`land_from` with their current assertions unchanged; then delete `read_ways`, `read_land`, `read_ways_from_bytes`, `read_land_from_bytes` and the equivalence tests of Step 1 (they compared against the deleted functions; the moved tests keep the behaviour pinned). Run `cargo test --workspace --all-targets --locked`. Expected: PASS.
- [ ] **Step 6: Commit** `refactor(core): ways and land come from OsmData` (no user-visible change).

### Task 7: The build reads the copy: coverage check, notices, report, credits

**Files:**
- Modify: `crates/naves-core/src/osmcopy.rs` (`CopyError::Missing`, `CopyError::NotCovering { missing: GeoWindow }`), `crates/naves-core/src/surface.rs` (`SurfaceError::Copy`), `crates/naves-export/src/build.rs` (`BuildError::OsmCopy(CopyError)`, report, `attribution`), `crates/naves-export/src/generate.rs`, `crates/naves-core/src/validate.rs` (rules `osm_source_changed`, `osm_sources_changed`, severity information), `docs/manual/src/getting-started.md`, `troubleshooting.md` (the two errors and their fix), `CHANGELOG.md`
- Test: `crates/naves-export/tests/build_osm_copy.rs`

**Interfaces:**
- Consumes: Task 4 `load_copy`, `covers`, `Manifest`; Task 5 `index`, `Kind::id`; Task 6 readers.
- Produces: `report.json` key `osm` = `{ "kinds": { "<kind id>": { "count": n, "length_km": x?, "area_km2": y? } }, "imports": [ { "replication_timestamp": … } ] }`; the credits line "Map data: © OpenStreetMap contributors … (data of YYYY-MM-DD)" or "(data of YYYY-MM-DD to YYYY-MM-DD)".

- [ ] **Step 1: Write the failing tests** (fake tools as the existing build tests use, `crates/naves-export/src/testing.rs`):
  - `old_project_needs_one_import` (extracts listed, no copy → `BuildError::OsmCopy(CopyError::Missing)`, message contains `naves osm import`);
  - `build_stops_when_area_leaves_the_copy` (import, then move the centre 5 km → `CopyError::NotCovering`);
  - `read_source_and_surfaces_give_the_same_error` (`read_source` and `project_surfaces` on that project fail with the same message: what the agent chat's MCP host and the GUI show);
  - `map_without_osm_builds_without_copy` (`osm = []`);
  - `build_reads_the_copy_not_the_extracts` (import, then delete the extract files → build succeeds);
  - `changed_extract_is_reported_not_used` (import, then rewrite an extract with an extra road → build succeeds, the road is absent, `osm_source_changed` in the report);
  - `report_counts_kinds` (a synthetic copy with 2 rail ways and 1 meadow → `osm.kinds.railway.count == 2`, `ground.area_km2 > 0`);
  - `credits_name_the_data_date`.
- [ ] **Step 2: Run** `cargo test -p naves-export --test build_osm_copy`. Expected: FAIL.
- [ ] **Step 3: Implement**: remove Task 6's temporary fallback to the extracts (`osmcopy::project_data` now returns `CopyError::Missing` or `CopyError::NotCovering`, carried by `BuildError` and by the errors of `read_source` and `LoadedSources::open` (`SurfaceError` gains a `Copy(CopyError)` variant), so MCP clients and the GUI get the same message); load the copy once per build, check `covers(manifest, map window without margin)` inside `project_data`, hand `&OsmData` to `read_source_with` and the surfaces; compare each listed extract's current sha256 with the manifest's (only when the file exists) for `osm_source_changed`; compare the listed paths with the manifest's for `osm_sources_changed`. Kinds counted with `index` over elements that touch the map window; lengths and areas in map km / km².
- [ ] **Step 4: Run** `cargo test --workspace --all-targets --locked`. Expected: PASS.
- [ ] **Step 5: Docs and changelog**: getting-started (import before the first build), troubleshooting (both errors), `CHANGELOG.md` under `## [Unreleased]`: "Added: `naves osm import` keeps a copy of the OpenStreetMap data of the map's area in the project; builds read only that copy (run it once for existing projects)."
- [ ] **Step 6: Commit** `feat: builds read the project's own OSM copy`.

### Task 8: Place names on the map (`class Names`)

**Files:**
- Modify: `crates/naves-export/src/config.rs` (`world_config_with` takes `names: &[PlaceName]`), `crates/naves-export/src/build.rs` (collect names), `crates/naves-core/src/osmkind.rs` (`pub fn place_names(data: &OsmData, frame: &MapGeo, map_size_m: f64) -> Vec<PlaceName>`), `CHANGELOG.md`, `docs/manual/src/getting-started.md` (one sentence)
- Test: `crates/naves-export/tests/config.rs` (snapshot), `crates/naves-core/tests/osmkind.rs`

**Interfaces:**
- Produces: `pub struct PlaceName { pub osm_id: String, pub name: String, pub x: f64, pub y: f64, pub kind: NameKind }`; `pub enum NameKind { City, Village, Local }` with `config_type(self) -> &'static str`.

- [ ] **Step 1: Verify the type names**: derapify Livonia's `class Names` with dayzmake (`dayzmake extract --derapify <DayZ>/Addons/worlds_enoch.pbo /var/tmp/naves-names` and read `config.cpp`), note which `type` values exist (expected `NameCity`, `NameVillage`, `NameLocal`, `NameCityCapital`, …). Use what Livonia uses; record the values found in the commit message. No game data goes into the repo.
- [ ] **Step 2: Write the failing tests**: `village_node_becomes_a_name` (`place=village`, `name=Ramsau` at the map centre → one `PlaceName` with `kind Village`, position in map metres); `area_place_uses_its_centroid`; `places_outside_the_map_are_dropped`; `names_sorted_by_osm_id`; `place_without_name_is_skipped`; the config snapshot contains `class Name_<osm id>` entries with `name`, `position[]`, `type`, and `Center` uses the same scheme.
- [ ] **Step 3: Run** `cargo test -p naves-core --test osmkind` and `cargo test -p naves-export --test config`. Expected: FAIL.
- [ ] **Step 4: Implement.** Class names `Name_<n|w|r><id>` (ASCII; the `name` string keeps UTF-8). Mapping: `city` → City, `town` → City, `village` → Village, `hamlet`, `isolated_dwelling`, `farm`, `locality` → Local; others skipped.
- [ ] **Step 5: Run** `cargo test --workspace`. Expected: PASS. Then one in-game look (Task 10) decides whether umlauts show; if not, a follow-up transliterates (note it in the plan's ledger, not here).
- [ ] **Step 6: Commit** `feat(export): place names from OSM on the in-game map`.

### Task 9: CLI, MCP and GUI entry points

**Files:**
- Modify: `crates/naves/src/cli.rs` (`Command::Osm { action: OsmAction }`, `OsmAction::Import { dir: Option<PathBuf>, #[arg(long)] refresh: bool }`); `crates/naves/src/mcp/host.rs` (`Host::osm_import(&self, refresh: bool) -> Result<ImportOutcome, String>` which also clears the source and map caches; `Host::osm_data(&self) -> Result<Arc<OsmData>, String>` cached like `source`, keyed by the project and `osm/copy.toml`; implemented in `StandaloneHost`, and in `EditorHost` if it exists); `crates/naves/src/mcp/osm.rs` (new: argument structs, the JSON of a feature); `crates/naves/src/mcp/mod.rs` (tools `osm_import { refresh: bool }`, `osm_kinds`, `osm_features { kind: String, x0, y0, x1, y1: f64, page: Option<u32> }`, 200 features per page, rectangle checked like `roads_list`'s, points in map metres rounded to 0.01 m as `roads::way_json` does); `crates/naves/src/gui/` (an "Import OSM data" button next to the OSM sources in the project settings, running in the background with progress like the build, and the build's two copy errors shown as the build's error), `docs/manual/src/cli.md`, `mcp.md`, `gui.md`, `CHANGELOG.md`
- Test: `crates/naves/tests/cli_osm.rs` (`assert_cmd`, temp `HOME`/XDG dirs), `crates/naves/tests/mcp.rs` (existing harness)

**Interfaces:**
- Consumes: Task 4 `import`, `ImportOutcome`; Task 5 `index`, `Kind`.

- [ ] **Step 1: Write the failing tests**: `osm_import_creates_the_copy` (exit 0, prints "Imported … nodes, … ways, … relations"); `osm_import_again_is_up_to_date`; `osm_import_refresh_lists_changed_edits`; `osm_import_without_sources_fails` (non-zero, stderr names `sources.osm`); MCP `osm_kinds_lists_counts`, `osm_features_pages`, `osm_import_tool_runs`, `osm_import_refreshes_roads_list` (after `osm_import` of a copy with one more road, `roads_list` shows it without restarting the server), `cli_import_while_server_runs_is_seen` (the cache key follows `osm/copy.toml`), and `lists_tools` gains the three names.
- [ ] **Step 2: Run** `cargo test -p naves --test cli_osm --test mcp`. Expected: FAIL.
- [ ] **Step 3: Implement the CLI and MCP tools** (`spawn_blocking` for the import in MCP; no `std::sync::Mutex` across `.await`). Help texts and `cli.md` agree word for word on the options.
- [ ] **Step 4: GUI button**: invoke `/frontend-design:frontend-design` first; follow the existing settings panel style (dark default, colourblind-safe). No new GUI test harness; the existing GUI tests (if any cover the settings panel) gain a case that the button exists.
- [ ] **Step 5: Run** `make check` (with `TMPDIR=/var/tmp`). Expected: PASS, coverage ≥ 85 %.
- [ ] **Step 6: Commit** `feat: naves osm import, MCP osm tools and the GUI import button`.

### Task 10: The example, end to end

**Files:**
- Modify: `examples/berchtesgaden/` (add `osm/copy.osm.pbf` and `osm/copy.toml` — openly licensed OSM data, about 2 MB, ODbL attribution already in the example's README; check `REUSE.toml` covers them), `examples/berchtesgaden/README.md` (the import step)

- [ ] **Step 1: Baseline**: on `main` before this branch, build the example (`naves build examples/berchtesgaden`, terrain size 2048) and keep `build/report.json` and the surface mask under `/var/tmp/naves-osmstore/before/`.
- [ ] **Step 2: Import** on the branch: `naves osm import examples/berchtesgaden` under `/usr/bin/time -v` (peak RSS and time into the ledger); expect about 2 MB, the counts printed.
- [ ] **Step 3: Build** and compare with the baseline: `roads.*` numbers, `bridges`, `violations` and the surface mask must be **identical**; differences are bugs of Task 6 unless they come from rule 4 areas the old reader also had (then they are equal too) — investigate every difference before going on.
- [ ] **Step 4: In game** (one run, `tools/ingame-check/` or by hand per `AGENTS.md`): the 2D map shows the place names (Berchtesgaden, Schönau am Königssee, Ramsau …), umlauts readable; remove the DayZ install's mod symlink afterwards.
- [ ] **Step 5: Commit** `chore(example): the Berchtesgaden example keeps its own OSM copy`.

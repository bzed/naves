<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Agent chat, phase 1: the host seam and the road tools — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `naves mcp <dir>` gets a host seam and the road edit tools (`roads_list`, `road_add`, `road_modify`,
`road_delete`, `road_reset`) plus server instructions, so that an agent can read and edit the roads of a project
without the GUI.

**Architecture:** `crates/naves/src/mcp.rs` becomes the module `mcp/` with the tool table in `mod.rs` and a `Host`
enum in `host.rs`. Phase 1 has one variant, `StandaloneHost` (today's caches and file access, moved). Phase 2 adds
`EditorHost` as a second variant without touching the tools. How a point list in map metres becomes node ids lives
in `naves_core::edits::lines`, so both hosts (and the fuzz target) share it.

**Tech Stack:** Rust 1.99 (edition 2024), rmcp 3.9 (server, macros), tokio, `naves_core::edits`,
`naves_export::generate::{Source, read_source}`.

**Spec:** `docs/superpowers/specs/2026-11-28-agent-chat-design.md` (sections 5.1, 5.3, 5.8, 6, 7 phase 1, 8).
Phases 2 to 4 get their own plans after this one is merged.

## Global Constraints

- AGENTS.md in full: test first, no panics on untrusted input (tool arguments are untrusted), checked arithmetic and
  no truncating `as` casts, `#[expect(lint, reason = "…")]` instead of `#[allow]`, `thiserror` in libraries,
  `tracing` not `println!` in libraries, atomic writes, deterministic output (`BTreeMap`, id order).
- `crates/naves/tests/mcp.rs` passes **unchanged** after Task 1; later tasks change only `lists_tools` (new names) and
  add tests.
- Coordinates in and out of the new tools are **map metres from the south-west corner**, as `render_topdown` has
  them; conversion with the project's `MapGeo` (`Source::frame`).
- Edits go through `naves_core::edits` with `By::Ai`; standalone, the result is written with `WayEdits::save`
  (atomic, validated). No undo standalone.
- Limits (spec 5.3): tags at most `MAX_TAGS` = 64, key and value at most `MAX_TAG_LEN` = 255 characters, at most
  1 000 ways per `roads_list` answer with `more: true` when cut, finite coordinates inside `0..=map_size_m`. Points
  per way: **`MAX_WAY_NODES` = 2 000**, not the spec's 10 000 — the edit file cannot hold more than 2 000 nodes per
  way (OSM's own limit), so a higher tool limit would only move the error. Tell the maintainer in the hand-off.
- The new limit constants go into `naves_core::edits` next to the edit-file limits (`MAX_WAY_NODES` …), not into
  `naves_core::limits`, which holds road-geometry limits only (the spec's "where the edit files' limits are" means
  this place).
- A breach of a limit or an edit the edit API refuses is a **tool error** whose text names the limit or starts with
  "The edit cannot be made: " (as `EditSession::apply` words it); nothing is written then.
- Ids in and out of the road tools are **strings** in the edit-file notation: `"812"` for an OSM way, `"u3"` for a
  drawn one (`ElementId`'s `Display` / `FromStr`).
- Every commit that changes behaviour updates `docs/manual/src/mcp.md` and adds a line to `CHANGELOG.md` under
  `## [Unreleased]`.
- Builds and tests on this machine: `CARGO_BUILD_JOBS=12`, wrapped as
  `systemd-run --user --scope -p MemoryMax=24G -p CPUQuota=1200% <cmd>` (linux-basics skill; follow it).
  Temporary files only under `/tmp` or `/var/tmp`.

## Review Focus

1. **Stale roads after an edit:** `roads_stats`, `validation_list` and the road layers of `render_topdown` today
   cache the generated roads keyed on `naves.toml` only; after `road_delete` they must show the roads without that
   way. Test in Task 4 (`roads_follow_an_edit`).
2. **Coordinates echoed with rounding:** an agent copies a point from `roads_list` (rounded to 0.01 m) into
   `road_add`; the new way must join that node, not lie 4 mm beside it. Test in Task 2
   (`a_point_near_a_node_reuses_it`) and Task 4 (`road_add_joins_an_existing_node`).
3. **Bad or stale ids:** `"u999"`, `"0"`, `"abc"`, a way that is already deleted, an OSM id that is not in the
   extract — each a tool error, the edit file byte for byte unchanged. Test in Task 4
   (`road_tools_refuse_bad_ids_and_leave_the_file`).
4. **Parallel calls:** an agent fires several `road_add` at once; every way arrives with its own id, none is lost to
   a read-modify-write race. Test in Task 4 (`parallel_road_adds_all_arrive`).
5. **A damaged `edits/ways.toml`:** the road tools report it with the file's name and never overwrite it; tools
   that do not need the roads keep working. Test in Task 3 (`a_damaged_edit_file_is_reported`) and Task 4 (the
   same file stays unchanged after a refused `road_add`).

---

### Task 1: The host seam (refactor, no behaviour change)

**Files:**
- Delete: `crates/naves/src/mcp.rs`
- Create: `crates/naves/src/mcp/mod.rs` (the tool table, `NavesMcp`, `serve`, `build_json` and its unit tests)
- Create: `crates/naves/src/mcp/host.rs` (`Host`, `StandaloneHost`, `MapNeeds`, `MapData`, the caches)
- Test: `crates/naves/tests/mcp.rs` (unchanged)

**Interfaces:**
- Produces (in `crate::mcp::host`; all methods `async`, errors are the message text as today):
  - `pub enum Host { Standalone(StandaloneHost) }` — each method below is a `match` that forwards to the variant.
  - `pub struct StandaloneHost` with `pub fn new(dir: &Path, locate_tools: ToolLocator) -> Self`; holds what
    `NavesMcp` holds today (`dir`, `locate_tools`, the `Cached` map cache, the `writes` mutex).
  - `pub type ToolLocator = Arc<dyn Fn() -> Result<Tools, String> + Send + Sync>;` (moved)
  - `pub struct MapNeeds { pub surfaces: bool, pub imagery: bool, pub roads: bool }`
  - `pub struct MapData { pub project, pub heightmap, pub surfaces, pub imagery, pub roads }` (moved, fields `pub`)
  - `Host::project(&self) -> Result<Project, String>`
  - `Host::set_project_value(&self, path: String, value: serde_json::Value) -> Result<Project, String>`
  - `Host::map(&self, needs: MapNeeds) -> Result<MapData, String>`
  - `Host::fetch_imagery(&self, id: String, request: FetchRequest) -> Result<(Vec<ImageryFile>, Vec<String>), String>`
    (files and warning lines; `ImageryFile` is what `naves_export::imagery_fetch::fetch` returns)
  - `Host::build(&self) -> Result<(BuildOutput, Vec<String>), String>` (output and progress lines)
  - `NavesMcp::new(host: Host) -> Self`; `NavesMcp` keeps `host: Arc<Host>` and the `tool_router`.

- [ ] **Step 1: Move the code.** `git mv crates/naves/src/mcp.rs crates/naves/src/mcp/mod.rs`, then move the cache,
  `compute_heightmap`, `blocking` (as `pub(super)`), and the bodies of `project_get`/`project_set`/`map`/
  `imagery_fetch`/`build` that touch files into `host.rs` behind the methods above. The tools keep their
  descriptions, argument structs, JSON shapes and error texts exactly; `imagery_options` uses `host.project()` and
  keeps its catalogue code. `serve(dir)` builds `Host::Standalone(StandaloneHost::new(dir, locate))`.

- [ ] **Step 2: Run the existing tests**

  Run: `cargo test -p naves --locked --test mcp` and `cargo test -p naves --locked --lib mcp::`
  Expected: PASS, with no change to `crates/naves/tests/mcp.rs`.

- [ ] **Step 3: Lint**

  Run: `cargo clippy -p naves --all-targets --locked -- -D warnings`
  Expected: no warnings. (A single-variant enum is fine; if clippy objects, `#[expect(…, reason = "EditorHost comes in phase 2")]`.)

- [ ] **Step 4: Commit**

  ```bash
  git add crates/naves/src/mcp
  git commit -m "refactor: the MCP server's tools ask a host for the project and its data"
  ```

---

### Task 2: Lines in map metres to node ids (`naves_core::edits::lines`)

**Files:**
- Create: `crates/naves-core/src/edits/lines.rs`
- Modify: `crates/naves-core/src/edits.rs` (add `pub mod lines;` and re-export `SNAP_M`, `MAX_LISTED_WAYS`,
  `NodeIndex`, `check_line`)
- Modify: `crates/naves-core/src/edits/ops.rs` (make `drop_unused_user_nodes` `pub(super)`)
- Create: `fuzz/fuzz_targets/road_line.rs`, `fuzz/corpus/road_line/{valid,too-few,outside,long-tag,no-highway}`
- Modify: `fuzz/Cargo.toml` (`[[bin]] road_line`), `crates/naves-core/tests/fuzz_seeds.rs` (`road_line_seeds`)
- Test: `crates/naves-core/tests/edits_lines.rs`

**Interfaces:**
- Consumes: `WayEdits::{new_node, add_way, modify_way}`, `MapGeo::{to_map, to_geo, map_size_m}`, `OsmWays`, `OsmWay`.
- Produces:
  - `pub const SNAP_M: f64 = 0.05;` — a point this close to a node is that node.
  - `pub const MAX_LISTED_WAYS: usize = 1_000;`
  - `pub struct NodeIndex` with `pub fn new(ways: &OsmWays, frame: &MapGeo) -> NodeIndex` (every node of every
    way, projected once; nodes that cannot be projected are left out; a node shared by ways appears once) and
    `pub fn near(&self, x: f64, y: f64) -> Option<i64>` (the raw id of the nearest node within `SNAP_M`, ties by
    smaller id).
  - `pub fn check_line(what: &str, points: &[(f64, f64)], map_size_m: f64) -> Result<(), EditsError>`
  - `impl WayEdits`:
    - `pub fn line_nodes(&mut self, index: &NodeIndex, frame: &MapGeo, points: &[(f64, f64)], by: By) -> Result<Vec<ElementId>, EditsError>`
    - `pub fn add_line(&mut self, index: &NodeIndex, frame: &MapGeo, tags: BTreeMap<String, String>, points: &[(f64, f64)], by: By) -> Result<ElementId, EditsError>`
    - `pub fn modify_line(&mut self, index: &NodeIndex, frame: &MapGeo, original: &OsmWay, current: &OsmWay, tags: Option<BTreeMap<String, String>>, points: Option<&[(f64, f64)]>, by: By) -> Result<(), EditsError>`

Rules the code must follow (the spec leaves the mapping to the builder; these are the decisions):

- `check_line`: at least 2 and at most `MAX_WAY_NODES` points (checked before any per-point work), every coordinate
  finite and inside `0.0..=map_size_m`. Problems: `"a way needs at least 2 nodes"`, `"too many nodes"`,
  `"a point is not a finite number"`, `"a point is outside the map"`.
- `line_nodes`: for each point, in order — the node `index.near` finds; else a node already made **in this call**
  within `SNAP_M` (so a closed line reuses its first node); else `new_node(to_geo(point), by)`. No OSM node is moved.
  `NodeIndex` sorts by `x` and searches with `partition_point` over `x ± SNAP_M` (no float-to-int casts).
- `add_line`: `check_line`, then the tags must have a `highway` key (`"a road needs a highway tag"`; the editor
  only shows `highway` ways), then `line_nodes` and `add_way`.
- `modify_line`: both `None` is `"nothing to change"`; omitted points keep `current.nodes`, omitted tags keep
  `current.tags`; given tags need `highway`; then `modify_way(original, …)` and `drop_unused_user_nodes` for the
  user nodes that `current` listed and the new list does not.
- On any error the set is as it was: run the steps on a clone and assign it at the end.

- [ ] **Step 1: Write the failing tests** in `crates/naves-core/tests/edits_lines.rs`. Fixture: `Project::new("T",
  47.0, 12.0, 1000.0)`, `frame = MapGeo::new(&p)`, and `OsmWays` with way 10 `[1, 2, 3]` and way 20 `[4, 2, 5]`
  (node 2 at map `(500.0, 500.0)`, positions made with `frame.to_geo`), all tagged `highway=residential`.

  ```rust
  #[test] fn a_point_near_a_node_reuses_it()          // (500.004, 499.996) -> ElementId::Osm(2); edits.nodes stays empty
  #[test] fn a_point_off_a_node_is_a_new_node()        // (500.1, 500.0) -> ElementId::User(_); one NodeEdit, by == By::Ai
  #[test] fn a_closed_line_reuses_its_first_new_node() // [(100,100),(200,100),(200,200),(100,100.01)] -> nodes[0] == nodes[3]
  #[test] fn add_line_needs_a_highway_tag()            // tags {name=x} -> Err(Invalid{problem: "a road needs a highway tag"}); edits == before
  #[test] fn modify_line_keeps_an_unmoved_junction()   // way 10 to [(p1 moved 5 m), (500,500), p3] -> nodes[1] == Osm(2)
  #[test] fn modify_line_drops_unused_user_nodes()     // add u-way of 3 new points, modify it to its first and last point -> middle NodeEdit gone
  #[test] fn modify_line_keeps_what_is_omitted()       // tags only -> nodes unchanged; points only -> tags unchanged (plus naves:keep)
  #[test] fn modify_line_with_nothing_is_an_error()    // (None, None) -> "nothing to change"
  #[test] fn check_line_limits()                       // 1 point, MAX_WAY_NODES + 1 points, (-0.1, 5), (5, 1000.1), (f64::NAN, 5) -> the five problems above
  #[test] fn a_refused_line_changes_nothing()          // a valid prefix then an outside point -> Err, edits == clone taken before
  ```

- [ ] **Step 2: Run them to see them fail**

  Run: `cargo test -p naves-core --locked --test edits_lines`
  Expected: FAIL to compile (`lines` does not exist).

- [ ] **Step 3: Implement `lines.rs`** with the signatures and rules above; rustdoc on every public item.

- [ ] **Step 4: Run the tests**

  Run: `cargo test -p naves-core --locked --test edits_lines`
  Expected: PASS.

- [ ] **Step 5: Fuzz target and seeds.** `road_line`: parse the input as JSON `{"points": [[x, y], …], "tags": {…}}`
  (return on anything else), then `check_line` and `add_line` on `WayEdits::default()` with
  `Project::new("Fuzz", 47.0, 12.0, 4000.0)` and an empty `NodeIndex`. Seeds: `valid` (3 points, `highway=track`)
  and one per refusal. `road_line_seeds` in `fuzz_seeds.rs` asserts `is_ok() == (name == "valid")`, like
  `way_edits_parse_seeds`.

  Run: `cargo test -p naves-core --locked --test fuzz_seeds road_line` and `cd fuzz && cargo +nightly fuzz run road_line -- -max_total_time=10`
  Expected: PASS; the fuzzer finds no crash.

- [ ] **Step 6: Commit**

  ```bash
  git add crates/naves-core fuzz
  git commit -m "feat: edits from lines in map metres, joined to the nodes they touch"
  ```

---

### Task 3: `roads_list` and the source layer in the host

**Files:**
- Modify: `crates/naves/src/mcp/host.rs` (`Host::source`, the source cache in `StandaloneHost`)
- Create: `crates/naves/src/mcp/roads.rs` (the JSON of a way, the rectangle test, the argument structs of the road tools)
- Modify: `crates/naves/src/mcp/mod.rs` (the tool)
- Modify: `crates/naves/tests/mcp.rs` (`lists_tools` gains `roads_list`; new tests)
- Modify: `docs/manual/src/mcp.md`, `CHANGELOG.md`

**Interfaces:**
- Consumes: `naves_export::generate::{Source, read_source}`, `naves_core::edits::MAX_LISTED_WAYS`.
- Produces:
  - `Host::source(&self) -> Result<Arc<Source>, String>`: the extract with the current edits applied.
    `StandaloneHost` keeps `(Project, Arc<Source>)`; each call loads `naves.toml` and `edits/ways.toml`; the
    extract is read again (blocking pool) only when the project differs, and the edits are reapplied
    (`Source::reapply`) only when the loaded `WayEdits` differs from the cached one. A damaged edit file is the
    error `read_source`/`WayEdits::load` gives, which names `edits/ways.toml`.
  - `roads::RectParams { x0, y0, x1, y1: f64, limit: Option<u32> }` (`Deserialize`, `JsonSchema`, doc comments as
    in `RenderParams`).
  - `roads::way_json(source: &Source, way: &OsmWay, edited: &BTreeSet<i64>, moved: &BTreeSet<i64>) -> serde_json::Value`
    → `{"id": "<ElementId>", "origin": "osm" | "edited" | "new", "tags": {…}, "points": [[x, y], …]}`; points in map
    metres rounded to 0.01 m, a point that cannot be projected left out. `origin` follows `EditSession::origin`:
    user id → `new`; a way edit or a moved node → `edited`; else `osm`.
  - `roads::touches(points: &[(f64, f64)], rect: (f64, f64, f64, f64)) -> bool`: a point inside the closed
    rectangle, or a segment that crosses it (segment–rectangle clipping).

The tool: `roads_list` — "The roads and paths (OpenStreetMap ways with a highway tag, edits applied) that touch a
rectangle of the map in map metres: id, origin (osm, edited, new), tags and points. At most 1000 ways (limit),
`more` is true when there are more." Ways with a `highway` tag only, in id order; `limit` defaults to and is
reduced to `MAX_LISTED_WAYS`; `limit: 0` is an error. The rectangle is checked like `render_topdown`'s (finite,
`x0 < x1`, `y0 < y1`). Result `{"ways": [...], "more": bool}`. A project without OSM files gives `{"ways": [], "more": false}`.

- [ ] **Step 1: Write the failing tests** in `crates/naves/tests/mcp.rs` (fixtures `start_roads(false)` and `start()`):

  ```rust
  #[tokio::test] async fn roads_list_lists_the_highways_of_a_rectangle()
  // whole map 0..1500: ways non-empty; every way has a string id, origin == "osm", tags["highway"] present, >= 2 points;
  // a quarter 0..750 lists fewer ways than the whole map, and more than none
  #[tokio::test] async fn roads_list_cuts_at_the_limit()          // limit 1 -> 1 way, more == true; limit 0 -> tool error
  #[tokio::test] async fn roads_list_without_osm_is_empty()       // start(): {"ways": [], "more": false}
  #[tokio::test] async fn roads_list_rejects_a_bad_rectangle()    // x1 < x0 -> is_error
  #[tokio::test] async fn a_damaged_edit_file_is_reported()
  // write "not toml" to proj/edits/ways.toml: roads_list is an error whose text contains "edits/ways.toml";
  // render_topdown without layers still returns a PNG
  ```
  and add `"roads_list"` to `lists_tools`.

- [ ] **Step 2: Run them to see them fail**

  Run: `cargo test -p naves --locked --test mcp roads_list damaged lists_tools`
  Expected: FAIL (unknown tool `roads_list`).

- [ ] **Step 3: Implement** `Host::source`, `roads.rs` and the tool.

- [ ] **Step 4: Run the tests**

  Run: `cargo test -p naves --locked --test mcp`
  Expected: PASS, all of them.

- [ ] **Step 5: Document.** `mcp.md`: a row for `roads_list` in the tool table and a paragraph on coordinates and
  ids (`"812"`, `"u3"`). `CHANGELOG.md` under Unreleased → Added: "`naves mcp` lists the roads of a part of the map
  (`roads_list`)."

  Run: `mdbook build docs/manual`
  Expected: builds without broken links.

- [ ] **Step 6: Commit**

  ```bash
  git add crates/naves docs/manual CHANGELOG.md
  git commit -m "feat: roads_list, the roads of a rectangle with their edits, for MCP clients"
  ```

---

### Task 4: `road_add`, `road_modify`, `road_delete`, `road_reset`

**Files:**
- Modify: `crates/naves/src/mcp/host.rs` (`Host::edit_ways`; the map cache follows the edits)
- Modify: `crates/naves/src/mcp/roads.rs` (argument structs), `crates/naves/src/mcp/mod.rs` (the four tools)
- Modify: `crates/naves/tests/mcp.rs`, `docs/manual/src/mcp.md`, `CHANGELOG.md`

**Interfaces:**
- Consumes: Task 2's `NodeIndex`, `WayEdits::{add_line, modify_line}`; existing `WayEdits::{delete_way, reset_way}`;
  Task 3's `Host::source`, `roads::way_json`.
- Produces:
  - `Host::edit_ways<T: Send + 'static>(&self, op: impl FnOnce(&mut WayEdits, &Source) -> Result<T, EditsError> + Send + 'static) -> Result<(T, Arc<Source>), String>`.
    `StandaloneHost`: holds `writes` for the whole call, takes `source()`, runs `op` on a clone of `source.edits`
    in the blocking pool, errors as `"The edit cannot be made: {e}"`, saves with `WayEdits::save` (errors as
    `"Could not save the edit: {e}"`), reapplies, caches and returns the new source. Phase 2's `EditorHost` will
    run the same `op` through `EditSession::apply`.
  - The map cache (`Cached`) remembers the `WayEdits` its `roads` were generated from and generates again when
    the edits on disk differ.
  - Argument structs in `roads.rs`: `AddParams { points: Vec<[f64; 2]>, tags: BTreeMap<String, String> }`,
    `ModifyParams { id: String, points: Option<Vec<[f64; 2]>>, tags: Option<BTreeMap<String, String>> }`,
    `IdParams { id: String }`.

Tool results and errors:

| Tool | Result | Errors (tool errors; nothing written) |
|---|---|---|
| `road_add` | the new way as `way_json` | the limits of Task 2, the highway tag |
| `road_modify` | the way after the edit as `way_json` | unknown id, `"way <id> is deleted; road_reset brings it back"`, nothing to change, limits |
| `road_delete` | `{"deleted": "<id>"}` | unknown id, already deleted |
| `road_reset` | the way after the reset as `way_json`, or `{"removed": "<id>"}` for a drawn way | unknown id, `"way <id> has no edit"` |

An id is parsed with `ElementId::from_str` (`"not a way id: …"` on failure). "Unknown" means: no way of that id in
`source.ways` (for modify and delete) and no edit and no extract way (for reset). The original for `modify_way` /
`delete_way` is the extract's way for an OSM id and the current way for a user id (as `EditSession::original`).
`road_reset` drops the way edit only; moved OSM nodes stay moved (they may belong to other ways too) — the tool
description says so.

- [ ] **Step 1: Write the failing tests** in `crates/naves/tests/mcp.rs`; read the edit file in the tests with
  `naves_core::edits::WayEdits::load(&f.proj)`:

  ```rust
  #[tokio::test] async fn road_add_adds_a_way_and_writes_the_edit()
  // 3 points inside the map, {highway: track}: result id starts with "u", origin "new"; the file has one Add way edit with by == By::Ai
  // and tags highway=track, naves:keep=yes; roads_list of its rectangle contains the id
  #[tokio::test] async fn road_add_joins_an_existing_node()
  // first point = a roads_list point of an OSM way plus 0.004 m: the Add edit's first node is ElementId::Osm(_), and no NodeEdit has that id
  #[tokio::test] async fn road_modify_changes_the_tags_and_keeps_the_line()
  // tags {highway: track}: points equal before and after, origin "edited", the file has a Modify edit with a base
  #[tokio::test] async fn road_modify_without_changes_is_an_error()
  #[tokio::test] async fn road_delete_and_reset_bring_the_way_back()
  // delete -> not in roads_list; reset -> in roads_list again with origin "osm"; the file has no way edits
  #[tokio::test] async fn road_tools_refuse_bad_ids_and_leave_the_file()
  // after one road_add: road_modify/road_delete/road_reset with "u999", "0", "abc"; road_modify of a deleted way;
  // each is_error, and the bytes of edits/ways.toml are unchanged
  #[tokio::test] async fn road_tools_name_the_limit()
  // 2001 points -> "too many nodes"; a point at x = -1 -> "outside the map"; a 256-character tag value -> "too long";
  // tags without highway -> "highway tag"; each leaves the file unchanged
  #[tokio::test] async fn parallel_road_adds_all_arrive()
  // 8 road_add calls joined at once: 8 distinct ids, the file has 8 Add way edits
  #[tokio::test] async fn roads_follow_an_edit()
  // start_roads(true): roads_stats, delete the way with the most points, roads_stats again: the sum of km_by_family is smaller
  ```
  and add the four names to `lists_tools`.

- [ ] **Step 2: Run them to see them fail**

  Run: `cargo test -p naves --locked --test mcp road`
  Expected: FAIL (unknown tools).

- [ ] **Step 3: Implement** `Host::edit_ways`, the cache rule, and the four tools. Each tool builds the
  `NodeIndex` from `source.ways` inside `op` (it runs in the blocking pool).

- [ ] **Step 4: Run all MCP tests**

  Run: `cargo test -p naves --locked --test mcp`
  Expected: PASS.

- [ ] **Step 5: Document.** `mcp.md`: rows for the four tools, the joining rule (a point within 5 cm of a node is
  that node), the limits, that edits are written at once to `edits/ways.toml` marked `by = "ai"`, that there is no
  undo in `naves mcp <dir>` (the editor has it, phase 2), and that a build is needed before the generated roads in
  the game change. `CHANGELOG.md` → Added: "`naves mcp` can add, change, delete and reset roads (`road_add`,
  `road_modify`, `road_delete`, `road_reset`)." and → Fixed: "`roads_stats`, `validation_list` and the road layers
  of `render_topdown` in `naves mcp` follow changes of `edits/ways.toml`."

- [ ] **Step 6: Commit**

  ```bash
  git add crates/naves docs/manual CHANGELOG.md
  git commit -m "feat: road edit tools for MCP clients"
  ```

---

### Task 5: Server instructions, documentation, `make check`

**Files:**
- Modify: `crates/naves/src/mcp/host.rs` (`Host::instructions`), `crates/naves/src/mcp/mod.rs` (`get_info`)
- Modify: `crates/naves/tests/mcp.rs`, `docs/manual/src/mcp.md`, `CHANGELOG.md`,
  `docs/superpowers/specs/2026-11-27-editing-design.md`

**Interfaces:**
- Produces: `Host::instructions(&self) -> &'static str`; the standalone text says, in a few sentences: coordinates
  are map metres from the south-west corner (as in `render_topdown`, `roads_list` and the road tools); look at the
  map with `render_topdown`; ids are `"812"` (OpenStreetMap) and `"u3"` (drawn); edits are written at once to the
  project; a `build` is needed to see them in the generated roads. Phase 2 adds the editor's text (with
  `view_get` and undo).
- `impl ServerHandler for NavesMcp` gets its own `get_info` (rmcp's `tool_handler` then does not generate one):
  `ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(Implementation::from_build_env()).with_instructions(self.host.instructions().to_string())`.

- [ ] **Step 1: Write the failing test**

  ```rust
  #[tokio::test]
  async fn the_server_explains_itself() {
      let f = start().await;
      let info = f.client.peer_info().unwrap();
      let text = info.instructions.clone().unwrap();
      for word in ["map metres", "south-west", "render_topdown", "roads_list", "build"] {
          assert!(text.contains(word), "{word}: {text}");
      }
      assert!(info.capabilities.tools.is_some());
  }
  ```

- [ ] **Step 2: Run it to see it fail**

  Run: `cargo test -p naves --locked --test mcp the_server_explains_itself`
  Expected: FAIL (`instructions` is `None`).

- [ ] **Step 3: Implement** `Host::instructions` and `get_info`.

- [ ] **Step 4: Run it**

  Run: `cargo test -p naves --locked --test mcp`
  Expected: PASS.

- [ ] **Step 5: Document.** `mcp.md`: a sentence that the server sends these instructions to every client.
  `CHANGELOG.md` → Added: "`naves mcp` tells its clients how its coordinates, ids and edits work." In
  `2026-11-27-editing-design.md`, under phase 5 "MCP edit tools", one line: "Replaced by
  `2026-11-28-agent-chat-design.md`: each phase brings its MCP tools."

- [ ] **Step 6: Run the whole check**

  Run: `systemd-run --user --scope -p MemoryMax=24G -p CPUQuota=1200% env CARGO_BUILD_JOBS=12 make check`
  Expected: every step passes; coverage at or above 85 %.

- [ ] **Step 7: Commit**

  ```bash
  git add crates/naves docs CHANGELOG.md
  git commit -m "feat: the MCP server explains its coordinates, ids and edits to clients"
  ```

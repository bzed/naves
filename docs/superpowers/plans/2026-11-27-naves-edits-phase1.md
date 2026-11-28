<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Edits, phase 1: the edit set and the build — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A project can hold hand-made road edits in `edits/ways.toml` (OSM-style nodes and ways in lat/lon, user ids
`u<n>`), and every build applies them right after the OSM import, also for a project without OSM; edited lines are
kept as drawn by every stage that would change a line.

**Architecture:** A new module `naves_core::edits` (ids, file, operations, applying) works on `OsmWays` before
`features::road_features`. A Náves tag `naves:keep=yes` becomes a `held` attribute of road features and stretches;
the clean-up, the smoothing and (once merged) the lot stage and road planning leave held stretches' lines alone. The
build reports broken, changed and outside-the-map edits as new rules.

**Tech Stack:** Rust 1.99, edition 2024; `serde` + `toml` (already used by `objects.rs`), `tempfile` for atomic
writes, `thiserror`, `libfuzzer-sys` fuzz target. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-11-27-editing-design.md` (sections 5.1, 5.2, 5.5; phase 1 of section 6).

## Global Constraints

- AGENTS.md applies in full: test first; integration tests in `crates/naves-core/tests/`; no `unwrap`/`expect`/
  indexing on untrusted data in library code; `#[expect(..., reason = "...")]`, never `#[allow]`; `BTreeMap`/sorted
  output; atomic writes (temp file in the same folder, then rename); `thiserror` errors with file and object context;
  every public item has rustdoc; `make check` passes before each commit; coverage stays ≥ 85 %.
- The edit files are untrusted input: parsing never panics, every count and length is checked against a limit before
  use, numbers must be finite.
- Positions in the file are WGS84 `lat`, `lon` in degrees; never map metres.
- User ids: written `u<n>` (n ≥ 1) in the file, `USER_ID_BASE + n` in the program, `USER_ID_BASE = 1 << 40`; an
  integer id in the file is an OSM id and must be `1..USER_ID_BASE`.
- Feature ids: OSM ways keep `osm/way/<id>/<part>`; user ways are `user/way/<n>/<part>` with `Source::User`.
- `naves:keep=yes` = keep the line as drawn (no drop, no merge that moves its nodes, no smoothing, no rerouting, no lot
  removal or cut). `naves:plan=yes` = road planning may improve it anyway. Every way the edit API adds or modifies
  gets `naves:keep=yes` unless the caller passes `naves:keep=no`.
- Edits always win: a `modify`/`delete` whose original changed or is gone is still applied, and reported.
- User-visible change → `CHANGELOG.md` line under `## [Unreleased]` and `docs/manual/` in the same commit.
- Execution order: Tasks 1–6 start now on branch `gui-editing` (worktree `/var/tmp/naves-gui-edit/wt`, based on
  `main`). Before Task 7, rebase onto `main` after PR #26 (lots) and PR #25 (DEM fetch, which brings `road-planning`)
  are merged; Tasks 8 and 9 need them. If only one has landed, do the tasks that need only it and leave the other.

## Review Focus

1. A way edit that references a node that neither the extract nor the file has (for example the extract was cut
   smaller): the way is left out and reported as `edit_broken`, the build goes on. Test in Task 3.
2. Two edits for the same way id, or a node listed twice in `nodes`: refused at parse time with a message that names
   the way. Test in Task 2.
3. A modified OSM way that is no longer in the extract at all (area moved away from it, or deleted in OSM): applied
   from the file alone, reported `edit_original_changed`. Test in Task 3.
4. A project with no OSM sources and only drawn roads builds roads (custom map). Test in Task 7.
5. A held stretch shares a junction node with an unheld one that the junction merge wants to move: the node is not
   moved. Test in Task 6.

---

## File structure

| File | Responsibility |
|---|---|
| `crates/naves-core/src/edits.rs` | module root: re-exports, `EditsError` |
| `crates/naves-core/src/edits/ids.rs` | `ElementId` (OSM or user), parsing `u<n>`, `USER_ID_BASE` |
| `crates/naves-core/src/edits/ways.rs` | `WayEdits` file model, parse, validate, load, save, fingerprint |
| `crates/naves-core/src/edits/ops.rs` | edit operations the GUI and MCP call |
| `crates/naves-core/src/edits/apply.rs` | `apply_way_edits` and `EditReport` |
| `crates/naves-core/tests/edits.rs` | integration tests of the module |
| `fuzz/fuzz_targets/way_edits_parse.rs` + `fuzz/corpus/way_edits_parse/` | fuzz target and seeds |
| `crates/naves-core/src/features.rs` | `held`, user feature ids |
| `crates/naves-core/src/roadnet.rs` | `Stretch::held` |
| `crates/naves-core/src/netclean.rs`, `netclean/*.rs`, `smooth.rs` | leave held stretches alone |
| `crates/naves-core/src/validate.rs` | three new rules |
| `crates/naves-export/src/generate.rs` | load and apply edits, report |
| `docs/manual/src/project-file.md`, `troubleshooting.md`, `CHANGELOG.md` | user docs |

---

### Task 1: Element ids

**Files:**
- Create: `crates/naves-core/src/edits.rs`, `crates/naves-core/src/edits/ids.rs`
- Modify: `crates/naves-core/src/lib.rs` (add `pub mod edits;`)
- Test: `crates/naves-core/tests/edits.rs`

**Interfaces:**
- Produces:
  - `pub const USER_ID_BASE: i64 = 1 << 40;`
  - `pub enum ElementId { Osm(i64), User(u64) }` with `Ord`, `Copy`, `Debug`, `Display` (`123` / `u7`),
    `FromStr`, serde as either a TOML integer (OSM) or a string `"u<n>"` (user).
  - `impl ElementId { pub fn raw(self) -> Option<i64>; pub fn from_raw(raw: i64) -> Option<ElementId>; pub fn is_user(self) -> bool }`
    `raw` is the id inside the program (`n + USER_ID_BASE` for a user id, `None` when that overflows or `n` is 0);
    `from_raw` is its inverse (`None` for `raw <= 0`).

- [ ] **Step 1: Write the failing tests** in `crates/naves-core/tests/edits.rs`:

```rust
use naves_core::edits::{ElementId, USER_ID_BASE};

#[test]
fn user_ids_read_and_write_as_u_n() {
    let id: ElementId = "u7".parse().unwrap();
    assert_eq!(id, ElementId::User(7));
    assert_eq!(id.to_string(), "u7");
    assert_eq!(id.raw(), Some(USER_ID_BASE + 7));
    assert_eq!(ElementId::from_raw(USER_ID_BASE + 7), Some(id));
}

#[test]
fn osm_ids_stay_plain() {
    assert_eq!("8421".parse::<ElementId>().unwrap(), ElementId::Osm(8421));
    assert_eq!(ElementId::Osm(8421).raw(), Some(8421));
    assert_eq!(ElementId::from_raw(8421), Some(ElementId::Osm(8421)));
}

#[test]
fn bad_ids_are_refused() {
    for text in ["", "u", "u0", "u-1", "x3", "0", "-5", "1099511627776", "u99999999999999999999"] {
        assert!(text.parse::<ElementId>().is_err(), "{text}");
    }
}
```

`1099511627776` is `USER_ID_BASE`: an OSM id must be below it.

- [ ] **Step 2: Run** `cargo test -p naves-core --test edits` — expect a compile failure (module missing).
- [ ] **Step 3: Implement** `ids.rs` with the interface above, and `edits.rs` with `pub mod ids; pub use ids::*;` and
  the module rustdoc (what the edit set is, pointing to the spec section 5.2).
- [ ] **Step 4: Run** `cargo test -p naves-core --test edits` — PASS.
- [ ] **Step 5: Commit** `feat: element ids for hand-made edits (OSM or user, u<n>)`.

---

### Task 2: The `edits/ways.toml` file

**Files:**
- Create: `crates/naves-core/src/edits/ways.rs`, `fuzz/fuzz_targets/way_edits_parse.rs`,
  `fuzz/corpus/way_edits_parse/{valid,bad-lat,dup-way,too-many-nodes}`
- Modify: `crates/naves-core/src/edits.rs`, `fuzz/Cargo.toml` (a `[[bin]]` like `overrides_parse`),
  `crates/naves-core/tests/fuzz_seeds.rs`, `REUSE.toml` if the seeds need an entry (as the other corpora have)
- Test: `crates/naves-core/tests/edits.rs`

**Interfaces:**
- Consumes: `ElementId` (Task 1).
- Produces (all `pub`, serde with `deny_unknown_fields`):
  - `pub enum By { User, Ai }` (`"user"`, `"ai"`), default `User`.
  - `pub struct NodeEdit { pub id: ElementId, pub lat: f64, pub lon: f64, pub base: Option<(f64, f64)>, pub by: By }`
    (`base`: the OSM position when an OSM node was moved; `None` for user nodes).
  - `pub enum WayAction { Add, Modify, Delete }` (`"add"`, `"modify"`, `"delete"`).
  - `pub struct WayEdit { pub id: ElementId, pub action: WayAction, pub tags: BTreeMap<String, String>, pub nodes: Vec<ElementId>, pub base: Option<String>, pub by: By }`
    (`base`: fingerprint of the OSM original, for `modify` and `delete`).
  - `pub struct WayEdits { pub format: u32, pub next_id: u64, pub nodes: Vec<NodeEdit>, pub ways: Vec<WayEdit> }`,
    `Default` = format 1, next_id 1, empty.
  - `impl WayEdits { pub fn parse(text: &str) -> Result<WayEdits, EditsError>; pub fn validate(&self) -> Result<(), EditsError>; pub fn load(project_dir: &Path) -> Result<WayEdits, EditsError>; pub fn save(&self, project_dir: &Path) -> Result<(), EditsError> }`
    — file `edits/ways.toml`; a missing file is `WayEdits::default()`.
  - `pub fn fingerprint(tags: &BTreeMap<String, String>, nodes: &[i64]) -> String` — 16 hex digits of a stable
    64-bit FNV-1a over `key=value\n` lines in key order, then `nodes` as decimal lines. (Not `DefaultHasher`: it is
    not stable across Rust versions.)
  - `pub enum EditsError` (`thiserror`): `Io { path, source }`, `Parse(toml::de::Error)`, `Write(toml::ser::Error)`,
    `Invalid { what: String, problem: &'static str }` where `what` names the element (`way u3`, `node 812`).
  - Limits, `pub const`: `MAX_WAY_EDITS = 100_000`, `MAX_NODE_EDITS = 1_000_000`, `MAX_WAY_NODES = 2_000` (OSM's own
    limit), `MAX_TAGS = 64`, `MAX_TAG_LEN = 255` (OSM's limit, in characters).

Validation rules (each its own `problem` text): `format` must be 1; `next_id` greater than every user id used;
`lat` in −90..=90, `lon` in −180..=180, both finite (also `base`); no node id twice in `nodes`; no way id twice in
`ways`; `add` ways have a user id, `modify`/`delete` an OSM id; `add`/`modify` have 2..=`MAX_WAY_NODES` nodes (a
closed way repeats its first node, which is the only allowed repeat, and only at the end); `delete` has no nodes
and no tags; tag keys non-empty; counts and lengths within the limits; a user node edit has `base = None`.

- [ ] **Step 1: Write the failing tests:**
  - `ways_file_round_trips`: a `WayEdits` with one user node, one moved OSM node (with `base`), one `add` and one
    `modify` way saves to a `TempDir` and loads back equal; the saved text contains `id = "u1"` and `nodes = [`.
  - `missing_file_is_empty`: `WayEdits::load(empty_tempdir) == Ok(WayEdits::default())`.
  - `duplicate_way_is_refused`: two entries with `id = 8421` → `Err(EditsError::Invalid { what, .. })` with
    `what == "way 8421"`.
  - `repeated_node_is_refused_except_closing`: nodes `[u1, u2, u1]` accepted (closed), `[u1, u1, u2]` refused.
  - `limits_are_checked`: 2001 nodes in one way → refused; `lat = 91.0` → refused; `lat = nan` → refused.
  - `add_needs_user_id_and_modify_needs_osm_id`: both wrong combinations refused.
  - `fingerprint_is_stable`: `fingerprint({highway=track}, [1, 2])` equals a fixed hex string you compute once and
    write into the test, and changes when a tag or node changes.
- [ ] **Step 2: Run** `cargo test -p naves-core --test edits` — FAIL.
- [ ] **Step 3: Implement** `ways.rs`; `save` writes `toml::to_string_pretty` atomically as `Overrides::save` does
  (`crates/naves-core/src/objects.rs:253`), after `validate`.
- [ ] **Step 4: Add the fuzz target** `way_edits_parse` (calls `WayEdits::parse` on UTF-8 input, like
  `fuzz/fuzz_targets/overrides_parse.rs`), the four seeds, and `way_edits_parse_seeds` in `fuzz_seeds.rs`
  (`is_ok() == (name == "valid")`).
- [ ] **Step 5: Run** `cargo test -p naves-core --test edits --test fuzz_seeds` and `make fuzz` — PASS.
- [ ] **Step 6: Commit** `feat: edits/ways.toml holds hand-made nodes and ways`.

---

### Task 3: Applying way edits to the OSM ways

**Files:**
- Create: `crates/naves-core/src/edits/apply.rs`
- Modify: `crates/naves-core/src/edits.rs`
- Test: `crates/naves-core/tests/edits.rs`

**Interfaces:**
- Consumes: `WayEdits`, `fingerprint`, `ElementId::raw` (Tasks 1–2); `naves_core::osm::{OsmWay, OsmWays}`.
- Produces:
  - `pub enum EditIssue { Broken { way: ElementId, node: ElementId }, OriginalChanged { way: ElementId }, OriginalGone { way: ElementId } }`
  - `pub struct EditReport { pub applied: usize, pub issues: Vec<(EditIssue, (f64, f64))> }` — each issue with a
    (lat, lon) to show it at: the first known node of the way.
  - `pub fn apply_way_edits(ways: &OsmWays, edits: &WayEdits) -> (OsmWays, EditReport)`

Rules, in this order (spec 5.2): node edits replace the position of that node id in every way; `delete` removes the
way; `modify` replaces tags and nodes; `add` inserts. Node positions of an edited way come from the node edits, else
from any way of the extract that has that node, else the way is `Broken` and left out. `modify`/`delete`: no way of
that id in the extract → `OriginalGone`; there but `fingerprint(tags, nodes)` differs from `base` → `OriginalChanged`;
applied in both cases (a `delete` of a gone way changes nothing). Output ways are in id order (raw ids), so the
result does not depend on file order. `skipped` is carried over.

- [ ] **Step 1: Write the failing tests** (small `OsmWays` built in the test):
  - `moved_node_moves_in_every_way`: two OSM ways share node 5; a node edit moves 5; both output ways have the new
    position at 5.
  - `modify_replaces_tags_and_nodes`, `delete_removes_way`, `add_inserts_user_way_with_raw_ids` (the added way's
    `id == USER_ID_BASE + 1`, its node ids raw too).
  - `added_way_joins_existing_node`: an `add` way whose first node is OSM node 5 takes 5's position from the extract.
  - `missing_node_is_broken`: `modify` referencing node 999 not in extract or file → way absent from the output,
    one `Broken { node: Osm(999), .. }`, all other ways kept.
  - `gone_original_still_applies`: `modify` of way 77 not in the extract → output has way 77, issue `OriginalGone`.
  - `changed_original_is_reported`: `base` mismatching → applied, `OriginalChanged`.
  - `order_of_file_does_not_matter`: the same edits in two orders give equal output.
  - `no_osm_is_fine`: `apply_way_edits(&OsmWays::default(), &edits_with_one_add)` gives one way.
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement** `apply.rs`. **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat: way edits are applied to the OSM ways, edits win`.

---

### Task 4: Edit operations

**Files:**
- Create: `crates/naves-core/src/edits/ops.rs`
- Modify: `crates/naves-core/src/edits.rs`
- Test: `crates/naves-core/tests/edits.rs`

**Interfaces:**
- Consumes: Tasks 1–3; `OsmWay`.
- Produces (methods on `WayEdits`, all keep the set valid and return `Result<_, EditsError>` where a limit can be hit):
  - `pub fn new_node(&mut self, lat: f64, lon: f64, by: By) -> Result<ElementId, EditsError>` — next user id.
  - `pub fn move_node(&mut self, id: ElementId, lat: f64, lon: f64, osm_at: Option<(f64, f64)>, by: By) -> Result<(), EditsError>`
    — replaces an earlier edit of the same node; `osm_at` becomes `base` for an OSM node.
  - `pub fn add_way(&mut self, tags: BTreeMap<String, String>, nodes: Vec<ElementId>, by: By) -> Result<ElementId, EditsError>`
  - `pub fn modify_way(&mut self, original: &OsmWay, tags: BTreeMap<String, String>, nodes: Vec<ElementId>, by: By) -> Result<(), EditsError>`
    — for an OSM way: stores `base = fingerprint(original)` the first time and keeps it on later modifications; for a
    user way (id ≥ `USER_ID_BASE`): updates its `add` entry.
  - `pub fn delete_way(&mut self, original: &OsmWay, by: By) -> Result<(), EditsError>` — a user way's entry is removed,
    and its user nodes that no other way uses.
  - `pub fn reset_way(&mut self, id: ElementId)` — drops the way's edit (back to OSM); user nodes no other way uses go too.
  - `add_way` and `modify_way` insert `naves:keep=yes` unless `tags` has `naves:keep` already.

- [ ] **Step 1: Write the failing tests:** `add_way_sets_keep`, `keep_no_is_respected`,
  `modify_twice_keeps_first_base`, `modify_user_way_updates_add`, `delete_user_way_drops_its_nodes`,
  `reset_returns_to_osm` (after `reset_way`, `apply_way_edits` gives the original way back),
  `ids_are_never_reused` (`new_node` after deleting `u3` gives `u4`).
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat: edit operations for ways and nodes`.

---

### Task 5: Held roads and user features

**Files:**
- Modify: `crates/naves-core/src/features.rs` (`FeatureKind::Road`, `classify`, `road_features`),
  `crates/naves-core/src/roadnet.rs` (`Attrs`, `Stretch`)
- Test: `crates/naves-core/tests/features.rs`, `crates/naves-core/tests/roadnet.rs`

**Interfaces:**
- Consumes: `USER_ID_BASE`, `ElementId::from_raw`.
- Produces:
  - `FeatureKind::Road { .., #[serde(default)] held: bool, #[serde(default)] plan: bool }` — `held` from
    `naves:keep=yes`, `plan` from `naves:plan=yes`.
  - `road_features`: a way with id ≥ `USER_ID_BASE` gets `id = "user/way/<n>/<part>"` and `Source::User`.
  - `Stretch { .., pub held: bool, pub plan: bool }`; both are part of `Attrs`, so a held run is its own stretch.
  - `OsmWay` tags: the reader's kept-tag list (`osm.rs`, the doc on `OsmWay::tags`) does not need `naves:*`; the
    tags only come from edits.

- [ ] **Step 1: Write the failing tests:** `keep_tag_makes_held_road`; `user_way_gets_user_feature_id`
  (way id `USER_ID_BASE + 3` → `"user/way/3/0"`, `Source::User`); `held_part_is_its_own_stretch` in `roadnet.rs`
  (one way split at node 2 into held and unheld parts via two ways → two stretches, `held` true on one).
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** Fix every `FeatureKind::Road { .. }` and `Stretch { .. }`
  literal the compiler names (tests included) with `held: false, plan: false`. **Step 4: Run** `cargo test -p naves-core` — PASS.
- [ ] **Step 5: Commit** `feat: naves:keep marks a road whose line is kept as drawn`.

---

### Task 6: The clean-up and the smoothing leave held lines alone

**Files:**
- Modify: `crates/naves-core/src/netclean.rs` (`drop_sidepaths` via `netclean/sidepath.rs`, `stubs_and_specks_if`,
  `merge_junctions`), `crates/naves-core/src/netclean/approach.rs`, `crates/naves-core/src/smooth.rs`
- Test: `crates/naves-core/tests/netclean.rs`, `crates/naves-core/tests/smooth.rs`

**Interfaces:**
- Consumes: `Stretch::held` (Task 5).
- Produces: no new API. Behaviour: a held stretch is never dropped as sidepath, stub or speck; `merge_junctions`
  does not merge a junction one of whose nodes ends a held stretch (it reports it in `not_merged` with the existing
  reason type, adding a variant `Held` if the reasons are an enum); `make_approaches` does not reshape a held
  stretch; the smoothing does not change a held stretch's points. Each skip is counted in the existing reports.

- [ ] **Step 1: Write the failing tests**, each taking an existing test of the step and marking the affected stretch
  held: `held_sidepath_is_kept`, `held_stub_is_kept`, `held_junction_is_not_merged` (the review-focus case: held and
  unheld stretch share the node; the node keeps its position), `held_bridge_approach_untouched`,
  `held_curve_is_not_smoothed` (points equal before and after).
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** `cargo test -p naves-core` — PASS.
- [ ] **Step 5: Commit** `feat: the clean-up and smoothing keep held road lines`.

---

### Task 7: The build applies the edits (after rebasing onto #25 and #26)

**Files:**
- Modify: `crates/naves-export/src/generate.rs` (`read_features`, `generate`, `Generated`, the report),
  `crates/naves-core/src/validate.rs` (`Rule`, names, levels), `crates/naves/src/mcp.rs` (the rule list in the
  `problems` tool description), `docs/manual/src/project-file.md` (new section `edits/ways.toml`),
  `docs/manual/src/troubleshooting.md` (the three rules), `CHANGELOG.md`
- Test: `crates/naves-export/tests/` (the existing generate tests' fixtures), `crates/naves-core/tests/validate.rs`

**Interfaces:**
- Consumes: `WayEdits::load`, `apply_way_edits`, `EditReport` (Tasks 2–3).
- Produces:
  - `Rule::EditBroken` (`"edit_broken"`, warning), `Rule::EditOriginalChanged` (`"edit_original_changed"`,
    information, for both `OriginalChanged` and `OriginalGone`), `Rule::EditOutsideMap` (`"edit_outside_map"`,
    information: an edited or added way none of whose nodes lies on the map).
  - `pub fn edit_violations(report: &EditReport, outside: &[(ElementId, (f64, f64))], frame: &MapGeo) -> Vec<Violation>`
    in `validate.rs`; issues whose position cannot be projected are placed at `(0.0, 0.0)` with the message saying so.
  - `Generated::edits: EditReport`; `report.json` gains `"edits": { "applied", "broken", "changed", "outside" }`; the build's
    progress line and the MCP `build` result show the same four counts when any is non-zero.
  - `read_features` reads OSM if there is any, else starts from `OsmWays::default()`, then applies the edits; it
    returns the `EditReport`.

- [ ] **Step 1: Rebase** `gui-editing` onto `main` after #25 and #26 are merged; `make check` — PASS before changes.
- [ ] **Step 2: Write the failing tests:**
  - `edits_change_the_built_roads`: fixture project + `edits/ways.toml` deleting one OSM way → the built network has
    no stretch with that way id; `report.edits.applied == 1`.
  - `custom_map_without_osm`: a project with DEM, `sources.osm = []` and one drawn `highway=track` way of 300 m → the
    build places pieces for one stretch whose feature id starts with `user/way/1/`.
  - `edit_outside_map_is_reported`: an added way 5 km outside the area → one `edit_outside_map`, build succeeds.
  - `broken_edit_does_not_stop_the_build`: a way referencing an unknown node → one `edit_broken`.
  - `damaged_edits_file_stops_the_build`: `ways.toml` with `lat = 200` → `BuildError` whose text names the file and
    `node`, like a damaged `overrides.toml` today.
- [ ] **Step 3: Run** `cargo test -p naves-export` — FAIL. **Step 4: Implement.**
- [ ] **Step 5: Docs:** project-file section with a short example, the field table, the id rules, "edits always win";
  troubleshooting entries; CHANGELOG `### Added` line "Hand-made road edits in `edits/ways.toml` …".
- [ ] **Step 6: Run** `make check` — PASS. **Step 7: Commit** `feat: the build applies hand-made road edits`.

---

### Task 8: The lot stage keeps held roads and takes edited lots (needs #26)

**Files:**
- Modify: `crates/naves-core/src/netclean/lot.rs` (road removal and access cut), the lot reading in
  `crates/naves-export/src/generate.rs` / `crates/naves-core/src/lot.rs`, `docs/manual/src/project-file.md`, `CHANGELOG.md`
- Test: `crates/naves-core/tests/` lot tests from #26, `crates/naves-export/tests/`

**Interfaces:**
- Consumes: `Stretch::held`, `WayEdits`, `fingerprint`.
- Produces: `pub fn apply_lot_edits(lots: &[LotPolygon], ways: &OsmWays, edits: &WayEdits) -> (Vec<LotPolygon>, EditReport)`
  in `edits/apply.rs`: a closed `add` or `modify` way whose tags make `lot_tags` (osm.rs, #26) say "lot" replaces the
  way-lot of that id (relations are not editable in phase 1) or adds one; a `delete` removes it; nodes resolve as in
  Task 3, using `ways` and the node edits. The lot stage does not remove or cut a held stretch.

- [ ] **Step 1: Write the failing tests:** `held_road_in_lot_is_kept`, `held_access_road_is_not_cut`,
  `edited_lot_outline_replaces_osm`, `deleted_lot_is_gone`, `drawn_lot_on_custom_map`.
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Docs + CHANGELOG.** **Step 5: Run** `make check` — PASS.
- [ ] **Step 6: Commit** `feat: lots keep held roads and take hand-edited outlines`.

---

### Task 9: Road planning keeps held lines unless `naves:plan=yes` (needs road-planning, via #25)

**Files:**
- Modify: the route planning stage of `road-planning` (the function that reroutes a stretch, `reroute_graph` and
  its callers), `docs/manual/src/project-file.md`, `CHANGELOG.md`
- Test: the route planning integration tests of that branch

**Interfaces:**
- Consumes: `Stretch::held`, `Stretch::plan`.
- Produces: a held stretch is rerouted only when `plan` is true; skipped ones are counted in the planning report.

- [x] **Step 1: Write the failing tests:** `held_tight_bend_is_not_rerouted` (reported `road_curve_too_tight`
  instead), `held_with_plan_is_rerouted`.
- [x] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Docs + CHANGELOG.** **Step 5: Run** `make check` — PASS.
- [x] **Step 6: Commit** `feat: road planning keeps drawn lines unless naves:plan=yes`.

---

## After this plan

Phase 2 (road line editing in the GUI) gets its own plan once Task 4's API is merged; it consumes
`WayEdits` operations, `apply_way_edits` and the `held` attribute unchanged.

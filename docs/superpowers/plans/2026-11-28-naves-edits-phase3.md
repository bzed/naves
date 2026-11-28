<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Terrain and earthwork editing (editing phase 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
> Every agent reads `/home/bzed/.claude/skills/linux-basics/SKILL.md` and follows it (at most half the cores:
> `CARGO_BUILD_JOBS=12`, `nice -n 19 ionice -c 3`, `systemd-run --user --scope -p CPUQuota=1200% -p MemoryMax=16G
> -p MemorySwapMax=1G`; no root; no pagers or editors). A UI task starts with `/frontend-design:frontend-design`
> (AGENTS.md) and follows the design notes below.

**Goal:** Shape the ground by hand: brushes (raise, lower, smooth, flatten) and earthwork shapes (pad, cut, fill,
mound, pit; ditch, embankment, ramp, terrace) drawn from the item selector on every level, stored in
`edits/terrain.toml`, applied by every build in the stage each edit names (`ground` before the roads, held against
their earthwork; `finish` after it). Before that, close the open points of phase 2.

**Architecture:** A new `naves_core::edits::terrain` module holds the file (parse with limits, save atomically, edit
operations) and `apply_terrain_edits`, which changes a `Heightmap` in place for one stage and returns the vertices a
`ground` shape holds. `conform` gets a `held` mask: held vertices keep their height, a road that wants another height
there is a `road_conflict`. `generate` runs DEM → ground edits → roads and `conform_held` → finish edits. The GUI's
`EditSession` holds both edit files as one snapshot, so one gesture is one undo step across files; the map previews
terrain edits on the DEM at once and Ctrl+R builds the rest.

**Tech Stack:** Rust 1.99, egui/eframe with the vendored `naves-ui` design system, `egui_kittest`,
`naves_core::{edits, conform, heightmap}`, `naves_export::generate`, `proptest` (fixed seeds), `cargo fuzz`.

**Spec:** `docs/superpowers/specs/2026-11-27-editing-design.md` (5.1, 5.4, 5.5, 5.6, phase 3), with the UI spec
`docs/superpowers/specs/2026-11-06-naves-editor-ui.md` section 4 (keys as revised 2026-11-28). Phase 2's open points
come from its final review (memory `gui-terrain-and-road-editing`, M3–M15).

## Global Constraints

- AGPL-3.0-or-later SPDX header on every new file; `make check` passes after every task (fmt, clippy `-D warnings`,
  doc, reuse, deny, test, fuzz, coverage ≥ 85 %, mdbook).
- `edits/terrain.toml` is untrusted input: every count, length and number is checked before use; no panics, no
  unchecked indexing, no allocation from a length in the file; `#[expect(..., reason)]`, never `#[allow]`.
- Positions in edit files are `(lat, lon)` WGS84; heights and sizes are **real metres**. The build projects with
  `MapGeo::to_map`, multiplies heights by `project.scale.height` and sizes (radius, width, `blend_m`) by
  `project.scale.horizontal` (spec 5.1). Nothing in map metres is written to an edit file.
- `offset` heights are relative to the **DEM as sampled** (`Generated::dem`) at the shape's reference point, never
  to an edited or shaped heightmap, so a shape keeps its meaning when roads or other edits change.
- Same input, same output: within a stage, strokes and shapes apply in ascending `id` order (ids are given in
  creation order from one `next_id`); `BTreeMap`/`BTreeSet` only; no float `==`.
- Every GUI edit is `By::User`; an edit outside the map is kept and reported (`edit_outside_map`), never deleted.
- Chrome palette and copy rules as in phase 2 (`theme::tokens`; sentence case, plain verbs, colour never the only
  cue). Map items keep the DayZ look.
- Keys (UI spec 4, already in `SHORTCUTS`): Terrain B raise (Shift lowers), R smooth, T flatten, Shift+R ramp
  (picks the ramp shape), Shift+C hole (not built in this phase: stays off the rail); V Area and C Line on every
  level; Shift+V picks the last shape again; `,` `.` brush smaller/larger, Ctrl+`,` Ctrl+`.` weaker/stronger
  (Alt+wheel, Ctrl+wheel too).
- Tests are deterministic: no sleeps (poll with `HANG_GUARD`), synthetic heightmaps or the existing fixtures, temp
  dirs; property tests with fixed seeds and regression files in the repo.
- `docs/manual/src/gui.md`, `project-file.md`, `shortcuts.md`, `settings.md` and `CHANGELOG.md` change in the
  commit that changes what they describe.

## Designer's calls in this plan (for the maintainer)

1. **Brush strokes are not held.** Spec 5.4 holds "the shape's core"; a stroke has no core, and holding a smoothed
   area would stop every road through it from being fitted. A `ground` stroke corrects the ground and the roads fit
   onto it; only `ground` **shapes** hold their core.
2. **Mound and pit follow the ground** (a raised or lowered copy of it, by `height_m`), pad, cut and fill go to one
   flat target, terrace is a flat shelf along a line, ramp runs from one height to another. That makes the nine
   shapes the spec names nine different results.
3. **A shape attached to a way that no longer exists is not applied** and is reported `edit_broken` (kept in the
   file). Deleting the way in the GUI deletes its shapes in the same step.
4. **The "Terrain changes" layer** shows the final terrain against the DEM (roads and hand edits together) and the
   shapes' outlines show which part is yours.

## Review Focus

1. **A shape whose ring is degenerate** (two points, all points on a line, a bow-tie, repeated points) or a line
   of zero length: parse rejects what cannot be drawn, apply never panics and changes nothing for a zero-area ring.
   Tests in Task 4 (`a_ring_needs_three_distinct_points`) and Task 5 (`a_bow_tie_ring_fills_even_odd`).
2. **The DEM source changes under an `offset` shape** (PR #25 fetch replaced the files): the shape keeps its height
   relative to the new ground. Test in Task 5 (`an_offset_follows_the_dem`).
3. **A `ground` shape under a road**: the held vertices keep the shape's height, the road is reported as a
   `road_conflict` that names hand-shaped ground, and the earthwork outside the shape is still made. Tests in
   Task 6 (`a_held_vertex_keeps_its_height_under_a_road`) and Task 7 (`a_pad_under_a_road_is_a_conflict`).
4. **An edit partly or wholly outside the map** after the area was moved: the part on the map is applied, a wholly
   outside one is reported and stays in the file byte for byte. Test in Task 7 (`a_terrain_edit_outside_the_map_is_kept`).
5. **Undo across both files**: deleting a way with an attached shape and pressing Ctrl+Z restores both files; a
   save that fails half-way (terrain written, ways not) leaves both as they were. Tests in Task 8
   (`undo_restores_a_way_and_its_shape`, `a_failed_second_save_restores_the_first_file`).

## Design notes (frontend-design pass)

**Subject:** the same map editor; the job here is to let the map maker fix ground the DEM got wrong and make room
for things, without leaving the map. The stack stays the one loud element. Terrain editing adds three quiet things:
the brush ring, shape outlines and the stage switch.

**On the map:** the brush is a 1.5 px `ink` circle at the brush radius (map metres → pixels) with a second, dotted
`ink-quiet` circle at the inner, full-strength radius (half the radius); while lowering (Shift) the outer ring is
dashed, and the status line says "Lower". A stroke being dragged is its swept outline in `ink-quiet`. A shape is
its outline: area shapes a 1.5 px `ink-quiet` ring with the `blend_m` band as a dotted ring outside it; line shapes
their two edges and the dotted band; an edited/drawn shape carries the same end tick as an edited way. Selected: the
`active` 2 px outline with square 7 px vertex handles. A `finish` shape draws its outline dashed, so ground and finish
can be told apart without colour. No fills.

**Selector, Earthwork group** (on every level, sentence-case header "Earthwork", after the level's own groups):

```
| Earthwork                   |
|  ▢     Pad            20 m  |   area shapes: their default plan outline, at the shared 2 px/m scale
|  ▽▢    Cut to a height 20 m  |
|  △▢    Fill to a height 20 m |
|  ◠     Mound          20 m  |
|  ◡     Pit            20 m  |
|  ═══   Ramp            6 m  |   line shapes: a line at their default width
|  ┅┅┅   Ditch           2 m  |
|  ███   Embankment      6 m  |
|  ▬▬▬   Terrace step    8 m  |
```

The thumbnail of an area shape is its default 20 m square footprint (a `Footprint` at the shared scale, so it
reads as the same drawer as the road pieces); a line shape is a `Line` at its default width. Picking one switches to
Area or Line; the status line says how to draw it.

**Status line while a terrain tool is active** (left: the tool's text; right: the stage switch):

```
Raise: drag to raise the ground, Shift lowers. Radius 20 m, 2.0 m (, . and Ctrl+, .)      ( Ground | Finish )
```

The switch is the design system's two-segment control, labelled "Ground" and "Finish", with a tooltip each
("Ground: corrects the ground the roads are fitted on; roads do not change it." / "Finish: shapes the finished
terrain after the roads."). It sets the stage of the next stroke or shape; a selected shape shows the same switch in
its properties.

**Checked against the defaults:** no new colours (the outline styles carry meaning by dash pattern, not hue), no
cards, no all-caps, no gradient heat maps for heights in the chrome. First draft coloured raise and lower rings
green and red; dropped (red against green, and colour as the only cue): the ring's dash says it.

## File structure

| File | Responsibility |
|---|---|
| `crates/naves-core/src/edits/terrain.rs` | `edits/terrain.toml`: types, parse, limits, load, save, operations |
| `crates/naves-core/src/edits/shape.rs` | applying strokes and shapes to a heightmap, the held mask |
| `crates/naves-core/src/conform.rs` | `conform_held`, `RoadConflict::held` |
| `crates/naves-core/src/features.rs`, `roadnet.rs` | `naves:earthwork=no` |
| `crates/naves-export/src/generate.rs` | stages in the pipeline, `Source::terrain`, `generate_from`, `Generated::{dem, terrain}` |
| `crates/naves-core/src/validate.rs` | terrain edit findings |
| `crates/naves-export/src/build.rs` | counts in `report.json` and the progress line |
| `crates/naves/src/gui/edit/mod.rs` | snapshot of both files, undo across them, change detection |
| `crates/naves/src/gui/edit/earthwork.rs` | the shape items, Area and Line for shapes, Shift+V, Shift+R |
| `crates/naves/src/gui/edit/brush.rs` | Raise/Lower, Smooth, Flatten, brush size and strength |
| `crates/naves/src/gui/edit/shapes.rs` | shape overlay, hit-test, properties, vertex move |
| `crates/naves/src/gui/worker.rs` | `TerrainPreviewJob`, `RoadJob` from a `Source` |
| `fuzz/fuzz_targets/terrain_edits_parse.rs` | fuzz target and corpus seed |

---

### Task 1: Phase 2 open points in the edit operations (M3, unused nodes, Bridge at a junction)

**Files:**
- Modify: `crates/naves/src/gui/edit/tools.rs` (`join`, `bridge_event`), `crates/naves-core/src/edits/ops.rs`,
  `CHANGELOG.md`
- Test: `crates/naves/src/gui/tests/edit.rs` (or `tests/roads.rs`, where the Join and Bridge tests are),
  `crates/naves-core/tests/edits.rs`

**Interfaces:**
- Produces: `WayEdits::prune_nodes(&mut self)`: removes every `NodeEdit` with a user id that no way edit lists;
  OSM node edits stay. Called at the end of `delete_way`, `modify_way` and `reset_way`.

- [ ] **Step 1: Write the failing tests**
  - `joining_across_a_gap_keeps_both_end_nodes` (M3): ways A `[1,2,3]` and B `[7,8,9]` of the same class, Join
    clicks node 3 then node 7 → A's nodes are `[1,2,3,7,8,9]`, B is deleted. (Today 7 is dropped.)
  - `joining_at_a_shared_node_lists_it_once`: A `[1,2,3]`, B `[3,8,9]` → `[1,2,3,8,9]` (already green, keep it).
  - `a_deleted_drawn_way_leaves_no_nodes`: draw a way with three new nodes, delete it → `edits.nodes` is empty.
  - `a_moved_osm_node_survives_pruning`: move an OSM node, delete a drawn way → the OSM node edit is still there.
  - `bridge_at_a_junction_takes_the_road_under_the_pointer`: ways 10 and 20 share node N; the pointer hovers way 20
    (the `hovered_way` of the overlay), first Bridge click on N → `pending.first.way == 20`.
- [ ] **Step 2: Run** `cargo test -p naves-core --test edits` and `cargo test -p naves --bin naves gui::tests`:
  the new tests FAIL.
- [ ] **Step 3: Implement.** In `join`, skip B's first node only when `a.0 == b.0`. In `bridge_event` (and Join's
  first click, same rule), order the hits so the hovered way comes first. `prune_nodes` as above.
- [ ] **Step 4: Run**: PASS. **Step 5:** CHANGELOG, Fixed: "Join no longer drops the end of the second road when
  the two ends are apart. Bridge on a junction takes the road under the pointer. Nodes of deleted roads are removed
  from `edits/ways.toml`."
- [ ] **Step 6: Commit** `fix: join keeps both ends, bridge takes the hovered road, unused nodes are pruned`.

### Task 2: Phase 2 open points in the selector (M7, M11, M12)

**Files:**
- Modify: `crates/naves/src/gui/palette.rs` (`all_items`, `in_scope`, `item_row`, `palette_ui`),
  `crates/naves/src/gui/mod.rs` (frame start), `CHANGELOG.md`
- Test: `crates/naves/src/gui/tests/palette.rs`

**Interfaces:**
- Produces: `NavesApp::items(&mut self) -> std::sync::Arc<Vec<PaletteItem>>`: the items of every level and the
  catalogue models, built at most once per frame (a cache cleared at the start of `update`). `all_items` becomes
  private to it; every caller (`palette_ui`, `has_palette`, `left_panel`, the command palette) uses `items()`.
  Later tasks add the earthwork items there.

- [ ] **Step 1: Write the failing tests**
  - `the_items_are_built_once_per_frame` (M7): a test-only counter in the cache; one `harness.step()` on the Roads
    level with the list shown → the counter grew by exactly 1.
  - `everything_keeps_the_road_classes_widest_first` (M11): Roads level, Everything scope → the `LineClass` items
    appear in `ROAD_CLASSES` order.
  - `the_size_column_clears_the_scroll_bar` (M12): a list longer than the panel; the rect of the "12 m" text of the
    first item ends left of `panel_rect.right() - ui.spacing().scroll.allocated_width()`.
- [ ] **Step 2: Run** `cargo test -p naves --bin naves gui::tests::palette`: FAIL.
- [ ] **Step 3: Implement.** Everything: sort only by `(fits, kind != LineClass, first level)` and keep the levels'
  own order inside a key (stable sort); models then by group and name. The row reserves the scroll bar's allocated
  width at its right edge.
- [ ] **Step 4: Run**: PASS. **Step 5:** CHANGELOG, Fixed: "In Everything the road classes stay widest first; the
  sizes are no longer hidden by the scroll bar."
- [ ] **Step 6: Commit** `fix: the item list is built once per frame, keeps the class order and clears the scroll bar`.

### Task 3: Phase 2 open points in keys, hints and accessibility (M13, M15, names, checkboxes) and one read of the extract (M6)

**Files:**
- Modify: `crates/naves/src/gui/shortcuts.rs` (the "works while typing" key list), `crates/naves/src/gui/worker.rs`
  (`RoadsState::NoDem`, `RoadJob::start`), `crates/naves/src/gui/mod.rs` (`reload_roads`, `regenerate`),
  `crates/naves/src/gui/levels/roads.rs` (`panel`), `crates/naves/src/gui/edit/properties.rs`,
  `crates/naves/src/gui/theme.rs` (checkbox visuals), `crates/naves-export/src/generate.rs`, `CHANGELOG.md`
- Test: `crates/naves/src/gui/tests/{shortcuts,roads,theme,stack,edit}.rs`, `crates/naves-export/tests/generate.rs`
  (the existing generate test file)

**Interfaces:**
- Produces: `pub fn generate_from(project_dir: &Path, project: &Project, tools: &Tools, stages: Stages, source:
  Option<Source>, progress: &dyn Fn(&str)) -> Result<Generated, BuildError>`; `generate_with` calls it with `None`.
  With `Some(source)`, `read_features` uses it instead of `read_source` (the extract is not read again).
  `RoadJob::start(dir: &Path, tools: Result<Tools, String>, source: Option<Source>)`.
  `RoadsState::NoDem`.

- [ ] **Step 1: Write the failing tests**
  - `enter_in_a_text_field_does_not_finish_a_line` (M13): Draw with two points, focus the search field, press
    Enter → still two points, `edits/ways.toml` absent. Space in the field types a space. Esc still cancels.
  - `without_elevation_files_the_roads_say_so` (M15): a project with OSM but no DEM → the Roads panel says "Add
    elevation files first: the roads are placed on them." and not the OpenStreetMap hint.
  - `the_way_properties_have_names` : `harness.get_by_label("Class")`, `"Surface"`, `"Layer"`, `"Bridge"`, `"Keep
    as drawn"`, `"Let road planning route it"` each find exactly one widget.
  - `stack_checkboxes_are_readable_on_dark`: `theme::contrast(check_stroke, tokens::PANEL) >= 3.0` and the same for
    the unchecked box's border, with the dark visuals.
  - `generate_from_does_not_read_the_extract_again` (M6): `read_source` on the fixture with roads, then replace its
    `.osm.pbf` with the fixture without roads, then `generate_from(.., Some(source), ..)` → `features` is not empty.
  - `the_roads_start_after_the_source` (M6): on load, `roads_job` is `None` until the session is `Ready`, then a
    `RoadJob` runs with that source.
- [ ] **Step 2: Run** the named tests: FAIL.
- [ ] **Step 3: Implement.** Enter and Space no longer fire while `typing(ctx)`; Escape and the Ctrl combinations of
  the list still do. Every property widget gets `.labelled_by(label.id)` from its row label. The checkbox colours
  come from `tokens::INK` (mark) and `tokens::INK_QUIET` (border) in dark mode. `reload_roads` starts the `RoadJob`
  when the `SourceJob` result arrives; `regenerate` passes `session.source.clone()`.
- [ ] **Step 4: Run**: PASS. **Step 5:** CHANGELOG, Fixed: "Enter in a text field no longer finishes a line.
  Without elevation files the Roads level says so. The road properties can be read by screen readers. The layer
  checkboxes are readable in the dark theme. The OpenStreetMap extract is read once per reload instead of twice."
- [ ] **Step 6: Commit** `fix: keys in text fields, the no-DEM hint, accessible properties, one read of the extract`.

### Task 4: `edits/terrain.toml`: the file and its operations

**Files:**
- Create: `crates/naves-core/src/edits/terrain.rs`, `fuzz/fuzz_targets/terrain_edits_parse.rs`,
  `fuzz/corpus/terrain_edits_parse/valid`
- Modify: `crates/naves-core/src/edits.rs` (module, re-exports), `fuzz/Cargo.toml`,
  `crates/naves-core/tests/fuzz_seeds.rs`, `docs/manual/src/project-file.md`, `CHANGELOG.md`
- Test: `crates/naves-core/tests/terrain_edits.rs`

**Interfaces:**
- Produces (all `pub`, serde, `#[serde(deny_unknown_fields)]`):
  - `enum Stage { Ground, Finish }` (`"ground"`, `"finish"`), `enum BrushTool { Raise, Lower, Smooth, Flatten }`,
    `enum ShapeKind { Pad, Cut, Fill, Mound, Pit, Ramp, Ditch, Embankment, Terrace }` (snake case) with
    `fn is_area(self) -> bool` (the first five);
  - `enum Height { Offset(f64), Level(f64) }`, serde as `{ offset = 1.5 }` / `{ level = 612.0 }`;
  - `enum Attached { Way(ElementId) }`, serde as `{ way = "u3" }` (models and bridges join in phase 4);
  - `struct Stroke { id: u64, tool: BrushTool, points: Vec<(f64, f64)>, radius_m: f64, strength: f64, stage: Stage
    (default Ground), by: By }`;
  - `struct Shape { id: u64, kind: ShapeKind, ring: Vec<(f64, f64)> (area kinds), line: Vec<(f64, f64)> (line
    kinds), width_m: Option<f64> (line kinds), target: Option<Height> (pad, cut, fill, terrace), from: Option<Height>,
    to: Option<Height> (ramp), height_m: Option<f64> (mound, pit, ditch depth, embankment), blend_m: f64 (default
    10.0), stage: Stage (default Ground), attached: Option<Attached>, by: By }`;
  - `struct TerrainEdits { format: u32 (1), next_id: u64 (1), strokes: Vec<Stroke>, shapes: Vec<Shape> }`, TOML
    arrays `[[stroke]]` and `[[shape]]`; `parse`, `validate`, `load(project_dir)`, `save(project_dir)` as
    `WayEdits` (missing file = empty; atomic write via `tempfile` in `edits/`);
  - operations, each validating first and leaving the set unchanged on error: `add_stroke(Stroke without id) ->
    u64`, `add_shape(Shape without id) -> u64` (take a `NewStroke`/`NewShape` struct without `id`),
    `replace_shape(id, Shape)`, `delete(id)`, `delete_attached(way: ElementId) -> usize`;
  - limits: `MAX_STROKES = 10_000`, `MAX_STROKE_POINTS = 5_000`, `MAX_SHAPES = 10_000`, `MAX_SHAPE_POINTS = 2_000`;
    `radius_m` 0.5..=500, `width_m` 0.5..=200, `blend_m` 0..=200, `height_m` 0..=100, `strength` for Raise/Lower
    0.01..=50 (metres at the centre), for Smooth/Flatten 0.01..=1; `Height::Offset` −500..=500, `Height::Level`
    −500..=9000; all finite; positions through `check_position`; ids unique and below `next_id`.

- [ ] **Step 1: Write the failing tests**
  - `a_terrain_file_round_trips`: one stroke of each tool and one shape of each kind → `save`, `load` equal.
  - `a_missing_file_is_no_edits`: `load` on an empty dir → `TerrainEdits::default()`.
  - `a_ring_needs_three_distinct_points`: a pad with 2 points, or 3 points of which two are equal → `Invalid`.
  - `each_kind_needs_its_fields`: a ramp without `from`, a pad with a `line`, a ditch without `width_m`, a mound
    with a `target` → `Invalid` with the shape's id in `what`.
  - `limits_are_enforced`: radius 501, 2 001 ring points, a NaN in a point, `next_id` not above an id → `Invalid`.
  - `ops_leave_the_set_unchanged_on_error`: `add_shape` with a bad ring → error, set equal to before.
  - `delete_attached_removes_only_that_ways_shapes`.
  - `the_ids_come_in_creation_order`: two `add_*` calls give ids 1 and 2 and `next_id == 3`.
  - fuzz seed test `terrain_edits_parse_seeds` in `fuzz_seeds.rs`.
- [ ] **Step 2: Run** `cargo test -p naves-core --test terrain_edits --test fuzz_seeds`: FAIL.
- [ ] **Step 3: Implement** as specified; the fuzz target calls `TerrainEdits::parse` on the input as UTF-8.
- [ ] **Step 4: Run**: PASS; `cargo +nightly fuzz run terrain_edits_parse -- -max_total_time=10` finds nothing.
- [ ] **Step 5: Docs.** `project-file.md`: a section "edits/terrain.toml" with one example of a stroke and of each
  geometry (area, line, ramp) and the table of kinds, fields and limits. CHANGELOG, Added: "`edits/terrain.toml`:
  brush strokes and earthwork shapes, in latitude and longitude and real metres."
- [ ] **Step 6: Commit** `feat: edits/terrain.toml, terrain strokes and earthwork shapes`.

### Task 5: Applying strokes and shapes to a heightmap

**Files:**
- Create: `crates/naves-core/src/edits/shape.rs`
- Modify: `crates/naves-core/src/edits.rs`
- Test: `crates/naves-core/tests/terrain_apply.rs` (+ its `proptest-regressions` file when proptest writes one)

**Interfaces:**
- Consumes: Task 4's types; `Heightmap`, `MapGeo`, `Scale`.
- Produces:
  - `pub enum TerrainIssue { Outside { id: u64 }, Detached { id: u64, way: ElementId } }`;
  - `pub struct TerrainReport { pub applied: usize, pub issues: Vec<(TerrainIssue, (f64, f64))> }` (position = first
    point, `(lat, lon)`), `Default`, with `fn merge(&mut self, other: TerrainReport)`;
  - `pub fn apply_terrain_edits(heights: &mut Heightmap, dem: &Heightmap, edits: &TerrainEdits, stage: Stage,
    frame: &MapGeo, scale: &Scale, way_exists: &dyn Fn(ElementId) -> bool) -> (TerrainReport, Vec<bool>)`: changes
    `heights` for the strokes and shapes of `stage` in id order and returns the held mask (`size * size`, true
    inside the core of a `ground` shape, all false for `Finish`).

The rules the tests pin (r, w, b are radius, width and `blend_m` times `scale.horizontal`; heights times
`scale.height`):

- falloff `f(t) = 1 − (3t² − 2t³)` for `t = d / r` clamped to 0..1, `d` the distance to the stroke's polyline (one
  point = a disc). Raise: `h += strength · f`; Lower: `h −= strength · f`. A stroke acts once along its path: the
  effect at a vertex is that of the nearest point of the path, not a sum per point.
- Smooth: `h = lerp(h, m, strength · f)`, `m` the mean of the heights before the stroke within `r / 4` (at least the
  four neighbours). Flatten: `h = lerp(h, h₀, strength · f)`, `h₀` the height before the stroke at its first point.
- A shape's core: inside the ring (even-odd) or within `w / 2` of the line. Its band: outside the core and within
  `b` of it, weight `f(dist / b)`. A vertex takes `lerp(h, shape_h, weight)` (weight 1 in the core).
- `shape_h`: Pad = target; Cut = `min(h, target)`; Fill = `max(h, target)`; Mound = `h + height_m`; Pit =
  `h − height_m`; Ditch = `h − height_m`; Embankment = `h + height_m`; Terrace = target; Ramp = `from` to `to`,
  linear in the length along the line of the nearest point.
- Reference ground of an `Offset`: the DEM at the ring's centroid (area kinds), at the first point (terrace, `from`)
  or the last point (`to`); `Level` is metres above sea level. Both times `scale.height`.
- Only the vertices of the edit's bounding box plus `r` or `w / 2 + b` are visited.
- An edit with no point on the map (none of its geometry's bounding box overlaps it) is `Outside` and not applied;
  an attached shape whose way does not exist is `Detached` and not applied. `applied` counts the others.

- [ ] **Step 1: Write the failing tests** (synthetic 65×65 heightmaps, 4 m cells, flat at 100 m unless said;
  positions made with `MapGeo::to_geo` of a fixture project):
  - `raise_lifts_the_centre_by_strength`: one-point stroke, strength 2, radius 20 → +2.0 at the point, +0 at 20 m,
    between at 10 m (`f(0.5) = 0.5` → +1.0 ± 1e-3).
  - `a_long_stroke_does_not_pile_up`: 50 points along one line, Raise 1 → no vertex above +1.0.
  - `lower_is_raise_inverted`; `smooth_removes_a_spike` (one vertex +10 → after strength 1 below +3);
    `flatten_pulls_to_the_first_point`.
  - `a_pad_is_flat_at_its_target` (offset +3 → every core vertex 103.0; band between; outside unchanged);
    `cut_only_lowers` and `fill_only_raises` on a 45° ramp of ground; `a_mound_follows_the_ground`;
    `a_ramp_runs_from_one_height_to_the_other` (from +0 to +6 over 60 m → +3 at the middle ± 0.05);
    `a_ditch_is_lower_by_its_depth`; `a_terrace_is_one_height_along_its_line`.
  - `a_bow_tie_ring_fills_even_odd` and `a_collinear_ring_changes_nothing` (no panic).
  - `an_offset_follows_the_dem`: same shape, DEM 100 m vs 250 m → core at 103 and 253.
  - `sizes_and_heights_follow_the_scale`: `horizontal = 0.5`, `height = 2` → a 20 m radius covers 10 map metres,
    strength 2 raises by 4.
  - `the_stages_are_separate`: a `finish` shape does nothing in the `Ground` pass and vice versa.
  - `the_held_mask_is_the_core_of_ground_shapes`: pad core vertices true, band false, strokes never true.
  - `an_edit_off_the_map_is_outside`; `a_shape_of_a_gone_way_is_detached`.
  - `the_order_is_the_id_order`: pad id 2 then raise id 1 differs from raise id 2 then pad id 1, and each matches
    applying them one by one in id order.
  - proptest `apply_never_panics_and_stays_finite` (fixed seed, 256 cases): random valid edits of any kind on a
    random finite heightmap → all heights finite.
- [ ] **Step 2: Run** `cargo test -p naves-core --test terrain_apply`: FAIL.
- [ ] **Step 3: Implement** in `shape.rs`; distances to rings and lines with the existing `lot::segment_distance`
  and `ring_contains`; parallel rows with `rayon` only if the test timings need it (results must not depend on it).
- [ ] **Step 4: Run**: PASS.
- [ ] **Step 5: Commit** `feat: apply terrain strokes and earthwork shapes to a heightmap`.

### Task 6: Held ground in `conform`, and `naves:earthwork=no`

**Files:**
- Modify: `crates/naves-core/src/conform.rs`, `crates/naves-core/src/validate.rs` (the `road_conflict` message),
  `crates/naves-core/src/features.rs` (`FeatureKind::Road::earthwork`), `crates/naves-core/src/roadnet.rs` (the
  stretch carries it), `crates/naves-export/src/generate.rs` (strips of such stretches are not given to `conform`)
- Test: `crates/naves-core/tests/conform.rs`, `crates/naves-core/tests/features.rs`

**Interfaces:**
- Produces: `pub fn conform_held(base: &Heightmap, strips: &[RoadStrip], patches: &[LevelPatch], areas:
  &[AreaPatch], bridges: &[BridgeSpan], held: &[bool]) -> Result<Conformed, ConformError>`; `conform_areas` calls it
  with `&[]` (empty = nothing held; a mask of the wrong length is `ConformError::BadHeightmap`).
  `RoadConflict { .., pub held: bool }` (false everywhere it is made today). Features: a road tagged
  `naves:earthwork=no` has `earthwork: false` (default true); its strips are left out of `conform` (the road still
  gets pieces and heights, and its misses are reported by `road_misses_terrain`).

- [ ] **Step 1: Write the failing tests**
  - `a_held_vertex_keeps_its_height_under_a_road`: a road strip across a 9×9 held block 3 m above the road → every
    held vertex equals `base`, the vertices of the road outside the block take the road's height.
  - `a_road_over_held_ground_is_a_conflict`: same → one `RoadConflict` with `held: true` and a difference ≈ 3 m;
    with the block at the road's height (within 0.3 m) → none.
  - `held_ground_is_not_flooded`: a road beside a held cliff block → no held vertex changes, the slope is made on
    the free side.
  - `nothing_held_is_conform_areas`: for the existing fixtures of `conform.rs`, `conform_held(.., &[])` and
    `conform_held(.., &vec![false; n])` equal `conform_areas` bit for bit.
  - `a_wrong_mask_is_an_error`.
  - `earthwork_no_is_read`: `naves:earthwork=no` → `earthwork == false`; absent or `yes` → true.
  - validate: a held conflict's message is "A road meets ground you shaped by hand {d:.1} m away from its height
    here".
- [ ] **Step 2: Run** `cargo test -p naves-core --test conform --test features --test validate`: FAIL.
- [ ] **Step 3: Implement.** In `claim_cores`, a held vertex becomes a core at its own `base` height with a rank
  above every road (the road candidates there lose and are compared for the conflict); `flood`, `clamp_flooded` and
  `round` skip held vertices; the "never raised under a deck" pass leaves them too.
- [ ] **Step 4: Run**: PASS (and the whole `conform` test file unchanged).
- [ ] **Step 5: Commit** `feat: conform holds hand-shaped ground; naves:earthwork=no switches a road's earthwork off`.

### Task 7: The build: stages, findings and counts

**Files:**
- Modify: `crates/naves-export/src/generate.rs` (`Source::terrain`, `read_source`, `generate_from`, `place_roads`,
  `lots_only`, `Generated`), `crates/naves-core/src/validate.rs` (`edit_violations` takes the terrain report),
  `crates/naves-export/src/build.rs` (`edit_counts`, `edits_json`, the progress line), `crates/naves/src/mcp.rs`
  (the build result), `crates/naves-core/src/render.rs` (`TerrainChanges` from `dem`),
  `crates/naves/src/gui/levels/terrain.rs` (the layer's tooltip), `docs/manual/src/project-file.md`,
  `docs/manual/src/cli.md`, `CHANGELOG.md`
- Test: `crates/naves-export/tests/generate.rs`, `crates/naves-export/tests/build.rs`

**Interfaces:**
- Consumes: Tasks 4–6.
- Produces: `Source::terrain: TerrainEdits` (loaded by `read_source`, error `BuildError::Edits` with the path
  `edits/terrain.toml`); `Generated::dem: Heightmap` (the DEM as sampled), `Generated::base` = the DEM with the
  `ground` edits (what the roads are fitted on, as its doc says), `Generated::terrain: TerrainReport`. Order in
  `generate_from`: `build_heightmap` → `dem` → ground pass (mask) → `place_roads` on that base with the mask passed
  to `conform_held` (also in `lots_only`) → finish pass on the conformed (or base) heightmap → everything after
  as today. Findings: `Outside` → `edit_outside_map` "Terrain edit {id} lies outside the map"; `Detached` →
  `edit_broken` "Terrain edit {id} belongs to way {way}, which is gone". `report.json` `edits` gets
  `"terrain_applied"`, `"terrain_broken"`, `"terrain_outside"`; the progress line and the MCP `build` result say
  "{n} terrain edits applied" when there are any.

- [ ] **Step 1: Write the failing tests** (the fixture project with roads, a fake toolchain as today):
  - `a_ground_pad_lifts_the_ground_the_roads_are_fitted_on`: a pad (offset +5) away from roads → `base` and
    `heightmap` +5 in its core, `dem` unchanged, `terrain.applied == 1`.
  - `a_pad_under_a_road_is_a_conflict`: a `ground` pad across a road → a `road_conflict` violation whose message
    names hand-shaped ground; the pad's core in `heightmap` is the pad's height.
  - `a_finish_shape_comes_after_the_roads`: a `finish` pit on a road → `heightmap` lower there than without it, the
    road's pieces as without it, `road_misses_terrain` reported.
  - `a_terrain_edit_outside_the_map_is_kept`: a shape 5 km away → `edit_outside_map`, `edits/terrain.toml` bytes
    unchanged after the build.
  - `earthwork_off_leaves_the_ground`: a drawn way with `naves:earthwork=no` → `heightmap == base` along it.
  - `a_build_without_terrain_edits_is_unchanged`: no `edits/terrain.toml` → `base == dem`, and `heightmap`, `objects` and `violations` equal those of a build with an
    empty `edits/terrain.toml`; the existing generate tests pass unchanged.
  - `the_report_counts_terrain_edits` in `build.rs` tests.
- [ ] **Step 2: Run** `cargo test -p naves-export --test generate --test build`: FAIL.
- [ ] **Step 3: Implement** as specified. `way_exists` checks the edited ways (`Source::ways`) by raw id.
- [ ] **Step 4: Run**: PASS; `make test` green.
- [ ] **Step 5: Docs.** `project-file.md`: what each stage does in the build. `cli.md`: the new report fields.
  Terrain layer tooltip: "What the roads and your terrain edits did to the ground: cut (earth) and fill (blue)."
  CHANGELOG, Added: "Builds apply `edits/terrain.toml`: ground edits before the roads (roads do not change their
  shapes), finish edits after them; the report counts them."
- [ ] **Step 6: Commit** `feat: builds apply terrain edits in their stage`.

### Task 8: The edit session holds both files

**Files:**
- Modify: `crates/naves/src/gui/edit/mod.rs`, `crates/naves/src/gui/mod.rs` (`edit_way`, new `edit_terrain`,
  delete of a way), `crates/naves/src/gui/worker.rs` (`MapData::dem`, `HeightmapJob`, `TerrainPreviewJob`),
  `CHANGELOG.md`
- Test: `crates/naves/src/gui/tests/edit.rs`, new `crates/naves/src/gui/tests/terrain.rs`

**Interfaces:**
- Consumes: `Source::terrain` (Task 7), `apply_terrain_edits` (Task 5).
- Produces:
  - `pub struct Snapshot { pub ways: WayEdits, pub terrain: TerrainEdits }`; the undo and redo stacks hold
    `Snapshot`s; `EditSession::apply(dir, op: impl FnOnce(&mut Snapshot, &Source) -> Result<(), EditsError>)`
    saves only the files whose part changed (ways first, then terrain); if the second save fails, the first file
    is written back from the old snapshot and the error says "Could not save the edit: …".
  - change detection (M10): the session keeps the FNV `fingerprint` of each file's text as last read or written;
    before a save it reads the file again, and if it differs the edit is refused with "edits/{file} was changed
    outside Náves; it is read again" and the session reloads (`SourceJob`).
  - `NavesApp::edit_terrain(op)` as `edit_way`; deleting a way also runs `terrain.delete_attached(way)` in the
    same step.
  - `MapData { dem: Heightmap, heightmap: Heightmap (dem with both stages of the terrain edits, no roads), .. }`;
    `TerrainPreviewJob::start(data: Arc<MapData>, edits: TerrainEdits, project: Project)` recomputes `heightmap`
    after a terrain edit; the map shows it when it arrives (the roads layer keeps the last build until Ctrl+R).

- [ ] **Step 1: Write the failing tests**
  - `undo_restores_a_way_and_its_shape`: a drawn way with an attached pad; delete the way → both gone from their
    files; Ctrl+Z → both back, byte for byte.
  - `a_failed_second_save_restores_the_first_file`: make `edits/terrain.toml` a directory, delete a way with a
    shape → error shown, `ways.toml` as before.
  - `an_outside_change_is_not_overwritten` (M10): change `ways.toml` on disk, then move a node in the GUI → the
    file keeps the outside text, the problem names the file, the session reloads and shows the outside change.
  - `a_terrain_edit_previews_on_the_map`: add a pad through `edit_terrain` → after polling the preview job, the
    map's `heightmap` is +offset in the core, `dem` unchanged.
  - existing edit tests stay green (they use `WayEdits` through `Snapshot::ways`).
- [ ] **Step 2: Run** `cargo test -p naves --bin naves gui::tests::edit gui::tests::terrain`: FAIL.
- [ ] **Step 3: Implement** as specified.
- [ ] **Step 4: Run**: PASS. **Step 5:** CHANGELOG, Fixed: "The editor no longer overwrites changes made to the
  edit files outside Náves; it reads them again."
- [ ] **Step 6: Commit** `feat: the edit session holds the road and terrain edits as one undo step`.

### Task 9: Earthwork shapes in the selector, Area and Line for shapes (frontend-design first)

**Files:**
- Create: `crates/naves/src/gui/edit/earthwork.rs`
- Modify: `crates/naves/src/gui/palette.rs` (`items()` adds the shapes; group "Earthwork" after the level's own),
  `crates/naves/src/gui/mod.rs` (`left_panel`: list then the level's panel on every level; `route`: a picked shape's
  tools win), `crates/naves/src/gui/shortcuts.rs` (`EarthworkShape` built), `crates/naves/src/gui/levels/terrain.rs`
  (Shift+R), `docs/manual/src/gui.md`, `docs/manual/src/shortcuts.md`, `CHANGELOG.md`
- Test: `crates/naves/src/gui/tests/earthwork.rs`

**Interfaces:**
- Consumes: `NavesApp::edit_terrain`, `TerrainEdits::add_shape` (`NewShape`).
- Produces: `pub const SHAPES: &[ShapeItem]` with `ShapeItem { kind: ShapeKind, name: &'static str, size_m: f64 }`
  in the order of the design notes; item ids `earthwork.<kind>`, `kind: ItemKind::Preset`, `fits` = every built
  level (Terrain first), `default_tool` AREA for area kinds and LINE for line kinds; `pub const EARTHWORK_TOOLS:
  &[ToolDef]` (Area, Line) consulted by `route` and `status_text` before the level's own tools when the picked item
  is a shape. Defaults of a new shape: pad/cut/fill target `Offset(0.0)`, terrace `Offset(0.0)`, ramp `from
  Offset(0.0)` `to Offset(3.0)`, mound/pit `height_m` 2, ditch 1 (width 2), embankment 2 (width 6), ramp width 6,
  terrace width 8, `blend_m` 10, stage from the status-line switch (default Ground). Area drawing: clicks make a ring
  (Space, Enter, right click or double click finish; at least three points), a drag makes a rectangle, Alt+drag a
  circle of 32 points. Line drawing: as Draw (Task 1 of phase 2), at least two points.

- [ ] **Step 1: Write the failing tests**
  - `every_level_lists_the_earthwork_group`: on Project, Terrain, Ground and Roads, Level scope → a header
    "Earthwork" and the nine names in order.
  - `picking_a_pad_switches_to_area`; `picking_a_ditch_switches_to_line`; `on_roads_a_line_shape_draws_a_shape_not_a_road`
    (pick Ditch on Roads, click 2 points, Space → `terrain.toml` has a ditch, `ways.toml` absent).
  - `four_clicks_and_space_make_a_pad`: the shape's ring has 4 points in lat/lon that project back within 0.01 m.
  - `a_drag_makes_a_rectangle` and `alt_drag_makes_a_circle` (32 points, radius within 1 %).
  - `two_points_are_not_an_area`: Space after two clicks → nothing saved, the status line says "Click at least three
    points."
  - `shift_v_picks_the_last_shape_again` (on any level, after a ditch was used; nothing used yet → nothing
    happens and the status line says "No shape used yet.").
  - `shift_r_picks_the_ramp` on Terrain.
  - `the_level_panel_stays_under_the_list` (the Terrain numbers still show).
  - `the_manual_lists_every_shortcut` stays green.
- [ ] **Step 2: Run** `cargo test -p naves --bin naves gui::tests::earthwork`: FAIL.
- [ ] **Step 3: Implement** as specified; thumbnails per the design notes (`Footprint` 20 m square for area kinds,
  `Line` at the default width for line kinds).
- [ ] **Step 4: Run**: PASS. **Step 5: Docs.** `gui.md` "Earthwork" (the shapes, drawing, Shift+V); `shortcuts.md`;
  CHANGELOG, Added: "Earthwork on every level: pick a shape in the item list and draw it with Area or Line."
- [ ] **Step 6: Commit** `feat: earthwork shapes in the item list, drawn with Area and Line on every level`.

### Task 10: Terrain brushes (frontend-design first)

**Files:**
- Create: `crates/naves/src/gui/edit/brush.rs`
- Modify: `crates/naves/src/gui/levels/terrain.rs` (tools BRUSH, SMOOTH, FLATTEN, RAMP on the rail),
  `crates/naves/src/gui/shortcuts.rs` (brush size and strength actions built while a brush is the tool; Alt+wheel
  and Ctrl+wheel), `crates/naves/src/gui/mod.rs` (status line with the stage switch, the brush ring in the overlay),
  `crates/naves-core/src/prefs.rs` (`MapPrefs::brush`), `crates/naves/src/gui/prefs_window.rs` (Map tab),
  `docs/manual/src/gui.md`, `docs/manual/src/settings.md`, `docs/manual/src/shortcuts.md`, `CHANGELOG.md`
- Test: `crates/naves/src/gui/tests/brush.rs`, `crates/naves-core/tests/prefs.rs`

**Interfaces:**
- Produces: `pub struct BrushPrefs { pub radius_m: f64 (20), pub raise_m: f64 (2.0), pub strength: f64 (0.5) }`
  as `MapPrefs::brush` (serde default; limits as Task 4); the stage switch state `NavesApp::stage: Stage` (not saved).
  One drag (Press → Drag → Release) is one stroke and one edit: points every `radius / 4` map metres, converted with
  `to_geo`; a click without a drag is a one-point stroke. B with Shift held at Press makes a Lower stroke. `,` and
  `.` scale the radius by 0.8 and 1.25 (clamped 0.5–500 m); Ctrl+`,` and Ctrl+`.` the strength the same way (raise
  0.01–50 m, smooth/flatten 0.01–1).

- [ ] **Step 1: Write the failing tests**
  - `a_drag_is_one_stroke`: B, drag over 60 m → one `Raise` stroke with ≥ 4 points, radius and strength from the
    prefs, stage Ground; one Ctrl+Z removes it.
  - `shift_lowers`; `r_smooths_and_t_flattens`.
  - `brackets_change_the_radius_and_the_prefs_keep_it`: `.` twice → 31.25 m, saved in the settings file.
  - `ctrl_brackets_change_the_strength`.
  - `the_stage_switch_sets_the_next_stroke`: click "Finish" in the status line → the next stroke has `stage = finish`.
  - `the_brush_ring_follows_the_pointer`: hover → the overlay has a circle of the radius at the pointer; dashed with
    Shift held.
  - `brush_keys_do_nothing_without_a_brush` (in Select, `.` changes nothing).
  - prefs: `brush_prefs_round_trip` and `bad_brush_prefs_fall_back` (radius 0 → default, with a message).
- [ ] **Step 2: Run** `cargo test -p naves --bin naves gui::tests::brush` and `cargo test -p naves-core --test
  prefs`: FAIL.
- [ ] **Step 3: Implement** as specified; the status line text per the design notes ("Raise: drag to raise the
  ground, Shift lowers. Radius 20 m, 2.0 m (, . and Ctrl+, .)"; Smooth and Flatten say "strength 50 %").
- [ ] **Step 4: Run**: PASS. **Step 5: Docs.** `gui.md` "Shaping the terrain" (brushes, stages); `settings.md`
  (brush defaults); `shortcuts.md` (brush keys now work); CHANGELOG, Added: "Terrain brushes: raise, lower, smooth
  and flatten, each stroke saved in `edits/terrain.toml` as ground or finish."
- [ ] **Step 6: Commit** `feat: terrain brushes on the Terrain level`.

### Task 11: Shapes on the map: outline, selection, properties, moving a vertex (frontend-design first)

**Files:**
- Create: `crates/naves/src/gui/edit/shapes.rs`
- Modify: `crates/naves/src/gui/edit/overlay.rs` (`OverlayShape::{Ring, Band}` with a `dashed` flag), the overlay
  painter in `crates/naves/src/gui/viewport/map.rs`, `crates/naves/src/gui/edit/hit.rs` (shapes), the Terrain
  level's `pick`, `crates/naves/src/gui/mod.rs` (selection of a shape, Delete, Move on a shape's vertex),
  `crates/naves/src/gui/edit/properties.rs` (`shape_properties`, the way's "Shape the ground to fit" switch),
  `docs/manual/src/gui.md`, `CHANGELOG.md`
- Test: `crates/naves/src/gui/tests/shapes.rs`

**Interfaces:**
- Produces: `Target::Shape(u64)` in the selection; `pub fn shape_properties(ui, shape: &Shape, attached_name:
  Option<String>) -> Option<ShapeChange>` with `ShapeChange::{Target(Height), From(Height), To(Height),
  HeightM(f64), WidthM(f64), BlendM(f64), Stage(Stage), Delete}`, each one edit; `WayChange::Earthwork(bool)`
  (sets or removes `naves:earthwork=no`). Picking: on the Terrain level a click inside a shape's core or on its line
  selects the shape before the terrain point; on other levels only a click within the hit tolerance of its outline,
  and only when the level's own pick finds nothing. G on a selected shape drags one vertex (one edit on release).

- [ ] **Step 1: Write the failing tests**
  - `a_pad_is_drawn_as_its_outline_and_band`: the overlay has a `Ring` (not dashed) and a `Band` for a ground pad,
    dashed for a finish pad.
  - `a_click_inside_a_pad_selects_it_on_terrain`; `on_roads_a_road_wins_over_a_shape_it_crosses`.
  - `the_properties_change_one_thing_per_edit`: change the offset to 2.5, then the stage → two undo steps, the file
    has both.
  - `the_properties_say_what_a_shape_serves`: an attached pad shows "Shapes the ground for way u3"; an unattached one
    "Earthwork only".
  - `x_deletes_the_selected_shape`; `g_moves_one_vertex`.
  - `shape_the_ground_to_fit_switches_earthwork_off`: unticking it on a way sets `naves:earthwork=no`; ticking
    removes the tag.
  - every property widget has an accessible name (as Task 3).
- [ ] **Step 2: Run** `cargo test -p naves --bin naves gui::tests::shapes`: FAIL.
- [ ] **Step 3: Implement** as specified; numbers right-aligned, offsets in real metres with the unit, "Height above
  the ground here" / "Height above sea level" as the two choices of a `Height`.
- [ ] **Step 4: Run**: PASS. **Step 5: Docs.** `gui.md` (selecting and changing shapes, the way switch); CHANGELOG,
  Added: "Earthwork shapes can be selected, changed, moved and deleted; a road's properties can switch its
  earthwork off."
- [ ] **Step 6: Commit** `feat: select, change and move earthwork shapes`.

### Task 12: End-to-end check

**Files:** none new (fixes go into the task they belong to, as their own `fix:` commits).

- [ ] **Step 1:** On a copy of the working example project in `/var/tmp` (2048 map), draw in the GUI: a ground pad
  beside a road, a ground pad across a road, a finish ditch, one raise and one smooth stroke. Ctrl+R.
- [ ] **Step 2:** Check: the report counts five terrain edits applied; the road across the pad is a `road_conflict`
  naming hand-shaped ground; the Terrain changes layer shows the pads and the ditch; undo of every step restores
  `edits/` to empty files; a screenshot of the map with the outlines for the maintainer.
- [ ] **Step 3:** `make check` green on the branch; coverage at or above 85 %.
- [ ] **Step 4:** In-game check is not needed for this phase (only heights change; the grid and formats are those of
  every build). If the maintainer asks for one, follow AGENTS.md "Testing a built map in the game".

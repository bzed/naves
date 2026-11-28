# Road line editing in the GUI (editing phase 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
> Every agent reads `/home/bzed/.claude/skills/linux-basics/SKILL.md` and follows it (at most half the cores:
> `CARGO_BUILD_JOBS=12`, `nice -n 19 ionice -c 3`, `systemd-run --user --scope -p CPUQuota=1200% -p MemoryMax=16G
> -p MemorySwapMax=1G`; no root; no pagers or editors). A UI task starts with `/frontend-design:frontend-design`
> (AGENTS.md) and follows the design notes below.

**Goal:** Draw, move, cut, join, delete and retag roads on the map in the GUI, using a reusable item selector that
shows what fits the active level or everything. Each edit is saved at once to `edits/ways.toml`, can be undone, and Ctrl+R rebuilds.

**Architecture:** The GUI edits the *source layer*: the OSM ways with the edits applied (`apply_way_edits`), held in
an `EditSession` beside the built roads. A background job reads the extract once. Every edit runs one
`WayEdits` operation from phase 1, saves the file atomically, re-applies the edits and redraws the overlay; the built
roads stay as they were until Ctrl+R. The selector is one widget fed by `PaletteItem`s that say which levels they `fit`;
its scope switch filters them.

**Tech Stack:** Rust 1.99, egui/eframe with the vendored `naves-ui` design system, `egui_kittest` for widget tests,
`naves_core::edits`, `naves_export::generate`, `naves_export::catalog_read::road_catalog`.

**Spec:** `docs/superpowers/specs/2026-11-27-editing-design.md` (sections 5.1, 5.2 and 5.6, phase 2), with the UI
spec `docs/superpowers/specs/2026-11-06-naves-editor-ui.md` (levels, rail, shortcuts) still in force.

## Global Constraints

- AGPL-3.0-or-later SPDX header on every new file; `make check` passes after every task (fmt, clippy `-D warnings`,
  doc, reuse, deny, test, fuzz, coverage ≥ 85 %, mdbook).
- No panics on file or user input in library code; `#[expect(..., reason)]`, never `#[allow]`.
- Positions in the edit set are `(lat, lon)`; the GUI converts with `MapGeo::to_map` / `MapGeo::to_geo`
  (`naves_core::heightmap`). Nothing in map metres is ever written to `edits/ways.toml`.
- Every edit made in the GUI is `By::User` and gets `naves:keep=yes` (done by the phase 1 operations).
- Chrome palette (dark default), copied from spec 5.6: ground `1e2329`, panel `272d35`, raised `323a44`, ink
  `e4e8ec`, ink-quiet `9aa4ae`, active `56b4e9`, problem `d55e00` (always with ▲ and a word), notice `f0e442`
  (always with ●). Level bands: Terrain `cc79a7`, Ground `009e73`, Water `0072b2`, Roads `e4e8ec`; the others keep
  their hue but pass 3:1 against ground. Colour is never the only cue. Map items keep the DayZ look.
- Keys: the UI spec section 4 as revised on 2026-11-28 (Task 1 makes the table match it). Left hand on the keyboard,
  right hand on the mouse: WASD pan (3D: move), Q/E zoom (3D: down/up), Shift ×3 faster, Ctrl ×3 slower; Esc
  select/cancel, G move, C draw (line), X delete, Roads R cut, T join, Shift+T bridge; Space finish (Enter, right
  click, double click too); Ctrl+Z / Ctrl+Shift+Z undo/redo (Ctrl+Z while drawing takes back the last point);
  Ctrl+R regenerate (F5 too); Ctrl+F search. A set that does not fit moves to the right half whole.
- Copy rules: sentence case, plain verbs, no all-caps labels, the action word stays the same through a flow ("Draw"
  in the rail, "Drawn by you" in the properties).
- Tests are deterministic: no sleeps (poll with `HANG_GUARD`), synthetic or the existing fixtures only, temp dirs.
- GUI chapter `docs/manual/src/gui.md`, `shortcuts.md`, `settings.md` and `CHANGELOG.md` are updated in the commit
  that changes what they describe.

## Review Focus

1. **The extract changes under a session** (the user swaps `sources.osm` or moves the area while edits are loaded):
   the session reloads from disk and keeps the edits file as it is. Test in Task 4
   (`a_source_change_reloads_the_session_and_keeps_the_edits`).
2. **Saving fails** (read-only project folder): the in-memory set returns to the state before the operation, the
   problem list says "Could not save the edit: …", and undo still works. Test in Task 4 (`a_failed_save_undoes_the_edit`).
3. **A map without OSM** (no `sources.osm`): the Roads level still has the selector and Draw, and the first drawn
   way builds. Test in Task 7 (`drawing_works_without_osm`).
4. **A click with nothing near it** in Cut, Join, Bridge or Move: nothing changes, nothing is saved, the status line
   says what to click. Test in Task 8 (`a_click_on_nothing_changes_nothing`).
5. **Picking an item of another level** in Everything scope: the active level stays, the tool works with the item
   (a road class picked on Buildings still draws a road), and Esc goes back to Select on the active level. Test in
   Task 6 (`an_item_of_another_level_keeps_the_active_level`).

## Design notes (frontend-design pass)

**Subject:** a map editor for one person who builds DayZ terrains from OSM and fixes what the generator got wrong.
The job of the screen is to show *the map as built* and *what you changed* at the same time, and to make the
change cheap. The stack (levels) stays the one loud element; editing adds nothing loud.

**Colour:** the chrome tokens above, and nothing else. On the map: the source ways of the active level are drawn
as a 2 px ink-quiet line over the built roads (so that where the build moved a line is visible); an edited or drawn
way is an `active` line with a small square tick at each end; the selection is an `active` 2 px outline with
square 7 px node handles (ground fill, active border); a line being drawn is ink, dashed, with its handles. No
fills, no glows.

**Type:** the design system's Inter, its own scale; numbers in the properties right-aligned. No new font.

**Layout:**

```
+-- left panel (300) ----------+-rail-+------------- map ---------------+-- right (230) ------+
| [ Search roads      Ctrl+F ] | Esc  |                                 | stack               |
| ( Roads | Everything )       |Select|      built roads (DayZ look)    |                     |
| Last used                    |  G   |      source line, ink-quiet     | Properties          |
|  ━━━━  Secondary             | Move |      edited line, active +ticks | Secondary road      |
| Roads                        |  C   |      ■──■──■ selected           | Drawn by you        |
|  ━━━━━ Primary        12 m   | Draw |                                 | Class   [Secondary] |
|  ━━━━  Secondary       9 m   |  R   |                                 | Surface [asphalt  ] |
|  ──    Track, grade 2  5 m   | Cut  |                                 | Bridge  [ ]  Layer 0|
| Pieces                       |  T   |                                 | Keep as drawn [x]   |
|  ▭▭    asf2 12 m straight    | Join |                                 | Let planning route [ ]
|  ◠     asf2 arc 25 m         |  ⇧T  |                                 | [Reset to OSM]      |
| Bridges                      |Bridge|                                 |                     |
|  ▭▭▭▭  bridge_concrete3 32 m |      |                                 |                     |
+------------------------------+------+---------------------------------+---------------------+
status: Edited: press Ctrl+R to rebuild.
```

Left-aligned throughout. The scope switch is a two-segment control directly under search, labelled with the active
level's name and "Everything", not "This level" / "All".

**The one specific choice (the selector's thumbnails):** every thumbnail is drawn at **one shared scale** (2 px per
metre, clipped at the column's 64 px with a fade), as the item's plan view: a road class as its line at true width,
a road piece as its footprint between its two ports, a bridge as its deck outline. So a 12 m asphalt piece is visibly
wider than a 5 m track, and the list reads like a parts drawer, not like a list of names. The width or length in
metres sits at the right edge in ink-quiet. In Everything scope an item of another level shows that level's band
chip (digit on band colour) before the metres.

**Checked against the defaults:** no cards, no shadows, no all-caps group headers (the list's headers are sentence
case), no numbered markers, no monospace data labels, no "→" on buttons; a dark chrome with one sky-blue accent is
the spec's decision, set by colour-vision safety, not by taste. The first draft had a "Fits this level / All
models" checkbox; it was changed to the named two-segment switch because a checkbox hides which level "this" is.

## File structure

| File | Responsibility |
|---|---|
| `crates/naves/src/gui/theme.rs` | chrome tokens, dark default, contrast helpers |
| `crates/naves-core/src/prefs.rs` | `MapPrefs::theme`, `MapPrefs::selector_scope` |
| `crates/naves-export/src/generate.rs` | `read_source` (the extract and the edits, no placement) |
| `crates/naves/src/gui/edit/mod.rs` | `EditSession`: the source layer, operations, save, undo/redo |
| `crates/naves/src/gui/edit/hit.rs` | hit-testing ways and nodes in map metres |
| `crates/naves/src/gui/edit/overlay.rs` | the shapes of the editing overlay |
| `crates/naves/src/gui/edit/tools.rs` | Move, Draw, Cut, Join, Bridge, Delete on view events |
| `crates/naves/src/gui/edit/properties.rs` | the properties of a selected way |
| `crates/naves/src/gui/palette.rs` | the reusable selector: `fits`, scope, footprint thumbnails |
| `crates/naves/src/gui/levels/roads.rs` | Roads level: items, tools, pick |
| `crates/naves/src/gui/worker.rs` | `SourceJob`, `CatalogJob` |
| `crates/naves/src/gui/tests/edit.rs`, `tests/palette.rs` | widget and behaviour tests |

---

### Task 1: Left-hand default shortcuts and moving the view with WASD

**Files:**
- Modify: `crates/naves/src/gui/shortcuts.rs` (`SHORTCUTS`, new actions), `crates/naves/src/gui/mod.rs` (held-key
  movement each frame), `crates/naves/src/gui/viewport/mod.rs` (movement through `focus`/`set_focus`),
  `docs/manual/src/shortcuts.md`, `docs/manual/src/gui.md`, `CHANGELOG.md`
- Test: `crates/naves/src/gui/tests/shortcuts.rs`, `crates/naves/src/gui/tests/viewport.rs`

**Interfaces:**
- Produces: the table of the UI spec section 4 (2026-11-28) exactly; `Action::{PanNorth, PanWest, PanSouth,
  PanEast, ZoomOut, ZoomIn}` as **held** actions (they act every frame while down, not once); `pub const
  PAN_VIEWS_PER_S: f64 = 0.75` (fraction of the view's span per second), `pub const ZOOM_PER_S: f64 = 2.0` (factor
  per second), `pub const FAST: f64 = 3.0`, `pub const SLOW: f64 = 1.0 / 3.0`; a second row per action where the spec
  names a second key (ids `<id>.alt`, e.g. `finish.alt`, `edit.delete.alt`, `regenerate.alt`, `level.objects.alt`).
  Select has no letter: Esc is Select. Shortcut ids of existing rows stay, so a user's saved changes still apply.

- [ ] **Step 1: Write the failing tests**
  - `every_default_used_with_the_mouse_is_on_the_left_half`: `LEFT = Num1..=Num6, Q W E R T A S D F G X C V B,
    Escape, Tab, Space`; every row that is not an `.alt` row and whose action is not in a right-half set
    (`HideLevel`, `ShowOthersInFull`, `BrushSmaller`, `BrushLarger`, `BrushWeaker`, `BrushStronger`, `NextProblem`,
    `PreviousProblem`, `Help`, `Settings`, `CommandPalette`) has its key in `LEFT`.
  - `a_right_half_set_moves_whole`: for each set (`HideLevel`/`ShowOthersInFull`, the four brush keys,
    `NextProblem`/`PreviousProblem`) the keys are all left or all right.
  - `the_table_is_the_spec` (replaces `every_key_of_the_spec_is_in_the_table`): the spec's rows, typed out in the
    test, equal the table's defaults.
  - `holding_d_pans_east`: hold D for 30 frames → the view's centre x grew by `30 × dt × PAN_VIEWS_PER_S × span`
    (within 1 %), y unchanged; with Shift held the move is `FAST` times that, with Ctrl `SLOW` times.
  - `q_and_e_zoom`: holding E for 30 frames makes the span smaller by the factor `ZOOM_PER_S^(30 dt)`; Q the inverse.
  - `wasd_does_nothing_while_typing`: with the project name field focused, holding W moves nothing and types "w".
  - `esc_is_select`: in Draw, Esc → tool Select; the rail shows "Esc" over "Select".
  - `alt_digits_reach_the_upper_levels`: Alt+1 goes to level 7 when it is built and does nothing while it is not.
  - existing and still green: `no_key_is_assigned_twice_in_a_scope`, `no_level_key_repeats_an_everywhere_key`,
    `ids_are_unique_and_pinned` (with the new `.alt` ids), `no_default_needs_shift_or_altgr_on_qwertz`,
    `the_manual_lists_every_shortcut`.
- [ ] **Step 2: Run** `cargo test -p naves --bin naves gui::tests::shortcuts gui::tests::viewport`: FAIL.
- [ ] **Step 3: Implement.** Held actions are read with `ctx.input(|i| i.key_down(..))` once per frame, skipped
  while `typing(ctx)`, scaled by `i.stable_dt` and applied to the active view through `focus()`/`set_focus()`, so the
  map and the 3D view both move and the fake view in tests sees it. Request a repaint while a movement key is down.
  Shift and Ctrl change the speed only for the movement keys.
- [ ] **Step 4: Run**: PASS. **Step 5: Docs** (`shortcuts.md` is the table; `gui.md` "Moving around") + CHANGELOG
  ("Changed: the default shortcuts are on the left half of the keyboard, so the right hand stays on the mouse; WASD
  pans, Q and E zoom, Shift and Ctrl make it faster and slower. Select is Esc, Move is G, Line is C, Area is V,
  Delete is X. Shortcuts you changed in the settings stay.").
- [ ] **Step 6: Commit** `feat: left-hand default shortcuts, WASD and Q/E move the view`.

### Task 2: Dark theme and the chrome tokens

**Files:**
- Modify: `crates/naves/src/gui/theme.rs`, `crates/naves/src/gui/levels/mod.rs` (`Level::colour`),
  `crates/naves-core/src/prefs.rs` (`MapPrefs`), `crates/naves/src/gui/prefs_window.rs` (Map tab),
  `docs/manual/src/settings.md`, `docs/manual/src/gui.md`, `CHANGELOG.md`
- Test: `crates/naves/src/gui/tests/theme.rs`, `crates/naves-core/tests/prefs.rs` (or the existing prefs test file)

**Interfaces:**
- Produces: `pub mod tokens { pub const GROUND, PANEL, RAISED, INK, INK_QUIET, ACTIVE, PROBLEM, NOTICE: Color32 }`
  in `theme.rs`; `pub enum ThemeChoice { Dark, Light, System }` (serde `"dark"`, `"light"`, `"system"`, default
  `Dark`) as `MapPrefs::theme`; `pub fn apply_theme(ctx: &egui::Context, choice: ThemeChoice)`. `GREEN` and
  `ERROR` are replaced by `ACTIVE` and `PROBLEM` at every use.

- [ ] **Step 1: Write the failing tests**
  - `the_default_theme_is_dark`: a fresh `NavesApp` in the harness has `ctx.style().visuals.dark_mode == true` and
    `visuals.panel_fill == tokens::PANEL`.
  - `light_can_be_chosen_in_the_map_tab`: set the Map tab's theme to Light, the next frame has `dark_mode == false`,
    and the saved prefs contain `theme = "light"`.
  - `chrome_tokens_read`: `contrast(INK, GROUND) >= 7.0`, `contrast(INK_QUIET, GROUND) >= 4.5`,
    `contrast(ACTIVE, GROUND) >= 3.0`, `contrast(PROBLEM, GROUND) >= 3.0`.
  - `bands_read_on_ground`: for every `Level::ALL`, `contrast(level.colour(), GROUND) >= 3.0`, and the four fixed
    bands are exactly `cc79a7`, `009e73`, `0072b2`, `e4e8ec`.
  - prefs: `theme_round_trips` (`"dark"`, `"light"`, `"system"`), and the default `Dark` is not written.
- [ ] **Step 2: Run** `cargo test -p naves --bin naves gui::tests::theme` and the prefs test: FAIL (no `tokens`, no
  `theme`).
- [ ] **Step 3: Implement.** `apply_theme` calls `naves_ui::apply_style_and_install_loaders`, then
  `ctx.set_theme(egui::ThemePreference::{Dark, Light, System})`, then for the dark visuals overrides
  `panel_fill`, `window_fill`, `extreme_bg_color` (GROUND), `faint_bg_color` (RAISED), `selection.bg_fill` and the
  focus stroke (ACTIVE). The vendored crate stays unchanged. Light keeps the design system's light visuals with
  ACTIVE and PROBLEM as accents (both pass 3:1 on white; test it).
- [ ] **Step 4: Run** the tests: PASS. **Step 5: Docs** (Map tab: Theme) + CHANGELOG ("The GUI is dark by
  default; light and the system theme are in Settings, Map.").
- [ ] **Step 6: Commit** `feat: the GUI is dark by default, with colour-blind safe chrome`.

### Task 3: The source layer as a library call

**Files:**
- Modify: `crates/naves-export/src/generate.rs`
- Test: `crates/naves-export/tests/edits.rs`

**Interfaces:**
- Produces: `pub struct Source { pub frame: MapGeo, pub map_size_m: f64, pub extract: OsmWays, pub edits: WayEdits,
  pub ways: OsmWays, pub report: EditReport }` and
  `pub fn read_source(project_dir: &Path, project: &Project) -> Result<Source, BuildError>`;
  `impl Source { pub fn reapply(&mut self) }` (recomputes `ways` and `report` from `extract` and `edits`).
  `read_features` is rewritten to call `read_source` so that the build and the GUI read the same thing.

- [ ] **Step 1: Write the failing tests**
  - `read_source_applies_the_edits`: a project with the Königssee roads fixture and an `edits/ways.toml` that deletes
    one OSM way and adds a user way: `source.ways` lacks the deleted id and has `USER_ID_BASE + n`; `source.extract`
    still has the deleted way.
  - `read_source_without_osm_is_the_edits_alone`: no `sources.osm`, one added way: `extract.ways` is empty,
    `ways.ways.len() == 1`.
  - `reapply_follows_the_edit_set`: after `source.edits.delete_way(..)`, `reapply()` drops the way.
- [ ] **Step 2: Run** `cargo test -p naves-export --test edits read_source`: FAIL.
- [ ] **Step 3: Implement** in `generate.rs` (no new placement work; this is the first half of `read_features`).
- [ ] **Step 4: Run**: PASS, and `cargo test -p naves-export` still passes. **Step 5: Commit**
  `refactor: the build reads its source layer through read_source` (no user-visible change, no changelog).

### Task 4: The edit session: operations, save, undo and redo

**Files:**
- Create: `crates/naves/src/gui/edit/mod.rs`
- Modify: `crates/naves/src/gui/mod.rs` (field `edit: EditState`), `crates/naves/src/gui/worker.rs` (`SourceJob`),
  `crates/naves/src/gui/shortcuts.rs` (`action_exists` for Undo/Redo/Regenerate)
- Test: `crates/naves/src/gui/tests/edit.rs` (new, add `mod edit;` in `tests/mod.rs`)

**Interfaces:**
- Consumes: `read_source`, `Source::reapply` (Task 3); `WayEdits::{new_node, move_node, add_way, modify_way,
  delete_way, reset_way, save}` (phase 1).
- Produces:
  - `pub enum EditState { None, Loading, Ready(EditSession), Failed(String) }`;
  - `pub struct EditSession { pub source: Source, undo: Vec<WayEdits>, redo: Vec<WayEdits>, pub dirty: bool }`
    with `pub const UNDO_DEPTH: usize = 100`;
  - `pub fn apply(&mut self, dir: &Path, op: impl FnOnce(&mut WayEdits, &Source) -> Result<(), EditsError>) ->
    Result<(), String>`: pushes the old set, runs `op` on a copy, saves, then swaps it in and calls `reapply`;
    on any error the session is unchanged and the error text starts with "Could not save the edit: " or
    "The edit cannot be made: ";
  - `pub fn undo(&mut self, dir: &Path) -> Result<bool, String>`, `pub fn redo(&mut self, dir: &Path) ->
    Result<bool, String>` (save, then swap; `false` when the stack is empty);
  - `pub fn way(&self, raw: i64) -> Option<&OsmWay>` (in `source.ways`), `pub fn original(&self, raw: i64) ->
    Option<&OsmWay>` (the extract's way for an OSM id, else `way`), `pub fn origin(&self, raw: i64) -> Origin` with
    `pub enum Origin { Osm, EditedOsm, Drawn }`.
  - `NavesApp::regenerate()` (Ctrl+R, F5): sets `dirty = false` and calls `reload_roads()`; status after an edit:
    "Edited: press Ctrl+R to rebuild."

- [ ] **Step 1: Write the failing tests** (on `EditSession` directly, with a temp project; and one in the harness)
  - `an_edit_is_saved_at_once`: `apply` with `delete_way`, then `WayEdits::load(dir)` has the delete edit.
  - `undo_and_redo_restore_the_file`: delete, undo → file has no edits and `source.ways` has the way again; redo →
    deleted again; a second undo past the start returns `Ok(false)`.
  - `the_undo_stack_holds_one_hundred`: 101 edits, 100 undos succeed, the 101st returns `Ok(false)`.
  - `a_new_edit_clears_redo`.
  - `a_failed_save_undoes_the_edit` (Review Focus 2): make `edits/` read-only (`std::fs::set_permissions`, unix),
    `apply` returns `Err` starting with "Could not save the edit: ", `source.edits` equals the set before, the undo
    stack did not grow.
  - `a_source_change_reloads_the_session_and_keeps_the_edits` (Review Focus 1): with a session ready, change
    `form.project.sources.osm` and save the project → `EditState::Loading`, then `Ready` with the same
    `edits/ways.toml` content byte for byte.
  - `ctrl_r_rebuilds_after_an_edit`: after an edit the status line reads "Edited: press Ctrl+R to rebuild."; Ctrl+R →
    `RoadsState::Generating`, `dirty == false`.
- [ ] **Step 2: Run** `cargo test -p naves --bin naves gui::tests::edit`: FAIL.
- [ ] **Step 3: Implement.** `SourceJob` mirrors `RoadJob` (thread + channel, `read_source` on the saved project);
  it starts with the road job and on the same triggers (`reload_roads`). `Regenerate` and `Undo`/`Redo` exist in
  `action_exists` while the session is `Ready` (Undo/Redo also need a non-empty stack).
- [ ] **Step 4: Run**: PASS. **Step 5: Commit** `feat: the GUI keeps an edit session with save, undo and redo`
  (CHANGELOG: "Edits made in the GUI are saved at once to `edits/ways.toml`; Ctrl+Z and Ctrl+Shift+Z undo and redo
  them, Ctrl+R rebuilds."; `gui.md`, `shortcuts.md`).

### Task 5: Hit-testing and the editing overlay

**Files:**
- Create: `crates/naves/src/gui/edit/hit.rs`, `crates/naves/src/gui/edit/overlay.rs`
- Modify: `crates/naves/src/gui/viewport/mod.rs` (`ViewFrame::overlay: &'a [OverlayShape]`),
  `crates/naves/src/gui/viewport/map.rs` (paint them after the layers), `crates/naves/src/gui/selection.rs`
- Test: `crates/naves/src/gui/tests/edit.rs`, unit tests in `hit.rs`

**Interfaces:**
- Consumes: `EditSession` (Task 4).
- Produces:
  - `pub const HIT_PX: f64 = 8.0;` `pub fn tolerance_m(view_span_m: f64, view_width_px: f64) -> f64` (= `HIT_PX` ×
    metres per pixel, at least 0.5 m);
  - `pub enum Target { Way(i64), Node { node: i64, way: i64 } }`;
  - `pub fn hit(session: &EditSession, at: WorldPoint, tol_m: f64) -> Option<Target>`: a node within `tol_m`
    wins over a way; among nodes the nearest; among ways the nearest segment; ties go to the higher raw id (drawn
    after) so the result is deterministic;
  - `Selection::target: Option<Target>` (new field; `None` for picks that are not source items);
  - `pub enum OverlayShape { Line { points: Vec<WorldPoint>, style: LineStyle }, Handle(WorldPoint), Tick
    { at: WorldPoint, heading: f64 } }`, `pub enum LineStyle { Source, Edited, Selected, Drawing }`;
  - `pub fn overlay(session: &EditSession, selection: Option<&Target>, drawing: &[WorldPoint]) -> Vec<OverlayShape>`.

- [ ] **Step 1: Write the failing tests**
  - `a_node_wins_over_its_way`: a way (0,0)-(100,0) with node at (50,0); `hit` at (51, 1) with tol 4 →
    `Target::Node`; at (25, 1) → `Target::Way`; at (25, 10) → `None`.
  - `the_tolerance_follows_the_zoom`: `tolerance_m(1000.0, 1000.0) == 8.0`, `tolerance_m(10.0, 1000.0) == 0.5`.
  - `edited_and_drawn_ways_get_ticks`: a session with one edited OSM way and one drawn way → two `Edited` lines and
    four `Tick`s; unchanged ways are `Source` lines with no ticks.
  - `the_selected_way_has_handles`: one `Handle` per node of the selected way, and its line is `Selected`.
  - `a_click_in_select_picks_the_source_way` (harness, Roads level, `with_roads`): a queued `ViewEvent::Click` on a
    source way's middle → `app.selection.target == Some(Target::Way(id))`, the properties title is the way's class
    name, and the built piece under it is still listed under "Built here".
- [ ] **Step 2: Run**: FAIL. **Step 3: Implement.** Projection: `session.source.frame.to_map` per point (skip a
  point that fails). The overlay is painted with `ui.painter()` using the view's `map_to_screen`; styles from the
  design notes (Source 2 px INK_QUIET, Edited 2 px ACTIVE, Selected 3 px ACTIVE, Drawing 2 px INK dashed; handles 7 px
  squares, GROUND fill, ACTIVE border; ticks 6 px across the line).
- [ ] **Step 4: Run**: PASS. **Step 5: Commit** `feat: the Roads level draws the source ways and picks them`
  (`gui.md`: what the lines and ticks mean).

### Task 6: The reusable selector: fits, scope and footprints

Start with `/frontend-design:frontend-design`; the design notes are the brief.

**Files:**
- Modify: `crates/naves/src/gui/palette.rs`, `crates/naves/src/gui/mod.rs` (`item_level`), `crates/naves-core/src/prefs.rs`
  (`MapPrefs::selector_scope: BTreeMap<String, Scope>` keyed by level id, default empty = level)
- Test: `crates/naves/src/gui/tests/palette.rs` (new)

**Interfaces:**
- Produces:
  - `PaletteItem` gains `pub fits: Vec<Level>` (non-empty), `pub size_m: Option<f64>` (shown at the right);
  - `Thumbnail::Footprint { outline: Vec<(f32, f32)> }` in metres, centred; `Thumbnail::Line` keeps `width_m` and
    is drawn at the shared scale;
  - `pub const THUMB_PX_PER_M: f32 = 2.0;` `pub const THUMB_COLUMN_PX: f32 = 64.0;`
  - `pub enum Scope { Level, Everything }` (serde `"level"`, `"everything"`);
  - `pub fn in_scope<'a>(items: &'a [PaletteItem], level: Level, scope: Scope) -> Vec<&'a PaletteItem>`: `Level`
    keeps items whose `fits` contains `level`; `Everything` keeps all, ordered: items fitting `level` first, then by
    the first level of `fits` in stack order, then group, then name;
  - `LevelDef::items` stays the level's own items; the list for Everything is the union of every built level's
    `items` plus `app.models()` (Task 7), deduplicated by `id`;
  - `NavesApp::item_level: Option<Level>`: the level of the picked item when it is not the active level;
    `has_tool` and the rail consult that level's `tools` too; Esc clears it.
- [ ] **Step 1: Write the failing tests** (the fake level and two fake items with `fits: [Roads]` and
  `fits: [Buildings]`)
  - `the_scope_switch_is_named_after_the_level`: on the fake Buildings level the switch reads "Buildings" and
    "Everything".
  - `level_scope_shows_what_fits`: only the Buildings item is listed.
  - `everything_shows_all_with_the_band_chip`: both are listed, the Roads item has a chip with text "4".
  - `an_item_of_another_level_keeps_the_active_level` (Review Focus 5): pick the Roads item on Buildings →
    `panel.active == Buildings`, `item_level == Some(Roads)`, the rail shows the item's tool; Esc → tool Select,
    `item_level == None`.
  - `the_scope_is_kept_per_level`: set Everything on Buildings, go to Roads (Level scope), back to Buildings →
    Everything; prefs saved with `selector_scope = { "level.buildings" = "everything" }`.
  - `thumbnails_share_one_scale`: unit test of the drawing helper: a `Line { width_m: 12.0 }` is 24 px thick and a
    `Line { width_m: 5.0 }` 10 px; a footprint 40 m long is clipped to 64 px.
  - `search_spans_the_scope`: in Everything, searching the Roads item's name finds it; in Level scope it does not.
- [ ] **Step 2: Run** `cargo test -p naves --bin naves gui::tests::palette`: FAIL.
- [ ] **Step 3: Implement.** The switch is `naves_ui`'s segmented/tab control if it has one, else two
  `selectable_label`s in a row with the RAISED fill on the chosen one. The chip is the band colour with the
  `band_text` digit, 14 px square, with the level name as its tooltip and accessible label.
- [ ] **Step 4: Run**: PASS. **Step 5: Docs** (`gui.md`: the selector, its two scopes, the chip) + CHANGELOG ("The
  item list can show what fits the level or everything; an item of another level can be used without leaving the
  level."). **Step 6: Commit** `feat: one item selector for every level, with level and everything scopes`.

### Task 7: Roads items: road classes and catalog models

**Files:**
- Modify: `crates/naves/src/gui/levels/roads.rs` (`items`), `crates/naves/src/gui/worker.rs` (`CatalogJob`),
  `crates/naves/src/gui/mod.rs` (`models()`), `crates/naves/src/gui/edit/properties.rs` (model card)
- Test: `crates/naves/src/gui/tests/roads.rs`

**Interfaces:**
- Consumes: `palette` (Task 6), `road_catalog(tools, &tools.cache_dir)`.
- Produces:
  - `pub struct RoadClass { pub id: &'static str, pub name: &'static str, pub group: &'static str, pub tags: &'static
    [(&'static str, &'static str)] }` and `pub const ROAD_CLASSES: &[RoadClass]`: Primary (`highway=primary`),
    Secondary, Tertiary, Residential street (`highway=residential`), Service road (`highway=service`), Track
    grade 1 to 5 (`highway=track`, `tracktype=grade1..5`) in group "Roads"; Footpath (`highway=footway`), Path
    (`highway=path`) in group "Paths". Each class's `width_m` comes from `classify(tags)` →
    `Family::width_m()` (a path: 2 m). Items: `ItemKind::LineClass`, `default_tool: ToolId::LINE`, `fits:
    [Roads]`, id `roads.class.<id>`;
  - catalog models: one item per `RoadPiece` model, junction model and `BridgeModel` (`ItemKind::Model`,
    `ItemSource::Game`, groups "Pieces", "Junctions", "Bridges", id `model.<model path>`, `fits: [Roads]`,
    `Thumbnail::Footprint` from the ports and width, `size_m` = chord or length), `default_tool: ToolId::SELECT`
    (placing comes in phase 4);
  - picking a model shows its card in the properties: name, size, the levels it fits, "Placing models comes in a
    later version." (exact copy).
- [ ] **Step 1: Write the failing tests**
  - `the_roads_level_lists_the_road_classes`: the item names in group "Roads" start with Primary, Secondary,
    Tertiary; every class item's `size_m` equals the width of what `classify(tags)` gives for its tags (computed in
    the test, not written as constants).
  - `picking_a_class_switches_to_draw`: tool becomes `ToolId::LINE`, status "Draw: click to add points, click a node to join, Space to finish, Ctrl+Z to take one back."
  - `catalog_models_are_listed_once_the_catalog_is_read` (the `FakeEnv` tools): groups "Pieces" and "Bridges" exist
    and every model item has a `Footprint`.
  - `picking_a_model_shows_its_card`.
  - `drawing_works_without_osm` (Review Focus 3): a project without `sources.osm` → the Roads level shows the
    selector (not "Add an .osm.pbf file…"; that text becomes the hint under the list: "No OpenStreetMap file: draw
    the roads yourself, or add one to sources.osm.").
- [ ] **Step 2: Run**: FAIL. **Step 3: Implement.** `CatalogJob` like `RoadJob`; the catalog is kept as
  `Arc<PieceCatalog>`; a missing tool leaves the model groups out and puts the existing "Roads need dayzmake" line
  under the list.
- [ ] **Step 4: Run**: PASS. **Step 5: Docs** + CHANGELOG. **Step 6: Commit**
  `feat: the Roads level lists road classes and the game's road pieces and bridges`.

### Task 8: The tools: Move, Draw, Cut, Join, Bridge, Delete

**Files:**
- Create: `crates/naves/src/gui/edit/tools.rs`
- Modify: `crates/naves/src/gui/levels/roads.rs` (`tools: &[...]`), `crates/naves/src/gui/shortcuts.rs`
  (`action_exists`: `Finish` while drawing, `Delete` with a target selected)
- Test: `crates/naves/src/gui/tests/edit.rs`

**Interfaces:**
- Consumes: `EditSession::apply` (Task 4), `hit` and `Target` (Task 5), the picked `RoadClass` (Task 7).
- Produces: `ToolDef`s for `MOVE` ("Move"), `LINE` ("Draw"), `CUT` ("Cut"), `JOIN` ("Join"), `BRIDGE` ("Bridge"),
  each `on_event: Some(..)`; `pub struct Drawing { pub points: Vec<(WorldPoint, Option<i64>)> }` in `NavesApp`
  (the node id when a point joins an existing node).

Behaviour, one rule each (the status lines are the exact copy):

- **Move (G):** press on a node, drag, release → one `move_node` (lat/lon from `frame.to_geo`; `osm_at` = the
  extract's position for an OSM node). The handle follows the drag; Esc during the drag cancels. Status: "Move: drag
  a node."
- **Draw (C):** each click adds a point; a click within tolerance of a node uses that node (join). Ctrl+Z (and
  Backspace) removes the last point; Space, Enter, a right click or a double click with two points or more →
  `new_node` for each new point and `add_way` with the class tags;
  Esc discards. Without a picked class, Draw uses Residential street. Status as in Task 7.
- **Cut (R):** click an inner node of a way → the way ends at that node (`modify_way`) and a new way with the same
  tags starts there (`add_way`), in one `apply` (one undo step). Status: "Cut: click a node inside a road."
- **Join (T):** click an end node of one way, then an end node of another. Same tags → one way (`modify_way` on
  the first with the nodes joined through the shared end, `delete_way` on the second); different tags → the second
  way's end is replaced by the first way's end node (`modify_way`), both kept. Status: "Join: click the end of a
  road, then the end of another."
- **Bridge (Shift+T):** click two nodes of one way → the part between them is split out (as Cut, twice) and tagged
  `bridge=yes`, `layer=1`; on a way that already has `bridge=yes` one click removes both tags. Status: "Bridge:
  click two nodes of a road to make the part between them a bridge."
- **Delete (X, the Delete key too):** a selected way → `delete_way`; a selected node of a drawn way → the node is taken out of its way
  (`modify_way`; a way left with fewer than two nodes is deleted). OSM nodes are not deleted, only moved.

- [ ] **Step 1: Write the failing tests** (queued `ViewEvent`s through the fake view, with a session on a synthetic
  extract of three ways; every test also asserts the saved file with `WayEdits::load`)
  - `move_drags_a_node`; `esc_during_a_drag_moves_nothing`.
  - `draw_adds_a_way_with_the_class_tags`: three clicks, Space → one `add` way with `highway=secondary`,
    `naves:keep=yes`, three user nodes.
  - `draw_joins_an_existing_node`: the first click on an OSM node → the new way's first node is that OSM id, and no
    node edit is made for it.
  - `ctrl_z_takes_a_point_back` (Ctrl+Z while drawing changes no file); `space_with_one_point_draws_nothing`;
    `right_click_and_double_click_finish`.
  - `cut_splits_a_way_in_one_undo_step`: two ways after, one undo → one way.
  - `join_with_the_same_tags_makes_one_way`; `join_with_other_tags_connects_the_ends`.
  - `bridge_splits_out_the_part_and_tags_it`; `bridge_on_a_bridge_untags_it`.
  - `delete_removes_the_selected_way`; `delete_takes_a_drawn_node_out`.
  - `a_click_on_nothing_changes_nothing` (Review Focus 4): Cut, Join, Bridge and Move on an empty spot → no save
    (file mtime and content unchanged), undo stack unchanged, status line unchanged.
- [ ] **Step 2: Run**: FAIL. **Step 3: Implement** in `edit/tools.rs`; every tool turns its events into at most
  one `EditSession::apply` call. **Step 4: Run**: PASS.
- [ ] **Step 5: Docs** (`gui.md`: each tool with its key; `shortcuts.md`: the Roads row from Task 1) + CHANGELOG ("On the Roads
  level roads can be drawn (C), moved (G), cut (R), joined (T), made a bridge (Shift+T) and deleted (X)."). **Step 6: Commit**
  `feat: draw, move, cut, join, bridge and delete roads in the GUI`.

### Task 9: Properties of a selected way

**Files:**
- Create: `crates/naves/src/gui/edit/properties.rs` (the way part; the model card from Task 7 lives here too)
- Modify: `crates/naves/src/gui/stack.rs` (or wherever the properties are drawn today) to call it for a
  `Target::Way`
- Test: `crates/naves/src/gui/tests/edit.rs`

**Interfaces:**
- Consumes: `EditSession::{way, original, origin, apply}`, `ROAD_CLASSES`.
- Produces: the properties for a way: Class (combo of `ROAD_CLASSES` plus "Other: highway=<value>" when the way's
  tags match none), Surface (combo: asphalt, paved, gravel, compacted, dirt, grass, "—"), Bridge (checkbox), Layer
  (-5..5), "Keep as drawn" (`naves:keep`), "Let road planning route it" (`naves:plan`), the origin line ("OSM way
  8421, unchanged" / "OSM way 8421, edited by you" / "Drawn by you"), and "Reset to OSM" for `Origin::EditedOsm`.

- [ ] **Step 1: Write the failing tests**
  - `the_origin_line_says_where_a_way_came_from` (three cases, exact copy above).
  - `changing_the_class_retags_the_way`: Secondary → Track grade 2 → saved tags have `highway=track`,
    `tracktype=grade2`, the old `surface` kept.
  - `keep_and_plan_are_switches`: unticking "Keep as drawn" writes `naves:keep=no`; ticking "Let road planning
    route it" writes `naves:plan=yes`.
  - `reset_to_osm_removes_the_edit`: the edit is gone from the file and the origin reads "unchanged"; one undo
    brings it back.
  - `an_unknown_highway_is_shown_as_other`.
- [ ] **Step 2: Run**: FAIL. **Step 3: Implement**; every change is one `apply`. **Step 4: Run**: PASS.
- [ ] **Step 5: Docs** (`gui.md`: the properties) + CHANGELOG. **Step 6: Commit**
  `feat: the properties of a road can be changed and reset to OSM`.

### Task 10: The whole flow, checked once by hand

**Files:**
- Modify: `docs/manual/src/gui.md` (screenshots if the chapter has them), `docs/manual/src/getting-started.md`
  (a short "Fix a road" section)
- Test: `crates/naves/src/gui/tests/edit.rs`

- [ ] **Step 1: Write the failing test** `draw_save_rebuild`: a project without OSM; pick Secondary, draw three
  points, Space, Ctrl+R; wait for `RoadsState::Ready` (poll with `HANG_GUARD`) → `generated.features` has one road with
  id `user/way/<n>/0` and `held == true`, `generated.pieces` is not empty.
- [ ] **Step 2: Run**: FAIL until Tasks 2–9 are in; then PASS.
- [ ] **Step 3: Run the GUI by hand** (`cargo run -p naves -- gui <a copy of the Berchtesgaden project in
  /var/tmp>`, under the CPU and memory caps): draw a road, move a node, cut, join, bridge, undo, Ctrl+R; pan with WASD and zoom with Q/E. Take one
  screenshot of the Roads level with a selected edited way and the selector in Everything scope; look at it against
  the design notes (one accent, ticks visible, no colour-only cue) and fix what does not hold.
- [ ] **Step 4: Docs** (getting-started "Fix a road"). **Step 5: Commit** `docs: fixing a road in the GUI, start to
  finish`.

---

## After this plan

Phase 3 (terrain and earthwork) adds the Terrain tools and the earthwork switch to these properties; phase 4 adds
the Place tool (T) that the model items of Task 7 switch to, through the same selector and its Everything scope.

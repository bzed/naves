<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Náves: the editor's shell — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `naves gui` becomes the final editor's interface from the spec (the stack, levels, band and status line, bottom dock, shortcuts, settings), with what exists today in its place, built so that each later plan adds its levels, layers, tools, items and the 3D view by adding entries, without moving anything.

**Architecture:** the shell knows nothing about any feature. It reads four small tables (levels, map layers, shortcuts, dock tabs) and draws through one `Viewport` trait that speaks map metres, not pixels. The map is the first `Viewport`. Layers are images over a rectangle of the map, so a 3D view can drape the same images. A test-only second viewport proves the Ctrl+Tab switch works before any 3D code exists, and the last task proves the seam with real wgpu on a branch that is not merged.

**Tech Stack:** Rust 1.95, `eframe`/`egui` 0.37 (glow backend for now, see *2D and 3D*), `egui_kittest`, `toml`. No new crate in the merged code.

**Spec:** `docs/superpowers/specs/2026-11-06-naves-editor-ui.md`, all sections. Background: `docs/superpowers/specs/2026-11-04-naves-design.md`, sections 5.2, 5.9, 5.10.

**Starts from:** `main` at `1eb0532` or later (land cover and imagery merged: `gui/imagery.rs`, `MapData`, `Overlay::Surface`, `Overlay::Imagery`). Runs **before the roads plan** and takes over its Tasks 12 and 13 (Task 1 amends both documents).

Not in this plan: objects, undo and history (houses plan), AI proposals and the AI settings tab (milestone 2), any editing tool, a real 3D view, MCP `layers` (stays in the roads plan, with the layers it is for).

## How later plans extend it

This table is the point of the plan. Task 12 checks that it holds.

| To add | You write | The shell then gives you |
|---|---|---|
| A level | `gui/levels/<name>.rs` with `pub const DEF: LevelDef`, one line in `levels::BUILT` | the stack row, digit key, left panel, band colour, F1 and Keyboard rows |
| A map layer | a `MapLayer` variant and arm in `naves_core::render` (the compiler lists the arms), its id in the level's `layers` | the checkbox and key under the level, fade and hide, texture cache, draping in 3D |
| A tool | a `ToolDef` in the level's `tools` (the key is already in `SHORTCUTS`) | the rail letter, status line, world-space events from whichever view is active |
| Things to place | the level's `items` function returning `PaletteItem`s | the palette with search, last used, groups; picking one switches tool |
| Something to inspect | the level's `pick` function | click, Alt+click, properties, Frame selection |
| A problem | `Problem { message, at, level }` from its source | the count, the dock row, the map marker, F8 |
| A dock tab | a `DockTabDef` in `dock::TABS` | the tab, resizing, folding |
| A settings tab | a `PrefsTab` variant and arm | the modal, the save rule |
| The 3D view | `impl Viewport`, one entry in `viewport::default_views` | Ctrl+Tab, a shared place on the map, markers, band, status line, all layer state |

## 2D and 3D

Checked in the code on 2026-11-06:

- **A 3D view cannot be written on the current backend.** `glow`'s context API is `unsafe fn` throughout (393 of them), and `unsafe_code = "forbid"` holds. `egui_wgpu::CallbackTrait` is safe, and the design already names wgpu (spec 5.9). So the real 3D view brings `eframe`'s `wgpu` feature with it. `wgpu` is in `Cargo.lock` but not compiled today (`cargo tree -i wgpu` prints nothing).
- **Decision:** stay on glow in merged code; nothing here depends on the backend, because `Viewport::show` gets an `egui::Ui` and a rectangle. Task 12 tries the switch on a branch, so that the cost is known and the trait is proven before anyone relies on it.
- **The key is Ctrl+Tab, not the spec's Tab (the maintainer's decision, 2026-11-06).** `Focus::begin_pass` reads plain Tab and Shift+Tab from the raw input before the app runs, and `consume_key` comes too late: the view would switch and focus would land on the Name field. egui's focus code does not claim Ctrl+Tab, so it is an ordinary row in `SHORTCUTS`, plain Tab keeps moving through the panels, and no raw input is touched.

What the seam rests on:

1. Everything outside a viewport speaks `WorldPoint` (map metres). Tools never see pixels.
2. A `ViewFocus` (centre and span in metres) is the shared place on the map, so Ctrl+Tab keeps you where you are.
3. State about what is shown lives in the shell and is handed to the view each frame. A view owns only its camera and its GPU or texture cache.
4. Band, status line, problem and selection markers are drawn by the shell, through `Viewport::project`.
5. Empty states (no DEM, loading, failed) are handled by the shell once, not per view.
6. Layers render to an image of any map rectangle: the map asks for what is on screen, a 3D view for the whole map.

## Design pass (`/frontend-design:frontend-design`, 2026-11-06)

The spec fixes palette, type and layout, and the brief wins over the skill's defaults: egui's default family, no new font. The review went through the plan against the skill and changed these things:

- **The stack must not come out as a list with coloured dots**, which is what a level table would give by default. It is the one memorable thing, so it is specified here: strata touch, with no gap and no rounding. A closed level is 24 px: a 28 px block in its colour holding the digit, the name beside it on the panel. **The active stratum is visibly thicker:** its colour fills the whole row, and its layers unfold below it with a 4 px edge of the same colour running down the left. Order bottom-up, 0 at the bottom. Missing levels leave no mark and no gap. Nothing else in the window has square blocks of colour, and rounding stays egui's elsewhere.
- **Contrast, measured.** White on dry grass is 2.86 and on ochre 3.17, so band text is chosen per level (ink there, 5.26 and 4.75; white on the other eight, 4.93 or better). Earth `8c6a45` as text is 4.13 on the panel, under 4.5: it stays a band colour, and today's brown text (warnings, the map line) goes to ink, as the word "Warning:" already carries the meaning. A test pins all of it.
- **Keyboard focus stays visible.** `apply_theme` sets `selection.stroke` to white, which vanishes on the light panel. Focus gets a green ring (`GREEN`), checked in Task 11.
- **Motion.** One motion, per the spec: the unfold, 100 ms. egui and eframe expose no reduced-motion signal (searched 0.37's sources), so the Map tab gets "Unfold without animation", which sets `animation_time` to 0.
- **Words.** Same action, same name: "Build" on the button and in the log, "Go to Terrain" in tooltip, F1 and Keyboard tab. Empty states say what to do. Zero problems reads "No problems" in the dock and plain "Problems" in the row, no "0". Layer rows are named for what you see. No separators of dots or dashes in labels.
- **Not cards.** Palette rows are rows: thumbnail, name, what it is in words. No per-item frame or shadow. The settings window has a stroke and the dimmed window behind it, no drop shadow.
- **Removed:** a wavy "unconformity" mark where levels are missing (it would be a promise in disguise), and level icons (the digit is the key and the order).
- **Numbers** align right in a grid, in the default family. No monospace for small labels; the build log stays monospace because it is a log.

The review found no reason to change the architecture.

## Decisions

The maintainer confirmed the walk limit, reading a point, and this plan coming before roads on 2026-11-06.

- **`WALK_LIMIT_DEG = 60.0`, provisional.** Roads Task 1 measures the real limit. 60 is Livonia's navmesh `maxSlope`, an upper bound, so the layer under-reports until then but never marks walkable ground. It is here because H, Shift+H and fading need a real layer.
- **Click with Select reads a point** (position, height, slope in Terrain; ground class in Ground). The spec's selection is about objects, of which there are none yet. This gives the selection path real use today, and later objects replace it by returning their own `Selection`.
- **Rail and palette are built now and stay hidden** until a level has more than one tool or any item, which is the spec's rule stated as data. The maintainer asked for a UI ready for later items and left the timing to the plan.
- **Spec gap:** the External tools tab lists armake2, Steam, DayZ Tools, DayZ and Proton, but a build also needs DayZ Server (`DAYZ_SERVER`). Added as a sixth row.

## Global Constraints

- **`AGENTS.md` applies to every task:** tests first, coverage at or above 85 %, strict lints, `make check` before each commit, manual and `CHANGELOG.md` line in the same commit, CLI help and manual agree. Where this plan and `AGENTS.md` disagree, `AGENTS.md` wins.
- **Every UI task starts with `/frontend-design:frontend-design`** (`AGENTS.md`, *User interface*), and follows *Design pass* above.
- Widths: left 300 px, rail 56 px, right 230 px; band 3 px; fade 40 %; tints below 50 %; colour never the only cue on the map.
- **Levels, digits and colours are the spec's table, verbatim:** 0 Project `5a6660`, 1 Terrain `8c6a45`, 2 Ground `a39a5c`, 3 Water `3f6f9e`, 4 Roads and paths `1d2924`, 5 Vegetation `2e6a4c`, 6 Buildings `9a4f3c`, 7 Objects `6b6f76`, 8 Life `7a4a7f`, 9 Economy `b58a1e`. Ground `f2f5ec`, panel `e7ede0`, ink `1d2924`, green `2e6a4c`, error `b03a2e`.
- Spec, verbatim: "**A level appears when it can do something.** There are no greyed-out promises." "**Typing is typing.** While a text field has focus, no single-key shortcut fires." "**A key already in use is never silently taken.**" "Esc, Enter and F1 cannot be changed". "A damaged file is not overwritten".
- Settings live in `$XDG_CONFIG_HOME/naves/settings.toml`, never in `naves.toml`. Tests never touch the real one: `NavesApp::open` has no path unless `with_preferences` gives one.
- Tests are deterministic and drive the GUI through the accessibility tree. Unit tests are for private helpers with real logic. Local runs with `TMPDIR=/var/tmp`. Screenshots in a headless sway with `grim`.
- `dead_code` is a hard error under `clippy --all-targets`. A type only tests construct until a later plan (`PaletteItem`, its `Thumbnail`, `ToolDef::on_event`) carries `#[cfg_attr(not(test), expect(dead_code, reason = "<which plan uses it>"))]`, never `allow`.
- Map coordinates: x east, y north, metres, origin south-west.

## Review Focus

1. **A widget has focus that is not a text field** (a clicked checkbox, a stack row, a combo box): digits and letters still work. `egui_wants_keyboard_input` is true for any focus, so it is the wrong test; use `Context::text_edit_focused`. A `DragValue` being typed in still counts as typing. Test in Task 4: `a_clicked_checkbox_does_not_block_shortcuts`, `typing_in_a_drag_value_fires_nothing`.
2. **Space and Tab belong to widgets too:** Space on a focused checkbox toggles it and must not start a pan; plain Tab always moves focus, and Ctrl+Tab never does, not even from a text field. Tests in Tasks 4 and 6: `tab_still_moves_focus`, `ctrl_tab_does_not_move_focus`, `ctrl_tab_works_while_typing`, `space_on_a_focused_checkbox_does_not_pan`.
3. **The project changes under the shell:** the map size changes, or the data goes from ready to loading or failed, with a selection, a problem target or a framed view open. The selection is dropped, the view refits, F and F8 do nothing harmful. Test in Task 6: `a_changed_map_drops_the_selection`, `framing_without_data_does_nothing`.
4. **A very small window:** dock folds, then the left panel, and the map stays. At 200 × 150 nothing panics and `MapView` is not given a zero or negative size. Test in Task 3: `a_tiny_window_does_not_panic`.
5. **A settings file edited by hand or shared between versions:** an unknown id is kept, an unknown key name falls back to the default and says so, two actions on one key is shown and not fatal, a read-only folder says "Not saved: ...". Tests in Tasks 9 and 10: `unknown_ids_survive_a_save`, `a_bad_key_name_keeps_the_default_and_says_so`, `a_duplicate_in_the_file_is_shown`, `an_unwritable_file_is_reported`.

---

### Task 1: Levels, theme, and the documents that say what comes when

**Files:**
- Create: `crates/naves/src/gui/theme.rs`, `crates/naves/src/gui/levels/mod.rs`
- Move: `crates/naves/src/gui/tests.rs` to `crates/naves/src/gui/tests/mod.rs`
- Create: `crates/naves/src/gui/tests/levels.rs`
- Modify: `crates/naves/src/gui/mod.rs`, `crates/naves/src/gui/imagery.rs`, `docs/superpowers/specs/2026-11-06-naves-editor-ui.md`, `docs/superpowers/plans/2026-11-06-naves-roads.md`

**Interfaces:**
- Produces, in `gui::theme`: the palette constants (`GREEN`, `ERROR`, `INK`, `GROUND`, `PANEL`), `apply_theme(ctx)`, `contrast(a: Color32, b: Color32) -> f32`.
- Produces, in `gui::levels`: `pub enum Level { Project, Terrain, Ground, Water, Roads, Vegetation, Buildings, Objects, Life, Economy }` (`Copy + Ord`), with `ALL`, `digit(self) -> u8`, `name(self) -> &'static str`, `colour(self) -> Color32`, `band_text(self) -> Color32`, `id(self) -> &'static str` (`level.terrain`).

- [ ] **Step 1: Amend the documents.** Spec section 7: a first row "Editor shell (this plan)": stack, levels 0 to 2, band and status line, base maps, the layer mechanism with Too steep to walk, Select to read a point, dock with Build and Problems, the whole shortcut table, F1, the settings window with Keyboard, Map and External tools, the rail and palette widgets (hidden until there is something for them), the `Viewport` seam. Roads row: level 4's content and its layers. Houses row: "the 3D view (a `Viewport`)". Section 4: the row for the views reads Ctrl+Tab. Section 8: "Tab and Shift+Tab cannot be assigned, so that the way through the panels always works." Roads plan: **Task 12** shrinks to level 4's `DEF`, its `MapLayer` variants, generation job, `RoadStats` panel, `pick`, `Violation` to `Problem`, and the MCP part; **Task 13** is removed apart from `naves build` already using the settings; **Task 10** replaces the value of `WALK_LIMIT_DEG` from `render.rs` with the measured one. Header: starts after this plan.
- [ ] **Step 2: Write the failing tests** in `gui/tests/levels.rs`: `digits_never_change` and `colours_are_the_spec_s` (all ten, literally); `ids_are_pinned`; `band_text_is_readable` (`contrast(colour, band_text) >= 4.5` for all ten); `contrast_is_right` (black on white is 21 within 0.01); `no_text_is_below_4_5_on_the_panel` (`INK`, `ERROR`, `GREEN`).
- [ ] **Step 3: Run them to see them fail.** Run: `cargo test -p naves levels`. Expected: compile errors.
- [ ] **Step 4: Implement.** Move the constants and `apply_theme` out of `mod.rs`. Remove `BROWN` and `LICHEN` (now `GROUND`): warnings and the map line become `INK`. `selection.stroke` becomes `GREEN` so that focus shows.
- [ ] **Step 5: Run** `cargo test -p naves`. Expected: PASS, with today's GUI tests unchanged.
- [ ] **Step 6: Commit** `refactor(gui): levels and theme in their own modules; docs: the shell comes before roads`.

### Task 2: The `Viewport` seam

**Files:**
- Create: `crates/naves/src/gui/viewport/mod.rs`, `crates/naves/src/gui/viewport/map.rs`, `crates/naves/src/gui/testing.rs` (`#[cfg(test)]`), `crates/naves/src/gui/tests/viewport.rs`
- Modify: `crates/naves/src/gui/map_view.rs`, `crates/naves/src/gui/mod.rs`, `crates/naves/src/gui/tests/mod.rs`

**Interfaces:**
- Produces, in `gui::viewport`:
  - `pub struct WorldPoint { pub x: f64, pub y: f64 }`; `pub struct ViewFocus { pub center: WorldPoint, pub span_m: f64 }`.
  - `pub enum ViewEvent { Hover(WorldPoint), Left, Press { at: WorldPoint, modifiers: Modifiers }, Drag { at: WorldPoint, modifiers: Modifiers }, Release { at: WorldPoint, modifiers: Modifiers }, Click { at: WorldPoint, modifiers: Modifiers } }`.
  - `pub struct ViewFrame<'a> { pub data: &'a Arc<MapData>, pub project: &'a Project, pub base: Overlay, pub generation: u64 }`. `generation` counts up with every new `MapData`.
  - `pub trait Viewport { fn name(&self) -> &'static str; fn show(&mut self, ui: &mut egui::Ui, rect: egui::Rect, frame: &ViewFrame<'_>) -> Vec<ViewEvent>; fn focus(&self) -> Option<ViewFocus>; fn set_focus(&mut self, focus: ViewFocus); fn project(&self, at: WorldPoint, rect: egui::Rect) -> Option<egui::Pos2>; fn reset(&mut self); }`. `project` is `None` when the point is not visible, for a 3D view also behind the camera.
  - `pub struct ViewportSet`, with `new(Vec<Box<dyn Viewport>>)`, `active(&self)`, `active_mut(&mut self)`, `can_switch(&self) -> bool`, `switch(&mut self)` (carries `focus()` over with `set_focus`), `reset_all(&mut self)`. `pub fn default_views() -> Vec<Box<dyn Viewport>>`: the map, alone.
- Produces, in `gui::viewport::map`: `pub struct MapViewport` (owns `MapView`, the base-map texture, a pending focus, the render counter `renders: u64`).
- Produces, in `map_view`: `MapView::map_to_screen(&self, x: f64, y: f64, width: f64, height: f64) -> (f64, f64)`, `MapView::from_focus(focus: ViewFocus, width: f64, height: f64) -> MapView`, `MapView::span_m(&self, width: f64, height: f64) -> f64`.
- Produces, in `NavesApp`: `views: ViewportSet`; `#[cfg(test)] with_views(self, Vec<Box<dyn Viewport>>) -> Self`. `gui::testing::FakeViewport`: records events, focus and frames, `name` is `"3D"`.

`NavesApp` keeps `form`, `heightmap`, `job` and `reload_wanted`. The `texture` field moves into `MapViewport`, with `app.map_texture_is_empty()` for the one test that looks at it. Empty states (no DEM, loading, failed, no imagery) stay in the shell, outside the view. Left drag keeps panning here; Task 6 moves panning to the spec's keys.

- [ ] **Step 1: Write the failing tests.**
  - `map_view.rs`: `map_to_screen_inverts_screen_to_map` (`proptest`, at several zooms); `focus_round_trips` (`from_focus` then `span_m`, within 1e-6); `span_stays_within_the_zoom_limits`.
  - `tests/viewport.rs`: `the_map_reports_clicks_in_metres` (a click at a known pixel gives the `WorldPoint` from `screen_to_map`); `switching_keeps_the_place` (focus of the map is the focus of the fake after `switch`, and back); `a_view_is_told_the_place_before_it_was_drawn` (`set_focus` before the first `show` is honoured); `switching_needs_two_views` (`can_switch` is false with the map alone, and `switch` then changes nothing); `the_empty_state_is_the_same_in_both_views` ("Add elevation files" with the fake active; its `show` is not called); `reloading_resets_every_view`.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p naves viewport map_view`. Expected: compile errors.
- [ ] **Step 3: Implement** `Viewport`, `MapViewport` and `ViewportSet`, moving `map_ui` and `draw_map` over. A wheel zoom stays inside `MapViewport`: a camera is the view's own business.
- [ ] **Step 4: Run** `cargo test -p naves`. Expected: PASS, today's tests included.
- [ ] **Step 5: Commit** `refactor(gui): draw the map through a Viewport that speaks map metres`.

### Task 3: The stack, the left panel by level, band and status line

**Files:**
- Create: `crates/naves/src/gui/stack.rs`, `crates/naves/src/gui/layout.rs`, `crates/naves/src/gui/levels/project.rs`, `crates/naves/src/gui/levels/terrain.rs`, `crates/naves/src/gui/levels/ground.rs`, `crates/naves/src/gui/tests/stack.rs`
- Modify: `crates/naves/src/gui/levels/mod.rs`, `crates/naves/src/gui/mod.rs`, `crates/naves/src/gui/imagery.rs`, `crates/naves/src/gui/tests/mod.rs`, `crates/naves/src/gui/testing.rs`, `docs/manual/src/gui.md`, `CHANGELOG.md`

**Interfaces:**
- Produces, in `gui::levels`:

```rust
pub struct LevelDef {
    pub level: Level,
    /// Left panel, when the level has no items.
    pub panel: fn(&mut NavesApp, &mut egui::Ui),
    /// The status line while nothing else speaks.
    pub status: &'static str,
}
pub const BUILT: &[&LevelDef]; // project::DEF, terrain::DEF, ground::DEF
```

  Later tasks add `layers`, `pick`, `tools` and `items` to this struct, with the empty value in each `DEF`. Functions, not a trait, because panels need `&mut NavesApp`, and per-level state stays a field of `NavesApp`, as imagery's does.
- Produces, in `gui::stack`: `pub struct LayerPanel { pub base: Overlay, pub active: Level }` with `Default` (`Hillshade`, `Project`); `pub fn stack_ui(app: &mut NavesApp, ui: &mut egui::Ui)`; `pub const BASE_MAPS: [(Overlay, &str); 5]`: Relief, Slope, Height, Ground, Imagery.
- Produces, in `gui::layout`: `pub struct Fold { pub dock: bool, pub left: bool }`; `pub fn fold_for(window: egui::Vec2) -> Fold`: the dock folds to its tab row below 560 px of height, the left panel below 900 px of width (**[my call]**: 900 is left + rail + right + 300 of map). A folded left panel leaves a thin strip that opens it.
- Produces: `NavesApp.levels: Vec<&'static LevelDef>`, `NavesApp.panel: LayerPanel`; `#[cfg(test)] with_extra_level(self, &'static LevelDef) -> Self`, sorted into its place by `digit`. `gui::testing::FAKE_LEVEL` is level 6.

**Panels.** Level 0: everything the settings panel holds today, in the same order, imagery included (`levels/project.rs` calls `settings_ui`). Level 1: map size, vertex spacing, lowest and highest point, elevation files. Level 2: share of the map per `SurfaceClass` (`pub fn ground_shares(surfaces: &SurfaceGrid, map_size_m: f64) -> [f64; 4]`, sampled on a fixed 256 × 256 grid), land cover and OSM files, and what the ground follows when there are none. With no data the numbers are replaced by what to do: "Add elevation files to see the map."

**Base maps.** The buttons above the map go. Base maps are exclusive rows at the top of the right column; Ground's rows unfold the surface legend, Imagery's the line "Ground without imagery is checkered." The row Problems stays out until Task 7. Status line at the bottom of the middle, 3 px band at its top in the active level's colour. Copy for Project: `Project settings are on the left. Press 1 to read the terrain.`

Look: *Design pass*, first point. Rows are `selectable_label`-style buttons exposing the role and the name `4 Roads and paths`.

- [ ] **Step 1: Write the failing tests** in `tests/stack.rs`: `only_built_levels_are_shown` (0, 1, 2; not `Water`); `the_stack_is_bottom_up` (`y` of 2 < `y` of 1 < `y` of 0); `an_added_level_takes_its_place_without_changing_the_digits` (with `FAKE_LEVEL`: 6 above 2, nothing else moves); `clicking_a_row_makes_it_active`; `the_left_panel_follows_the_level`; `level_0_keeps_the_settings_working` (type in Name, saved); `base_maps_are_exclusive`; `relief_is_the_default`; `ground_unfolds_its_legend`; `imagery_says_what_checkered_means`; `the_band_has_the_level_s_colour` (shapes are not in the accessibility tree, so this tests `LayerPanel::band_colour(&self) -> Color32`, which the band paints with); `the_status_line_follows_the_level`; `switching_levels_does_not_touch_the_project_file` (bytes equal); `ground_shares_add_up_to_one`; `terrain_numbers_show_the_heights`; `without_data_the_panels_say_what_to_do`; `fold_rules` (table over sizes: dock first, then left, right never); `a_tiny_window_does_not_panic` (`Harness::builder().with_size((200.0, 150.0))`, 8 steps, with data loaded). Change existing tests that click "Surface" to "Ground".
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p naves stack`. Expected: FAIL.
- [ ] **Step 3: Implement.** `show` draws the panels in the spec's order and hands the middle to `views.active_mut().show`. `MapView::fit` gets a size of at least 1 × 1.
- [ ] **Step 4: Run** `cargo test -p naves`. Expected: PASS.
- [ ] **Step 5: Update `docs/manual/src/gui.md`** for the new layout: stack, levels, base maps. "Hillshade" reads "Relief" there and in `CHANGELOG.md`'s new line; MCP keeps `hillshade` (`docs/manual/src/mcp.md` says so). `make docs`.
- [ ] **Step 6: Commit** `feat(gui): the stack of levels, the left panel by level, band and status line`.

### Task 4: Shortcuts: one table

**Files:**
- Create: `crates/naves/src/gui/shortcuts.rs`, `crates/naves/src/gui/tests/shortcuts.rs`, `docs/manual/src/shortcuts.md`
- Modify: `crates/naves/src/gui/mod.rs`, `crates/naves/src/gui/stack.rs`, `crates/naves/src/gui/tests/mod.rs`, `docs/manual/src/SUMMARY.md`, `docs/manual/src/gui.md`, `CHANGELOG.md`

**Interfaces:**
- Produces, in `gui::shortcuts`:
  - `pub struct ToolId(pub &'static str)` (`Copy + Ord`), with consts `SELECT`, `MOVE`, ... for every tool of spec section 4.
  - `pub enum Action { Level(Level), Tool(ToolId), HideLevel, ShowOthersInFull, SwitchView, FrameSelection, FrameMap, WorldAxes, FollowGround, Delete, Duplicate, Undo, Redo, SearchItems, BrushSmaller, BrushLarger, BrushWeaker, BrushStronger, Finish, Cancel, Regenerate, Build, NextProblem, PreviousProblem, Help, Settings }`.
  - `pub enum Scope { Everywhere, Level(Level) }`; `pub struct Binding { pub key: Key, pub modifiers: Modifiers }`.
  - `pub struct Shortcut { pub id: &'static str, pub binding: Binding, pub action: Action, pub label: &'static str, pub scope: Scope }`; `pub const SHORTCUTS: &[Shortcut]`: **all of spec section 4**, also what is not built, so that double assignments and stable ids hold for the final set from today.
  - `pub fn action_exists(action: Action, app: &NavesApp) -> bool`: a level or tool in `levels`, `SwitchView` with `views.can_switch()`, others once built. Exhaustive `match`, so a new `Action` cannot be forgotten.
  - `pub fn action_for(input: &egui::InputState, level: Level, typing: bool) -> Option<Action>`, with exact modifiers (`Modifiers::matches_exact`), so that Ctrl+B is not B. `typing` silences everything without Ctrl or F-key. Escape, Enter and F-keys still work while typing.
  - `pub fn typing(ctx: &egui::Context) -> bool`: `ctx.text_edit_focused()`. If a `DragValue` in edit mode does not count there, the test below says so and this function gets what it needs.
- Produces: `NavesApp::run_action(&mut self, action: Action)`, called at the top of `show`. F1: a sheet listing `SHORTCUTS` that exist for the active level, grouped Everywhere, then the level. Tooltips on stack rows name the key. `SwitchView` shows in the sheet only with two views.

Ids: `level.terrain`, `tool.move`, `tool.terrain.smooth`, `view.switch`, `build`, as the spec's two examples set them. Common tools (Q, W, E, R, A, N, L, B) are `Everywhere`; the level's own (`S`, `T`, `M`, `O`, `K`, `J`, `G`, `C`) are per level, one row per level that has them.

Built in this task: `Level`, `Cancel` (Esc: back to Select), `Build` (Ctrl+B), `Help` (F1), `SwitchView` (Ctrl+Tab). `action_exists` is false for the rest, and they never fire.

- [ ] **Step 1: Write the failing tests** in `tests/shortcuts.rs`:
  - The table: `no_key_is_assigned_twice_in_a_scope`, `no_level_key_repeats_an_everywhere_key` (over the whole table); `every_key_of_the_spec_is_in_the_table` (a literal list of section 4 against the table); `ids_are_unique_and_pinned` (the ids as a literal list, "stable forever"); `no_default_needs_shift_or_altgr_on_qwertz` (no `[`, `]`, `/`, `?`, no `Y` or `Z` as a tool); `esc_enter_and_f1_are_as_the_spec_says`.
  - Behaviour: `a_digit_makes_a_level_active`; `a_level_that_is_not_built_ignores_its_digit` (4); `an_action_that_does_not_exist_does_nothing` (W, Ctrl+Z); `ctrl_b_builds_and_b_does_not`; `typing_in_a_field_fires_no_shortcut` (typing 4 in Name keeps the level and changes the name); `typing_in_a_drag_value_fires_nothing`; `a_clicked_checkbox_does_not_block_shortcuts` (click a stack row, press 1: level changes); `f1_shows_the_shortcuts_of_this_level_only`; `ctrl_tab_is_listed_only_with_two_views`; `ctrl_tab_switches_views` (with `FakeViewport`) and `does_nothing_with_one_view`; `ctrl_tab_does_not_move_focus` (no widget focused afterwards); `ctrl_tab_works_while_typing` (from Name: the view switches, the text stays); `tab_still_moves_focus` (with one view and with two); `tooltips_show_the_key`; `the_manual_lists_every_shortcut` (reads `docs/manual/src/shortcuts.md`, all rows of `SHORTCUTS` by label and key).
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p naves shortcuts`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p naves`. Expected: PASS. Write `shortcuts.md` from the table, and `make docs`.
- [ ] **Step 5: Commit** `feat(gui): one table of keyboard shortcuts, F1 and level keys`.

### Task 5: Layers, with one real layer

**Files:**
- Create: `crates/naves/src/gui/layers.rs`, `crates/naves/src/gui/tests/layers.rs`
- Modify: `crates/naves-core/src/render.rs`, `crates/naves-core/tests/render.rs`, `crates/naves/src/gui/levels/mod.rs`, `crates/naves/src/gui/levels/terrain.rs`, `crates/naves/src/gui/stack.rs`, `crates/naves/src/gui/shortcuts.rs`, `crates/naves/src/gui/viewport/mod.rs`, `crates/naves/src/gui/viewport/map.rs`, `crates/naves/src/gui/testing.rs`, `docs/manual/src/gui.md`, `CHANGELOG.md`

**Interfaces:**
- Produces, in `naves_core::render`:
  - `pub const WALK_LIMIT_DEG: f64 = 60.0;`. The doc comment says it is provisional, what it comes from, and that roads Task 1 measures it.
  - `pub enum MapLayer { TooSteepToWalk }` (`Copy + Ord`), with `pub const ALL: &[MapLayer]` (the drawing order, bottom first) and `pub fn id(self) -> &'static str` (`too_steep_to_walk`).
  - `pub struct LayerData<'a> { pub heightmap: &'a Heightmap }`.
  - `pub fn render_layer(layer: MapLayer, data: &LayerData<'_>, req: &RenderRequest) -> Result<RgbaImage, RenderError>`: transparent where it draws nothing. The hatching is in image pixels: diagonal, every 6 px, ink at 55 %.
- Produces, in `gui::layers`: `pub struct LayerDef { pub layer: MapLayer, pub name: &'static str, pub tooltip: fn() -> String, pub key: fn(&mut egui::Ui) }`; `pub fn visible(panel: &LayerPanel, levels: &[&LevelDef], fade: f32) -> Vec<VisibleLayer>`, in `MapLayer::ALL` order: nothing from a hidden level, opacity 1 for the active level, `fade` for others, 1 with `show_others_in_full`.
- Changes: `pub struct VisibleLayer { pub layer: MapLayer, pub opacity: f32 }` in `gui::viewport`; `LevelDef.layers: &'static [LayerDef]`; `LayerPanel` gains `on: BTreeSet<MapLayer>`, `hidden: BTreeSet<Level>`, `show_others_in_full: bool`; `ViewFrame.layers: &[VisibleLayer]`. `MapViewport` keeps one texture per layer, keyed by `(MapLayer, RenderRequest, generation)`, and paints them with `Color32::WHITE.gamma_multiply(opacity)`, so that **a fade or a hide renders nothing**. Default on: none, level 0 active, so a new project opens clean.
- Built: `HideLevel` (H), `ShowOthersInFull` (Shift+H). Making a hidden level active shows it. Tooltip: `Ground steeper than 60°. Players cannot walk up it.`, the number from the constant.

- [ ] **Step 1: Write the failing tests.**
  - `render.rs`: `each_layer_is_transparent_where_it_draws_nothing` (over `MapLayer::ALL`, flat `Heightmap`); `hatching_marks_only_ground_above_the_limit` (a heightmap with one known wall); `hatching_stays_at_55_percent` (no pixel above that alpha); `drawing_order_is_stable`; `the_whole_map_can_be_asked_for` (a request for the map's bounds has hatching over the wall and none elsewhere: the 3D view will ask this way).
  - `tests/layers.rs`: `layers_are_checkboxes_with_their_names`; `the_key_shows_only_when_the_layer_is_on`; `a_level_s_layers_show_only_when_it_is_open`; `visible_follows_fade_hide_and_full` (a table: active, other, hidden, Shift+H); `h_hides_the_active_level_and_shift_h_shows_the_others_in_full`; `making_a_hidden_level_active_shows_it`; `switching_renders_nothing` (`renders` stays put for H, Shift+H and a level change, once a layer is on); `the_view_gets_the_same_layers_in_both_modes` (the fake gets the same `VisibleLayer` slice as the map); `layer_state_does_not_touch_the_project_file`; `the_tooltip_has_the_number_of_the_constant`.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p naves-core --test render` and `cargo test -p naves layers`. Expected: FAIL.
- [ ] **Step 3: Implement.** `render_layer` shares the sampling loop with `render_topdown` (a private helper, parallel by row), no second copy.
- [ ] **Step 4: Run** `cargo test -p naves -p naves-core`. Expected: PASS.
- [ ] **Step 5: Commit** `feat: switchable map layers in the stack, starting with too steep to walk`.

### Task 6: Selection, framing and panning

**Files:**
- Create: `crates/naves/src/gui/selection.rs`, `crates/naves/src/gui/tests/selection.rs`
- Modify: `crates/naves/src/gui/levels/mod.rs`, `crates/naves/src/gui/levels/terrain.rs`, `crates/naves/src/gui/levels/ground.rs`, `crates/naves/src/gui/viewport/map.rs`, `crates/naves/src/gui/stack.rs`, `crates/naves/src/gui/shortcuts.rs`, `crates/naves/src/gui/mod.rs`, `crates/naves/src/gui/testing.rs`, `docs/manual/src/gui.md`, `CHANGELOG.md`

**Interfaces:**
- Produces, in `gui::selection`: `pub struct Selection { pub level: Level, pub title: String, pub rows: Vec<(String, String)>, pub origin: Option<String>, pub at: WorldPoint, pub span_m: Option<f64> }`; `pub fn pick_at(app: &NavesApp, at: WorldPoint, modifiers: Modifiers) -> Option<Selection>`: the active level only, or with Alt every built level from the top down, first hit.
- Changes: `LevelDef.pick: Option<fn(&NavesApp, WorldPoint) -> Option<Selection>>`. Terrain: position, latitude and longitude from `MapGeo`, height, slope. Ground: position and ground class. `NavesApp.selection: Option<Selection>`.
- Built: `Cancel` clears the selection on a second press, `FrameSelection` (F), `FrameMap` (Shift+F), Space + drag and middle button pan, the left button no longer does. `SHORTCUTS` rows are unchanged.
- The shell draws the selection's marker (a ring with a white edge, constant size) through `Viewport::project`, and the properties under the stack, with `origin` as the last line. Clicks reach `pick_at` as `ViewEvent::Click`, whichever view sent them. Selection is dropped in `reload_heightmap`.

- [ ] **Step 1: Write the failing tests** in `tests/selection.rs`: `a_click_selects_only_in_the_active_level` (in level 0 nothing); `a_click_reads_height_and_slope_in_terrain` (values against `Heightmap::sample`); `a_click_reads_the_class_in_ground`; `alt_click_switches_to_the_level_it_hit` (with `FAKE_LEVEL`, whose `pick` hits); `alt_click_prefers_the_higher_level`; `the_fake_view_can_select_too` (an event from `FakeViewport` gives the same selection: tools are not tied to the map); `esc_clears_the_selection`; `f_frames_the_selection`, `shift_f_frames_the_map`; `framing_without_data_does_nothing`; `a_changed_map_drops_the_selection`; `the_marker_uses_project_of_the_view` (the fake's answer `None` draws nothing and does not panic); `space_drag_pans_and_a_left_drag_does_not`; `middle_drag_pans`; `space_on_a_focused_checkbox_does_not_pan`; `a_hidden_active_level_selects_nothing`.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p naves selection`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p naves`. Expected: PASS. Update the manual (the pan keys change for existing users) and the status lines: `Select: click to read the height and slope here.`, `Select: click to read the ground here.`
- [ ] **Step 5: Commit** `feat(gui): click to read a point, frame it, pan with Space or the middle button`.

### Task 7: The dock and Problems

**Files:**
- Create: `crates/naves/src/gui/dock.rs`, `crates/naves/src/gui/problems.rs`, `crates/naves/src/gui/tests/dock.rs`
- Modify: `crates/naves/src/gui/mod.rs`, `crates/naves/src/gui/imagery.rs`, `crates/naves/src/gui/stack.rs`, `crates/naves/src/gui/shortcuts.rs`, `crates/naves/src/gui/layout.rs`, `crates/naves/src/gui/testing.rs`, `docs/manual/src/gui.md`, `CHANGELOG.md`

**Interfaces:**
- Produces, in `gui::dock`: `pub struct DockTabDef { pub id: &'static str, pub label: fn(&NavesApp) -> String, pub show: fn(&mut NavesApp, &mut egui::Ui) }`; `pub const TABS: &[DockTabDef]` (Build, Problems); `NavesApp.dock: &'static str` (Build by default). Build tab: today's `build_ui`. The Settings button at the right end of the row joins in Task 10.
- Produces, in `gui::problems`: `pub struct Problem { pub message: String, pub at: Option<WorldPoint>, pub level: Option<Level> }`; `pub struct ProblemList { pub rows: Vec<Problem>, pub all: bool, pub cursor: Option<usize> }`, with `shown(&self, active: Level) -> Vec<usize>` (the level's rows and those of no level, or everything with `all`), `next(&mut self, active: Level) -> Option<&Problem>`, `previous(...)`, wrapping.
- Sources today: `BuildOutput.warnings` and failed builds' errors (cleared when a build starts), imagery `MergeReport` warnings (replaced with each `MapData`), `form.error`. They stay in the log as well. Nothing has a position yet, so a row without one does not move the map.
- Built: `NextProblem` (F8), `PreviousProblem` (Shift+F8). The row Problems above the stack: a checkbox, on by default, named `Problems 7` or `Problems`. Markers: diamond with a white edge, `ERROR`, constant size, through `project`. A row with `at` centres the view through `set_focus`, keeping the span.

- [ ] **Step 1: Write the failing tests** in `tests/dock.rs`: `the_tabs_are_build_and_problems`; `a_new_tab_needs_only_an_entry` (`TABS` drives the row; a fake list in a test of the row function); `build_warnings_become_problems`; `merge_warnings_become_problems` and `are_replaced_when_the_map_reloads`; `a_form_error_is_a_problem_until_fixed`; `the_count_matches_the_list`; `no_problems_says_so_and_shows_no_zero`; `problems_follow_the_active_level_unless_all_is_on`; `a_row_goes_to_the_map` and `f8_goes_to_the_next_problem_and_centres_the_map` (seeded with positions, also with the fake view); `f8_wraps_and_shift_f8_goes_back`; `f8_with_no_position_does_not_move_the_map`; `f8_with_no_problems_does_nothing`; `the_dock_folds_to_its_tabs_in_a_low_window`; existing build tests stay, clicking "Build" still builds.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p naves dock`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p naves`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(gui): the bottom dock with Build and Problems`.

### Task 8: Rail and palette, waiting for their first tools

**Files:**
- Create: `crates/naves/src/gui/rail.rs`, `crates/naves/src/gui/palette.rs`, `crates/naves/src/gui/tests/rail.rs`
- Modify: `crates/naves/src/gui/levels/mod.rs`, `crates/naves/src/gui/mod.rs`, `crates/naves/src/gui/shortcuts.rs`, `crates/naves/src/gui/testing.rs`

**Interfaces:**
- Produces, in `gui::levels`: `pub struct ToolDef { pub tool: ToolId, pub word: &'static str, pub status: &'static str, pub on_event: Option<fn(&mut NavesApp, &ViewEvent)> }`. `LevelDef.tools: &'static [ToolDef]`. Select is implied in every level with a `pick`.
- Produces, in `gui::palette`: per spec section 3, plus `group`, which it needs for "groups by the level's own terms":

```rust
pub struct PaletteItem { pub id: String, pub name: String, pub group: String, pub thumbnail: Thumbnail,
    pub kind: ItemKind, pub default_tool: ToolId, pub source: ItemSource }
pub enum Thumbnail { Swatch([u8; 3]), Line { width_m: f64, rgb: [u8; 3] }, Image(egui::TextureId) }
pub enum ItemKind { Preset, Model, Surface, LineClass, Group }
pub enum ItemSource { Game, Mod, Project }
pub struct Palette { pub search: String, pub last_used: BTreeMap<Level, Vec<String>> }
pub fn filter(items: &[PaletteItem], search: &str) -> Vec<&PaletteItem>;
```

- `LevelDef.items: fn(&NavesApp) -> Vec<PaletteItem>`, empty in every `DEF` so far. `NavesApp.tool: ToolId`, `NavesApp.item: Option<String>`.
- **Rule:** the rail is drawn when the active level has a tool other than Select, the palette when it has items, otherwise `panel`. Both as the spec's section 7 says. `ViewEvent`s go to the active tool's `on_event`; with Select to `pick_at`.
- Rail look: 56 px. Each tool is its letter, one step up, with its word below, from `Bindings`-to-be (`SHORTCUTS` until Task 10). The active tool is filled in the level's colour with `band_text`. Status line reads the tool's `status`.

- [ ] **Step 1: Write the failing tests** in `tests/rail.rs`, with a `FAKE_LEVEL` that gains two tools and three items: `production_levels_show_no_rail_and_no_palette`; `the_rail_shows_the_level_s_tools`, `only_letters_that_apply_are_drawn`; `a_letter_picks_the_tool`, `a_click_on_the_rail_picks_it_too`; `esc_returns_to_select`; `the_status_line_follows_the_tool`; `tool_events_arrive_in_metres_from_either_view`; `the_palette_replaces_the_panel`; `picking_an_item_switches_to_its_tool`; `another_tool_can_be_picked_after`; `ctrl_f_focuses_the_search_and_letters_then_type` (typing "w" filters, no Move); `search_filters_by_name_and_group`; `last_used_comes_first`; `groups_are_in_stable_order`; `each_item_says_what_it_is` (the words "Preset" and "Single model" are there, per spec section 3).
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p naves rail`. Expected: FAIL.
- [ ] **Step 3: Implement.** Mark what only tests build, per *Global Constraints*, naming milestone 2.
- [ ] **Step 4: Run** `cargo test -p naves` and `make lint`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(gui): the tool rail and the items panel, shown when a level has them`.

### Task 9: Preferences file and tool lookup

**Files:**
- Create: `crates/naves-core/src/prefs.rs`, `crates/naves-core/tests/prefs.rs`, `fuzz/fuzz_targets/prefs_parse.rs`, `fuzz/corpus/prefs_parse/*`
- Modify: `crates/naves-core/src/lib.rs`, `crates/naves-core/src/imagery/catalog.rs`, `crates/naves-core/tests/fuzz_seeds.rs`, `crates/naves-export/src/tools.rs`, `crates/naves-export/tests/` (tool lookup), `crates/naves/src/cli.rs`, `crates/naves/src/mcp.rs`, `crates/naves/tests/cli.rs`, `fuzz/Cargo.toml`, `docs/manual/src/installation.md`, `docs/manual/src/troubleshooting.md`, `CHANGELOG.md`

**Interfaces:**
- Produces, in `naves_core::prefs` (no egui, so that it can be fuzzed; the crate `naves` has no library):
  - `pub struct Preferences { pub shortcuts: BTreeMap<String, String>, pub map: MapPrefs, pub tools: ToolPaths }`, plus private captured unknown keys per table (`#[serde(flatten)]`). `shortcuts` holds only what differs from the default; `""` is no key.
  - `pub struct MapPrefs { pub invert_zoom: bool, pub fade_percent: u8, pub interface_scale: f32, pub no_unfold_animation: bool, pub start_base: String, pub start_layers: Vec<String> }`: defaults false, 40, 1.0, false, `hillshade`, empty.
  - `pub struct ToolPaths { pub armake2: Option<PathBuf>, pub steam_root: Option<PathBuf>, pub dayz_tools: Option<PathBuf>, pub dayz: Option<PathBuf>, pub dayz_server: Option<PathBuf>, pub proton: Option<PathBuf> }`.
  - `pub fn config_dir() -> Option<PathBuf>`, moved out of `user_catalog_path`, which now uses it. `pub fn default_path(config_home: &Path) -> PathBuf`. `Preferences::load(path: &Path) -> Result<Preferences, PrefsError>` (a missing file gives the defaults), `Preferences::parse(text: &str)`, `save(&self, path: &Path)` atomically (same way as `Project::save`, also creating the folder), clamping fade to 0..=100 and scale to 0.5..=3, a NaN to 1.
- Produces, in `naves_export::tools`: `pub fn locate_preferring(paths: &ToolPaths) -> Result<Tools, ToolError>`: `locate_with` and a layered `env` closure: environment, settings, search. `pub struct ToolOrigin { pub tool: &'static str, pub path: Option<PathBuf>, pub from: Origin }`, `pub enum Origin { Environment, Settings, Found, Missing }`, `pub fn tool_origins(paths: &ToolPaths) -> Vec<ToolOrigin>`. `ToolError::Missing` names the setting when the setting's path does not exist.
- `naves build` and `naves mcp` call `locate_preferring` with the file from `config_dir()`. A damaged file there is a warning on stderr, then environment and search.

- [ ] **Step 1: Write the failing tests.** `prefs.rs`: `a_missing_file_gives_the_defaults`; `values_round_trip`; `only_changed_values_are_written`; `unknown_ids_survive_a_save`, `unknown_keys_survive_a_save` (in every table); `a_damaged_file_is_an_error_and_is_not_overwritten`; `saving_is_atomic`; `out_of_range_values_are_clamped` (fade above 100, a scale of 0 or NaN); `the_project_file_format_is_untouched`. Seeds for `prefs_parse`, also run from `fuzz_seeds.rs`. Lookup: `the_environment_beats_the_settings`; `the_settings_beat_the_search`; `a_path_in_the_settings_that_does_not_exist_names_the_setting`; `origins_say_where_each_path_came_from`. CLI: `naves_build_uses_the_settings` (`XDG_CONFIG_HOME` in a temp dir, a fake armake2 named only there); `a_damaged_settings_file_does_not_stop_a_build`.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p naves-core --test prefs`, `cargo test -p naves-export`, `cargo test -p naves --test cli`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test --workspace`, `make fuzz`, `make reuse`. Expected: PASS.
- [ ] **Step 5: Commit** `feat: per-user settings file, and tool paths from it`.

### Task 10: The settings window

**Files:**
- Create: `crates/naves/src/gui/prefs_window.rs`, `crates/naves/src/gui/tests/prefs_window.rs`, `docs/manual/src/settings.md`
- Modify: `crates/naves/src/gui/shortcuts.rs`, `crates/naves/src/gui/dock.rs`, `crates/naves/src/gui/layers.rs`, `crates/naves/src/gui/viewport/map.rs`, `crates/naves/src/gui/mod.rs`, `docs/manual/src/SUMMARY.md`, `docs/manual/src/shortcuts.md`, `docs/manual/src/gui.md`, `CHANGELOG.md`

The code says `Preferences`, because `gui/settings.rs` is the project's form. The window says "Settings".

**Interfaces:**
- Consumes: `Preferences`, `locate_preferring`, `tool_origins`, `SHORTCUTS`.
- Produces, in `gui::shortcuts`: `Binding::parse(text: &str) -> Option<Binding>` and `Display` (with `Key::from_name`, `Key::name`); `pub struct Bindings`, with `Bindings::new(overrides: &BTreeMap<String, String>) -> (Bindings, Vec<String>)` (the second value: what could not be read, in words), `key_for(&self, Action, Level) -> Option<Binding>`, `conflict(&self, new: Binding, action: Action) -> Option<Action>` (knows the scopes), `set`, `clear`, `reset`, `overrides(&self) -> BTreeMap<String, String>`. `action_for(input, bindings: &Bindings, level, typing)`. The rail, the tooltips and F1 read `Bindings`.
- Produces, in `gui::prefs_window`: `pub struct PrefsWindow { pub tab: PrefsTab, pub capturing: Option<Action>, pub pending: Option<(Binding, Action, Action)>, pub search: String }`; `pub enum PrefsTab { Keyboard, Map, ExternalTools }`. While open, `action_for` does not run. Esc closes, or cancels a capture first.
- Produces: `NavesApp::with_preferences(self, path: PathBuf) -> Self`, called by `run`. Without it: defaults in memory, nothing saved. `prefs_error: Option<String>`: a damaged file is reported in the window and never saved over; changes hold for the session, and the window says so. `locate_tools` in `run` reads the file fresh at each build.
- Built: `Settings` (F2), the dock's Settings button. The Map tab: wheel direction (`MapView`, `invert_zoom`), fade (`visible`), interface scale (`ctx.set_zoom_factor`), "Unfold without animation", start base map and layers (`LayerPanel::from_prefs`).

Copy, verbatim from the spec: `Press a key`; `R is Scale in every level.` with Replace and Cancel; `changed`, `Reset`, `no key`, `Reset all to defaults`. One line at the top: `These apply to every project. Each project's own values are in level 0.` External tools shows each tool's path and its origin in words, and a "Not saved: ..." line when a write fails. A key that egui has no name for (on a German board `ö`, `ü`, `ä`, `ß`) leaves the button at "Press a key": the manual says so.

- [ ] **Step 1: Write the failing tests** in `tests/prefs_window.rs`:
  - `the_button_and_f2_open_the_window`; `esc_closes_it`; `shortcuts_do_not_fire_while_it_is_open`; `no_setting_changes_the_project_file`; `tabs_are_in_the_spec_s_order`; `rows_are_grouped_by_level_in_the_stack_s_order_and_only_what_exists`.
  - `pressing_a_key_rebinds_and_saves`; `the_new_key_works_and_the_old_one_does_not`; `esc_cancels_a_capture`; `esc_enter_and_f1_cannot_be_changed`; `tab_and_shift_tab_cannot_be_assigned` (the capture says why and keeps listening; Ctrl+Tab can be changed); `a_used_key_asks_before_replacing`; `cancel_changes_nothing`; `replace_leaves_the_other_action_with_no_key`; `the_same_key_in_two_levels_is_no_conflict`; `a_global_key_conflicts_with_every_level`; `reset_restores_one_and_reset_all_restores_all`; `search_filters_the_rows`; `tooltips_and_f1_show_the_new_key`; `a_new_default_reaches_a_file_that_does_not_mention_it`.
  - `invert_zoom_inverts_the_wheel` (plain numbers on `MapView`); `fade_percent_changes_the_other_levels`; `start_layers_are_used_when_a_project_opens`; `a_start_layer_that_does_not_exist_is_ignored`; `the_zoom_factor_follows_the_scale`.
  - `a_damaged_file_shows_its_error_and_is_not_overwritten`; `a_bad_key_name_keeps_the_default_and_says_so`; `a_duplicate_in_the_file_is_shown`; `an_unwritable_file_is_reported`; `tests_never_touch_the_user_s_file` (`open` without `with_preferences` writes nothing with `XDG_CONFIG_HOME` in a temp dir).
  - `external_tools_show_the_origin`; `changing_a_path_is_used_by_the_next_build`.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p naves prefs_window`. Expected: FAIL.
- [ ] **Step 3: Implement.** Move the table's readers over to `Bindings`. `shortcuts.md` keeps the defaults and links to `settings.md`.
- [ ] **Step 4: Run** `cargo test -p naves`, `make docs`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(gui): settings for shortcuts, the map and external tools`.

### Task 11: Look at it

**Files:**
- Modify: whichever files what you see requires; `docs/manual/src/gui.md`, `docs/manual/src/images/map-hillshade.png` (replaced by `editor.png`), `docs/manual/src/getting-started.md`, `CHANGELOG.md`

- [ ] **Step 1: Run `naves gui` on `examples/berchtesgaden`** in a headless sway with `grim`. Take: each built level active; each base map; the layer on, faded and full, over Relief, Slope and Imagery; F1; every settings tab with a rebound key, a conflict and a damaged file; at 1180 × 760, at 1920 × 1080, and at a size where the dock and then the left panel are folded.
- [ ] **Step 2: Check it against *Design pass*, one point at a time, and these questions.** Does the stack read as a column in strata at a glance, and not as a menu? Is the active stratum obvious without colour vision (thickness, not only hue)? Is keyboard focus visible on the panel and on a coloured row? Does the hatching read on Slope, the worst case? Are the numbers aligned? Does the band's colour tell you the level while you look at the map? If an element says nothing, remove it, per the skill: one accessory off before leaving the house.
- [ ] **Step 3: Fix what does not read,** with a test where it is behaviour.
- [ ] **Step 4: Finish the manual:** what the window is, levels, keys, a new screenshot with the source named.
- [ ] **Step 5: Run** `TMPDIR=/var/tmp make check`. Expected: PASS, coverage at or above 85 %. **Commit** `docs: the editor's shell in the manual`.

### Task 12: Proof of the seam with real 3D

Time-boxed to two days, on a branch `spike/3d-view` that is **not merged**. It answers whether the promise of *2D and 3D* holds, before the houses plan relies on it. **Stop and report to the maintainer** if wgpu will not run on their machine or fails `make deny`.

**Files:**
- Create: `docs/research/2026-11-06-3d-view.md` (this one is merged, on the main branch of work)
- Modify on the spike branch only: `crates/naves/Cargo.toml`, `crates/naves/src/gui/viewport/`, `crates/naves/src/gui/mod.rs`
- Modify, if the proof finds a flaw: `crates/naves/src/gui/viewport/mod.rs` and its users, on the main branch of work, with a test

**Interfaces:**
- Consumes: `Viewport`, `ViewFrame`, `ViewEvent`, `render_layer`.

- [ ] **Step 1: Swap the backend.** `eframe` features `wgpu` in place of `glow`, `renderer: Renderer::Wgpu` in `run`. Answer: does `make deny` pass, what do `cargo tree` (crate count), the release binary and a clean build cost against glow, does `naves gui` start on the maintainer's machine, in a headless sway, and does `egui_kittest` still work?
- [ ] **Step 2: Write a `TerrainViewport`,** in `viewport/terrain.rs`: a grid mesh of the heightmap in a compute job, `egui_wgpu::CallbackTrait` to paint it, the base map from `render_topdown` over the whole map as its texture, an orbit camera. No `unsafe`. Register it in `default_views`. Answer: does anything need more than the trait gives? Expected trouble: `CallbackTrait` is `Send + Sync`, and GPU resources are registered from `eframe::CreationContext`, which `default_views` does not get.
- [ ] **Step 3: Walk the seam.** With real data: Ctrl+Tab there and back keeps the place; Too steep to walk is draped, and H, Shift+H and fading work on it (`VisibleLayer` is enough); a problem marker and the selection ring sit on the ground through `project`, and vanish behind the camera; a click gives the same `Selection` as on the map (ray against the heightmap, in `show`); level, band and status line do not notice; the empty states stay in the shell; the wheel is the view's own. Note what a view needs that a level or tool assumes about a top-down picture. The image hatching is known to be in image pixels when draped.
- [ ] **Step 4: Write `docs/research/2026-11-06-3d-view.md`:** each answer and the numbers, what the MCP's "3D view from a given camera" (spec 5.10) needs on top (offscreen rendering, no window), what the houses plan has to do, and a recommendation on when to switch the backend.
- [ ] **Step 5: Fix what the proof found** in the merged code, each with a failing test first, with `FakeViewport` growing what the real one needed. If it finds nothing, say so in the note. Delete the spike branch only after the maintainer has seen the note.
- [ ] **Step 6: Commit** `docs: what a real 3D view takes` (and `refactor(gui): ...` for any fix).

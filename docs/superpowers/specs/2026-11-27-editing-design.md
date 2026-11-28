<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Náves: editing roads, terrain and placed models

Date: 2026-11-27. Status: draft for approval. Extends `2026-11-04-naves-design.md` (sections 5.1, 5.2, 6) and
`2026-11-06-naves-editor-ui.md` (levels, rail, items panel, shortcuts); where this document and those disagree, this
one wins. UI parts made with the `frontend-design` skill.

## 1. Intent

Global data (Copernicus or Mapterhorn DEM, OpenStreetMap) is not exact enough everywhere, and the automatic decisions
(clean-up, lots, bridges, heights, earthwork) are sometimes wrong. The map maker needs to **override them by hand,
per OSM part**, in the GUI: edit and draw roads, replace bridges, place road pieces and other models, and shape the
ground. Maps made **without any OSM data** must be possible with the same tools: "an OSM layer without OSM as
reference".

Maintainer's decisions (2026-11-14, 2026-11-18, 2026-11-27):

- Both: OSM-style line editing (the generators still fit pieces and heights) **and** placing models by hand that the
  generators leave alone; built in that order.
- **Edited parts always win** over the OSM original and over automatic decisions.
- **Edits survive changes of the area**: moving the centre slightly, enlarging or shrinking the map.
- Bridges: any generated bridge can be **replaced by a (scaled) model**, including the concrete-plate bridges Náves
  builds over dips.
- **Earthwork** is automatic for a placed piece or feature but can be switched off, and **earthwork on its own**
  ("make space for something") is a tool on every level.
- A map without OSM data: adding streets must work; turning OSM on later for such a map is not required.
- Tool buttons and an item selector; the same selector later places trees, houses and any model.
- GUI chrome: dark mode, the default; no bright brownish colours; one palette usable with colour-vision deficiency.
  Map items keep the look DayZ has.

## 2. Goals, non-goals, anti-goals

**Goals**
- One edit model for every feature kind: roads and paths now, lots (car parks, PR #26), then rivers, railways, power
  lines and the other OSM features planned (memory: future OSM features).
- Edits are project data, diff-friendly text, applied at the start of every build, so a build stays reproducible.
- The GUI and later the MCP server use one edit API.

**Non-goals (this design)**
- Regenerating only the changed area (spec section 6 asks for it eventually); a full rebuild after edits is accepted.
- A map without a DEM (flat or imported heightmap, spec 5.1 "from-scratch"): its own small task later. A custom map
  here still has a DEM; it may simply ignore it with earthwork.
- Persistent, attributed edit history (spec 5.2): undo/redo is per session here; every edit records who made it.
- Terrain holes and bunkers (spec section 6).

**Anti-goals** (would make it a failure even if it works)
- An edit is lost or silently changed: by a rebuild, an OSM update, a DEM change (PR #25) or an area change.
- A build whose output depends on the order edits were loaded in, or on anything but the project folder.
- Editing that needs OSM to be present.
- A palette that relies on red against green, or bright browns, in the chrome.

## 3. Constraints

- AGENTS.md in full: untrusted input never panics (the edit files are untrusted: fuzz target, limits on counts and
  lengths), atomic writes, `BTreeMap` ordering, `thiserror` in libraries, test first, coverage ≥ 85 %.
- PRs #25 (DEM fetch, based on the unmerged `road-planning` branch) and #26 (lots) are not merged. Both change
  `features.rs`, `netclean`, `conform.rs` and `generate.rs`. The build integration is planned against their versions:
  pure new modules first, integration after they land.
- The road graph joins ways by **node id** (`roadnet::build_graph`); cut ends at the map edge get negative ids
  `-(way * 4096 + n + 1)` (`features::cut_id`), which needs a positive way id below 2^51.
- `edits/overrides.toml` exists (moved, deleted, replaced generated objects, in **map metres**, pinned by
  `road/<stretch>/<slot>`). It does not survive an area change and must be migrated.

## 4. Approaches

Considered (maintainer picked A on 2026-11-27):

- **A. Edits as a source layer next to OSM.** Edits are stored as data in geographic coordinates and applied right
  after the OSM import; every later stage sees the edited data. Picked: it keeps rebuilding from OSM, treats a drawn
  road like an OSM road (spec 5.1), and future feature kinds use the same mechanism.
- **B. Replayed log of operations.** Undo and attribution come free, but replay against a changed extract or a moved
  area is fragile: one step that no longer applies breaks all after it.
- **C. Freeze, then edit.** After the first generation the network becomes project data (Terrain Builder style).
  Simplest editing, but OSM updates and the planned automatic features are lost. Works against the intent.

## 5. The design

### 5.1 Principle: edits are stored in real-world terms

Every position is **latitude and longitude** (WGS84), every height and size of terrain edits **real metres**. The build
projects them with the project as it is now (`MapGeo::to_map`, `Scale::height`, and `Scale::horizontal` for terrain
sizes: brush radius, widths, `blend_m`; a 10 m wide pad is 5 m wide on a 1:2 map, like every other part of the
world). The GUI shows and takes map metres and converts. So moving the centre, changing the
extent or the scale keeps every edit where it was in the world. Exception: a placed model keeps its own game size and
its own scale factor; only its position is projected. **[my call]**, follows from "edits survive area changes".

An edit that lies outside the current map is kept unchanged in the file, is not applied, and is reported
(`edit_outside_map`, information). Nothing is ever deleted by the build.

### 5.2 Feature edits: an OSM-style diff

Stored in `edits/ways.toml`. The model is OSM's own (nodes and ways, like an osmChange), so the editor is "OSM-style"
and a map without OSM is a map whose whole OSM layer is this file.

- **Nodes** that are created or moved: id, `lat`, `lon`; a moved OSM node also stores `base` (its OSM position when it
  was moved).
- **Ways**: `action` = `add`, `modify` or `delete`; `tags` (OSM tags, so `features::classify` treats a drawn road like
  an OSM one); `nodes` (ordered ids); for `modify` and `delete` a `base` fingerprint of the OSM original (tags and node
  list) it was made against.
- **Ids.** OSM ids are plain integers. User ids are written `u<n>` in the file; inside the program they are
  `USER_ID_BASE + n` with `USER_ID_BASE = 2^40`, positive, far above any OSM id and below the 2^51 that `cut_id`
  needs. The file holds `next_id`, so ids are never reused. Feature ids of user ways read `user/way/<n>/<part>` and
  carry `Source::User` (the enum exists already).
- **Edit operations the GUI offers**, and what they store:
  - move a node → node entry (a node shared by two ways moves in both: it is one junction);
  - draw a road → `add` way plus new nodes; its ends may reuse existing node ids, which joins it to the network;
  - delete a road → `delete`;
  - change class, surface, bridge, layer → `modify` with the new tags;
  - replace a part between two nodes, cut at a node, join two ends → `modify` with the new node list (the whole way is
    stored, OSM style). **[my call]**: the whole way rather than a node range, because the stored way is then
    self-contained and "edits win" makes a finer merge pointless.
- **Náves attributes as tags** in a `naves:` namespace, for decisions that are not OSM's: `naves:earthwork=no`
  (switch off the automatic earthwork of this feature, 5.4), `naves:keep=yes` (keep the line as drawn, see below). A bridge is marked OSM style: the way is cut at the bridge's ends and the middle part tagged `bridge=yes`. An edited
  or drawn way gets `naves:keep=yes` by default, which is how "edits always win" reaches the clean-up and the lot stage.
- **What the generators may still do to an edited way [my call]:** fit pieces, heights, junction pieces and its
  earthwork, as for any road. They do **not** change its line: no rerouting (road planning), smoothing, junction merge
  that moves its nodes, sidepath or stub removal. A drawn bend too tight for the pieces is reported
  (`road_curve_too_tight`), not rebuilt. All of this is what `naves:keep=yes` means; `naves:plan=yes` on a way lets
  road planning improve it like an OSM road (for "draw it roughly, let Náves route it").
- **Source.** Every way and node entry records `by = "user"` or `"ai"` (spec 5.10).
- **Applying** (`apply_way_edits`): runs on the `OsmWays` read from the extract (or an empty set without OSM), before
  `road_features`. Rules, in this order: node entries replace node positions everywhere; `delete` removes the way;
  `modify` replaces tags and nodes; `add` inserts. A node of an edited way that is neither in the extract nor in the
  file makes the way unusable: it is left out and reported (`edit_broken`, warning). A `modify` or `delete` whose OSM
  original is gone, or whose fingerprint no longer matches, is **still applied** (edits win) and reported
  (`edit_original_changed`, information). The lots of PR #26 (`OsmLand::lots`, way ids) get the same treatment: a
  closed way in the file with lot tags replaces or adds a lot; a deleted way removes it.

### 5.3 Placed models and replacements

Stored in `edits/objects.toml` (phase 4).

- **Placed model**: model path, `lat`, `lon`, yaw, scale, height (`ground` default, or `offset` above the shaped
  ground in metres), `earthwork` (default `fit` for road pieces, `none` for others, 5.4). Generators never touch it.
  A placed road piece is a placed model of a road piece; it is not part of the road graph. **[my call]**: joining a
  hand-placed piece into the generated network is not attempted; the line editor is the tool for network changes.
- **Replacing a generated object**: today's `moved`, `deleted`, `replaced`, re-keyed by a stable id that survives an
  area change: the OSM way and node range of its stretch plus its place along it, and a geographic position as a
  fallback match (within 25 m, as `SAME_OBJECT_M` does today).
- **Replacing a bridge**: one entry replaces the **whole** bridge, whichever kind the build chose (an OSM bridge, a
  plate grid over a dip, a fitted model): `bridge` = the geographic centre of the span and the OSM way id(s) of its
  stretch. **[my call]** It matches the built span whose centre is nearest, within 25 m, preferring one on the same
  way; no match is `edit_broken` (warning) and the entry is kept. The built bridge spans therefore carry their way ids
  and centre through the clean-up into the report (phase 4 work); `model`, `scale`,
  optional `yaw` and `offset`; `earthwork` = `fit` (abutments shaped to the model) by default. The bridge's road keeps
  its profile; the model is placed at the deck height the solve gave. **[verify in game, early in phase 4]**: a scaled
  bridge model keeps working collision, walking and driving, and the navmesh build accepts it.
- **Migration**: an existing `overrides.toml` is read with the project as it is and written to `objects.toml` on the
  next save; the old file is renamed `overrides.toml.migrated`. Until then the build reads both.

### 5.4 Terrain and earthwork

Stored in `edits/terrain.toml` (phase 3). Earthwork is one kind of edit, available on every level.

- **Shapes**: a pad (flat), a cut or fill to a target, a ramp between two heights, a ditch or embankment along a line.
  Geometry is a closed ring or a centre line with a width, in lat/lon; a `blend_m` band joins the shape to the ground
  around it. Target heights are either `offset` (real metres relative to the original ground at the shape's centre) or
  `level` (real metres above sea level); both are multiplied by the height scale. **[my call]**: `offset` is the
  default, so a shape keeps its meaning when the DEM source changes (PR #25).
- **Attached or alone.** A shape may name what it serves (a way id, a placed model, a bridge replacement); deleting
  that removes the shape too. Unattached, it is "earthwork only": making room for something added later.
- **Automatic earthwork**: features get it from the generators as today (`conform`); placed road pieces and bridge
  replacements get a generated fit shape; `naves:earthwork=no` or `earthwork = "none"` switches either off.
- **Brushes** (raise, lower, smooth, flatten): stored as **strokes** (points, radius in real metres, strength), not as
  pixels, so they are re-applied at any resolution and area.
- **Each brush stroke and shape says where it sits** (maintainer, 2026-11-27: "per edit"): `stage = "ground"` or
  `"finish"`.
  - `ground`: it corrects the ground the roads are fitted on (a DEM error, a cliff that is not there). Applied to the
    base before the road solve, and **held**: the road earthwork (`conform`, later the grid height solve as
    constraints) does not change vertices inside the shape's core (outside `blend_m`). A road that cannot meet it is
    reported by the existing checks (`road_misses_terrain`, `road_conflict`).
  - `finish`: a final touch on top of the finished terrain, applied after the road earthwork. It may bury or undercut a
    road; the checks on the final terrain report that.
  - Defaults **[my call]**: brushes and unattached shapes `ground`, fit shapes of placed models and bridge
    replacements `finish`. The GUI shows the choice as a switch in the tool's status line and in the properties.
- **Shapes are items, drawn with Area and Line (maintainer, 2026-11-28).** There is no key per shape. The selector
  has an "Earthwork" group on every level (the shapes `fit` all levels): area shapes (pad, cut or fill to a height,
  mound, pit) and line shapes (ditch, embankment, ramp, terrace step), each with a plan-view thumbnail at the shared
  scale. Picking a shape switches to the tool that draws it: an area shape to Area (V; clicks make a ring, a drag a
  rectangle, Alt+drag a circle), a line shape to Line (C; Space finishes). **Shift+V** picks the last used shape again
  on any level, Ctrl+F finds any shape. Height, slope, width, `blend_m` and `stage` are in the properties. "Shape the
  ground to fit" stays a switch in the properties of a way, a placed model or a bridge replacement. The Terrain level
  keeps its brushes (B raise or lower, R smooth, T flatten); its Shift+R picks the ramp shape, Shift+C stays the hole
  tool. A new shape later is a new item, not a new key.
- **Terrain stack**, in order: base heightmap (DEM) → `ground` strokes and shapes, in file order → the road solve and
  `conform` (holding the `ground` shapes) → `finish` strokes and shapes, in file order → fit shapes of placed models.

### 5.5 Build integration

- New module `naves_core::edits` (files, parsing, validation, applying), one function per edit kind; the build reads
  `edits/` once and passes the parsed set down. Missing files mean no edits.
- New rules in `validate::Rule`: `edit_outside_map` (information), `edit_broken` (warning), `edit_original_changed`
  (information); `orphaned_edit` stays for objects.
- `read_features` runs with an empty `OsmWays` when the project has no OSM sources but has way edits.
- `report.json`, the build's progress line and the MCP `build` result count applied, skipped and changed edits.

### 5.6 The GUI

**What is edited [my call]:** the source layer, OSM style. The GUI keeps the `OsmWays` of the last read with the edits
applied (a cheap step of the build, cached by the worker) and draws it as the editing overlay over the built map; a
click selects a source way or node, never a built piece. Positions shown on the built map and on the source layer
differ where the build moved or rerouted a line; the overlay makes that visible.

The layout, levels, rail and shortcut rules of `2026-11-06-naves-editor-ui.md` stay; these parts are new or changed.

**Theme (changes section 6 of the UI spec).** Dark is the default, light stays available in the settings (Map tab).
Chrome palette, derived from the Okabe–Ito set so that hues stay apart under the common colour-vision deficiencies,
and never carrying meaning by colour alone:

| Token | Hex | Use |
|---|---|---|
| ground | `1e2329` | window background (a blue slate, not a neutral black) |
| panel | `272d35` | panels, dock |
| raised | `323a44` | hovered rows, keycaps |
| ink | `e4e8ec` | text |
| ink-quiet | `9aa4ae` | secondary text, inactive keys |
| active | `56b4e9` | active tool, selection, focus ring (sky blue) |
| problem | `d55e00` | errors, always with a ▲ glyph and a word (vermilion, not brown) |
| notice | `f0e442` | warnings, always with a ● glyph |

Level bands move off the browns: Terrain `cc79a7` (reddish purple), Ground `009e73` (bluish green), Water `0072b2`,
Roads `e4e8ec` (pale tarmac line on the dark chrome), the others keep their hue family but get a lightness that passes
3:1 against `ground`. The band always shows the level's digit and name, so colour is never the only cue.

**The one memorable thing** stays the stack. Editing adds no second loud element: edited parts are marked on the map
with an overlay of the chrome (not a recolour of map items): a thin `active`-coloured outline with square node
handles while selected, and a small tick at each end of an edited or drawn way at any time, so "what did I change"
is always visible. AI proposals keep the dashed outline of the UI spec.

**Item selector (left panel), Roads level.**

```
+----------------------------+
| Search roads        Ctrl+F |
| Last used                  |
|  ━━━━  Secondary, asphalt  |
|  ────  Track, gravel       |
| Roads                      |
|  ━━━━━ Primary             |
|  ━━━━  Secondary           |
|  ━━━   Tertiary            |
|  ──    Track (grade 1-5)   |
| Paths                      |
|  ┄┄    Footpath            |
| Bridges                    |
|  [img] Plate grid (auto)   |
|  [img] bridge_wood2        |
|  [img] bridge_concrete3    |
+----------------------------+
```

Roads are drawn as their line at true width (UI spec section 3), bridges with the catalog thumbnail. Picking a road
class switches to Draw (N); picking a bridge model with a bridge selected replaces it (phase 4). The list comes from
the tags `classify` understands and from the install's catalog; it is the same for a map without OSM.

**One selector for every level, with two scopes (maintainer, 2026-11-28).** The selector is one widget that every
level and every later phase uses. Above the list a two-part switch picks its scope:

- **the level's own** (labelled with the level's name, "Roads"; the default): only what fits the active level, the
  road classes and the road pieces, junctions and bridges in the Roads level;
- **Everything**: every item of every level and every model of the catalog, so that a road piece can be placed while
  you work on houses, or used for something else. Each item from another level carries that level's band chip (its
  digit and colour), and picking it leaves the active level as it is: the tool works with the item, a road class
  still draws a road.

An item says which levels it fits (`fits`), so the scope is a filter, not a second list. The scope is per level and
kept in the settings. Phase 2 lists the catalog models with their footprint and shows them in the properties;
placing a model is phase 4, through the same selector.

**Tools.** The keys are those of the UI spec section 4 as revised on 2026-11-28 (left hand on the keyboard, WASD and
Q/E move the view). Roads level: Esc select (a way, or a node when near one), G move a node (drag), C draw (click
points, click an existing node to join, Space, Enter, a right click or a double click to finish), R cut at a node,
T join two ends, Shift+T mark or unmark a bridge between two nodes, X delete. Terrain level: B raise/lower, R smooth,
T flatten, Shift+R ramp, and the shape tools (pad, cut/fill, ditch). **Earthwork on every level**: a tool that draws an unattached shape, and
in the properties of a selected feature or model an "Shape the ground to fit" switch. It is not a key of its own:
the shapes are items drawn with Area (V) and Line (C), and Shift+V repeats the last shape (5.4).

**Properties (right, under the stack)** for a selected way: class, surface, bridge, layer, the `naves:` switches,
where it came from ("OSM way 8421, edited by you" / "drawn by you" / "OSM, unchanged") and **Reset to OSM** for an
edited OSM way.

**After an edit** the change is saved at once (atomic write), drawn at once as the edited line over the old build,
and the status line says "Edited: press Ctrl+R to rebuild". Ctrl+R (F5 too) runs the full build in the worker (as the Build tab
does). Undo and redo (Ctrl+Z, Ctrl+Shift+Z) work on the edit set: each operation stores the edit files' state before
it.

### 5.7 MCP

The edit operations are one API in `naves_core::edits` (add, modify, delete, move node, reset; later shapes and
models). The MCP server gets tools over the same API in phase 5; in approval mode they become proposals.

## 6. Phases

Each phase has its own plan and ends with a working, tested state.

1. **Edit set and build** (no GUI): `edits/ways.toml`, ids, parsing with limits, `apply_way_edits`, `naves:` tags in
   the clean-up and lot stage, the new rules, build without OSM. Testable from files and the CLI build.
2. **Road line editing in the GUI**: dark theme, rail and selector for the Roads level, the tools above, properties,
   save, undo, F5.
3. **Terrain and earthwork**: `edits/terrain.toml`, shapes, strokes, holds in `conform`, the tools on the Terrain level
   and earthwork on every level.
4. **Placed models and replacements**: `edits/objects.toml`, migration of `overrides.toml`, bridge replacement with
   scaled models (game check first), placing road pieces and any catalog model with the same selector.
5. **MCP edit tools**.

Phases 1 and 2 are written so that they can start now on `main`; the integration tasks of phase 1 wait for #25 and
#26 to land (see the plan).

## 7. Left to the builder

- Exact TOML layout of the files within the rules above; limits on counts and lengths (they must exist).
- The fingerprint function (any stable hash of tags and node list, in the file as hex).
- How the GUI hit-tests ways and nodes (pixel tolerance), drawing details of handles and ticks.
- Undo stack depth.

## 8. Designer's calls to check

1. Defaults for the per-edit stage (5.4): brushes and unattached shapes `ground`, fit shapes `finish`.
2. A modified way is stored whole, not as a node range (5.2).
3. Edited and drawn ways get `naves:keep=yes` automatically (5.2); maintainer confirmed that the line is kept and only
   fitted (2026-11-27), `naves:plan=yes` lets road planning improve it.
7. The GUI edits the source layer, not the built network (5.6).
8. Bridge replacements match the nearest built span within 25 m (5.3); terrain sizes scale with the map (5.1).
4. Placed road pieces do not join the road graph (5.3).
5. The dark palette and the new level band colours (5.6).
6. Phase order: line editing before terrain, although the request named terrain first, because earthwork attaches to
   features and models and needs them to exist.

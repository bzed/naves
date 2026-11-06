<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Náves: the editor's interface

Date: 2026-11-06. Status: draft for approval. It extends sections 5.9, 5.10, 6 and 7 of
`2026-11-04-naves-design.md` and describes the **final** interface, so that each plan builds a part of one design and
nothing has to move later. Section 7 says which plan builds what. Milestone 3 gets its own design; what is here for
it is the place it will go. Made with the `frontend-design` skill.

## 1. Who and what for

A map maker, working for hours, often with one hand on the keyboard: looks at what the generators made, fixes it,
adds to it. The AI does the same through MCP. The map is the work, and everything else stays out of its way.

## 2. The idea: levels, stacked like the map

The world is built in layers, from the source data up to what lives on it. The interface says so: **a level is what you
are working on**, and picking one decides everything else on the screen: which items you can add, which tools you have,
and what a click on the map selects.

The right side of the window is the **stack**, drawn like a geologist's column: the levels in their real order, bottom
up, each with its own colour band, and the active one open. It is also the map's legend. This column is the one
memorable thing in the interface, and everything around it stays quiet.

| Key | Level | You add | Map layers |
|---|---|---|---|
| 0 | Project | area, scales, sources (settings, no items) | |
| 1 | Terrain | shapes: pad, ditch, embankment, hole | Terrain changes, Too steep to walk, Terrain holes |
| 2 | Ground | surfaces: grass, rock, forest floor, soil | (the Ground base map), Painted areas |
| 3 | Water | lake, river, pond | Water, Water depth, Water leaks |
| 4 | Roads and paths | road classes, track, path, railway, bridge | Roads, Paths, Railways, Drivable |
| 5 | Vegetation | fill presets (spruce forest, mixed forest, scrub, tree row), single trees | Trees, Bushes, Fill areas |
| 6 | Buildings | houses by style and use, compositions, bunkers | Buildings, Style, Entrances, Dropped for scale |
| 7 | Objects | fences, walls, power lines, props | Fences and walls, Power lines, Props |
| 8 | Life | infected groups (village, city, farm, military, industrial), animal groups (forest, open, mountain), player spawns | Infected areas, Animal areas, Spawns |
| 9 | Economy | loot tiers, vehicle and event spawns | Loot tiers, Events |

Above the stack, always: the base map (one of Relief, Slope, Height, Ground) and Problems with its count. They belong to
no level.

Rules:

- **A level appears when it can do something.** There are no greyed-out promises. The number keys never change, so
  key 4 is Roads and paths even while 3 does not exist yet.
- **A click selects only in the active level.** You cannot move a house while working on the forest. Alt+click picks
  across levels and switches to the level of what it hit.
- **The other levels fade while you work:** drawn at 40 % so that you see what you work against. Shift+H shows them in
  full. H hides the active level. Making a hidden level active shows it.
- **The stack is not a list of files.** Levels have a fixed order. Layers belong to their level and cannot be moved.

## 3. Items: what you can add

The left panel holds what the active level adds. One item model for all levels, so that one widget serves all of them:

`PaletteItem { id, name, thumbnail, kind (preset, model, surface, line class, group), default_tool, source (game, mod, project) }`

- **Search** (Ctrl+F) on top, then the last used items, then groups by the level's own terms: style and use for buildings,
  biome for vegetation, class for roads, kind for groups.
- **Thumbnails** from the catalog (spec 5.11) where there is a model. A swatch where there is a surface. The line itself,
  drawn at true width, where it is a road class, the same as in the legend.
- **Picking an item switches to its tool:** a house to Place, a road class to Line, a forest to Area. Esc goes back to
  Select. You can always pick another tool: a forest can be brushed.
- **An item is a preset or a single model, and it says so.** A preset goes through the generators and can regenerate. A
  single model is a placed object that generators never touch (spec 5.2).
- Level 0 has no items: the panel holds the project settings. A level without items yet shows its numbers there.

## 4. Tools and keyboard shortcuts

The rail between the left panel and the map shows the tools of the active level. **Each tool is drawn as its key**, a
letter with its word below it, so the rail teaches the keyboard and needs no icon set. 56 px wide.

The same letter always means the same thing, in every level where it applies:

| Keys | Tool |
|---|---|
| Q | Select (the default; Esc returns here) |
| W, E, R | Move, Rotate, Scale |
| A, N, L, B | Add as a point, Line, Area, Brush |

Then what a level has of its own, and only there:

| Level | Keys |
|---|---|
| Terrain | B raise or lower (Shift inverts), S smooth, T flatten, M ramp, O hole |
| Ground | B paint, S soften an edge |
| Water | L lake, N river, B shape the bed |
| Roads and paths | N draw, K cut, J join, G mark a bridge |
| Vegetation | L fill an area, N along a line, B brush, A one tree, K cut out |
| Buildings | A place, C make a composition, G tag an entrance, O hole for a bunker |
| Objects | A place, N along a line, C make a composition |
| Life | L draw an area, A place a group or a spawn |
| Economy | B paint a tier, A place an event |

Everywhere:

| Keys | Action |
|---|---|
| 0 to 9 | Go to a level |
| H, Shift+H | Hide the active level; other levels faded or in full |
| Tab | Switch between map and 3D view |
| F, Shift+F | Frame the selection; frame the whole map |
| Space + drag, middle button | Pan (wheel zooms) |
| X, Shift+T | World or local axes; follow the ground |
| Delete, Ctrl+D | Delete, duplicate |
| Ctrl+Z, Ctrl+Shift+Z | Undo, redo |
| Ctrl+F | Search items |
| `,` and `.` | Brush smaller, larger (or Alt+wheel) |
| Ctrl+`,` and Ctrl+`.` | Brush weaker, stronger (or Ctrl+wheel) |
| Enter, Esc | Finish a line or area; cancel and go back to Select |
| F5 | Regenerate the active level in the changed area |
| Ctrl+B | Build |
| F8, Shift+F8 | Next problem, previous problem |
| F1 | Show all shortcuts for this level |
| F2 | Open the settings |

These are the defaults: every shortcut can be changed in the settings (section 8). Rules for the keyboard:

- **One table in the code** holds all default shortcuts, and a test fails on a double assignment. The rail, the tooltips and
  the F1 sheet show the keys in force, the manual's page lists the defaults.
- **The maintainer's keyboard is German.** Letters are logical keys, so a tool's letter is what the keycap says. No default
  uses `Y` or `Z` as a tool (they swap on QWERTZ), or a key that needs Shift or AltGr on QWERTZ: no `[ ]`, `/`, `?`.
- **Typing is typing.** While a text field has focus, no single-key shortcut fires.
- Q, W, E, R and "follow ground" match what STAG.vision and most editors use (research note 6.7).
- Holding Shift with a brush inverts it, and a tool's status line says so.

## 5. Layout

```
+----------------+---+--------------------------------------+-----------------+
|                | Q |##### band in the level's colour #####| Base map        |
|  Items         |   |                                      | (•) Relief      |
|  or settings   | W |                                      | ( ) Slope       |
|                | E |                                      | ( ) Height      |
|  search        | R |                                      | ( ) Ground      |
|  last used     |   |                                      | [x] Problems 7  |
|  groups        | A |             the map                  |                 |
|  thumbnails    | N |          or the 3D view              | 9 Economy       |
|                | L |                                      | 8 Life          |
|                | B |                                      | 7 Objects       |
|                |   |                                      | 6 Buildings     |
|                | K |                                      |#5 Vegetation####|
|                | J |                                      |  [x] Trees      |
|                | G |                                      |  [ ] Fill areas |
|                |   |                                      | 4 Roads, paths  |
|                |   |                                      | 3 Water         |
|                |   |                                      | 2 Ground        |
|                |   |                                      | 1 Terrain       |
|                |   | B  Brush: click to fill, Shift erases| 0 Project       |
|                |   |                                      | Selected: ...   |
+----------------+---+--------------------------------------+-----------------+
| Build | Problems 7 | History | AI proposals 2                      Settings   |
+-----------------------------------------------------------------------------+
```

- **Left, 300 px:** the active level's items, or its settings or numbers.
- **Rail, 56 px:** the level's tools. Letters that do not apply in a level are not drawn.
- **Middle:** the map. A band of 3 px in the active level's colour runs along its top, so that you always know what a
  click will change. A status line at its bottom says what the tool does and what to press: "Line: click to add points,
  Enter to finish, Esc to cancel."
- **Right, 230 px:** base map, Problems, the stack, and under it the selection's properties (spec 5.9, Terrain Builder's
  exact numeric fields, and where it came from: "Generated from road 8421, piece 17. Moved by hand, kept when
  regenerating.").
- **Bottom, resizable:** the Settings button at the right end of its tabs (section 8), always there. Build (button, log, Ctrl+B), Problems (rows go to the map; follow the active level unless "All"
  is on), History (spec 5.2: every change and who made it; a row undoes back to there), AI proposals (spec 5.10, approval
  mode: accept and reject each one; proposals are drawn on the map with a dashed outline).

The window scales down by folding the bottom dock, then the left panel. The map stays.

## 6. Look

- **Palette:** keeps the manual's and today's GUI: ground `f2f5ec`, panel `e7ede0`, ink `1d2924`, village green `2e6a4c`,
  error `b03a2e`. Levels:

| Level | Band |
|---|---|
| Project | `5a6660` slate |
| Terrain | `8c6a45` earth |
| Ground | `a39a5c` dry grass |
| Water | `3f6f9e` water |
| Roads and paths | `1d2924` tarmac |
| Vegetation | `2e6a4c` forest |
| Buildings | `9a4f3c` brick |
| Objects | `6b6f76` steel |
| Life | `7a4a7f` dusk |
| Economy | `b58a1e` ochre |

- **Type:** egui's default family, no new font. One size for rows, one step up for headings, the numbers in tables aligned.
  Sentence case. No small caps labels above sections: the stack's order explains itself.
- **On the map, colour is never the only cue:** hatching, line weight and ticks carry the meaning too. Tints stay below
  50 %, so that the terrain stays readable under any combination.
- **Motion:** only in answer to an action. The active level unfolds, and that is all.
- **Rejected:** a GIS layer list with eye icons (it says nothing about what is on top of what, and has no idea of a
  working level); a toolbar of custom icons (a set of 40 icons to draw, and they teach nothing); floating panels (the
  map gets covered, and the window layout has to be remembered); a dark theme by default (the map is mostly mid-tones
  and greens, and reads better on a light ground; a dark theme can come later, as the colours are constants).

## 7. What comes when

| Plan | Adds |
|---|---|
| Roads (plan 3) | The settings window with Keyboard, Map and External tools; the stack with levels 0, 1, 2 and 4; the map's band and status line; base maps and layers; Select to inspect (no editing); bottom dock with Build and Problems; the shortcut table with what exists by then; F1 |
| Houses | Level 6, Tab and the 3D view, History and undo |
| Water, Railways, Scaling | Their layers in levels 3, 4 and 6 |
| Vegetation | Level 5 and level 7's layers, view only |
| Milestone 2 (editor) | The tool rail, the items panel, all editing tools, properties you can change, AI proposals, the AI tab in the settings, brush defaults |
| Milestone 3 | Levels 8 and 9, in their own design, using the same items, tools and keys |

The rail and the item panel are not built before there is more than one tool and something to place. Until then, the left
panel shows settings or numbers, in the place the items will take.

## 8. Settings

The Settings button at the right end of the bottom dock, or F2, opens a window that dims the rest. It holds what belongs
to the person, not to the map: **it applies to every project**, and is stored per user in
`$XDG_CONFIG_HOME/naves/settings.toml` (on Windows the user's config folder), never in `naves.toml`, so that a project stays
the same for everyone who opens it. The window says so in one line and points to level 0 for the project's own values.

Tabs on the window's left, in this order, each saved as soon as it changes:

| Tab | What you change |
|---|---|
| Keyboard | every shortcut |
| Map | zoom direction of the wheel, how strongly other levels fade (40 %), interface scale, the base map and layers a project opens with |
| External tools | paths to armake2, the Steam folder, DayZ Tools, DayZ and Proton; each shows the path in use and where it came from |
| AI | the MCP server on or off, its default mode (from milestone 2, when the editor hosts the server) |

Brush defaults join Map when there are brushes. A dark theme would be a line here when it exists.

**Keyboard tab.**

```
| Search shortcuts: [            ]                        |
|                                                         |
| Everywhere                                              |
|   Go to Terrain            [ 1 ]                        |
|   Hide the active level    [ H ]                        |
|   Build                    [ Ctrl+B ]   changed  Reset  |
| Terrain                                                 |
|   Smooth                   [ S ]                        |
|                                                         |
|                                   Reset all to defaults |
```

- Rows are grouped by level, in the stack's order, and only what exists is listed. A changed row says "changed" and has
  its own Reset.
- Click a key: the button reads "Press a key". The next key with its modifiers is the new one, Esc cancels, and no shortcut
  fires meanwhile.
- **A key already in use is never silently taken.** "R is Scale in every level." with Replace and Cancel. Replace leaves the
  other action with no key, and that row says "no key". The same key in two different levels is no conflict, but a key
  used everywhere conflicts with all levels.
- Esc, Enter and F1 cannot be changed, so that a cancel and the way to the help always work. Digit keys and all others can.
- Any key the keyboard can produce is allowed: the QWERTZ rule of section 4 is for the defaults.

**The file.** Shortcuts are stored by a stable id (`tool.move`, `level.terrain`) and only when they differ from the default,
so a later version's new defaults still reach you. Ids the program does not know are kept, for a file shared between versions.
A damaged file is not overwritten: the program starts with the defaults and says what is wrong in the window. Precedence for
external tools: an environment variable wins over the settings, the settings over the search, and the window shows
which one applies.

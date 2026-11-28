<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# GUI

```sh
naves gui [<project folder>]
```

opens the project in the current folder (or the one you name) in a window titled *Náves – (project name)*.
There is no object editing yet; the window shows the project, reads the terrain and builds it.

![The editor: the Terrain level with the map, the project panel and the dock](images/editor.png)

*Level 1, Terrain. The chrome is Rerun's design system (`naves-ui`): the GUI is dark by default (light and the system theme are in Settings, Map), the font is Inter. The data is the Königssee crop of the test fixtures (`tests/fixtures`: Copernicus GLO-30); the map is the heightmap render.*

## The stack (right)

The right column is a stack of **levels**, drawn like a geologist's column: level 0, Project, at the bottom, and
what lives on the ground above it. Click a level, or press its number, to work on it. The active level is
the thick row in its own colour. A level appears when it can do something; today these are:

| Key | Level | The left panel shows |
|---|---|---|
| 0 | Project | the project settings (below) |
| 1 | Terrain | map size, vertex spacing, lowest and highest point, the elevation files |
| 2 | Ground | the share of the map for each ground class, the land cover and OpenStreetMap files |

Hover a row to see its key; **F1** lists all [keyboard shortcuts](shortcuts.md) of the level you are in, and **Ctrl+B** builds.

### Layers

A level can have **layers**: things drawn over the base map. They are in the stack, under the active level, as
checkboxes. Switching one on shows its key under it (the symbol as drawn on the map). Level 1 has **Too steep to
walk**: diagonal hatching over ground steeper than 60° (a provisional limit, the steepest slope of the Livonia
navigation mesh, so the layer may miss some ground that is too steep, but never marks walkable ground), and
**Terrain changes**: what the roads did to the terrain, cut in earth brown and fill in blue, stronger the more
(half opaque from 5 m, nothing below a quarter of a metre).

Level 4, **Roads and paths**, has **Roads** (at their true width, at least a pixel and a half, with a white casing so
that they read on every base map), **Paths** (dashed, brown) and **Drivable**, drawn along each road piece: green
for a piece vehicles can drive, amber for one that is steeper or more tilted than its family is aimed at but still drivable, red
and twice as wide with cross ticks for one that cannot be driven, grey for a track that only has to be walkable.
The road layers appear when the roads are generated, which takes a moment after the project opens (the panel says
"Placing roads"); they are not drawn before. The roads need the sources listed in `sources.osm`, `dayzmake` and
the DayZ Server install: without a tool the panel says which and has a button that opens the settings.
**Click a road** to read its model, surface, grade, cross fall, distance from the ground and whether it can be
driven, and where it was generated from. What is wrong with the roads is listed under Problems, each with its
position.

![Roads, paths and drivability over the hillshade](images/map-layers.png)

The levels you are not working on are drawn faded, at 40 %, so that you see what you work against. **H** hides the
active level, **Shift+H** shows the others in full; making a hidden level active shows it again. Layers and the
active level belong to the window and are not saved in `naves.toml`. Switching, fading and hiding draw nothing
anew.

A 3 px band in the active level's colour runs along the top of the map, and a line below the map says what
you can do. The numbers never change: key 4 is Roads and paths.

Above the stack are the **base maps**, one at a time: **Relief** (shaded, sun in the north-west), **Slope**
(green flat to red at 45° and steeper; the surface painter makes ground above 35° rock), **Height**, **Ground**
and **Imagery**.

- **Ground** shows the ground classes Náves will paint (grass, rock, forest, soil and tarmac, the paved ground of car parks), in the colours of the
  satellite image, with a legend under the row. It is decided from the land cover and OpenStreetMap files
  of the project, and from slope and altitude where they say nothing, exactly as the build does. The map takes
  a little longer to appear when the project lists such files.
- **Imagery** shows the satellite or aerial images of `sources.imagery`, merged as the build merges them, so
  you can check the result before a long build. Ground that no file covers is drawn in a checkered grey, so
  a gap cannot be mistaken for ground. Without imagery files the map says how to add them.

On a small window the bottom dock folds to its button row (below 560 px of height), then the left panel to
a thin strip with a button that opens it (below 900 px of width). The map stays.

## Project settings (level 0, left)

The fields of [`naves.toml`](project-file.md): name, centre, real extent, both scales and the two grid sizes. A
change is checked at once and saved to `naves.toml` when it is valid. A value that is out of range is **not**
saved; a red line under the form says which field and why, and the file stays as it was. A saved change
recomputes the map. The line below the form shows the size of the map in metres and the distance between
heightmap vertices.

The elevation, land cover and OpenStreetMap files are listed under the form. They are edited in `naves.toml` for now;
see [the project file](project-file.md#sources). Changing a list reloads the map.

## Map (centre)

A top-down view of the heightmap, north up.

- The **middle mouse button + drag** and **W A S D** move the map (a left drag belongs to the tools of the editor, and
  **Space** finishes a line). **Scroll** zooms in and out around the pointer.
- **Click** reads a point in the active level: in Terrain its position, latitude and longitude, height and
  slope, in Ground the ground class. The result is under the stack, and a ring marks the point. **Alt+click** picks in
  the highest level that has something there and switches to it. Levels you hid (H) pick nothing. **Esc** clears it.
- **F** centres the map on the selected point, **Shift+F** shows the whole map.

### Moving around

The left hand works the keyboard and the right hand stays on the mouse. Hold **W**, **A**, **S** or **D** to pan the
view north, west, south or east, and **Q** or **E** to zoom out or in; **Shift** makes it three times faster,
**Ctrl** three times slower. The wheel and the middle mouse button still work. **Esc** cancels what you are doing and
goes back to Select. The digits **1** to **6** go to levels 1 to 6; **Alt+1** to **Alt+4** go to levels 7, 8, 9 and 0
(the digits **7**, **8**, **9** and **0** work too). See [keyboard shortcuts](shortcuts.md) for the whole table.

![The map close up, with the slope hatching where the ground is steeper than 60°](images/map-slope.png)

The project panel has a section **Satellite imagery files**:

- **Available for this area** lists the sources of the [imagery catalogue](imagery.md) that have something for the
  area, best first, each with how much of the map it covers, its pixel size and its licence. A source that does not
  allow commercial use says **non-commercial** in words. **Fetch** downloads it into the project in the background:
  the log shows the steps, **Cancel** stops it and leaves no half-written file, and one fetch runs at a time. For a
  source of scenes you can type the dates (`YYYY-MM-DD`) of the search next to *Dates for scenes*; without them the
  last two summers are searched. The files appear in the list below and the Imagery map updates.
- The **files of the project**, in the order that decides among equally sharp images (the first is drawn on top),
  with **Up**, **Down** and **Remove**. Remove only takes a file off the list; it stays in the project folder, and the
  GUI says so. Under the list, one line per file tells what the merge found: how much of the map it covers, how much of
  that other images cover as well, and the colour correction applied (or that its colours are kept).
- **Imagery brightness**, in the settings above, sets `[imagery] brightness`; the Imagery map shows the result at once.
- **Add your own file** takes a path inside the project folder, a credit and a licence; the file is accepted only when
  all are there, and a licence with `NC` in it needs the box *The licence does not allow commercial use*.

The project panel lists the **satellite imagery files** with their licences. A file whose licence does not allow
commercial use says so in words, and a line **Non-commercial map** appears when such a file touches the map.
Notes of the merge (a file that overlaps no other image, so its colours are not matched) appear in the log.

If the project has no elevation files, or the files do not cover the area, the map says so instead of drawing.

## The road lines on the map

On the Roads level the roads as read from OpenStreetMap are drawn over the built roads, so that you can see where
the build moved a line:

- a thin grey line is a road as OpenStreetMap has it;
- a blue line is a road you changed or drew; a short tick across each end marks it, so that it is still told
  apart from the grey ones where the colours are hard to see;
- the selected road is a heavier blue line with a small square handle on each of its nodes;
- a dashed light line, with its handles, is the road you are drawing.

A click with the Select tool picks the node under the pointer, or else the road line, within 8 screen pixels (so it
is easier to hit when you zoom in). The properties name the road class and say where it comes from (see below), and
list the built piece under the click below *Built here*. A click on nothing picks nothing.

## The item selector

A level that can add things (houses, trees, road classes, bridges) shows one list of items in the left panel instead
of its settings. Every level uses the same list:

- **Search** (**Ctrl+F**) narrows the list by name or group, over the items of the chosen scope.
- Under the search a two-part switch picks the **scope**: the name of the level (for example *Buildings*) shows the
  items that fit the level; *Everything* shows all items, the ones that fit the level first. The choice is
  remembered for each level and kept between sessions; it is not in Settings.
- Each row has a thumbnail, the name, what the item is, and at the right edge its width or length in metres. All
  thumbnails share one scale (2 px per metre, cut off at the 64 px column), so a 12 m asphalt road is visibly
  twice as thick as a 5 m track.
- In *Everything*, an item that belongs to another level carries that level's colour chip with its number; the
  level name shows when you rest the pointer on it.
- *Last used* lists the items you picked last in this level.

Picking an item of another level does not change the level you are working on: you keep the level and use the
item's tool. **Esc** ends it and goes back to Select.

### The Roads list

The Roads level always shows the list, also in a project without OpenStreetMap files; under it come the state of the
roads (placing, missing tools, errors, the numbers of the last build) or, without an OpenStreetMap file, the hint
*No OpenStreetMap file: draw the roads yourself, or add one to sources.osm.*

- **Road classes** come first: *Roads* (Primary, Secondary, Tertiary, Residential street, Service road, Track grade 1
  to 5) and *Paths* (Footpath, Path). The metres at the right are the width the generator gives the class. Picking a
  class switches to **Draw**.
- When the game's road pieces have been read (this needs dayzmake and DayZ Tools, see
  [External tools](settings.md)), the list also has the groups *Pieces* (one group for each road family, for example *Pieces, asphalt*), *Junctions* and *Bridges* with one row per
  model of the game; the thumbnail is the model's plan view at the shared scale. Without the tools these groups are
  left out and the panel says which tool is missing.
- Picking a model shows its card in the properties: name, size, the levels it fits, and *Placing models comes in a
  later version.* Models cannot be placed yet.

## Editing road lines

The Roads level has five tools in the rail, and **X** (or **Delete**) for the selection. Every gesture is one
saved edit that **Ctrl+Z** takes back in one step. A click that finds nothing to work on changes nothing. **Esc**
drops what a tool holds (a line, a drag, a first click) and, with nothing held, goes back to Select.

| Tool | Key | What it does |
|---|---|---|
| Move | **G** | Press on a node, drag, release: the node moves. **Esc** during the drag leaves it where it was. |
| Draw | **C** | Each click adds a point; a click on a node of a road joins that node. **Space**, **Enter**, a right click or a double click ends the line (it needs two points). **Ctrl+Z** or **Backspace** takes the last point back and saves nothing. **Esc** discards the line. The line gets the class you picked in the list, *Residential street* if you picked none. |
| Cut | **R** | Click a node inside a road: the road ends there and a new road with the same tags starts there. |
| Join | **T** | Click the end of one road, then the end of another. With the same tags they become one road; with other tags the end of the second road moves to the end of the first, and both stay. |
| Bridge | **Shift+T** | Click two nodes of a road: the part between them becomes a bridge (`bridge=yes`, `layer=1`). A click on a road that is a bridge makes it a plain road again. |
| Delete | **X**, **Delete** | Deletes the selected road, or takes the selected node out of a road you drew. A node from OpenStreetMap is only moved, never deleted. |

![The Roads level: the item list in Everything scope, the rail, a road drawn by you selected, and its properties](images/roads-editing.png)

The status line names the tool and the key it needs. When you picked a road class while another level is active, the
Roads tools join the rail until **Esc**, and a line you draw shows on that level too.

## The properties of a road

Select a road or one of its nodes and the properties on the right show the road and let you change it. Each change
is one saved edit, taken back with **Ctrl+Z**.

| Property | What it does |
|---|---|
| Class | The kind of road. Changing it replaces the `highway` and `tracktype` tags and keeps the others (surface, name, ...). A road of another kind shows *Other: highway=...*. |
| Surface | asphalt, paved, gravel, compacted, dirt, grass, or *—* to remove the tag. |
| Bridge | Sets `bridge=yes` (and `layer=1` when the road has no layer); unticking removes `bridge`. |
| Layer | -5 to 5, the `layer` tag; 0 removes it. |
| Keep as drawn | `naves:keep`. Unticked, it is written as `naves:keep=no`. |
| Let road planning route it | `naves:plan=yes`; unticked, the tag is removed. |

The last line says where the road comes from: *OSM way 8421, unchanged*, *OSM way 8421, edited by you* or *Drawn by
you*. For an edited OpenStreetMap road, **Reset to OSM** removes the edit and the road is as OpenStreetMap has it;
**Ctrl+Z** brings the edit back.

## Edits, undo and redo

Edits to the road lines are written to `edits/ways.toml` at once; there is no Save button. If the file cannot be
written, the edit is not made and the log says why. **Ctrl+Z** takes back the last edit and **Ctrl+Shift+Z** does it
again; the last 100 edits can be undone while the project is open. After an edit the status line reads *Edited: press
Ctrl+R to rebuild.* **Ctrl+R** (or **F5**) builds the roads again from the saved edits. A project without
OpenStreetMap files shows the roads of its edits.

## Settings

The **Settings** button at the right end of the dock (or **F2**) opens the [settings](settings.md): keyboard shortcuts, the map and the paths of the external tools. They apply to every project.

## The dock (bottom)

The dock has the tabs **Build** and **Problems**. On a low window it folds to its tab row.

**Build** runs the same build as [`naves build`](cli.md) in the background (also with **Ctrl+B**). The log shows
the steps as they happen; when the build succeeds the last lines give the mod and mission folders, and when it
fails they give the reason (a missing tool is named together with the variable to set, see
[Installation](installation.md)). The button is disabled while a build is running or while the settings hold an
invalid value.

**Problems** lists what is wrong: the warnings and the error of the last build, the notes of the imagery merge,
and a setting that was not saved. They stay in the build log too. The tab says how many there are ("Problems 3");
with none it says "No problems". The list follows the level you are in, and the box **All** shows every level.
A row that has a position takes the map there when you click it. **F8** goes to the next problem and
**Shift+F8** to the previous one, wrapping around. The row *Problems* above the stack switches the red
diamonds on the map on and off.

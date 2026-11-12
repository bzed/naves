<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Roads that run closer together than the terrain grid can tell apart (hairpin legs, streets side by side, junction arms) are brought to one height before the terrain is shaped, the less important road moving more and none more than 5 m. On the Berchtesgaden example the pieces that miss the terrain fall from 5001 to about 3800 and the road that cannot be driven from 37 km to 28 km. This replaces the earlier switchback pass.
- Level 4, Roads and paths, in the GUI: the layers **Roads** (true width, white casing), **Paths** (dashed) and **Drivable** (green, amber, red with cross ticks, grey), and **Terrain changes** under Terrain (cut and fill); a panel with the numbers of the network; click a road for its model, surface, grade and where it was generated from; what is wrong with the roads in Problems, with positions. The roads are generated in the background when a project with OpenStreetMap files opens; the road layers wait for them. A missing tool is named in the panel, with a button that opens the settings.
- MCP: `render_topdown` takes `layers` (`terrain_changes`, `too_steep_to_walk`, `roads`, `paths`, `drivable`), and the new tools `roads_stats` and `validation_list` (optionally for one rule) report the generated roads.
- `naves build` builds roads and paths from the OpenStreetMap files in `sources.osm`: `highway` ways become the game's own road pieces (asphalt, streets, gravel and dirt, junctions, end caps, bridges) laid along the OSM line, with the terrain shaped to fit them (benches on valley sides, embankments, cuttings; held to a slope limit, reaching up to 60 m), the road beds and footpaths painted on the ground, and the objects written to the world file. The pieces are read from the DayZ Server install (`addons/structures_roads.pbo`) through armake2 and cached per user; no model is copied into the mod. The result is checked on the final terrain, and findings (a road that cannot be driven, a piece in the air, a path too steep to walk, a road in a lake, roads that meet without a junction, tunnels left out) go into the build warnings and `build/report.json`. **The drivable and walkable limits are provisional**, pending measurement in the game. Edits go in `edits/overrides.toml` and survive a rebuild. A navigation mesh made earlier no longer fits the new terrain; the build warns. New manual section "Roads".
- A settings window in the GUI (the Settings button in the dock, or F2) with Keyboard, Map and External tools. Every shortcut can be changed; a key already in use asks before it is replaced; Esc, Enter and F1 cannot be changed and Tab and Shift+Tab cannot be assigned. The Map tab sets the wheel direction, how strongly the other levels fade, the interface scale, whether the stack unfolds with animation, and the base map and layers a project opens with. External tools shows each path and where it comes from. Everything is saved at once in the per-user `settings.toml`; a damaged file is shown and never overwritten. New manual page "Settings".
- Per-user settings in `settings.toml` in Náves' configuration folder (`$XDG_CONFIG_HOME/naves`): the `[tools]` table names the paths of armake2, Steam, DayZ Tools, DayZ, DayZ Server and Proton. An environment variable still wins; the settings come before the search. `naves build`, `naves mcp` and the GUI use them, a path that does not exist is named, and a damaged file is a warning and is left alone. New: `DAYZ_SERVER` is documented.
- The GUI's bottom dock has the tabs Build and Problems. Problems lists the warnings and the error of the last build, the notes of the imagery merge and a setting that was not saved; a row with a position (and F8 and Shift+F8) takes the map there, and diamonds mark them on the map.
- Click in the GUI map to read a point in the active level (position, latitude and longitude, height and slope in Terrain; the ground class in Ground), Alt+click to pick across levels, Esc to clear, F to frame it and Shift+F to frame the whole map.
- Switchable map layers in the GUI, under the active level in the stack, starting with **Too steep to walk** (hatching over ground steeper than 60°, a provisional limit). H hides the active level, Shift+H shows the other levels in full; they are drawn at 40 % otherwise. New `naves_core::render::render_layer`.
- Keyboard shortcuts in the GUI, from one table: the digits 0 to 2 go to a level, Ctrl+B builds, F1 lists the keys that work in the level you are in, Esc closes the list. While a text field has the keyboard, no single key fires. The table already holds the final set of keys of the editor; a key whose action is not built yet does nothing. New manual page "Keyboard shortcuts".
- The GUI is built around the stack of levels: Project, Terrain and Ground (keys 0 to 2), each with its own left panel, a band in the level's colour on top of the map and a status line below it. The base maps (Relief, Slope, Height, Ground, Imagery) moved to the right column; "Hillshade" is called Relief and "Surface" Ground there. The panels fold on small windows.
- The GUI sets the imagery brightness (`[imagery] brightness`), and the Imagery map (GUI and MCP) shows the imagery with it, as the build makes it.
- The GUI lists the imagery sources for the area of the project and fetches them in the background (with progress and cancel), shows the files in their order with up, down and remove, accepts files of your own once they have a credit and a licence, and shows what the merge found out about each file.
- `naves imagery fetch <id>` (and the MCP tool `imagery_fetch`) downloads imagery from a catalogue source for the area of a project into its `imagery` folder and lists the files with their licence in `naves.toml`: mosaics from a WMS in tiles, Sentinel-2 scenes from a STAC search (the clearest ones of the last two summers, or of the dates given), cropped to the map. `naves build` still never uses the network. `make ca-bundle` (CI weekly and at every release) checks that the bundled CA list is the newest.
- `naves imagery list` (and the MCP tool `imagery_options`) shows which imagery sources exist for the area of a project, with pixel size, licence and a NON-COMMERCIAL mark; a user file `imagery-sources.toml` in the configuration folder can add sources. New manual chapter "Imagery".
- An **Imagery** overlay in the GUI map and in the MCP `render_topdown` tool shows the merged imagery (ground without imagery is checkered). The GUI lists the imagery files with their licences, says "Non-commercial map" when a file that touches the map is non-commercial, and shows the merge's notes in the log.
- `naves build` paints the satellite images of the terrain from the GeoTIFF files in `sources.imagery`: files that overlap are matched in colour and merged by sharpness with soft edges, and `[imagery] brightness` sets the brightness. A map that uses a non-commercial file says so in `ATTRIBUTION.txt` (mod and mission), in `mod.cpp` and in the build warnings; the credits of the files used are written to `ATTRIBUTION.txt`, and the mod folder now holds a `mod.cpp` with the name of the map.
- Project file: `sources.imagery` lists satellite or aerial GeoTIFFs with their attribution and licence (`non_commercial` is required where the licence says NC), and `[imagery] brightness` sets how bright they are drawn.
- `naves build` paints the ground from ESA WorldCover land cover and OpenStreetMap water, rock and built-up areas (`sources.land_cover`, `sources.osm`); the slope and altitude rules apply only where neither says anything. The build warns when the files do not cover the map or an extract cuts polygons, and writes `ATTRIBUTION.txt` (Copernicus, WorldCover, OpenStreetMap as used) into the mod folder.
- The GUI shows the build's warnings in its log. OpenStreetMap extracts are merged before areas are built, so a polygon that every extract cuts at its border is whole, and only rings on the map are counted in the skipped-rings warning.
- A **Surface** overlay in the GUI map (with a legend) and in the MCP `render_topdown` tool shows the ground classes; the GUI lists the land cover and OpenStreetMap files.
- The terrain has four ground layers (grass, rock, forest, soil); the ground of each map cell is decided in one place (`SurfaceGrid`).
- Project file `naves.toml`.
- Copernicus GLO-30 GeoTIFF DEM input.
- MCP server `naves mcp`: `project_get`, `project_set`, `heightmap_stats`, `render_topdown` and `build` for AI clients over stdio.
- GUI (`naves gui`): project settings that save to `naves.toml`, a pannable top-down map (hillshade, slope, height) and a build button with a live log.
- Project file: `sources.land_cover` (ESA WorldCover) and `sources.osm` (OpenStreetMap extracts) list the data that will paint the ground surfaces.
- `naves new` creates a project, `naves build` builds the signed mod and the server mission.
- Binarize worlds with DayZ Tools under Proton (sandboxed, with a timeout); the build checks for armake2,
  DayZ Tools and Proton up front and names the variable to set.
- World `config.cpp` and server mission generation (the player spawns in the middle of the map). The world
  inherits Livonia's sky, weather and terrain materials by reference and packs a `ce/` folder from the installed
  server mission; the game client needs both.
- Navigation mesh support: the world config has a `Navmesh` class (inherited agent and link types, seed in the map
  centre) and `naves build` packs `<project>/navmesh/<world>.nm` when it exists; the manual describes how to make
  the mesh with DayZDiag and Flare Navgen.
- Procedural surfaces: a hillshaded satellite image, a grass/rock mask (rock above 35° slope and above a real
  altitude of 2100 m) and a tile material for every terrain tile.

### Changed

- Road alignment after the fixes, on the Berchtesgaden example: the road that cannot be driven falls from 27.5 to 5.7 km at 1024 (21.4 to 4.8 km at 2048), road conflicts from 2412 to 1979 (2166 to 1729), and earthworks that stop at the 60 m cap from 1904 to 728 vertices (3459 to 1322). The numbers and what is left are in `docs/research/2026-11-10-road-alignment.md`, section 5.
- Earthworks follow the hillside: a cutting is 45° and an embankment 34° on gentle ground and both get steeper with the natural slope, up to 63° and 45°. A cutting in a steep hillside can be too steep to walk up; the downhill side can always be left on foot. On the Berchtesgaden example the earthworks that stop at the 60 m cap fall from 1904 vertices to 728.
- A road gives up its grade aim before it leaves the ground: where holding the aim would put it more than 4 m above or below the terrain, it is solved again with a steeper grade, up to nine tenths of what vehicles manage. Such pieces are amber (**Steep**), not not drivable.
- A junction piece is no longer level along its length: it carries up to 6 % like any road near a meeting. A road is at most 6 % steep where another road meets it, and gentler in a tight bend so that its inner edge stays within the family's aim.
- A road's profile is limited by a vertical radius per family (600 m on `asf1` down to 60 m on a dirt track) instead of one change of grade per joint, so a short piece no longer bends more per metre than a long one; the grade excesses are measured on the joints after settling.
- The road limits are in one place (`naves_core::limits`): the steepest grade a car climbs is 30 % on asphalt and streets (was 35 %); gravel and dirt tracks keep 30 % and 28 %, because some of them are used by a tractor or a tougher vehicle rather than a car, and keep their aims of 18 % and 25 %; and a road may be tilted across by up to 20 %, with 8 % as the aim (was one hard limit of 5 %). A road between its family's aim and those limits is **Steep** (amber) instead of not drivable. The limits are estimates from real vehicles, not measured in the game yet.
- The GUI's look: boxes, radio buttons and fields have an outline that shows on the panel, selected rows are a light green that ink reads on, tabs are marked by a bar instead of a filled button, and keyboard focus is a green ring (white or ink on a coloured level row). The shortcut sheet (F1) is centred and has no shadow.
- The GUI map moves with Space + drag or the middle mouse button; the left button no longer pans.
- `naves.toml` with an option that Náves does not know (a typo, or an option of a newer Náves) is rejected with the option named, in the CLI, the GUI and over MCP, instead of being read and dropped when the file is saved again.
- Lighter baked shading on the satellite images: rock faces were nearly black.

### Fixed

- The ground under a bridge keeps its height: the road on either side now ends flat at the abutment and its earthworks no longer spread under the span (a 25 m bridge used to have the whole gap filled to deck level).
- A bridge deck is held to the ground at its abutments, not to the valley floor under it, so a deck over a gorge stays at bank height instead of pulling the approaches down into cuttings.
- `edits/overrides.toml` with `nan` or `inf`, or with an empty or non-ASCII model name, is refused when it is read, with a message that names the object and the field (it used to reach the world file as a NaN transform, or fail late in the world writer).
- Hairpin legs keep their heights when a step fits between them: roads closer than their half widths plus three cells still share a height, but from there on they may differ by the step one cell can hold (up to 45°), so a hairpin on a steep flank is no longer pulled together.
- A road that ends on another now lies on the other road's ground for as far as that ground reaches (up to eight variables inward, within half a width and 1.5 cells of its centre line), instead of leaving it after the first joint. This removes the step where the side road's surface left the through road's.
- A bend is measured along the bend, not along its chord: the grade and the cross fall of an arc are read along the arc and at right angles to it, so a tight arc on a grade no longer reports part of its grade as cross fall.
- A road that ends near another road no longer takes its height when it does not lie on it: a chain end now only meets a road whose surface it is on (within its half width, or on a junction piece, whose side ports reach beyond the through road's edge), and it takes that road's height where they meet, interpolated between the joints - not the height of whatever joint happens to be within 10 m. This removes steps of up to 22 m where unrelated roads passed close by, and makes side roads level where they join, between the joints of the road they meet. On the Berchtesgaden example the pieces that miss the terrain fall from about 3800 to 3786 and the road that cannot be driven from 27.9 km to 27.5 km.
- Bringing close roads to one height no longer breaks their grade limits at a free end: the move an end is asked for sticks only as far as the limit allows, and the heights stay near the first solve (the smoothing of the re-solve rings at a free end and could drift a road 60 m off the terrain over the rounds).
- The reported grade violations (`road_grade`) are measured on the heights that are actually used, after close roads have been brought together - not on the heights the chains had before. The count on the Berchtesgaden example is now about 500 instead of the 121 snapshot; most of them are hairpins and other places where roads that share ground cannot both keep their limits.
- A small real change in one of several overlapping images (a field, snow on a slope) no longer leaves a halo in the merged satellite image: cells are also compared with their neighbourhood.
- `naves imagery fetch` lists fetched files with a `/` in `naves.toml` on every system, so a project written on Windows builds on Linux.
- A missing imagery file is reported as missing (not as a decoding problem with a JPEG hint), with the `naves imagery fetch` command to get it again when a fetch wrote it; imagery of a large map is fetched coarser instead of failing above 16 000 pixels.
- Imagery of very different brightness (aerial photographs next to a satellite scene) is matched in brightness: the gain limits were too narrow, the change threshold could not recognise darker cells, and a gain per colour channel tinted rock green; the merge now uses one gain for all channels and keeps the colours of the sharper image.
- `naves imagery fetch` names the clearest scene and suggests other dates when every scene of the default dates is cloudy, and writes the files with the usual mode instead of a private one.
- Terrain images with real detail (photographs) no longer make `binarize` stop: `img2paa` of armake2 marked mipmaps as LZO that it had stored uncompressed, and Náves now clears the mark (armake2 itself is fixed too).
- The game client no longer crashes a few seconds after entering a generated world. The world now uses
  Livonia's grid geometry: `grid.land_size` must be exactly `grid.terrain_size / 8` (default 128; other values
  are rejected with the right value named) and the surface tiles are 8 × 8 land cells at 60 pixels per cell. The
  GUI and `project_set` derive the land grid from the terrain grid. Existing projects need `land_size` changed and
  their navigation mesh regenerated.
- The terrain no longer looks dark red in the game: the satellite and mask textures get the `AVGC`/`MAXC` (and,
  on masks, `FLAG`) tags that armake2's `img2paa` leaves out.
- The terrain shows its satellite images instead of a flat colour per tile with straight edges: the build removes
  the byte armake2's `img2paa` appends to LZO-compressed mipmaps, which the game cannot read.
- The no-data value of a GeoTIFF (for example -9999) counts as a gap in the elevation data, not as a height.
- Saving `naves.toml` keeps the file's permissions and syncs it to disk before it replaces the old file.
- A failed build keeps the mod and mission of the last good build; a new build replaces them only when it is finished.
- Relative paths in `ARMAKE2`, `PROTON` and similar variables work. External tools no longer inherit stdin, which is
  the protocol stream under `naves mcp`. On Windows, Proton is not required and Steam is found in its program folder.
- The generated mission has no gas zones at Livonia's positions. Still from Livonia: `areaflags.map` and the wildlife
  territories.
- `naves mcp`: `project_set` and `build` run one at a time, and the heightmap follows edits of `naves.toml` from
  outside. The GUI runs one heightmap job at a time and does not reload the elevation data when the name changes.
- The sun is in the right part of the sky: the world config writes the latitude with the game's sign (positive to
  the south).
- Surface masks select the right layers: grass is black and rock red at full alpha. Before, grass showed the rock
  texture and rock showed nothing.

<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Fixed

- Starting an external tool that was written or replaced moments earlier no longer fails with "text file busy": the kernel releases a closed file's write permission a moment after close, and the spawn now waits that moment out (up to one second).

### Changed

- Build: vendor Rerun's design system (from rerun_ui_kit's re_ui 0.38.1 extraction, MIT OR Apache-2.0) as the workspace crate naves-ui; not yet used by the GUI.
- Build: the Rust toolchain pin moved from 1.95 to 1.99 (required by the vendored design system).

### Added

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

- The GUI's look: boxes, radio buttons and fields have an outline that shows on the panel, selected rows are a light green that ink reads on, tabs are marked by a bar instead of a filled button, and keyboard focus is a green ring (white or ink on a coloured level row). The shortcut sheet (F1) is centred and has no shadow.
- The GUI map moves with Space + drag or the middle mouse button; the left button no longer pans.
- `naves.toml` with an option that Náves does not know (a typo, or an option of a newer Náves) is rejected with the option named, in the CLI, the GUI and over MCP, instead of being read and dropped when the file is saved again.
- Lighter baked shading on the satellite images: rock faces were nearly black.

### Fixed

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

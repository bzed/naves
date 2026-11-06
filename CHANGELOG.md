<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Project file `naves.toml`.
- Copernicus GLO-30 GeoTIFF DEM input.
- MCP server `naves mcp`: `project_get`, `project_set`, `heightmap_stats`, `render_topdown` and `build` for AI clients over stdio.
- GUI (`naves gui`): project settings that save to `naves.toml`, a pannable top-down map (hillshade, slope, height) and a build button with a live log.
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

### Fixed

- The game client no longer crashes a few seconds after entering a generated world. The world now uses
  Livonia's grid geometry: `grid.land_size` must be exactly `grid.terrain_size / 8` (default 128; other values
  are rejected with the right value named) and the surface tiles are 8 × 8 land cells at 60 pixels per cell. The
  GUI and `project_set` derive the land grid from the terrain grid. Existing projects need `land_size` changed and
  their navigation mesh regenerated.
- The terrain no longer looks dark red in the game: the satellite and mask textures get the `AVGC`/`MAXC` (and,
  on masks, `FLAG`) tags that armake2's `img2paa` leaves out.
- Surface masks select the right layers: grass is black and rock red at full alpha. Before, grass showed the rock
  texture and rock showed nothing.

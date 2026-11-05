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
- `naves new` creates a project, `naves build` builds the signed mod and the server mission.
- Binarize worlds with DayZ Tools under Proton (sandboxed, with a timeout); the build checks for armake2,
  DayZ Tools and Proton up front and names the variable to set.
- World `config.cpp` and server mission generation (the player spawns in the middle of the map).
- Procedural surfaces: a hillshaded satellite image, a grass/rock mask (rock above 35° slope) and a tile
  material for every terrain tile. `grid.land_size` must now be 32 to 4096.

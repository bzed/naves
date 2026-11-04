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
- Procedural surfaces: a hillshaded satellite image, a grass/rock mask (rock above 35° slope) and a tile
  material for every terrain tile. `grid.land_size` must now be 32 to 4096.

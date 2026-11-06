<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Getting started

You need Náves and the external tools from [Installation](installation.md), and a Copernicus GLO-30 elevation
file that covers your area.

## 1. Create a project

```sh
naves new berchtesgaden --name Berchtesgaden --center 47.585,12.965 --extent 12800
```

This makes `berchtesgaden/naves.toml`. The area is a 12.8 km square around the centre; the default scale of 0.5
turns it into a 6.4 km map.

## 2. Add elevation data

Download the Copernicus DEM tiles that cover the area (GeoTIFF, 30 m) into `berchtesgaden/dem/` and list them in
`naves.toml`:

```toml
[sources]
dem = ["dem/Copernicus_DSM_COG_10_N47_00_E012_00_DEM.tif",
       "dem/Copernicus_DSM_COG_10_N47_00_E013_00_DEM.tif"]
```

An area can span several tiles; Náves joins them without a seam. If a spot is not covered, the build stops and
names it.

## 3. Build

```sh
naves build berchtesgaden
```

The first build in a new project creates a signing key in `berchtesgaden/secrets/`. When it is done, the last
lines name the two folders you need:

```text
Mod:     berchtesgaden/build/@naves_berchtesgaden
Mission: berchtesgaden/build/missions/dayzOffline.berchtesgaden
```

## 4. Load it on a server

1. Copy (or link) the mod folder next to your DayZ server and put its `keys/*.bikey` into the server's `keys/`.
2. Copy (or link) the mission folder into the server's `mpmissions/` and set
   `template="dayzOffline.berchtesgaden";` in `serverDZ.cfg`.
3. Start the server with `-mod=@naves_berchtesgaden`, and use a port other than the default if another server
   or a game is running.

The server log shows `Loading landscape 'berchtesgaden'` when the map loads. New characters spawn in the middle
of the map. The first version has procedural surfaces only (grass, and rock on steep slopes and above 2100 m real
altitude), no roads, houses or water. Infected and animals only move once the project has a
[navigation mesh](navmesh.md).

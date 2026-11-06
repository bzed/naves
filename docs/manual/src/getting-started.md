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
of the map. Without land cover or OpenStreetMap data the ground is procedural (grass, and rock on steep slopes and
above 2100 m real altitude); see [Ground from real data](#ground-from-real-data). There are no roads, houses,
trees or water planes yet (a lake is only soil-coloured ground). Infected and animals only move once the project
has a [navigation mesh](navmesh.md).

## Ground from real data

Add ESA WorldCover tiles and OpenStreetMap extracts to the `[sources]` table
(see [the project file](project-file.md#sources)) and `naves build` paints the ground from them. For every cell
of the map the first answer wins:

1. an OpenStreetMap area: water and reservoirs (soil), bare rock, scree, shingle and glaciers (rock), and
   residential, industrial, commercial, farmyard and military land (soil); where areas overlap, water beats rock
   beats built-up land;
2. the WorldCover class: tree cover is forest, grassland, shrubland and wetland are grass, cropland, built-up
   land and water are soil, and bare ground, snow and moss are rock;
3. the slope and altitude rules of the first version.

Real data is never overridden by the rules: grassland on a steep slope stays grass. If the files do not cover
the whole map, the build says how much of it the rules decided and does not fail. Náves reads only the part of
a WorldCover file that the map needs. For OpenStreetMap it reads the whole extract, three times, and keeps the
water, rock and built-up areas of all of it in memory until the map is done: a regional extract needs several
hundred megabytes, a country-sized one several gigabytes. Prefer the smallest extract that covers the map.
Several extracts are merged before the areas are built, so an area that each of them cuts at its border is still
whole.

A map made from OpenStreetMap contains OSM-derived data under the ODbL: if you publish it, keep the attribution
that `naves build` writes to `ATTRIBUTION.txt` in the mod folder, and share the derived database alike. The same
file carries the WorldCover (CC BY 4.0) and Copernicus credits. It lists only the sources the build used. See
[Credits and data licences](credits.md).

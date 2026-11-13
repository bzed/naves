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
above 2100 m real altitude); see [Ground from real data](#ground-from-real-data). Roads and paths come from OpenStreetMap, see [Roads](#roads); there are no houses,
trees or water planes yet (a lake is only soil-coloured ground). Infected and animals only move once the project
has a [navigation mesh](navmesh.md).

## Satellite images from real imagery

By default the satellite image that the game shows from a distance is painted from the ground classes. List
GeoTIFF files in `[[sources.imagery]]` (see [the project file](project-file.md#sourcesimagery)) and `naves build`
paints the satellite images from them instead, without baked shading, because the image holds its own shadows. Where
no file covers the map, the class colours are used, and the build says how much of the map that is.

Several files can overlap. Náves measures their colours against each other where they overlap, evens out
differences in exposure, draws the sharpest image where several cover a spot, and fades one image into the next
over a few land cells. The image with the most coverage keeps its colours; a Sentinel-2 image of the whole map
under aerial photos needs no setting. The colours are made 30 % brighter by default, because satellite images are dark next to the game's own terrain; change `[imagery] brightness` to taste.

Imagery comes with a licence. Náves writes the credits of every file that touches the map into `ATTRIBUTION.txt`
(in the mod and in the mission) and a short note into `mod.cpp`. If a file is marked `non_commercial = true`, the
attribution opens with **NON-COMMERCIAL USE ONLY**, the launcher shows it in the mod's overview and the build
warns. A file that does not touch the map gets neither notice nor credit. This is information, not legal advice:
whether you may publish the map is your responsibility.

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

## Roads

List OpenStreetMap extracts in `sources.osm` and `naves build` also builds the **roads**: the `highway` ways of the
map become the game's own road pieces (asphalt, streets, gravel and dirt tracks, with junctions, end caps and
bridges), laid along the OSM line, and the terrain is shaped to fit them. The pieces are read from the game: the
DayZ Server install needs its `addons/structures_roads.pbo`, and `armake2` unpacks it once (the result is cached
per user and rebuilt when the game is updated). No model of the game is ever copied into your mod.

What is built, from the `highway` tag:

| OpenStreetMap | Result |
|---|---|
| `motorway`, `trunk`, `primary` | asphalt, 12 m |
| `secondary` | asphalt, 9 m |
| `tertiary`, `unclassified` | narrow asphalt, 7 m |
| `residential`, `living_street`, `service` | street, 10 m |
| `track` with `tracktype` `grade1` or `grade2`, or a hard `surface` | gravel, 6 m |
| `track` otherwise | dirt track, 5 m |
| `track` with `tracktype` `grade4`, `grade5` or a very bad `smoothness` | dirt track that must only be walkable |
| `path`, `footway`, `cycleway`, `steps`, `sac_scale=hiking` | only painted on the ground, 1.6 m |
| `sac_scale=alpine_hiking` or harder, `trail_visibility=bad` | nothing |

`access` and `motor_vehicle` tags do not matter: a forestry road in a national park is still physically a road.

The terrain is **held to a slope limit around every road**: a road on a valley side gets a bench, a road across a
valley an embankment, a road through a ridge a cutting. On gentle ground a cutting is 45° and an embankment 34°; on a
steeper hillside both get steeper with the natural slope, up to 63° and 45°. A cutting in a steep hillside can
therefore be too steep to walk up, while the downhill side can always be left on foot. The earthworks reach only as
far as they are connected to the road (up to 60 m); a cliff beyond stays a cliff. The result is checked on the final terrain: a road that vehicles
cannot drive, a piece that hangs in the air or is buried, a path that is too steep to walk, a road in a lake. Every
finding is a line in the build warnings and an entry in `build/report.json`, with the numbers of the network.

**Where roads meet, they meet at one height.** Every junction (a junction piece, a crossing without a junction
piece, a road that ends on another) gets a single height: the median of the original ground within 10 m of it. Every
road that reaches the junction is held to that height, the through road at the junction piece as much as the side
road at its end, so a road no longer inherits the height error of the road it meets. Where two junctions are too close
together for the difference of their ground heights (the steepest grade the road between them may have is nine tenths
of what vehicles manage), they are moved toward each other by just enough, the one on the more important road less, but no junction moves more than 3 m from the ground. A road is held at one joint where it passes a junction, at its end where it ends there, and a junction piece is level along its length; roads that merely pass near a junction (a parallel street, a road on a bridge above) are not held to it. The road between two junctions is measured from the joint that holds one height to the joint that holds the other. What cannot be repaired within 3 m is the `junction_too_steep` finding.

A road is at most **6 % steep where another road meets it** (asphalt and streets; gravel and dirt tracks keep their own
limit, so a steep track is not lifted out of the ground), and gentler in a tight bend: the inner edge of a bend is
steeper than its centre line, so the bend's grade is lowered to keep that edge within the family's aim.

A road keeps to its family's grade where it can. Where holding it would put the road more than **4 m above or below
the ground**, the road gives way and becomes steeper, up to nine tenths of what vehicles manage; it is then amber in
the Drivable layer, not red.

The terrain grid is coarse (a cell is 6.25 m on a 6.4 km map), and every cell under a road takes the road's height
for a margin of one and a half cells beyond its edge. **Roads closer together than that** (the two legs of a hairpin,
streets side by side, the arms of a junction) cannot differ in height, so the build brings them to one height before
it shapes the terrain, moving the less important road more and no road more than 5 m from the terrain. Where the
ground between two roads is too steep for that, the terrain under one of them is still wrong; those places are the
`road_misses_terrain` and `road_not_drivable` findings that remain.

**What is not built:** tunnels (they need terrain holes; the road ends in end caps at the portals and the build
says so), junctions at an angle other than 90° (the game has only right-angle junctions; the side road bends to the OSM
line after the junction), railways, houses and trees. Where the game has no junction for a pair of road types the
roads simply meet; the build lists those places. On a bridge, the span has a level deck and the ground below keeps
its height; the ground starts to be shaped only at the abutments. The ground under the pieces that were placed
(not under the OSM line, which they may leave by some metres) is painted as soil, 2 m wider than the road.

Your own changes to what was generated are kept in `edits/overrides.toml`
([the project file](project-file.md#editsoverridestoml)).

**The terrain changes where there are roads, so a [navigation mesh](navmesh.md) made earlier no longer fits.**
`naves build` warns about that when the project has one.

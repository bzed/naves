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
DayZ Server install needs its `addons/structures_roads.pbo` (the road pieces, the bridges and the concrete
panels); `sakhal/addons/structures_sakhal.pbo` (the bridges `bridge_wood2` and `bridge_concrete3`) and
`addons/structures_industrial.pbo` (the concrete pipe) are read too, and a build without them says so and goes on
without those parts. Folder names are found in any case (`Addons` on a client install). `dayzmake` lists each
archive and takes out only the models it needs, once (the result is cached per user and rebuilt when one of the
archives changes, appears or goes). No model of the game is ever copied into your mod.

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
therefore be too steep to walk up, while the downhill side can always be left on foot. **On steep ground the uphill
side of a road may be a rock face:** where the original ground rises more than 35 % across the road (measured 10 m to
each side of the road, as the median over the pieces within 25 m), the cutting is as steep as the terrain grid allows,
one cell wide, and nobody can walk up it. A player can then no longer always walk off a road on its uphill side. The
ground under a junction without a junction piece lies on the junction's plane (tilted along its through road, see below), in a circle of half the width
of the widest road plus one cell. **Roads close together keep their own ground:** where two roads, or two legs of one
road, at different heights come closer than their level strips, each keeps the terrain inside its own width and the
height difference becomes a face one cell wide between them; the vertices just outside a road's width that both need
go to the road whose edge is nearer, a tie to the road of the higher rank. The ground under a junction patch never
takes a road's own width. Where roads at different heights are less than about three cells apart, more pieces miss the
terrain slightly than before, and both roads can miss; a finer terrain grid lowers this. More footpaths beside such a
rock face are reported as too steep to walk (`path_too_steep`). Only where the widths of two roads at different
heights overlap is it a conflict (`road_conflict`). The earthworks reach only as far as they are connected to the road
(up to 60 m); a cliff beyond stays a cliff. The result is checked on the final terrain: a road that vehicles cannot
drive, a piece that hangs in the air or is buried, a path that is too steep to walk, a road in a lake. Every finding
is a line in the build warnings and an entry in `build/report.json`, with the numbers of the network.

**The heights of all roads are solved as one problem.** Every height of every road, and the height and tilt of every
junction, are found together, not road by road: the build looks for the heights that keep the roads close to the
original ground and smooth, and keep every rule below. A rule that cannot be kept everywhere is broken as little as
possible, and where it is broken it is a finding (see [troubleshooting](troubleshooting.md)). The result does not
depend on the order of the roads, and a build gives the same heights every time.

**Where roads meet, they meet at one plane.** Every junction (a junction piece, a crossing without a junction
piece, a road that ends on another) has one plane: its height at its point starts as the median of the original
ground within 10 m of it, and it tilts along the road that runs through it (below). Every road that reaches the
junction is held to that plane, the through road at the junction piece as much as the side road at its end, so a road
no longer inherits the height error of the road it meets. A junction is pulled to the ground, not fixed to it: where
two junctions are too close together for the difference of their ground heights (the steepest grade the road between
them may have is nine tenths of what vehicles manage), they move toward each other, as far as it takes, and the roads
around them with them. A road is held at one joint where it passes a junction, at its end where it ends there, and a junction piece lies on its plane, which leans along the through road (below) and is level where there is none; roads that merely pass near a junction (a parallel street, a road on a bridge above) are not held to it. The road between two junctions is measured from the joint that holds one height to the joint that holds the other. Joints of a road less than a metre apart (the overlapped straights of a long road put two a few centimetres apart) are one height. What cannot be kept at all is the `junction_too_steep` finding.

**Junctions on a slope lean with the road.** A junction is not level on a hillside: its plane tilts along the road that runs through it (a junction piece's own axis, the road on top at a crossing without a junction piece, the road a chain ends on) by the slope of the original ground under that road (measured along the road over 25 m either side), at most 8 percent, and has no fall across the road. The limit is the fall across that a road is aimed at: the junction's tilt is the fall across every road that meets the through road there, and those roads have no fall across of their own. The tilt, like the height, is pulled toward the ground's and not fixed: where the roads around a junction need it, it is flatter. The joints of a junction piece, the end of a side road and the ground patch under a crossing lie on that plane, so two junctions only a few metres apart on a slope are connected by a road that climbs instead of a step. A crossing where all roads end, and a junction on a roundabout, stay level (a roundabout has its own plane, which the solve does not change).

Between a free end and a junction a road is at most **6 % steep** (asphalt and streets; gravel and dirt tracks keep their
own limit, so a steep track is not lifted out of the ground, and between two junction heights the road's own grade limits
apply). The same 6 % holds on the pieces of a road that pass within 12.5 m beyond its edge of where another road ends on it (that road's end lies on
it, within a metre of its edge); a road that ends beside it without reaching it, such as a dead end a few metres up the
slope or a road that ends at a junction the first road only passes near, does not count. A road is gentler in a tight bend: the inner edge of a bend is
steeper than its centre line, so the bend's grade is lowered to keep that edge within the family's aim.

**A road follows the ground, smoothly, within its limits.** A road keeps to its family's grade (its aim) where the
ground lets it. Where the ground asks for more, the road becomes steeper than its aim, the more the steeper the ground
is, but not steeper than the limit of its family, nine tenths of what vehicles manage, unless nothing else is possible;
it is then amber in the Drivable layer, not red. The grade of a road may not change faster than the vertical radius
of its family allows (600 m on asphalt 1 down to 60 m on a dirt track), and a bridge deck is level, so the road
before and after it bends into it within the same radius. Where nothing else is possible, the limit is broken as
little as possible and `road_grade` says where.

The terrain grid is coarse (a cell is 6.25 m on a 6.4 km map at `terrain_size = 1024`, and never less than 4 m, see [the grid](project-file.md#grid)), and every cell under a road takes the road's height
for a margin of one and a half cells beyond its edge. **Roads closer together than that** (the two legs of a hairpin,
streets side by side) cannot differ in height, so the solve ties them to one height before the terrain is shaped,
side by side: each spot of a road with the nearest spot of the other. Beyond that distance two roads may differ by the
step one cell can hold. **Roads are tied only where the ground lets them:** where the original ground under two
roads side by side differs by more than that step and a metre (a hairpin on a steep flank), they keep their own
heights, and the terrain under one of them is still wrong; those places are the `road_misses_terrain` and
`road_not_drivable` findings that remain. Roads that meet at a junction at an angle are tied only near where they
meet, and roads that leave a junction side by side are tied like any others. Only a bridge and the road it crosses
(within 50 m of the crossing), and the two ends of a road that leaves and re-enters one junction, are never tied to
each other.

**What is not built:** tunnels (they need terrain holes; the road ends in end caps at the portals and the build
says so), junctions at an angle other than 90° (the game has only right-angle junctions; the side road bends to the OSM
line after the junction), railways, houses and trees.

**Which junction piece is used.** A T crossing needs the game's T piece for the pair of surfaces. For a crossing of two
roads the game has an X junction for the pair with the roles swapped (the street crossing an asphalt 1 road has only the
asphalt 1 road crossing a street); that piece is used, turned by 90°. A T crossing is never swapped.

**One road through a node.** Two roads of one surface that meet alone at a node (a bridge cut from its way, a way that
only changes bridge, tunnel or layer there) are one road up to a bend of 150°, so a bridge that begins in a bend does not
split the road. Nodes with three or more roads keep a 45° limit for choosing the through road.

**Where the game has no junction piece for a pair of road types** (most pairs of different surfaces, and gravel with
gravel), one road lies on top of the other. The higher road is the one of the better surface (asphalt 1, asphalt 2,
street, asphalt 3, gravel, dirt track); of two of the same surface the longer one. It runs on unchanged. A lower road
that ends on it stops just under its edge (it reaches under the edge by up to 2 m, but never past the centre line)
and has no end cap there. A lower road that crosses it (two higher roads that leave it on opposite sides) is interrupted: it ends under one edge and starts again under
the other; two higher roads that leave it on the same side (a fork) do not interrupt it, they end on it. A higher road that ends on a lower one lies on top of it, again without an end cap. At the cut the lower
road is 5 cm below the surface of the higher one, so the two surfaces do not flicker; the ground there lies on the
junction's plane like under any junction. A piece of road between two such cuts that is shorter than half its width is left out, and the
build says so. These overlaps are intended; they are not `road_conflict` findings and not `join_without_junction`.
`join_without_junction` is left for crossings where no road runs through (several roads of different surfaces that all
end at one point).

**A road that OpenStreetMap draws twice** (two ways on top of each other)
is placed once. Two roads that run along each other for at least 25 m, with their centre lines
closer than half the width of the narrower road and their directions within 15° of each other, are taken as one road:
the higher one (the rule above: better surface, then the longer, then the lower way id) stays, and the part of the other
that runs along it is not placed. Where the lower road goes on after that part, it ends on the kept road under the
overlay rule above (the kept road is cut there so that the two meet at a crossing). Each dropped part is reported as a
`road_duplicate_dropped` finding in the middle of the part. Only roads are compared; paths (sidewalks, for instance)
stay. A bridge is compared only with bridges and a tunnel only with tunnels, and is never cut: a bridge that runs along
another one for at least 25 m is not placed at all.

**Junctions move to close a road.** A road that cannot reach the port of a junction piece (it would end more than 0.5 m
from it, a `road_gap` finding) is tried again with that junction moved along its through road, in steps of 6.25 m up to
25 m, first to one side and then to the other. The first position at which the road closes is taken, else the one with
the smallest gap if that is smaller than before. The network is then placed again with the moved junctions and the
result is kept only if it has fewer gaps, loses no junction piece and adds no join without a junction. A through road
never ends further from a junction that was moved than it did before (beyond the 0.5 m): a move after which it does is
taken back and the network is placed once more, and if that does not hold either, nothing is moved. A moved
junction keeps a junction length (12.5 m) from its neighbours on the road and stays off bridges; a junction on a
roundabout ring is never moved. A junction that moved sits up to 25 m from where OpenStreetMap has the crossing. This
makes the placement take about three times as long.

**Roundabouts.** A closed loop of roads tagged `junction=roundabout` (or `junction=circular`) is built as one ring: a
closed loop of the game's arcs and straights, with no junction piece on it, lying on top of every road that joins it
(whatever their surfaces; they end under its edge by the overlay rule above). The ring lies on **one tilted plane** that
follows the slope: the plane is fitted to the original ground under the ring's road (a band as wide as the road, least squares), and its tilt is cut down to
the steepest grade the ring's road may have. The build shapes the ground under the road and one cell beside it to that plane (the ground inside the ring keeps its height), so
a ring on a steep street needs no large flat bench, and the roads that join it end at the plane's height (the lower
5 cm below, as above). A ring whose centre line is tighter than the tightest arc of its surface (10 m for the game's
own roads) cannot be laid out with the game's pieces; it is taken as a **plain crossing**: the ring is left out and the
roads that joined it meet at one point at its centre, with a junction piece where the game has one and the overlay
rule elsewhere. This is the common case on a map at a small scale (a mini roundabout of 9 m radius is 4.6 m at half
scale). An open arc of roundabout ways, a loop with a way in it that is not tagged as a roundabout, one that a third
roundabout way leaves, and one with several surfaces are not rings; they stay ordinary roads.

**Bridges join their road.** A bridge deck lies on its OpenStreetMap line, with the heading of the line. The road before
it is fitted to the deck's start; where the pieces cannot end exactly there, the deck slides at most 2.5 m along its
line (0.25 m steps, the shortest slide first) until the road ends within 0.1 m and 1 degree of the deck's port. The road
behind the deck starts at the deck's end, on the line. Where no such slide exists, or where the road turns into the
bridge (it arrives more than 2.5 degrees off the line's chord, as on a bridge on a curve), the deck keeps its place and
follows the road: it starts where the road ends, with the road's heading.

**A bridge over a road keeps its clearance.** Where a road tagged as a bridge crosses another road without a shared
node, the underside of the deck keeps **4.5 m** from the surface of the road below. The room is shared between the
two roads: the bridge's road rises and the road below may be lowered into a cutting under the bridge, each as far
as its grade limits and its vertical radius allow, and the terrain under the bridge is shaped for both. The deck's thickness is measured from the bridge model's Geometry LOD (the deepest underside over the longest open stretch between its solid parts, at most 4 m; what reaches lower, such as piers and abutments, is not the deck). Every bridge model has its own thickness (stone, wooden and metal bridges differ). A model whose Geometry LOD cannot be read takes the thickest deck measured among the models of its type (stone, wood, metal), else the thickest of all, and the build warns with the model's name; if no bridge model can be read, the clearance is not checked and the build says so. The deck is held at least that high above the road below in the same calculation as all other road heights, so the bridge, its approaches and the junctions near it rise and the road below dips only as far as the cost of moving each road from its ground shares it out. Where the two together cannot make 4.5 m within their limits, the
finding is `bridge_clearance`, with the clearance reached. Crossings that are not tagged as a bridge (and tunnels) are
not treated like this.

**A bridge over a road is built so that the road passes through its opening.** For every road below a bridge (a road
that crosses its line at another height; roads that meet at a node join there, so a crossing within the deck's half
width plus the road's half width of a shared node is none, while a road that leaves the node, loops and crosses the line
further on is a road below at that place), the part must carry the whole of what the road needs along the span: the road's
footprint (its width over the sine of the angle between the two lines), a metre of margin on each side, the skew of
the road's centre line across the part (as wide as what hangs under the deck), and, where the part holds nothing back
(panels, `bridge_wood2`), the run of the cutting's walls under the deck (the clearance plus the deck's thickness, over
the cutting's slope of 2). A road nearly along the bridge asks for a long interval (the sine is never taken below
0.01). The road needs 4.5 m under the deck at its centre line and 3 m over its footprint; standing geometry (end
blocks, arch flanks, posts, the cross walls of `bridge_concrete3`) never passes a road. The first of these that works is
used: the part the table gives, slid along the bridge's line within its straight room (and the turn limit and the posts
of the model); the panel grid, lengthened into the road beside the bridge by exactly what each end lacks (at most 15 m at an end,
as far as that road goes on, keeping a metre before a junction, another bridge, a tunnel or a dead end; a grid needs
no straight room and follows the bend of the road it grows into, its steps running chord to chord along the line, and
the grid is then computed again for the longer length). An end where the road's footprint would still lie outside the
deck after that does not grow at all (more deck would not help the road); an end where only the cutting's walls stay
outside grows as far as it can. A bridge of 3 m or less over a road is such a grid, not plain road. Else what is still
missing, the placement that leaves the least of the road's interval
under a solid part or past the end of the deck, with the finding `bridge_blocks_road` and the missing metres. A bridge
that was slid shows it in `build/report.json` (`slid_m`, `part_start_m`, and `below` with the interval each road needs
and what is missing). The clearance of the height solve keeps 4.5 m under the deck's deepest point over the road's centre line (a metre either
side of the crossing) and 3 m over its whole footprint (for the stone arch: its depth where the road passes, not its crown
and not the abutments), not the deck's thickness over its longest opening. A road below is a finding only where a placed
piece of it (or of a part split from it) runs under the placed deck (its centre line inside the deck's outline widened by
the road's half width); a road that the placement laid away from the deck (a bend in its line too tight for the pieces)
has `placed: false` in the report and is not a finding.

**The ground under a bridge is shaped for the bridge.** The bridge wins and the terrain is adapted to it, so that roads
can pass onto, over and under bridges. After the roads are placed, the terrain under every placed part (a model between
its ports, a step of a panel grid, as placed: a grid lengthened over a road, a model slid or grown, a grid that follows
a bend) is cut to a ceiling: the height of the deck, less what hangs under it there (the catalog measures it in slices
of 1 m: the deck, an arch, a truss floor), less 1 m of air (`BRIDGE_AIR_M`). Ground that is higher is cut, and the cut
spreads sideways with the cutting slopes (1:1 to 2:1 by the hillside), from the width of what is below the deck and
one and a half cells, so that the opening is open at both sides. Where something stands (the abutment blocks of the
metal bridge, the flanks of the stone arch, the posts of the wooden bridge, the cross walls of `bridge_concrete3`,
about every 3 m) the bridge's cut is the same: it stops at the ceiling and never goes to the foot of what stands; the
foot (the deck less the model's deepest point) is only the lowest that the cutting of another road may take the ground
under it, and a vertex beside the deck has none. Each vertex takes the lowest ceiling within half a cell along the
span. At an end that holds nothing back (panels, `bridge_wood2`) the ceiling rises to the height of the road at the
port, a metre of height per metre of run; where the part retains the ground there is no such ramp. Within the end
guard of a free end of the bridge (a cell along the deck, `cell * (|ax| + |ay|)` for a deck turned against the grid, so
that the whole of a deck shorter than two guards is in it) the ground is under the deck but not cut for it, because a
terrain cell that straddles the port line would slope the road's last piece down to the cut: it keeps its height and
no level patch claims it. Beyond the guard ground under a deck is never raised above the original ground. Inside the
guard the bridge's own road (the strips of the road the bridge is part of, up to the next junction at each end) may hold
or raise ground up to its road height, as the abutment's fill; beyond the guard it holds nothing under the deck. A road
that passes under the deck keeps its core and the ground around it, and its cutting spreads under the open parts of
the deck. A road above the underside of the deck (a junction arm, a road beside the deck end) claims a vertex under the
deck only within its own width, never with the shoulder ring of its earthwork. The core of another road is never
changed. Ground that is in the deck is the finding `bridge_buried`: more than 0.1 m above the underside of the deck
(the deck's height less what hangs under it there, without the metre of air and never the foot of what stands) beyond
the end guard, more than 0.1 m above the deck's surface inside it; whoever holds it (the message names the road or the
level patch, or says that no road holds it: the ground was not cut there). A bridge model on posts (`bridge_wood_25`)
is chosen where the posts reach the original ground; if the shaped ground, under the deck as placed, is more than 0.5 m
below the foot of a post, pier or wall, the finding is `bridge_post_floats`. A `road_conflict` that the cut under a deck
causes says so ("The cut under a bridge's deck and a road want heights ... apart here").

**Bridges.** On a bridge, the span has a level deck and the ground below is cut to its ceiling (see above), not left
alone; the ground starts to be shaped for the road at the abutments. Gravel and dirt roads get the wooden bridges, the
paved roads the stone and metal ones: asphalt 1 and streets the metal bridge, asphalt 2 the stone bridge, asphalt 3
the stone bridge or `bridge_concrete3`, gravel and dirt tracks `bridge_wood_25` or `bridge_wood2`. A model may be
up to 1 m narrower than the road (wider is always fine); a model more than 1 m narrower is not used. Bridge models are never
scaled (the panels of a grid are). Which of them, or whether a grid of concrete panels, is told below. For each model the
catalog measures, on its collision geometry, what hangs below the deck in slices of 1 m: the deck, the solid ends
(the abutment blocks of the metal bridge, the flanks of the stone arch) and whether they hold the ground back, and
the wooden posts. The thickness used for the clearance over a road below is what hangs under the deck over that road's
own interval (see below); a bridge over a dip, which has no part of its own, takes the thickness over the longest open
stretch between the solid parts.

**What a bridge is built from.** Every OpenStreetMap bridge is built from a part that fits its length (on the map; bridge ways that follow each other
are one bridge):

- **At most 3 m:** plain road on the terrain, on purpose (no finding), unless the bridge meets other bridges at a node
  where only bridges meet (a fork of bridges): then it is at least one panel.
- **At least two thirds as long as a bridge model of its road:** the shortest such model that is at least as long as
  the bridge. A bridge longer than all of them gets a chain of the longest end to end (`bridge_wood2` is never
  chained; it is laid alone where it covers two thirds of the bridge). `bridge_wood_25` stands on posts and is used
  only where the original ground under every post (over the whole post, beside the deck too) is within the posts'
  reach (2.6 m) below the deck and at most 0.5 m above it, the deck taken on the line between the ground at its two
  ends; else the next model.
- **Anything else:** concrete panels (`panel_concrete_1`), laid crosswise as a grid: some one after the other along
  the bridge's own line, from its start to its end (also on a bend), and as many side by side as the road's width
  needs. All panels of a bridge have one scale, between 0.75 and 3, chosen so that they cover the bridge's length
  exactly and the grid comes as near the road's width as it can (not more than 1 m narrower where any grid avoids
  it). An 8.85 m bridge on a 12 m road gets 3 panels along and 2 abreast, 11.8 m wide.

A panel grid that is not over a road never grows (see the bridge over a road below for the one case where it does). A model longer than the bridge takes the missing length evenly from the road at both ends,
but **only along a straight line and only on a straight bridge**: the bridge's own line and the road the model grows
into may turn by at most 5 degrees together, and the model keeps at least 1 m before a junction, another bridge or a
dead end; where one side has no room the other takes the rest. A model without room, or on a bridge that bends,
becomes a panel grid of the bridge's length. No bridge is dropped: the build knows for every OpenStreetMap bridge what
it became and why (and why its room ends at each end). If the install has neither panels nor a model that fits, the
bridge is plain road and the finding is `bridge_not_built`.

**How a bridge is laid.** The deck lies node to node on its OpenStreetMap line. Where the road beyond a bridge's end
leaves it turned away (by at least 1 degree), the last stretch of that road before the bridge is made straight on the
bridge's axis, `2 R tan(turn / 2)` metres long (at least 4 m; R is the tightest curve of the road's pieces), so that the
pieces can join the deck; the road leaves its OpenStreetMap line by up to the lead's length near a bridge (measured up to 12 m). At a junction, a node of
bridges or where the road is too short nothing is moved, the road then still reaches the deck's start in position (a miss of
more than 0.5 m is a `road_gap` finding) and arrives on its own heading; where that heading is more than 5 degrees
off the deck's, it is a `road_kink` finding at the bridge's end (only there, not at junctions). A panel grid is a real bridge: its panels lie crosswise (the long side across the road),
each on the heading of the bridge's line at its centre, so a grid follows a bend; the panels of a step are centred on
the line. The grid is one level deck for the height solve (as a model is), as thick as one panel at its scale, with no
road under it and no solid part; the origin of every panel is 0.12 m times its scale below the deck level (measured in
the game: the panels lie flush with the road and carry). Panels are the only objects of a bridge that are scaled. A model lies
the way round its stretch is stored (its first end at the stretch's start); a model shorter than its bridge is centred
on it (it keeps to the end where another bridge meets it: bridges that meet at a node of bridges end exactly at the
node), and the road between its end and the end of the bridge is plain road. `build/report.json` has a `bridges`
list with, for every OpenStreetMap bridge, its ways, its position, what it was built from (`fill`, `panels 3 x 2 at
scale 0.98`, `bridge_wood2 x 1`) and why it is not the part the table gives first; the build prints the same, one
line per bridge. The bridge models of the Sakhal archive (`sakhal/addons/structures_sakhal.pbo`) are unpacked for
`binarize` next to the road models when the map uses them, and removed again; a build that needs such an archive
that the install does not have stops and names the model.

A road that stands more than 3 m above the original ground over some length is crossing a dip. Where the dip is at
least **8 m deep** and a bridge model fits (the dip is at least half a model long, a whole number of models laid evenly over it stays 1 m clear of junctions,
dead ends, other bridges and a bend right after its stretch, and the road is straight within 2 m over each model; an OpenStreetMap bridge built from its own part gets none) it gets a bridge, or a chain of bridges end to end over
a long dip; the ground below is shaped for it as for any bridge. A bridge of a dip needs straight road before and after it: at each end, every point of the road within 12.5 m outside the bridge (also across a change of surface) must lie within 10 degrees of the direction of the bridge model there (with less than 12.5 m, at least 4 m must be measurable), and the road over 4 m inside the bridge within 20 degrees. When the bridge laid evenly over the dip passes the straightness test over each model but fails this, it is moved along the dip, as far as it still covers it, in 0.5 m steps (the shorter move first, the earlier position on a tie); if no position passes, the dip is filled instead. Otherwise (a dip less than 8 m deep, or no model fits) a dip without water is
filled, as before, and one with a stream
(`waterway=stream`, `river` or `canal` crossing the road), a river or a lake under it is left as it is and reported as
`road_over_water` for you to look at. Only after the heights are solved is the dips' bridging decided, so **when a dip gets a
bridge the roads are placed a second time and the road part of the build takes about twice as long** (there are no
further rounds). The bridges of a dip are objects like any other and can be changed in `edits/overrides.toml`; their
ids are `road/<stretch>/bridge<n>/<slot>`.

The ground under the pieces that were placed (not under the OSM line, which they may leave by some metres) is painted
as soil, 2 m wider than the road.

**Roads in the game.** `binarize` is given the game's road models (the build unpacks `structures_roads.pbo` and puts the models on its `P:` drive for the run, and removes them afterwards), and roads, junctions and bridges have been looked at in the game. The vehicle limits are less tested. If a bridge misbehaves, replace it in `edits/overrides.toml`.

Your own changes to what was generated are kept in `edits/overrides.toml`
([the project file](project-file.md#editsoverridestoml)).

**The terrain changes where there are roads, so a [navigation mesh](navmesh.md) made earlier no longer fits.**
`naves build` warns about that when the project has one.

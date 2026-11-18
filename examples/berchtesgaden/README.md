<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Berchtesgaden test map

The reference project of milestone 1: a 16.4 km square of the Berchtesgaden Alps at half scale (an 8.2 km map,
`terrain_size = 2048`, terrain cells of 4 m). Half scale keeps the slopes steep, which is what the road placement
has to cope with. Until 2026-11-14 it was a 12.8 km square (a 6.4 km map); the 2048 grid on it had cells of 3.1 m,
which crash the game client, so the area grew, about a tenth towards north and east and the rest towards south and
west.

| | |
|---|---|
| Centre | 47.571769 °N, 12.946451 °E |
| Area | about 47.496–47.647 °N and 12.835–13.058 °E |
| In it | the Königssee, the Watzmann massif (west of the lake), the Hochkalter group, the Hintersee, Berchtesgaden town |
| Elevation data | Copernicus GLO-30, two tiles that meet at 13.0 °E, so it also tests the tile seam |
| Ground | ESA WorldCover (one tile) and OpenStreetMap (Oberbayern and Austria: the south and east of the map are Austrian) |
| Satellite image | Sentinel-2, one clear scene of 2025-09-19 (10 m), which covers the whole map |

## Fetch the elevation data

The tiles are not in the repository (about 42 MB each). Run this in `examples/berchtesgaden/dem/`:

```sh
B=https://copernicus-dem-30m.s3.amazonaws.com
for e in E012 E013; do
  t=Copernicus_DSM_COG_10_N47_00_${e}_00_DEM
  curl -fO $B/$t/$t.tif
done
```

## Fetch the land cover and OpenStreetMap data

Not in the repository either. Run this in `examples/berchtesgaden/landcover/` (85 MB):

```sh
curl -fO https://esa-worldcover.s3.eu-central-1.amazonaws.com/v200/2021/map/ESA_WorldCover_10m_2021_v200_N45E012_Map.tif
```

and this in `examples/berchtesgaden/osm/` (260 MB and 815 MB; the build reads only what the map needs, but needs
about 1.2 GiB of memory with both):

```sh
curl -fLO https://download.geofabrik.de/europe/germany/bayern/oberbayern-latest.osm.pbf
curl -fLO https://download.geofabrik.de/europe/austria-latest.osm.pbf
```

Without these two folders, remove `land_cover` and `osm` from `naves.toml`: the ground then follows slope and
altitude only.

## Fetch the satellite image

The satellite image of the terrain is merged from two sources: Sentinel-2 (10 m, one clear scene of 2025-09-19 that
covers the whole map; Austria and Bavaria) and the aerial photographs of Bavaria (DOP40, 0.4 m, used at the
resolution of the map; the south and east of the map are Austrian, where the service answers blank and the Sentinel-2
image shows). Both are free to use, commercial use included. Náves fetches them into `imagery/`:

```sh
naves imagery fetch sentinel2-scenes examples/berchtesgaden   # 2 min, a 7 MB crop of a 340 MB scene
naves imagery fetch bayern-dop40 examples/berchtesgaden       # 14 min, a 575 MB file
```

You can also choose and fetch the sources in the GUI (`naves gui examples/berchtesgaden`, the project panel).

The scene is dark for a terrain; the build makes the imagery 30 % brighter, and `[imagery] brightness` in `naves.toml`
changes that. Náves evens out the colours of the photographs and the scene where they overlap; the photographs are
much brighter than the scene, and where the kind of ground differs (forest is nearly black in the scene) a soft band
of colour remains at the border to Austria. Without the files, remove the `[[sources.imagery]]` blocks from
`naves.toml`: the satellite image is then painted from the ground classes. With only Sentinel-2, the map is built
from a smaller file and has no band. For a non-commercial map, `eox-cloudless-2025` is the newest cloud-free
mosaic; the map then says that it is non-commercial.

Cost of the satellite image (release binary, `naves build`, **without roads**; same map without imagery in brackets)
and of the whole build with roads (OpenStreetMap, bridges over dips, `binarize` included):

| Map | `terrain_size` | Build time | Peak memory | PBO | With roads: time | memory | PBO |
|---|---|---|---|---|---|---|---|
| 6.4 km | 1024 | 16 s (15 s) | 1.7 GiB (1.2 GiB) | 75 MB (51 MB) | 325 s | 1.7 GiB | 78 MB |
| 6.4 km | 2048 (crashes the client) | 38 s (32 s) | 1.7 GiB (1.2 GiB) | 220 MB (118 MB) | 350 s | 1.8 GiB | 223 MB |
| 8.2 km | 2048 | | | | 897 s (2026-11-16, height solve at `98cfcf4`; 1775 s road look; 396 s 2026-11-15) | 3.5 GiB (2026-11-15, not measured since) | 233 MB (2026-11-15) |

The figures with roads are from 2026-11-14 to 2026-11-16, each row as dated (AMD Ryzen 9 9900X, 24 threads). On the 6.4 km map 2048
gives terrain cells of 3.1 m, and the DayZ client crashes on cells under 4 m (see the grid in
`docs/manual/src/project-file.md`); the 8.2 km map has cells of 4 m. Placing the roads takes most of the time, and
they are placed twice when a dip gets a bridge: on the 6.4 km map at 2048 the two rounds took 152 s and 148 s,
`binarize` 31 s, everything else about 15 s.

The finer grid does not remove the road findings: on the 6.4 km map `road_misses_terrain` was 4604 at 1024 and 4015
at 2048, `road_conflict` 749 and 1130 (`docs/research/2026-11-10-road-alignment.md`, section 7.9).

## Build

```sh
naves build examples/berchtesgaden
```

## Navigation mesh

The mesh is too large for the repository (21 MB). See [Navigation mesh](../../docs/manual/src/navmesh.md) in the manual
for how to make it; put it at `navmesh/berchtesgaden.nm`. The results below were made with one.

## The 84 bridges on the 8.2 km map at 2048 (2026-11-18)

What each of the 84 OpenStreetMap bridges of the map became, measured with `road-analysis` at `terrain_size = 2048`
on the branch `bridges` (the run is of `f5f1fda`; `2d0e09c`, the head, differs from it in comments and a rewritten
condition in `validate.rs`, no behaviour). Details and the reasoning: `docs/research/2026-11-18-bridges.md`; the
design: `docs/superpowers/specs/2026-11-16-bridges-and-terrain-design.md`. Nothing was tuned for these figures.

**What they became.** Every bridge of more than 3 m is built from a game part; the five of 2.4 to 3.0 m are plain
road on purpose. No bridge is plain road because the install lacks a part (`bridge_not_built` 0).

| Built from | Bridges |
|---|---|
| concrete panels (a grid of one or more, 199 panels, 128 of them scaled) | 76 |
| `bridge_wood2` | 1 |
| `bridge_concrete3` | 1 |
| `bridge_metal_25` | 1 |
| plain road (2.4 to 3.0 m) | 5 |

- 6 bridges lie over another road. 14 deck ends were grown past the bridge's end, to give the road below its opening
  or to meet the road (8 bridges); 12 of them grew into a road that turns by at most 5 degrees, two do not (299428041,
  7.2 degrees at its start; 36788104, 6.2 degrees), the largest turn being 7.2 degrees.
- 6 bridges have ground in their deck (`bridge_buried`: 48450084 0.81 m, 39537999 0.54 m, 258387240 0.52 m,
  1341010238 0.27 m, 150368334 0.19 m, 172178577 0.12 m); there were 33 before the ground under the bridges was
  shaped.
- 3 solid parts stand on a road below (`bridge_blocks_road`: 1341010238 Silbergstraße 1.4 m of the 21.0 m the road
  needs, 74208396 0.4 m of 19.8 m, 74208422 0.6 m of 12.8 m).
- 5 of the 6 bridges over roads were measured; four keep the 4.50 m clearance under the deck and 299428041 reaches
  2.30 m (`bridge_clearance` 1). Without the clearance in the height solve they were between -1.32 m and -0.28 m.
  Silbergstraße (1341010238) was not measured: the OpenStreetMap crossing is not where the placed road below runs
  under the lengthened deck.
- 3 decks end at a height step from the junction they join (`bridge_end_step`): 130047131 0.5 m above, 39663890
  0.6 m below and 74208396 0.8 m below. A level deck cannot follow two junctions that want different heights.

**Findings of the whole map**, against the state before the bridge work (the same map and size):

| Finding | Before | Now |
|---|---|---|
| `road_gap` | 356 | 356 |
| `road_kink` | 21 | 21 |
| `road_conflict` | 905 | 899 |
| `road_not_drivable` | 186 | 185 |
| `road_misses_terrain` | 4271 | 4285 |
| `road_grade` | 3 | 4 |
| `bridge_clearance` | 0 | 1 |
| `bridge_blocks_road` | 3 | 3 |
| `bridge_buried`, `bridge_end_step`, `bridge_post_floats` | (rule did not exist) | 6, 3, 0 |
| `bridge_not_built` | 0 | 0 |

What got **worse**, and why:

- `road_misses_terrain` 4271 to 4285. The count was not tuned; the figure after the bridge work and before the decks
  were held to their junctions was 4292, and the baseline before any of it 4317.
- `road_grade` 3 to 4 and `bridge_clearance` 0 to 1 are one place, bridge 299428041 at (2349, 6546): the road below
  starts at the bridge's own end node, 2 m from the deck, and loops under it. Before, the deck floated 5.7 m above
  that road's start and nothing reported it. Now the road starts at the deck's height and cannot get under it
  within 13 m: 2.3 m of the 4.5 m clearance and a crest of 35 m vertical radius (60 m is the minimum). Neither is
  drivable; it is a placement problem that is now reported.
- Silbergstraße has 0.27 m of ground in its deck (0.14 m before).

**Time** (`road-analysis` at 2048, release build): placement 803.8 s, shaping the ground under the bridges
(`conform`) 30.4 s, the whole analysis 848.0 s with a peak of 1.3 GiB. The height solve: 4.1 s in all, one solve of
0.88 s, 97 571 unknowns, 200 132 rows, 16 iterations. `naves build` was not timed at this state.

**In the game** (DayZ 1.30, headless client, three checks with `tools/ingame-check`, 21 to 37 sites photographed;
the pictures are not kept in the repository):

- Check 1 (`4b0dea6`): the decks of 39674984 (Bergwerkstraße), 286196692, 48450084 and 130657216 stood above or
  below the road at their ends.
- Check 2 (`775b101`): after the decks were held to the junction at their end, 39674984, 286196692 and 130657216
  were flush.
- Check 3 (`2d0e09c`, the head): the same three flush, and at 240795460 the road now meets the deck in the top view
  after the junction hold.
- Still wrong in the game at the head: 48450084 (the own width of a neighbouring road overlaps the deck's corner),
  299428041 (the path below climbs a steep ramp to the bridge's end; `bridge_clearance` 2.3 m) and 74208396 (a 0.76 m
  step at one end, reported as `bridge_end_step`; the solve prefers the step to a short ramp because of the crest
  and dip rule, and a follow-up task changes that).
- **No vehicle was driven over any bridge.** Only the photographed sites were looked at.

**Still open:**

- The three sites above.
- The clearance at Silbergstraße (1341010238) is not measured.
- Two grown deck ends turn by more than 5 degrees (299428041 and 36788104).
- Sidepaths next to a bridge and junctions on a bridge end, which the planned clean-up of the road network addresses.
- A deck with a pitch (a level deck has the steps above where its two ends meet different heights) is not done.

<details>
<summary>All 84 bridges (way, place in map metres, length, what it was built from, notes)</summary>

| Way | At (x, z) | Length (m) | Built from | Notes |
|---|---|---|---|---|
| 1008887764 | 8143, 7700 | 10.6 | concrete panels |  |
| 1093361022 | 7256, 4498 | 3.7 | concrete panels |  |
| 123928797 | 2161, 6105 | 9.8 | concrete panels |  |
| 1299228927 | 7612, 7402 | 5.4 | concrete panels |  |
| 1299522645 | 7122, 5797 | 12.5 | concrete panels |  |
| 130047129 | 2907, 5952 | 9.6 | concrete panels |  |
| 130047131 | 2454, 6103 | 9.9 | concrete panels | ends 0.5 m above its junction (`bridge_end_step`) |
| 130657211 | 1543, 6246 | 15.2 | concrete panels |  |
| 130657212 | 828, 6035 | 11.0 | concrete panels |  |
| 130657216 | 1299, 6236 | 9.4 | concrete panels |  |
| 1341010238 | 4689, 7320 | 8.8 | concrete panels | grown by 5.6 m at the start and 5.1 m at the end; over a road; `bridge_buried` 0.27 m; `bridge_blocks_road` 1.4 m |
| 1346625682 | 4215, 6429 | 2.4 | plain road |  |
| 136213584 | 6168, 7041 | 4.2 | concrete panels |  |
| 136213585 | 6197, 7057 | 7.3 | concrete panels |  |
| 1374824779 | 2618, 6016 | 4.7 | concrete panels |  |
| 1411269163 | 5972, 6013 | 5.0 | concrete panels |  |
| 1467990031 | 5838, 8051 | 5.0 | concrete panels |  |
| 150368334 | 6170, 7036 | 8.6 | concrete panels | `bridge_buried` 0.19 m |
| 1544648747 | 7242, 7546 | 4.6 | concrete panels |  |
| 1547208616 | 615, 5961 | 10.1 | concrete panels |  |
| 157901070 | 5092, 7688 | 8.3 | concrete panels |  |
| 157901090 | 1583, 6634 | 3.1 | concrete panels |  |
| 172178568 | 5431, 5141 | 7.1 | concrete panels |  |
| 172178577 | 5261, 5099 | 6.8 | concrete panels | `bridge_buried` 0.12 m |
| 172190184 | 6440, 7292 | 15.3 | concrete panels |  |
| 174770474 | 3079, 5927 | 10.8 | concrete panels |  |
| 180749711 | 2616, 4867 | 6.5 | concrete panels |  |
| 230093457 | 349, 5824 | 5.9 | concrete panels |  |
| 233434385 | 6625, 7863 | 5.9 | concrete panels |  |
| 23449348 | 3314, 5807 | 13.8 | concrete panels |  |
| 23458060 | 5683, 5756 | 17.7 | concrete panels |  |
| 23458920 | 5030, 6597 | 17.9 | `bridge_concrete3` | grown by 3.2 m at the start and 0.0 m at the end |
| 236042178 | 535, 5681 | 7.2 | concrete panels |  |
| 236042179 | 378, 5584 | 6.1 | concrete panels |  |
| 23631314 | 5556, 5270 | 19.1 | concrete panels |  |
| 240795460 | 5706, 7097 | 8.4 | concrete panels |  |
| 240795464 | 5718, 7074 | 15.8 | concrete panels |  |
| 245085906 | 5688, 8054 | 6.6 | concrete panels |  |
| 247142192 | 6094, 3047 | 6.4 | concrete panels |  |
| 258380012 | 509, 8080 | 7.8 | concrete panels |  |
| 258382729 | 1550, 7303 | 35.0 | concrete panels |  |
| 258387240 | 1724, 6741 | 11.5 | concrete panels | `bridge_buried` 0.52 m |
| 258392493 | 2316, 6516 | 12.2 | concrete panels |  |
| 260098913 | 1314, 7305 | 2.9 | plain road |  |
| 263005056 | 5227, 5099 | 7.6 | concrete panels |  |
| 267873872 | 5983, 5251 | 7.5 | concrete panels |  |
| 286196692 | 5554, 5167 | 10.9 | concrete panels |  |
| 28885465 | 6096, 6613 | 15.9 | `bridge_wood2` | grown by 1.9 m at the start and 1.9 m at the end |
| 296178597 | 2248, 6118 | 6.4 | concrete panels |  |
| 299428041 | 2349, 6546 | 19.1 | concrete panels | grown by 1.3 m at the start and 0.0 m at the end; grown end turns 7.2 degrees; over a road; `bridge_clearance` 2.3 m of 4.5 m |
| 312506426 | 6312, 7823 | 9.0 | concrete panels |  |
| 36788104 | 7995, 7319 | 14.4 | concrete panels | grown by 2.4 m at the start and 3.5 m at the end; grown end turns 6.2 degrees; over a road |
| 39537999 | 6195, 7055 | 8.4 | concrete panels | `bridge_buried` 0.54 m |
| 39605281 | 5104, 7915 | 36.2 | concrete panels | over a road |
| 39623494 | 7315, 8074 | 14.6 | concrete panels |  |
| 39663890 | 2841, 5203 | 12.5 | concrete panels | ends 0.6 m below its junction (`bridge_end_step`) |
| 39674984 | 6832, 7682 | 18.5 | `bridge_metal_25` | grown by 3.3 m at the start and 3.3 m at the end |
| 39782354 | 6871, 7907 | 11.5 | concrete panels |  |
| 39782358 | 6802, 7686 | 5.0 | concrete panels |  |
| 40306826 | 6552, 3739 | 5.2 | concrete panels |  |
| 40883071 | 4395, 6437 | 14.2 | concrete panels |  |
| 419057266 | 824, 6057 | 3.0 | plain road |  |
| 421568995 | 471, 6045 | 2.7 | plain road |  |
| 433025400 | 1774, 6201 | 7.7 | concrete panels |  |
| 48391149 | 6198, 7022 | 13.0 | concrete panels |  |
| 48391150 | 6171, 7042 | 3.6 | concrete panels |  |
| 48391152 | 6202, 7051 | 8.0 | concrete panels |  |
| 48450084 | 6057, 6492 | 17.2 | concrete panels | `bridge_buried` 0.81 m |
| 48450231 | 3326, 5847 | 10.6 | concrete panels |  |
| 495868617 | 1160, 6083 | 3.9 | concrete panels |  |
| 496713926 | 2272, 6134 | 3.2 | concrete panels |  |
| 496713937 | 2264, 6164 | 3.2 | concrete panels |  |
| 52072930 | 6526, 7642 | 4.8 | concrete panels |  |
| 598164112 | 4945, 4982 | 2.8 | plain road |  |
| 70774846 | 6218, 5254 | 11.3 | concrete panels |  |
| 716596044 | 6976, 5978 | 3.9 | concrete panels |  |
| 74208396 | 6733, 7393 | 3.7 | concrete panels | grown by 8.7 m at the start and 6.9 m at the end; over a road; `bridge_blocks_road` 0.4 m; ends 0.8 m below its junction (`bridge_end_step`) |
| 74208422 | 6744, 7378 | 5.3 | concrete panels | grown by 3.5 m at the start and 3.4 m at the end; over a road; `bridge_blocks_road` 0.6 m |
| 788287708 | 5140, 7452 | 7.8 | concrete panels |  |
| 830320923 | 5574, 5267 | 4.0 | concrete panels |  |
| 861008574 | 5587, 5772 | 3.0 | concrete panels |  |
| 953825999 | 2257, 6046 | 3.3 | concrete panels |  |
| 957061371 | 6548, 7559 | 15.5 | concrete panels |  |
| 957941893 | 2335, 6339 | 3.1 | concrete panels |  |

</details>

## Results with roads on the 8.2 km map at 2048 (2026-11-16, height solve)

The height solve (release binary: all road heights and junctions are one quadratic program, see the manual's getting
started), measured with `road-analysis` against `road-analysis` of the old pipeline at `e82cc74` (both at
`terrain_size = 2048`). The figures are of the commit that applies the meeting grade only where a road really ends on
another, after the clearance under a bridge over a road was put into the same solve with both roads free. The time of
`naves build` and the second game check were measured at `98cfcf4`, before the meeting fix, the first game check at
`55bcdd0`.

| Finding | Before | Now |
|---|---|---|
| `road_grade` | 1079 (751 bends, 328 grades) | 3 (3 bends at held joints, no grade; the grade is now measured against the hard ceiling of the road's family) |
| `road_misses_terrain` | 4625 (1285 over 0.5 m, 142 over 2 m, largest 10.4 m) | 4317 (1204, 163, largest 7.5 m) |
| `road_conflict` | 1025 | 900 |
| `road_not_drivable` | 208 | 189 |
| `road_gap` | 343 | 342 |
| `join_without_junction` | 65 | 65 |
| `junction_too_steep` | 8 | 0 |
| `bridge_clearance` | 1 | 0 (all four bridges over roads reach 4.50 m; Silbergstraße 0.50 m before) |

- **Time:** `naves build` 897 s at `98cfcf4`, before the meeting fix (1775 s before; the roads are placed once, not
  twice). The road analysis (`road-analysis`) takes 921 s (1828 s before). The height solve itself takes 3.0 s, one
  solve of 0.86 s: 97 619 unknowns, 200 125 rows (with the four rows of the clearance under bridges over roads), 17
  iterations.
- **Under the bridges over roads** the lift is shared. The deck against the ground at its two ends, and the road below
  against its ground at the crossing: Silbergstraße +4.78 / +2.12 m and +1.34 m; (7995, 7320) −5.00 / +3.35 m (the deck
  lies across a slope) and −8.52 m; (2347, 6546) +0.24 / −0.62 m and −7.60 m (5.6 m in the old pipeline already);
  (5101, 7913) +3.43 / +0.71 m and −4.75 m. Both roads stay within their grade ceilings within 100 m of every
  crossing, and there is no `road_grade` finding there.
- **How well the road follows its grade aim and the ground** (measured at `55bcdd0`, before this change): 39.3 km of road
  is over its grade aim before, 38.7 km now. Segments over the hard ceiling of their family: 413 before, 5 now. Road
  points more than 4 m off the original ground: 3070 before, 844 now.
- **Bridges over dips:** 3 before, none now. The old profile held the aim and stood high over a dip; the new one is
  allowed to follow the ground into it within the limits of the road, so fewer places stand more than 3 m above the
  ground and none is deep enough for a bridge (dips found: 545 before, 336 now).
- **In the game, second check** (at `98cfcf4`, before the meeting fix): all four roads below the bridges over roads
  are passable, with room under the deck (the bridge at (7995, 7320) is a stone arch and the road passes through it);
  the banks of the cuttings look plausible, the cut edge at (2347, 6546) is a little blocky, and Silbergstraße's
  headroom cannot be seen from the camera. Sites 23, 37 and 14 are unchanged.
- **In the game, first check** (at `55bcdd0`, before the clearance was put into the same solve; DayZ 1.30.164289,
  headless client; the same 11 sites and camera as the previous check):
  - Better: Silbergstraße (site 51; the steep ramp up to the bridge deck is gone and the road runs nearly level onto
    it) and the crossing that had a grade finding (site 37; the V-shaped notch where the dirt road dips into the main
    road is gone). The Silbergstraße pictures show the state at `55bcdd0`, where junction 384 and the road below were
    lifted 4.9 m for the bridge; now the junction is lower and the road below dips under the bridge.
  - Unchanged or slightly better: the Kehlsteinstraße bridge (50), the hairpin (7), the embankment (11), the
    junctions (28 and 29; at 29 the side arm runs further up the hill) and the slight-angle sites (1 and 2).
  - Unchanged: the wooden bridge (23): its deck end still sits slightly proud of the road.
  - At site 28 the truss bridge in the background is gone (a bridge over a dip that is not built any more); at site 14
    no bridge is seen in either picture, and the road follows the ground, with no hole and no wall.
- **Still open:**
  - Deep cuttings under bridges: the road below at (7995, 7320) lies 8.52 m under its ground at the crossing, at
    (2347, 6546) 7.60 m (within their grade limits; no `road_far_from_ground`). The game check found them passable.
  - The thickness of an arch bridge's deck is taken as the lowest vertex of its Geometry LOD within 4 m below the deck,
    anywhere in the model (3.96 m for the stone arch at (7995, 7320)), not at the crown, so the clearance asked for
    under an arch is larger than a vehicle needs.
  - Larger terrain misses (more than 2 m: 142 before, 163 now): the legs of a hairpin on a steep flank are tied to one
    height only where their ground differs by less than the terrain cell allows plus 1 m, so they are not pulled
    together where it differs more.
  - No bridges over dips (above); whether the old bridge sites are now acceptable fills was looked at only at sites 14
    and 28.

## Results with roads on the 8.2 km map at 2048 (2026-11-16, road look)

The road-look work (commit `1363758`, release binary) against the 2026-11-15 results below:

- `naves build`: exit 0, **1775 s** (396 s before; the road height solve, the junction and bridge fits and the second
  placement round cost the time). 24 623 road pieces in 2965 stretches (asf1 12.5 km, asf2 11.0 km, asf3 32.6 km,
  city 75.7 km, grav 66.7 km, mud 59.3 km); 343 open ends. Peak memory was not measured. There are 3 bridges over
  dips: a dip bridge needs about 12.5 m of road within 10 degrees of straight at each end, which is a curve radius of
  roughly 107 m on this map.
- Findings, 2026-11-15 in brackets: 4625 `road_misses_terrain` (4806; 1158 above 0.5 m, 136 above 2 m, largest
  10.4 m), 1079 `road_grade` (1729; 751 bends, 328 grades), 1026 `road_conflict` (1089), 343 `road_gap` (388), 208
  `road_not_drivable` (250; 4.73 km), 65 `join_without_junction` (877), 8 `junction_too_steep` (2), 168
  `path_too_steep`, 82 `earthworks_capped`, 29 `road_in_water`, 21 `tunnel_omitted`, 9 `road_duplicate_dropped`, 3
  `road_over_water`, 1 `bridge_clearance`. Largest cut 43.0 m, highest fill 30.6 m.
- **In the game (DayZ 1.30.164289, headless client):** 51 road sites photographed before the work and at Checkpoint B
  (commit `075198b`), and sites 7, 11, 14 and 28 again at `1363758`.
  - Better at Checkpoint B: 3 (the doubled road is gone), 6 (a stray loop is gone), 14 (the bridge over a dip is
    gone, the road follows the ground), 17 (a smoother diagonal junction) and 43 (the bridge spans the dip).
  - The hairpins at 7 and 11 (Kehlsteinstraße) were lost at Checkpoint B and are back at `1363758`: the roads now
    follow their OpenStreetMap line, and the sites moved to the OSM apex of each hairpin. Site 14 and site 28 are
    unchanged (at 28 a distant bridge looks different).
  - Still open: gaps between pieces at sites 1, 2, 21 and 34, the ski jump at the wooden bridge at 23, and the
    wedges where the road climbs to the lifted decks of the two bridges over another road, at 50 (Kehlsteinstraße)
    and 51 (Silbergstraße). They wait for the joint height solve (`docs/superpowers/plans/2026-11-15-naves-road-look.md`).

## Results with roads on the 8.2 km map at 2048 (2026-11-15)

- `naves build`: exit 0, 396 s, 3.5 GiB, signed PBO 233 MB. 24 815 road pieces in 2984 stretches (asf1 12.6 km,
  asf2 11.1 km, asf3 32.6 km, city 75.9 km, grav 66.9 km, mud 59.3 km); 388 open ends.
- Findings: 4806 `road_misses_terrain` (1345 above 0.5 m, 145 above 2 m, largest 8.5 m), 1729 `road_grade`,
  1089 `road_conflict`, 877 `join_without_junction`, 388 `road_gap`, 250 `road_not_drivable` (5.43 km), 171
  `path_too_steep`, 93 `earthworks_capped`, 25 `road_in_water`, 21 `tunnel_omitted`, 3 `road_over_water`, 2
  `junction_too_steep`. Largest cut 49.2 m, highest fill 54.6 m.
- `binarize` still reports "Cannot open object" for the road models (104 times), so no road piece is drawn in the
  game; see below.

## Results with roads on the 6.4 km map at 2048 (2026-11-14)

- `naves build` (commit `2e2a3e1`): exit 0, 350 s, 1.8 GiB, signed PBO 223 MB. 20 946 road pieces in 2569 stretches
  (asf1 10.5 km, asf2 8.4 km, asf3 26.7 km, city 69.9 km, grav 54.7 km, mud 45.3 km); 76 bridge objects
  (`bridge_metal_25` 34, `bridge_stone_25` 30, `bridge_wood_25` 12, 11 of them scaled by 1.18).
- Findings: 4015 `road_misses_terrain` (1212 above 0.5 m, 129 above 2 m, largest 20.1 m), 1409 `road_grade`,
  1130 `road_conflict`, 740 `join_without_junction`, 343 `road_gap`, 235 `road_not_drivable` (5.05 km), 93
  `earthworks_capped`, 78 `path_too_steep`, 24 `road_in_water`, 21 `tunnel_omitted`, 4 `road_far_from_ground`, 4
  `road_over_water`, 1 `junction_too_steep`. Largest cut 40.9 m, highest fill 68.9 m.
- **In the game (DayZ 1.30.164289, headless client):**
  - The 2048 build **crashes the client while the world loads** (an access violation about a second after the player
    connects). It does so without roads too, and without imagery; the 1024 builds load. The cause, found later: the
    client cannot show terrain cells under 4 m. Náves now refuses such grids, and the map grew to 8.2 km.
  - **No road piece is drawn.** `binarize` reports "Cannot open object" for the road models (104 times; they are not on
    `P:`) and writes no road objects (`Road N dirs: 0`); only the bridges, the terrain benches and the painted ground
    are there. With the game's road models made readable to `binarize` (a test outside Náves) the pieces are drawn.
  - With that test build at 1024: bends, tight R10 arcs, a 180° hairpin, junctions, end caps, the deep cutting, the
    high embankment and a rock face look right. Gaps between pieces show as steps of about 0.5 to 1 m, some bridge
    decks are not joined to their roads, roads that meet without a junction piece leave corners sticking out, and
    steep grades show as humps. The two OSM bridges over another road (Kehlsteinstraße, Silbergstraße) have no
    clearance under them.
- The navigation mesh below was made for the old terrain; regenerate it (`naves build` warns).

## Results with land cover and OpenStreetMap (2026-11-06)

- `naves build`: 15 s, peak memory 1.2 GiB, PBO 50.9 MB, no warnings (the two extracts are merged before the
  polygons are built, so areas that cross the German-Austrian border are whole). `ATTRIBUTION.txt` in the mod folder
  names Copernicus, WorldCover and OpenStreetMap.
- Ground (share of the map): forest 57.9 %, grass 16.8 %, rock 16.8 %, soil 8.5 %. The Königssee and the
  Berchtesgaden basin are soil, the massifs rock. Comparison of the sources: `docs/research/2026-11-05-reference-terrains.md`.
- Headless game client 1.30: the player connects at the east shore (x 3962, z 1400, on the soil-coloured lake bed),
  at x 1500, z 2800 and at x 3600, z 3800; no crash, no navmesh error, and the ground textures match the overlay.

## Results of the first version, procedural ground only (2026-11-06)

- `naves build`: no error. Mod `build/@naves_berchtesgaden`, mission `build/missions/dayzOffline.berchtesgaden`.
- Grids (as again now, after a time at 2048): 1024 terrain vertices (6.25 m), 128 land cells (50 m), 16 × 16 surface tiles (400 m).
- Local DayZ Server 1.30 (own tree, non-default ports, `verifySignatures = 2`, key from `keys/`): loads the mod and
  the mission, `Player connect enabled`, no `Unable to load navmesh`.
- Navigation mesh: `dznav generate`, 16 383 tiles in 749 s, 93 377 reachable polygons.
- Headless game client 1.30: the player connects and stays in the world at a real ground height, for example at
  x 3962, z 1400, height 371.9 m on the east shore of the Königssee. Looking west across the basin shows the Watzmann
  massif with rock above 2100 m and on steep slopes. Near the player the ground has its grass texture and clutter.
- Top-down render of the whole map (`render_topdown`, hillshade): the Königssee runs south from just below the
  centre, the Watzmann ridge lies west of it, the flat Berchtesgaden basin north of the lake, and the terrain is
  continuous across the 13.0 °E tile seam.
- Not tested: walking. A player who spawns at the right height on the terrain is taken as enough for now.
- Still made for Livonia in the mission: `areaflags.map` (no generator yet) and the wildlife territories in `env/`.
- Not in this milestone step: water (the Königssee is a flat basin; with OSM it is soil-coloured), roads, buildings, trees.

How the client crashes and wrong colours on the way to this were found: `docs/research/2026-11-05-reference-terrains.md`.

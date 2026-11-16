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
| 8.2 km | 2048 | | | | 905 s (2026-11-16, height solve; 1775 s road look; 396 s 2026-11-15) | 3.5 GiB (2026-11-15, not measured since) | 233 MB (2026-11-15) |

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

## Results with roads on the 8.2 km map at 2048 (2026-11-16, height solve)

The height solve (release binary: all road heights and junctions are one quadratic program, see the manual's getting
started), measured with `road-analysis` against `road-analysis` of the old pipeline at `e82cc74` (both at
`terrain_size = 2048`). The figures are of commit `982f1d6`, where a junction that holds the road below a bridge keeps
its height; the time of `naves build` and the game check below were measured at `55bcdd0`, before that fix.

| Finding | Before | Now |
|---|---|---|
| `road_grade` | 1079 (751 bends, 328 grades) | 8 (8 bends, no grade; the grade is now measured against the hard ceiling of the road's family) |
| `road_misses_terrain` | 4625 (1285 over 0.5 m, 142 over 2 m, largest 10.4 m) | 4324 (1208, 174, largest 7.5 m) |
| `road_conflict` | 1025 | 893 |
| `road_not_drivable` | 208 | 189 |
| `road_gap` | 343 | 342 |
| `join_without_junction` | 65 | 65 |
| `junction_too_steep` | 8 | 0 |
| `bridge_clearance` | 1 | 2 (Silbergstraße: 0.93 m of 4.5 m, 0.50 m before; (5101, 7913): 3.98 m, 4.50 m before; the other two bridges over roads reach 4.50 m) |

- **Time:** `naves build` 905 s at `55bcdd0` (1775 s before; the roads are placed once, not twice). The road analysis
  (`road-analysis`) takes 862 s (1828 s before). The height solve itself takes 4.2 s: 97 615 unknowns, 200 117 rows and
  17 iterations for the first phase, 97 619 unknowns, 200 146 rows and 19 iterations for the second (it adds the
  clearance under the bridges over roads and holds the road below them).
- **How well the road follows its grade aim and the ground** (measured at `55bcdd0`, before the fix): 39.3 km of road
  is over its grade aim before, 38.7 km now. Segments over the hard ceiling of their family: 413 before, 5 now. Road
  points more than 4 m off the original ground: 3070 before, 844 now.
- **Bridges over dips:** 3 before, none now. The old profile held the aim and stood high over a dip; the new one is
  allowed to follow the ground into it within the limits of the road, so fewer places stand more than 3 m above the
  ground and none is deep enough for a bridge (dips found: 545 before, 349 now).
- **In the game** (at `55bcdd0`, before the fix; DayZ 1.30.164289, headless client; the same 11 sites and camera as
  the previous check):
  - Better: Silbergstraße (site 51; the steep ramp up to the bridge deck is gone and the road runs nearly level onto
    it) and the crossing that had a grade finding (site 37; the V-shaped notch where the dirt road dips into the main
    road is gone). The Silbergstraße pictures show the state before the fix, where the junction and the road below
    were lifted 4.9 m for the bridge; that junction now keeps its height and the bridge stays low.
  - Unchanged or slightly better: the Kehlsteinstraße bridge (50), the hairpin (7), the embankment (11), the
    junctions (28 and 29; at 29 the side arm runs further up the hill) and the slight-angle sites (1 and 2).
  - Unchanged: the wooden bridge (23): its deck end still sits slightly proud of the road.
  - At site 28 the truss bridge in the background is gone (a bridge over a dip that is not built any more); at site 14
    no bridge is seen in either picture, and the road follows the ground, with no hole and no wall.
- **Still open:**
  - Silbergstraße: the bridge's road and the road below share junction 384, which keeps its height (322.80 m on
    320.86 m of ground) with the road below, so the clearance stays at 0.93 m. The bridge at (5101, 7913) reaches
    3.98 m: a junction that holds the road below there keeps its height too.
  - Larger terrain misses (more than 2 m: 142 before, 174 now): the legs of a hairpin on a steep flank are tied to one
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

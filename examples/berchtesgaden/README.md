<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Berchtesgaden test map

The reference project of milestone 1: a 12.8 km square of the Berchtesgaden Alps at half scale (a 6.4 km map).

| | |
|---|---|
| Centre | 47.585 °N, 12.965 °E |
| Area | about 47.527–47.643 °N and 12.880–13.050 °E |
| In it | the Königssee, the Watzmann massif (west of the lake), the Hochkalter group, Berchtesgaden town |
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
naves imagery fetch sentinel2-scenes examples/berchtesgaden   # 100 s, a 4 MB crop of a 340 MB scene
naves imagery fetch bayern-dop40 examples/berchtesgaden       # 5 min, 20 requests, a 160 MB file
```

The scene is dark for a terrain; the build makes the imagery 30 % brighter, and `[imagery] brightness` in `naves.toml`
changes that. Náves evens out the colours of the photographs and the scene where they overlap; the photographs are
much brighter than the scene, and where the kind of ground differs (forest is nearly black in the scene) a soft band
of colour remains at the border to Austria. Without the files, remove the `[[sources.imagery]]` blocks from
`naves.toml`: the satellite image is then painted from the ground classes. With only Sentinel-2, the map is built
from a smaller file and has no band. For a non-commercial map, `eox-cloudless-2025` is the newest cloud-free
mosaic; the map then says that it is non-commercial.

Cost of the satellite image (release binary, `naves build`; same map without imagery in brackets):

| `terrain_size` | Build time | Peak memory | PBO |
|---|---|---|---|
| 1024 | 16 s (15 s) | 1.7 GiB (1.2 GiB) | 75 MB (51 MB) |
| 2048 | 38 s (32 s) | 1.7 GiB (1.2 GiB) | 220 MB (118 MB) |

## Build

```sh
naves build examples/berchtesgaden
```

## Navigation mesh

The mesh is too large for the repository (21 MB). See [Navigation mesh](../../docs/manual/src/navmesh.md) in the manual
for how to make it; put it at `navmesh/berchtesgaden.nm`. The results below were made with one.

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
- Grids: 1024 terrain vertices (6.25 m), 128 land cells (50 m), 16 × 16 surface tiles (400 m).
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

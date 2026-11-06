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

## Fetch the elevation data

The tiles are not in the repository (about 42 MB each). Run this in `examples/berchtesgaden/dem/`:

```sh
B=https://copernicus-dem-30m.s3.amazonaws.com
for e in E012 E013; do
  t=Copernicus_DSM_COG_10_N47_00_${e}_00_DEM
  curl -fO $B/$t/$t.tif
done
```

Data licence and attribution: see "Credits and data licences" in the manual.

## Build

```sh
naves build examples/berchtesgaden
```

## Navigation mesh

The mesh is too large for the repository (21 MB). See [Navigation mesh](../../docs/manual/src/navmesh.md) in the manual
for how to make it; put it at `navmesh/berchtesgaden.nm`. The results below were made with one.

## Results (2026-11-06)

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
- Not in this milestone step: water (the Königssee is a flat green basin), roads, buildings, trees.

How the client crashes and wrong colours on the way to this were found: `docs/research/2026-11-05-reference-terrains.md`.

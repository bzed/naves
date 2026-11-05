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

## Results (2026-11-05)

- `naves build`: 6 s on the development machine, no error. Mod `build/@naves_berchtesgaden`, mission
  `build/missions/dayzOffline.berchtesgaden`.
- Local DayZ Server 1.30 (own tree, non-default ports, `verifySignatures = 2`, key from `keys/`): the RPT shows
  `Loading landscape 'berchtesgaden'`, `Loading landscape took: 91.758000 ms`, `[CE][Hive] :: Init sequence finished.`
  and `Player connect enabled`. Not working yet, as expected: `AI (F): Unable to load navmesh for 'berchtesgaden'`
  (no navigation mesh, so infected and animals cannot path).
- Top-down render of the whole map (`render_topdown`, hillshade): the Königssee runs south from just below the
  centre, the Watzmann ridge lies west of it, the flat Berchtesgaden basin north of the lake, and the terrain is
  continuous across the 13.0 °E tile seam.
- **Not done:** joining with a game client and looking at the terrain. The recipe for a headless client links
  the mod into the Steam `DayZ` folder, which the project rules forbid. Whether the procedural surfaces look
  right in game, and whether a player can walk on the terrain, is therefore still to be checked by hand.

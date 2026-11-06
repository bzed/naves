<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Test fixtures

No game data lives here. Everything is synthetic or openly licensed.

## `dem/*.tif`: Copernicus GLO-30 crops

Small crops of the Copernicus DEM GLO-30 (30 m) near Berchtesgaden, about 15 KB each.

> © DLR e.V. 2010-2014 and © Airbus Defence and Space GmbH 2014-2018 provided under COPERNICUS by the
> European Union and ESA; all rights reserved

| File | Content |
|---|---|
| `koenigssee.tif` | 109 × 73 px around the Königssee (surface at 47.5534 °N, 12.9765 °E is 602 m) |
| `seam_w.tif` | 72 × 73 px, the last columns of tile `N47_00_E012_00` |
| `seam_e.tif` | 73 × 73 px, the first columns of tile `N47_00_E013_00` |

The two seam files meet exactly where the two source tiles meet (Copernicus tiles are centred on whole
degrees, so the seam lies half a pixel west of 13 °E). They test that heights stay continuous across tiles.

Created with GDAL (needs network access; the tests do not):

```sh
B=https://copernicus-dem-30m.s3.amazonaws.com
T12=Copernicus_DSM_COG_10_N47_00_E012_00_DEM; T13=Copernicus_DSM_COG_10_N47_00_E013_00_DEM
CO="-co TILED=YES -co BLOCKXSIZE=64 -co BLOCKYSIZE=64 -co COMPRESS=DEFLATE -co PREDICTOR=3"
gdal_translate -q $CO -projwin 12.96 47.56 12.99 47.54 /vsicurl/$B/$T12/$T12.tif koenigssee.tif
gdal_translate -q $CO -projwin 12.98 47.64 13.00 47.62 /vsicurl/$B/$T12/$T12.tif seam_w_full.tif
gdal_translate -q $CO -srcwin 0 0 72 73 seam_w_full.tif seam_w.tif   # drop the padding column outside the tile
gdal_translate -q $CO -projwin 13.00 47.64 13.02 47.62 /vsicurl/$B/$T13/$T13.tif seam_e.tif
```

Reference values: `gdallocationinfo -wgs84 -valonly koenigssee.tif 12.9765 47.5534` prints 602.

## `landcover/koenigssee.tif`: ESA WorldCover crop

A 360 × 240 px crop (10 m pixels, about 6 KB) of ESA WorldCover 2021 v200 around the Königssee, tile `N45E012`.

> © ESA WorldCover project 2021 / Contains modified Copernicus Sentinel data (2021) processed by ESA WorldCover
> consortium, licensed CC BY 4.0

Class values (`gdallocationinfo -valonly -geoloc koenigssee.tif <lon> <lat>`): 80 (water) at 12.9765 °E 47.5534 °N,
30 (grassland) at 12.9605 °E 47.5595 °N, 10 (tree cover) at 12.9605 °E 47.541 °N.

Created with GDAL (needs network access; the tests do not):

```sh
U=/vsicurl/https://esa-worldcover.s3.eu-central-1.amazonaws.com/v200/2021/map/ESA_WorldCover_10m_2021_v200_N45E012_Map.tif
gdal_translate -q -co TILED=YES -co BLOCKXSIZE=64 -co BLOCKYSIZE=64 -co COMPRESS=DEFLATE \
  -projwin 12.96 47.56 12.99 47.54 $U koenigssee.tif
```

## `osm/koenigssee.osm.pbf`: OpenStreetMap crop

A 38 KB extract of OpenStreetMap around the Königssee: water, rock, built-up and (to test that they are ignored)
forest areas, with the complete ways and multipolygon relations that touch 12.96–12.99 °E, 47.54–47.56 °N.

> © OpenStreetMap contributors, ODbL 1.0, https://www.openstreetmap.org/copyright

Created with `osmium` 1.19 from the Geofabrik extract `oberbayern-latest.osm.pbf` of 2026-11-06:

```sh
osmium tags-filter oberbayern-latest.osm.pbf -o filtered.osm.pbf \
  wr/natural=water,glacier,bare_rock,scree,shingle,wood \
  wr/landuse=reservoir,residential,industrial,commercial,farmyard,military,forest
osmium extract -b 12.96,47.54,12.99,47.56 -s smart filtered.osm.pbf -o koenigssee.osm.pbf
```

`koenigssee_west.tif` is the western half of it (180 × 240 px, `-srcwin 0 0 180 240`), for tests of partial coverage.

`osm/koenigssee_cut.osm.pbf` is that file cut again (`osmium extract -b 12.97,47.545,12.98,47.552 -s simple`): the lake is cut at the border and its rings do not close, as in a real regional extract.

`osm/koenigssee_north.osm.pbf` and `osm/koenigssee_south.osm.pbf` split the lake: each has only the ways that touch
its half (`osmium extract -b 12.9,47.548,13.1,47.7 -s simple` and `-b 12.9,47.4,13.1,47.552 -s simple`), so each alone
cannot close the lake's ring, and the two together can.

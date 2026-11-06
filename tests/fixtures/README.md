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

<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Credits and data licences

## Elevation data: Copernicus DEM

Náves reads the Copernicus DEM GLO-30 (30 m) elevation model, and its test files contain small crops of it.

> © DLR e.V. 2010-2014 and © Airbus Defence and Space GmbH 2014-2018 provided under COPERNICUS by the
> European Union and ESA; all rights reserved

The Copernicus DEM is free to use under the Copernicus data licence. If you publish a map made from it,
keep this attribution with the map.

## Land cover: ESA WorldCover

Náves can read ESA WorldCover (10 m land cover classes) to decide the ground surfaces, and its test files contain a
small crop of it.

> © ESA WorldCover project 2021 / Contains modified Copernicus Sentinel data (2021) processed by ESA WorldCover
> consortium

WorldCover is licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). If you publish a map made
from it, keep this attribution with the map (`naves build` writes it to `ATTRIBUTION.txt` in the mod folder).

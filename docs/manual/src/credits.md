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

## Map data: OpenStreetMap

Náves can read OpenStreetMap extracts to paint water, rock and built-up areas, and its test files contain a small
crop of them.

> © OpenStreetMap contributors, ODbL 1.0, https://www.openstreetmap.org/copyright

The data is licensed under the [Open Database Licence](https://opendatacommons.org/licenses/odbl/1-0/). A map made
from it contains OSM-derived data: if you publish it, keep this attribution with the map, and note that the ODbL
requires the derived database to be shared alike. `naves build` writes the attribution to `ATTRIBUTION.txt` in the
mod folder.

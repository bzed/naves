<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Project file (naves.toml)

A Náves project is a folder with a `naves.toml` in it. `naves new` creates the file; you can edit it by
hand or in the GUI. Náves checks every value when it loads the file and names the field that is out of
range. It saves the file atomically, so a crash never leaves half a file behind.

```toml
name = "Berchtesgaden"
world = "berchtesgaden"

[area]
center_lat = 47.585
center_lon = 12.965
real_extent_m = 12800.0

[scale]
horizontal = 0.5
height = 0.5

[grid]
terrain_size = 1024
land_size = 256

[sources]
dem = ["dem/Copernicus_DSM_COG_10_N47_00_E012_00_DEM.tif"]
```

## Top level

| Field | Default | Valid values |
|---|---|---|
| `name` | given to `naves new` | any text; shown in the GUI and as the map's name in the game |
| `world` | derived from `name` | ASCII identifier, `^[a-z][a-z0-9_]*$`; used for the config class, the PBO prefix and file names |

The default `world` is the name with accents removed, lowercased, and every run of other characters
replaced by one `_`: *Königssee Test* becomes `konigssee_test`, *3 Gipfel* becomes `w3_gipfel`.

## `[area]`

The real-world square that the map is cut from.

| Field | Default | Valid values |
|---|---|---|
| `center_lat` | given to `naves new` | -90 to 90 (degrees, WGS84) |
| `center_lon` | given to `naves new` | -180 to 180 (degrees, WGS84) |
| `real_extent_m` | given to `naves new` | greater than 0; side length of the square in real metres |

## `[scale]`

| Field | Default | Valid values |
|---|---|---|
| `horizontal` | 0.5 | greater than 0 and at most 1; map metres per real metre |
| `height` | 0.5 | greater than 0 and at most 2; multiplier for all heights |

The map is `real_extent_m × horizontal` metres wide. With the defaults, a 12 800 m area becomes a
6 400 m map.

## `[grid]`

| Field | Default | Valid values |
|---|---|---|
| `terrain_size` | 1024 | power of two from 256 to 8192; heightmap vertices per side |
| `land_size` | 256 | power of two that divides `terrain_size`; surface material cells per side |

The distance between heightmap vertices is the map width divided by `terrain_size`.

## `[sources]`

| Field | Default | Valid values |
|---|---|---|
| `dem` | empty | list of paths relative to the project folder |

`dem` lists the elevation files. Absolute paths and paths with `..` are rejected, so a project folder
can be moved or shared. Náves reads single-band Float32 GeoTIFF files in EPSG:4326, for example the
Copernicus GLO-30 tiles.

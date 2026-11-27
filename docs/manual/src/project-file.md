<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Project file (naves.toml)

A Náves project is a folder with a `naves.toml` in it. `naves new` creates the file; you can edit it by
hand or in the GUI. Náves checks every value when it loads the file and names the field that is out of
range. It also rejects options it does not know (a typo, or an option of a newer Náves) and names them, instead of
reading the file and dropping them when it saves it again. It saves the file atomically, so a crash never leaves half a file behind.

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
terrain_size = 2048
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
| `terrain_size` | see below | power of two from 256 to 8192; heightmap vertices per side |
| `land_size` | `terrain_size / 8` | exactly `terrain_size / 8`; surface material cells per side |

The distance between heightmap vertices (the terrain cell) is the map width divided by `terrain_size`.

**The terrain cell must be 4 to 16 m long**, or the DayZ client crashes a few seconds after the player enters the
world. `naves build` refuses such a grid before it starts and names a `terrain_size` that fits the map. The limits per
grid:

| `terrain_size` | Map width |
|---|---|
| 512 | 2 048 to 8 192 m |
| 1024 | 4 096 to 16 384 m |
| 2048 | 8 192 to 32 768 m |
| 4096 | 16 384 to 65 536 m |

So a 6 400 m map (the default scale of a 12 800 m area) is built at 1024 (6.25 m cells, as Livonia); 2048 needs a
map of at least 8 192 m. The reason: the client takes a surface tile (8 × 8 material cells) as its unit of terrain
detail only when the tile is 256 to 1024 m long; with smaller tiles it picks a larger unit from the view distance,
asks for a terrain level that the unit cannot have, and reads a texture that was never made. Vanilla and modded
worlds all have terrain cells of 4 m or more.

A new project gets the finest grid that keeps the cells at 4 m or more (1024 on a 6.4 km map). A project file that
names a size keeps it, and changing the grid is a choice, not an update. `land_size` has no default in the file: a
file that names only `terrain_size` is refused with a message that names the missing `land_size`. A finer grid costs
build time and disk (see the table of costs in `examples/berchtesgaden/README.md`, which has the figures with
roads).

`land_size` is not a free choice: the game needs 8 heightmap cells per material cell, as on Livonia and
Chernarus, and the game client crashes on other ratios. Náves rejects any other value and names the right one;
the GUI and `project_set` of `grid.terrain_size` set it for you.

The surface images are cut into tiles of 8 × 8 material cells, so a map has `land_size / 8` tiles per side
(16 with the defaults). Each tile gets a satellite image and a surface mask of 512 × 512 pixels with 60 pixels per
material cell; a larger `terrain_size` therefore means a sharper surface and a longer build.

## `[sources]`

| Field | Default | Valid values |
|---|---|---|
| `dem` | empty | list of paths relative to the project folder |
| `land_cover` | empty | list of paths relative to the project folder |
| `osm` | empty | list of paths relative to the project folder |

`dem` lists the elevation files; where files overlap the first one that covers a point wins. `naves dem fetch`
adds `dem/mapterhorn_dtm.tif` at the front, so it is used ahead of 1° Copernicus tiles listed after it.
`land_cover` the [ESA WorldCover](https://esa-worldcover.org/) GeoTIFF
tiles and `osm` the OpenStreetMap extracts (`.osm.pbf`) that decide the ground surfaces. Both are optional;
Náves does not download anything. Fetch for example:

```sh
# land cover, a 3° × 3° tile (here N45 E012)
curl -fO https://esa-worldcover.s3.eu-central-1.amazonaws.com/v200/2021/map/ESA_WorldCover_10m_2021_v200_N45E012_Map.tif
# OpenStreetMap, a regional extract
curl -fLO https://download.geofabrik.de/europe/germany/bayern/oberbayern-latest.osm.pbf
```

Absolute paths and paths with `..` are rejected in all three lists, so a project folder can be moved or
shared.

Náves reads single-band Float32 GeoTIFF files in EPSG:4326 (latitude and longitude on WGS84), tiled or in
strips, uncompressed or DEFLATE-compressed, such as the Copernicus GLO-30 tiles. Anything else is
rejected with a message that says why. Files may be neighbouring tiles; heights are interpolated
across the border between them. A spot that no file covers is an error when you build, not a hole in
the map. The data licence is in [Credits and data licences](credits.md).

### `[[sources.imagery]]`

Satellite or aerial images that become the satellite images of the terrain. One block per file, in priority
order: among equally sharp images the first one listed is drawn on top.

```toml
[[sources.imagery]]
path = "imagery/sentinel2-S2C_T33TUN_20250919T101514_L2A.tif"
attribution = "Contains modified Copernicus Sentinel data 2025"
licence = "Copernicus Sentinel data, free and open"

[[sources.imagery]]
path = "imagery/eox-cloudless-2024.tif"
attribution = "EOxCloudless https://cloudless.eox.at by EOX IT Services GmbH (Contains modified Copernicus Sentinel data 2024)"
licence = "CC BY-NC-SA 4.0"
non_commercial = true
```

| Field | Default | Valid values |
|---|---|---|
| `path` | required | relative path inside the project folder |
| `attribution` | required | one line of at most 500 characters, without control characters; written into the finished map |
| `licence` | required | one line of at most 500 characters, for example `CC BY 4.0` |
| `non_commercial` | `false` | must be `true` when the licence contains the word `NC` (for example `CC BY-NC-SA 4.0`) |

## `[imagery]`

| Field | Default | Valid values |
|---|---|---|
| `brightness` | 1.3 | 0.25 to 4; factor on the colours of the imagery (Sentinel-2 images are dark for a terrain) |

Both sections are left out of the file while they are empty or default.

## `edits/overrides.toml`

Not part of `naves.toml`: a file of its own in the project folder, for the changes you make by hand to what Náves
generates. The editor will write it; for now you edit it by hand. A missing file means no edits.

Every object that Náves generates has a name that says where it comes from, such as `road/123:1-5/4` (the fifth
object along the road stretch `123:1-5`; the numbers are OpenStreetMap way and node ids). An edit is pinned to that
name, so a build that generates the map again **keeps your edits**.

```toml
[[edits]]
id = "road/123:1-5/4"
generated_model = "dz\\structures\\roads\\parts\\asf3_25.p3d"
generated_at = [10.5, 20.25]
generated_yaw = 0.5

[edits.edit]
kind = "moved"
x = 12.0
y = 21.0
yaw = 0.5
```

| Field | Valid values |
|---|---|
| `id` | the name of the object |
| `generated_model` | the model the object had when you edited it |
| `generated_at` | `[east, north]` in map metres where it was |
| `generated_yaw` | the rotation the object had, in radians clockwise from north; optional (files without it read as 0). It is where a `replaced` edit stands when its object is gone |
| `edit.kind` | `moved` (with `x`, `y`, `yaw` in map metres and radians clockwise from north), `deleted`, or `replaced` (with `model`) |

Every number must be finite (TOML also allows `nan` and `inf`, which are rejected) and every model name must be
non-empty and ASCII. A file that breaks this is refused when it is read, with a message that names the object and the
field.

If the object an edit was made for is gone, has become another model, or is more than 25 m from where it was, the edit
was made for something else. It is **kept exactly as it is**, as an object of its own, and the build warns about it.
A deletion with no object left to delete is dropped without a warning.

## `edits/ways.toml`

Not part of `naves.toml` either: the roads you drew or changed, in the way OpenStreetMap itself stores them (nodes and
ways). A map without OpenStreetMap files is a map whose roads are all in this file. A missing file means no edits. The
editor writes it; you can also edit it by hand or let an AI client write it through MCP.

```toml
format = 1
next_id = 4

# a node you drew: the program writes `u1` as a user id
[[nodes]]
id = "u1"
lat = 47.5500
lon = 12.9700

[[nodes]]
id = "u2"
lat = 47.5510
lon = 12.9720

# an OpenStreetMap node you moved (`base` is where OpenStreetMap has it)
[[nodes]]
id = 3141592
lat = 47.5502
lon = 12.9710
base = [47.5501, 12.9710]

# a road you drew
[[ways]]
id = "u3"
action = "add"
nodes = ["u1", "u2"]
by = "user"

[ways.tags]
highway = "track"
tracktype = "grade2"
"naves:keep" = "yes"

# an OpenStreetMap road you removed
[[ways]]
id = 8421
action = "delete"
base = "0123456789abcdef"
```

| Field | Valid values |
|---|---|
| `format` | `1` |
| `next_id` | the number the next element you draw gets; greater than every `u<n>` in the file, so ids are never reused |
| `nodes.id` | `u<n>` for a node you drew, an OpenStreetMap node id (an integer) for one you moved |
| `nodes.lat`, `nodes.lon` | WGS84 degrees, never map metres |
| `nodes.base` | `[lat, lon]`, where OpenStreetMap has a moved node; left out for a node you drew |
| `ways.id` | `u<n>` for `add`, the OpenStreetMap way id (an integer) for `modify` and `delete` |
| `ways.action` | `add`, `modify` (the tags and nodes replace the original's) or `delete` |
| `ways.tags` | OpenStreetMap tags of the way after the edit, as for any road (`highway`, `bridge`, `layer`, `surface`, ...); none for `delete` |
| `ways.nodes` | the node ids of the way after the edit, in order; a closed way repeats its first node at the end; none for `delete` |
| `ways.base` | a fingerprint of the OpenStreetMap original (`modify` and `delete`), which the program writes |
| `by` | `user` (default) or `ai`: who made the edit |

**Ids.** An integer is an OpenStreetMap id; `u<n>` (`n` of 1 or more) is an element you drew. A drawn way builds as
`user/way/<n>/<part>`. Every position is a latitude and longitude, so moving the centre or changing the extent or the
scale of the project keeps every edit where it was in the world.

**Edits always win.** The build applies the file to what the OpenStreetMap files hold: moved nodes replace the
positions of that node in every way, `delete` removes a way, `modify` replaces its tags and nodes, `add` inserts a way.
If the OpenStreetMap original of a `modify` or `delete` has changed or is gone since you made the edit, the edit is
still applied and the build reports `edit_original_changed` (see [Troubleshooting](troubleshooting.md)).

**`naves:keep` and `naves:plan`.** `naves:keep=yes` means Náves keeps the line as drawn; the editor gives it to every way you draw or change (write it yourself when you edit the file by hand).
The network clean-up does not remove it or move its nodes, the smoothing and the junction merge leave it alone, and
no lot removes or cuts it. It still gets pieces, heights, junction pieces and earthwork like any road, and a bend that
is too tight for the pieces is reported, not rebuilt. `naves:keep=no` hands the way back to the generator, and
`naves:plan=yes` is for the road planning that is still to come: it will let it improve the line anyway (draw it roughly and let Náves route it); today it does nothing.

**Lots.** A closed way whose tags make a lot (`amenity=parking`, `amenity=tourist_bus_parking`, `highway=rest_area`,
`tourism=camp_site`, `tourism=caravan_site`) is a lot, not a road: `add` draws a car park or camp site, `modify`
replaces the outline of the lot with that way id, `delete` removes it. Give the way a node edit for **every node** of
its outline (a lot's corners are not stored as nodes, so the build cannot find them elsewhere); a node without a
position leaves the way out and the build reports `edit_broken`. A `modify` whose tags are no lot any more removes the
lot. The edited lot has no holes; lots that are relations cannot be edited yet. Lots exist only while the lot stage is
on; a map without OpenStreetMap files has the lots drawn in this file alone. The lot stage never removes or cuts a way
with `naves:keep=yes`: a held road inside a lot, or a held access road, stays as drawn.

Every number must be finite, the counts and lengths are limited, and ids must be as above. A file that breaks this
stops the build with a message that names the file and the element.


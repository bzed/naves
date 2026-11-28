<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# MCP server

```sh
naves mcp <project folder>
```

starts a [Model Context Protocol](https://modelcontextprotocol.io/) server for one project. It talks over
standard input and output, so it is local only: there is no network port. An AI client uses it to read the
project, look at the map and run a build.

The server sends its instructions to every client when the client connects: how the coordinates work (map metres
from the south-west corner), how road ids look and that edits are written at once, without undo.

## Registering it with a client

Most clients take a command and its arguments. For a client that reads a JSON configuration:

```json
{
  "mcpServers": {
    "naves": {
      "command": "naves",
      "args": ["mcp", "/home/you/maps/berchtesgaden"]
    }
  }
}
```

Give the full path of the project folder. The environment variables from [Installation](installation.md)
(`DAYZMAKE`, `STEAM_ROOT`, …) must be visible to the client's process for the `build` tool to find its tools.

## Tools

| Tool | Arguments | Result |
|---|---|---|
| `project_get` | none | the project as JSON (name, area, scale, grid, sources) |
| `project_set` | `path`, `value` | sets one field, for example `{"path": "scale.height", "value": 0.75}`; returns the new project |
| `heightmap_stats` | none | `{min, max, size, cell_m, map_size_m}` of the heightmap |
| `imagery_options` | none | `{area, options}`: the imagery sources for the area of the project with `id`, `name`, `kind`, `covers`, `pixel_m`, `licence`, `attribution`, `non_commercial` and `note` |
| `imagery_fetch` | `id`, `from`?, `to`? | fetches imagery from a catalogue source into the project and lists it in `naves.toml`; returns `{files, warnings}`. Takes catalogue ids only, never an address |
| `render_topdown` | `x0, y0, x1, y1, px, overlay`, `layers`? | a PNG of that part of the map, north up, with the layers drawn over it |
| `roads_list` | `x0, y0, x1, y1`, `limit`? | `{ways, more}`: the roads and paths (OpenStreetMap ways with a `highway` tag, your edits applied) that touch the rectangle, each with `id`, `origin` (`osm`, `edited`, `new`), `tags` and `points`; at most 1000 ways, `more` is true when there are more |
| `road_add` | `points`, `tags` | the new way, as `roads_list` shows it. `points` are `[x, y]` in map metres (two to 2000), `tags` are OpenStreetMap tags and need `highway` |
| `road_modify` | `id`, `points`?, `tags`? | the way after the change; what is left out stays. At least one of `points` and `tags` is needed |
| `road_delete` | `id` | `{"deleted": id}` |
| `road_reset` | `id` | the way as OpenStreetMap has it, or `{"removed": id}` for a way that was added; the edit is dropped, nodes that were moved stay moved |
| `roads_stats` | none | the numbers of the generated roads: `km_by_family`, `pieces`, `stretches`, `open`, `median_deviation_m`, `max_grade`, `max_cut_m`, `max_fill_m`, `not_drivable_m`, `largest_miss_m`, `paths_too_steep_m` |
| `validation_list` | `rule`? | `{violations}`: what is wrong with the roads, each with `rule`, `x`, `y` (map metres) and `message` |
| `build` | none | `{mod_dir, mission_dir, warnings, bridges, progress}` after the build; `bridges` has one line per OpenStreetMap bridge: what it was built from |

**`project_set`** accepts only fields that exist and checks the value like a project file does (see
[Project file](project-file.md)): a wrong type or an out-of-range value is an error that names the field, and
`naves.toml` is left as it was. A value that is accepted is written to `naves.toml` at once, and the cached
heightmap is dropped.

**`render_topdown`** takes the rectangle in map metres from the south-west corner (`x0,y0` south-west,
`x1,y1` north-east), a width `px` in pixels (larger values are reduced to 2048; the height follows from the
rectangle) and an `overlay`: `hillshade`, `slope`, `height`, `surface` (the ground classes in the colours of the satellite
image, decided as the build does it; the first `surface` render reads the project's land cover and OpenStreetMap
files and is cached until the project changes) or `imagery` (the merged images of `sources.imagery`; ground without
imagery is checkered, and a project without imagery files is an error that names `sources.imagery`). Inverted, empty or non-finite rectangles are
errors. The map's size is in `heightmap_stats` as `map_size_m`.

`render_topdown` also takes `layers`, a list of ids drawn over the overlay in a fixed order (the order of the list
does not matter): `terrain_changes`, `too_steep_to_walk`, `roads`, `paths` and `drivable`. An unknown id is an
error that lists the known ones. The layers that show the roads generate them (the first time; the result is cached
until the project or `edits/ways.toml` changes), which needs `dayzmake` and the DayZ Server install; a render without those layers never
does.

**`roads_list`** takes the rectangle like `render_topdown` (an inverted, empty or non-finite one is an error) and
an optional `limit` (1 to 1000, the default; 0 is an error). Points are `[x, y]` in map metres from the
south-west corner, rounded to 0.01 m. Ids are strings as in `edits/ways.toml`: `"812"` for an OpenStreetMap way,
`"u3"` for a way that was drawn. `origin` is `new` for a drawn way, `edited` for an OpenStreetMap way with an edit
or a moved node, and `osm` otherwise. The tool reads the OpenStreetMap files and `edits/ways.toml` but does not
generate the roads, so it needs no tools; a damaged `edits/ways.toml` is an error that names the file. A project
without OpenStreetMap files gives `{"ways": [], "more": false}`.

**`road_add`, `road_modify`, `road_delete` and `road_reset`** change the roads. A point that lies within 5 cm of
an existing node is that node, so a road that starts where another one ends is joined to it (the 0.01 m rounding of
`roads_list` does not matter); any other point becomes a new node, and no OpenStreetMap node is moved. The limits are
2000 points per way, 64 tags of at most 255 characters per key and value, and every point finite and inside the map
(`0` to `map_size_m`). A way that is not `highway`-tagged is refused. An id that is not an id, an unknown way, a way that
is already deleted (`road_reset` brings it back) and a `road_reset` of a way without an edit are errors. Every edit is
written at once to `edits/ways.toml`, marked `by = "ai"`; when a tool reports an error nothing was written, and a damaged
`edits/ways.toml` is never overwritten. There is no undo in `naves mcp <dir>` (the editor has one). The edits are
in the generated roads of `roads_stats`, `validation_list` and the road layers of `render_topdown` at once, but the
game changes only with the next `build`.

**`roads_stats`** and **`validation_list`** generate the roads too. A project without OpenStreetMap files has
empty numbers and no violations, not an error. `validation_list` takes an optional `rule` (`road_not_drivable`,
`road_misses_terrain`, `path_too_steep`, `road_grade`, `road_gap`, `road_kink`, `road_in_water`, `road_conflict`,
`earthworks_capped`, `orphaned_edit`, `join_without_junction`, `tunnel_omitted`, `junction_too_steep`, `road_far_from_ground`, `road_over_water`, `road_duplicate_dropped`, `bridge_clearance`, `bridge_blocks_road`, `bridge_not_built`, `bridge_buried`, `bridge_post_floats`, `bridge_end_step`, `road_roundabout_collapsed`, `road_sidepath_dropped`, `road_stub_dropped`, `road_junctions_merged`, `road_junctions_not_merged`, `bridge_approach_made`, `bridge_end_at_junction`, `road_curve_widened`, `road_curve_too_tight`, `road_curve_planned`, `road_curve_not_planned`, `lot_smoothed`, `lot_surface_only`, `lot_excluded`, `lot_steep`, `lot_move_limited`, `road_in_lot_dropped`, `edit_broken`, `edit_original_changed`, `edit_outside_map`) and then lists only that rule. If
the tools are missing, the error starts with "Roads need dayzmake" and says what is missing.

**`build`** runs the same build as [`naves build`](cli.md), which can take minutes. The result lists the
progress lines the build printed, `planning`, the one-line summary of road planning, `smoothing`, the one-line summary of the road smoothing, `lots`, the one-line
summary of the lot stage, and `edits`, the one-line summary of the hand-made road edits (left out when there are none). `planning` and `smoothing` are `null` while their stages are off, as they are in this release; `lots` is left out of
the result if the lot stage did not run. With road planning on, `planning` has the zones, planned and kept, tight length before and after, largest move and legs steeper than drawn, the numbers in `roads.planning` of `build/report.json`. With the smoothing on, `smoothing` has the chains, largest move, tight length before and after and the zones
reached, the numbers in `roads.smoothing` of `build/report.json`; `lots` has the lots by kind,
the smoothed ones by form and surface, the steep and move-limited ones, the removed and cut roads and the entries lost, the numbers in
`roads.lots`. If a tool is missing, the error names it and the variable to set.

Errors are returned as tool errors with a message (the `isError` flag of the MCP result); the server keeps
running. The heightmap needs DEM files in `sources.dem`; without them `heightmap_stats` and `render_topdown`
say so.

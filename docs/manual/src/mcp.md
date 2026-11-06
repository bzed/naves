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
(`ARMAKE2`, `STEAM_ROOT`, …) must be visible to the client's process for the `build` tool to find its tools.

## Tools

| Tool | Arguments | Result |
|---|---|---|
| `project_get` | none | the project as JSON (name, area, scale, grid, sources) |
| `project_set` | `path`, `value` | sets one field, for example `{"path": "scale.height", "value": 0.75}`; returns the new project |
| `heightmap_stats` | none | `{min, max, size, cell_m, map_size_m}` of the heightmap |
| `imagery_options` | none | `{area, options}`: the imagery sources for the area of the project with `id`, `name`, `kind`, `covers`, `pixel_m`, `licence`, `attribution`, `non_commercial` and `note` |
| `imagery_fetch` | `id`, `from`?, `to`? | fetches imagery from a catalogue source into the project and lists it in `naves.toml`; returns `{files, warnings}`. Takes catalogue ids only, never an address |
| `render_topdown` | `x0, y0, x1, y1, px, overlay` | a PNG of that part of the map, north up |
| `build` | none | `{mod_dir, mission_dir, warnings, progress}` after the build |

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

**`build`** runs the same build as [`naves build`](cli.md), which can take minutes. The result lists the
progress lines the build printed. If a tool is missing, the error names it and the variable to set.

Errors are returned as tool errors with a message (the `isError` flag of the MCP result); the server keeps
running. The heightmap needs DEM files in `sources.dem`; without them `heightmap_stats` and `render_topdown`
say so.

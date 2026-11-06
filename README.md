<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Náves

*Náves – terrain generator and map editor for DayZ.*

**Náves** is a Czech word for the village green, the open space at the centre that a village is built around. The tool does the same for a DayZ map: it starts from real terrain and builds the place around it.

Náves turns OpenStreetMap and elevation data into DayZ terrains. Status: **early development**.
Licence: AGPL-3.0-or-later. See the [design](docs/superpowers/specs/2026-11-04-naves-design.md) and the
user manual in `docs/manual/` (published to GitHub Pages).

## Quick start

```sh
naves new maps/berchtesgaden --name Berchtesgaden --center 47.585,12.965 --extent 12800
# put Copernicus GLO-30 tiles into maps/berchtesgaden/dem/ and list them in naves.toml (sources.dem)
naves build maps/berchtesgaden
```

`examples/berchtesgaden` is a ready project; the manual's *Getting started* chapter walks through it.
`naves gui <project>` opens the editor and `naves mcp <project>` starts the MCP server (see the manual).

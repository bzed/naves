<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Navigation mesh

Infected, animals and AI move along a navigation mesh (a `.nm` file). A terrain without one still loads, but the
server logs `Unable to load navmesh` and nothing walks by itself. Náves does not make the mesh: the game does, through
the data server of `DayZDiag_x64.exe`, and a separate tool turns what the data server sends into the file.
This page describes the steps that were tested on Linux with Proton.

What Náves does for you: the generated world config has a `Navmesh` class that inherits the game's agent size, tile
size and jump and climb link types from Livonia and sets the file name and the seed position (the middle of the
map). **If `<project>/navmesh/<world>.nm` exists, `naves build` packs it into the mod.**

## Steps

1. Build the project once: `naves build <project>`. The mod has to be built with the navmesh class, but no mesh yet.
2. Start `DayZDiag_x64.exe` as a server with the data server switched on, with your mod and an empty mission
   (a mission whose `init.c` has an empty `main()`), for example:

   ```text
   DayZDiag_x64.exe -server -mod=@naves_<world> -startNavmeshDataServer
       -config=serverDZ_navmesh.cfg -mission=<empty mission folder> -profiles=<folder> -port=<free port>
   ```

   Use a server config with `verifySignatures = 0;` and `BattlEye = 0;`, and a port that is not the default. On Linux
   run it through Proton like the game client (see `AGENTS.md`, *Testing a built map in the game*); the mod folder has
   to be visible to the game, which means a symlink in the DayZ install folder.
3. The server listens on port 36533. Generate the mesh with the
   [Flare Navgen](https://github.com/CallMeLewis/flare-navgen) command-line engine, which builds and runs on Linux:

   ```sh
   dznav probe 127.0.0.1                 # shows the world size, tile grid, seed and link types
   dznav generate <world>.nm             # the whole map (about 20 tiles per second)
   ```

   A 6.4 km map has 128 × 128 tiles of 50 m and takes about a quarter of an hour. Flare Navgen is a separate GPL-3.0
   program from another author; it is not part of Náves.
4. Put the file at `<project>/navmesh/<world>.nm` and run `naves build <project>` again.

The server log of the final map should no longer contain `Unable to load navmesh`. Regenerate the mesh whenever the
terrain changes (heights, and later roads, buildings or trees), because the mesh follows the geometry exactly.

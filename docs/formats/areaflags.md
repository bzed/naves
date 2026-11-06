<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# `areaflags.map`

What is known about the file, from looking at the files of the installed server missions and at what a local
server does with changed copies (DayZ Server 1.30, 2026-11-06). Nothing here comes from disassembling a program.

## What the file looks like

The mission folder of every installed map has an `areaflags.map`:

| Mission | Map size | File size | Header | Data |
|---|---|---|---|---|
| `dayzOffline.chernarusplus` | 15 360 m | 83 886 104 | `4096 4096 15360 15360 32 0` | 83 886 080 bytes |
| `dayzOffline.sakhal` | 15 360 m | 83 886 104 | the same | 83 886 080 bytes |
| `dayzOffline.nasdara` | 15 360 m | 83 886 104 | the same | 83 886 080 bytes |
| `dayzOffline.enoch` (Livonia) | 12 800 m | 75 497 496 | `4096 4096 12800 12800 32 0` | 75 497 472 bytes |

- A header of six little-endian 32-bit integers: 4096, 4096, the map width and height in metres, 32, 0.
  Presumably a grid of 4096 × 4096 cells (3.75 m on Chernarus, 3.125 m on Livonia) and the number of flag bits.
- The data is 5 bytes per cell on the 15 360 m maps (4096 × 4096 × 5) but 4.5 bytes per cell on Livonia
  (4096 × 4096 × 4.5). So the size is not simply `cells × bits / 8`. The layout of the data was not worked out.
- The data is almost all zero. In the first 4 MiB, Chernarus has 24 non-zero bytes (all `0x10`); Livonia has 8 851
  non-zero bytes with 12 different values (the most common are `0x02`, `0x10`, `0x01`, `0x04`, `0x20`, `0x03` and
  `0x90`). So the file holds sparse flags.

What the flags mean in the game was not determined. It is likely that they mark the areas that the Central Economy
uses for loot, but that was not checked.

## What the server does with changed copies

Test: the Berchtesgaden mission (6400 m) with a different `areaflags.map`, the mod loaded, a server started on its own
ports, and the log read for 75 seconds. "Starts" means that `Player connect enabled` appears.

| File | Result |
|---|---|
| Livonia's file (12 800 m header, 75 MB), as the generated mission has it now | starts, no message |
| none | starts; warns `File "naves\worlds\berchtesgaden\ce\areaflags.map" does not exist` and `[WorldRes] Cannot open file` |
| empty (0 bytes) | starts, no message |
| header for 6400 m and 4096 × 4096 plus 83 886 080 zero bytes | starts, no message |
| header only (24 bytes) | **the server stops**: `Out of memory (requested -2048 KB)` |
| the first 10 MB of Livonia's file | **the server stops**: the same message |

So the server does not check the header against the map size, accepts a missing or empty file, and stops on a
file that is too short for the grid its header names (a negative size is requested while reading it).

With Livonia's file and with an empty file, the Central Economy starts the same way
(`[CE][LootRespawner] ... Nominal:13701`, `Init sequence finished`). Whether loot is placed differently over a longer
run was not measured.

## Recommendation

Leave the file as it is for now. Livonia's file is the wrong size and holds another map's flags, but the server
takes it, and nothing seen shows that an empty or missing file is harmless for the economy: that needs a longer
test that counts the loot that spawns, with a file and without one. If it turns out to be harmless, `write_mission`
should write an empty file (it saves 75 MB per mission and does not ship another map's data).

Generating the file from `SurfaceGrid` is not possible before the meaning of the flags is known.

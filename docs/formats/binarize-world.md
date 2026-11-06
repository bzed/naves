<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Binarizing a world with DayZ Tools under Proton

Result of the gate experiment of milestone 1, step 1 (`tools/bridge-experiment.sh`, replayable). **It works:**
BI's `binarize.exe` turns our editable world (8WVR, see `naves-formats`) into an `OPRW` world (version 32) when
run under Proton on Linux, and a local DayZ 1.30 server loads the result.

Tested 2026-11-04: DayZ Tools from Steam, Proton Hotfix, bzed's armake2 fork, DayZ server 1.30 (build 127733 data),
flat test world (terrain 256 × 256, land 64 × 64 cells of 25 m, height 100 m, one tile material).

## Result

| Step | Outcome |
|---|---|
| `binarize.exe` | exit 0 in about 3 s, `out/bridgetest.wrp` starts with `OPRW`, version `32` (18 051 bytes for the flat world) |
| `armake2 build` + sign | `naves_bridgetest.pbo` (prefix `naves\worlds\bridgetest`) and a `.bisign`, 57 KB |
| local server (1.30, own ports) | RPT: `Loading landscape 'bridgetest'`, CE init finished, `Player connect enabled` |
| client join and screenshot | **not done**: the skill's recipe links the mod into the Steam `DayZ` folder, which `AGENTS.md` forbids; left for the user |

Warnings that did not stop anything are listed under "Open points".

## Layout the tool expects

Binarize runs in a sandbox that becomes Wine's `Z:`. The project root `P:` is a folder inside it:

```
<sandbox>/                      Z:\
  Binarize/                     copy of DayZ Tools' Bin/Binarize
  prefix/                       STEAM_COMPAT_DATA_PATH (Wine prefix, created by the first Proton run)
  out/                          output folder
  tex/                          (empty, passed as -textures)
  P/                            P:\   project root; also the current directory
    naves/worlds/<world>/
      config.cpp              world config as text (binarize reads it)
      <world>.wrp             the 8WVR
      data/layers/            s_XXX_YYY_lco.paa, m_XXX_YYY_lca.paa, p_XXX-YYY.rvmat
```

Wine's `Z:` must be confined to the sandbox, or binarize walks the whole filesystem:

```sh
export STEAM_COMPAT_CLIENT_INSTALL_PATH=$STEAM STEAM_COMPAT_DATA_PATH=<sandbox>/prefix SteamAppId=830640
"$STEAM/steamapps/common/Proton Hotfix/proton" run cmd.exe /c exit      # creates the prefix
ln -sfn <sandbox>   <sandbox>/prefix/pfx/dosdevices/z:
ln -sfn <sandbox>/P <sandbox>/prefix/pfx/dosdevices/p:
cd <sandbox>/P
"$PROTON" run cmd.exe /c 'Z:\Binarize\binarize.exe -always -textures=Z:\tex -binPath=P:\ P:\naves\worlds\bridgetest Z:\out > Z:\binarize.log 2>&1'
```

Notes:

- Every path has a drive letter and backslashes. The log is only readable through `cmd.exe /c "... > file"`.
- The first run creates the Wine prefix (about 30 s); later runs take seconds.
- Vanilla textures (`dz\surfaces_bliss\...`) did **not** have to exist under `P:` for the world to binarize.
- Nothing is written into Steam directories. Everything lives under `/var/tmp`.

## What binarize needs from the config

`config.cpp` next to the world file, with the world's class under `CfgWorlds` and `worldName` pointing at the
`.wrp` by its in-game path. The minimal config that worked is `tools/bridge-experiment/config.cpp`:

- `CfgPatches` with `requiredAddons[] = {"DZ_Data", "DZ_Surfaces_Bliss"}`,
- `CfgWorldList { class <world> {}; }`,
- `CfgWorlds`: `class DefaultWorld; class CAWorld: DefaultWorld { class Weather { class Overcast { class Weather1; ... Weather12; }; }; };`
  (the forward declarations the game's own world configs carry), then `class <world>: CAWorld` with `access`,
  `description`, `worldName`, `OutsideTerrain`, `Grid`, `centerPosition`, the clutter distances,
  `midDetailTexture`, `terrainNormalTexture`, `heightBlendingMode`, `bicubicMode` and an empty `class Names {}`.

The 8WVR material list holds the in-game path of every tile material, here
`naves\worlds\bridgetest\data\layers\p_000-000.rvmat`; the tile material format is in `terrain-layers.md`.

## Packing

`armake2 build -k <key>` on a folder whose `$PBOPREFIX$` is `naves\worlds\<world>` gives the PBO. It contains the
**binarized** `<world>.wrp` (not the 8WVR), the config (rapified to `config.bin`) and `data\layers\...`.

## Server side

- Mission: `mpmissions/dayzOffline.<world>`; the world is found by its `CfgWorlds` class name. A copy of
  `dayzOffline.enoch` with `init.c`, `db/`, `cfgeconomycore.xml` and a changed `cfgplayerspawnpoints.xml`
  (one position in the middle of the map) started the economy.
- Start with the mod in `-mod=@<world>`, the `.bikey` in `keys/`, and non-default ports.

## Open points

- The log has `Last problem is with texture ...s_000_000_lco.paa` and `...m_000_000_lca.paa` while building the
  primary texture mapping; the world still binarizes and loads. Whether the surface looks right in game is not
  checked (no client run). To check before relying on procedural satmaps: look at the world from a client.
- `OutsideTerrain>>satellite need to have TGA|PNG extension in config` (we used Livonia's outside satellite PAA
  path); harmless for loading.
- Server: no `ceFiles`, no navmesh (`AI (F): Unable to load navmesh for 'bridgetest'`), no `Names`. The server
  keeps running; infected and AI will not path without a navmesh (a later plan).
- Mission files missing from our copy: `cfgundergroundtriggers.json` and `cfgeffectarea.json` give warnings.
  Copying them from the source mission silences these.
- Binarize prints many `No entry '/CfgWorlds/DefaultWorld.*'` lines: values that the game's base config provides.

## Client requirements found later (2026-11-05)

The minimal config above is enough for binarize and for a server, but not for a game client:

- the config needs `ceFiles` pointing at a folder in the PBO that holds `cfgenvironment.xml` (and `env/`); without it
  the client logs `File "\cfgenvironment.xml" does not exist` and crashes;
- it also needs the sky, cloud, horizon and weather entries. Náves gets them by inheriting Livonia's world class
  (`class <world>: Enoch`, `requiredAddons` with `DZ_Worlds_Enoch`) and replaces `Navmesh`, `Holes` and `Names`;
- the player spawn points in `cfgplayerspawnpoints.xml` must be inside a `<group>` below `generator_posbubbles`,
  or the server logs `No points given inside <fresh> configuration` and the player cannot spawn.

Status: with these, a headless client connects and the player is placed on the map, but it still crashes a few
seconds later. The same config with Livonia's own world file works, so the open problem is in the binarized
world; see `examples/berchtesgaden/README.md`.

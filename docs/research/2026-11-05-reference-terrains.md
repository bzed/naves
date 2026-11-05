<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Reference terrains: what Bohemia's sample and the Islands project show

Facts observed on two reference projects while chasing the game-client crash of the generated worlds. Nothing
from these projects is copied into the repository; they were read from scratch folders under `/var/tmp`.

- **Bohemia's Test_Terrain sample** (Utes, <https://github.com/BohemiaInteractive/DayZ-Samples>, explained on the
  wiki page "DayZ: Terrain sample"). Licence: Arma Public License Share Alike.
- **VisTerRains Islands** (`/tmp/islands-map` on the development machine): a source skeleton with configs and Windows
  debug scripts, no world file and no textures.

## The 8WVR of the sample (a real Terrain Builder file)

| Field | Value |
|---|---|
| magic, grids | `8WVR`, land 128 × 128, terrain 512 × 512 (4 terrain vertices per land cell) |
| cell size | 40 m (`0x42200000`), so a 5 120 m map |
| materials | 256 entries (index 0 empty), one per 8 × 8 land cells, named `…\data\layers\P_XXX-YYY_L00.rvmat` |
| objects | 14 891, then the terminator |
| elevation | −103.2 … 80.4 m |

- Our reader parses the file completely, and **reading it and writing it back with our writer reproduces the file
  byte for byte except the 4-byte terminator id**. The 8WVR layout (header, grids, material list, object records)
  is therefore right.
- The terminator is twelve NaNs with the bit pattern `0x7fc87f88`, an arbitrary id (23 260 988 here) and an empty
  model. Náves' reader accepts any all-NaN record with an empty model; its writer uses Terrain Builder's pattern.
  Binarize counts the terminator as an object ("Total 14892 objects" for 14 891), as it does for ours.

## Tile geometry differs from Livonia

The sample's satellite and mask images are 512 × 512 px with 1 m per pixel, so an image tile covers 480 m, and the
materials (`P_XXX-YYY`) cover 8 × 8 land cells (320 m). The tile material for a block of cells samples the right
part of the larger image through its `TexGen3`/`TexGen4` `pos`. Livonia aligns image tiles and material blocks
(400 m, 32 cells). Náves follows the Livonia scheme (block = image tile), which is valid but not the only one.
`pos.y` is `0.03125 + map_size × aside` (30.03125 for Livonia, 10.03125 for the sample); that equals
`tiles × 0.9375` only when the tile count is an integer.

## World config of the sample

It inherits straight from the base `class CAWorld;` with `requiredAddons[] = {"DZ_Data"}` and has no sky or
weather entries of its own, but it does set `ceFiles`, `UsedTerrainMaterials` ("for specular to work"),
`OutsideTerrain` with its own satellite image, `midDetailTexture`, `terrainNormalTexture`, `volFogOffset`, ocean
materials, `Names`, `Navmesh` (via an include), a grid and ILS arrays. The Islands project adds
`requiredVersion = 1.14; // DO NOT USE 0.1`, `worldId`, `initWorld`, `mapsize`, `mapzone` and explicit sky objects,
and says not to set `access = 3`.

## Packaging (from the wiki)

Four PBOs, packed separately with Addon Builder (Binarize and "Binarize all textures" checked, `*.xml` and `*.nm`
copied as they are): `world` (config and wrp), `data` (textures, layer images, materials), `ce` and `navmesh`.
The rvmats of the sample name `.png` files; the texture conversion happens while packing. The mission is a folder
`<name>.<world class>` with an `init.c`, for the sample spawning at `3372 0 3090`. The Windows debug scripts of the
Islands project start the client with absolute `-mod=` paths and with `DayZDiag_x64.exe`.

## What was tried against the headless client (2026-11-05)

A control run of the same client against a vanilla Livonia server works. All of these still crash the client
a few seconds after it enters the world:

| Variant | Result |
|---|---|
| generated Berchtesgaden world | crash, access violation reading `0xffffffffffffffff` (same site every time) |
| generated world with Livonia's satellite and mask images | same |
| a flat 8WVR | same |
| the config's `requiredVersion` set to 1.14 | same |
| generated world with one of Livonia's binarized tile materials instead of ours | player gets a real height (501.8 m) and the crash site moves |
| **Bohemia's sample**, binarized under Proton and packed with armake2, stock Livonia mission or a minimal `init.c` | crash at another site (reading `0x99e11000`) |

So the crash is **not specific to the generator**: the known-good sample fails the same way in this pipeline.
Suspects, in order: the rapified `.rvmat` files (armake2), the texture conversion (armake2 `img2paa` writes no
`AVGC`/`MAXC` tags; Livonia's PAA files have them), and binarize under Proton. A decisive next step is to build the
sample with Bohemia's own Addon Builder; it could not be started under Proton here (no output, exit 255).

## armake2 limitations hit

- the config parser panics on `1e+030` (use `1e30`) and on hexadecimal integer literals such as `0x66ff0000`
  (use decimal; values above 2³¹ − 1 as negative numbers);
- `img2paa` reads PNG only (TGA has to be converted first) and writes no `AVGC`/`MAXC`/`GALF` tags.

## AddonBuilder under Proton works (recipe)

Bohemia's own packer ran headless on the sample. What it needed (all measured 2026-11-05):

- a **throwaway copy of the Steam-initialised tools prefix** (`compatdata/830640`, 684 MB), not a fresh prefix;
- `PROTON_USE_WOW64=1` (the 32-bit .NET tool otherwise stops with "Wine cannot find the FreeType font library");
- a **display**: run it inside a headless `sway` (`WLR_BACKENDS=headless sway -c <config with an exec line>`);
- `dosdevices/p:` → the work drive and the default `z:` → `/`; Proton deletes extra drive links such as `s:` or `t:`
  on every start, so the tool is started by its Steam path `Z:\home\…\DayZ Tools\Bin\AddonBuilder\AddonBuilder.exe`
  (a copy of the tools on a confined `z:` never logged and exited 255);
- **the first start in a prefix hangs until it is killed; the second works**;
- options (from the executable): `<source> <dest> -clear -temp=<dir> -prefix=<addon prefix> -project=P:\
  -binarizeAllTextures [-packonly] [-include=<list>]`; the result code is in `DayZ Tools/Bin/Logs/AddonBuilder.rpt`
  (`[ResultCode]=0`), the exit status of the process is not reliable. A wildcard list passed with `-include=`
  failed with "Illegal characters in path".

Results for the sample: `world.pbo` 1.72 MB, `data.pbo` 64 MB (BI-converted textures), `navmesh.pbo` 15.4 MB, `ce.pbo`
empty without the include list (the XML files were not copied).

| Variant (rest packed by armake2) | Server | Client |
|---|---|---|
| BI `world` | starts | crashes a few seconds after entering the world |
| BI `world` + BI `data` | **segfaults** during mission start after loading `utes_global_nohq.paa` | not reached |
| BI `navmesh`, BI `data` alone | starts | not tried |

The server log also says the sample's navmesh is "version 6, desired version 9": **the 2019 sample is stale for
1.30** and is therefore not a reliable known-good baseline. The stock Livonia mission is also wrong for a
5 km map (contaminated areas at Livonia coordinates are "outside world coords"; the mission's `areaflags.map`
belongs to the map size it was made for).

Open: a baseline that is known to work on 1.30 (a current community terrain with its `8WVR` or its packed PBOs),
or the maintainer's real game client on the Berchtesgaden build, to tell a pipeline problem from an environment one.

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
  (`[ResultCode]=0`), the exit status of the process is not reliable. A list with one pattern per line in the include file
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

## Navmesh regenerated with DayZDiag and Flare Navgen (2026-11-05)

`DayZDiag_x64.exe -server -startNavmeshDataServer` runs under Proton (headless sway, empty mission, mod symlinked into
the install) and serves the terrain geometry on TCP 36533. Flare Navgen's `dznav generate` built the Berchtesgaden mesh
in 747 s: 16 383 tiles (one empty), 94 513 polygons, 20.9 MB, agent height 1.5 m, radius 0.3 m. With the mesh packed to
`navmesh\berchtesgaden.nm` the server no longer logs `Unable to load navmesh`.

**The client still crashes** at the same site (access violation reading `0xffffffffffffffff`, a few seconds after the
join). The missing navmesh was therefore not the cause. A client without the mod folder in the install is kicked with
"Missing PBO from unknown origins", which is a different, harmless failure.

## The same world packed with Bohemia's AddonBuilder (2026-11-05)

The whole Berchtesgaden source tree (PNG layers, text `.rvmat`, `config.cpp`, 8WVR, `ce` XML files, `navmesh`) was packed
in one run with AddonBuilder under Proton (`-binarizeAllTextures`, `-prefix=naves\worlds\berchtesgaden`,
`-include=<file>`; the include file holds one line, `*.xml;*.nm`, and the option takes a file path, not the list). The
result code was 0 and the PBO holds a binarized world, `config.bin`, `texHeaders.bin`, the XML files and the navmesh.
Signed with the project key, the server loads it with no navmesh error, and **the client crashes at the same site**
(access violation reading `0xffffffffffffffff`, 20-30 s after the join). Packing with armake2 is therefore not the
cause either. Logs read: the client RPT shows nothing about our files before the crash (only vanilla noise: missing
sound shaders, `Type 'ManType' must be inherited from class 'DayZAnimalType'`, which the server logs too), the crash
follows the creation of the animal entity types.

## Root cause of the client crash (2026-11-05)

Found with `winedbg` attached to the running client (registers, memory dumps, a hardware watchpoint) and the
disassembly of `DayZ_x64.exe` 1.30.164289.

1. **Heap overflow in the land segments.** The client picks the size of its land segments from the number of land
   cells that share one satellite texture (the surface tile) and fills, per segment, a rectangle of
   (segment + 4) × (terrain cells per land cell) heights into a work buffer of a fixed 128 heights. Livonia: tiles of
   8 land cells, 8 terrain cells per land cell, 96 heights (measured at the same code in a Livonia session). The
   generated world: tiles of 32 land cells, 4 terrain cells per land cell, 144 heights. The 16 extra heights landed
   in the land-segment pointer array next to the buffer, and the client crashed when it used one of them as a
   pointer (the "access violation reading `0xffffffffffffffff`": the bad pointer was two of our own height values).
   With 8-cell tiles the player joins and gets a real height.
2. **Missing terrain level texture.** With 8-cell tiles but still 4 terrain cells per land cell, a terrain render
   job read the texture of a land level that had never been created (null read at `+0xa0`). With 8 terrain cells per
   land cell (`land_size = terrain_size / 8`) it does not happen.

With both (Livonia's geometry) the headless client stays in the Berchtesgaden world. The earlier claim in
`terrain-layers.md` that Livonia has 1024 land cells of 12.5 m and 32-cell tiles was wrong; the binarized
`enoch.wrp` header says 256 land cells of 50 m and a 2048 terrain grid. Bohemia's sample has 4 terrain cells per land
cell too.

## Why the crash-free world looked wrong (2026-11-06)

Two separate faults, each seen in the client and fixed:

1. **Red far terrain.** The satellite PAAs decode correctly (checked with a separate LZO and DXT1 reader; the colours
   match the source PNG), but armake2's `img2paa` writes no `AVGC`/`MAXC` tags. Adding the tags turned the far terrain
   green, but only because the game then drew each tile in its average colour: see item 3.
2. **Wrong near texture.** Grass was red and rock green in the mask, which select slots 1 and 2. Bohemia's sample
   shows the mapping (table in `terrain-layers.md`). With grass black and rock red the ground has its grass texture
   and clutter.
3. **Straight colour edges along the tile grid.** A software render of the same view from the heightmap and the
   satellite tiles put the edges exactly on the 400 m tile grid. With a marker colour in `AVGC` nearly the whole
   terrain took that colour, so the images were not loading. Cause: armake2's `img2paa` appends a byte after the end
   of every LZO stream (`compressed.push(0)` in `src/paa.rs`; `lzo1x::compress` already writes the end marker). With
   that byte removed, every tile shows its own labelled image in the right place. armake2's `paa2img` rejects the
   same files, and it was wrong to dismiss that earlier as a reader problem.

## Land-cover sources for the example area (2026-11-06)

Box 12.8797-13.0503 E, 47.5275-47.6425 N, about 164 km². Both sources were read once for this note; Náves does not
use them yet.

- **ESA WorldCover 2021** (10 m, CC BY 4.0, Cloud Optimized GeoTIFF on S3, the 3° tile is 85 MB): trees 61.5 %, grass
  26.7 %, bare 5.6 %, water 3.0 %, built-up 2.5 %, snow 0.3 %. Every pixel has a class. Stated overall accuracy 77 %.
- **OpenStreetMap** (Overpass, 2348 ways and relations with `natural` or `landuse`): `landuse=forest` 126.6 km²,
  `natural=wood` 34.0, `natural=bare_rock` 31.1, `landuse=meadow` 20.6, `natural=scrub` 9.0, `natural=scree` 7.8,
  `landuse=residential` 6.7, `natural=grassland` 6.2, `natural=water` 5.5. These areas are rough: polygons may extend
  past the box and may overlap, and unclosed relation members were skipped. Coverage is not complete, but rock, scree
  and water are named as such.

The two disagree on rock (5.6 % against up to about 24 % of the box), so a comparison on the map should come before
relying on either one for rock.

## What the two sources say together on the example map (2026-11-06)

`naves build` on the Berchtesgaden example with ESA WorldCover (tile `N45E012`) and the Geofabrik extracts of
Oberbayern and Austria (the south and east of the map lie in Austria). Share of the 6.4 km map per ground class,
from the surface overlay at 1600 px:

| Sources | Grass | Rock | Forest | Soil |
|---|---|---|---|---|
| WorldCover only | 26.8 % | 6.3 % | 61.3 % | 5.6 % |
| OSM only (the rules decide the rest) | 59.8 % | 32.3 % | – | 7.9 % |
| Both (OSM first) | 16.8 % | 16.8 % | 57.9 % | 8.5 % |

The Königssee and the Berchtesgaden basin come out as soil with sharp edges, the lower slopes as forest, and the
massifs as rock.

**Rock from OSM (`natural=bare_rock`, `scree`, `shingle`, `glacier`) stays.** On the Steinernes Meer and the
Watzmann east face, WorldCover shows a ragged mix of grass and rock, because sparse alpine vegetation on karst counts
as grassland. OSM draws the plateau and the east wall as rock down to the lake shore, which is closer to the real
ground. Its edges are coarse: straight polygon sides cut across forest patches. If that looks wrong on a map, drop
`bare_rock` from the table in `crates/naves-core/src/osm.rs` and the WorldCover rock stays.

In the game (DayZ 1.30, headless client, noon, clear weather) the player connects at all three test views, nothing
crashes, and the ground textures match the overlay: soil on the lake bed (x 3962, z 1400), forest floor with clutter
on the slopes, grass and soil patches in the basin, grey rock and green patches on the far ridges. The server log has
no navmesh error (the geometry is unchanged).

Cost of the build (release binary, `naves build examples/berchtesgaden`, with `oberbayern-latest.osm.pbf` 260 MB and
`austria-latest.osm.pbf` 815 MB): 15 s, peak memory 1.2 GiB (1233 MiB). Most of the memory is the nodes of every lake, rock and
built-up polygon in both extracts, which are only known after the whole window test, so it grows with the extract;
a country-sized extract needs a few GiB. The PBO is 50.9 MB. On the Oberbayern extract alone, reading the window takes
0.9 s and 460 MiB. The two extracts are merged before polygons are built; read one by one, 49 rings
were skipped where each extract cuts a polygon at the border, and the build warned every time, because the count
also covered the rest of the extract. Now only rings on the map count, and there are none here.

## The satellite image from Sentinel-2 (2026-11-06)

The example project lists one scene, `S2C_T33TUN_20250919T101514_L2A` (true colour, 10 m, 0.0006 % cloud), which
covers the whole map. `naves build` paints the satellite images from it where it covers a pixel, without baked
shading.

- **Brightness.** The scene is dark for a terrain: mean red, green, blue 44, 53, 34 over the map against 63, 68, 37
  for a Livonia tile's `AVGC`. In the game (DayZ 1.30, headless client, noon, clear weather, x 3962, z 1400, yaw 270,
  over the Königssee towards the Watzmann) the image at `brightness` 1.0 shows the lake, the town and the snow where
  they are, with no seams between tiles and no crash, but the forests are nearly black. At 1.3 the forests and
  meadows read as green and the snow is bright without losing its structure. 1.6 was not tried: the snow is already
  close to white at 1.3. The default is 1.3; a project that sets its own `brightness` keeps it.
- **Cast shadows.** The scene holds real shadows of the Watzmann's east face. They look right in the distance and
  are not a problem from the ground view of the test position; the shift from the far image to the near ground
  textures, which the mask still draws, was not objectionable in this view.
- **What the build found.** The first build with the scene made `binarize` stop (exit 5) while the control build
  without imagery worked. armake2's `img2paa` stored mipmaps that LZO did not shrink as raw data but marked them as
  LZO, so 74 of 512 satellite tiles had a mipmap that no decoder (nor DayZ's `binarize`) could read; a flat image
  never showed it because it compresses everywhere. DayZ Tools' `ImageToPAA` on the same images writes LZO
  streams that end at the `11 00 00` marker, marks only compressed mipmaps and compresses better (mip 0 of a tile
  94 725 bytes against 117 691 for armake2). armake2 is fixed (no stray byte after the stream, mark only what is
  compressed) and Náves clears the wrong mark in files from an older armake2.

## Real areas, one source and two (satellite imagery plan, Task 9, 2026-11-06)

- **One source, whole area.** `naves imagery fetch sentinel2-scenes` for the example: 103 s, 66 MiB, one scene
  (`S2C_T32TQT_20250919T101514_L2A`, zone 32, 0.0003 % cloud) covers the whole map, a 4 MB file is kept of the
  340 MB download. The Imagery overlay and the Surface overlay of the same map line up: the Königssee, Berchtesgaden
  town, the Watzmann's rock and the forest edges are in the same places within a pixel or two.
- **Overlap, the case the merge is for.** `bayern-dop40` over the Sentinel-2 base: 20 requests, 285 s, 353 MiB, a
  160 MB file. The measurement is the mean difference between the merged picture and the Sentinel-2 picture on a
  grid of 100 m cells (the aerial photographs show more detail, so the grid averages them out), over the whole map.
  *Before* (gain limits 0.4 to 2.5, change threshold 25 % of the median): median gain 0.40, 0.43, 0.40, **at the
  lower limit**, 86 % of the cells treated as real change; mean difference 19.3 levels, 39.6 % of the cells more than
  12 and 20.1 % more than 25 levels off, blue 7 levels too high and red 4 too low, so the aerial part looked grey and
  a step stood at the border. The orthophotos are about twice as bright as the scene (mean 93, 105, 94 against 44,
  52, 32) and their blue is three times as bright, which a gain of 0.4 cannot undo. *After* (limits 0.15 to 6,
  the change threshold a factor of 2 in either direction, blur radius 2 cells): median gain 0.34, 0.43, 0.27;
  mean difference **15.2** levels, 15.1 % of the cells more than 25 off. Wider thresholds do better on the number (a
  factor of 2.5: 13.4; no detection: 10) but would stamp a real change into the colours, which
  `changed_content_leaves_no_stamp` forbids. The first change test also could never recognise a cell that is
  *darker* than expected, from a threshold of 100 % up; it is now symmetric.
  **What stays:** a satellite scene and an orthophoto differ in more than exposure: forest is nearly black in the
  scene and not in the photograph, snow and rock the other way round. One smooth gain per 100 m cell cannot follow
  that, and a soft band of colour remains where the photographs end (the border to Austria). A tone curve per
  channel (matching the histograms of the overlap) would be the next step; it is a design decision, not a tuning
  of the values in the plan, so it is **not** done here.
- **A colour cast, found in the game.** In the game the first merged map looked right in forest and meadow but the
  Watzmann's rock face was bright green: the gain per colour channel (0.34, 0.43, 0.27) came from the typical
  ground, the forest, and tinted the grey rock, where the photograph and the scene relate differently. The merge now
  uses **one gain for all three channels** per cell (the geometric mean of the channel ratios): the colours of the
  sharper image are kept and only its brightness is matched. The mean difference from the satellite colours per cell
  is 16.8 levels (15.2 with the tinting gains), 15.7 % of the cells more than 25 off, with a neutral rock face
  (`neutral_ground_stays_neutral`; it failed with the gain per channel).
- **In the game, the merged map** (DayZ 1.30, headless client, noon, clear weather, x 3962, z 1400, yaw 270 over the
  Königssee, and a ground view at x 5200, z 2400 on the Austrian side): the player connects at both views and
  nothing crashes. The photographs show the forest as single trees and the meadows and the rock face as they are; with
  the gain per channel the rock face was green, with one gain it is grey and white like the scene's. No seam between
  tiles can be seen. The soft colour band at the end of the photographs was not looked for at ground level.
- **After the review.** A small real change (a field 1.5 times as bright, not aligned to the cells) left a darker ring
  of 4 levels around it, because only changes of more than a factor of 2 were recognised and the gain was then
  blurred. A cell is now also compared with the median gain of its 13 x 13 neighbours (more than 15 % away is a
  change, and takes that median): sensor differences vary over large areas, real changes from cell to cell. On the
  real map the mean difference from the satellite colours went from 16.8 to **13.4** levels, 15.7 % to **10.8** %
  of the cells more than 25 off (`a_small_real_change_leaves_no_halo`).
- **Non-commercial.** The same map with `eox-cloudless-2025`: `ATTRIBUTION.txt` in the mod and in the mission opens
  with the notice and the share-alike line, `mod.cpp` says "Non-commercial use only", the build warns on the command
  line (the GUI shows the line "Non-commercial map"; the launcher's own display of `overview` was not looked at).
- **Another part of the world.** `naves imagery list` for the Andes (Cusco, -13.5, -71.5): the world sources and no
  Bavaria. The default dates (southern summer, November to March) are the rainy season there: 128 scenes, all above
  20 % cloud, and the fetch stopped with "no usable scene"; it now says that the clearest scene has x % cloud and to try
  other dates. With `--from 2025-06-01 --to 2025-08-31` it fetched `S2A_T18LZL_20250704T150100_L2A` and the Imagery
  overlay shows the dry altiplano. A map across the border of the UTM zones 32 and 33 at 12 °E (47.30 °N) is covered
  by one scene of zone 32, so the seam of two zones is only tested with synthetic files
  (`file_in_the_neighbouring_zone_lines_up`).
- **Cost** (release binary, `naves build` in the headless session, same map without imagery in brackets):
  `terrain_size` 1024: 16.1 s (15 s), peak 1.76 GiB (1.2 GiB), PBO 75.4 MB (50.9 MB); 2048: 38.0 s (31.7 s), peak
  1.76 GiB (1.25 GiB), PBO 220 MB (118 MB). The build is 1.07 and 1.2 times as slow and the PBO 1.5 and 1.9 times as
  large; the plan's limit (twice as slow, or a PBO too large to use) is not reached, but a 2048 map with photographs
  is a 220 MB mod.

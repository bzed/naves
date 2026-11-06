<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Terrain layers: geometry of the satellite, mask and tile materials

Observed on Livonia (`worlds_enoch_data.pbo`, read-only, files extracted to a temporary folder and not kept) and
checked against Náves' own output in the binarize experiment (`binarize-world.md`). Livonia is a 12 800 m map
with a terrain grid of 2048 × 2048 vertices (6.25 m) and a land grid of 256 × 256 cells (50 m per cell), read from
the header of the binarized `enoch.wrp`; Chernarus has the same grids with 60 m land cells.

## What a tile is

The map is cut into square tiles. Per tile the game needs

- a satellite image `s_XXX_YYY_lco.paa` (colour seen from far away),
- a mask `m_XXX_YYY_lca.paa` (which surface layer shows where),
- a tile material `p_XXX-YYY_<layers>.rvmat` that binds both and lists the surface textures of its layers.

`XXX` is the tile column from the west, `YYY` the tile row **from the north** (tile 000_000 is the north-west
corner). All three files of one tile exist in `data\layers\` of the world's data folder. Livonia has 32 × 32 = 1024
tiles of each image.

## Numbers measured on Livonia

| Quantity | Value | How |
|---|---|---|
| tile size in the world | 400 m (12 800 m / 32) | tile count |
| image size | 512 × 512 px (`paa2img` on `S_000_000_lco.paa` and `M_000_000_lca.paa`) | observed |
| overlap | 16 px on every side, so 480 px cover the 400 m | `pos.x` of tile 0,0 is 0.03125 = 16 / 512 |
| pixel size | 400 m / 480 px = 0.8333 m | derived |
| `uvTransform.aside` | 0.0023437499 = 1 / (512 px × 0.8333 m) | observed in every tile |
| tile 0,0 `pos` | (0.03125, 30.031248) | observed |
| tile 1,0 `pos` | (-0.90624994, 30.031248) | observed |
| tile 0,1 `pos` | (0.03125, 29.093748) | observed |

## Formulas

With `n` tiles per side, `E = 480 / 512 = 0.9375` (the part of an image that is the tile itself) and
`O = 16 / 512 = 0.03125` (one side's overlap):

- `pos.x(tx) = O − tx · E`
- `pos.y(ty) = O + (n − ty) · E` (tile row `ty` counts from the north)
- `aside = 1 / (512 · px_m)` and `dir = (0, −aside, 0)`, `up = (0, 0, aside)`
- the material's `TexGen3` (satellite) and `TexGen4` (mask) carry the same transform
- `u = pos.x + aside · x` and `v = pos.y − aside · z` with `x` east and `z` north in metres: on the tile's own
  area `u` runs from `O` to `1 − O`, and `v` from `O` (north edge) to `1 − O` (south edge)

Check: `pos.y(0) = 0.03125 + 32 · 0.9375 = 30.03125`, `pos.y(1) = 29.09375`, `pos.x(1) = −0.90625`.

## Relation to the land grid

Livonia's land cells are 50 m, 8 cells per tile and 60 px per cell, with 8 terrain cells per land cell. Náves
uses the same ratios:

- `land_size = terrain_size / 8`, `cell_m = map_size / land_size`
- `tile_m = 8 · cell_m`, `n = land_size / 8` tiles per side
- `px_m = cell_m / 60`; images are always 512 × 512 with 16 px overlap

These ratios are not a matter of taste. The game client takes the land cells that share one satellite texture (the
tile) as the size of its land segments and asks for (segment + 4) × (terrain cells per land cell) heights per
segment edge, which must fit a fixed buffer of 128 heights: Livonia needs (8 + 4) × 8 = 96. A map with 32-cell
tiles and 4 terrain cells per land cell needs 144, overflows that buffer and crashes the client a few seconds after
it enters the world; with 4 terrain cells per land cell the client also renders a terrain level whose texture was
never created and crashes. Both were found with a debugger on the client, see
`docs/research/2026-11-05-reference-terrains.md`.

## Mask colours

Each mask pixel selects a layer slot of the tile material. Read from Bohemia's Test_Terrain sample, whose material
names show how many slots a tile uses (`P_000-000_L00` uses one):

| Slots in the tile | Colour that appears | Slot |
|---|---|---|
| 1 | black, alpha 255 (the whole mask) | 0 |
| 2 | red, alpha 255 | 1 |
| 3 | green, alpha 255 | 2 |
| 4 | blue, alpha 255 | 3 |
| 5 or more | colours at alpha 128 | 4 and up (the exact colours are not needed yet) |

Livonia's masks use the same colours. Náves uses the first four slots:

| Slot | Class | Mask | Texture (`dz\surfaces_bliss\data\terrain\`) |
|---|---|---|---|
| 0 | grass | `(0, 0, 0, 255)` | `en_grass1` |
| 1 | rock | `(255, 0, 0, 255)` | `en_stones` |
| 2 | forest | `(0, 255, 0, 255)` | `en_forest_con` |
| 3 | soil | `(0, 0, 255, 255)` | `en_soil` |

Slots 2 and 3 are read from Bohemia's sample; check them in the game before relying on them (see the test notes in
`AGENTS.md`).

## Where grass ends

Where neither land-cover nor OSM data says anything, Náves makes ground rock on slopes above 35° and above a real altitude of 2100 m (map
height divided by `scale.height`). The limit is measured: ESA WorldCover 2021 (10 m) against Copernicus GLO-30 on the
Berchtesgaden example area gives trees on 31 % of the ground at 1700 to 1900 m and 2.5 % at 1900 to 2100 m; bare
ground on 27 % at 1900 to 2100 m, 58 % at 2100 to 2300 m and 87 % at 2300 to 2500 m. The limit fits the Alps; other
regions differ.

## PAA tags

Livonia's satellite images carry the PAA tags `AVGC` (average colour, stored blue, green, red, alpha; `25 44 3f ff`
on `S_010_010_lco.paa`) and `MAXC` (`ff ff ff ff`) before `OFFS`; its masks also carry `FLAG` = 1. armake2's
`img2paa` writes only `OFFS`. Náves adds them after the conversion (`naves_formats::paa`), with the average colour of
the source image.

The game draws a tile in its `AVGC` colour until it can read the image, and in dark red without the tag. So when
tiles show flat colours with straight edges along the tile grid, their images do not load. A diagnostic that finds
this quickly: label every satellite tile and set a marker colour in `AVGC`.

armake2 appends one byte to every LZO-compressed mipmap (512 and 256 px), after the stream's end marker `11 00 00`.
The game cannot read such a mipmap, and neither can armake2's own `paa2img`; the raw smaller mipmaps still work, so
only far tiles show their image. Náves removes the byte after the conversion.

## Tile material (`.rvmat`)

Livonia's tile materials have `PixelShaderID = "TerrainX"`, `VertexShaderID = "Terrain"` and

- `Stage0` = satellite image (`texGen = 3`), `Stage1` = mask (`texGen = 4`),
- `Stage2` = the placeholder `#(rgb,1,1,1)color(0.5,0.5,0.5,1,cdt)` (`texGen = 0`),
- `Stage3 … Stage14` = six layers as pairs: the layer's `_nopx` texture (`texGen = 1`), then its `_ca` texture
  (`texGen = 2`); an unused layer has `texture = ""`,
- `TexGen0` identity, `TexGen1` and `TexGen2` scale the surface textures by 10.

The file name lists the layer ids of the six slots (`P_000-000_L00_L01_L02_L10_L09_L04.rvmat`; `N` = unused slot).
Náves names its materials with the plain tile address (`p_XXX-YYY.rvmat`); the name has no meaning for the game
as long as the 8WVR's material list points at the file.

<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Road pieces: what the game files say

Observed on 2026-11-06 for the roads plan, read-only, from DayZ 1.30 `structures_roads.pbo`, Bohemia's `Test_Terrain`
sample (`utes.wrp`, a real Terrain Builder 8WVR) and Livonia's binarized `enoch.wrp`. The tools were throwaway scripts
that scan for byte patterns, not parsers, so **these numbers are evidence for the plan, not a format description**. The
ODOL reader and the gate task of the roads plan have to confirm them.

## Road pieces are not pitched in the world file

This contradicts section 3.2 of the first research note ("they can be pitched/rolled per object but not bent").

| World | Road records | Up axis exactly vertical | Pitch p99 | Pitch max |
|---|---|---|---|---|
| Livonia (road net records in `enoch.wrp`, pattern scan) | 35 730 | 97.9 % | 0.059° | 15.6° |
| Bohemia's sample (8WVR objects below `roads\parts`) | 549 | 100 % | 0° | 0° |

Neighbouring pieces in the sample sit at different heights with no pitch (height step at the join: median 0.27 m; this
is nearest-neighbour matching and includes ends that have no real neighbour). A hilly vanilla map with flat transforms
suggests that **the engine fits road pieces to the terrain when it draws them**, so that the height profile of a road
is decided by the terrain under it, not by the object's transform. This is an inference and is not yet seen in the game.
What the 2.1 % of pitched records in Livonia are was not classified. The bridges are candidates.

`asf1_25.p3d` is a 2229-byte file, and its memory LOD has 4 points. The visual mesh was not read, but a file this small
can only hold few vertices, so a 25 m piece is probably one quad. If so, it spans terrain cells without touching them.

## How the 8WVR stores an object

The 3 × 3 rotation is stored **column by column**: `transform[0..3]` is the image of the model's x axis, `[3..6]` of y,
`[6..9]` of z, then position x, height, y. Only this reading makes the sample's pieces join (median gap 0.34 m over all
ends; read row by row it is 8.1 m). `crates/naves-formats/src/wrp8.rs` documents it as "row by row", which is wrong or
at best misleading.

Model paths are lower case with backslashes: `dz\structures\roads\parts\asf3_10 25.p3d`. The space is part of the name.

## Road piece geometry (memory LOD)

Selections `lb`, `pb` (left and right at the start), `le`, `pe` (at the end), one point each. Left is at negative x, and
the piece runs from negative to positive z. The midpoint of a pair is the join point. The model origin is **not** the
middle of a curved piece, so the numbers must come from the model.

Width by family (`|lb - pb|`, all pieces of a family agree):

| `asf1` | `asf2` | `asf3` | `city` | `grav` | `mud` |
|---|---|---|---|---|---|
| 12 m | 9 m | 7 m | 10 m | 6 m | 5 m |

Pieces per family (same geometry in every family, the origin differs slightly). The heading change is the same size in
both directions: a piece used the other way round turns the other way.

| Name | Heading change | Chord | Radius (derived) |
|---|---|---|---|
| `_6`, `_6konec`, `_6_crosswalk` | 0° | 6.25 m | |
| `_12` | 0° | 12.5 m | |
| `_25` | 0° | 25 m | |
| `_0 2000` | 0.5° | 17.453 m | 2000 m |
| `_1 1000` | 1° | 17.453 m | 1000 m |
| `_7 100` | 7.5° | 13.081 m | 100 m |
| `_10 100` | 10° | 17.431 m | 100 m |
| `_10 75` | 10° | 13.073 m | 75 m |
| `_10 50` | 10° | 8.716 m | 50 m |
| `_10 25` | 10° | 4.358 m | 25 m |
| `_15 75` | 15° | 19.579 m | 75 m |
| `_22 50` | 22.5° | 19.509 m | 50 m |
| `_30 25` | 30° | 12.941 m | 25 m |
| `_60 10` | 60° | 10 m | 10 m |

The first note's guess of 0.75° for `0 2000` was wrong. Two readings failed the sanity check and are unexplained:
`asf1_10 25` and `city_10 25` (the first valid-looking block of floats is not the memory LOD there), and `city_10 100`
(no `pe` found). All three were scan failures, not odd models: the same names read fine in the other families.

## Junctions

`kr_t_<through>_<side>` and `kr_x_<through>_<cross>` are 12.5 m long along the through road and have a side at a right
angle only. Extra selections: `ld`, `lh` for a T (one side road), and `ld`, `lh`, `pd`, `ph` for an X (two side roads).
The origin of a T is not on the through road's centre line (`kr_t_asf3_asf3`: 1.373 m off).

| T | X |
|---|---|
| asf1/asf2, asf1/asf3, asf1/city, asf2/asf2, asf2/asf3, asf3/asf2, asf3/asf3, asf3/mud, city/asf3, city/city, mud/mud | asf1/asf3, asf1/city, asf2/asf3, city/city, city/city/asf3 |

There is no `grav` junction at all, and no junction with a free angle. `kr_x_city_city_asf3` has a different shape
(through road along x), and the T list is not symmetric (`asf3/asf2` and `asf2/asf3` differ).

## How tightly Bohemia joins pieces by hand

Horizontal distance between the join points of neighbouring pieces in the sample:

| Pair | Ends | Median | p90 | Below 2 cm |
|---|---|---|---|---|
| arc to arc | 417 | 9 mm | 22 mm | 87 % |
| arc to straight | 424 | 9 mm | 32 mm | 81 % |
| straight to straight | 190 | 6 mm | 31 mm | 88 % |
| arc to junction | 30 | 8 mm | 14 mm | 100 % |
| straight to junction | 37 | 5 mm | 8.8 m | 76 % |

The 8.8 m probably comes from junction side ports, which this measurement did not count as ends. Bohemia's roads join within a
few centimetres, and Livonia uses the small heading changes heavily (`mudenoch_1 1000` 4287 times, `mudenoch_0 2000`
1994 times).

## Other facts

- Livonia's roads use its own set `dz\structures_bliss\roads\parts\` (`asf1enoch`, `asf2enoch`, `mudenoch`, `quarryenoch`,
  `pathenoch_*`). The junctions, `asf3`, `city` and `grav` are only in the base set.
- Road pieces carry the named properties `class=road` and `map=main road` (`track` for paths). The patch class in
  `config.bin` is `DZ_Structures_Roads`.
- The PBOs are also in the DayZ Server install (`addons/`), which the build already requires.
- Bridges (`Bridge_Metal_25`, `_Stone_25`, `_Wood_25`, `_Wood_50`, in the bliss set `_Concrete1_25`, `_Concrete2_25`) are
  100 to 370 KB, and the scan could not read their memory points. They need a real ODOL reader. Each has a `StaticObj_*`
  class in `config.bin`.

<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Road objects: models and how the game treats them

Two parts: the binarized model files (ODOL) that the road catalog is read from, and how road objects
behave from the world file to the game. The first part is measured and implemented in
`crates/naves-formats/src/odol.rs`. **The second part is not measured yet**: it needs the gate
experiment of the roads plan (Task 1, steps 2 to 7: a test server, a client, vehicles on ramps), which
has not been run. Every statement there is marked as open.

## Part 1: ODOL version 56

Read from DayZ 1.30 `structures_roads.pbo` (265 models), 2026-11-06. The general structure follows the
public `bis-file-formats` reader (MIT); this section lists what was seen in the 1.30 files, all little
endian. Offsets in the examples are of `Parts/asf1_25.p3d`.

### Header

| Field | Type | Note |
|---|---|---|
| signature | `ODOL` | |
| version | u32 | 56. Versions below 58 have neither the app id nor the muzzle-flash string. |
| LOD count *n* | u32 | |
| resolutions | *n* × f32 | 1.0 and up: visual. 10¹³ (stored as 9.999 999 8·10¹²): geometry. 10¹⁵ (stored as 9.999 999 9·10¹⁴): **memory**. 3·10¹⁵: roadway. Compare with a range, not with 1e15. |

### ModelInfo

Fixed fields up to the bounding box (`special`, two spheres, `remarks`, two hint words, aiming centre, two
colours, view density: 60 bytes), then **bounding box min and max** (2 × 3 f32), then the visual box,
bounding centre, geometry centre, centre of mass and the inverse inertia (5 × 12 + 36 bytes).

What version 56 changes, found by lining up 265 models and checking the LOD address table against the file:

- five booleans (auto centre, lock auto centre, can occlude, can be occluded, disable cover);
- `ht_min` (f32), **4 new bytes** (always zero in the files seen), `ht_max`, `af_max`, `mf_max`, `m_fact`, `t_body`;
- `force_not_alpha` (u8), `shadow_buffer_source` (i32), `prefer_shadow_volume` (u8), then 8 bytes
  (the shadow offset and a sibling; zero and `FLT_MAX` in the files seen);
- `animated` (u8); the skeleton: a zero-terminated name, and only if it is not empty a discrete flag, the
  bones (count, then name and parent name each) and a pivots name;
- map type (u8); the mass array (count, then count × f32, stored compressed from 1024 bytes); mass, inverse mass,
  armour, inverse armour (4 × f32); **13 LOD index bytes** (memory, geometry, fire geometry, view geometry and
  nine more); minimum shadow (u32); can blend (u8); class and damage (zero-terminated); frequent (u8);
  an unused u32; **one more byte in version 56**; `has animations` (u8). The reader refuses a model with
  animations (`OdolError::Unsupported`); none of the road pieces has any.

### LOD address table

*n* start addresses, *n* end addresses (u32, absolute), *n* "permanent" bytes. Every LOD that is not
permanent has 25 more bytes of info. The LODs are stored in descending order of resolution.

### A geometry-type or memory LOD, up to the named properties

Proxies (a name, a 3×4 matrix, four ints each), sub-skeleton ints, skeleton map, 56 bytes (vertex count, face
area, hints, bounding box and radius), textures, embedded materials (count, none in these LODs), the
point-to-vertex and vertex-to-point maps (compressed u16 arrays), the faces (count, 6 bytes, then a length and
u16 indices each), the sections, the **named selections** (name; faces; 5 bytes; sections; **vertices** as a
compressed u16 array; weights), the **named properties** (pairs of strings). The memory LOD goes on with key
frames, 17 bytes, the clip flags, the UV sets and the **vertices** (3 × f32 each). A selection's indices point
into these vertices. A "compressed" array is `count`, then `count × element size` bytes, or LZO from 1024 bytes;
the reader refuses to decode LZO, and none occurs in these LODs.

Not parsed: the visual LODs.

### What the road pieces contain

Memory LOD points `lb`, `pb` (left, right at the start), `le`, `pe` (at the end): see
`docs/research/2026-11-06-road-pieces.md`; the reader reproduces its numbers, including the two pieces the first
scan could not read (`asf1_10 25`, `city_10 100`). The class and map properties (`class=road`, `map=main road`)
are in the geometry LOD, not in the memory LOD.

All 265 models of `structures_roads.pbo` are read without an error. A model has memory points unless it
has no memory LOD (most `Decals`, all `Panels`, `Sidewalks` and `Tunnels`, the `path_*` pieces and the two 50 m
wooden bridges) or an empty one (seven `runway_*` pieces). Every road piece, junction and the other four bridges
has them.

### Bridge models

Bounding boxes from ModelInfo (x, y, z), memory points `lb` `pb` `le` `pe` where the model has a memory LOD.
Point heights are above the model origin.

| Model | Bounding box min | max | `lb` | `pe` |
|---|---|---|---|---|
| `Bridge_Metal_25` | -8.84, -12.42, -12.50 | 8.84, 12.42, 12.50 | -6.00, 8.06, -12.50 | 6.00, 8.06, 12.50 |
| `Bridge_Metal_25_1` | -8.89, -12.42, -13.93 | 8.89, 12.42, 13.93 | -6.01, 8.06, -12.68 | 5.99, 8.06, 12.32 |
| `Bridge_Stone_25` | -7.49, -8.90, -12.50 | 7.49, 8.90, 12.50 | -4.47, 7.95, -12.50 | 4.53, 7.95, 12.50 |
| `Bridge_Wood_25` | -16.72, -2.60, -4.55 | 16.72, 2.60, 4.55 | -15.61, 0.06, 2.58 | 15.99, 0.06, -2.57 |
| `Bridge_Wood_50` | -25.87, -2.60, -4.55 | 25.87, 2.60, 4.55 | no memory LOD | |
| `Bridge_Wood_50_1` | -25.87, -3.90, -4.55 | 25.87, 3.90, 4.55 | no memory LOD | |

Observations for the bridge task (Task 9 of the roads plan): the metal and stone bridges are 25 m long along
z with the deck about 8 m above the model origin (the box is not tight: it includes more than the deck). **The
wooden bridges lie along x** (the memory points of `Bridge_Wood_25` run from x = -15.6 to 16.0, deck 5 m wide,
0.06 m above the origin), and **the 50 m wooden bridges have no memory LOD at all**, so no ports can be read
from them. The "25 m" of `Bridge_Wood_25` is not its length. The bridge catalog must therefore take length and
orientation from the points of the model and not from its name, and leave the two 50 m models out.

## Part 2: road objects in the world file and in the game (open)

Open until Task 1 steps 2 to 7 of the roads plan have been run. What is known:

- The 8WVR stores the 3×3 rotation **column by column**: `transform[0..3]` is the image of the model's x
  axis, `[3..6]` of y, `[6..9]` of z, then position x, height, y. `crates/naves-formats/tests/wrp8.rs`
  (`transform_is_stored_column_by_column`) pins this.
- Hypothesis from vanilla data, **not tested in the game**: road pieces have a vertical up axis (97.9 % of
  Livonia's records, 100 % of the sample), so the engine probably drapes them on the terrain. See
  `docs/research/2026-11-06-road-pieces.md`.

To be measured, none done: whether binarize needs the models below `P:`, what the height of an object means,
what a piece does over a crest, whether overlapping pieces flicker, the surface under the wheels, how steep a
vehicle can climb per surface (`DRIVE_GRADE_*`), how steep a player can walk (`WALK_LIMIT_DEG`).

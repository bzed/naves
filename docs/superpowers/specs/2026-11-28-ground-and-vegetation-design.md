<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Náves: ground, climate, forests, hedges and single trees

Date: 2026-11-28. Status: draft for approval. Third of three designs agreed on 2026-11-28; needs
`2026-11-28-osm-store-design.md` (the OSM copy and the feature table) and comes after
`2026-11-28-rail-and-water-design.md` (vegetation keeps clear of tracks and water beds). Extends
`2026-11-04-naves-design.md` sections 5.2 (presets), 5.3 (stages 6 and 8) and the land-cover plan
(`2026-11-06-naves-land-cover.md`); where this document and those disagree, this one wins. Figures are the
Berchtesgaden example, OSM counted on 2026-11-28 (areas once; the counts are approximate), models and ground materials
from the local DayZ 1.30 install read with `dayzmake inspect`.

## 1. Intent

Forest today is only coloured ground; the land-cover plan named trees as "the natural next step". This design adds the
objects that make land look like land: **forests, scrub, hedges, tree rows and single trees**, chosen by forest type
and altitude from DayZ's tree models. It also fixes the **ground** first, because trees stand on it and the grass the
game draws comes from it.

Maintainer's decisions (2026-11-28):

- **Grass/woods generated automatically.** "Check OSM for data about the forest (mixed, needle/leave trees), check the
  height. Match DayZ models to height levels and create a randomized forest. Also take care of hedges and similar
  features."
- **"Ensure grassy areas from OSM are all marked properly."** Today they are not read at all (below).
- **Where OSM has no data on the ground, grass or rock depending on the height** is the fallback.
- **Deserts and dry environments, snow and cold areas** are supported; the Sakhal (Frostline) and Nasdara content is in
  the game.
- **Climate (option b):** the world has **one climate for its weather** (temperate, cold, dry); **ground and trees are
  chosen per place from the data** (snow on the Watzmann under temperate weather).

## 2. What exists today

- `SurfaceClass`: grass (slot 0), rock (1), forest (2), soil (3), tarmac (4). Ground materials from `surfaces_bliss`:
  `en_grass1`, `en_stones`, `en_forest_con`, `en_soil`, `en_tarmac_old`. **Only four slots work in game**: the client
  picks the layer from the mask's colour and ignores alpha, so slot 4 (alpha 128) does not show (game check of
  2026-11-27, `docs/formats/terrain-layers.md`); paved lots are painted soil (`TARMAC_PAINTED = false`). Each tile has
  its own material (rvmat) that names the texture of each slot, so the four slots can hold **different materials on
  different tiles**.
- Decision per cell (`surface::build_surfaces`), first answer wins: an OSM area (only `natural=water`,
  `landuse=reservoir` → soil; `glacier`, `bare_rock`, `scree`, `shingle` → rock; residential, industrial, commercial,
  farmyard, military → soil); the WorldCover class (`landcover::surface_of`: tree cover → forest, shrubland, grassland,
  wetland → grass, cropland, built-up, water → soil, bare, snow, moss → rock; mangroves → forest); the procedural rule (rock above 35° or
  above 2100 m real, else grass).
- **OSM grass, meadow, heath, scrub, farmland and forest tags are not read** (`osm.rs`: "Forest and grass tags are
  deliberately not read: ESA WorldCover draws those edges better"). On the example that is about 520 meadows, 110
  `landuse=grass`, 220 grassland, 260 scrub, 200 forest and 50 wood areas; grass shows only where WorldCover also says
  so.
- WorldCover snow (70) becomes rock: "no snow texture exists among the vanilla surfaces". Out of date: the install has
  `sakhal/Addons/surfaces_sakhal.pbo` (`sakhal_snow`, `sakhal_snow_forest`, `sakhal_grass_brown`, `sakhal_grass_green`,
  `sakhal_forest_birch`, `sakhal_forest_spruce`, `sakhal_gravel`, `sakhal_beach`, …) and
  `nasdara/addons/surfaces_nasdara.pbo` (`nasdara_desert`, `nasdara_desert_gravel`, `nasdara_sand`,
  `nasdara_sand_windswept`, `nasdara_grass_dry`, `nasdara_grass_green`, `nasdara_forest_pine`, `nasdara_stony`,
  `nasdara_soil`, `nasdara_mud`, …).
- `surfaces_bliss` alone has more than Náves uses: `en_grass2`, `en_flowers1/2/3`, `en_forest_dec`, `en_stubble`,
  `en_deforested`; `surfaces` (Chernarus) has `cp_grass_tall`, `cp_gravel`, `cp_conifer_*`, `cp_broadleaf_*`.
- The game draws grass tufts and small plants (**clutter**) from the ground material: each material has a surface
  character that lists clutter models, and the world's `class Clutter` defines them. Náves' world inherits Livonia's
  (`class <world>: Enoch`), so Livonia's clutter exists; Sakhal's and Nasdara's **[verify]** do not unless the config
  adds them.
- Tree and bush models in the install (by folder): `Addons/plants.pbo` (140 trees, 12 bushes, `DZ\plants`),
  `plants_bliss.pbo` (77 trees, 12 bushes, Livonia), `sakhal/Addons/plants_sakhal.pbo` (`winter`, `frozen`, `latefall`,
  `dark` variants of birch, spruce, poplar), `nasdara/addons/plants_nasdara.pbo` (16 trees, 21 bushes: Aleppo pine,
  olive, fig, date palm, oleander, …). Names carry the Latin species: `t_PiceaAbies_3f`, `t_FagusSylvatica_2s`,
  `t_LarixDecidua_1f`, `t_PinusSylvestris_2fb`, `b_corylusAvellana_2s`, …; `_Trail_B/G/R/Y` variants carry painted
  trail markers.

## 3. Goals, non-goals, anti-goals

**Goals**
- Every grassy OSM area is grass of the right kind on the map; every OSM ground tag of the feature table has a ground
  class.
- Where neither OSM nor WorldCover says anything: grass or rock by height and slope (as today), and snow above a snow
  line.
- Snow, sand, desert and dry grass grounds from the data, wherever they are, under one world climate.
- Forests and scrub filled with trees and bushes of the right type for the place: the OSM leaf type where tagged, else
  the altitude band; edges with bushes and young trees; clearings, roads, rails, water, lots and buildings kept free.
- Hedges, tree rows and single trees from OSM.
- Seeded and stable: the same project gives the same trees; an edited tree stays edited.
- A generic **fill mechanism** (inside an area, along its edge, along a line, at a point) that later serves fences,
  power lines and props (spec 5.2 presets): the groundwork item of this design.

**Non-goals**
- Clutter models placed as objects (the game draws clutter from the ground material).
- Seasons (a winter map of a summer place) beyond what the climate setting gives.
- Buildings, fences, power lines, props: their generators come later; this design keeps their ground free where OSM has
  them (buildings, power line corridors) so they fit later.
- Painting vegetation by hand: the editing design's item selector places single trees later; brush tools are milestone
  2.
- Real tree heights or crown sizes from LiDAR or canopy height maps (memory: no finer DEM; global data only).

**Anti-goals**
- Forest that looks planted: regular grids, the same model over and over, hard straight edges.
- A forest in a meadow OSM draws, trees on roads, tracks, in rivers or lakes, or in houses.
- Palms in the Alps, broadleaved trees above the tree line, green grass in a desert.
- A map that the game cannot run smoothly because of the object count.
- Grass where OSM says meadow and the map shows bare soil (the case the maintainer named).

## 4. Constraints

- `AGENTS.md` (as in the other designs).
- No game data in the repo: the tree catalog is read from the install at run time (all four plant PBOs, and the DLC
  folders `sakhal/Addons` and `nasdara/addons`, not only `Addons/`). The **tag file** that says which model is which
  species, size and use holds only model names and our tags, as the building tag file of the spec does.
- **Ground layers per tile: four.** Slots 0 to 3 (black, red, green, blue) are what works in game; a fifth slot by
  alpha was disproven (section 2). So the class set of this design is a **palette per tile**: each tile chooses the
  four classes it needs most and writes their materials into its own rvmat; the other classes on that tile are mapped
  to their nearest class (5.2). **[verify] first** (a spike before the ground plan): that slot materials may differ
  from tile to tile in game (Livonia's per-tile rvmats suggest so; the satellite image hides small seams), how the
  seam between two tiles with different palettes looks, and that mixing materials from `surfaces_bliss`,
  `surfaces_sakhal` and `surfaces_nasdara` in one world works (CfgPatches `requiredAddons`, clutter). A fifth slot
  stays a later finding (`TARMAC_PAINTED`), not something this design needs.
- **DLC content.** The maintainer's statement: the Sakhal (Frostline) and Nasdara content is in the main game, so its
  materials and plants may be used on any map without a fallback. **[verify]** once in game with an account state as
  close to "no DLC" as is available (or from Bohemia's notes), and record the addon names in `requiredAddons`.
- **Object count.** Trees are objects in the world file. The example map is 8.2 km × 8.2 km = 67 km² of map, roughly
  half forest; at 250 trees per hectare that is about 0.8 million trees. The build, the world file, binarize and the
  game must cope; the plan measures it before choosing densities (5.6).
- Scale: trees keep their real size (spec 5.5); densities are per hectare of **map**.

## 5. The design

### 5.1 Climate of the world

`[world] climate = "temperate" | "cold" | "dry"` in `naves.toml`, default `temperate` **[my call]**.

| Climate | World inherits (weather, sky, temperature, snowfall) | Grass | Bare | Forest floor |
|---|---|---|---|---|
| temperate | `Enoch` (today) | `en_grass1/2`, `en_flowers*` | `en_stones` (rock) | `en_forest_con` / `en_forest_dec` |
| cold | `Sakhal` **[verify]** the class name and what inheriting it brings (freezing, snow on objects) | `sakhal_grass_brown/green` | `sakhal_gravel`, rock | `sakhal_forest_spruce` / `sakhal_forest_birch` |
| dry | Nasdara's world class **[verify]** | `nasdara_grass_dry` | `nasdara_desert_gravel`, `nasdara_stony` | `nasdara_forest_pine` |

The climate picks the world class and the **default material for each ground class**; the data picks the ground class
per cell (5.2). Snow and sand are classes of their own and look the same under every climate. Changing the climate of a
map is a setting change and a rebuild; nothing else moves.

### 5.2 Ground classes

The classes grow from five to the set below **[my call]**; each has a material per climate (5.1). A tile uses at most
four (section 4): it takes the four classes with the most cells on it, and maps every other class on it to its nearest
neighbour in a fixed table (flowers → grass, stubble → soil, gravel → rock, snow → rock, sand → soil, conifer floor ↔
deciduous floor, …) **[my call]**; the build reports the share of merged cells per class (`ground_class_merged`). On
most tiles of the example four or fewer classes occur (valley: grass, meadow, forest floor, soil; mountain: rock,
grass, forest floor, snow), so merging is the exception; the plan measures it.

| Class | Source (first match wins, in this order) |
|---|---|
| water bed | lake and river beds (rail and water design), as soil today |
| tarmac, concrete | paved lots (exists) |
| snow | OSM `natural=glacier`; WorldCover 70 (snow and ice); the procedural rule above the **snow line** |
| sand | OSM `natural=sand`, `beach`, `desert`; WorldCover 60 (bare) in the `dry` climate |
| rock | OSM `natural=bare_rock`, `scree`, `shingle`, `landuse=quarry`; WorldCover 60 and 100 (moss and lichen) outside the `dry` climate; the procedural rule (steep or above the grass line) |
| meadow (flowers) | OSM `landuse=meadow`, `natural=grassland` (`grassland=meadow`), `landuse=village_green` |
| grass | OSM `landuse=grass`, `leisure=park`, `garden`, `pitch` (natural surface), `golf_course`, `landuse=cemetery`, `recreation_ground`, `allotments` (as grass **[my call]**), `natural=heath`, `natural=fell`; WorldCover 30 (grassland) and 90 (wetland) |
| scrub ground | OSM `natural=scrub`; WorldCover 20 (shrubland): grass material, scrub fill (5.5) |
| farmland | OSM `landuse=farmland` (`en_stubble`, else soil); WorldCover 40 (cropland) |
| orchard | OSM `landuse=orchard`, `vineyard`: grass material, orchard fill (5.5) |
| forest floor (conifer, deciduous) | the forest outline (5.3): the floor follows the forest type |
| settlement ground | OSM residential, commercial, retail, industrial, farmyard, military: soil today; stays soil until buildings come **[my call]** (grass in gardens comes with the buildings) |
| fallback | grass or rock by height and slope **(maintainer)**, as today (rock above 35° or the grass line of 2100 m real); snow above the snow line |

- **OSM ground tags win over WorldCover** **[my call]**: an OSM meadow is a deliberate drawing, WorldCover is a
  classifier; this is what "marked properly" asks for. The exception is the forest **edge** (5.3).
- **Snow line**: `[ground] snow_line_m` in real metres, default from the climate (temperate 2900 m, cold 1200 m, dry
  4500 m **[my call]**); on the example it does nothing outside WorldCover snow and the three glacier areas.
- **Grass line**: stays 2100 m real (measured on WorldCover around Berchtesgaden); a project setting
  `[ground] grass_line_m` **[my call]** so other mountains can set theirs.
- **Satellite image**: the classes keep their average colours for the procedural satmap; with real imagery nothing
  changes.

### 5.3 Where forest is

- **The outline**: WorldCover tree cover (10 m pixels) is the outline **[my call, as decided in the land-cover plan]**,
  cleaned: clusters smaller than 0.1 ha real are not forest but **tree groups** (the `group` fill, 5.5), holes under
  0.05 ha real are closed **[my call]**.
- **OSM adds** forest where WorldCover has none and OSM has `landuse=forest` / `natural=wood` (newer clearings or
  forests WorldCover misses), and **OSM removes** forest where OSM draws a meadow, grass, farmland, water, a lot, a
  building or a power line corridor inside it (clearings are well mapped).
- **Kept free inside forest** (the fill's exclusion mask, map metres at game size from the edge of each, since they keep game objects clear): roads (half width + 3 m),
  paths (1 m), rails (bed + 4 m), water beds (bank + 2 m), lots, OSM buildings (footprint + 3 m), power lines
  (`power=line`: 15 m each side; `minor_line`: 4 m), aerialways (10 m), ski pistes (`piste:type=downhill` areas and
  ways, 15 m) **[my call]** on all figures. This mask is shared with the later generators.
- **Slope**: no trees above 45° (rock faces) **[my call]**; between 35° and 45° the density halves.

### 5.4 Which trees

**Type of a forest area**, per cell, first answer wins:
1. OSM `leaf_type` (`broadleaved`, `needleleaved`, `mixed`, `leafless`) of the OSM forest or wood area the cell lies
   in; the old tag `wood=coniferous|deciduous|mixed` the same.
2. The **altitude band** of the cell's real height (project setting, defaults below).
3. Nothing (no OSM area, no band): the band's default.

Only about 15 % of the example's forest areas carry a leaf type, so the bands decide most of it.

**Altitude bands** (real metres, `[[vegetation.band]]` in `naves.toml` or a shareable preset file; defaults per
climate **[my call]**, the temperate ones fit the northern Alps):

| Temperate band | Mix (share of trees) |
|---|---|
| below 800 m | broadleaved 60 % (beech, oak, ash, maple, birch), spruce 30 %, pine 10 % |
| 800 to 1400 m | mixed: spruce 50 %, beech 35 %, larch 10 %, others 5 % |
| 1400 to 1800 m | spruce 70 %, larch 30 % |
| 1800 to 2000 m (tree line) | larch and spruce, sparse (density 30 %), bushes (dwarf birch, small spruce) as the dwarf-pine belt |
| above 2000 m | none |

Cold: birch and spruce (Sakhal's models), `winter`/`frozen` variants on snow cells. Dry: Aleppo pine and olive in
forest, fig, oleander, date palm only near water and settlements **[my call]**. A `leaf_type` from OSM picks the
band's broadleaved or needleleaved part only.

**The tag file** (`data/vegetation-tags.toml`, in the repo, names only **[my call]** on the path): per model name:
`species` (Latin, from the name), `leaf` (broadleaved, needleleaved), `form` (tree, bush, dead, stump), `size`
(1 small, 2 middle, 3 large; from the digit in the name **[verify]** against the models' bounding boxes), `stand`
(`f` forest, `s` solitary, `fb`/`sb`… variants; **[verify]** what the letters mean by looking at the models),
`climate` (temperate, cold, dry), `variant` (winter, frozen, late fall, dark, trail marker). A model the install has
but the file does not know is reported and not used; a model the file names but the install lacks is skipped. The
build reads the bounding box of each model it uses (as for road pieces) for spacing and the exclusion mask. The first
version of the file is made by name rules plus a look at each model (MCP render or in game), as the spec proposes for
buildings.

### 5.5 How they are placed: fills

A **fill** turns a feature into objects (spec 5.2 presets): `inside` (an area), `edge` (a band along an area's edge),
`along` (a line), `point`. It has a weighted model list, a spacing (minimum distance between objects), a density, a
slope limit, a size mix and the exclusion mask. Fills are data **[my call]**: built-in defaults per climate, a project
may override them (`[[fill]]` in `naves.toml` or a preset file), so a later fence or power line generator is a fill
with other models.

**Units.** Trees keep their real size (spec 5.5), so everything that is about the distance between objects is in **map
metres** (spacing, row and hedge intervals, densities per hectare of map); everything that is about the land is in
**real metres** and converted with `scale.horizontal` (edge band width, cluster and hole sizes,
riparian strip). A real tree row 400 m long becomes 200 m on a 1:2 map and keeps 8 m between its trees: half as many
trees, not a wall **[my call]**. Clearances to game objects (the exclusion mask, 5.3) are map metres, as in the rail and water design. A spacing is never smaller than the crowns of the two models allow (bounding boxes).

**Density field.** A fill's density is a base value times a factor that may vary with place: distance to the forest
edge, distance to roads, rails and settlements, slope **[my call]**. The defaults are flat (factor 1) except the
slope rule; the field exists so that a low object budget (5.6) can keep the forest dense where players see it (along
roads, edges, near settlements) and thin it in the interior, if the measurement calls for it.

Placement: **Poisson-disc sampling** (minimum distance, seeded) per map tile of 256 m **[my call]**, each tile's seed
from the project seed and the tile index, points near a tile border checked against the neighbour's (so tiles join
without seams and a change in one tile changes only it and its neighbours). Each point then draws a model from the
band's mix (weighted), a random rotation, a small random scale (0.9 to 1.1 **[verify]** that tree scaling looks right
in game) and a small random tilt only for bushes.

**Identity and edits.** Identity: `veg/<feature>/<tile>/<n>`, stable across rebuilds while the tile's inputs do not
change. When they change (a road nudged, a density change, an OSM refresh), the Poisson points of the tile and its
neighbours are drawn anew and `n` no longer names the same tree. So an override of a tree is matched **by id first,
then by place** **[my call]**: if the tree with that id is not where the edit was made (further than 3 m map), the
edit takes the nearest generated tree of the same model within 3 m; failing that, the spec's orphan rule applies (the
edit is kept exactly as it is: a moved or re-typed tree becomes a placed object, a deleted tree becomes a small
"no tree here" exclusion of 3 m, with a warning). The editing design's "cut out" area tool (Vegetation level, R) is an
exclusion area, independent of ids, and survives any change.

| Fill | Where | Spacing / density (map) | Models |
|---|---|---|---|
| forest inside | forest outline | 6 m minimum, about 250 trees per ha (tuned in 5.6) | the band's trees, sizes 3:2:1 = 50:35:15 %, `f` stand |
| forest edge | the outer 15 m real of every forest outline | denser bushes, trees of size 1 and 2, `s` stand | the band's bushes and small trees |
| undergrowth | forest inside | 1 bush per 200 m² | the band's bushes |
| scrub | scrub ground | 3 m, bushes; a tree per 400 m² | bushes, small trees |
| orchard | orchard | rows 8 m apart, 6 m in the row, along the area's long axis | fruit trees (apple, pear, plum, walnut) |
| tree row | OSM `natural=tree_row` | every 8 m along the line | solitary trees of the leaf type, else the band's broadleaved |
| hedge | OSM `barrier=hedge` | every 1.5 m along the line, alternating sides by 0.3 m | hedge bushes (hazel, blackthorn, dog rose, hawthorn, elder), `height` picks bush sizes |
| tree group | WorldCover tree clusters under 0.1 ha real (5.3) | forest inside density, at least one tree | the band's trees, `s` stand |
| single tree | OSM `natural=tree` | at the node | by `species`/`genus` if the tag file knows it, else `leaf_type`, else the band; `s` stand, size 3 |
| riparian | 10 m real along rivers and streams outside forest | sparse (1 per 150 m²) | willow, poplar, alder-like (`salixAlba`, `populus*`) |

All figures **[my call]**; the riparian strip and the orchard are what the research doc listed ("riparian strips",
"orchards"). Trail-marker trees (`_Trail_*`) are left for later (they would go along marked hiking routes: OSM route
relations).

### 5.6 Object budget

Before densities are fixed, the plan measures on the example: build time and memory with 0.25, 0.5 and 1 million trees;
world file size; binarize time under Proton; the game's frame rate at three fixed views (tool `tools/ingame-check/`).
The default density is the largest one that keeps the map playable, compared with a Livonia forest at the same view
**[my call]**. The build reports trees per km² and per fill (`vegetation` in `report.json`). A project setting
`[vegetation] density = 1.0` scales all fills.

### 5.7 Stages

```
… → earthwork (roads, rails, water) → ground classes (5.2) → forest outline and exclusion mask (5.3)
  → fills (5.5) → surfaces (the floor under forest follows its type) → checks → export
```

Vegetation comes after the earthwork **[my call]**: trees stand on the final ground, and the exclusion mask knows the
final road, rail and water beds.

### 5.8 Checks

- `tree_on_road`, `tree_in_water`, `tree_on_rail`, `tree_in_building` (the mask failed: should never fire);
- `tree_above_tree_line` (a model above its band);
- `vegetation_model_missing` (the tag file names a model the install lacks), `vegetation_model_untagged` (information);
- `ground_class_merged` (5.2), `ground_material_missing` (a climate material the install lacks, for example a missing
  DLC folder);
- measures: forest share, trees per ha per band, share of forest cells typed by OSM and by band.

## 6. Testing

- Ground: one test per table row of 5.2 (an OSM meadow over WorldCover cropland is meadow; WorldCover snow is snow; a
  `dry` climate turns WorldCover bare into sand); the fallback; old projects build the same ground as before for the
  classes that exist today unless OSM grass now applies (the change is the point; the regression test pins the new
  result on a synthetic project).
- Fills: Poisson-disc spacing holds; seeded output is identical across runs; a tile's change moves only that tile and
  its neighbours; exclusion mask (no tree on a synthetic road, rail, river, lot, building); bands pick the right models;
  `leaf_type` wins over the band.
- Tag file: parsing (fuzz target, seeds run in `make test`); an unknown model is reported.
- Real catalogs only in `#[ignore = "needs DayZ"]` tests.
- In game: the three views of 5.6; a meadow in a forest clearing; the tree line on the Watzmann; snow on a glacier; a
  hedge; a single OSM tree; a cold and a dry test map (a small synthetic area under `cold` and `dry`) to check the
  inherited world class, the materials and clutter.

## 7. Left to the builder

- The exact ground class list (names, merging rules), the materials per climate after the slot spike.
- Poisson-disc implementation (crate first: `fast_poisson`, `poisson-diskus` and others, reviewed).
- Fill data format and where presets live.
- The tag file's first content and the name rules.
- Tile size of the placement.
- Whether hedges use bushes or a hedge model if the install has one (`DZ\plants` has none by name **[verify]**).

## 8. Designer's calls to check

1. `climate` setting with three values; temperate default; the world class per climate (5.1).
2. OSM ground tags win over WorldCover, except the forest edge (5.2, 5.3).
3. More ground classes, rarest merged per tile when slots run out (5.2).
4. Snow line defaults 2900 / 1200 / 4500 m; grass line stays 2100 m and becomes a setting (5.2).
5. Settlement ground stays soil until buildings come (5.2).
6. Forest outline from WorldCover, cleaned at 0.1 ha / 0.05 ha (5.3).
7. Exclusion distances (5.3), no trees above 45° (5.3).
8. Altitude bands and mixes (5.4); dry-climate palms only near water and settlements.
9. Fill figures (5.5); riparian strips and orchards included; trail-marker trees later.
10. Density chosen by measurement, not fixed now (5.6).
11. Vegetation after the earthwork (5.7).
12. Four classes per tile as a per-tile palette, rarest merged to the nearest class (4, 5.2).
13. Spacing in map metres, land sizes in real metres (5.5).
14. Tree edits matched by id, then by place within 3 m, else the orphan rule (5.5).
15. A density field with flat defaults (5.5); small tree clusters get a group fill (5.3, 5.5).

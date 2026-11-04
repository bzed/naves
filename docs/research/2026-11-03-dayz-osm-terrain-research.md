# DayZ terrains from OpenStreetMap: research notes

Date: 2026-11-03. Status: research, not a design. Nothing here is decided except what the
"Decisions so far" section says.

## Goal (as stated)

A tool like [ArmaRealMap / GameRealisticMap](https://github.com/jetelain/ArmaRealMap), but better
and for DayZ: generate a playable terrain from OSM + elevation data, with roads, footpaths,
railways, buildings and other objects placed automatically, using every DayZ object and, where
possible, objects from mods. Roads must use fitting road pieces so they are not clunky, and must
be fitted to the heightmap (OSM and elevation disagree on steep ground).

## Decisions so far (from the user)

- Written in **Rust**. Linux first, Windows a bonus.
- **No dependency on BI's Windows tools in the end state.** Terrain Builder, Buldozer and the
  other DayZ Tools need replacements. Clean-room reverse engineering and re-implementation where needed.
- **Binarizing without Proton goes into bzed's armake2 fork** (not a separate tool).
- **Navmesh:** [flare-navgen](https://github.com/CallMeLewis/flare-navgen) exists (GPL-3, needs DayZDiag's
  navmesh data server; DayZDiag is expected to run under Proton like DayZ). Long term: reverse engineer
  the `.nm` path too.
- Overlapping road pieces is allowed, especially straights.
- Road fitting can be an optimization problem solved per stretch from one crossing to the next.

## Verdict in one paragraph

Feasible. The automatic part (roads, rails, fences, buildings, vegetation from OSM) is a
hard-but-tractable algorithm problem; a throwaway spike already closes half of all
crossing-to-crossing road stretches exactly using only vanilla road pieces, and finds a vanilla
model within ±15% per side for 97% of 2295 real OSM building footprints (numbers below). The
expensive part is not the generation, it is everything around it that BI's Windows tools do today:
the binarized world format (OPRW **v32**; a public reader with a full layout exists since Oct 2026, no writer yet), model
binarization, the terrain texture/rvmat pipeline, the 2D map, and the navmesh. Those are the long
poles and the real reverse-engineering work.

---

## 1. What exists already

| Thing | What it does | What to take from it |
|---|---|---|
| **ArmaRealMap / GRM** (C#, .NET 10, ~1050 files, beta 0.11, active) | OSM + SRTM + Sentinel-2 -> Arma 3 terrain, WPF studio | Architecture (lazy typed builder DAG), the list of ~60 data layers, building fitting, "default area" fill, region asset libraries, terrain constraint ideas. Not its road output (Arma 3 renders roads from `roads.shp`; DayZ does not). Not its elevation solver (heuristic regression, self-described as mediocre). Uses BI `binarize.exe` (Windows) for WRP. |
| **[Big G' Terrain Tools](https://dayzmodcast.com/biggtoolz/qgis/)** (QGIS plugin, GPL-3, beta) | Data prep: Copernicus GLO-30/SRTM, imagery, OSM -> heightmap `.asc`, satmap, surface mask, `roads.shp`, `buildings.shp`, TB setup guide | Copernicus GLO-30 as default DEM; its OSM-tag -> surface mask mapping. It stops where our tool starts: placement and baking are still manual in Terrain Builder. |
| **[flare-navgen](https://github.com/CallMeLewis/flare-navgen)** (GPL-3, early, Recast/Detour based) | DayZ `.nm` navmesh, byte-identical tiles vs BI reference for same geometry | Use as the navmesh step now. Its docs/validation are a free description of the `.nm` format for later RE. Geometry comes from DayZDiag, so it needs the game running (Proton). |
| **[HEMTT](https://hemtt.dev)** (Rust) | Arma 3 build tool: PBO, rapify, PAA, P3D inspect | Possible crates for PBO/PAA/config. Its binarize also shells out to BI's exe under Wine. |
| **bzed/armake2** (Rust, fork) | PBO pack/sign, rapify/derapify, paa2img/img2paa, `--proton-binarize` | The home for native binarize (decided). Already used to read every game file in this research. |
| **[bis-file-formats](https://github.com/jetelain/bis-file-formats)** (C#, MIT) | Readers/writers for WRP (OPRW up to v27 + 8WVR), P3D (MLOD/ODOL), PAA, PBO, RTM | Best public reference for the formats. MIT, so reading it is fine. Does not know OPRW 28-32. |
| [uksf/bis-file-formats](https://github.com/uksf/bis-file-formats) (C#, MIT) | Fork of the above (~300 commits, Jun–Nov 2026). Adds a naive MLOD→ODOL writer (v73, Arma 3), ODOL→MLOD, MLOD writer fixes, PBO linter/deobfuscator, Enfusion PAK/EBO readers, BISign, SQF/stringtable linters | WRP code unchanged (still no OPRW 28-32). The ODOL writer fills ModelInfo with zeros/defaults and leaves proxies, animations and geometry-LOD processing empty, so it is not a binarize replacement. Useful only as a field-order cross-check. It replaced the Braini01 copyright line, so take code from upstream. |
| **[dayz-wrp-reader](https://github.com/DevilinJake/dayz-wrp-reader)** (Python, MIT, Oct 2026) | Reader/exporter for DayZ OPRW **v32**: objects, classed models, road net, heightmap, ENFI surface grid, geography, map info | Read-only, ~840 lines. Gives the v32 layout we lacked (see section 2). Credits arma-tools/arma-file-formats-rs (Rust), which is worth a look for our Rust crates. |
| **BeamNG generators** ([TerraDrive](https://boxthislap.org/beamng-drive-real-world-map-generator-terradrive/), [MapNG](https://mapng.com), [EarthNG](https://github.com/Alezx311/earthng/wiki)) | OSM -> BeamNG levels | Prior art for road reconstruction; users report the same road/terrain bumps we want to avoid. |
| **Real World Terrain** (Unity asset) | OSM -> Unity terrain + objects | Feature list prior art only. |

No OSM -> DayZ generator exists. No Rust WRP reader/writer exists.

---

## 2. What the game data says (read-only recon of the local DayZ 1.30 install)

All read with `armake2 inspect/cat/derapify` into the session scratchpad; nothing written to Steam dirs.

**World file.** `DZ\worlds\enoch\world\enoch.wrp` (Livonia, 237 MB) starts with `OPRW`, version
**32**, then the bytes `0FNE` (`ENF0` reversed, presumably an Enfusion-era block). Public docs and
bis-file-formats stop at v27. ChernarusPlus is the same family.

**Update:** [DevilinJake/dayz-wrp-reader](https://github.com/DevilinJake/dayz-wrp-reader) (Python, MIT, Oct 2026)
documents the full v32 layout section by section: header `0FNE` + u8 + u32; Arma quadtrees for geography,
sound and material index; peaks; an LZO `ENFI` block (tiled 4-bit dominant-surface grid, absent 64x64 tiles =
sea) replacing the grass/primary-texture arrays; LZO float32 heightmap; rvmat and model lists; classed models
and road parts with an extra u32 `flags`; 64-byte object records; map info with DayZ-specific record sizes.
The road net stores every road piece with its connection points, so the generator must emit that too.
Checked independently with our own minimal parser on both vanilla files: the header, the three quadtrees and
the ENFI size match the description exactly (Livonia 1024/1024 ENFI tiles, Chernarus 826 = sea gaps; the LZO
stream starts with literal `ENFI`). Remaining unknowns are a few constants and flag words
(header `0FNE`/u8/u32, ENFI header word `0x867f0406`, the `0x80xxxx00` flags, map-info types 41-43).
Their code was not run (blocked by the sandbox as downloaded code); only the layout was used.
-> **The world format moves from "reverse-engineer" to "write from a known spec, copy observed constants,
verify by round-tripping vanilla files".** It is still the biggest single writer.

**From the DayZ-Modding-Knowledge-Pack** (willy92wins, MIT, installed as skills under
`~/workspace/skills/DayZ-Modding-Knowledge-Pack`). Each item is the pack's own claim and is unverified here unless stated:
- **OPRW history:** DayZ 1.29 shipped OPRW **v29** (v28/v29 share one layout, with sub-magic `0FNE`). 1.30 moved both
  vanilla maps to **v32** and grew them by about 5%. DayZ Tools 1.30.164.078 (2026-10-15) binarizes mod maps to v32
  as well. The vault note `dayz-wrp-roadgraph-extraction.md` describes the v29 road net (RoadLink = connection count,
  positions, connection types, objectId, extra u32, asciiz p3d, 4x3 matrix). It also notes that many maps place
  roads as loose objects that are *missing from the road net*. Their GPS road-graph extractor stitches those back in.
  A real v29 file spec (`WRP_V29_FORMAT.md`) exists but is not published.
- **The 1.30 object record's extra u32** was 0 in all 58,708 objects of a v32 mod map. The 1.30 changelog adds a
  per-object "underground" flag (Terrain Builder import gets an underground column), which is a likely candidate.
- **Terrain holes (1.30):** `class Holes { class X { tiles[] = {{x,z},...}; }; }` in CfgWorlds, or a `holes.cfg`
  next to `layers.cfg` with an include. Cells are heightmap cells (6.25 m on Livonia). There is a script call
  `SurfaceIsHole`, and underground triggers live in `cfgundergroundtriggers.json`. This makes generated tunnels,
  bunkers and cellars possible later.
- **Navmesh `.nm`:** a `TESM` v6 container (48-byte header: tile count, tiles x/z, origin, tile size), then an
  offset table, then per tile a 16-byte prefix plus a standard **Detour `dtMeshHeader` (magic `DNAV`, version 11;
  upstream Recast writes 7)** and the tile data. The tile reader in `tools/dayz-navmesh-compare` was checked against
  all three vanilla meshes (Chernarus 94,249 tiles, Livonia 65,536, Sakhal 94,249). So a Rust Recast/Detour build
  is a realistic native navmesh path: a Detour tile writer plus the container. The pack also reports that DayZ Tools'
  generator is not byte-deterministic, so it compares meshes by structure, and that custom terrains must regenerate
  `.nm` (no backward compatibility).
- **ODOL v56:** binarize.exe 1.30.164.078 writes v56. v56 equals v55/v54 plus **4 zero bytes in ModelInfo**
  (right after the float read as `ht_min`), with LOD offsets shifted by 4. Their measurement covers 488 vanilla
  models compared against their v54 files. This is directly useful for the native binarize in the armake2 fork.
- **Unbinarized models:** 1.30 loads unbinarized MLOD `.p3d` from unpacked `-mod` folders (`-filePatching`,
  `-cacheP3D=1` writes a `.p3dcache`). For development, generated road and terrain meshes could skip binarize
  entirely. Shipping still binarizes.
- **binarize.exe stall:** without `-addon=<parent of the staged source>`, binarize can wait minutes or never start.
  This is relevant to the Proton binarize bridge.
- **Water:** vanilla rivers and streams are pitched `river_24_*` / `stream_14_*` pieces (median pitch 1.5° and
  2.2°). The engine reads the water level from the piece's Roadway face, so `SurfaceIsPond` is false on most of
  them. Flat stepped planes looked worse on the Baltic map. The OSM waterway generator should place pitched
  pieces the same way as road pieces.
- **Tools:**
  - `py3d-dayz` (MIT, MLOD read/write and validators for winding, components and autocenter): the reference if we
    generate our own road meshes (MLOD).
  - `dayz-3d-viewer` (MLOD/PAA/RVMAT to glTF): a preview path.
  - `dayz-pbo-diff`.
  - **[dayz-mcp](https://github.com/willy92wins/dayz-mcp)** (MIT): an MCP bridge that drives DayZDiag server and
    client (teleport, raycast, `surface_query`, screenshots, vehicle control and trace). It is the natural tool
    for automated in-game acceptance of a generated map: drive every road, raycast building entrances, screenshot
    sample points. Tried on 2026-11-03: the Enforce bridge runs unchanged on the native Linux server and polls a Linux daemon. The Python daemon needed shims (about 9 Windows-only spots; leases are still missing). The Proton client loaded the mod, but the run stopped on a client/server build mismatch. Details: the knowledge pack's `knowledge/linux-proton-findings-2026-11.md`.
  - `dayz-odol-strict` depends on an unlicensed backend, so we don't use it.
- **Other projects:** the Baltic 12 km terrain project (unpublished) uses Terrain Builder, NavMeshGenerator and
  a Win32 driver for Terrain Builder. Flynn's Terrain Tools (`Naviata/FTT-Releases`) is binary only and has a
  hole generator.

**World PBO layout (Livonia).** `worlds_enoch.pbo` = `config.bin` + `enoch.wrp`. `worlds_enoch_data.pbo`
= 11479 files under `layers\` (per-tile `M_xxx_yyy_lca.paa` masks, `S_...` sat tiles, per-tile rvmats),
1024 `usermap\` tiles (the 2D map, raster), `texHeaders.bin`, `enoch_satout_co.paa`,
`enoch_middle_mco.paa`, `enoch_global_nohq.paa`, `enoch_deepmap_co.edds` (an Enfusion `.edds` texture).
`worlds_enoch_nav.pbo` = `navmesh.nm` (430 MB). `worlds_enoch_ce.pbo` = the Central Economy set
(`areaflags.map` 75 MB, `cfgeventspawns.xml`, `cfgplayerspawnpoints.xml`, `env\*_territories.xml`, `db\*.xml` ...).

**World config.** `CfgWorlds >> Enoch` has `worldName`, `class Navmesh` (with `GenParams`, agent sizes
and jump/fence `Links`), `OutsideTerrain`, `Holes`, `Grid` (map zoom levels), `centerPosition`,
`SoundMapValues`, `UsedTerrainMaterials` (12 materials `DZ\surfaces_bliss\data\terrain\en_*.rvmat`),
`clutterGrid/clutterDist`, `midDetailTexture`, ocean materials (`.emat`), `class Names` (locations).
There is **no `newRoadsShape`** -> DayZ roads are not shapefile-rendered like Arma 3. Roads are objects.

**Road pieces** (`DZ\structures\roads\Parts`, ODOL). Families `asf1` (12 m wide main), `asf2`, `asf3`,
`city`, `grav` (gravel), `mud`, plus `path_dirt_*`, `path_rock_*`, runway/taxiway sets, `Centerline_*`.
Per family:
- straights `_6`, `_12`, `_25`, end cap `_6konec`, `_6_crosswalk` (asf/city)
- arcs named `<angle> <radius>`: `0 2000`, `1 1000`, `7 100`, `10 100`, `10 75`, `10 50`, `10 25`,
  `15 75`, `22 50`, `30 25`, `60 10` (assumed: 0.75 deg, 1 deg, 7.5 deg, 22.5 deg; to verify from model memory points)
- junctions `kr_t_<a>_<b>` (T) and `kr_x_<a>_<b>` (X) between specific families
- bridges `Bridges\Bridge_{Metal,Stone,Wood}_{25,50}`

**Rails** (`DZ\structures\rail\Tracks`): straights 2/4/8/25/40 m, curves `L25_10`, `L25_5`, `L30_20`
(and R), turnouts L/R, passing loops, `Up_25/40` and `Down_25/40` grade-change pieces, `_lc` variants
(presumably level-crossing/collision variants), bridges 15/40 m, crossings, platforms, stations.

**Walls/fences** (`structures_walls`): segment pieces 3-8 m (`Wall_CBrk_5`, `Wall_Fen1_5`,
`Wall_FenW_7`, `Wall_CncSmall_8` ...), with `_Pole`, `_End`, `_Corner`, damaged `_D`, `_Hole` variants.

**Models.** All 1012 structure models checked are **ODOL v56** (older than Arma 3's v7x). ModelInfo
bounding boxes read fine with the public layout, so a footprint catalog is cheap (3.5 min for 1012
models through `armake2 cat`; a native reader would be seconds).

**DayZ Tools present locally:** AddonBuilder, Binarize, BinMake, CeEditor, CfgConvert, DsUtils,
ImageToPAA, NavMeshGenerator, ObjectBuilder, PboUtils, Publisher, TerrainBuilder, TerrainProcessor,
Workbench, WorkDrive. These define the replacement list in section 6.

---

## 3. Roads, footpaths, railways: is automatic placement possible?

### 3.1 Spike (throwaway, `scratchpad/spike/fit_v6.py`)

Area: 47.55-47.58 N, 11.05-11.10 E (Ettal / Graswang valley, Bavarian Alps: steep, curvy, good worst case).
OSM via Overpass, elevation from AWS terrarium tiles (z14, ~6 m/px).

Method:
1. Classify OSM ways into part families (my call for the spike): primary->asf1, secondary->asf2,
   tertiary/unclassified->asf3, residential->city, track grade1-2->grav, grade3-5->mud.
   **path/footway (incl. `sac_scale=demanding_mountain_hiking`) get no road pieces.**
2. Build the network, join ways through degree-2 nodes, split only at real crossings
   -> 172 crossing-to-crossing stretches, 56 km.
3. Per stretch: heading tracker emits catalog arcs whenever chain heading lags OSM heading
   (arc choice = best curvature match over the arc's own length).
4. Straights are **continuous** (overlapping 6/12/25 m pieces), so for a fixed arc sequence the end
   point is linear in the straight lengths -> bounded least squares closes onto the next crossing.
5. Local search: swap arc radii with the same angle (10 deg exists at R 25/50/75/100 = 4.4-17.5 m long:
   same heading change, different length), insert near-invisible S-bends (+1/-1 deg) into long straights.

Results (vanilla pieces only, no junction pieces, no elevation in the fit):

| family | stretches | km | closes < 0.5 m | < 3 m | median dev from OSM | heading err p90 | pieces /100 m |
|---|---|---|---|---|---|---|---|
| asf1 | 16 | 3.5 | 50% | 62% | 2.9 m | 3.2 deg | 6.5 |
| asf2 | 15 | 3.0 | 53% | 67% | 3.8 m | 2.9 deg | 6.3 |
| city | 21 | 4.0 | 52% | 62% | 4.3 m | 3.0 deg | 6.9 |
| grav | 58 | 17.4 | 50% | 67% | 6.9 m | 6.3 deg | 7.7 |
| mud | 62 | 14.2 | 58% | 66% | 6.9 m | 4.7 deg | 7.3 |

What the spike taught:
- **It works in principle.** Half of all stretches close exactly with a greedy search written in an evening.
- **The catalog has no short low-angle pieces.** 1 deg costs 17.5 m of road (R=1000), 0.75 deg costs
  26 m. Small heading corrections in short stretches can only come from tight arcs (`10 25`, 4.4 m), which
  look like kinks. Heading bookkeeping must be length-aware (bug found and fixed in the spike).
- **Split only at real crossings.** Splitting at every OSM way boundary created many short stretches
  that each had to close exactly; joining through degree-2 nodes was the biggest single improvement
  (p90 gap 35-125 m -> 8-12 m).
- **Closure needs a real solver**, which matches the user's idea. The free variables are:
  arc choice per curve (discrete), straight lengths (continuous, >= 0, with overlap allowed),
  S-bend insertion (discrete), and **junction positions/rotations** (continuous, a few metres around the
  OSM node; the spike pinned them, which is the main reason for the remaining gaps).
  Formulations worth trying: beam search / A* over arc sequences with an LSQ inner loop (what the spike
  does, but global instead of greedy); MILP (arc counts per radius class as integers, straights continuous,
  closure as linear equality after fixing heading order); or CP-SAT. Solve the whole network jointly
  so junctions can float.
- **Overlap is the escape hatch.** Overlapping straights make length continuous. Overlap at arc ends is
  also possible but needs care (z-fighting; a Steam comment reports road flicker after 1.27, cause unknown).

### 3.2 Elevation along the same roads (spike numbers, raw DEM, no fitting)

| family | max grade p90 | median cross-slope | cut/fill at road edge p90 |
|---|---|---|---|
| asf1 | 9.2% | 13.2% | 1.7 m |
| asf2 | 9.5% | 1.6% | 0.3 m |
| city | 22.7% | 3.3% | 1.3 m |
| grav | 30.1% | 7.4% | 1.0 m |
| mud | 33.1% | 8.1% | 1.0 m |

Road pieces are rigid models: they can be pitched/rolled per object but not bent. So the heightmap must
be reshaped: flat across the road (no bank), smooth pitch change between consecutive pieces, embankments
and cuts blended into the surroundings, bridges where OSM says so. The main road here sits on a slope with
13% cross-fall, i.e. ~1.7 m cut/fill at the edge for p90. That is normal civil-engineering territory and
must be done as a proper constrained optimisation on the heightmap (see section 5), not ARM's 20-pass
regression.

### 3.3 Footpaths and trails

Not road pieces. Options, in order of preference:
1. Paint them into the surface mask + satmap (a "path" surface material with no clutter), and lower clutter
   along them. This is what most hand-made DayZ maps do; cheap and looks right.
2. `path_dirt_*` / `path_rock_*` pieces (10/30 m + curve) for well-used trails in flatter terrain.
3. Use `sac_scale`, `trail_visibility`, `smoothness`, `incline` to decide; demanding mountain paths get
   only a faint mask stroke, or nothing.

### 3.4 Railways

Same machinery as roads with a different catalog (2/4/8/25/40 m straights, 25/30 m curves, turnouts).
Easier in one way: trains do not run in DayZ, so rails are scenery and tolerances can be looser.
Harder in another: real rail grades are tiny (< 3-4%), so the heightmap must give way much more (long
cuttings, embankments, bridges, tunnels -> portals or simply omit). `Up_*/Down_*` pieces handle grade breaks.
Turnouts give stations for free where OSM has `railway=switch`.

### 3.5 Creative alternative: generated road meshes

Instead of tiling vanilla pieces, generate a custom road model per stretch (MLOD with visual LODs,
roadway LOD for surface/physics, geometry LOD), textured with the vanilla road rvmats, and binarize it.
Pros: exact fit to OSM and to the terrain, no closure problem, smooth vertical curves, any width,
junctions of any angle. Cons: needs an MLOD writer and the native binarizer (both on the list anyway);
performance (unique meshes, draw calls; chunk to ~50-100 m); must verify how DayZ treats non-vanilla
roadway LODs (surface sounds, vehicle grip, AI). **Worth a spike**, because it removes the hardest
algorithm and leaves only geometry generation. A hybrid is likely best: vanilla pieces where they fit,
generated pieces for junctions and closures.

---

## 4. Buildings and other objects from OSM

### 4.1 Spike (throwaway, `scratchpad/spike/catalog.py`, `buildings.py`)

Catalog: 1012 vanilla structure models (residential, industrial, military, specific), ODOL bbox read
directly; 351 plausible building models after dropping props/walls/ruins/wrecks/proxies, 112 of them
house-like by folder name. OSM: 2295 building footprints in 47.55-47.60 N, 11.05-11.12 E (Ettal,
Oberammergau outskirts), minimum-area rectangle per footprint.

| match rule (model w/l vs footprint w/l) | any building model | house-like only |
|---|---|---|
| strict, 0.85-1.15 per side | 97% | 92% |
| loose, 0.7-1.25 | 99% | 98% |
| ARM-like, 0.5-1.25 | 99% | 99% |

- Footprint area p10/p50/p90: OSM 21/112/346 m², vanilla house bboxes 25/183/541 m² (bboxes include
  porches/overhangs, so real footprints are a bit smaller; read the geometry LOD later).
- House-like candidates per footprint at the strict rule: p10 1, median 3, p90 11 -> enough variety for most,
  but some sizes will repeat the same model; mods widen this.
- **79% of OSM buildings are tagged only `building=yes`.** Size is easy; *what kind of building* has to come from
  context: surrounding land use, `building:levels`, neighbours, distance to the centre, shops/amenities inside.
- 17% are < 30 m² (sheds, garages, huts): good candidates for sheds/garages or to skip.

Verdict: **placing buildings from OSM is clearly possible**; geometry matching is the easy part, classification
and the "DayZ look" (density, enterable vs not, loot balance) are the real work.

### 4.2 How placement would work (ARM ideas + DayZ specifics)

- **Classify** each OSM footprint: `building=*`, `amenity=*`, `shop=*`, `landuse` of the surrounding area,
  `building:levels`, `roof:shape`, `height`. ARM's classifier is a good starting table.
- **Fit** a model by footprint (oriented min-area rectangle) and category: size within tolerance, best
  surface match, entrance facing the nearest road, nudge away from road polygons. Bounding boxes come from
  the ODOL ModelInfo; better footprints come from the geometry LOD (to do).
- **Not every OSM footprint needs a model.** DayZ towns are sparse compared to real Bavarian villages; we
  could place fewer, larger, enterable buildings and fill the rest with ruins, gardens, sheds or nothing.
  A "density budget" per town (performance + gameplay) matters more than 1:1 realism.
- **Fill areas OSM leaves empty** (ARM's DefaultAreas idea): residential without footprints, farmyards,
  industrial zones, military areas -> procedural layouts from compositions.
- **Compositions:** hand-curated groups (house + fence + garden props + shed) placed as one unit; the
  library JSON idea from ARM.
- **Linear objects:** fences/walls/hedges (`barrier=*`), power lines (`power=line` + poles), tree rows,
  guard rails along embankments, street lamps procedurally along city roads.
- **Point objects:** `man_made=*`, `amenity=*` (bus stops, benches, wells, crosses, chapels, water towers,
  hunting stands `amenity=hunting_stand`, signs).
- **Vegetation:** forest by `landuse=forest/natural=wood` with `leaf_type`, edge/interior zones, scrub,
  isolated trees, riparian strips; Poisson-disc placement with species weights per biome.
- **Loot and gameplay follow placement.** Vanilla buildings already have loot points in `mapgroupproto.xml`;
  `mapgrouppos.xml` is just class + position + rotation of every placed lootable building, so we can write
  it directly. Same for `cfgplayerspawnpoints.xml`, territories (from forest/meadow areas), event spawns
  (roads -> vehicle wrecks, heli crash sites in open areas), `areaflags.map` (from land use).

### 4.3 Mod assets

Possible: scan Workshop mod folders (`@*/addons/*.pbo`), read `config.bin` for `CfgVehicles` house classes and
`CfgPatches`, read each model's ODOL bbox, classify with name heuristics + manual tagging, and remember which
mod each object comes from so the terrain's `requiredAddons` and Workshop dependencies are correct. Mod
models need `mapgroupproto` entries for loot (some mods ship them). Encrypted/obfuscated PBOs: skip.

---

## 5. Heightmap conforming

Treat it as a constrained least-squares problem on the heightmap grid (sparse, millions of unknowns, but
very structured):
- data term: stay close to the DEM (weighted by DEM confidence; low near roads, high elsewhere)
- smoothness term: second differences (terrain should look natural)
- hard/soft constraints from roads: cross-section flat at road width, longitudinal profile = smoothed
  road profile with max grade per class, shoulders blending over N metres
- rails: much tighter max grade; bridges/tunnels where tagged
- water: rivers monotonically downhill, lakes flat at their level, shoreline blending
- building pads: flat under each placed building footprint (DayZ buildings are rigid too)

Solve with a sparse CG/multigrid solver (e.g. on the GPU or `faer`/`nalgebra-sparse` in Rust). The road
profile itself is a 1-D optimisation per stretch (vertical alignment) and can be solved before the 2-D
problem, then fed in as constraints. Road pieces are then pitched to the solved profile.
Iterate road fitting <-> terrain once or twice: plan view first, then vertical.

Use **Copernicus GLO-30** as the default DEM (better than SRTM, no login), with optional national 1 m LiDAR
DEMs where available (e.g. Bavaria's open DGM1). GDAL is installed locally (3.13.3) for experiments; the Rust
tool should read GeoTIFF/COG itself or through the `gdal` crate.

---

## 6. The complete list: what is needed to reach the goal

Legend: **[have]** exists and usable, **[reuse]** exists elsewhere, adapt, **[build]** new code from known
specs, **[RE]** clean-room reverse engineering needed.

### 6.1 Data ingest
- [build] OSM: read Geofabrik `.osm.pbf` extracts offline (`osmpbf` crate); Overpass only for tiny previews
  (it was overloaded twice during this research).
- [build] DEM: Copernicus GLO-30 COG tiles (AWS open data), optional national LiDAR; reprojection to UTM.
- [build] Imagery: Sentinel-2 cloudless (EOX, CC-BY), or none (procedural satmap from materials, ARM's "fake sat").
- [build] Attribution/licence bundle (ODbL for OSM, Copernicus, EOX) written into the mod.
- [build] Area definition: UTM zone, origin, size, cell size; DayZ terrain sizes and grid constraints (to verify).

### 6.2 Core pipeline
- [build] Typed, cached, parallel layer DAG (ARM's builder pattern; Rust trait + `OnceLock`/rayon).
- [build] Geometry: `geo` + `geo-booleanop`/`i_overlay` (Clipper-like), rstar spatial index, polygon offsetting.
- [build] Deterministic seeded randomness per position (ARM does this; essential for stable regeneration).
- [build] Project file format + incremental regeneration (only rebuild layers whose inputs changed).

### 6.3 Asset catalog
- [have] PBO reading (armake2). [build] ODOL v56 ModelInfo reader (done in spike: bbox), geometry-LOD
  footprint, memory points (road piece start/end, door positions), named selections.
- [build] Config reading: `CfgVehicles` house classes, `CfgPatches`, scope; map class <-> model.
- [build] Categorisation: rules + manual tags, stored as a library (JSON/TOML) per biome ("Livonia",
  "Chernarus", "Alpine", "Sahrani-like"...). Seed it from ARM's Livonia library and vanilla map statistics:
  **read the object lists of ChernarusPlus/Livonia** to learn real usage (which houses appear together,
  typical fence lengths, tree species mixes). This needs the OPRW v32 object section -> one more reason to
  RE the format early.
- [build] Mod scanning (Workshop folders), dependency tracking.

### 6.4 Generators
- [build] Road network: classification, crossing-to-crossing solver, junction piece selection
  (`kr_t`/`kr_x` per family pair), bridges, end caps, crosswalks; optional generated meshes (3.5).
- [build] Rails: same solver, rail catalog, turnouts, platforms, crossings with barriers.
- [build] Paths: mask/satmap strokes, optional path pieces.
- [build] Buildings: classify, fit, orient, de-overlap, density budget, compositions, default-area fill.
- [build] Linear objects: fences, walls, hedges, power lines, guard rails, lamps.
- [build] Point objects from OSM tags.
- [build] Vegetation: forests (edge/interior), scrub, trees, tree rows, riparian; clutter config.
- [build] Water: sea (ocean in config), lakes (DayZ water surfaces/ponds; check how vanilla does inland lakes),
  rivers (terrain carving + water models?), to research.
- [build] Surfaces: surface mask from land use + slope + altitude, material set per biome (`UsedTerrainMaterials`),
  respecting the per-tile material limit (to verify for DayZ).
- [build] Satmap: real imagery blend, or procedural; roads/paths painted in.
- [build] Location names (`class Names`), 2D map (`usermap` tiles), map grid config.
- [build] Central Economy: `mapgrouppos.xml`, player spawns, territories, event spawns, `areaflags.map`
  (format to RE, simple raster?), `cfgeconomycore`, mission template `dayzOffline.<world>`.

### 6.5 Terrain shaping
- [build] Constraint system + sparse solver (section 5), road vertical alignment, building pads, water.

### 6.6 Output formats and BI tool replacements (the long poles)

| BI tool / file | Replacement | Kind |
|---|---|---|
| PBO packing, signing (`PboUtils`, `AddonBuilder`, `DsUtils`) | armake2 fork | [have] |
| Config rapify (`CfgConvert`) | armake2 rapify | [have] (verify DayZ world configs round-trip) |
| PAA (`ImageToPAA`) | armake2 img2paa / paa2img | [have] (verify DXT5 + mipmaps quality for satmap/masks; LCA/NOHQ/CO variants) |
| `.edds` (Enfusion DDS) | — | [RE] small; only if a world needs one |
| `texHeaders.bin` | writer | [RE] small, partly documented by community |
| Model binarize MLOD -> ODOL v56 (`Binarize`) | native in armake2 fork (decided) | [RE] large; Proton path works today as the bridge |
| Editable world `8WVR` (Terrain Builder output) | writer | [reuse] format known (bis-file-formats) |
| Binarized world OPRW v32 (`Binarize`) | native writer in armake2 fork | [reuse] layout from dayz-wrp-reader; [RE] only the leftover constants/flags. Largest writer: heightmap, ENFI surface grid, quadtrees, objects, road net with connection points, geography/sound/map info |
| Layers/tiling (Terrain Builder "generate layers": per-tile masks, sat tiles, rvmats) | own tiler | [RE] medium; outputs are visible in `worlds_enoch_data.pbo`, so black-box comparison works |
| Terrain Processor (scripted placement) | our generators | n/a |
| Terrain Builder (editor) | **our editor** (see 6.7) | [build] |
| Buldozer (live 3D preview) | own viewer and/or live DayZDiag | [build] |
| Object Builder | not needed, except MLOD writer for generated roads/compositions | [build] |
| NavMeshGenerator / Workbench navmesh | flare-navgen now; own later | [reuse] -> [RE] `.nm` + geometry extraction from our own world data (no DayZDiag) |
| CeEditor | not needed (we generate CE files) | n/a |
| Publisher | nozwock/steam-workshop-uploader | [have] |
| Work drive `P:` | not needed: all paths resolved from PBOs + project dir | [build] virtual file system over PBOs |

### 6.7 Editor and preview (Terrain Builder + Buldozer replacement)
- 2D editor (web UI served by the Rust binary, or egui/wgpu native): layers, OSM overlay, per-object
  overrides (move/delete/replace), "pin" overrides so regeneration keeps manual edits (ARM Studio does this).
- 3D preview: wgpu renderer of heightmap + satmap + object bounding boxes or decoded ODOL LODs (needs the
  ODOL mesh reader; bbox boxes first). This is the Buldozer replacement for layout work.
- Ground truth preview: boot DayZDiag/DayZ under Proton with the generated mod (dayz-dev skill already
  scripts local servers and headless clients), screenshot from fixed cameras for regression tests.
- Round-trip: export to TB-compatible text/8WVR so people with Terrain Builder can still finish by hand.

**Requested by the user (2026-11-03):**
- An editor like STAG.vision (prior art below), to edit the map features and to place, add and edit houses.
- Terrain modification, for example adding bunkers.

**Prior art: STAG.vision 0.1.2 (JMC Labs, https://jmclabs.de/games/stagvision, screenshots from 2026-10-31).**
Closed source, Windows, written from scratch in Rust with wgpu (DX12/Vulkan) and egui, our planned stack.
It edits your own map only, not the vanilla maps. It ships no game data and can't edit binarized `.p3d` (it displays them).

What the three screenshots show:
- **Layout:**
  - Outliner on the left: Objects (TerrainBuilder), 3.57 M objects; heightmap 4096²; satmap 16384 px; Mask with "162 violations".
  - About 40 named object layers with counts and visibility toggles (JMC_Roads, JMC_Water, TM_Forest, EVENT_*, playerspawns, …).
  - Large 3D view in the middle; asset browser below it (11 457 templates); tool/property panel on the right.
- **Asset browser:**
  - Thumbnails, grouping, library and mod-pack filters.
  - "Target layer" setting, random rotation, align to surface.
  - "Click a template, then a click into the terrain places an object".
- **Object editing:**
  - Move (W), Rotate (E), Scale (R), World/local (X), "Follow ground (Shift+T)", grid snap (1 m).
  - Property panel: model path, layer, position, height above ground or absolute, yaw/pitch/roll, scale.
  - Garden and Compound groups.
- **Selection:**
  - Arrow tool with rectangle selection.
  - Sphere/cylinder brush select with radius and an "only models matching the asset filter" option.
- **Terrain AutoFlat:**
  - Flattens terrain under placed objects automatically, by rules.
  - Settings: size %, transition (m), curve (smooth/linear/steep), height (mean/lowest/highest/object), offset.
  - Bake, or remove the flat.
- **Fill presets (program-wide):**
  - Spline/area fills: Inside (fills the closed area), Edge (a band that frays out), Along (fixed spacing on a line), Fence (pieces end to end), Roadway (decals on the road), Power line (poles with wires).
  - Each fill has a density per ha, random or along-spline rotation, max slope, and the rules "no water", "interrupt at roads" and "avoid fills before".
  - An optional "forest floor" (mask and satmap under the plants).
  - A weighted object list, a 20×20 m preview with reroll, and per-terrain tags (Chernarus/Nasdara/Mixed).
  - Saving re-applies the preset to the linked splines.
- **Objects into the satmap:**
  - Renders the placed objects top-down into a layer of the satmap.
  - Per category: buildings 224 k, trees 1.24 M, bushes 698 k, roads 79 k, other 1.19 M.
  - Sun shading, brightness and saturation; tile bake ("794 tiles changed").
- **Mask view:** a surface-mask overlay painted in false colour in 3D, with a cylinder brush ("left click paints").
- **Status bar:** "Heightmap unsaved", "Splines unsaved", and a frame time of 6.9 ms with 3.6 M objects.

What it implies for us:
- The fill-preset model (spline + preset + rules, re-applied when the spline changes) is the same idea as our generators. In our tool, OSM ways and areas are the splines.
- AutoFlat is a manual version of our section 5 constraints (building pads).
- A STAG-like editor and the OSM pipeline can share one data model: generated objects carry their source (OSM id + preset), edits pin them (see above).

**Terrain modification and bunkers.** Two different things:
1. **Heightmap edits:** brushes (raise/lower/smooth/flatten/ramp) and shapes (pads, ditches, embankments), stored as constraints or overrides, so that regeneration keeps them.
2. **Bunkers/cellars/tunnels:**
   - Terrain holes (1.30, section 2), `class Holes { tiles[] }` or `holes.cfg`, cut the terrain where an entrance object goes.
   - The underground part is placed objects (vanilla bunker parts or mod parts), with the per-object underground flag.
   - `cfgundergroundtriggers.json` for darkness and sound.
   - A holes editor (tile picker) plus "bunker" compositions: entrance + hole + interior + triggers placed as one unit.
   - Open: hole tile size and how the hole edge meets the entrance model (check vanilla Livonia/Sakhal bunkers and the knowledge pack `dayz-underground` skill).

### 6.8 Verification and testing
- Golden tests for every RE'd format: our writer vs BI's tool output on the same input (BI's DayZ-Samples
  `Test_Terrain` is the obvious corpus), byte-level or semantic diff.
- Read-back tests: parse vanilla `enoch.wrp`/`chernarusplus.wrp` completely, re-write, diff.
- In-game smoke test: local server boots with the terrain (dayz-dev `testing/local-server.md`), client spawns,
  screenshots; vehicle drive test along generated roads (bumps = failure).
- Spike-style metrics as CI checks: closure rate, deviation, grades, cut/fill.

### 6.9 Clean-room process (proposal)
- Two roles: **observers** produce format specs from files we are allowed to have (game files we own, outputs of
  DayZ Tools on our own inputs) and black-box behaviour of the tools; **implementers** write Rust only from those
  specs and public docs. No disassembly of BI binaries by implementers; specs never contain BI code.
- With AI agents this maps cleanly to separate agents/sessions: a spec-writer agent with hex/diff tools and no
  repo write access, and an implementer agent that only sees `docs/formats/*.md`.
- Keep a provenance log per spec section (which file, which experiment).
- Legal check before starting: DayZ Tools EULA terms on reverse engineering vs EU interoperability exceptions
  (in Germany §69e UrhG). Not legal advice; worth one look by someone who knows. Format analysis of data files
  you own is the low-risk part; decompiling `binarize.exe` is the high-risk part, so avoid it.

---

## 7. Creative ideas (unsorted)

- **Learn from vanilla maps.** Once OPRW v32 objects can be read, mine ChernarusPlus/Livonia for statistics:
  house-to-road distances, fence patterns, forest edge composition, how BI joined road pieces at junctions.
  Use them as priors for placement and as a test oracle ("does our village look like a Livonia village?").
- **Road piece solver as a reusable library** (`dayz-roadfit`): useful even for people hand-making maps in TB.
- **Generated road meshes** (3.5), and also generated junctions of any angle.
- **Gameplay-aware generation**: loot density targets, sightline checks around towns, POI spacing,
  "no spawn in valley death traps", vehicle-reachable road graph check.
- **Real-world fidelity knobs**: 1:1 OSM vs "DayZ-ified" (fewer, bigger buildings, more ruins, more forest).
- **Seasonal/biome skins**: same OSM, different asset library (Livonia autumn vs Chernarus).
- **Incremental regeneration**: change one OSM area or one setting, rebuild only affected tiles and objects.
- **Map-from-anywhere CLI**: `dayzmap new --center "47.57N 11.07E" --size 10km --biome alpine` -> bootable mod.
- **Use the spike numbers as product metrics** shown in the editor (closure, bumps, missing assets).

---

## 8. Risks and open questions

1. **OPRW v32**: the layout is now public (dayz-wrp-reader) and has no precomputed shadows or clutter.
   Remaining risk is the derived data we must compute ourselves (geography flags, ENFI dominant surface,
   map info, road-net connection points, object quadtree order) and the unknown flag words. Mitigation:
   round-trip vanilla files byte-for-byte, Proton binarize as a fallback.
2. Does DayZ load an **unbinarized 8WVR** world at all (for development)? If yes, a big shortcut for iteration.
3. Road piece **exact geometry** (start/end memory points, true arc angles): read from ODOL memory LOD.
4. **Per-tile surface material limit** in DayZ and how the per-tile rvmats are structured.
5. **Inland water** in DayZ (lakes/ponds/rivers) — how vanilla does it.
6. **Performance budget**: object counts per km² vs Livonia/Chernarus.
7. **Legal**: EULA vs clean-room RE; ODbL share-alike obligations for the generated map data.
8. Terrain size/grid constraints accepted by DayZ (heightmap sizes, cell sizes, satmap resolution).

## 9. Possible order (one suggestion, not decided)

1. OPRW v32 + ODOL v56 read-only (parse vanilla maps fully): unlocks statistics, test oracle, and the writer.
2. Data ingest + core DAG + heightmap only -> bootable empty terrain via Proton binarize (end-to-end early).
3. Road solver (network-wide, floating junctions) + vertical alignment + terrain conforming.
4. Buildings/vegetation/fences + CE files -> playable.
5. Native OPRW writer and model binarize in armake2 -> no Proton.
6. Editor + preview; navmesh RE.

## Appendix: spike artefacts

Throwaway, in the session scratchpad (not in the repo):
`/tmp/claude-1000/-home-bzed-workspace-dayz-tools-DayzMapCreator/5d379b45-1a9f-46e1-9a17-7a174ab648ee/scratchpad/spike/`
(`fit_v6.py` road fitting, `closure_stats.py`, `catalog.py` ODOL bbox catalog, `data/catalog.json`).

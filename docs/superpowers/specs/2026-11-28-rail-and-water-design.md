<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Náves: railways, rivers, streams and lakes

Date: 2026-11-28. Status: draft for approval. Second of three designs agreed on 2026-11-28; needs
`2026-11-28-osm-store-design.md` (the OSM copy and the feature table) first. Extends `2026-11-04-naves-design.md`
sections 5.7 (water) and 5.8 (validation) and the road designs (`2026-11-13-road-heights-design.md`,
`2026-11-16-bridges-and-terrain-design.md`, `2026-11-16-height-solve-design.md`); where this document and those
disagree, this one wins. Figures are the Berchtesgaden example (map area 16.4 km real, scale 1:2 horizontal and height,
terrain size 2048) counted in the OSM extracts on 2026-11-28; the model lists are from the local DayZ 1.30 install,
read with `dayzmake inspect`.

## 1. Intent

A map with roads but no railways and no water looks unfinished, and the design promised both from the start (spec 5.7:
"the Königssee and the rivers, lakes deep enough to swim", "the railway, never through water"). Railways and waterways
are networks like the roads, so they use the same machinery: a graph from OSM ways, a line fit with vanilla pieces, a
height profile, terrain shaping, bridges, checks.

What differs:

- **Railways need proper terrain** **(maintainer)**: a track keeps a small grade and changes it gently, so the terrain
  gives way far more than for roads: long cuttings and embankments, bridges.
- **Water flows downhill** **(maintainer)**: rivers and larger streams have a surface that never rises along the flow.
- **Small creeks can be interrupted** and may have dry stretches **(maintainer)**.
- **Bridges for railways and roads are handled properly** **(maintainer)**: rail over water and roads, roads over
  rails, level crossings.

Maintainer's decisions (2026-11-28):

- A **river** (the Ache, canals, anything with a river area) is **deep enough to swim**; roads and rails cross it on
  bridges. A **stream that flows all year** is **wadeable**. An **intermittent stream, ditch or drain** is a dry bed,
  or short pieces of water where the ground allows. **Fords where OSM tags them** (`ford=*`): there even a river is
  crossed at ground level.
- **Abandoned railways** get only their **trackbed** (shaped into the terrain, a gravel path on it); **narrow gauge**
  uses the **small track set where it makes sense**, else the normal pieces.
- **Water: standard pieces now, our own water models later** (W3, 4.2); river models of Chernarus and other maps may be
  used; small rivers are built the way Chernarus' west river is.
- From the earlier work, still in force: untagged crossings at different heights become bridges; 4.5 m clearance under
  road bridges; roads must be drivable and usable (memory: road crossing rules, roads goal).

## 2. Goals, non-goals, anti-goals

**Goals**
- Every `railway=rail`, `light_rail`, `narrow_gauge` on the map gets track pieces that join without gaps, at a grade
  within the line's limit, on terrain shaped to carry it.
- Switches (`railway=switch`) get turnouts; level crossings (`railway=level_crossing`) get crossing pieces with road and
  rail at one height; signals beside the track; platforms at stops and stations.
- Abandoned, disused and razed lines get a shaped trackbed with a gravel path.
- Rivers and streams get a bed carved into the terrain that falls monotonically along the flow, and a water surface
  (pieces) where they carry water; waterfalls and weirs are steps.
- Lakes, ponds and reservoirs get a flat water surface over a bed deep enough to swim, and no water leaks outside them.
- Roads and rails cross rivers on bridges (OSM's, or added where OSM has none and no ford), streams on culverts, fords or
  bridges as tagged.
- Validation rules for all of it, in the report and the GUI's Problems list.

**Non-goals**
- Running trains (DayZ has none); rails are scenery and a walking line.
- Tunnels: a rail or water way in a tunnel is left out and reported (`rail_tunnel_omitted`, `waterway_tunnel_omitted`),
  as roads do today (`tunnel_omitted`). Proper tunnels stay their own item.
- Station buildings, signal boxes, rail warehouses (buildings, later); catenary (overhead wires) **[my call]**: the
  game has rail poles (`Rail_PoleF`, `Rail_PoleT`, …) but placing them is a polish step after the track works.
- The sea and coasts (the example is inland; the ocean is inherited from the world config).
- GUI editing of rails and water beyond what the edit model gives for free (a railway or waterway drawn or edited as an
  OSM-style way in `edits/ways.toml` is picked up like an OSM one; drawing tools for them come later).

**Anti-goals**
- A track that bends up and down with the ground, or kinks where pieces meet.
- A railway or road in water without a bridge (spec 5.7).
- Water that flows uphill, or a river surface floating above its banks.
- A lake you can walk through, or water showing where there is no lake.
- A road that becomes undrivable because a river or rail took its ground (roads must stay drivable and usable).

## 3. Constraints

- `AGENTS.md` (tests first, coverage, lints, no panics, determinism, crate review, docs and changelog per commit).
- No game data in the repo: catalogs are read from the user's install at run time, as the road pieces are; tests use
  synthetic pieces.
- Clean-room: piece geometry from the models' memory points and bounding boxes, read the way road pieces are read
  today, and from observing vanilla maps. Today's catalog is **road-only**: `Family` has the six road surfaces with
  fixed widths, `catalog::classify` knows road name prefixes only, a piece needs the memory points `lb`, `pb`, `le`, `pe`
  (`MissingPoint` otherwise), and `catalog_read.rs` opens `structures_roads`, `structures_sakhal` and
  `structures_industrial` only. The catalog's data model is widened to kinds (road, rail, water) and the reader to
  `structures_rail`, `structures_rail_bliss`, `water`, `water_bliss`. **[verify] first** (a catalog spike) which memory
  points the rail, river, stream and streambed models carry; where they have none, the ends come from the bounding box
  and the name (length, angle), as a documented fallback.
- **Units [my call].** Every size in this design is in **map metres at game size** unless it says "real": players,
  vehicles and pieces keep their real size on a scaled map (spec 5.5), so clearances, water depths (swim, wade, ford),
  bed and track widths, platform lengths and thresholds measured against them are **not** scaled. Only what describes
  the land is scaled: positions, lengths along a line, OSM areas (a river area or lake outline is projected like any
  polygon), DEM heights (`scale.height`) and real altitudes (bands, as "real"). This matches the code (the road
  clearance `BRIDGE_CLEARANCE_M` = 4.5 m is in map metres) and the editing design's "real metres" apply only to terrain
  edits. An OSM `width` tag is a game-size width, as a road's is.
- The terrain grid is 4 m at terrain size 2048 on the example (`terrain-layers.md`); a feature narrower than a few cells
  cannot be carved exactly.
- The order of the build today: OSM → features and edits → network clean-up, planning, smoothing → road fit and
  placement → profiles (height solve) → bridges, dips → conform (earthwork) → surfaces → checks → export. New stages
  slot into it (section 5.1).

## 4. Approaches

### 4.1 One network or three

- **A. Railways and waterways as further kinds of the road graph** (one graph, one placement, one height solve, one
  earthwork), with kind-specific pieces, limits and rules. Crossings between kinds are found once, bridges and
  clearances use one code path. **My pick**: the road code already handles "a line below a bridge" and clearance in one
  solve; railways and rivers are what is below most often.
- **B. Separate pipelines per kind**, each with its own graph and solve, crossings reconciled afterwards. Simpler to
  start, but every crossing becomes a negotiation between two finished results, which is what the height-solve work
  removed for roads.
- **C. Water as terrain only** (carve beds, no water pieces) and rails as A. Cheapest, but rivers without water fail
  the maintainer's "deep enough to swim".

A for the graph, with two kind-specific parts that roads do not have: the **flow profile** of waterways (5.4) and the
**water surfaces** (5.5).

### 4.2 How water is shown

The install has three kinds of water models:

- **Generic river pieces** (`DZ\water\Streams`): `river_24_6/12/24` and curves `river_24_24_L/R10`, `L/R30`;
  `stream_14_6/12/24` and curves `L/R10`, `L/R30`, `L/R45`, `stream_14_16_L/R45`, `L/R90`. Pitched like road pieces;
  the engine reads the water level from the piece's roadway face (research doc 2). 24 m and 14 m wide.
- **Streambed pieces** (`DZ\water\Streambed`): `streambed_{sand,moss,leaf}_{long,short}_{straight,curve_left,
  curve_right,snake,slanted,arrow}`, with `_water` proxy variants and stone groups (`stones_*_bar`, `_group_*`).
  Narrow beds for creeks, wet or dry.
- **Ponds and lake planes**: `pond_big_*`, `pond_small_*`, `lake_50x50`, `pond_50x50`, Livonia's `enoch_50x50`; and
  Livonia's own lakes and rivers as **custom models made for that map** (`enoch_river_0…`, 804 pieces;
  `enoch_grabinskie_*`, `enoch_sitnicke_*`, … for its lakes).

So Livonia did not build its rivers from generic pieces. Options:

- **W1. Generic pieces, fitted like roads** (rivers `river_24`, streams `stream_14`, creeks streambed), lakes from
  `lake_50x50` planes. Uses only vanilla models; the road beam search fits them. Limits: two widths (24 m, 14 m: a
  perennial stream that is 3 m wide in reality gets 14 m of water or a streambed), the curve set is coarse. **My pick
  for the first version**: nothing new to generate, and the fitting exists.
- **W2. Generated water meshes** per river and lake (MLOD written by Náves, binarized through the existing bridge),
  following the bed exactly, as Livonia's custom pieces do. Best look and any width, but a new model writer and a
  new kind of output; the spec lists generated road meshes as later work (milestone 3).
- **W3. W1 now, W2 later for what W1 cannot do** (narrow perennial streams, wide rivers from a river area, lakes of any
  shape). The data model (bed, surface profile, width) is the same; only the surface output changes.

**The maintainer picked W3** (2026-11-28): "Standard pieces now, our own water models later." And: "we can always use
river models from Chernarus and other maps", so the catalog takes every generic water model of the install (`DZ\water`
and the DLC water PBOs), not one map's set. Livonia's per-map models (`enoch_river_*`, `enoch_<lake>_*`) are shaped for
Livonia's terrain and are not used **[my call]**.

**How Chernarus does it** (read on 2026-11-28 from `ChernarusPlus.wrp` of the install: the model list and the object
records, model index followed by a 3×4 matrix; a throw-away reading, not code for the repo). The maintainer pointed
at the small river in the west that runs north to south as the pattern to follow ("perfectly fine for our needs"):

| Model | Objects | Tilt p50 / p90 | Scale |
|---|---|---|---|
| `river_24_*` | 660 | 1.5° / 3.8° | 1.0 |
| `stream_14_*` | 459 | 2.1° / 6.4° | 0.63 to 1.16 |
| streambed pieces | 1681 | 4.3° / 40° | 0.51 to 1.48 |
| stone groups | about 600 | | |
| `pond_50x50` (+ moss) | 321 | flat | 0.59 to 1.0 |
| `lake_50x50` | 27 | flat | 1.0 |
| `pond_big_*`, `pond_small_*` | the rest | flat | |

- **The west river is a chain of 752 pitched `river_24` and `stream_14` pieces** from about 210 m above sea level at
  z ≈ 9480 down to the coast (1.3 m): `stream_14` in the upper part, `river_24` in the lower. There are no flat
  stepped planes on it; each piece is tilted with the river. These are water surfaces laid on (and above) the carved
  ground, which is what the maintainer remembered as "water areas above ground".
- **Pieces are scaled**: `stream_14` down to 0.63 (about 9 m wide), streambeds 0.5 to 1.5, ponds down to 0.59. So the
  two widths are not a hard limit; a stream piece scaled to the stream's width is the vanilla way.
- **Creeks are streambeds**, 1681 pieces in the interior, often steep (p90 40°), with stone groups along them.
- **Ponds are flat `pond_50x50` planes**, scaled down for small ponds; `lake_50x50` for a few larger lakes.

The design below follows this: W1 built the way Chernarus does it, with the outputs kept replaceable by W2.

## 5. The design

### 5.1 Stages

```
OSM copy → features (roads, rails, waterways, water areas) + edits
  → network clean-up (roads; rails: join only, 5.2)
  → flow network and water levels (5.4)            new
  → road + rail fit and placement (5.3)            rails new
  → water piece fit (5.5)                          new
  → crossings: bridges, culverts, fords, level crossings (5.6)
  → profiles: one height solve for roads and rails, with water levels as fixed constraints (5.3, 5.6)
  → earthwork: roads, rails, river beds, lake beds (5.7)
  → surfaces, checks (5.8), export
```

Water levels come **before** the height solve and are fixed in it **[my call]**: a river does not move to suit a road;
the road and rail heights near water are solved against the water levels and the bridge clearances.

### 5.2 Railways: what is built

From the feature table (OSM store design 5.4):

| OSM | Built |
|---|---|
| `railway=rail`, `light_rail` | track (`Rail_TrackE_*` family **[verify]** against `Rail_Track_*`: the `E` set is the complete one — straights 2, 4, 8, 25, 40 m, curves `L/R25_5`, `L/R25_10`, `L/R30_20`, turnouts — and `_lc` variants **[verify]** what `_lc` / `_nolc` mean before using them) |
| `railway=narrow_gauge` | the **small track set** of `structures_rail_bliss` (`Rail_TrackSmall_3m/10m/20m`, `_7deg/15deg/30deg`, `_Switch`, `_TurnOutL/R`, `Rail_Crossing_Block_Small`, `Rail_LineBreak_Small`) "where it makes sense" **(maintainer)**: for every narrow-gauge line, and for standard-gauge `service=spur` or `usage=industrial` lines whose curves are too tight for the normal pieces **[my call]**; a narrow-gauge line the small set cannot fit gets the normal pieces (the maintainer's first answer) and is reported |
| `railway=abandoned`, `disused`, `razed` | trackbed only **(maintainer)**: the rail profile and earthwork, a gravel path painted on it (`FeatureKind::Path`, 3 m) |
| `service=siding`, `yard`, `spur`, `crossover` | track, like the main line, lower rank |
| `railway=switch` node | in the first step of the rail plan plain track through the switch; then turnout `Rail_TrackE_TurnOutL/R` at the node, side from the geometry of the two leaving ways; `Rail_Track_Passing_*` where two switches form a passing loop **[verify]** the geometry |
| `railway=level_crossing`, `crossing` | `Rail_Crossing_Block_10/25` under the road (and `Rail_Crossing_Block_Small` on the small set), barriers `Rail_Crossing_Barrier` at level crossings of roads of class tertiary and above **[my call]** |
| `railway=signal` | `Rail_Signals` beside the track, on the side OSM gives (`railway:signal:position`), else right in the direction of travel **[my call]** |
| `railway=stop`, `station`, `halt`, `platform` | platform from `Rail_Platform_Start` + `_Segment`s + `Start` (mirrored), length from the OSM platform way or 60 m real **[my call]**, on the side OSM's platform lies, else both sides for a station and one for a halt |
| `railway=buffer_stop` | end of track (`Rail_LineBreak_Concrete`) |
| `bridge=*` on a railway | rail bridge (5.6) |
| `tunnel=*` | omitted and reported |

**Graph.** Rail ways are joined like road ways (`roadnet::build_graph`) into stretches between switches, crossings and
ends. The road network clean-up (sidepaths, stubs, junction merge, planning, smoothing) does **not** run on rails
**[my call]**: OSM railway geometry is accurate, curves are large, and a rail is not a candidate for "parallel path"
removal. Rails only get the join step and the map-edge cut.

**Fit.** The road beam search (`roadfit`), extended: today it is not catalog-driven for straights (it assumes 25, 12.5
and 6.25 m, works in quarter units of 6.25 m in `run_pieces`, and lets straights of a run overlap by up to 6.25 m,
which draped road textures hide). For rails the straights come from the catalog (2, 4, 8, 25, 40 m) and **runs close
exactly without overlap** **[my call]**: doubled sleepers and rails would show; the 2 m piece gives the resolution.
Water pieces (6, 12, 24 m) may overlap like roads (the water surface hides it) **[my call]**. Rails tolerate less lateral deviation than roads do
from the OSM line (a rail line is well mapped) and no kinks: the fit's lateral tolerance and the end-pose closure for
rails are **[left to the builder]**, measured on the example. Real curves of the branch lines (radius ≥ about 200 m)
become ≥ 100 m at 1:2; the curve pieces give radii of about 86 m (`L30_20`), 143 m (`L25_10`) and 286 m (`L25_5`)
**[verify]** from the memory points.

**Profile.** One height per rail joint, solved in the roads' joint height solve. That is `profile::solve_clearing` (the
build's call; it holds the bridge clearance and the graph ties), not `solve_profiles`. Rails use
**rail limits** **[my call, to be tuned on the example]**:
- grade ≤ **4 %** by default (`[rail] max_grade` in `naves.toml`, per line override with the tag `naves:max_grade`
  later), measured in map metres after both scales;
- grade change per piece ≤ **0.5 %** (roads allow far more), with the `Up_25/40` and `Down_25/40` pieces where the
  change is larger **[verify]** their grade from the models;
- no tilt across (cross fall 0).

Where the line cannot keep its grade (the ground is steeper over a long stretch than the limit allows), the profile
takes the limit and the earthwork cuts or fills; above a cut or fill height of **10 m** **[my call]** the
stretch is reported (`rail_earthwork_large`) so the maintainer sees it, and nothing is bent.

**Track bed.** A track is 4.5 m wide at the sleepers; the bed for terrain shaping is **6 m** single, **10.5 m** double
**[my call]**, with **1.5:1** cut and **2:1** fill slopes (railway practice) and the rail's own height held across the
bed. The bed is painted with the gravel surface (`en_gravel` or the closest vanilla ground) **[verify]** which ground
layer is available; until a gravel layer exists, soil.

**Does track drape?** Road pieces drape on the terrain in the game. **[verify] on the first build** whether track
pieces do too, or are rigid models that need pitch (as bridges do). This decides whether the placement writes pitch
per piece; the placement code supports both (bridges are pitched today).

### 5.3 Railways and roads in one solve

The rails enter the joint height solve as chains with their own limits. Interaction rules:

- **Level crossing** (OSM `railway=level_crossing` node shared by a road and a rail, or a road and a rail that cross
  without a shared node or a bridge, at a height difference within 1.5 m **[my call]**): one height for both at
  the crossing; the rail's grade limit wins, the road gets a hump only within its own limits. The crossing piece lies
  in the road's line.
- **Different heights** (a tagged bridge on one of them, or an untagged crossing with more height difference than
  above): a bridge (5.6). Untagged ones follow the rule "untagged height crossings become bridges".
- **Rank.** Where earthwork of a rail and a road compete (they run side by side, as in many valleys), the rail wins
  the ground **[my call]** (its limits are stricter) unless the road would become undrivable, which is reported as
  `road_rail_conflict` with both ids. The maintainer's rule "roads must be drivable and usable" stays in force; this
  is the case where both cannot be met and a person has to look.

### 5.4 Waterways: the flow network and water levels

**Classes** (from the feature table), with the maintainer's decision on what a player can do:

| Class | OSM | Water | Player | Bed (map metres, game size) |
|---|---|---|---|---|
| river | `waterway=river`, `canal`; any waterway inside a `water=river` area | yes | swims | width from the river area, else `width`, else 15 m; depth ≥ swim depth + 0.5 m |
| stream | `waterway=stream`, `brook` without `intermittent`/`seasonal` | yes | wades | width from `width`, else 3 m; depth 0.6 m |
| creek | `waterway=stream` with `intermittent=yes` or `seasonal=yes`; `ditch`, `drain` | short pieces only where the ground holds them (below), else dry | walks | width 1.5 m; depth 0.4 m |

Widths and depths are **[my call]**, the class rules are the maintainer's. Swim depth: **[verify]** in game (the design
already lists this); 1.5 m is the working figure.

**Graph.** Waterways are joined by node id into a directed graph; the OSM way direction is the flow direction (OSM's
rule). Junctions are confluences. A way whose direction contradicts the terrain over its whole length (OSM error) is
reversed and reported (`waterway_reversed`) **[my call]**. Lakes are nodes of the graph: a waterway that ends in a lake
area flows into it; one that starts at it flows out.

**Water level along a line.** For rivers and streams, the water surface is a profile along the line, solved per
connected tree from the mouths upwards:
- **monotone**: never higher downstream than upstream (an isotonic fit of the terrain sampled along the line, minus the
  bed depth, the standard "pool adjacent violators" method **[my call]**; any equivalent is fine);
- **steps** where OSM says so: `waterway=waterfall` (a step of `height` or the terrain difference), `weir`, `dam`,
  `lock_gate`: the surface may drop by any amount there and nowhere else by more than the piece pitch allows
  (`river_24` at most about 1.5°, `stream_14` 2.2°, the median pitches on vanilla maps, research doc 2: steeper
  stretches get several small steps, each at a piece joint) **[my call]**;
- **joins**: at a confluence the tributary's surface equals the main river's; at a lake the surface equals the lake's
  level (5.5); where the result would need the tributary to flow uphill, the tributary's bed is cut deeper.
- **culverts** (`tunnel=culvert`): the line continues under the road at the same level; no water pieces there; the
  road crosses on its own ground (5.6).

For creeks there is no surface profile; the bed follows the terrain monotonically (the same fit, depth 0.4 m) and is
painted (streambed models, 5.5).

**Carved bed.** The terrain under the line is set to (surface − depth) across the bed width, with banks at a slope of
**1:1.5** for rivers and **1:1** for streams and creeks **[my call]**, up to the original ground. A bed narrower than
two terrain cells (most streams: 3 m on a 4 m grid) cannot be shaped exactly; such a bed is a V of one cell width at
the bed depth, and the water pieces' banks cover the rest **[my call]**; the measured effect is reported per class.

### 5.5 Water surfaces (W1, replaceable by W2)

- **Rivers:** `river_24_*` pieces fitted along the river line with the road beam search, pitched to the surface
  profile, as Chernarus' west river is built (4.2); tilt within Chernarus' range (p90 about 4°), steeper stretches get
  more pieces at smaller steps. A river area wider than 24 m (the Ache in Berchtesgaden town) gets a second row of pieces **[verify]** that
  overlaps render well, else it waits for W2 and is reported (`river_wider_than_pieces`).
- **Streams:** `stream_14_*` pieces, **scaled across to the stream's width** as Chernarus scales them (down to 0.63, about
  9 m). A stream whose width (OSM `width`, its river area, else the class default) needs less than that gets the
  streambed set instead: streambeds with their `_water` proxies **[verify]** what the proxy shows in game, and where
  that does not look like water, a dry bed until W2 **[my call]**. The lower scale limit is **[verify]** in game
  (Chernarus' 0.63 is known to work; smaller may too).
- **Creeks:** streambed pieces (sand in the valley, moss in the forest, leaf under broadleaved trees: the surface or
  forest class at the bed decides), dry; `_water` variants only on stretches where the bed's grade is under 2 % and
  the stretch is longer than 20 m **[my call]**: "short pieces where the ground allows". Stone groups at random along
  the bed (seeded, 5.9 below).
- **Lakes, ponds, reservoirs:** the water level is the median of the DEM along the lake's outline **[my call]** (the
  DEM gives the surface); the lake is covered with `lake_50x50` planes (scaled and rotated, as few as cover the outline
  within the leak rule) **[verify]** in an early spike: whether planes can be scaled and rotated, whether overlaps
  render, and how vanilla inland lakes are built from them (Chernarus uses `pond_big_*`, Livonia custom models). The
  bed is lowered inside the outline (spec 5.7: always deep enough to swim, a lake has a depth setting), and **outside
  the outline but inside a plane, the terrain stays above the water** (raised if needed); where that would change the
  ground by more than 1 m, more, smaller planes are used. Shores get a gentle slope (1:5 for the first 5 m
  **[my call]**) and a shore ground class.
- **Intermittent lakes** (`intermittent=yes`) and `natural=wetland`: no water plane, a soil or wet ground surface.

All water objects get stable identities (`water/<feature>/<slot>`), so the editing design's overrides work on them.

### 5.6 Crossings: bridges, culverts, fords, level crossings

Every crossing of two lines (road, rail, river, stream, creek) is classified once (A in 4.1):

| Upper ↓ / lower → | road | rail | river | stream | creek |
|---|---|---|---|---|---|
| **road** | today's rules (junction, overlay, bridge with 4.5 m clearance) | road bridge, clearance **5.5 m** **[my call]**; OSM level crossing → crossing piece | OSM bridge or added bridge; ford if tagged | OSM bridge; ford if tagged; else **culvert** | culvert (road on its own ground, the creek interrupted) |
| **rail** | rail bridge over the road (4.5 m clearance) or level crossing | crossing of two rails: `Rail_Track_SP`/crossing piece **[verify]** | rail bridge (OSM or added) | rail bridge if OSM, else culvert | culvert |

- **Road over water without OSM bridge or ford**: rivers always get a bridge (they are swim-deep): the road bridge rules
  of `bridgekind` choose the part by length; the bridge's length is the river bed's width plus banks. Streams get a
  culvert **[my call]**: in reality most untagged road-stream crossings are culverts that mappers left out, and a
  14 m-wide water piece across a road would be wrong. A culvert means the stream's pieces and bed stop 2 m from the
  road's edge and continue on the other side. Where the road crosses the stream high above the ground (a gully),
  today's dip rules still apply (`dips`: a dip deep enough gets a bridge); a culvert is only for a road on the ground.
- **Fords** (`ford=*` on the crossing node or the road way): the road goes through at ground level; the river or
  stream bed is raised to a wadeable depth (0.5 m) under the road's width **[my call]**, the water surface
  continues. A drivable road through a ford stays drivable by the check's rules, else reported.
- **Rail bridges**: `Rail_Bridge_15`, `Rail_Bridge_40`, `Rail_Bridge_15_Curve`, `Rail_Bridge_15_End` as end pieces;
  chained to cover the OSM bridge length like the road bridges' "chain of the family's longest model" (`bridgekind`).
  The example has 2 rail bridges (plus 1 abandoned, which as a trackbed gets no bridge: the trackbed ends at the gap,
  reported, **[my call]**).
- **Clearance** is one constraint of the joint solve, as for roads today: a bridge over a road, rail or water and the
  line below are solved together (the "line below" in `bridgefit` generalises from a road to any kind; for water the
  "line below" has the fixed water level and needs clearance above the **surface**: 2 m **[my call]**).
- **Waterway under a bridge of OSM** is not carved into the deck: the deck spans the bed; the bridge-ground rules
  (`bridgeground`, `conform_bridges`) treat the bed as the ground under the span.

### 5.7 Earthwork

The existing `conform` holds the terrain to slope limits around each road. It is extended to rails (their bed width and
slopes, 5.2) and to water (bed and banks, 5.4; lake beds and the "terrain above water inside a plane" rule, 5.5). Order
where they overlap **[my call]**: water beds first (fixed levels), then rails, then roads, each holding what the earlier
ones set except where a bridge spans it. A road or rail that is pushed off its ground by a water bed without a bridge
is a bug the crossing table should have prevented; it is reported (`*_in_water`).

The planned grid height solve (`2026-11-18-grid-height-solve-design.md`) would solve terrain vertices jointly; this
design does not depend on it, but keeps its inputs (bed widths, slopes, fixed levels) in the same form as the roads',
so the grid solve can take rails and water when it comes.

### 5.8 Checks and validation rules

New rules (stable ids, in the report, MCP and the Problems list):

- `rail_grade` (over the line's limit), `rail_grade_change`, `rail_gap` (pieces not closing), `rail_kink`,
  `rail_in_water` (spec 5.7), `rail_earthwork_large`, `rail_tunnel_omitted`, `rail_bridge_not_built`,
  `rail_clearance`, `road_rail_conflict`;
- `waterway_uphill` (should never fire: the profile prevents it; it is the check), `waterway_reversed`,
  `waterway_tunnel_omitted`, `river_wider_than_pieces`, `road_over_water` (exists: a dip over a water line without a
  bridge, `validate::dip_violations`; with the crossing table it fires only for a crossing the table could not build), `water_leak` (water visible outside a lake outline: terrain
  below the plane outside the outline), `lake_too_shallow` (spec 5.8), `road_in_water` (exists, now against carved beds
  and lakes), `water_clearance`;
- the existing road checks see the new ground (a culvert, a ford).

Measures for the plan and the report: rails per km and pieces per km; max grade and max grade change on rails; cut and
fill volumes per kind; rivers per km with surface pieces; the share of streams that got water, a streambed, a culvert.

### 5.9 Switched off until checked in game

The crossing rules change roads that exist today: a road over a river gets a bridge today only where it stands at
least 8 m above the ground (`dips::BRIDGE_DIP_M`), otherwise `road_over_water` is reported; under this design rivers
always get a bridge and streams culverts, and rails take ground from roads. So, like the smoothing and the clean-up
steps, **railways and water each ship behind a switch** (`RAIL_ENABLED`, `WATER_ENABLED` constants, as the clean-up's
`..._ENABLED`) **[my call]**, off until a game check of the example; a test pins that the build is unchanged with both
off. The maintainer turns them on.

### 5.10 Randomness and identity

Streambed variants, stone groups and platform sides that are chosen at random use the project's seed with the
feature id and the slot (spec 5.2), as the road objects do. Every generated object has a stable identity
(`rail/<stretch>/<slot>`, `water/<feature>/<slot>`), so the editing design's overrides apply to them.

## 6. Testing

- Synthetic networks in tests (as the road tests do): a rail line over a hill (grade limit, cut and fill); a switch; a
  level crossing with a road; a road bridge over a rail (clearance); a rail bridge over a river; a stream with a
  culvert under a road; a ford; a river into a lake (levels meet); a tributary that would flow uphill (bed cut); a
  waterfall step; an intermittent creek (dry streambed); an abandoned line (trackbed, path, no pieces).
- Catalogs from synthetic piece descriptions (no game data), real catalogs only in `#[ignore = "needs DayZ"]` tests.
- Properties (proptest, fixed seeds): the water surface never rises downstream; pieces close; the terrain outside a
  lake inside its planes stays above water.
- In game (manual, `tools/ingame-check/` with a site list): photographs of the two rail bridges, a level crossing, the
  Ache in town, a ford, a culvert, the Königssee shore, a dry creek, a rail cutting; a swim test in a river and a lake
  (does the player swim?), a wade test in a stream.

## 7. Left to the builder

- Fit tolerances for rails and water pieces; the exact rail limits after measuring on the example.
- How the flow profile is solved (isotonic fit or another monotone method), and how steep stretches are split into
  steps.
- The data structures for the joint graph (one graph with kinds, or a graph per kind with shared crossing objects).
- The order of plans (rails and water can be two plans after a shared crossings/graph plan).
- The spike's method for lakes.

## 8. Designer's calls to check

1. One graph and one height solve for roads, rails and water (4.1 A).
2. Livonia's per-map water models are not used; every generic water model of the install is (4.2).
3. Water levels fixed before the height solve; roads and rails adapt (5.1).
4. No clean-up or smoothing on rails (5.2).
5. Rail runs close exactly without overlapping straights; water pieces may overlap (5.2).
6. Switches as plain track first, turnouts as a later step of the rail plan (5.2).
7. Rails and water behind switches, off until checked in game (5.9).
8. Rail limits: 4 % grade, 0.5 % change per piece; bed 6 m single, 10.5 m double; report earthwork over 10 m (5.2).
9. Rails win the ground over roads where both compete, unless the road becomes undrivable (reported) (5.3).
10. Level crossing for untagged crossings within 1.5 m height difference (5.3).
11. Clearance under road bridges over rails 5.5 m; above water 2 m (5.6); all sizes at game size, not scaled (3).
12. Untagged road–stream crossings are culverts, untagged road–river crossings are bridges (5.6).
13. Widths and depths per class (5.4); streams under 6 m get streambeds instead of 14 m stream pieces (5.5).
14. Lake level from the median DEM height along the outline; shores 1:5 (5.5).
15. Barriers at level crossings of tertiary roads and above; signals on the right; platforms 60 m (5.2).
16. Catenary poles later (2).

<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Náves: design

Náves (Czech: village green, the centre a village is built around) is the tool's name. ASCII spelling
`naves` for the crate, binary and repo; "DayZ" stays out of the name (Bohemia's trademark) and goes into the
tagline: *Náves – terrain generator and map editor for DayZ*. Working title before: DayzMapCreator.

Date: 2026-11-04. Status: draft for approval (builder check done, answers merged). Covers milestones 1 and 2 in detail; milestone 3 is only outlined
and gets its own design later.

Background research, spike results and format notes:
`docs/research/2026-11-03-dayz-osm-terrain-research.md` (called "the research doc" below). This document
does not repeat that material; it says what gets built.

Marking: statements without a marker are the user's decisions. **[my call]** marks choices made by the
designer that the user has not confirmed; they can be revisited. **[verify]** marks assumptions about DayZ
that must be checked early.

---

## 1. What this is and why

A public, open-source tool that turns a real landscape (OpenStreetMap + elevation data) into a playable DayZ
terrain, and that is also a full terrain editor for building maps by hand. Think ArmaRealMap/GameRealisticMap,
but for DayZ, with road-piece fitting and terrain shaping done properly, plus an editor in the spirit of
STAG.vision and Terrain Builder/Buldozer, and an AI that can look at the map and fix it.

Why: today a DayZ map means weeks in BI's Windows-only Terrain Builder, placing roads piece by piece and
houses one by one. No tool generates a DayZ map from real data. The user is a DayZ modder on Linux.

Who it serves: DayZ map makers, both

- people who want a real place converted more or less automatically, and
- people who build a terrain from scratch by hand.

Both must be able to keep modifying the map later. The user's own needs set the order of work.

## 2. Goals

1. Convert a real area (OSM + DEM + optional imagery) into a DayZ terrain mod that loads on a server and that
   players can walk and drive around in.
2. Roads, railways and bridges built from DayZ road/rail models that fit each other and the terrain (no bumps,
   no clunky joins); footpaths and mountain trails as ground texture, never as road pieces.
3. Houses chosen by context (city vs village vs farm), not only by footprint size, and turned so their entrances
   face roads, paths, open areas, waterways or rails.
4. Lakes that look right and are deep enough to swim; railways never running through water.
5. Configurable scaling: a horizontal scale and a separate height scale. When a scaled map gets too tight,
   thin out objects only where it is too tight.
6. An editor (GUI) for everything, plus an MCP server, on by default, through which an AI can see renderings and
   data and make the same edits a user can.
7. Every edit survives regeneration; full undo history.
8. Linux first. Windows is a bonus, but the code must not rule it out.

## 3. Non-goals and anti-goals

Non-goals (not planned):

- Updating a project from newer OSM data. OSM is imported once. (Interesting idea, maybe later.)
- Shipping any game data (models, textures, maps). The tool works with the user's own DayZ install.
- Opening or editing the vanilla terrains (Chernarus, Livonia, Sakhal) as your own map. Reading them for
  statistics and as a test oracle is fine.
- Editing binarized `.p3d` models.
- Running trains (DayZ has none; rails are scenery).

Anti-goals (would be a failure even if it technically works). Derived from what the user said; **[my call]** on
the exact wording:

- Losing a manual edit when something is regenerated.
- Log cabins in a city centre, or city blocks on a farm.
- Railways or roads running through a lake or river.
- A lake you can walk through.
- Bumpy, kinked or floating roads.
- Depending on BI's Windows tools in the end state (a temporary bridge in milestone 1 is accepted, see 5.6).
- Code taken from BI tools or the game (clean-room only, see 9).

## 4. Constraints and settled decisions

- **Language:** Rust.
- **Licence:** AGPL-3.0.
- **Platform:** Linux first; Windows a bonus.
- **No BI tools in the end state.** Terrain Builder, Buldozer and the other DayZ Tools get replacements.
  Clean-room reverse engineering where needed (process in the research doc, section 6.9).
- **Native binarizing** (world OPRW v32, models MLOD→ODOL v56) belongs in bzed's armake2 fork
  (https://github.com/bzed/armake2), not in this repo.
- **Milestone 1 may use BI's `binarize.exe` under Proton** as a bridge, for the world file and for generated models.
  The pipeline must make this one replaceable step (5.6).
- **Navmesh:** flare-navgen for now (needs DayZDiag under Proton); our own `.nm` path later.
- **Milestone 1 uses vanilla assets only**, roads included: road pieces only from vanilla, with a small gap or overlap
  accepted within a tolerance where a stretch cannot close (tolerance left to the builder). Generated road models
  (research doc 3.5) for junctions and closures come later. Mod support (scanning, dependency tracking,
  `requiredAddons`) comes later too; the catalog and data model must not rule it out.
- **Road pieces may overlap** (especially straights). Road fitting is an optimisation per stretch from crossing to
  crossing (the research doc, section 3, has the spike and its numbers).
- **Footpaths/trails** (`sac_scale` etc.) never get road pieces.
- **DayZ Tools under Proton work** (AddonBuilder included). Steps that need game data need the game linked into
  the DayZ Tools Wine prefix. Do not design around "DayZ Tools can't run on Linux".
- **Game data access:** read directly from the user's DayZ install and the mods the user chooses (PBOs), never
  copied into the repo or releases. Steam directories are read-only for this tool.

## 5. Architecture

### 5.1 One project, two ways in

A project is one folder holding the area definition, the settings, the imported source data and the edit
history. Generated output is a cache that can be rebuilt. **[my call]** on the folder layout and file formats;
it must be diff-friendly where cheap (text for settings, rules and presets).

- **Automatic start:** pick an area, import OSM + DEM (+ imagery), and run the generators.
- **From-scratch start:** an empty or imported heightmap; the user draws features by hand.

The generators do not care where a feature came from: an OSM way and a hand-drawn line are the same kind of
feature, so a hand-drawn road gets the same road-piece fitting and a hand-drawn forest the same fill as their
OSM equivalents. **[my call]**, consistent with the user's "both must be possible".

### 5.2 Data model

- **Features:** points, lines and areas with a type (road class, rail, river, lake, forest, field, fence,
  power line, settlement, …) and attributes. The source is OSM, the user, or the AI, and an OSM feature can be
  edited.
- **Presets/rules:** how a feature type becomes objects (fill presets in the STAG.vision sense: inside, edge,
  along, fence, decals on a road, power line; density, slope limits, "interrupt at roads", weighted object lists).
  They are shareable text files.
- **Generated objects:** output of (feature, preset, settings, seed). Each one remembers its source. Randomness is
  seeded per position so regeneration is stable.
- **Placed objects:** objects the user (or AI) placed directly, including compositions (a group placed as one
  unit, e.g. a bunker). Generators never touch them.
- **Overrides:** a generated object that was moved, deleted, replaced or re-typed is pinned; regeneration keeps the
  override. Rule from the user: anything that differs from what was generated is kept.
- **Stable identity [my call]:** a generated object is identified by its source (feature id + preset + slot within
  that feature's output), not by position, so an override finds its object again after regeneration.
- **Orphaned overrides [my call]:** when the source of a pinned object changes or disappears (its road was moved,
  its forest deleted, it was dropped for scale), the edit is kept exactly as it is and becomes a placed object, and
  a validation warning points at it. Nothing is re-attached or deleted silently.
- **Terrain stack:** base heightmap (DEM, scaled) → generator constraints (road beds, rail beds, building pads,
  lake beds, river beds) → user/AI height edits → terrain holes. Each layer is stored separately, so a layer below
  can be recomputed without losing edits above. How a user edit is kept when the constraints under it change
  (stored as delta vs as a constraint) is left to the builder; edits must not be lost.
- **Surface layers:** surface mask (materials), satellite image, both derivable and paintable.
- **Edit history:** every change (user, generator run, AI) is an entry in one full undo history, attributed to who
  made it.

### 5.3 Pipeline

A typed, cached layer graph (the ARM idea, research doc 6.2): each layer is computed from its inputs and is
recomputed only when an input changed. Stages, roughly:

1. Ingest: OSM (Geofabrik `.osm.pbf`), DEM (Copernicus GLO-30 by default, national 1 m LiDAR such as Bavaria's
   DGM1 where available), imagery (optional). Reproject to a local metric grid.
2. Scale: apply the horizontal scale and the height scale (5.5).
3. Features: classify OSM into features; merge with user features.
4. Network fitting: roads and rails, plan view then vertical profile (research doc sections 3 and 5).
5. Terrain conforming: constrained least squares on the heightmap (research doc section 5) with roads, rails,
   building pads, water.
6. Objects: buildings (5.4), linear objects (fences, walls, power lines), vegetation, point objects.
7. Thinning for scale (5.5).
8. Surfaces: mask, satmap, terrain layers/tiling, rvmats.
9. Validation: rule checks (5.8).
10. Export: editable world + configs + models → binarize (5.6) → signed mod.

### 5.4 Buildings: type choice and orientation

**Type choice.** The generator picks a model that fits the footprint *and* the context:

- the kind of place (city, town, village, hamlet, isolated farm), from OSM `place`, settlement areas and building
  density
- land use (residential, commercial, industrial, farmyard, military)
- the OSM building tag (house, apartments, barn, church, garage, …)
- road class in front, distance to the settlement centre
- neighbours: houses on one street should look consistent

The catalog therefore needs categories per model (style: city/town/rural/industrial/…; use: residential/farm/
church/…). They come from rules plus a shareable tag file keyed by model/class name (no BI content). Seed it from
how vanilla maps use their buildings (research doc 6.3 and 7). The scoring method is left to the builder.
**[my call]** that the AI (via MCP) mainly tunes rules and presets and fixes areas, and does not place every
object.

**Entrances.** The catalog stores, per building model, its entrances: doors, stairs, gates, each with a position and
facing direction. The placement generator orients buildings so their entrances face something reachable, in
priority order: road, path, open area, waterway, rails. (Priority order: **[my call]**.)

**Milestone 1 [my call]:** doors from configs, a footprint heuristic (long side or door side toward the
access) where nothing is known, and AI-assisted tagging through MCP renders for the vanilla buildings the test map
uses; the result is a seed tag file in the repo. The hand-tagging UI comes with milestone 2.

Where entrances come from (**[verify]**): door positions are likely readable from the house config (`class Doors`)
and the model's memory points; stairs, open doorways and gates without animations are not declared anywhere and
need a tagging step (by hand in the editor, or AI-assisted through MCP: render the building top-down with
candidate points and let the AI pick). Tags are stored in the shareable tag file.

### 5.5 Scaling and thinning

- Two project settings: horizontal scale and height scale, independent (e.g. 1:2 horizontal with 1:1 height
  keeps real heights and doubles slopes).
- Before generating, show what the settings do: slope statistics, roads that would exceed their class's max grade,
  areas that become unwalkable. **[my call]**: a preview in the GUI and an MCP query.
- Buildings and road pieces keep their real size. Where the scaled map is too tight, thin out:
  - always kept: major roads, landmarks (churches, castles, stations, named huts), water, and anything in open land
    (a mountain path with its hut is never thinned)
  - removed first, and only inside overfull areas: ordinary houses, then minor roads and tracks, until the area
    fits again
  - spread removals evenly, so a village gets smaller but keeps its centre and shape **[my call]**
  - in narrow valleys, roads and water win over buildings **[my call]**
  - removed objects stay in the project as "dropped for scale" and can be brought back by hand **[my call]**
- "Overfull" means: objects overlapping each other or roads, or less free space around them than their rules require.
  Exact thresholds are left to the builder.

### 5.6 Binarize bridge (milestone 1)

DayZ needs a binarized world (OPRW v32) and binarized models (ODOL). Milestone 1 writes the **editable world**
(`8WVR`, format known from bis-file-formats) plus configs and MLOD models itself, and hands them to BI's
`binarize.exe` under Proton (the armake2 fork's `--proton-binarize` path, or a sandboxed call as documented in the
dayz-dev skill's `systems/mod-packaging.md`). Later the armake2 fork's native writers replace this step; nothing
before it changes.

**[verify] early in milestone 1:** that `binarize.exe` turns our `8WVR` + CfgWorlds config into a 1.30 world that
loads. If it does not, the fallback is writing OPRW v32 directly (layout is public via dayz-wrp-reader; research
doc section 2) and pulling that work forward.

**armake2 work [my call]:** changes in bzed's armake2 fork (the OPRW writer if the fallback is needed, world-config
rapify round-trips, satmap PAA quality) are part of this plan as a separate track in that repo, not a temporary
copy in this one.

### 5.7 Water

- **Lakes:** a DayZ water object is a flat rectangle at the water level; wherever the terrain inside it is lower,
  water shows. So:
  - inside the lake outline the terrain is lowered into a lake bed. The DEM only gives the surface, so the bed is
    generated, always deep enough that players must swim (minimum depth plus margin; **[verify]** DayZ's swim
    depth). Each lake has a depth setting and the bed can be deepened/shaped by hand.
  - elsewhere inside the rectangle the terrain must stay above the water level; the generator checks this and
    raises ground or uses more, smaller rectangles.
  - long, bent lakes (Königssee) need several rectangles. **[verify]** whether they can rotate and how overlaps
    render.
  - shores get a shore material and a gentle slope.
- **Rivers:** carved bed, monotonically downhill; vanilla river/stream pieces (`river_24_*`, `stream_14_*`,
  research doc section 2) where they fit. Exact method left to the builder.
- **Railways and roads never run through water:** OSM `bridge=*`/`tunnel=*` become bridges/tunnels from DayZ's
  existing bridge models; where OSM has none, route or shape the terrain so the track stays dry. A validation rule
  flags any rail/road segment below water level without a bridge.

### 5.8 Validation rules

A rule engine that finds problems and shows them in the GUI (as STAG.vision shows "162 violations") and through
MCP. Initial rules (**[my call]**, extend freely): road/rail grade over the class limit, road pieces not closing,
rail/road in water, water leaking outside a lake, overlapping buildings, entrance facing nothing reachable,
building on too-steep ground, too-shallow lake, missing assets (mod not loaded).

### 5.9 GUI

Rust native GUI. **[my call]**: egui + wgpu (the same stack STAG.vision uses; proven to handle 3.6 M objects).
Layout ideas from STAG.vision (research doc 6.7 has a description of its screenshots): outliner with layers, 3D
view, asset browser, properties panel, tool panel, warnings list.

- **Milestone 1 (basics):** create a project (area, scales, sources), run generation, view the result in 2D (map) and
  3D (heightmap + satmap + objects as boxes or real models), inspect objects and features, see the scaling preview
  and the validation warnings, export the mod. No editing yet.
- **Milestone 2 (editor):** everything in section 6.

### 5.10 MCP server (on by default)

The editor runs an MCP server whenever a project is open (it can be switched off). Through it an AI can:

- **read:** project settings, features, objects (with source and overrides), presets, rules, catalog entries, the
  validation results
- **see:** renderings on request: top-down map of an area, 3D view from a given camera, false-colour overlays
  (house style categories, slope, surface mask, water depth, violations)
- **act:** the same edit operations the GUI uses (features, objects, presets, terrain, tags), plus re-running
  generation with changed settings. AI edits go into the same undo history and are pinned like manual edits.

A project setting chooses the AI mode:

- **auto:** AI edits apply directly (undoable)
- **approval:** AI edits are proposals, shown in the editor; the user accepts or rejects each

Transport, auth and tool naming are left to the builder; local only by default **[my call]**.

**MCP scope in milestone 1 [my call]:** read and see in full; act limited to what milestone 1 has: project
settings, rules, presets, catalog tags (e.g. entrances) and re-running generation. Object, feature and terrain edits
arrive with milestone 2. The data model (stable object identity, overrides, undo history) is still designed and
built in milestone 1, so milestone 2 adds operations, not a new model.

### 5.11 Game data, catalog and mods

- Read PBOs from the user's DayZ install (milestone 1) and, later, chosen mods (Workshop folders) directly. Build a local catalog: model
  path, class, bounding box, footprint, memory points, entrances, category tags, thumbnail.
- The catalog is per user and is never distributed. Rules, presets and tag files (keyed by names, no BI content)
  are shareable and live in the repo/community. **[verify]** whether this is fine for every mod's assets (short
  legal look; research doc section 8 item 7 lists the related ODbL/EULA questions).

## 6. Milestone 2: the editor

Based on STAG.vision and on Terrain Builder/Buldozer. Must cover, for both automatic and hand-made maps:

- **Features:** draw and edit points, lines, areas; change types and attributes; the generators re-run on the
  affected area only.
- **Objects:** place from the asset browser (click template, click terrain), move/rotate/scale, follow ground, grid
  snap, exact numeric fields, layers, selection by rectangle and brush, replace model, delete.
- **Houses:** place, add, edit; entrance tagging.
- **Presets:** fill presets (inside, edge, along, fence, decals on a road, power line) with preview; editing a preset
  re-applies it to its features.
- **Terrain:** brushes (raise, lower, smooth, flatten, ramp), shapes (pads, ditches, embankments), lake bed depth
  and shaping, auto-flat under placed buildings.
- **Terrain holes and bunkers:** holes editor (DayZ 1.30 `class Holes`/`holes.cfg`), underground objects with the
  underground flag, `cfgundergroundtriggers.json`, and compositions that place entrance + hole + interior +
  triggers as one unit. **[verify]** hole tile size and how the hole edge meets the entrance model (vanilla
  Livonia/Sakhal bunkers; the knowledge pack's `dayz-underground` skill).
- **Surfaces:** paint the surface mask; objects rendered into the satmap.
- **Undo/redo** over everything; approval view for AI proposals.

## 7. Milestone 3 (outline only, separate design later)

Loot, zombies, animals, central economy:

- Loot positions from buildings (`mapgrouppos.xml`; `mapgroupproto.xml` reused for vanilla buildings), tiers from a
  generated, editable area map (`areaflags.map`). **[verify]** the file names and formats.
- Economy setup: investigate existing CE tools and reuse/integrate one if it fits.
- Zombie and animal areas: drawable, editable, generated by default from Chernarus groups assigned by location
  (village/city/farm/military/industrial for infected; forest/open/mountain for animals), written to the territory
  files referenced from `cfgenvironment.xml`.
- Player spawn points, vehicle events beyond the milestone 1 minimum, navmesh (flare-navgen) for AI.
  Navmesh in milestone 3 is **[my call]**: walking and driving don't need it.
- Mod support: Workshop mod scanning, per-object mod tracking, `requiredAddons`/Workshop dependencies.
- Generated road models for junctions and closures (hybrid with vanilla pieces).
- Later items: native OPRW/ODOL writers in armake2 (removing the Proton bridge), own navmesh generator.

## 8. Milestone 1 in detail

**Test map:** Berchtesgaden, the Königssee and the Watzmann. Exact bounding box, map size and default scales are
left to the builder **[my call]**; it is a small test map.

**Done means:** a signed DayZ mod that a local 1.30 server loads, that a client can join, and where a player can walk
and drive around, with:

- the real terrain, scaled by the two settings
- roads from fitted DayZ road pieces, drivable, with bridges where needed
- footpaths and trails as ground texture
- houses chosen by context and facing their access, thinned where too tight
- the Königssee and the rivers, lakes deep enough to swim
- the railway, never through water
- a few vehicle spawns (or documented admin spawning) so driving can be tested
- the GUI basics (5.9) and the MCP server (5.10) working against this project

**Suggested internal order** (each step ends in a map that loads; **[my call]**):

1. The binarize bridge check (5.6) with a heightmap-only world: walkable map.
2. Satmap, surface mask, terrain layers.
3. Roads (plan + vertical + terrain conforming).
4. Houses (type choice, entrances, pads).
5. Water (lakes, rivers).
6. Railways and bridges.
7. Scaling and thinning.

GUI and MCP grow alongside from step 1 (view the map, read the project, request a render).

**Testing** (research doc 6.8): format round-trips against BI tool output, read-back of vanilla worlds, spike-style
metrics as checks (road closure, deviation, grades, cut/fill), and an in-game smoke test on a local server and
headless client (dayz-dev skill's scripts).

## 9. Clean-room and legal

- Formats are implemented from public documentation and from our own observation of files we are allowed to have
  and of tool behaviour on our own inputs. No code from BI tools or the game; no disassembly of BI binaries.
  Process proposal in the research doc 6.9.
- Generated maps contain OSM-derived data: ODbL attribution (and share-alike for the derived database) must be
  written into the mod; same for Copernicus and imagery sources.

## 10. Left to the builder

- Crate structure, project file formats, the layer-graph implementation.
- Road/rail solver formulation (research doc 3.1 lists candidates), terrain solver choice.
- Building scoring method, thinning thresholds, validation thresholds.
- MCP transport and tool names.
- Exact test-map bounds and scale defaults.
- How the GUI looks beyond the STAG.vision-like layout.

## 11. Designer's calls to check

Listed so the user can confirm or overrule them:

1. Hand-drawn and OSM features go through the same generators (5.1).
2. Entrance priority: road > path > open area > waterway > rails (5.4).
3. AI mainly tunes rules/presets and fixes areas; it does not place every object (5.4).
4. Thinning spreads removals evenly, roads and water win in narrow valleys, dropped objects stay restorable (5.5).
5. egui + wgpu for the GUI (5.9).
6. MCP local only by default (5.10).
7. Navmesh in milestone 3 (7).
8. Milestone 1 internal order (8).
9. Milestone 1 MCP: read/see in full; act only on settings, rules, presets, tags and regeneration (5.10).
10. Generated objects identified by source + slot; orphaned overrides kept as placed objects with a warning (5.2).
11. Milestone 1 entrances: config doors + footprint heuristic + AI-assisted seed tag file (5.4).
12. armake2 changes are a separate track of this plan in the fork (5.6).

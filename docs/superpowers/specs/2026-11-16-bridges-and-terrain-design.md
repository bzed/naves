<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Bridges of the right length, over what passes below, on terrain shaped for them

Date: 2026-11-17. Follows `2026-11-16-height-solve-design.md` (merged as d7cb864).

## Why

The maintainer looked at the bridges in the game and said: they are not properly attached to the terrain; a bridge must
cover the road below it and no pier may stand on that road; use bridges without piers where needed; then do the
earthworks that attach the terrain to the bridge. The rule for a conflict between a bridge and the ground: **the bridge
wins, the terrain is adapted to the bridge**, because what a bridge crosses (a stream, a railway) is often not in the
terrain. And the goal behind all road work: roads that are drivable and usable.

Two measurements on the Berchtesgaden example (8.2 km, `terrain_size = 2048`, cell 4 m, `scale.horizontal = 0.5`, at
d7cb864; `/var/tmp/naves-bridgerecon/report.md` and `report2.md`) show where the pictures come from:

- **The bridges are far too long.** OpenStreetMap has 84 bridges here. On the map (lengths are halved by the scale) 5
  are at most 3 m long, 24 between 3 and 6 m, 37 between 6 and 13 m, 10 between 13 and 17 m, 6 between 17 and 21 m and 2
  longer. The shortest bridge model is 25 m. A bridge shorter than its model is grown into the roads beside it
  (`widen_bridges`), by 18 m in the middle case, and the deck is laid straight and level on the chord of the grown
  line: in 32 of 65 the grown part turns by more than 10° (the worst by 126°). Way 495868617, 3.9 m on the map, became
  a 37.4 m deck with one end 13 m inside a hillside. 19 bridges that could not be grown are plain road today.
- **So the spans are buried.** At 64 of 65 the ground under the span is above the deck's underside, at 38 more than
  0.5 m above its surface. All 58 placed bridges shorter than two thirds of their model are buried.
- **A model narrower than its road is scaled up** (the wooden one by 1.183 for gravel), which also lengthens it.
- **The models' ends are solid.** `bridge_metal_25`: blocks 3.4 m long and 16 m wide at both ends, 18.3 m clear between
  them. `bridge_stone_25`: an arch, crown 3.2 m under the deck, about 17.5 m wide at 8.5 m depth. `bridge_wood_25`:
  seven rows of posts 2.6 m long, no abutments. Four spans cross a road; eight solid parts stand on a road below.
- **Shorter parts exist in the game:** concrete panels (`panel_concrete_1`, `_2`, `_dam`: a slab 6.0 × 3.0 m, 0.24 m
  thick, with a roadway, nothing below), `bridge_wood2` (a plain deck 19.7 × 7.6 m, 0.9 m thick, no supports, with
  ports), `bridge_concrete3` (a concrete deck 21.1 × 7.6 m on cross walls about every 3 m), a concrete pipe
  (`misc_concretepipe`, 2.63 m across and 3.13 m long in its collision geometry; 2.46 × 3.04 m is its visual box). The catalog reads only `dz\structures\roads\bridges\` today. Pictures and sizes of
  all of them: the public dataset of samsobjectfinder.com (`/api/v1/objects.full.json`).

## Goal

1. Every bridge of OpenStreetMap is built with a part that fits its length; none is grown around a bend, none is
   scaled, none is lost.
2. A bridge over a road covers that road with its cutting; no solid part stands on a road below.
3. The terrain fits each bridge: no ground above the underside of the deck, the road arrives at the deck on the ground,
   and a road below is on the terrain.

## Non-goals

- Pitched decks: a deck stays level, as the height solve has it.
- Rivers, streams and railways as objects or terrain; painted paths under bridges (they have no height: a path below a
  bridge cannot be told from one that meets the deck).
- The Livonia concrete bridges (`bridge_concrete1`, `bridge_concrete2`: asphalt decks with guard rails, built from a
  25 or 40 m middle part with a pier and two end parts with abutments, 56 m and more together): the right look for
  long road bridges, but none of the lengths found needs them; the catalog change below makes them a small step later.
- The pillar of `bridge_wood2`, chains of `bridge_wood2`, and scaling length and width of an object separately (the
  world file allows it; what the game does with it is untested).
- Bending the road below a bridge so that it crosses squarely (the maintainer wants it as its own task after this one,
  for the crossings that still have a finding).
- Finer terrain than the project's cell.

## Anti-goals

- A bridge grown around a bend, or laid where its ends are not on the road's line.
- A bridge scaled to its road's width.
- A bridge of OpenStreetMap that silently becomes plain road. (At most 3 m it is filled on purpose, see below.)
- Terrain under a span raised above the original ground, or another road's ground cut away for a bridge.

## Approaches

**What a short bridge is built from.** (a) Grow it to a model, as today. (b) Plain road wherever no model fits. (c) A
ladder of parts by length. **The maintainer chose (c)** and named the parts: concrete panels, scaled, as the flexible
small bridge; `bridge_wood2`; a pipe with the road on top (dropped after the game check, see "Changes during
implementation"); terrain fill at 3 m and below; a model only where two thirds
of its length is not longer than the bridge ("2/3 is a guess, you can come up with better values"); no scaling of
bridges, "roads have their fitting bridges, roads might be slightly wider than the bridge".

**Bridges over roads.** (a) Fit the span to what passes below, another part where the model's opening is too short,
a finding where nothing fits; (b) only slide the model. **The maintainer chose (a)**, and bending the road below as a
later task. The first draft's "two `bridge_metal_25_1` back to back" is dropped: a row of panels of the needed length
does the same without piers and without 50 m of straight road.

**Buried spans.** (a) Cut the ground under every span; (b) plain road where there is nothing to bridge; (c) both.
**The maintainer chose (a).** With bridges of the right length the cut is a ditch under a short deck, not a trench.

## Design

All lengths are metres on the map (the models are placed at their own size whatever the project's scale).

### The catalog

- It reads, besides the bridges: the panels (`dz\structures\roads\panels\`; `panel_concrete_1` and `_2`, not the
  broken `_dam` and not the sunken `_legacy`), `bridge_wood2` and `bridge_concrete3`
  (`dz\structures_sakhal\roads\bridges\`, in the game's `sakhal/addons` folder) and `misc_concretepipe`
  (`dz\structures\industrial\misc\`). The maintainer states these are part of every install. A part that cannot be read
  is a warning, and the ladder below goes on without it.
- A part without ports (the panels, the pipe) gets its size from its Geometry and Roadway LODs.
- Per part, measured on the Geometry LOD with its faces (the reader keeps only vertices today): `underside`, the depth
  of the lowest geometry below the deck surface in slices of 1 m along the span; from it `solid` (the slices deeper than
  `MAX_DECK_THICKNESS_M` that reach a port or stand between the ports: blocks, arch flanks, posts), their across
  width, `reach_down`, and `retains` (a solid part at a port that is a wall across the deck's width). The thickness
  over an interval is the largest `underside` in it; it replaces the single `deck_thickness`.
- `MAX_BRIDGE_SCALE` and the scale-up in `bridge_for` go. A bridge may be narrower than its road's piece by at most
  `BRIDGE_NARROWER_M = 1.0`; wider is always allowed.

### What a bridge is built from

`L` is the length of the OpenStreetMap bridge along its line, `W` the width of the road's piece. The first line that
applies:

| `L` | built as |
|---|---|
| at most `FILL_MAX_M = 3.0`, unless the bridge touches a node where only bridges meet | plain road on the terrain. No finding; the report counts them. |
| below two thirds of the shortest model that fits `W`, or a model that has no straight room or would lie on a bent bridge | **panel row** covering the bridge exactly: `n` panels laid crosswise (the 6 m side across the road), one after the other along the bridge's own line from its start to its end, each on the line's heading there, each scaled by `s = L / (3.0 · n)`. `n` is the count nearest the nominal scale: `s0 = max(1, W / 6.0)`, `n0 = max(1, round(L / (3.0 · s0)))`; `n0`, or `n0 − 1` (at least 1) where `n0`'s scale would make the row more than `BRIDGE_NARROWER_M` narrower than the road. Wider than the road is always fine. |
| at least two thirds of a model that fits `W` | **that model**, the shortest that fits: `bridge_wood2` (19.7 m, a plank deck: Grav, Mud), `bridge_concrete3` (21.1 m, a concrete deck 7.6 m wide on cross walls 4.8 m deep about every 3 m, so it never spans a road: Asf3), `bridge_wood_25` (31.6 m with its end ramps; Grav, Mud, and only where the original ground under each post row is within the posts' 2.6 m of the deck), `bridge_metal_25` (Asf1, City), `bridge_stone_25` (Asf2, Asf3). Longer bridges are chains of the family's model as today. |

- "Two thirds" is `MODEL_FILL = 2.0 / 3.0`, one constant. On the example: 5 fills, 61 panel rows, and 18 with a model
  (about 6 `bridge_wood2`, the rest metal, stone or panels by width).
- A panel row never grows and needs no room: it covers the bridge exactly, also on a bend.
- Only a model grows. A model longer than `L` takes the missing length evenly from the road at both ends, but **only
  along a straight line and only on a bridge that is itself straight**: the bridge's own line and the grown parts
  together may turn by at most `GROW_TURN_MAX_DEG = 5`, and a grown part ends 1 m before a junction or another bridge.
  If one side has no room the other takes the rest; if the model still does not fit, or the line turns more, the
  bridge is a panel row. A bridge is never dropped for lack of room.
- At a node where only bridges meet (a fork of bridges), a part reaches the node itself, and a bridge that touches
  such a node is never filled (it is at least one panel).
- A panel row is one deck for the height solve (level, one variable), like a model.

### A bridge over a road

For every bridge stretch, what passes below is every road that crosses its line at another height: the crossings
without a shared node (`over_crossings`), and a road that leaves the bridge's own end node and runs under the deck.
The road below occupies, along the span, `footprint = width / sin(angle)`, plus `PASS_MARGIN_M = 1.0` each side, plus:

- where the part `retains` (metal, stone): the length over which a road crossing at an angle meets the solid ends,
  `across · cot(angle)`;
- where it does not (panels, `bridge_wood2`): the run of the cutting's walls under the deck,
  `(BRIDGE_CLEARANCE_M + thickness) / CUT_SLOPE_MAX` each side.

The bridge must have that interval inside its opening. In this order, the first that works:

1. The part the table gives, slid along the bridge's line within its straight room.
2. A panel row long enough for the interval (it has no solid part and any length), within the straight room.
3. Nothing fits: the placement that leaves the least of the interval under a solid part or outside the deck, and a
   finding `bridge_blocks_road` with the missing metres. These are the cases for the later task that bends the road
   below.

The clearance row of the height solve takes its thickness from `underside` over the road below's interval.

### The terrain at a bridge

`conform` no longer leaves the ground under a span alone. Per bridge, across the deck's width plus one and a half cells
(the reach of a road's core):

- **Under the deck** the ground is held at or below `deck surface − underside − BRIDGE_AIR_M` (`BRIDGE_AIR_M = 1.0`): a
  ceiling. What is higher is cut and the cut spreads sideways with the cutting slopes, so the opening is open at both
  sides. Under a solid part that retains, the same ceiling applies (the model is solid and hides it).
- **At an end that does not retain** the ceiling rises to the road's height at the port over
  `(underside + BRIDGE_AIR_M) / FILL_SLOPE_MAX`.
- **A road below** keeps its own core under the span; its cutting ends at the solid parts where the bridge retains and
  in slopes where it does not.
- **Never** is ground under a span raised above the original ground, and never is a vertex of another road's core
  changed: there the cut stops, and what stays above the ceiling is a finding.

### Findings

- `bridge_blocks_road`: a solid part of a bridge, or the end of its deck, on a road below; with the missing metres.
- `bridge_buried`: after `conform`, ground above the ceiling under a span (another road's core stopped the cut).
- `bridge_post_floats`: a post row or pier that does not reach the ground by more than 0.5 m.
- `bridge_not_built`: an OpenStreetMap bridge laid as plain road because the install has no part for it (no panels
  and no fitting model); a fill of at most `FILL_MAX_M` is none.
- The warning "no room for a bridge model" goes; the report says for every OpenStreetMap bridge what it was built from.

## How it is judged

1. **First, in the game, before the rest is built on it:** a panel row at scale 1 and at scale 2 and a
   `bridge_wood2`, each on a road of the example: one can drive over them, the joint between road and deck is flush,
   and they look like a small bridge. If a kind fails, the table loses that line and the next one takes its lengths.
2. On the example at 2048, against d7cb864: 84 of 84 OpenStreetMap bridges built (65 today); no deck grown around more
   than 5°; spans with ground above the underside 64 → 0, apart from `bridge_buried` findings; solid parts on a road
   below 8 → 0, apart from `bridge_blocks_road` findings; the four bridges over roads keep their clearance.
3. No road finding count rises by more than noise (`road_misses_terrain`, `road_not_drivable`, `road_conflict`,
   `road_grade`).
4. In the game (tools/ingame-check): the four bridges over roads from above and from below, way 495868617, the worst
   buried spans of the first measurement, one bridge of each kind: the deck is free, every bridge can be driven onto
   and over, and under where a road passes, and no part stands on a road.

## Left to the builder

How `underside` is stored and sliced, how a panel row is represented in the placed network, the split of
the work in `conform`, the wording of findings and report lines, and the order of work after the game check of point 1.

## Open points

- Whether vehicles drive cleanly over scaled panels and over the joint of two panels is not known (point 1).
- The wooden models' decks are not in their Geometry LOD; `bridge_wood_25`'s thickness is unknown (2.6 m is its posts).
- A road that is on a fill where it passes under a bridge (Silbergstraße, 1.3 m above its ground) needs no cutting; the
  run of the walls is then counted from the fill's foot.

## Changes during implementation (2026-11-17)

- `bridge_concrete3` has cross walls 4.8 m deep and 1.26 m thick about every 3 m (gaps of 1.73 m), not every
  6 m: it never spans a road below, and the ground under it is held at the walls' foot.
- The pipe is sized from the catalog: 2.63 m across and 3.13 m long measured on its collision geometry. `PIPE_M`
  is not a constant; the culvert rule uses `Pipe::diameter` and `Pipe::length`.
- The underside is stored per slice of 1 m as what hangs (the deepest point of geometry that does not reach the
  part's foot, `Underside::hang`) and what stands (the across intervals of the feet of geometry that reaches to
  within `STAND_TOL_M = 0.3` of the part's deepest point, `Underside::stand`), besides the deepest point over the
  whole width. Solid parts and the opening are exact to the ends of the deep geometry. `PartShape::passable`,
  `ceiling` and `stands` are the queries for a road below and for the terrain; standing slices never pass a road.
  Limitation: one foot depth per part; a model whose piers stand at different depths would need one per pier.
- A road below needs `BRIDGE_CLEARANCE_M` at its centre line and `PASS_CLEARANCE_M = 3.0` over its footprint
  (decided by the controller; Task 6 implements it).
- Under standing geometry the ground is held at its foot, not cut below it.
- **No culvert.** A game check showed that no road fill on the map shows a pipe at both sides (the terrain keeps a
  flat bench beside every road), so pipes are invisible under a road on this terrain: the culvert line of the table is
  dropped (with `CULVERT_MAX_M` and `COVER_M`), and bridges of 3 to 6 m are panel rows. The catalog still reads the
  pipe; nothing uses it.
- **The parts were checked in the game** (panel rows at scale 1.0, 1.67 and 2.0 sit flush and carry; `bridge_wood2`
  and `bridge_concrete3` carry). `bridge_wood_25` is used only where its posts reach the ground: the original ground
  under every standing post (`PartShape::stands`) is within `reach_down` below the deck level and at most
  `POST_ABOVE_DECK_M = 0.5` above it, sampled over each post's rectangle (its corners and every metre along and
  across it); the deck level is the line between the original ground at the two ends of the span.
- **Length and grouping.** `L` is the length of the OpenStreetMap bridge stretch in map metres as the graph has it
  before any growth. Bridge stretches that follow each other (joined at nodes of exactly two stretches, both bridges)
  are one bridge: they are joined into one stretch before the table is applied (the id of the lower outer stretch,
  the surface of the longest part, the highest layer); a ring of bridges is left as it is.
- **Which model.** Of the family's models (shortest first) those the bridge fills to at least `MODEL_FILL`, the
  shortest that is at least as long as the bridge; for a bridge longer than all of them a chain of the longest,
  `ceil(L / length)` models. `bridge_wood2` is never chained (`UNCHAINED_MODELS`): it is laid alone where it covers
  `MODEL_FILL` of the bridge (a model may then be shorter than the bridge; it is centred on it), else the bridge is a
  panel row.
- **The panel row:** the first panel by name (`panel_concrete_1`); its count and scale as in the table
  (`panel_row_size`).
- **Falling back.** A model without straight room, or whose bridge and grown parts turn by more than
  `GROW_TURN_MAX_DEG` together, becomes a panel row of `L`. The turn of the grown parts is counted per end from the
  bridge's own heading at that end, over the stretch that continues it (growth reaches only into that stretch), and
  added to the bridge's own turn. `fit_bridges` returns for every OpenStreetMap bridge what it became and why
  (`BridgeFit`, with the reason the room ends at each end); a fill because the install has no panels and no fitting
  model is the finding `bridge_not_built`. All thresholds are named constants at the top of `bridgekind.rs`.
- **Panel rows cover the bridge exactly** (fix round 1). Rows of whole panels at the nominal scale that had no room
  were shortened and centred, which left gaps of up to 2 m at the abutments on 25 bridges of the example; a row now
  scales its panels to the bridge's length and never grows. A filled bridge is plain road for the room of others.
- **A model needs a straight bridge** (fix round 1): Schornstraße (19.1 m, its own line turning by 14°) had got a
  straight 21.1 m `bridge_concrete3`.
- **Bridges that meet at a node of bridges reach the node** (fix round 1): at the two forks of three bridges on the
  example each bridge was fitted alone, keeping room at the shared node, and one became a fill in the middle of the
  complex.
- **Bridges over dips** use the same straight room: a span may reach the end of its stretch only where the road beyond
  goes on straight (else it keeps 1 m), and an OpenStreetMap bridge built from its own part gets no bridge over a dip.

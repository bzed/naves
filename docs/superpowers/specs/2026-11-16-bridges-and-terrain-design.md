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
  ports), `bridge_concrete3` (a concrete deck 21.1 × 7.6 m on cross walls), a concrete pipe (`misc_concretepipe`,
  2.46 m across, 3.04 m long). The catalog reads only `dz\structures\roads\bridges\` today. Pictures and sizes of
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
small bridge; `bridge_wood2`; a pipe with the road on top; terrain fill at 3 m and below; a model only where two thirds
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
| at most `FILL_MAX_M = 3.0` | plain road on the terrain. No finding; the report counts them. |
| up to `CULVERT_MAX_M = 6.0`, and the original ground under the span lies at least `PIPE_M + COVER_M` (2.46 + 0.5) below the road at both ends | **culvert**: plain road on a fill, and pipes laid end to end across under it at the lowest ground, as many as the fill is wide there. *(this document's call; the maintainer named the pipe as an option)* |
| below two thirds of the shortest model that fits `W` | **panel row**: panels laid crosswise (the 6 m side across the road), scaled by `W / 6.0` (never below 1), `ceil(L / (3.0 · scale))` of them along the bridge's line, centred on the bridge. |
| at least two thirds of a model that fits `W` | **that model**, the shortest that fits: `bridge_wood2` (19.7 m, a plank deck: Grav, Mud), `bridge_concrete3` (21.1 m, a concrete deck 7.6 m wide on cross walls 4.8 m deep: Asf3), `bridge_wood_25` (31.6 m with its end ramps; Grav, Mud, and only where the original ground under each post row is within the posts' 2.6 m of the deck), `bridge_metal_25` (Asf1, City), `bridge_stone_25` (Asf2, Asf3). Longer bridges are chains of the family's model as today. |

- "Two thirds" is `MODEL_FILL = 2.0 / 3.0`, one constant. On the example: 5 fills, 24 culverts or short panel rows, 37
  panel rows, and 18 with a model (about 6 `bridge_wood2`, the rest metal, stone or panels by width).
- A part longer than `L` takes the missing length evenly from the road at both ends, as `widen_bridges` does, but
  **only along a straight line**: the grown part may turn by at most `GROW_TURN_MAX_DEG = 5` in total and ends 1 m before
  a junction or another bridge. If one side has no room the other takes the rest; if the part still does not fit, the
  next shorter kind is taken (a model becomes a panel row, a panel row loses panels down to one). A bridge is never
  dropped for lack of room.
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
- **A culvert** is a road like any other; the pipes are objects in its fill.
- **Never** is ground under a span raised above the original ground, and never is a vertex of another road's core
  changed: there the cut stops, and what stays above the ceiling is a finding.

### Findings

- `bridge_blocks_road`: a solid part of a bridge, or the end of its deck, on a road below; with the missing metres.
- `bridge_buried`: after `conform`, ground above the ceiling under a span (another road's core stopped the cut).
- `bridge_post_floats`: a post row or pier that does not reach the ground by more than 0.5 m.
- The warning "no room for a bridge model" goes; the report says for every OpenStreetMap bridge what it was built from.

## How it is judged

1. **First, in the game, before the rest is built on it:** a panel row at scale 1 and at scale 2, a culvert and a
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

How `underside` is stored and sliced, how a panel row and a culvert are represented in the placed network, the split of
the work in `conform`, the wording of findings and report lines, and the order of work after the game check of point 1.

## Open points

- Whether vehicles drive cleanly over scaled panels and over the joint of two panels is not known (point 1).
- How the pipe looks under a road fill on 4 m cells; if it cannot be seen, the culvert is plain road and the line goes.
- The wooden models' decks are not in their Geometry LOD; `bridge_wood_25`'s thickness is unknown (2.6 m is its posts).
- A road that is on a fill where it passes under a bridge (Silbergstraße, 1.3 m above its ground) needs no cutting; the
  run of the walls is then counted from the fill's foot.

<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# A height solve whose answer can be built

Status: design, 2026-11-18. Written by the assistant on the maintainer's instruction ("Plan that work, we need the
solver to provide a solution that is actually implementable"; order left to the assistant: "Do as you think its the
better order"). Decisions the maintainer made are marked **(maintainer)**; everything else is the assistant's call.

Figures are Berchtesgaden at terrain size 2048 on `main` at da88a23. The feasibility spike is in
`/var/tmp/naves-gridspike/` (`report.md`; the code is a throw-away example in a scratch worktree).

## Why

Today the height solve picks one height per road joint. A separate step (`conform`) then shapes the terrain grid, one
height every 4 m, to those heights. The solve does not know that a grid vertex has one height, or how steep the bank
between two roads may be. Where it asks for more than the grid can carry, the earthwork step lets the higher-ranked
road win and the other road "misses" its terrain:

- `road_misses_terrain` 4285 pieces (1196 by more than 0.5 m, 156 by more than 2 m); about 75 % have another road
  nearby at a different height;
- today's roads lie far from the ground they were given: at the joints p50 0.75 m, p90 2.5 m, max 16 m; the deepest
  cut is 47 m and the highest fill 31 m.

Road pieces drape on the terrain in the game. So what a player drives on is the terrain under the road, not the
height the solve planned.

## Goal

**(maintainer)** The solver's answer is implementable: what it returns is the terrain, and every rule for a road is
kept on that terrain or reported where it cannot be kept.

Roads stay drivable and usable; that is the only goal for roads **(maintainer)**. Short drivable ramps are fine, no
sudden changes of steepness **(maintainer)**. The bridge wins over the terrain **(maintainer)**. An edited part
always wins **(maintainer)**.

## Non-goals

- Where roads run. The line comes from the network clean-up; a hairpin that cannot gain its height stays a reported
  site (changing the line is a different, non-convex problem).
- Which bridge is built, and whether. That stays with the bridge fit.
- Finer elevation data **(maintainer)**.
- The GUI editor; this design only says how an edit enters the solve.

## Anti-goals

- A solve that fails as a whole because one site is impossible. Every rule has a slack, and the slack is the report.
- A second shaping step after the solve that can undo what the solve promised.
- A build that is not reproducible (thread timing, map order).
- Run time or memory that only fits this one map.

## Approaches

1. **The terrain is the unknown (chosen).** The solve works on the grid vertices under and beside the roads; the
   rules are written on the terrain as the game shows it.
2. **Keep heights per joint, add a limit between nearby roads** (their height difference at most what a bank across
   the gap carries). Small, and it meets the main cause of misses, but the grid is still only approximated: curves,
   the lean across a road and three roads meeting stay outside the model.
3. **Loop:** solve, shape, measure the misses, add constraints, repeat. Several passes of a pipeline that takes
   minutes, and no promise that it settles.

Why the first: it is the only one where "can be built" is a property of the answer and not a check afterwards. The
spike shows it is affordable: the whole map as one problem, 456 000 vertices and 3.2 million rows, solved by
`clarabel` in 36 s with 4.5 GB on one thread. At the real limits no rule needed slack, and the roads lay much nearer
the ground (p50 0.11 m, max 8.8 m at the same joints). The spike left out decks, junctions and the grade aim, so
those numbers are a lower bound of the work, not a promise.

## Design

### What is solved

- **Unknowns:** the heights of the grid vertices in the corridors: under every road's footprint and `MARGIN_M` to
  each side. Everything outside keeps the original ground.
- **Objective:** stay near the original ground; keep each road near its family's grade aim (the present aims); keep
  the surface smooth along and across a road. Weights as named constants, in one place.
- **Rules, each with a slack that costs much and is reported when it is used:**

| rule | on what | limit |
|---|---|---|
| grade | along the centre line, sample to sample | the family's grade ceiling |
| grade break | between consecutive samples | `GRADE_BREAK_MAX` 0.10 in ramp zones, the family's vertical radius elsewhere; `GRADE_BREAK_HARD` 0.20 everywhere |
| lean across | left edge against right edge at each sample | `CROSS_FALL_LIMIT` |
| flat across | left, centre, right on one line | a tolerance |
| bank | between neighbouring vertices outside any road | the cut and fill slopes of `conform` today |
| corridor edge | the outermost vertices | equal to the original ground |
| junction | the vertices inside a junction's patch | on one plane (the plane is free) |
| deck | where a deck ends | the terrain at the deck's end equals the deck's height; the deck is level |
| under a deck | the vertices under a span | at most the underside less the air; over a road below, at least the clearance |
| water | vertices of a road beside or over water | not below the water level |
| edit | a vertex or a road sample the user set | equal to the edit **(maintainer: edits win)** |

- **Samples** are taken where the game takes them: the height of a point is read from the grid with the game's own
  rule for a cell (which triangle split it uses is measured in the game first, Task 1), not a guess.
- **Decks** are unknowns too: one height per deck, as today. The clearance row couples the deck with the terrain of
  the road below. Whether a deck may be pitched stays a separate decision.

### What it replaces

- `profile` (the joint heights) and most of `conform` (the shaping). What remains of `conform` is writing the
  solved vertices into the heightmap and the surface mask around roads.
- The rules between roads that exist only because the grid was outside the model: the ties of nearby roads, the
  rank by which one road's earthwork wins, the guard rules at deck ends, the crossing holds. On a shared grid two
  roads that touch share their vertices by construction.
- The findings that mean "the terrain could not follow": `road_misses_terrain`, `road_conflict`, `bridge_buried`,
  `bridge_end_step`, `earthworks_capped`. In their place each rule's slack is a finding with its place and size:
  `road_grade` (grade, break), `road_not_drivable` (lean), `bridge_clearance`, and one new finding for a bank
  steeper than its limit. A site with no slack has no finding.

### Size and time

- `MARGIN_M` decides the size. The spike used 3 cells (12 m). Gentle banks on steep ground need more; the margin is
  therefore per road and follows the side slope of the ground (as `earthwork_allowance_m` does today), capped.
- Expected at 2048: two to three times the spike, about 1 to 1.5 million vertices; measured again in Task 2 before
  anything is replaced. If one problem does not fit the memory cap, the corridor graph is cut at long plain road
  stretches into overlapping regions solved in a fixed order; the spike showed that plain connected components do
  not help (one holds 94 %).
- Deterministic: rows and unknowns are built in sorted order; the solver runs with a fixed thread count.

### How it is introduced

Beside the present solve, not instead of it, until it has been judged:

1. the grid solve is built as its own module and run by the `road-analysis` example next to the present pipeline,
   with the same findings printed for both;
2. a project setting chooses which one the build uses (default: the present one);
3. when the measurements and a game check say so, the default changes, and one release later the old solve and the
   rules listed above are removed.

### Constants

`MARGIN_M` (per side slope, capped), the weights of the objective, the slack costs, the tolerances of "flat across"
and "equal". The limits themselves (`grade_ceiling`, `CROSS_FALL_LIMIT`, `GRADE_BREAK_MAX`, `GRADE_BREAK_HARD`, the
bank slopes, clearance, air under a deck) are the existing constants, used unchanged.

## How it is judged

Berchtesgaden at 2048, the present pipeline against the grid solve, after the network clean-up:

- per rule: how many slacks, their sizes, the worst ten sites;
- how far roads lie from the original ground; cut and fill volume; the deepest cut and highest fill;
- grade and lean along the roads (distribution, share at the ceiling);
- the named bridge sites (39674984, 48450084, 240795460, 74208396, 299428041, 286196692, 130657216);
- run time and peak memory under the 12 GB cap; two runs give byte-identical heightmaps;
- a game check on the sites where the two differ most, the bridge sites, three hairpins and three junction clusters;
- **driving:** the test track of Task 1 gives the measured limits; a car is driven over at least five of the sites.

## Order

After the ramps task (its ramp zone and break limits are reused here) and after the network clean-up (a cleaned
network is a smaller and more feasible problem). The GUI editor's design can assume the "edit" rule above.

## Open points

- How the game interpolates a terrain cell, and whether a road piece drapes along its whole length or only at its
  ends: assumed from vanilla data and from our screenshots, not measured. Task 1 measures both; the sample rule
  follows the result.
- Whether junctions need to be planar or only smooth: planar is the present rule and the safe start.
- Pitched decks would remove more end steps; left out, as before.
- Hairpins that cannot gain their height: reported; a later task may change their line.

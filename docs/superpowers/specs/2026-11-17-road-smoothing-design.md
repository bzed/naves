<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Smoothing the road lines

Status: design, 2026-11-17. Written by the assistant on the maintainer's instruction ("start the smoothing design
now in parallel; create a plan, let sonnet implement it"); no approval gate. Decisions the maintainer made are marked
**(maintainer)**; everything else is the assistant's call and easy to change.

Facts and figures come from the reconnaissance in `/var/tmp/naves-smooth/recon.md` (Berchtesgaden, terrain size
2048, `scale.horizontal = 0.5`, branch `bridges` at 60af4e7). They are re-measured on `main` after the bridges
branch is merged, before any code changes (Task 1 of the plan).

## Why

The road graph keeps OpenStreetMap's point-to-point segments. Only the piece placement smooths them, implicitly:
its search rounds corners while it chains pieces. Every stage that reads the graph line itself sees the corners:

- bridge fitting puts decks and panel grids on the line, so a deck kinks against the road before it (up to 62°
  measured; 30 kinks over 5° remain after the bridge-local lead-in);
- the placement drifts from the line where the line cannot be followed: over 46 022 piece joints the median
  distance is 0.41 m, but 26 % are over 1 m and 5.7 % over 5 m;
- 17.8 % of the corners that turn 1° or more are too tight for any curve piece if rounded on their own, because
  the segments beside them are short (median segment 7.2 m, 5 % under 2 m; tightest curve piece radius 10 m).

## Goal

One smooth centre line per road in the graph, made before bridges are fitted, which all later stages share.
Roads stay drivable and usable; that is the only goal for roads **(maintainer)**.

- **Geometry first, pieces later (maintainer).** The line is smoothed as geometry. The pieces contribute one number
  per road family: the minimum radius, taken from the family's tightest curve piece (10 m for all six families
  today). The placement then fits pieces to the smoothed line as it does now.
- Junction nodes and bridge ends do not move.
- A road stays inside a corridor around its OpenStreetMap line.

## Non-goals

- Hairpins tighter than the pieces can turn are not solved here. They stay where they are (the corridor keeps them);
  the smoothing only opens them as far as the corridor allows.
- Junction geometry (the junction piece, its ports, the three junction exits right at a bridge) is not changed.
- No change to how pieces are chosen or fitted.
- No user option. The values are named constants.

## Anti-goals

- A road that leaves its corridor, cuts a switchback short or moves a junction.
- Output that depends on thread timing or map iteration order.
- A second line: no stage may keep reading the unsmoothed line after this.

## Approaches

1. **Round each corner with an arc (fillet).** Simple and exact, but the arc is limited by half the neighbouring
   segments. On Berchtesgaden 3681 of 20 690 corners (17.8 %) cannot take a 10 m arc that way; City 23.5 %,
   Grav 18.6 %. OSM draws curves as many short segments, and a per-corner rule cannot see the curve.
2. **Build the line from the pieces** (arcs of the catalog's radii). Rejected by the maintainer's decision: geometry
   first. It would also tie every stage to the piece set and say nothing about bridges.
3. **Fit a smooth curve to the whole line inside a corridor (chosen).** Resample the line densely, then move each
   sample sideways so that the line bends as little as possible while staying near the original and inside the
   corridor. This sees a curve drawn as ten short segments as one curve, keeps hairpins in place, and has hard
   guarantees (corridor, fixed points). It is a small convex problem per road; the solver for it (`clarabel`) is
   already a reviewed dependency of `naves-core`.

## Design

### Where it runs

In `generate::fitted_graph`, between `drop_duplicates` and `fit_bridges`. `fit_bridges` then decides bridge kinds
on the smoothed line, and every later stage reads `FittedGraph.graph` as before. Tests that call `build_graph` or
`place_network` directly are not affected.

### Chains

Smoothing works on chains, not on single stretches, so that a road is smooth across the nodes where it merely
continues.

- A chain is a maximal run of stretches joined at nodes where exactly two stretches meet and the turn is at most
  `CHAIN_TURN_MAX_DEG` (150°, the limit the chain pairing already uses).
- A chain ends at a junction (three or more stretches), a dead end, a tunnel end, or a two-stretch node that turns
  more than the limit.
- A ring without such an end is cut at its node with the lowest id.
- Bridge stretches are part of their chain.

### The curve fit

Each stretch of a chain is resampled at equal steps of about `SAMPLE_M` (2.0 m; the step divides the stretch
evenly, and a stretch shorter than one step keeps its two ends). The nodes are always samples.

Each sample `i` gets one unknown, its sideways offset `d_i` along the original line's normal at that sample. The new
point is `p_i + d_i · n_i`. The fit minimises

```
Σ c_i · |q_{i-1} − 2·q_i + q_{i+1}|²  +  W_STAY · Σ d_i²        with q_i = p_i + d_i · n_i
```

subject to:

- `|d_i| ≤ CORRIDOR_M` (2.0 m) for every sample;
- `d_i = 0` at fixed samples.

Fixed samples:

- both ends of the chain if they are junctions (and the next sample too, so the direction in which the road leaves
  the junction is unchanged; the junction code reads it to decide the side of each arm);
- a dead end's position (its direction is free);
- both end nodes of every bridge stretch (the bridge stays over what it spans).

Weights: `c_i = 1` on plain road and `BRIDGE_STRAIGHT_WEIGHT` (10) on samples inside a bridge stretch, so that a
bend moves off the bridge onto its approaches where the corridor allows. `W_STAY` (0.02) is small: the corridor is
the real limit, the term only makes the answer unique on straight roads.

One offset per sample keeps the problem small and stops samples from sliding along the road. The objective is a
convex quadratic and the constraints are bounds, so the result is unique and deterministic. Chains are independent;
they are solved in a deterministic order (and may run in parallel with `rayon`, collected in order).

The smoothed samples replace the stretch's line. Stretch ids, node ids and node positions are unchanged.

### Minimum radius

The fit does not enforce the radius as a hard rule (that would not be convex, and a real hairpin could not satisfy
it inside the corridor). Instead:

- `min_radius(family)` comes from the catalog: the tightest curve piece of the family;
- after the fit, the radius at every sample is measured, and the length of road tighter than its family's minimum
  is reported per stretch and in total (`SmoothReport`);
- that figure, before and after, is how the fit is judged.

### What a failed fit does

If the solver does not converge for a chain, or the input is degenerate (fewer than three samples, NaN), the chain
keeps its original line and the report counts it. Nothing panics.

### Effects on other stages

- **Bridges.** The straight lead-in of the bridges work stays as it is. After smoothing, most bridge ends turn
  under its threshold and need no lead; where the corridor was too narrow, the lead still helps.
- **User edits** are keyed by stretch id, preset and slot. Slots shift when the line changes, so edits on stretches
  whose line moved can become orphaned. This is reported as it is today (`orphaned_edit`) and noted in the changelog.
- **Run time.** The line has about four times as many points (2 m against a median of 7.2 m). If the placement
  becomes more than 20 % slower, collinear samples are dropped after the fit (points whose removal moves the line
  by less than `SIMPLIFY_TOL_M`, 0.05 m).

### Constants (all in one place, `smooth.rs`)

| name | value | meaning |
|---|---|---|
| `SAMPLE_M` | 2.0 | resampling step |
| `CORRIDOR_M` | 2.0 | furthest a road may move sideways from its OSM line |
| `W_STAY` | 0.02 | pull towards the original line |
| `BRIDGE_STRAIGHT_WEIGHT` | 10.0 | extra stiffness on bridge stretches |
| `CHAIN_TURN_MAX_DEG` | 150.0 | above this a two-stretch node ends a chain |
| `SIMPLIFY_TOL_M` | 0.05 | only if the run-time rule above triggers |

## How it is judged

Berchtesgaden at terrain size 2048, before against after, one run each:

- bridge ends: turn of a bridge end against its neighbour (now p50 0.7°, p95 11°, max 83° after leads); `road_kink`
  count and maximum; chain kinks over 5° (now 30, 17 of them inside bent grids);
- how many bridges get a model instead of a panel grid;
- road length tighter than the family's minimum radius, before and after;
- distance of placed piece joints from the line (now median 0.41 m, p95 5.45 m, 26 % over 1 m);
- largest sideways move of any road (must be at most `CORRIDOR_M`), and that no junction or bridge node moved;
- the findings must not get worse: `road_gap`, `road_conflict`, `road_not_drivable`, `road_misses_terrain`,
  `road_grade`, `bridge_clearance`, joins over 0.5 m; and the run time.

A game check with `tools/ingame-check` on about ten sites: bridge ends that kinked, two switchbacks, two city
corners, one straight road as a control.

## Open points

- `CORRIDOR_M` is a guess. At scale 0.5 a real 4 m shift is 2 m on the map. The measurement decides; it is one
  constant.
- The three junction exits right at a bridge and the fork of bridges keep their kinks (junction nodes are fixed).
- Smoothing does not use the terrain. A road that could avoid a steep bank by moving a metre is not moved for it.

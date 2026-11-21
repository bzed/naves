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
bend moves off the bridge onto its approaches where the corridor allows. `W_STAY` (0.02) is small, but it, not the
corridor, is what limits the fit: on a unit right-angle corner the largest offset is 0.97 m of the 2 m allowed, and the
corridor would bind only at about a tenth of that weight (0.002 is the untried candidate: 16 degrees turn, 2.7 m wanted).
The weight was not tuned (see Open points); it also makes the answer unique on straight roads.

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

### Tight curves are widened outward

Added 2026-11-21. The fit above shrinks curves: it penalises the bend between neighbouring samples, the samples
only move sideways, and pulling a curve inward shortens the spacing and so lowers the cost while the radius drops
(measured: a 90° corner comes out at 3.75 m, a hairpin of radius 3.96 m at 2.53 m). On a scaled-down map many real
curves are already tighter than the road pieces can build. The maintainer's decision: such curves are smoothed
outwards to a larger radius; the earthwork this needs is fine, and how far a road moves outward is no concern.

A second pass does this after the fit, chain by chain:

- **Tight zones.** On the fitted line, every run of samples whose radius is below `min_radius(family)` is a tight
  run. Its zone is the run plus `WIDEN_LEAD · min_radius` of road before and after it along the chain (the legs
  must be able to spread before the bend). Zones that touch are one zone. A zone's outside is the side away from
  the centre of its bend (the sign of the summed turn over the tight run).
- **Open corners may cut inward** (maintainer ruling, 2026-11-21). Let θ be the net turn of the fitted line over a
  tight run's zone (the absolute value of the summed signed turns, so resampling kinks and the two bends of an
  S-bend do not add up; for a zone found in a later pass the allowance of the zone it grows from is used). If
  θ ≤ `WIDEN_INWARD_MAX_DEG` (120°), the samples of that zone may also move inward of the original line, up to
  `E = min_radius · (1 − cos(θ/2))`: the deepest distance of a fillet of the minimum radius from the legs (2.93 m for
  90° at 10 m, `R/2` at 120°; not the distance from the vertex, which is `R · (sec(θ/2) − 1)`); above 120° the zone
  is outward only. In the first pass such a zone moves inward only (with both directions free the cheapest way to
  meet the chord-length bound is a 9 m outward detour); the later passes allow both. Measured: a right angle comes
  out at 9.22 m with the legs 1.6 m inward and 0.9 m outward of the original; a 110° corner is not filleted but
  detoured outward (12.5 m, 9.1 m outward, 0.15 m inward).
- **Corridor in a zone.** Unless the zone is an open corner, no sample of a zone may move inward of the *original* line (not of the fitted one: the fit
  pulls curves inward); a sample may retreat toward the original line once an earlier pass pushed it out. Outward it
  may move up to `WIDEN_REACH · min_radius` from the original line. Outside the zones the corridor stays `CORRIDOR_M` on both sides.
- **Radius bound.** For every sample of a zone the second difference is bounded:
  `|q_{i-1} − 2·q_i + q_{i+1}| ≤ h² / min_radius + s_i`, with `h` the sample step and a slack `s_i ≥ 0` that costs
  `WIDEN_SLACK_COST · s_i`. The solver wrapper has only linear rows, so the length is bounded by its projections on
  `WIDEN_DIRECTIONS` (8) evenly spaced directions, which lets the length exceed the bound by at most 1/cos(π/8) − 1 = 8.24 %; the bound is therefore taken as
  `h² / (min_radius · 1.08)`.
- **What is fixed.** Junction nodes (and the next sample), dead ends' positions and bridge end nodes, as in the
  fit; the samples of a bridge deck are held inside a zone (the zone widens the approaches). Nodes where a road merely continues are free inside a zone and move with the line; the graph's crossing
  list is rebuilt from the stretches afterwards, so that every reader of a node's position sees the new one.
  Outside zones nothing changes.
- **Passes.** Moving outward stretches the spacing, so the line is resampled and the pass repeated, at most
  `WIDEN_PASSES` (3) times or until no sample of the chain is tighter than the minimum. Only the samples inside
  zones are resampled, and only between held samples (zone ends, fixed samples, nodes), so held samples and nodes
  keep their positions. A sample that is in a zone keeps its outward side in later passes, so the reverse bends
  of the spreading legs cannot turn the zone around.
- **Where it cannot be done.** A zone whose smallest radius (its samples and the held sample on each side) is still
  below `WIDEN_KEEP_FRACTION` (0.9) of the wanted radius after the last pass goes back to the original line (the resampled OpenStreetMap vertices, since the fit is tighter than the original; its
  boundary samples are held, so this is seamless, and it moves no node). It stays in the report with its position,
  the radius the attempt reached, the radius wanted and `plate_candidate: true`: the bend cannot be built as a curve,
  and these are the candidates for the second allowed form, a concrete plate joining the two legs (maintainer,
  2026-11-21); the plate is its own later task, designed from this list. Between 0.9 and 0.99 the widened line is
  kept (`reached` false, `plate_candidate` false). A chain of more than `MAX_CHAIN_SAMPLES` samples (counted from the
  lengths of its stretches) is neither fitted nor widened and is listed in `kept` as too long.
- **Report.** `SmoothReport` gains, per stretch and in total, the largest outward move and the list of zones with
  radius before, after, and whether the minimum was reached (no zone sample tighter than 0.99 of the wanted
  radius; the held sample on each side of a zone is included). A zone also reports `max_inward_m`. `max_move_m` counts only samples outside zones; the zone samples are in `max_outward_m`. A chain whose
  widening solve fails keeps the fit and is listed in `SmoothReport::widen_failed`.

A bridge stretch inside a zone keeps its ends and its line; the zone widens its approaches.

### What a failed fit does

If the solver does not converge for a chain, or the input is degenerate (NaN, or a line that collapses to one point),
the chain keeps its original line and the report counts it. Nothing panics. A sound stretch with fewer than three samples
has nothing to smooth: it keeps its line and is counted separately (`skipped_short`), not listed among the failures. A
stretch whose line is not finite or collapses to one point is a failure even when it has fewer than three samples.

### Effects on other stages

- **Bridges.** The straight lead-in of the bridges work stays as it is. After smoothing, most bridge ends turn
  under its threshold and need no lead; where the corridor was too narrow, the lead still helps.
- **User edits** are keyed by stretch id, preset and slot. Slots shift when the line changes, so edits on stretches
  whose line moved can become orphaned. This is reported as it is today (`orphaned_edit`) and noted in the changelog.
- **Run time.** The line has about four times as many points (2 m against a median of 7.2 m). If the placement
  becomes more than 20 % slower, collinear samples are dropped after the fit (points whose removal moves the line
  by less than `SIMPLIFY_TOL_M`, 0.20 m; 0.05 m until 2026-11-21).

### Constants (all in one place, `smooth.rs`)

| name | value | meaning |
|---|---|---|
| `SAMPLE_M` | 2.0 | resampling step |
| `CORRIDOR_M` | 2.0 | furthest a road may move sideways from its OSM line |
| `W_STAY` | 0.02 | pull towards the original line |
| `BRIDGE_STRAIGHT_WEIGHT` | 10.0 | extra stiffness on bridge stretches |
| `CHAIN_TURN_MAX_DEG` | 150.0 | above this a two-stretch node ends a chain |
| `SIMPLIFY_TOL_M` | 0.20 | only if the run-time rule above triggers (0.05 until 2026-11-21) |
| `WIDEN_LEAD` | 4.0 | zone length before and after a tight run, in minimum radii (2.0 was measured too short: the legs of a hairpin 8 m apart need about 60 m to spread to a 10 m radius) |
| `WIDEN_REACH` | 2.0 | furthest outward move in a zone, in minimum radii |
| `WIDEN_DIRECTIONS` | 8 | directions that bound the second difference |
| `WIDEN_SLACK_COST` | 1e4 | cost of exceeding the radius bound |
| `WIDEN_PASSES` | 3 | resample-and-repeat limit |
| `WIDEN_INWARD_MAX_DEG` | 120.0 | an open corner (summed turn up to this) may cut inward |
| `WIDEN_KEEP_FRACTION` | 0.9 | a zone below this fraction of the wanted radius goes back to the original line |
| `MAX_CHAIN_SAMPLES` | 200 000 | a longer chain is neither fitted nor widened |

## Tuning round 1

Four rules, each from the first map run. (1) After the stage, a point whose removal moves the line by less than
`SIMPLIFY_TOL_M` (0.20 m; 0.05 m until 2026-11-21) is dropped (Ramer-Douglas-Peucker; ends stay); the figures are
those before the drop.
(2) A zone below `WIDEN_KEEP_FRACTION` goes back to the resampled original line, not to the fit (the fit had pulled
the bend tighter than drawn), with the fit's offset at the two held samples blended out over `WIDEN_REVERT_BLEND_M`
(6 m) of the zone. (3) A zone whose widened line has a sample in water where the original line has none goes back the
same way and is flagged `blocked_by_water` (the water is that of `road_in_water`). (4) Zone samples within
`WIDEN_BRIDGE_HOLD_M` (25 m) of a bridge end are held in the widening.

## How it is judged

Berchtesgaden at terrain size 2048, before against after, one run each:

- bridge ends: turn of a bridge end against its neighbour (now p50 0.7°, p95 11°, max 83° after leads); `road_kink`
  count and maximum; chain kinks over 5° (now 30, 17 of them inside bent grids);
- how many bridges get a model instead of a panel grid;
- road length tighter than the family's minimum radius, before and after;
- distance of placed piece joints from the line (now median 0.41 m, p95 5.45 m, 26 % over 1 m);
- largest sideways move of any road (outside widened zones at most `CORRIDOR_M`; inside a zone up to `WIDEN_REACH` times
  the minimum radius), and that no junction or bridge node moved (nodes where a road merely continues may move inside a zone);
- the findings must not get worse: `road_gap`, `road_conflict`, `road_not_drivable`, `road_misses_terrain`,
  `road_grade`, `bridge_clearance`, joins over 0.5 m; and the run time.

A game check with `tools/ingame-check` on about ten sites: bridge ends that kinked, two switchbacks, two city
corners, one straight road as a control.

## Results

Berchtesgaden at terrain size 2048 (ledger `.superpowers/sdd/2026-11-17-naves-road-smoothing/progress.md`). A is the
base with the stage off, B the first wiring with the stage on, C the placement fix with the stage off, D the placement
fix with the stage on (0.05 m simplification), E D with `SIMPLIFY_TOL_M` 0.20 m.

| Figure | A (off) | B | C (off, fix) | D | E (0.20 m) |
|---|---|---|---|---|---|
| tight length (m) | 16143 | 4237 | | 4088 | |
| `road_gap` count | 368 | 362 | 389 | 369 | 371 |
| `road_gap` length (m), largest (m) | 1526, 131.4 | 2189, 416.5 | 1618, 131.4 | 1615, 104.1 | |
| `road_not_drivable` | 138 | 155 | 141 | 146 | 145 |
| `road_conflict` | 812 | 837 | 797 | 828 | 828 |
| `road_misses_terrain` | 3823 | 3615 | 3752 | 3678 | 3627 |
| `road_in_water` | 8 | 8 | | | 3 |
| `earthworks_capped` | | | | 81 | 69 |
| `road_kink` | 22 | 18 | | 18 | 21 |
| points after the drop | 27894 | 60710 | | 59453 | 31961 |
| placement (s) | 557 | 822 | 549 | 888 | 791 |

Zones in D: 873, of which 578 reached, 41 near misses and 254 plate candidates; no fixed node moved, 54 of 210
two-stretch nodes moved. Quiet machine, same code, stage off against on: 551 s and 568 s against 877 s and 927 s at 0.05
m, and 791 s at 0.20 m (+44 %: the time bar of 20 % is not met; whether the stage ships on is the maintainer's decision). Blank cells
were not recorded.

## Open points

- `W_STAY` stays 0.02: the spec sentence that the corridor limits the fit was wrong, 0.002 is the untried candidate,
  and sharp corners stay tighter and nodes between two stretches keep a kink of about 8 degrees (fixed nodes) until it is
  tuned.

- `CORRIDOR_M` is a guess. At scale 0.5 a real 4 m shift is 2 m on the map. The measurement decides; it is one
  constant.
- The three junction exits right at a bridge and the fork of bridges keep their kinks (junction nodes are fixed).
- Smoothing does not use the terrain. A road that could avoid a steep bank by moving a metre is not moved for it.

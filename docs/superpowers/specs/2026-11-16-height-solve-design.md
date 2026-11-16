<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# One height solve for the whole road network

Date: 2026-11-16. Follows `2026-11-15-road-look-design.md` (merged as e84cb2f).

## Why

Today the heights of the roads are found in steps that each fix what the step before left over:

1. `junctions::find_junctions` gives every junction the ground's height and tilt, and `repair` moves junction heights
   until the links between them fit under their grade ceilings (at most 3 m, 50 rounds).
2. `profile::solve_profiles` solves each chain alone, in placement order, between the fixed junction heights (30 rounds
   of smoothing and clipping, three grade limits tried one after the other).
3. `settle` pulls roads that share terrain cells to one height (10 rounds, at most 5 m) and solves the chains again.
4. `solve_clearing` lifts bridge decks and the junctions of their road until 4.5 m are clear over a road below (up
   to 8 full solves, junctions lifted at most 10 m).

No step sees what the others need. What that leaves on the Berchtesgaden example (8.2 km, `terrain_size = 2048`,
at e84cb2f): 1079 `road_grade` findings (751 bends, 328 grades), 4625 `road_misses_terrain`, 208 `road_not_drivable`,
and in the game the approach wedges at lifted decks (sites 50 and 51: a deck pinned at one height and its junctions
lifted separately, the approach climbs at 23 to 37 %), the ski jump at a deck end (site 23) and the humps of
research 9.8 classes H1 (a hold on one side), H3 (room unused) and F1/F3 (free joints).

## Goal

All heights of the network come out of **one** optimisation problem: every joint height and every junction's height
and tilt are unknowns, every rule is a row, and what cannot be kept is a slack whose value is the finding. It always
has a solution, it is deterministic, and it replaces steps 1 (the repair), 2, 3 and 4.

Non-goals: the plan-view placement (stays as it is), the terrain shaping (`conform`, unchanged; it gets better
heights), retaining walls, the cross fall of arcs, the visible-miss threshold, feeding `conform` conflicts back.
Those stay open items.

Anti-goals: a solve that can fail to end; a result that depends on thread timing or hash order; roads that leave
the ground to make a number smaller.

## Approaches

1. **A quadratic program (chosen).** Convex, one global optimum, exact handling of the limits, slacks give the
   findings. Needs a solver for sparse QPs with about 10⁵ unknowns. Candidate: the `clarabel` crate (interior point,
   pure Rust, Apache-2.0), subject to the dependency review of AGENTS.md.
2. **Difference constraints (Bellman–Ford) for the bounds, then the per-chain solve inside them.** No dependency,
   but it handles only first differences: no vertical bends, no tilts, no cost. The fallback if no crate passes.
3. **More rounds of today's steps.** What road-look did; each task moved a count by ±30 and the wedges stayed.

If `clarabel` fails the review, the same problem is solved by our own ADMM (the OSQP iteration with a
conjugate-gradient inner solve, no factorisation); the problem description below does not change. The solver sits
behind one function for that reason.

## The problem

### Joints and unknowns

The chains, their variables (joints less than 1 m apart merged, the joints of a bridge deck one variable) and the
junctions that hold variables come from the code that exists (`chain_of`, `profile::holds`, `end_joins`). Every
variable `v` of every chain has a height that is an **affine expression** in the unknowns `x`:

| variable | height `h_v` |
|---|---|
| free | its own unknown |
| held by junction `j` (any `HoldRule`) | `z_j + s_j · a_v − drop_v`, with `a_v = u_j · (p_v − c_j)` |
| on a roundabout ring, or held by a junction on a ring | the ring plane's height at `p_v` (a constant) |
| a chain end that lies on another chain (a `Meeting`) and is not held | `(1 − t) · h_a + t · h_b` of that chain's two variables |

- `z_j`: the junction's height at its point `c_j`; an unknown per junction that holds at least one variable.
- `s_j`: the junction's tilt along its through road, rise per metre; an unknown where the junction has a through
  road (unit vector `u_j`, `Junction::tilt_axis`), else the junction is level (`s_j = 0`, no unknown).
- `drop_v`: `OVERLAY_DROP_M` (0.05 m) for a road under the top road of an overlay, else 0.
- `p_v`: the variable's position.

The unknown vector is `x = (free heights, z, s, slacks)`.

### Cost (minimised)

```
  Σ_v  (h_v − g_v)²                          every variable not on a ring; g_v is its ground target
+ SMOOTHNESS · Σ_i (REFERENCE_M² · κ_i)²     every interior variable of a chain
+ JUNCTION_GROUND · Σ_j (z_j − ground_j)²
+ JUNCTION_TILT   · Σ_j (25 · (s_j − s⁰_j))²
+ AIM · Σ_i e_i²                             grade above the aim, see (G)
+ NEAR · Σ_n n_n²                            nearby roads, see (N)
+ HARD · Σ (slack) + HARD · Σ (slack)²       every other slack: f (G), b (B), q (C)
```

- `g_v`: the original ground at `p_v` (for a deck variable the mean of the ground at its two abutments), as today.
- `κ_i = (γ_i − γ_{i−1}) / run_i` with `γ_i = (h_{i+1} − h_i) / L_i`, `L_i = max(spacing_i, MIN_STENCIL_M)` and
  `run_i = (L_{i−1} + L_i) / 2`: the change of grade per metre. `SMOOTHNESS = 30`, `REFERENCE_M = 12.5` as today.
  A deck variable stands for a level deck of length `D`, as in (B): it gets **two** terms, one per deck end, each
  with the change between the grade of the approach and 0 over `run = (L_approach + D) / 2`.
- `ground_j`: `Junction::ground`; `s⁰_j`: the ground's slope along the through road as `junction_gradient` fits
  it today (already cut to its cap). `JUNCTION_GROUND = 2`, `JUNCTION_TILT = 2`.
- `AIM = 1`, `NEAR = 10`, `HARD = 10⁴`. The linear term makes the hard slacks exact: they are zero unless the rows
  cannot all be kept.

### Rows

**(G) Grade**, per segment `i` between two variables of a chain (not inside a deck), with the limits
`aim_i = segment_grades(base = grade_aim)` and `ceil_i = segment_grades(base = grade_ceiling)` (the function that
exists: the meeting grade, the factor in bends, tracks):

```
−(aim_i · L_i + e_i) ≤ h_{i+1} − h_i ≤ aim_i · L_i + e_i        e_i ≥ 0
e_i ≤ (ceil_i − aim_i) · L_i + f_i                              f_i ≥ 0
```

The three limits tried in turn and `RELAX_OFF_M` go: a road is steeper than its aim where the ground asks for it,
at a cost, and never steeper than its ceiling unless nothing else is possible (`f_i > 0`).

**(B) Vertical bend**, per interior variable `i` of a chain, radius `R = min_vertical_radius(family)`:

```
−(run_i / R + b_i) ≤ γ_i − γ_{i−1} ≤ run_i / R + b_i            b_i ≥ 0
```

A deck variable stands for a level deck of length `D`: it gets **two** rows, one per deck end, each between the
grade of the approach and 0, with `run = (L_approach + D) / 2`. (Today the two approaches are compared with each
other as if the deck had no length; that is the ski jump of site 23.)
The cost on `b_i` is `HARD · REFERENCE_M · b_i` and its square (b is a grade, not a height).

**(T) Tilt**, per junction with a tilt: `−cap_j ≤ s_j ≤ cap_j`, `cap_j = min(JUNCTION_TILT_SHARE ·
grade_ceiling(through family), CROSS_FALL_AIM)` as today. No slack.

**(C) Clearance under a bridge over a road**, per over-crossing `c` (`roadnet::over_crossings`) whose bridge model
has a known deck thickness `d_c`, in the same problem as every other row (one solve):

```
h_deck(c) − h_low(c) ≥ BRIDGE_CLEARANCE_M + d_c − q_c             q_c ≥ 0
```

`h_deck(c)` is the height of the deck's variable at the crossing. `h_low(c)` is the height of the road below at the
crossing, `(1 − t) · h_from + t · h_to` of its two variables that bracket the crossing (the place `place_near` finds,
on the same segment that `measure_clearances` measures). Both are affine expressions in the unknowns, with held, ring
and meeting variables as everywhere else. Both roads are free: the bridge's road may rise and the road below may be
lowered, each inside its own rows (G), (B), (T) and costs, so the solve shares the lift between them. `q_c` costs
`HARD · q_c + HARD · q_c²`. A row whose two heights share all their unknowns (one junction holds the deck and the
road below, or both are constants) has only `q_c`. No crossing with a known thickness: no rows (C), and the problem is
that of `solve_profiles` with the same exclusions of (N).

A deck and the road below held by one junction cannot be parted: `q_c > 0`, the `bridge_clearance` finding. Where
the geometry asks more than the limits of both roads allow, the shortfall is shared between `q_c` and the other hard
slacks (a metre of `q_c` costs as much as a metre over a grade ceiling).

**(N) Roads that share terrain cells** (replaces settling). The spots (every 3 m on non-bridge segments) and the
close pairs are found as today (`spots`, `close_pairs`), from the positions and half widths of the placed roads
alone, before any height is solved. Of the close pairs only the **mutually nearest** are kept, chosen before the exclusions below: a pair of a spot `a`
(chain A) and `b` (chain B) stays if `b` is the nearest spot to `a` among `a`'s pairs with chain B and `a` is the
nearest to `b` among `b`'s pairs with chain A (the lower spot index on a tie; for a chain with itself, among the
pairs the `follows` rule lets through). "Share a height" means abreast: pairs with spots at other places along a
slope would flatten roads that run side by side, and the one-sided choice would depend on which road comes first.
For a pair of spots `a`, `b`:

```
allowed = MATCH_M                                              if distance < gap
        = MATCH_M + (distance − gap + cell) · FILL_SLOPE_MAX   otherwise
−(allowed + n) ≤ h(a) − h(b) ≤ allowed + n                     n ≥ 0
```

`h(a)` is the spot's height, linear between its segment's two variables. The pairs of two chains are grouped into
runs (consecutive spots of the first chain, at most `2 · SAMPLE_M` apart). **A run is tied only if the ground lets
it:** at every pair of the run `|ground(a) − ground(b)| ≤ allowed + TIE_STEP_M` (`TIE_STEP_M = 1.0`). A run that
fails gets no rows: roads side by side on a hillside at really different heights stay apart (the maintainer's
caveat; a wall or a conflict finding is the terrain shaping's business). No rows either for pairs of the bridge road
and the road below within `KEEP_APART_M` (50 m) of their crossing. No rows either for a pair of spots of one chain that both lie within `KEEP_APART_M` of a junction that the chain leaves
and re-enters (a loop: the junction holds two separate runs of its variables): there the legs are one surface, and two
spots at different places on a slope legitimately differ in height. Such pairs are not pairs: they do not count in the
run's ground test. Because only mutually nearest pairs are kept, roads that part at an angle (forks, merges, crossings)
keep few pairs, and fewer the wider the angle (25 at 7°); they stay untied beyond those; that is intended: rows (N)
are for roads running side by side.

### What comes out

- `RoadStrip` heights at every joint, as today; `Junctions` with `height = z_j` and `gradient = s_j · u_j`.
- `road_grade` findings are measured on the result as today (`excesses_of`), against the **ceiling** limits and
  `1 / R`, with a tolerance of `1e-4`, and with the deck-end bends measured per deck end as in (B).
- `Profiles::clearances` measured as today; `Junctions::unresolved` (the `junction_too_steep` finding) are the
  junctions that hold an end of a segment with `f_i > 1e-4 · L_i`.
- A solver that does not report a solution (not solved or almost solved) is an error of the build
  (`ProfileError::NotSolved { status }`), not a silent fallback.

### The solver

One function, `qp::solve(&Problem) -> Result<Vec<f64>, QpError>`, for
`min ½ xᵀP x + qᵀx  subject to  lo ≤ A x ≤ hi` with `P` positive semidefinite, both sparse. Requirements: pure
Rust, no `unsafe` of ours, deterministic (single thread, no time limit, fixed iteration limit 200), tolerances
`1e-7` (absolute and relative, gap and feasibility) with heights in metres. Sizes on the example: about 25 000
joints, 10⁵ unknowns with slacks, 2 · 10⁵ rows; a solve must take less than a minute.

## What is removed

`solve_chain`, `smooth`, `bands`, `clip`, `ramp_targets`, the three bases and `RELAX_OFF_M`; `settle`, `resolve`,
`wanted_shifts`, `bound_spans`, `MAX_SETTLE_M`, `SETTLE_ROUNDS`, `LOWER_APART_M`; `DeckFloor`, `deck_lifts`,
`lifted_decks`, `junction_floors`, `lower_road_junctions`, `lower_baseline`, `CLEARANCE_ROUNDS`;
`junctions::repair`, `repair_above`, `raise_floors`, `Link` ceilings as a repair input, `MAX_RAISE_M`, `MAX_MOVE_M`,
`REPAIR_ROUNDS`. Their tests are ported to the behaviour (same scenario, the assertion on the result) or dropped
where they pin a mechanism that no longer exists; the commit says which.

## Constraints

- AGENTS.md in full: test first, no panics on untrusted input, deterministic output, `make check`, coverage ≥ 85 %,
  docs and CHANGELOG with the change, the dependency review written into the commit that adds the crate.
- The maintainer's memory rule for every process started during the work
  (`systemd-run --user --scope -p MemoryMax=… -p MemorySwapMax=1G`).
- Figures are quoted for `terrain_size = 2048`.

## How it is judged

1. Synthetic tests per row type (the plan lists them).
2. `road-analysis` at 2048 against e84cb2f: `road_grade` well below 1079, `road_not_drivable` not above 208 + 30,
   `road_misses_terrain` not above 4625 + 30, the clearances of the four bridges over roads reached as far as the
   grade limits of the bridge's road and the road below allow, analysis time below today's. The only goal is roads
   that are drivable and usable: a bridge nobody can drive under is a failure, whichever road had to move.
3. In the game (tools/ingame-check, the road-look sites): 23 (no ski jump), 50 and 51 (no wedge), 7, 11, 28, 29, 37
   (tilted junctions still right), 1, 2 (unchanged or better).

## Left to the builder

The data layout of the problem (triplets or CSC), how unknowns are numbered (any fixed order derived from chain and
junction order), the split of `profile.rs` into files, and the exact home of `tilt_axis` and the tilt cap in
`Junction`.

## Open after this

Rank-dependent weights (a more important road moving less where two are tied), the weights themselves (`AIM`,
`NEAR`, `SMOOTHNESS` are first values, to be looked at in the game), walls, arc cross fall, the miss threshold.

## Changes during implementation (2026-11-16)

- Smoothness at a deck counts both deck ends (two rows per deck variable, see (B)).
- (C) is one solve with both roads free (see (C)). The two-phase form (phase A without (C), phase B with the road
  below pinned at its phase A height: two pins at the crossing, then also the height and tilt of every junction that
  holds the road below) is removed, with the pins, on the maintainer's word that the only goal is drivable and usable
  roads; "the road below keeps its height" was not their rule.
- The (N) pairs are the mutually nearest spots, chosen before the exclusions (see (N)).
- The only exclusion of a chain's own pairs is its loop through one junction (see (N)); no other connection exclusion.
- `AIM` was measured at 1, 4, 16 and 64 and stays 1. Over-aim length: 39.3 km old, 38.7 / 39.6 / 41.4 / 43.5 km.
  Segments over the ceiling: 413 old, 5 at `AIM` 1. Points more than 4 m off the ground: 3070 old, 844 / 869 / 970 /
  1268. No bridge over a dip at any value.
- The dip thresholds (`DIP_M`, `BRIDGE_DIP_M`) are unchanged.

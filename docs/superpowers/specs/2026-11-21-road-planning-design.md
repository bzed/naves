<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Road planning: re-routing curves the pieces cannot build

Status: design, 2026-11-21. Written by the assistant on the controller's brief, within the maintainer's decisions.
Decisions the maintainer made are marked **(maintainer)**, decisions of the controller **(controller)**; everything
else is the assistant's call and easy to change. Numbers marked **(spike)** are proposals that Task 1 of the plan
confirms or replaces before any production code is written.

The method comes from the clean-room notes `docs/research/2026-11-21-road-tools-clean-room-notes.md`, sections 3.2 to
3.6 (obstacles, profile band, cost field, arc search, margins). Where this design departs from the notes it says so.
No third-party code is used or sought.

Figures are Berchtesgaden at half scale (`scale.horizontal = 0.5`), terrain size 2048, 4 m cells, on `main` at
5137948.

## Why

The road pieces have fixed real sizes. The tightest curve piece of every family has a radius of 10 m on the map. Most
projects scale the world down (1:2 or similar) **(maintainer)**, so a real curve of 15 m becomes 7.5 m on the map,
and the pieces cannot build it.

On the example today:

- 16 143 m of road is tighter than the pieces (on a 2 m resample of the line), in 873 tight zones.
- The placement meets these places and does what it can. What it cannot do shows up as `road_gap` (389, 1618 m in
  total, the largest 131 m), `road_not_drivable` (141), `road_conflict` (797) and `road_misses_terrain` (3752).

The smoothing stage (`2026-11-17-road-smoothing-design.md`) tried to fix this by pushing samples sideways inside a
corridor. It brought the tight length to 4088 m, but 254 zones still could not be built (mostly town corners between
junctions), it does not look at the terrain, it has no guard against running into another road, and it made the
placement 44 % slower. It is switched off **(maintainer)**.

A curve that cannot be built has to be built somewhere else: moved outward or sideways until it has the radius the
pieces need. Moving a road for that is "no problem at all", and the earthwork for it is fine **(maintainer)**. To do
it well the stage must see the terrain, the water and the other roads. That is route planning.

## Goal

Every curve that the pieces cannot build, and that can be built nearby, is re-routed as a chain of arcs and straights
that the pieces can build. Roads stay drivable and usable; that is the only goal for roads **(maintainer)**.

- Only the places where the drawn line cannot be built are planned; everything else keeps its OpenStreetMap line
  **(controller)**.
- A planned curve keeps to the family's grade limit, stays out of water, away from other roads' earthworks, off the
  map edge, and as near the drawn line as the radius allows.
- A bend that cannot be planned keeps its line and is reported with the reason. That list is where the concrete-plate
  task and the curved-bridge task start **(maintainer)**.
- The stage is judged at half scale **(maintainer)**.

## Non-goals

- Whole roads are not routed between junctions. That is a possible v2.
- No bridges inside the search (notes section 4). A bend blocked by water is reported as a curved-bridge candidate
  **(maintainer)**.
- No concrete plates. A bend that cannot be built as a curve is reported as a plate candidate **(maintainer)**.
- No car parks (the task that follows this one) **(maintainer)**.
- No railways.
- The height solve is not replaced. The simple profile of notes section 5.1 is not built. The planning's band only
  decides whether a route is feasible and what it costs; the existing height solve still sets every height
  **(controller)**.
- No change to how pieces are chosen or fitted.
- No user option. The values are named constants.

## Anti-goals

- A junction, a dead end or a bridge end that moves.
- A planned road that runs into water, onto a bridge deck or into the earthwork of another road it did not touch
  before.
- A planned road that strays so far from the drawn one that a player who knows the place no longer recognises it.
- A route with a sudden very short change of steepness that a car cannot take **(maintainer)**.
- Output that depends on thread timing, map iteration order or wall-clock time.
- A zone's failure that changes anything outside that zone.

## Approaches

1. **Plan the tight zones only (chosen).** Find the zones where the line is tighter than the pieces, exactly as the
   smoothing stage does, and search a new line for each zone between two anchors on the same road where the drawn line
   is fine. The search is small (a zone is tens to a few hundred metres), the rest of the road keeps its OpenStreetMap
   line, and a failure stays local. It cannot improve a road that is buildable but badly placed on the terrain, and it
   cannot move a junction to give a hairpin room.
2. **Route whole roads between junctions.** The search sees the whole road and could lay a switchback stack better
   than zone by zone. But every road moves, the search space is a hundred times larger, roads stop being recognisable,
   every user edit keyed by slot is orphaned, and the waypoints that keep a road on its course would have to be
   invented. Kept as a possible v2 once v1 shows what the search does on real ground.
3. **Keep the widening of the smoothing stage and add a road guard.** The smallest change. But the widening is a
   convex fit of sideways offsets: it cannot see terrain, it cannot go round an obstacle (only revert), it shrinks
   curves before it widens them, and on the example it still left 254 bends unbuilt. A guard would only turn more
   widened zones back into failures.

Zones only gives roads that stay recognisable, a search small enough to run on every zone of the map, and failures
that stay local, at the price of leaving bends that need a moved junction to the plate task. The widening stays in the
code, switched off; after this stage is judged it is decided whether it is still needed **(maintainer)**.

## Design

### Where it runs, what it receives and returns

In `generate::fitted_graph_with`:

`build_graph` → `collapse_small_rings` → `drop_duplicates` → `netclean::clean` → **road planning** →
`smooth_graph_in_water` (off) → `fit_bridges`.

The stage lives in a new module `naves-core/src/reroute.rs` (with sub-modules). The name `plan_roads` is taken: it is
the export crate's function that places the roads (`generate::plan_roads`, returning `RoadPlan`). The stage is
therefore called *re-routing* in the code (`reroute_graph`, `RerouteReport`, `REROUTE_ENABLED`) and *road planning*
in the manual and the report (`roads.planning`).

It receives:

- the cleaned road graph (`RoadGraph`: stretches with their points in map metres, node ids at both ends, the
  crossings);
- the original terrain (`Heightmap`, the one bridges are fitted on);
- the minimum radius of each stretch's family (the catalog's tightest arc piece, `PieceCatalog::tightest_arc`;
  none means the family is never planned);
- the water test (`validate::water_test`, the definition of `road_in_water`);
- the parameters (the constants below, as a struct so tests can change them).

It returns a new `RoadGraph` and a `RerouteReport`. Stretch ids and node ids never change. Stretches outside zones are
equal bit for bit. With the switch off (`REROUTE_ENABLED = false`) the stage is not called and the graph is the
cleaned graph, bit for bit. The four combinations (planning off/on × smoothing off/on) can be chosen per call
without recompiling, so that `road-analysis` can compare them with the same code.

### Chains and tight zones

Zones are found exactly as the smoothing stage finds them **(controller)**, but on the cleaned graph's lines, not on a
fitted line:

- A **chain** is the smoothing's chain: a maximal run of stretches joined at nodes where exactly two stretches meet,
  turning by at most `CHAIN_TURN_MAX_DEG` (150°), with the same tunnel flag. It ends at a junction (three or more
  stretches), a dead end, a tunnel end or a sharper two-stretch node. A ring is cut at its node with the lowest id.
  The smoothing's chain code is reused, not copied.
- Each chain is resampled at `SAMPLE_M` (2 m) per stretch; nodes are samples.
- A **tight run** is a run of samples whose three-point radius is below the minimum radius of the stretch they belong
  to.
- A **zone** is a tight run plus `PLAN_LEAD` minimum radii of chain before and after it. Zones that touch are one
  zone. A zone is clipped at the ends of its chain.
- Because the zones are found on the unfitted line, their count differs from the 873 the smoothing found on its
  fitted line. Task 1 counts them.

Not planned at all (listed in the report as skipped, with the reason): a tunnel chain, a roundabout stretch, a
stretch whose line has a non-finite coordinate or collapses to one point, a family with no arc piece, and a zone longer
than `PLAN_MAX_ZONE_M`.

### What is fixed, anchors and legs

What never moves **(controller)**:

- a junction node, and the direction in which each of its arms leaves it;
- a dead end's position (its direction is free);
- a bridge end node, the bridge stretch itself and the first `PLAN_BRIDGE_HOLD_M` (25 m) of road on each side of it
  along the chain;
- a tunnel end node and its first 25 m, held like a bridge end (an addition: a portal is as fixed as a deck);
- the node where a ring chain is cut, with the line's heading there (an addition: it is where the chain starts and
  ends, so it is held like a junction).

An **anchor** is a pose: a point on the original line and the line's heading there. The heading is measured over one
sample step on each side of the point (at a chain end, over the first step). Each zone has an anchor at each end:

- where the zone ends inside the chain: the sample at the zone's end (by construction not in a tight run);
- where the zone reaches a junction: the junction node with its arm's direction (the direction from the node to the
  next sample). The leg's route begins with a straight of `PLAN_JUNCTION_STRAIGHT_M` along the arm, so that the
  junction piece has a straight arm to sit on (an addition, **(spike)**);
- where it reaches a dead end: the dead end's position with a free heading;
- where it reaches a held part (bridge, tunnel, ring cut): the end of the held part, with the line's heading there.

A held part inside a zone splits the zone into **legs**, each between two anchors. A leg is planned on its own.

A junction is never inside a chain, so a bend whose apex is a junction (a hairpin of the through road at a junction)
gives two zones on two chains, both anchored at that junction with their arm directions kept **(controller)**. They are
planned independently and reported as one bend: each zone names the junction and its partner zone.

A leg whose two anchors are closer than two sample steps, or whose route would have to start inside the junction
straight of the other end, is not searched (reason `held`).

### Obstacles and the distance raster

Notes section 3.2, built once per zone over the zone's box: the bounding box of the zone's original line, grown by
`PLAN_STRAY_MAX` minimum radii, the largest clearance and two raster cells.

- **Resolution:** half the terrain cell, clamped to 1 to 2.5 m (2 m at 4 m cells), multiplied by 1.25 until the
  raster has at most `PLAN_RASTER_MAX_CELLS` cells.
- **Water:** a raster cell whose centre is in water by `water_test`.
- **Other roads:** every stretch of the graph that is not part of the zone, including the zone's own chain outside
  the zone (the other leg of a switchback is an "other road" here). Each is stamped with its earthwork: every cell
  within `core_reach_m(half its width, terrain cell)` of its centre line (the clean-up's measure, `conform::core_reach_m`).
  Cells a segment touches are found by an exact grid traversal (Amanatides–Woo), then widened by the distance map.
- **Exempt roads:** a road whose earthwork the zone's *original* line already comes within the road clearance of is
  not stamped (a side road at an anchor, a road the zone already runs beside) **(controller)**. Its centre line still
  counts as a line the route may not cross where the original line does not: the number of times the route crosses
  each exempt road must equal the number of times the original zone line does.
- **Bridge decks and tunnels:** stamped like other roads, never exempt. A zone whose original line passes over or
  under a deck or a tunnel is not planned (reason `held`): the crossing must stay where the bridge is.
- **Map edge:** every cell within `core_reach_m(own half width)` of the map edge, and everything outside the map.
  The heightmap has no no-data value; outside the map counts as no data.
- **Distance maps:** a two-pass chamfer transform (orthogonal weight = resolution, diagonal = resolution · √2), stored
  in decimetres in 16 bits, saturating. One map for water, one for roads (decks, tunnels and the edge are in the road
  map), and one more of the distance to the zone's original line (for the stray term and the stray cap).
- **Clearances:**
  - roads: `core_reach_m(own half width) + one terrain cell`; together with the stamped earthwork this is the
    clean-up's "earthworks touch" test (two reaches and a cell);
  - water: `core_reach_m(own half width)`, but never more than the original zone line's own smallest distance to
    water, so a riverside road may stay as near the water as it already was (an addition).
- **Segment test:** walk every raster cell the segment touches; it is blocked if a cell is an obstacle or its
  distance is below `(clearance + res) · CHAMFER_ERROR + res · √2`. A passing segment keeps at least the clearance.
  A segment with a non-finite end is blocked.

The search, the band and the field ask one question of the raster: is this segment clear, and if not, of what
(water, road, edge). The answer's kind is what the failure reasons are built from.

### The feasibility band

Notes section 3.3, unchanged in method:

- Every segment of a step must pass the segment test.
- The step is sampled at half the terrain cell, clamped to 0.1 to 5 m (2 m on the example).
- At a sample with terrain height `z` the band is `[z − PLAN_CUT_M, z + PLAN_FILL_M]`.
- The reachable interval starts as the band at the leg's start anchor and propagates: `[lo − g·ds, hi + g·ds]`
  intersected with the band. Empty (lo > hi by more than 10⁻⁹) means the step is impossible.
- `g` is `limits::grade_ceiling(family)`: the hard grade the height solve also keeps. The notes search at 90 % of the
  grade first because their pieces are fitted in the loop; here the placement and the height solve come later, and the
  ceiling is already 90 % of what a vehicle climbs, so there is one pass (a departure).
- Along the way: the nominal height `h` (terrain clamped into the interval), the climb `Σ|h − h_prev|` and the
  earthwork `Σ|h − z| · ds` (m²).
- A sample with no terrain (outside the map, or a non-finite height) fails the step.

The band is forward only. It says whether a route can keep the grade from the start; the height solve later sets the
heights and keeps the vertical radius (`limits::min_vertical_radius`) and the grade-break rules. The band does not
model vertical curvature; if the judging shows planned routes that the height solve can only make with sharp grade
breaks, that is an open point.

If the original zone line itself fails the band, the zone is still searched, and the report says so
(`original_feasible: false`).

### The cost-to-go field

Notes section 3.4, one field per leg, over the zone box:

- Grid resolution `max(PLAN_STEP_M, terrain cell)` (4 m), multiplied by 1.25 until there are at most
  `PLAN_FIELD_MAX_CELLS` cells.
- Dijkstra from the cell of the leg's end anchor; the start cost is the distance from the cell centre to the anchor.
  8-connected.
- An edge between neighbouring cell centres is allowed if the band test passes with the grade limit removed
  (cut, fill, no data and obstacles still apply), run in the direction of travel, and both cells are within the stray
  cap.
- Edge cost: since cut + fill > 0, it is the distance (the notes' rule for that case).
- Estimate at a point: the cell's value divided by `CHAMFER_ERROR`, minus one field cell, not below 0. A cell without
  a value gives 0.

The field is a lower bound on the length still to go around obstacles. It is what makes the search go round a lake
instead of filling the space in front of it.

### The search for one leg

A hybrid A* over poses (notes section 3.5), from the start anchor's pose to the end anchor's pose.

**State:** position, heading, cost so far, the reachable height interval, the parent, whether it has arrived.

**Steps:** from each state, five arcs of length `s = PLAN_STEP_M`, with heading changes `0, ±θ/2, ±θ`,
`θ = min(s / R, 45°)`. `R` is the planning radius: the family's tightest arc times `PLAN_RADIUS_MARGIN` (1.05, the
notes' margin, section 3.6). So no step curves tighter than `R`. An arc is sampled about every 2° (at least one
segment) for the tests. In a zone whose stretches belong to more than one family, `R` is the largest of their
radii and `g` the smallest of their ceilings.

**Arrival at a pose (a departure).** The notes arrive at a point with a free heading; here the end anchor has a
heading the untouched line continues with. From every state within `PLAN_ARRIVE_REACH` of the end anchor the search
tries the six Dubins paths of radius `R` (two arcs and a straight, or three arcs; Dubins 1957, the analytic expansion of
hybrid A* in the public literature) from the state's pose to the end pose, in order of length. The first that passes
the band and obstacle tests and stays within the stray cap is an *arrived* successor, with its full step cost and its
position and heading set exactly to the anchor's. At a dead-end anchor (free heading) the notes' single arc through
the point is tried instead: allowed when the angle α between the heading and the direction to the point is under 90°
and its radius `c / (2·sin|α|)` is at least `R`. The start of a leg at a junction pose first takes the junction
straight as a fixed first step.

**Start:** one state at the start anchor with its heading and the band there as its interval. At a dead-end start
(free heading), one state per heading bin.

**Cost of a step** (in metres of road):

```
length + W_SLOPE · climb + W_CURVE · |δ| · R + W_EARTH · earthwork + W_STRAY · Σ d · ds
```

`δ` is the step's heading change; `d` is the distance of a step sample from the zone's original line (from the
distance map). The curvature term prefers straight runs (notes). The stray term keeps the planned road near the
OpenStreetMap line **(controller)**. Its weight is chosen by Task 1: among the candidates, the largest at which every
leg found at the smallest candidate is still found and its length grows by at most 10 %. That pulls routes as near the
line as they can get without losing routes or adding much road.

**Stray cap:** a step with a sample further than `PLAN_STRAY_MAX · R` from the zone's original line is not taken
(proposed 3 minimum radii, about 31 m; the smoothing allowed 2). It bounds the search and keeps roads recognisable.

**Heuristic** (0 for an arrived state): the larger of the field estimate and the notes' analytic bound,
`max(distance, Δ / g) + W_SLOPE · Δ` with `Δ = max(0, |z_end − z_here| − PLAN_CUT_M − PLAN_FILL_M)`.

**Duplicates:** the state key is (arrived, position in cells of `s / 2`, heading bin); the number of bins is
`⌈2π / (θ / 4)⌉`, clamped to 32 to 180 (66 on the example). A new state is dropped if a stored state with the same key
costs no more; otherwise it replaces it. A popped state whose priority is above its cost plus the weighted heuristic
is stale and skipped.

**End:** the first arrived state that is popped ends the search; its path is the route. There is one leg per search,
so the notes' arrivals per heading sector and their stop rules are not needed (a simplification).

**Budgets:** each leg is tried with (heuristic weight, budget of expanded states) `PLAN_BUDGETS`: first exact, then
weighted. The next pair is tried only when the budget ran out. An empty open set proves there is no route and ends
the leg at once. Budgets are counts of states, never wall time.

**Tie-breaking:** the open set is ordered by priority, then cost, then the state key, then the order of insertion, all
compared with `total_cmp` on floats. States live in an arena indexed by insertion order. The key table is a sorted map
or a dense array over the zone box; no hash map is iterated.

### Writing the route back into the graph

A planned leg's route is written as points: every arc sampled at `SAMPLE_M` (2 m) or finer, a straight as its two
ends. The route is not simplified: a coarser polyline of an arc measures tighter than the arc (a 4 m chord sampled
back at 2 m reads as half the radius), and the tight length is measured on 2 m samples.

- The chain's line from the start anchor to the end anchor is replaced by the route. An anchor that lies between two
  original vertices is inserted as a point; outside the legs, every original point stays.
- A node where the road merely continues (two stretches) and that lies inside a leg moves to the route point at the
  same fraction of the leg's length as it had on the original line. Its two stretches are cut there. The node keeps its
  id.
- The crossing list is rebuilt from the stretches (`crossings_of`), so that every reader of a node's position sees
  the new one.
- `bridge`, `bridge_kind`, `bridge_offset`, family, layer and every other attribute stay on their stretches. No
  stretch is added or removed.

Before it is accepted, every route is checked: every three-point radius on its 2 m samples at least
`PLAN_REACHED_FRACTION` of the family's tightest arc; the end pose met within 10⁻⁶ m and 10⁻⁶ rad; every sample within
the stray cap; every segment clear. A route that fails the check is a bug; the leg keeps its line with reason
`no_route` and the detail `check_failed`, and a test must pin it.

**Routes against each other.** Zones are searched independently against the original graph, in parallel. Then, in
zone order (by the chain's first stretch id, then along the chain), each accepted route is checked against the routes
accepted before it: a route whose earthwork comes within the road clearance of an earlier accepted route, where the
two original lines did not, is reverted with reason `blocked_by_road`. This is sequential and deterministic.

### Failure and reporting

A leg that is not planned keeps its original line. Nothing else changes: not the other leg, not other zones. Nothing
panics.

Reasons **(controller)**:

- `held`: a junction, a bridge or tunnel end, a dead end or the ring cut leaves no room. Used when the leg could not be
  searched (anchors too close, a deck crossing inside the zone) and when a search that starts or ends at a fixed pose
  fails and the tight run lies within `PLAN_LEAD` of that fixed pose.
- `blocked_by_water`: no route was found, and a diagnostic search without the water obstacle finds one.
- `blocked_by_road`: no route was found, and a diagnostic search without the road obstacle finds one; or the route
  was reverted against an earlier accepted route.
- `no_route`: none of the above. The report says whether the budget ran out (`budget_exhausted`) or the search proved
  there is none.

The diagnostic searches run only for failed legs, each with the last budget pair only. They are what makes the
reason honest; they cost time only where the stage already failed.

Each zone in the report has: its id (`<first stretch id of the chain>#<n>`, n counting along the chain), where it is
(its tightest sample), its stretches, its legs, the anchor kinds at both ends, the junction and partner zone where the
bend passes a junction, the outcome (`planned`, or `kept` with the reason and a detail), the radius before (the
smallest on the original line), wanted (the tightest arc) and after (the smallest on the route; the radius before
when kept), the length before and after, the largest distance from the original line, the climb, the earthwork, the
states expanded (with retries and diagnostics), the heuristic weight that found the route, `original_feasible`, and
the candidate the bend is for: `plate` for `held` and `no_route`, `curved_bridge` for `blocked_by_water`, none for
`blocked_by_road` or a planned zone.

Totals: zones by outcome and reason, tight length before and after (measured as the smoothing measures it), the
largest distance from the original line, states expanded, the chains and stretches skipped with reasons. The time
of the stage is measured by the caller beside the report (like `smooth_time`), never in it.

Outputs:

- `build/report.json`: `roads.planning`, with the totals and every zone; `{"enabled": false}` when the stage is off.
- Build summary: one line, for example `Road planning: 900 zones, 610 planned, 290 kept (no_route 120, held 110,
  blocked_by_water 25, blocked_by_road 35), tight length 16143 m -> 3900 m, largest move 24.0 m`.
- A progress line with the stage's time.
- Findings: `road_curve_planned` (information) for a planned zone, at its tightest point, with the radius before and
  after and the largest move; `road_curve_not_planned` (warning) for a kept zone, with the reason and the radius it
  has. New rules rather than `road_curve_too_tight`, so that planning and smoothing do not report the same bend twice
  when both are on.
- `road-analysis` prints `PLAN …` lines (totals, every kept zone) and `TIMING reroute`.
- The manual (getting started, troubleshooting, the MCP rule list) and `CHANGELOG.md`.

### Determinism

- Same input, same graph, bit for bit, on repeated runs and with any thread count **(controller)**.
- Zones are found in chain order. Zones are searched in parallel with `rayon` and collected in zone order; each search
  is single-threaded. The cross-check of routes is sequential in zone order.
- All budgets are counts of states. No wall time enters a decision.
- Floats are compared with `total_cmp`; sums are taken in a fixed order; no hash-map iteration.

## Constants

All in `reroute.rs`, all `pub const`, collected in `RerouteParams::default()`.

| name | value | meaning | from |
|---|---|---|---|
| `REROUTE_ENABLED` | `false` until judged | the switch; off, the stage is not called | controller; the judging proposes the default |
| `SAMPLE_M` | 2.0 m | resample of the lines for finding zones and for writing arcs back | `smooth::SAMPLE_M`, reused |
| `CHAIN_TURN_MAX_DEG` | 150° | a two-stretch node that turns more ends a chain | `smooth`, reused |
| `PLAN_LEAD` | 4.0 minimum radii | road before and after a tight run that belongs to its zone | `smooth::WIDEN_LEAD`; **(spike)** 2 to 6 |
| `PLAN_RADIUS_MARGIN` | 1.05 | planning radius over the tightest arc | notes 3.6 |
| `PLAN_REACHED_FRACTION` | 0.99 | a written route has no sample tighter than this fraction of the tightest arc | the smoothing's reached rule |
| `PLAN_STEP_M` | 4.0 m | length of a search step (one terrain cell) | **(spike)** 2, 4, 6 |
| `PLAN_MAX_TURN_DEG` | 45° | largest heading change of one step | notes 3.5 |
| `PLAN_HEADING_BINS` | 32 to 180 | clamp on the number of heading bins, `⌈2π / (θ/4)⌉` | notes 3.5 |
| `PLAN_ARRIVE_REACH` | `max(3 · s, 4 · R)` | from this distance of the end anchor, Dubins arrivals are tried | notes 3.5, adapted; **(spike)** |
| `PLAN_JUNCTION_STRAIGHT_M` | 6.25 m | straight at a junction anchor, half a junction piece | assistant; **(spike)** |
| `PLAN_BRIDGE_HOLD_M` | 25.0 m | road held on each side of a bridge or tunnel end | `smooth::WIDEN_BRIDGE_HOLD_M`; controller |
| `PLAN_STRAY_MAX` | 3.0 minimum radii | furthest a route may stray from the original line | assistant, proposal; **(spike)** 2 to 4 |
| `PLAN_CUT_M`, `PLAN_FILL_M` | 6.0 m each | band of road heights around the terrain | assistant; **(spike)** 3, 6, 12 |
| grade `g` | `limits::grade_ceiling(family)` | hard grade of the band | `limits.rs` |
| `W_SLOPE` | 4.0 | metres of road per metre of climb | notes 3.5 |
| `W_CURVE` | 0.1 | times `|δ| · R`: prefers straight runs | notes 3.5 |
| `W_EARTH` | 0.5 | metres of road per m² of earthwork | notes 3.5; **(spike)** 0.25, 0.5, 1 |
| `W_STRAY` | 0.1 | metres of road per m² between the route and the original line | assistant; **(spike)** 0.05, 0.1, 0.25, 0.5, by the rule above |
| `PLAN_BUDGETS` | (1.0, 20 000), (1.5, 40 000), (3.0, 80 000) | heuristic weight and expanded states per try | notes 3.5 scaled to zone size; **(spike)**: four times the largest count to the first arrival at weight 1 |
| `PLAN_MAX_ZONE_M` | 400 m | a longer zone is not planned | assistant; **(spike)** |
| raster resolution | half the terrain cell, 1 to 2.5 m | obstacle raster | notes 3.2 |
| `PLAN_RASTER_MAX_CELLS` | 4 000 000 | per zone; the resolution grows by 1.25 until it fits | assistant (notes: 40 M for a whole map) |
| field resolution | `max(s, terrain cell)` | cost-to-go grid | notes 3.4 |
| `PLAN_FIELD_MAX_CELLS` | 1 000 000 | per leg; grows by 1.25 until it fits | notes 3.4 |
| `CHAMFER_ERROR` | 1.0824 | chamfer and octile excess over true distance | notes 3.2, 3.4 |
| band spacing | half the terrain cell, 0.1 to 5 m | samples of the band test | notes 3.3 |
| road clearance | `core_reach_m(own half) + cell`, other road stamped at its `core_reach_m` | earthworks touch | `netclean` sidepath step, `conform::core_reach_m` |
| water clearance | `min(core_reach_m(own half), original line's distance to water)` | | assistant |

## How it is judged

Berchtesgaden at half scale, terrain size 2048. Four runs of `road-analysis` with the same binary: planning off/on ×
smoothing off/on. Compared on:

- road length tighter than the pieces, before and after (baseline 16 143 m; smoothing alone 4088 m);
- zones by outcome and reason; the list of bends that could not be built, with their candidate (plate, curved
  bridge);
- the largest and the 95th-percentile distance of a planned route from the original line;
- `road_gap` (count, total length, largest; baseline 389, 1618 m, 131 m), `road_not_drivable` (141),
  `road_conflict` (797), `road_misses_terrain` (3752), and every other rule count;
- that no junction, dead end, bridge end or tunnel end moved (must be 0);
- placement time (baseline about 550 s) and the planning's own time, separately.

Proposed bar for switching the stage on (the maintainer decides): tight length at most half the baseline with
planning alone; none of `road_gap`, `road_not_drivable`, `road_conflict`, `road_misses_terrain` worse by more than
5 % without a reason that can be named; planning under 60 s on the machine's threads; placement not more than 10 %
slower.

Then a game check with `tools/ingame-check` on about ten sites, planning off against on: two switchbacks, two town
corners, the hairpin at a junction, the bend beside water, two kept zones (one plate, one curved-bridge candidate), one
zone with a moved two-stretch node, one straight road as a control.

## Effects on other stages

- **The clean-up before it.** Planning sees the cleaned graph. The clean-up's new nodes (sidepath joints, merged
  junctions, bridge approaches) are junctions like any other and become anchors. A bridge approach made by the
  clean-up is within the 25 m hold.
- **Smoothing after it.** When both are on, the smoothing sees the planned lines. Its fit pulls curves inward and can
  make a planned arc tighter again; its widening then finds fewer zones. The four-run comparison shows how much. If
  the fit tightens planned arcs, the fix is for the smoothing to hold the samples of planned legs (open points).
- **Bridges.** No bridge stretch and no road within 25 m of a bridge end moves, so `fit_bridges` sees the bridges as
  before. Its straight lead-ins are unchanged.
- **Placement.** It fits pieces to the planned line as to any line. Planned arcs have 1.05 times the tightest arc's
  radius, so the fitter has a curve piece for them. The line has more points inside planned zones (2 m on arcs).
- **Height solve.** Unchanged. It receives the placed pieces and sets every height. A planned route was feasible in
  the band, so the solve should find the grade; what it does with vertical curvature is watched in the judging
  (`road_grade`, `road_not_drivable`).
- **Dips and bridges over dips** (`find_dips`, `with_bridge_spans`) run on the placed planned roads as before.
- **User edits** are keyed by stretch id, preset and slot. Ids do not change, but slots on a stretch whose line moved
  shift, so edits there can be orphaned. They are reported as today (`orphaned_edit`). Switching the stage on is a
  **breaking** change for such edits and the changelog says so in the release that switches it on.
- **Sites and in-game check.** `naves_export::sites` may pick planned zones as sites; not needed for v1.

## Open points

- **Streams.** `water_test` holds water areas only (the `road_in_water` rule). A re-routed switchback can cross a
  stream that its drawn line did not cross, which makes a dip or a culvert. Proposal: treat a stream that the zone's
  original line does not cross as an obstacle line. Task 1 counts how often it happens before this is decided.
- **Buildings.** Náves places no buildings yet, so a planned town corner can move into where a house stands in
  reality. When buildings come, they must be obstacles here or avoid planned roads.
- **Smoothing over planned roads.** If the four runs show the smoothing's fit tightening planned arcs, the smoothing
  should hold planned legs fixed. One input (the planned ranges) and a test; decided after the judging.
- **Vertical curvature.** The band keeps the grade only. If routes come out that the height solve can only follow with
  grade breaks above `GRADE_BREAK_HARD`, the band needs a curvature limit (the notes' profile smoothing, section 5.1,
  is the starting point).
- **Town corners between junctions.** If Task 1 shows they cannot be planned within the stray cap, v1 covers bends
  away from junctions only, and the town corners stay plate candidates (see the plan's Task 1).
- **A failed bend at a junction**, where both zones of the bend are kept, is the most likely plate site. The report
  pairs them; the plate task decides what to do with one leg planned and the other kept.
- **Default of the switch.** The stage lands switched off and is switched on after the judging, on the maintainer's
  word. Until then nothing in a build changes.
- **Cell-size dependence.** `PLAN_STEP_M`, the raster and the field follow the terrain cell where the notes do; the
  lead, the cap and the junction straight are in radii or metres. At other terrain sizes they may need to follow the
  cell.

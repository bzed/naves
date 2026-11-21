<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Road planning: re-routing curves the pieces cannot build

Status: design, 2026-11-21, amended the same day after the spike (plan Task 1,
`.superpowers/sdd/2026-11-21-naves-road-planning/spike/answers.md`). Written by the assistant on the controller's
brief, within the maintainer's decisions. Decisions the maintainer made are marked **(maintainer)**, rulings of the
controller **(controller)**; everything else is the assistant's call and easy to change. Numbers marked
**(re-check)** were measured by the spike without the grade rule and the corrected band of this amendment; they are
frozen only after the re-check on the map (plan Task 7b).

The method comes from the clean-room notes `docs/research/2026-11-21-road-tools-clean-room-notes.md`, sections 3.2 to
3.6 (obstacles, profile band, cost field, arc search, margins) and 5.1 (the two-pass profile, used here to measure
earthwork). Where this design departs from the notes it says so. No third-party code is used or sought.

Figures are Berchtesgaden at half scale (`scale.horizontal = 0.5`), terrain size 2048, 4 m cells, on `main` at
5137948; the spike ran on bc5ea8a, which has the same code.

## Why

The road pieces have fixed real sizes. The tightest curve piece of every family has a radius of 10 m on the map. Most
projects scale the world down (1:2 or similar) **(maintainer)**, so a real curve of 15 m becomes 7.5 m on the map,
and the pieces cannot build it.

On the example today:

- 16 135 m of road is tighter than the pieces (the smoothing's measure, on a 2 m resample of the drawn line). On the
  drawn lines that is 2261 tight zones at the lead of this design (1700 at a lead of four radii; the smoothing found
  873 on its fitted line, which had removed many OSM kinks).
- The placement meets these places and does what it can. What it cannot do shows up as `road_gap` (389, 1618 m in
  total, the largest 131 m), `road_not_drivable` (141), `road_conflict` (797) and `road_misses_terrain` (3752).

The smoothing stage (`2026-11-17-road-smoothing-design.md`) tried to fix this by pushing samples sideways inside a
corridor. It brought the tight length to 4088 m, but 254 zones still could not be built (mostly town corners between
junctions), it does not look at the terrain, it has no guard against running into another road, and it made the
placement 44 % slower. It is switched off **(maintainer)**.

A curve that cannot be built has to be built somewhere else: moved outward or sideways until it has the radius the
pieces need. Moving a road for that is "no problem at all", and the earthwork for it is fine **(maintainer)**. A
road that ends up steeper than it was meant to be is not fine **(controller, on the maintainer's question about the
switchback pictures)**. To do it well the stage must see the terrain, the water and the other roads. That is route
planning.

## Goal

Every curve that the pieces cannot build, and that can be built nearby, is re-routed as a chain of arcs and straights
that the pieces can build. Roads stay drivable and usable; that is the only goal for roads **(maintainer)**.

- Only the places where the drawn line cannot be built are planned; everything else keeps its OpenStreetMap line
  **(controller)**.
- A planned curve is not steeper than the road was meant to be, stays out of water, does not cross a stream it did
  not cross, keeps away from other roads' earthworks and from its own road elsewhere, stays off the map edge, and
  stays as near the drawn line as the radius allows.
- A bend that cannot be planned keeps its line and is reported with the reason. That list is where the concrete-plate
  task and the curved-bridge task start **(maintainer)**.
- The stage is judged at half scale **(maintainer)**.

## Non-goals

- Whole roads are not routed between junctions. That is a possible v2.
- No bridges inside the search (notes section 4). A bend blocked by water is reported as a curved-bridge candidate
  **(maintainer)**.
- No concrete plates. A bend that cannot be built as a curve is reported as a plate candidate **(maintainer)**.
- No car parks (their own task) **(maintainer)**.
- No railways.
- The height solve is not replaced. The planning's band and profile only decide whether a route is feasible and what
  it costs; the existing height solve still sets every height **(controller)**.
- No change to how pieces are chosen or fitted.
- No user option. The values are named constants.

## Anti-goals

- A junction, a dead end or a bridge end that moves.
- A planned road that runs into water, across a new stream, onto a bridge deck, into the earthwork of another road it
  did not touch before, or into its own road elsewhere.
- A planned road that is steeper than the drawn one and steeper than its family's aim: a switchback cut short is not a
  better switchback.
- A planned road that strays so far from the drawn one that a player who knows the place no longer recognises it.
- A route with a sudden very short change of steepness that a car cannot take **(maintainer)**.
- Output that depends on thread timing, map iteration order or wall-clock time.
- A zone's failure that changes anything outside that zone.

## Approaches

1. **Plan the tight zones only (chosen).** Find the zones where the line is tighter than the pieces, exactly as the
   smoothing stage does, and search a new line for each zone between two anchors on the same road where the drawn line
   is fine. The search is small, the rest of the road keeps its OpenStreetMap line, and a failure stays local. It
   cannot improve a road that is buildable but badly placed on the terrain, and it cannot move a junction to give a
   hairpin room.
2. **Route whole roads between junctions.** The search sees the whole road and could lay a switchback stack better
   than zone by zone. But every road moves, the search space is a hundred times larger, roads stop being recognisable,
   every user edit keyed by slot is orphaned, and the waypoints that keep a road on its course would have to be
   invented. Kept as a possible v2.
3. **Keep the widening of the smoothing stage and add a road guard.** The smallest change. But the widening is a
   convex fit of sideways offsets: it cannot see terrain, it cannot go round an obstacle (only revert), it shrinks
   curves before it widens them, and on the example it still left 254 bends unbuilt.

Zones only gives roads that stay recognisable, a search small enough to run on every zone of the map, and failures
that stay local, at the price of leaving bends that need a moved junction to the plate task. The spike confirmed it:
2046 of 2227 searched zones planned, the whole map in 5.3 s on 16 threads. The widening stays in the code, switched
off; after this stage is judged it is decided whether it is still needed **(maintainer)**.

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
- the stream lines: `features::water_lines` (ways tagged `waterway=stream`, `river` or `canal`, not culverts, cut
  at the map), the same lines `generate` already reads as `Read::water` and passes to `plan_roads` as `water` for the
  dips. `fitted_graph_with` gains them as a parameter;
- the parameters (the constants below, as a struct so tests can change them).

It returns a new `RoadGraph` and a `RerouteReport`. Stretch ids and node ids never change. Stretches outside zones are
equal bit for bit. With the switch off (`REROUTE_ENABLED = false`) the stage is not called and the graph is the
cleaned graph, bit for bit. The four combinations (planning off/on × smoothing off/on) can be chosen per call
without recompiling, so that `road-analysis` can compare them with the same code.

### Chains and tight zones

Zones are found as the smoothing stage finds them **(controller)**, but on the cleaned graph's lines, not on a fitted
line:

- A **chain** is the smoothing's chain: a maximal run of stretches joined at nodes where exactly two stretches meet,
  turning by at most `CHAIN_TURN_MAX_DEG` (150°), with the same tunnel flag. It ends at a junction (three or more
  stretches), a dead end, a tunnel end or a sharper two-stretch node. A ring is cut at its node with the lowest id.
  The smoothing's chain code is reused, not copied.
- Each chain is resampled at `SAMPLE_M` (2 m) per stretch; nodes are samples.
- A **tight run** is a run of samples whose three-point radius is below the minimum radius of the stretch they belong
  to.
- A **zone** is a tight run plus `PLAN_LEAD` minimum radii (2, about 21 m) of chain before and after it. Zones that
  touch are one zone. A zone that reaches the end of its chain within one lead of the tight run is anchored at that
  end node.

On the example: 2261 zones; 34 are not searched (27 with the tight run inside a held part, 4 roundabouts, 2 longer
than `PLAN_MAX_ZONE_M`, 1 tunnel). A lead of 4 merged zones into long ones (30 longer than 400 m) and left 3495 m
tight instead of 1518 m; a lead of 1 adds 77 routes that touch an earlier route.

Not planned at all (listed in the report as skipped, with the reason): a tunnel chain, a roundabout stretch, a
stretch whose line has a non-finite coordinate or collapses to one point, a family with no arc piece, and a zone longer
than `PLAN_MAX_ZONE_M`.

### What is fixed, anchors and legs

What never moves **(controller)**:

- a junction node, and the direction in which each of its arms leaves it;
- a dead end's position (its direction is free);
- a bridge end node, the bridge stretch itself and the first `PLAN_BRIDGE_HOLD_M` (25 m) of road on each side of it
  along the chain;
- a tunnel end node and its first 25 m, held like a bridge end;
- the node where a ring chain is cut, with the line's heading there.

An **anchor** is a pose: a point on the original line and the line's heading there, measured over one sample step on
each side of the point (at a chain end, over the first step). Each zone has an anchor at each end:

- where the zone ends inside the chain: the sample at the zone's end (by construction not in a tight run);
- where the zone reaches a junction: the junction node with its arm's direction. The leg's route begins with a
  straight of `PLAN_JUNCTION_STRAIGHT_M` (6.25 m) along the arm, so that the junction piece has a straight arm to sit
  on. It costs 45 (at 3 m) to 65 (at 0 m) zones of the map; it stays until the junction piece's real arm is measured
  (open points);
- where it reaches a dead end: the dead end's position with a free heading;
- where it reaches a held part (bridge, tunnel, ring cut): the end of the held part, with the line's heading there.

A held part inside a zone splits the zone into **legs**, each between two anchors. A leg is planned on its own. A bend
whose apex is a junction gives two zones on two chains, both anchored at that junction with their arm directions
kept **(controller)**; they are planned independently and reported as one bend (each zone names the junction and its
partner zone).

A leg whose anchors are closer than two sample steps, or (both anchors at junctions) closer than two junction
straights, is not searched (`held`, detail `anchors_close`; 49 zones on the example).

### The grade a leg may have

**A planned leg must not be steeper than the road was meant to be (controller).** For each leg:

- `Δz` is the terrain height at the end anchor minus that at the start anchor (bilinear, `Heightmap::sample`);
- the **drawn grade** is `|Δz|` over the drawn length between the anchors (along the original line, 2 m resample);
- the **grade limit** of the leg is `g_leg = min(grade_ceiling(family), max(grade_aim(family), drawn grade))`
  (`limits.rs`); in a zone of several families, the smallest aim and ceiling of them.

The drawn grade is the mean between the anchors, not the steepest window. The steepest 20 m window is measured and
reported (`drawn_grade_20m`) but not used: the drawn line has never been graded, so a short window measures the
terrain the line cuts across, not the road. On switchback-2 the mean is 7.4 % and the 20 m window 26.4 % (10 m:
35.4 %, against a ceiling of 27 %); a limit from the window would let the planned road go to the ceiling. The
height solve grades the drawn road to about its mean anyway. The aim is the floor, so a leg that is flat on the mean
but steep in places may still be planned at what the family is built for.

The band (below) keeps `g_leg` at every sample and pins the road to the terrain at both anchors within
`PLAN_ANCHOR_TOL_M`. A route therefore cannot be shorter than `(|Δz| − 2 · PLAN_ANCHOR_TOL_M) / g_leg`: a switchback
cannot be cut short. It has to keep its length, which means a wider loop.

**If no route exists within `g_leg`,** the leg is searched again with the limit raised by a third of the gap to the
ceiling, then two thirds, then the ceiling (`PLAN_GRADE_RELAX_STEPS` = 3). A route found at a raised limit is
planned, and its finding says how much steeper than drawn it is. A leg that has no route at the ceiling either keeps
its line and is reported as any other kept leg.

Every planned leg reports its drawn grade, its planned grade (`|Δz|` over the route's length), the limit it was
found at, and how many relaxation steps it needed. The report counts the legs planned steeper than drawn by more than
`PLAN_STEEPER_MARGIN` (2 points of grade) and names the worst.

Is the limit alone enough? A route is still drawn towards the limit, because the cost counts length: a gravel
switchback drawn at 15 % will come out near the aim of 18 %, not at 15 %. That is within what the family is built
for, so v1 has no extra cost term for grade above the drawn grade. Open points has the term and when to add it.

What the rule does to the six spike sites (from the spike's routes and the example's terrain, see the plan's Task 1
record): switchback-1 (gravel, `Δz` 21.4 m, drawn 145.6 m, 14.7 %; planned 90.8 m, 23.6 %) breaks it and must grow to
at least `(21.4 − 2 · 0.5) / 0.18 = 114` m (119 m with the anchors exactly on the terrain); switchback-2 (narrow asphalt, `Δz` 7.6 m, drawn 103.8 m, 7.4 %; planned 65.7 m, 11.6 %) is under its
aim of 12 % and stays; the two town corners (`Δz` 1.5 m), the junction hairpin (13.6 % against a dirt-track aim of
25 %) and the water bend (9.3 %) are not touched by the rule. On the whole map, 239 of the 2046 legs the spike planned
break it: half of them need less than 1.7 m more road, the worst 80 m; 15 of them are even above the ceiling on their
mean, which the spike's band allowed (see the band). The rule may push switchbacks against the stray cap; the re-check
(plan Task 7b) measures that before the constants are frozen.

### Obstacles and the distance raster

Notes section 3.2, built once per leg over its box: the bounding box of the leg's original line, grown by
`PLAN_STRAY_MAX · R` (the stray cap, with the planning radius `R` of the margin), the largest clearance and two raster
cells.

- **Resolution:** half the terrain cell, clamped to 1 to 2.5 m (2 m at 4 m cells), multiplied by 1.25 until the
  raster has at most `PLAN_RASTER_MAX_CELLS` cells.
- **Water:** a raster cell whose centre is in water by `water_test`.
- **Streams:** a stream line (above) that the leg's original line does not cross may not be crossed by the route
  (tested exactly, segment against segment, as a wall line). A stream the original line crosses `n` times must be
  crossed `n` times, counted away from the anchors (below). The spike: without the rule 10 of 1466 planned legs
  newly crossed a stream; with it, 0, at the cost of 3 to 6 zones.
- **Other roads:** every stretch of the graph that is not part of the leg's chain. Each is stamped with its earthwork:
  every cell within `core_reach_m(half its width, terrain cell)` of its centre line (`conform::core_reach_m`, the
  clean-up's measure). Cells a segment touches are found by an exact grid traversal (Amanatides–Woo).
- **Exemption, per place (controller, replacing the spike's per-road rule).** Where the leg's original line already
  comes near another road, the route may come as near: the samples (every 2 m) of another road that lie within its
  own reach plus the road clearance threshold of the original line, extended by `PLAN_EXEMPT_EXTEND_M` (10 m) along
  that road on each side, are not stamped; the rest of that road is. An exempt road's centre line is a wall line: the
  route crosses it as often as the original line does, counted away from the anchors. The spike exempted a whole road
  as soon as the original line came within about 31 m of it anywhere (its reach plus the test's threshold), and
  then a route could run in that road's earthwork far from where the original did; that rule is not used.
- **The own road outside the leg (self-clearance, controller).** The leg's own chain outside the leg is another road
  for this purpose, never exempt, except its first `PLAN_SELF_FREE` (the road clearance threshold plus its own reach,
  about 26 m for a town street) along the chain from each anchor, which is the road the leg continues into. Inside the
  route: two samples more than `PLAN_SELF_ALONG = π · R + 2 · road clearance` apart along the route must be at least
  the road clearance apart (the bend itself and the spread of its legs are one road; a loop that comes back onto
  itself is not). The spike had no self-clearance at all: the own chain outside the leg touches the leg at its
  anchors, so its per-road exemption always exempted it.
- **Bridge decks and tunnels:** stamped like other roads and never exempt, except a deck or tunnel within
  `PLAN_DECK_ANCHOR_M` (12 m) of a fixed anchor (junction, dead end, held part), which is part of that anchor and is
  exempt per place. Reason: the clean-up puts a junction 8 m from a bridge end, so the deck stamped its own anchor;
  about 75 of the 170 legs whose original line failed the test in the spike's first run were this. A leg whose
  original line passes over or under a deck or a tunnel elsewhere is not planned (`held`, detail `crosses_deck`).
- **Map edge:** cells outside the map, and cells within `core_reach_m(own half width)` of the edge, are obstacle
  cells. The edge has no clearance on top of that (it is not in the road distance map). If the leg's original line
  already lies within that margin (a road that runs to the edge), only the cells outside the map are obstacles. A
  dead end on the map edge (a stretch cut there) is a fixed anchor whose first segment is not tested against the edge.
  Reason: margin plus clearance blocked roads running to the edge and a leg 28 m from it.
- **Distance maps:** a two-pass chamfer transform (orthogonal weight = resolution, diagonal = resolution · √2), stored
  in decimetres in 16 bits, saturating: one for water, one for roads (decks and tunnels in it), one of the distance to
  the leg's original line (for the stray term and the cap).
- **One threshold for every test.** A segment is blocked if a cell it touches is an obstacle cell, or its chamfer
  distance is below `thr(c) = (c + res) · CHAMFER_ERROR + res · √2` for the clearance `c` (notes 3.2). The same
  `thr` is used for the exemption above; the water cap below is in the same stored measure. Reason: in the spec as first written the test
  used `thr` but the exemption and the water cap used the bare clearance, so 170 of 1645 original lines failed their
  own test (the anchor lay in blocked space). The spike's alternative, the bare clearance plus half a cell, left 68 of
  1645 failing and planned 22 more zones (1485 against 1463, at lead 4); it lets the true distance fall about 8 %
  short of the clearance (1.2 m at 15 m), which the one-cell gap in the road clearance absorbs. It is measured again
  in the re-check (open points).
- **Clearances:**
  - roads: `core_reach_m(own half width) + one terrain cell` (12.5 to 15 m); with the stamped earthwork of the other
    road this is the clean-up's "earthworks touch" test (two reaches and a cell);
  - water: the threshold `min(thr(core_reach_m(own half width)), d_orig)`, where `d_orig` is the smallest stored
    chamfer distance to water of the cells the original line touches. The cap is taken in the map's own measure (the
    stored value), not by inverting `thr`: the inverse `(d_orig − res · √2) / CHAMFER_ERROR − res` is not positive for
    an original line nearer than about 5 m to the water, and the cap then collapsed to "no clearance", letting a route
    go nearer to the water than the original was. With the cap in the stored measure the original line passes its
    own test exactly, and a riverside road may stay as near the water as it already was, however near that is. The raster keeps the cap itself (computed when the original line is set and the maps
    are finished), so a caller can neither forget it nor pass a wrong one; an original that touches water has
    stored distance 0, so the leg gets cap 0 and only water cells block there.
- **Crossings near the anchors.** Wall crossings (exempt roads, streams) are counted, and block a segment of the
  route when the wall allows none, only farther than `PLAN_CROSSING_END_M` (10 m) from either anchor. Reason: an exempt road that shares geometry with the original line
  near its node made the counts unstable; 7 good routes failed their check, 1 with the rule.

**What remains.** With these rules, 84 of the 2227 legs on the example still have an original line that fails the
test. The spike's diagnostics name the first blocking kind: 72 another road's earthwork (54 within the first or last
three segments, at the anchor), 8 water, 4 the map edge (dead ends on the edge, which the edge rule above now
exempts). In the cases inspected, the road is a bridge deck or tunnel 9 to 22 m from the line that is not within 12 m
of a fixed anchor (never exempt), or a road the anchor sits beside. Their outcomes in the spike: 34 kept `held` (no
route from the fixed pose), 24 kept because the junction straight was blocked, 11 `blocked_by_road`, 7 anchors too
close, 2 `blocked_by_water`, 2 deck crossings, 4 planned.

**A leg whose original line fails the test (controller):** it is still searched. A route is accepted if it passes the
test, although the original would not; if no route is found, the leg is kept and reported like any other, with
`original_feasible: false`.

The search, the band and the field ask one question of the raster: is this segment clear, and if not, of what (water,
road, edge, wall). The answer's kind is what the failure reasons are built from.

### The feasibility band and the profile

Notes section 3.3 for feasibility, with three changes found by checking the spike's "earthwork 0".

What the spike (and this spec as first written) computed: the reachable interval starts as the whole band at the start
anchor, `[z₀ − cut, z₀ + fill]`; it propagates by `± g · ds`, intersected with the band at each sample; the nominal
height is the terrain clamped into the interval; climb is `Σ|h − h_prev|` and earthwork `Σ|h − z| · ds`. Two things
follow:

- The nominal height is not a profile that keeps the grade; it is the terrain wherever the terrain lies inside the
  set of heights that some grade-keeping profile could reach. Its earthwork is a lower bound: it is 0 as long as the
  terrain stays inside a set that starts 12 m wide and widens. Its climb is the terrain's rise and fall along the
  route (switchback-1: 21.9 m, against a net 21.4 m).
- The anchors' road height floats within ±6 m, so up to 12 m of climb can be put into embankments at the anchors,
  where the untouched road meets the route.

At switchback-1 the planned route (90.8 m, mean 23.6 %, ceiling 36 %) is climbable on the mean, but the terrain along
it is steeper than the ceiling on 18.2 m of its 2 m samples, steepest 20 m window 54.9 %. A real profile there (the
notes' two-pass profile, section 5.1, closest to the terrain, anchors pinned within 1 m) needs 46 m² of earthwork, a
cut of 1.6 m and a fill of 2.3 m; with the anchors free in the band 64 m² and 3.9 m of fill. The drawn line needs
4 m². The spike reported 0. At switchback-2: 23 m², cut 1.8 m (reported 0). The spike's band is the spec's band, so
the spec changes, not just the code:

1. **Pinned anchors.** The start interval is `[z₀ − PLAN_ANCHOR_TOL_M, z₀ + PLAN_ANCHOR_TOL_M]`; a route arrives only
   if its final interval meets `[z_end − tol, z_end + tol]`. In the code a start is made only by
   `Reach::at_anchor(z, &Band)`, whose tolerance is the band's own, and the other reaches come out of the propagation.
   The spike's floating start `[z − 6, z + 6]` cannot be built except from a band with that tolerance; acceptance does
   not depend on it.
2. **The grade is the leg's limit** `g_leg`, not the ceiling (above).
3. **Real earthwork at arrival.** When an arrival candidate has passed the forward band, the two-pass profile of notes
   5.1 (forward and backward passes over the whole candidate route from anchor to anchor, then the profile closest to
   the terrain) is computed once, and its earthwork `Σ|h − z| · ds` replaces the lower bound in the route's cost. Its
   deepest cut and highest fill are reported. Because the lower bound never exceeds the real earthwork, the
   correction only raises the cost of an arrived state, and the search stays exact at weight 1. `arrive` is the only
   way to accept a route (it pins both ends at the terrain of the route's own ends, and a `Profile` can only come from
   it; its fields are read through accessors); `band_step` and `arrives` only prune.

Otherwise as the notes: every segment passes the segment test; the step is sampled at half the terrain cell, clamped to
0.1 to 5 m (2 m); the band at a sample is `[z − PLAN_CUT_M, z + PLAN_FILL_M]`; empty (lo > hi by more than 10⁻⁹)
fails the step; a sample with no terrain (outside the map, non-finite) fails the step. A band with a NaN or negative
grade, or a cut, fill or anchor tolerance that is NaN, negative or infinite, or a start that is not finite or is empty,
is refused (`BandFail::Invalid`); only the grade may be infinite (the cost field removes the grade limit). `leg_limit`
gives `(limit, ceiling)` and `None` without families; `relaxed_limits` gives `None` for a NaN.

The band and the profile keep the grade only. The height solve later sets the heights and keeps the vertical radius
and the grade-break rules. If the judging shows planned routes that the height solve can only make with sharp grade
breaks, that is an open point.

### The cost-to-go field

Notes section 3.4, one field per leg, over the leg's box:

- Grid resolution `max(PLAN_STEP_M, terrain cell)` (6 m), multiplied by 1.25 until there are at most
  `PLAN_FIELD_MAX_CELLS` cells.
- The grid is laid out so that the end anchor is the centre of a cell (it covers the leg's box and the anchor), so the
  start cost of the Dijkstra from that cell is 0, not a distance inside the cell. 8-connected; the heap is ordered by
  cost (`total_cmp`), then cell index. A target that is not a number, or a box too large to fit, gives an empty field; a box that is not a number gives the
  target's cell, and a band or stray cap that refuses every edge the target's cell alone (estimate 0 in each case).
- An edge between neighbouring cell centres is allowed if the segment test passes (obstacles, clearances), there is
  terrain at the band's samples along it (placed by the one helper that `band_step` also uses), no wall that allows no crossing blocks it, and both cells are within the
  stray cap (tested at the cell centres). It is run in the direction of travel. This is the band with the grade limit
  removed: at an infinite grade every sample is reachable from the one before it, so cut and fill never bind, and the
  code does not call `band_step` but does exactly this (a test pins that the answers agree).
- Edge cost: since cut + fill > 0, it is the distance (the notes' rule for that case).
- Estimate at a point: the cell's value divided by `CHAMFER_ERROR`, minus one field cell, not below 0. A cell without
  a value (water, a wall, missing terrain or the stray cap cut it off), a point outside the grid, an empty field and a
  target inside an obstacle all give 0: 0 does not mean reached, and the search must not prune on it. The estimate is
  a lower bound on heights, and in the plane only up to the grid: a corridor no 6 m segment fits, and the stray cap
  tested at centres, can make it too high in narrow places.

### The search for one leg

A hybrid A* over poses (notes section 3.5), from the start anchor's pose to the end anchor's pose.

**State:** position, heading, cost so far, the reachable height interval, the parent, whether it has arrived.

**Steps:** from each state, five arcs of length `s = PLAN_STEP_M` (6 m), with heading changes `0, ±θ/2, ±θ`,
`θ = min(s / R, 45°)`. `R` is the planning radius: the family's tightest arc times `PLAN_RADIUS_MARGIN` (1.05, notes
3.6). An arc is sampled about every 2° (at least one segment, at most 2 m apart). In a zone of several families, `R`
is the largest of their radii. The spike: step 6 plans the same legs as step 4 (1468 against 1466 of 1645) at a third
of the CPU time; step 2 loses legs inside the budgets; seven arcs (±θ/4 added) find nothing more.

**Arrival at a pose (a departure from the notes).** The end anchor has a heading the untouched line continues with.
From every state within `PLAN_ARRIVE_REACH` (`max(3 · s, 4 · R)`) of the end anchor the search tries the six Dubins
paths of radius `R` from the state's pose to the end pose, in order of length. The first that passes the band
(arriving inside the end anchor's height tolerance), the obstacle and wall tests, the self-clearance and the stray cap
is an *arrived* successor, with its full step cost (real earthwork, above) and its position and heading set exactly
to the anchor's. The spike verified the construction on 90 816 paths: worst end-pose error 2.5·10⁻¹³; 89 % of the legs
that end at a junction were planned. At a dead-end anchor (free heading) the notes' single arc through the point is
tried instead (angle α to the point under 90°, radius `c / (2 · sin|α|) ≥ R`). The start of a leg at a junction pose
first takes the junction straight as a fixed first step.

**Start:** one state at the start anchor with its heading and the pinned interval. At a dead-end start (free heading),
one state per heading bin.

**Cost of a step** (in metres of road):

```
length + W_SLOPE · climb + W_CURVE · |δ| · R + W_EARTH · earthwork + W_STRAY · Σ d · ds
```

`δ` is the step's heading change; `d` is the distance of a step sample from the leg's original line (from the
distance map). The curvature term prefers straight runs (notes). The stray term keeps the planned road near the
OpenStreetMap line **(controller)**. The earthwork of a step is the band's lower bound; at arrival it is replaced by
the real earthwork of the whole route.

**How `W_STRAY` was chosen (the rule that was applied).** On the six spike sites, over every combination of the other
constants (81 per site): the largest candidate at which every site is found in every combination and no route is more
than 10 % longer than at the smallest candidate (0.05). 0.1 and 0.25 pass 486 of 486 (worst +1.8 % and +6.0 %), 0.5
passes 405. So 0.25. On a broader set of 112 harder legs the same rule picks 0.05, because there 0.05 cuts across an
apex that the larger weights follow; the controller ruled for 0.25, because it halves the stray of switchbacks (p95
27.6 m at 0.05, 13.9 m at 0.25) and the maintainer wants planned roads to stay recognisable: moving outward is fine,
not something to maximise **(controller)**. 0.05 is the alternative in open points.

**Stray cap:** a step with a sample further than `PLAN_STRAY_MAX · R` (3 radii, 31.5 m) from the leg's original line
is not taken. The spike: cap 2 loses 3 of 34 town corners, caps 4 to 6 find none more; on the map the largest move was
28.5 m, p95 16.6 m. The grade rule makes switchbacks longer, so the cap may now bite there; the re-check measures it
(plan Task 5 for the two switchbacks, Task 7b for the map).

**Heuristic** (0 for an arrived state): the larger of the field estimate and the notes' analytic bound,
`max(distance, Δ / g_leg) + W_SLOPE · Δ` with `Δ = max(0, |z_end − z_here| − PLAN_CUT_M − PLAN_FILL_M)`.

**Duplicates:** the state key is (arrived, position in cells of `s / 2`, heading bin); bins `⌈2π / (θ / 4)⌉`, clamped to
32 to 180. A new state is dropped if a stored state with the same key costs no more; otherwise it replaces it. A popped
state whose priority is above its cost plus the weighted heuristic is stale and skipped.

**End:** the first arrived state that is popped ends the search; its path is the route. There is one leg per search,
so the notes' arrivals per heading sector and their stop rules are not needed.

**Budgets:** each try is the sequence `PLAN_BUDGETS` of (heuristic weight, expanded states): (1.0, 10 000),
(1.5, 20 000), (3.0, 40 000). The next pair runs only when the budget ran out; an empty open set proves there is no
route and ends the try. The spike: four times the largest count to the first arrival on the six sites (1745); on the
map, the same 2046 legs are planned with these budgets, with twice them and with one try of 400 000; 9 legs needed
weight 1.5; the largest count to a first arrival anywhere was 27 315, median 155. Budgets are counts of states, never
wall time.

**Tie-breaking:** the open set is ordered by priority, then cost, then the state key, then the order of insertion, all
floats compared with `total_cmp`. States live in an arena indexed by insertion order. The key table is a sorted map,
a dense array over the leg's box, or a hash map that is only looked up, never iterated.

### Writing the route back into the graph

A planned leg's route is written as points: every arc sampled at `SAMPLE_M` (2 m) or finer, a straight as its two
ends. The route is not simplified: a coarser polyline of an arc measures tighter than the arc.

- The chain's line from the start anchor to the end anchor is replaced by the route. An anchor that lies between two
  original vertices is inserted as a point; outside the legs, every original point stays.
- A node where the road merely continues and that lies inside a leg moves to the route point at the same fraction of
  the leg's length as it had on the original line. Its two stretches are cut there; the node keeps its id.
- The crossing list is rebuilt from the stretches (`crossings_of`).
- Every attribute stays on its stretch. No stretch is added or removed.

Before it is accepted, every route is checked: every three-point radius on its 2 m samples at least
`PLAN_REACHED_FRACTION` of the tightest arc; the end pose met within 10⁻⁶ m and 10⁻⁶ rad; every sample within the stray
cap; every segment clear; wall crossings equal away from the anchors; self-clearance; the two-pass profile feasible at
the limit it was found at. A route that fails is a bug: the leg keeps its line with reason `no_route`, detail
`check_failed` (1 leg in the spike).

**Routes against each other (controller).** Legs are searched independently against the original graph, in parallel.
Then, in zone order (by the chain's first stretch id, then along the chain, then leg order), each planned route is
checked against the routes accepted before it: a route that comes within the road clearance threshold of an earlier
accepted route, where the two original lines did not, is reverted, kept with reason `blocked_by_road`, detail
`route_of <zone id>`. Its original line then stays, and later routes are checked against it as they were in the
search. This is sequential and deterministic. The spike counted 26 such routes (1.3 % of 2046) and did not revert
them.

### Failure and reporting

A leg that is not planned keeps its original line. Nothing else changes. Nothing panics.

Reasons **(controller)**:

- `held`: a junction, a bridge or tunnel end, a dead end or the ring cut leaves no room. Used when the leg could not be
  searched (anchors too close, a deck crossing), when the junction straight itself is blocked, and when a search that
  starts or ends at a fixed pose fails (all relaxation steps) and the tight run lies within `PLAN_LEAD` of that pose.
- `blocked_by_water`: no route was found, and a diagnostic search without water (areas and streams) finds one.
- `blocked_by_road`: no route was found, and a diagnostic search without other roads finds one; or the route was
  reverted against an earlier accepted route.
- `no_route`: none of the above, with `budget_exhausted` or proven.

The diagnostic searches run only for failed legs, at the ceiling grade, with the last budget pair only.

On the example the spike (without the grade rule) kept 167 `held` (49 anchors too close, 25 junction straight blocked,
91 no route from a fixed pose, 2 deck crossings), 11 `blocked_by_road` (all open country, original line inside another
road's earthwork), 2 `blocked_by_water`, 1 `check_failed`.

Each zone in the report has: its id (`<first stretch id of the chain>#<n>`), where it is (its tightest sample), its
stretches, its legs, the anchor kinds at both ends, the junction and partner zone where the bend passes a junction, the
outcome (`planned`, or `kept` with the reason and a detail), the radius before, wanted and after, the length before
and after, the largest distance from the original line, the drawn grade (mean and 20 m window), the planned grade, the
grade limit used and the relaxation steps, the climb, the real earthwork with the deepest cut and highest fill, the
states expanded (with retries, relaxations and diagnostics), the heuristic weight that found the route,
`original_feasible`, and the candidate the bend is for: `plate` for `held` and `no_route`, `curved_bridge` for
`blocked_by_water`, none for `blocked_by_road` or a planned zone.

Totals: zones by outcome and reason, tight length before and after, the largest and p95 distance from the original
line, the legs planned steeper than drawn by more than `PLAN_STEEPER_MARGIN` and the worst of them, the routes reverted
against earlier routes, states expanded, the chains and stretches skipped with reasons. The stage's time is measured by
the caller beside the report (like `smooth_time`), never in it.

Outputs:

- `build/report.json`: `roads.planning`, with the totals and every zone; `{"enabled": false}` when the stage is off.
- Build summary: one line, for example `Road planning: 2227 zones, 2046 planned, 181 kept (no_route 1, held 167,
  blocked_by_water 2, blocked_by_road 11), tight length 16135 m -> 1547 m, largest move 28.5 m, <n> steeper than
  drawn` (the figures of the spike, which had no grade rule).
- A progress line with the stage's time.
- Findings: `road_curve_planned` (information) for a planned zone, at its tightest point, with the radius before and
  after, the largest move and, when a relaxation was needed, how much steeper than drawn it is; `road_curve_not_planned`
  (warning) for a kept zone, with the reason and the radius it has. New rules rather than `road_curve_too_tight`, so
  that planning and smoothing do not report the same bend twice when both are on.
- `road-analysis` prints `PLAN …` lines (totals, every kept zone, every leg planned steeper than drawn) and
  `TIMING reroute`.
- The manual (getting started, troubleshooting, the MCP rule list) and `CHANGELOG.md`.

### Determinism

- Same input, same graph, bit for bit, on repeated runs and with any thread count **(controller)**. The spike's
  per-leg output was identical at 1, 10 and 16 threads.
- Zones are found in chain order. Legs are searched in parallel with `rayon` and collected in zone order; each search
  is single-threaded. The cross-check of routes is sequential in zone order.
- All budgets are counts of states. No wall time enters a decision.
- Floats are compared with `total_cmp`; sums are taken in a fixed order; no hash-map iteration.

## Constants

All in `reroute.rs`, all `pub const`, collected in `RerouteParams::default()`. The last column says which ones the
re-check with the grade rule (plan Task 7b) must confirm before they are frozen.

| name | value | meaning | supported by | re-check |
|---|---|---|---|---|
| `REROUTE_ENABLED` | `false` until judged | the switch | controller | |
| `SAMPLE_M` | 2.0 m | resample for zones; arcs written back | `smooth::SAMPLE_M`, reused | |
| `CHAIN_TURN_MAX_DEG` | 150° | a two-stretch node that turns more ends a chain | `smooth`, reused | |
| `PLAN_LEAD` | 2.0 minimum radii | road beside a tight run that belongs to its zone | tight after 1518 m (lead 4: 3495 m), zones planned 2050 of 2227, median states 144 (lead 4: 430); lead 1 adds 77 route conflicts | yes: longer loops need room |
| `PLAN_RADIUS_MARGIN` | 1.05 | planning radius over the tightest arc | notes 3.6; routes 10.41 to 10.50 m | |
| `PLAN_REACHED_FRACTION` | 0.99 | a written route's tightest sample, of the tightest arc | smoothing's reached rule | |
| `PLAN_STEP_M` | 6.0 m | search step | same legs as 4 m (1468 / 1466), a third of the CPU; 2 m loses legs | |
| `PLAN_MAX_TURN_DEG` | 45° | largest heading change of a step | notes 3.5 | |
| heading bins | 32 to 180 | `⌈2π / (θ/4)⌉`, clamped | notes 3.5 | |
| `PLAN_ARRIVE_REACH` | `max(3 · s, 4 · R)` | Dubins arrivals tried from here | (2s, 3R) and (4s, 6R) give the same outcomes, ±12 to 20 % time | |
| `PLAN_JUNCTION_STRAIGHT_M` | 6.25 m | straight at a junction anchor | half a junction piece; 0 m plans 65 more zones, 3 m 45 more | revisit with the piece |
| `PLAN_BRIDGE_HOLD_M` | 25.0 m | road held beside a bridge or tunnel end | `smooth::WIDEN_BRIDGE_HOLD_M`; controller | |
| `PLAN_DECK_ANCHOR_M` | 12.0 m | a deck this near a fixed anchor belongs to it | spike: clean-up's 8 m approach | |
| `PLAN_STRAY_MAX` | 3.0 minimum radii | furthest a route strays | cap 2 loses 3 of 34 town corners, 4 to 6 find none more; largest move 28.5 m | yes: switchbacks under the grade rule |
| `PLAN_CUT_M`, `PLAN_FILL_M` | 6.0 m each | band around the terrain | found equal at 3, 6, 12; measured with the lower-bound earthwork | yes: real earthwork |
| `PLAN_ANCHOR_TOL_M` | 0.5 m | road height at an anchor within this of the terrain | assistant, from the switchback check | yes |
| grade limit `g_leg` | `min(ceiling, max(aim, drawn mean))` | grade of the band for a leg | controller; `limits.rs` | |
| `PLAN_GRADE_RELAX_STEPS` | 3 | thirds of the gap to the ceiling, tried in turn | assistant | yes: how many legs need them |
| `PLAN_STEEPER_MARGIN` | 0.02 | a planned leg this much steeper than drawn is counted | assistant | |
| `PLAN_DRAWN_WINDOW_M` | 20.0 m | window of the reported (not used) steepest drawn grade | controller's example | |
| `W_SLOPE` | 4.0 | metres of road per metre of climb | notes 3.5 | |
| `W_CURVE` | 0.1 | times `|δ| · R` | notes 3.5 | |
| `W_EARTH` | 1.0 | metres of road per m² of earthwork | −38 % (lower-bound) earthwork against 0.5 at equal length, found, states | yes: real earthwork |
| `W_STRAY` | 0.25 | metres of road per m² between route and original line | the selection rule above; switchback stray p95 13.9 m (0.05: 27.6 m) | yes |
| `PLAN_BUDGETS` | (1.0, 10 000), (1.5, 20 000), (3.0, 40 000) | weight and expanded states per try | 4 × 1745; whole map plans the same legs as with 400 000 | yes |
| `PLAN_MAX_ZONE_M` | 400 m | a longer zone is not planned | at lead 2 only 2 zones are longer; 1000 m plans 2 more | |
| `PLAN_EXEMPT_EXTEND_M` | 10.0 m | exempt part of a road beyond where the original comes near | assistant (per-place exemption) | yes |
| `PLAN_SELF_FREE` | road clearance threshold + own reach | own road beyond an anchor that is not an obstacle | assistant (self-clearance) | yes |
| `PLAN_SELF_ALONG` | `π · R + 2 · road clearance` | samples further apart along the route must keep the clearance | assistant (self-clearance) | yes |
| `PLAN_CROSSING_END_M` | 10.0 m | wall crossings this near an anchor are not counted | spike: 7 check failures to 1 | |
| raster resolution | half the terrain cell, 1 to 2.5 m | obstacle raster | notes 3.2 | |
| `PLAN_RASTER_MAX_CELLS` | 4 000 000 | per leg; grows by 1.25 | assistant | |
| field resolution | `max(s, terrain cell)` (6 m) | cost-to-go grid | notes 3.4 | |
| `PLAN_FIELD_MAX_CELLS` | 1 000 000 | per leg; grows by 1.25 | notes 3.4 | |
| `CHAMFER_ERROR` | 1.0824 | chamfer and octile excess | notes 3.2, 3.4 | |
| test threshold `thr(c)` | `(c + res) · CHAMFER_ERROR + res · √2` | one threshold for test, exemption, water cap | notes 3.2; spike 8.1 | yes: against `c + res/2` |
| road clearance | `core_reach_m(own half) + cell` | earthworks touch, with the other road stamped at its reach | `netclean`, `conform::core_reach_m` | |
| water threshold | `min(thr(core_reach_m(own half)), d_orig)`, `d_orig` the stored chamfer distance of the original line | the cap is in the map's measure, not an inverted `thr`; kept by the raster, not passed by the caller. An original that touches water (stored distance 0) gives cap 0 for that leg: only water cells block there | spike 8.1; review of Task 2 | |

## How it is judged

Berchtesgaden at half scale, terrain size 2048. Four runs of `road-analysis` with the same binary: planning off/on ×
smoothing off/on. Compared on:

- road length tighter than the pieces, before and after (16 135 m; smoothing alone 4088 m; the spike without the grade
  rule 1547 m);
- zones by outcome and reason; the full list of bends that could not be built, with their candidate;
- the largest and p95 distance of a planned route from the original line (spike: 28.5 m, 16.6 m);
- **planned legs steeper than drawn**: the count above `PLAN_STEEPER_MARGIN`, the count that needed a relaxation, and
  the worst (drawn grade, planned grade, family aim and ceiling);
- **drivability on planned legs**: `road_not_drivable`, `road_grade`, `road_gap` and `road_misses_terrain` inside
  planned zones, against the same places with planning off;
- the routes reverted against earlier routes (spike: 26 would be);
- `road_gap` (count, total length, largest; baseline 389, 1618 m, 131 m), `road_not_drivable` (141), `road_conflict`
  (797), `road_misses_terrain` (3752), and every other rule count;
- that no junction, dead end, bridge end or tunnel end moved (must be 0);
- placement time (baseline about 550 s) and the planning's own time, separately (spike: 5.3 s on 16 threads, 53.5 s
  on one, without the grade rule and the real earthwork).

Proposed bar for switching the stage on (the maintainer decides), with the spike's figures beside it:

| criterion | bar | spike (no grade rule) |
|---|---|---|
| tight length with planning alone | at most half the baseline, 8068 m | 1547 m of 16 135 m |
| `road_gap`, `road_not_drivable`, `road_conflict`, `road_misses_terrain` | none worse than off by more than 5 % without a reason that can be named | not measured (no placement) |
| planned legs steeper than drawn by more than 2 points | at most 1 % of planned legs, none above the ceiling | 239 of 2046 above the new limit, 15 above the ceiling: the reason for the grade rule |
| planning time | under 60 s on the machine's threads | 5.3 s on 16 threads |
| placement time | not more than 10 % slower | not measured |

Then a game check with `tools/ingame-check` on about ten sites, planning off against on: the two switchbacks, two town
corners, the junction hairpin, the bend beside water, two kept zones (one plate, one curved-bridge candidate), one zone
with a moved two-stretch node, one straight road as a control. The switchbacks are driven up, not only photographed.

## Effects on other stages

- **The clean-up before it.** Planning sees the cleaned graph. The clean-up's new nodes are junctions like any other
  and become anchors. A bridge approach made by the clean-up is within the 25 m hold and within `PLAN_DECK_ANCHOR_M`.
- **Smoothing after it.** When both are on, the smoothing sees the planned lines. Its fit pulls curves inward and can
  make a planned arc tighter again. The four-run comparison shows how much (open points).
- **Bridges.** No bridge stretch and no road within 25 m of a bridge end moves; `fit_bridges` sees the bridges as
  before.
- **Placement.** It fits pieces to the planned line as to any line. Planned arcs have 1.05 times the tightest arc's
  radius. The line has more points inside planned zones (2 m on arcs).
- **Height solve.** Unchanged. It receives the placed pieces and sets every height. A planned route had a feasible
  two-pass profile at its grade limit, with its anchors on the terrain, so the solve should find it; its vertical
  curvature is watched in the judging (`road_grade`, `road_not_drivable`).
- **Dips and bridges over dips** run on the placed planned roads as before. A planned route crosses no stream its
  drawn line did not.
- **User edits** are keyed by stretch id, preset and slot. Ids do not change, but slots on a stretch whose line moved
  shift, so edits there can be orphaned (`orphaned_edit`). Switching the stage on is a **breaking** change for such
  edits and the changelog says so in the release that switches it on.

## Open points

- **Buildings.** Náves places no buildings yet, so a planned town corner can move into where a house stands in
  reality. When buildings come, they must be obstacles here or avoid planned roads.
- **Smoothing over planned roads.** If the four runs show the smoothing's fit tightening planned arcs, the smoothing
  should hold planned legs fixed. One input (the planned ranges) and a test; decided after the judging.
- **Vertical curvature.** The band and the profile keep the grade only. If routes come out that the height solve can
  only follow with grade breaks above `GRADE_BREAK_HARD`, the profile needs a curvature limit (the notes' profile
  smoothing, section 5.1 step 6, is the starting point).
- **The junction straight** costs 45 (3 m) to 65 (0 m) zones. Revisit when the junction piece's straight arm is
  measured.
- **`W_STRAY` 0.05.** The selection rule applied to the 112 harder legs picks 0.05: more routes stay within +10 % of
  their shortest length there (at 0.25, 24 legs are over +10 %, the worst +180 %), at the price of twice the stray on
  switchbacks (p95 27.6 m against 13.9 m) and up to twice the states. 0.25 is the controller's ruling; 0.05 is the
  alternative if the game check finds roads that wander where a straighter line was possible.
- **Grade above the drawn grade.** v1 lets a planned leg go up to the family's aim even where the drawn leg was
  flatter. If the game check shows switchbacks that feel steeper than they should, add a cost term charged at arrival:
  `W_GRADE · max(0, |Δz| − drawn grade · L)` (with `W_GRADE · drawn grade > 1`, so that lengthening the route to the
  drawn grade pays for itself).
- **The test threshold.** The spike's alternative (bare clearance plus half a cell) plans 22 more zones and leaves 68
  original lines failing instead of 87 (lead 4); the re-check measures it at lead 2 with the per-place exemption.
- **Town corners between junctions.** Decided: they are planned (44 of 56 City corners of 60 to 120° between
  junctions at the first settings, 21 of 32 at the recommended ones; no flag is needed). The smaller share at the
  recommended settings is a selection effect of the lead, not a loss: at lead 2 only corners whose junctions are
  within about 21 m of the bend count as "between junctions"; those with the junctions 21 to 42 m away now have line
  anchors and are planned in another class. That is not a reason for a longer lead at junctions: a zone that reaches a
  junction within one lead is anchored at it already, and a longer lead would only replace free line anchors by the
  fixed junction pose and straight, which is more constrained. The re-check confirms it on the 56 corners by position.
- **Default of the switch.** The stage lands switched off and is switched on after the judging, on the maintainer's
  word.
- **Cell-size dependence.** The step, the raster and the field follow the terrain cell where the notes do; the lead,
  the cap, the anchor tolerance and the junction straight are in radii or metres. At other terrain sizes they may need
  to follow the cell.

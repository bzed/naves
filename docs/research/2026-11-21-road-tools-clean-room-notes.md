<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Two third-party road tools: a clean-room functional description

## How this document was made

This is the reader's half of a clean-room process. The reader studied the source of two third-party tools.
This text describes what they compute, in the reader's own words. The people who implement anything from it
never see the source. No code, no comments and no internal names were copied. Where a name appears, it is a fact
of the outside world: a game model name, a memory point name, a file-format tag or offset, or a Terrain Builder
term. Numbers that are settings or tolerances of the tools are given. Numbers that measure Bohemia Interactive's
models are not given. The text says how to get them from a model at run time instead.

Neither source has a licence file, and neither package manifest declares a licence. Their code is therefore all
rights reserved. Náves may use the ideas, the methods and the file-format facts. It may not use the code.

| Tool | Language | Commit | Date |
|---|---|---|---|
| **DayZ Road Studio** (also "Road Builder", version 2.5.3) | Rust | `644e6735bfef5cf55482a9ebca2d0f74cb490bca` | 2026-11-08 |
| **DayZ Road Builder** | C# | `c5d0af3f1849f33775e201e84baf11ff6f3306d6` | 2026-10-04 |

Below, "Studio" means DayZ Road Studio and "Builder" means DayZ Road Builder. Builder ships Bohemia's road
models in a `RoadParts/` folder. Those files were not opened.

Conventions in this text: a *heading* is a compass bearing in degrees, clockwise from north (+Z), unless the text
says otherwise. Builder works in that convention. Studio does its maths in counter-clockwise radians from east and
converts to clockwise degrees for Terrain Builder. Both tools agree on what they write out.

---

## 1. Chaining road pieces along a line

Both tools take a centre line and chain real road pieces along it. Every piece starts exactly where the one
before it ends, so there are no gaps. They search differently.

### 1.1 Geometry of a piece from its memory points

Both read unbinarised MLOD `.p3d` models. Both find the memory LOD by its resolution, 10¹⁵. Builder accepts a
tolerance of about 10⁹ around that value. Studio accepts any resolution between 9·10¹⁴ and 1.1·10¹⁵. Builder falls
back to any LOD that has the four road points if no memory LOD has them.

A named selection is a set of vertices. In MLOD, a selection's tag block holds one weight byte per vertex, then
one per face. A vertex is in the selection if its byte is not zero. Both tools take the **mean of the selected
vertices' X and Z** as the point. Height (Y) is ignored. Studio upper-cases selection names and skips tags that are
inactive or start with `#`. Builder compares names as they are.

The points used:

- `LB`, `PB`: left and right corner of the start edge.
- `LE`, `PE`: left and right corner of the end edge.
- `LD`, `LH`: the left side arm of a junction piece. `PD`, `PH`: the right side arm.

From these:

- **Port centre**: the midpoint of each pair.
- **Width**: the distance `LB`–`PB`.
- **Direction at an edge**. Builder rotates the left-to-right edge vector by 90° to get the forward direction at
  both the start and the end edge. Studio takes the normal of each edge and orients it *outward*, that is, away
  from the centroid of all visual-LOD vertices. The start port's outward normal points backwards, the end port's
  points forwards. Studio's way also works for side arms, whose outward direction points away from the piece.
- **Turn angle**: the signed difference of the forward directions at the start and the end, wrapped to
  (−180°, 180°]. Builder counts a piece as straight below 0.01°. Studio uses 10⁻⁶ rad in its geometry and 10⁻⁵ rad
  when choosing pieces.
- **Chord**: the distance between the two port centres.
- **Radius** of a curve: chord / (2 · sin(|turn| / 2)). **Arc length**: radius · |turn| in radians. For a straight,
  the length is the chord.
- **Centre line** of a curve: an arc from the start port centre with that radius, turning to the side of the
  turn sign. Builder samples it at points t ∈ [0, 1]. Studio samples it about every 2° of turn (at least two steps)
  and snaps the last sample onto the end port exactly.
- **Footprint**: Builder offsets the centre line by half the width each side and snaps the corners onto the
  four memory points. A junction footprint is a polygon through the corners and the arm points. Studio uses the
  real triangles of the visual LOD with the *smallest resolution that has faces*. All quads are split into two
  triangles. Only X and Z are kept.
- **Object reference point**. Builder computes the centre of the bounding box over *all LODs* in X and Z. By
  default it writes that point as the object position, because the engine uses it for these parts
  ("autocenter"). An option writes the model origin instead. Studio's road placements and Terrain Builder Road Tool
  records use the **model origin**.

Builder classifies a piece by its name, not by its geometry, only for special kinds. A name containing `konec`
or `end` is an end cap. A name containing `crosswalk` is a crosswalk. A name starting with `kr_` is a junction.
Anything else is a straight or a curve depending on the turn angle. The road type ("family") is the part of
the name before the first `_` that is followed by a digit.

Studio has a fallback: if a model file is missing it derives a length or an arc from the file name, with a few
hard-coded mappings from short name numbers to exact lengths. This is not a method Náves should use. Náves
already measures every piece from its memory points. Studio refuses to replace a model that exists but is not
MLOD.

### 1.2 Reversed curves

All vanilla curve pieces turn the same way. Both tools get the other turn direction by **driving a curve from its
end to its start**. Terrain Builder's Road Tool does the same (see topic 7b). A reversed piece enters through its
`LE`/`PE` edge and leaves through `LB`/`PB`. The heading change is the negated turn angle.

Builder prepares each piece once as a "variant" in a local driving frame. The entry port centre is at the origin
and the driving direction is +Z. A variant stores the exit point, the heading change, the samples of its centre
line, and the reference point, all in that frame. For the reversed variant, the frame starts at the end port centre
and is turned so that the end edge's backward direction is +Z. The object's yaw is then the chain heading plus a
fixed per-variant offset. Straights are not reversed (they would give the same geometry). Builder sorts the
variants by length, longest first. That only matters when costs tie.

Studio places a piece by rotating it so that its entry port's outward normal is exactly opposite to the open
port of the chain, then moving it so that the two port centres coincide. The entry port is the start port, or
the end port for a reversed piece. The exit is the other port. Only curves may be reversed.

### 1.3 Builder: beam search along the line

**Input.** A polyline (repeated points closer than 0.1 mm are dropped, at least two distinct points are needed),
the set of allowed pieces, optionally an end cap, and constraints: a fixed start heading, a desired end heading,
"the end is a fixed arm" (see topic 2) and a list of junctions to insert.

**State.** The open end of the chain: position, heading, the chainage along the line reached so far, the
accumulated cost, the signed lateral offset from the line, the heading error, and the index of the next junction
still to be placed. The parent link gives the chain.

**Projection onto the line.** A point is projected only onto the line segments inside a chainage window. For a
step from chainage s with a piece of length L, the window is from s − 2 m to s + 1.5·L + 10 m. This window lets
the tool handle loops, hairpins and lines that cross themselves. The projection returns the squared distance, the
chainage of the foot point, the segment, and the side (left or right).

**Start states.** If the start heading is fixed, there is one start state. Otherwise the reference heading is the
bearing from the first point to the point 3 m along the line (or the end, if the line is shorter). The tool adds
states at that heading plus offsets from −range to +range in steps of 0.5° (the range is a setting, 0° by default).
Each start state is charged heading weight · offset². If an end cap is wanted at the start, it is placed first;
see 1.6.

**Expansion.** Every allowed variant is appended to every kept state. The candidate's cost is:

- the parent's cost,
- plus a **penalty per piece** (default 1.0),
- plus a **penalty per degree of turn** (default 0.05 per degree of |turn|),
- plus the **integrated squared distance** to the line: the centre line is sampled at about 1 m spacing (the
  exact step is length / ⌈length / 1 m⌉, at least two samples), each sample is projected in the window, and its
  squared distance times the sample step is added,
- plus the **heading error** at the exit, squared, times 0.02 per degree². The error is the difference between the
  new heading and the bearing of the line segment that the exit projects onto.

A candidate is rejected if its exit does not progress along the line by at least one bin (0.5 m), unless it is
already at the end of the line.

**Bins and pruning.** Candidates go into bins by chainage, 0.5 m per bin. The bins are processed in increasing
chainage. In each bin the states are sorted by cost and at most the **search width** (default 12) are expanded.
Near-duplicates are skipped: two states are the same if they agree in lateral offset rounded to 0.2 m, heading
error rounded to 0.5°, and the next-junction index. The first (cheapest) of them is kept.

**Finishing.** A candidate counts as a finished road if every junction has been placed, and either its chainage
is at the end of the line, or its exit is within the **end tolerance** (default 3.5 m) of the line's end point
*and* its chainage is beyond the line length minus the end tolerance minus 2 m. That second condition stops a
closed line (start equals end) from finishing at once. The finish cost adds end weight · (distance to end)². The
end weight is 4 per m² normally and 40 per m² when the end is a fixed junction arm. If an end heading is wanted,
it also adds 0.3 · (heading error in degrees)². The cheapest finished road wins.

**Failure.** If nothing finishes, the result is the chain that got furthest. "Furthest" means the most junctions
placed, then the largest chainage, then the lowest cost. A warning says that the end was not reached.

**Report.** After the search, the tool reports the largest and the RMS lateral deviation of all centre-line
samples, the road length, the gap between the road's end and the line's end, and a sanity check: the largest
distance between neighbouring pieces' joint centres (always about zero).

### 1.4 Studio: depth-first fitting with backtracking

Studio fits in two cases. It fits a drawn line ("Fit models to points"). It also fits the result of its terrain
routing (topic 3). For a drawn line it first rounds the corners (1.5). For a routed line it does not, because the
routed line already respects the minimum radius.

**Pieces.** Only pieces of one road type (the Road Tool road type of the end cap, see topic 7b) whose category is
straight or curve and that have no side arms are used. A curve whose radius (length / turn) is below the
**minimum radius** setting is left out. Roads must have a `konec` end cap. Tracks have none.

**Tolerance corridor.** The route tolerance (default 5 m for roads, 3 m for tracks) is a hard corridor. Every
sample of a candidate's centre line must lie within it of the line. Projection is again windowed: from the
current chainage minus twice the tolerance to the current chainage plus a look-ahead. The look-ahead is the
longest piece (or cap) plus twice the tolerance plus 10 m.

**One step.** From the open port, every allowed piece (and every curve reversed) is tried. A candidate is
rejected if:

- its exit does not move forward along the line by more than 1 cm, or
- its exit is further than the tolerance from the line, or
- its exit's chainage plus the cap length runs past the line end plus the tolerance, or
- any centre-line sample leaves the corridor, or
- any centre-line segment is not permitted by the terrain check (grade, forbidden areas; topic 3), or
- it is a curve that does not reduce the heading error compared to the current state, while the current
  position is already within half the tolerance of the line. This rule stops weaving inside the corridor.

**Score** of a candidate (lower is better), with angles in radians:

- 3 · distance of the exit from the line,
- plus the distance from the line of a look-ahead point. That point lies along the exit direction, at
  min(10 m, remaining length), but never less than 0. Capping at the remaining length stops a straight exit from
  looking worse than a curve that curls back near the end,
- plus 4 · the heading error at the exit (against the line direction at the exit's chainage),
- plus |turn| of the piece,
- plus, within the last 50 m of the line, 0.2 · the distance from the exit to the end point,
- minus 0.03 · the piece length (longer pieces are preferred).

**Search.** The four best candidates are kept on a stack frame, best on top. The best is taken. If a later step
has no candidates, the search steps back and takes the next alternative of an earlier step. The chain is cut back
to match. The search gives up after 50 000 steps, and also when the stack is empty. The error then names the
furthest chainage reached.

**Finishing a road.** Before each step, the tool tries to put the end cap at the open port. That succeeds if the
cap's end is within the tolerance of the line's end, its chainage is beyond the line length minus twice the
tolerance, and it passes the corridor and terrain checks. The cap is placed forward (not reversed). The start cap
is placed first in the same way, and it must pass the same checks. A line shorter than two caps is refused.

**Finishing a track.** Tracks have no caps. The end may be missed by up to half the shortest piece more than the
tolerance. The track finishes when the open port is within tolerance plus that slack of the end, and the chainage
is beyond the line length minus twice the tolerance minus the slack. Then straight runs are merged (1.6).

### 1.5 Rounding corners

Builder does not round corners beforehand. The search rounds them itself, because only real curves can be
appended. A sharp corner becomes the tightest suitable curves, and the user may exclude tight curves.

Studio rounds a drawn line before fitting, with **circular fillets of the minimum radius**:

- At each interior vertex, the turn angle θ is the angle between the incoming and the outgoing direction.
- Below 0.01 rad the vertex is kept as it is.
- A turn of π − 0.01 or more is an error (the line turns back on itself).
- The tangent length is R · tan(θ/2). It must be at most 0.45 of both neighbouring segments, or the tool reports
  that the points are too close for the radius. The tool never shrinks the radius.
- The arc starts at the tangent point on the incoming segment and is sampled about every 2° (at least two steps).
- Repeated points are an error.

### 1.6 End caps

Builder: an end cap is optional, at the start, at the end, or both. It is allowed only at a real dead end (a
node where one line ends; topic 2). The tool assumes that the cap's end edge (`LE`/`PE`) is the open end of the
road. At the start the cap is placed reversed, so that its end edge faces backwards; at the end it is placed
forward. A "flip" option swaps both. The start cap is appended to each start state before the search. The end cap
is tried from every candidate whose exit is within cap length + end tolerance + 1 m of the line's end; the
candidate with the cap is then tested for finishing. For other road types in a network, the cap with the same
suffix is used, or the first cap of that type.

Studio: caps are mandatory for roads, at both ends, always forward (see 1.4). The cap is a part of the end-cap
category whose file name contains `konec`. Roads without such a part cannot be fitted.

### 1.7 Merging runs of short straights

Studio only, and only for tracks. After fitting (and per stretch between bridges), every run of two or more
consecutive straights is replaced by the **fewest straights of the same total length**:

- Lengths are counted in whole centimetres (rounded).
- A table over 0..total gives, for each length, the fewest pieces that add up to it exactly (an unbounded
  coin-change problem). Ties keep the first solution found in the catalogue order.
- The chosen pieces are sorted longest first.
- A run is replaced only if the result has fewer pieces than the run.
- Then the whole chain is placed again from the first piece's entry port, so every joint stays exact. Straights are
  collinear, so the shape does not change.
- Every merged piece is checked again (terrain grade, forbidden areas, level crossings; topics 3 and 6), because
  the new joints sample the ground elsewhere. If any merged piece of a run fails, that run is marked "keep" and
  the whole replacement is repeated. This continues until nothing is rejected.

### 1.8 Comparison

| | Builder | Studio |
|---|---|---|
| Search | breadth by chainage, beam of N per 0.5 m bin | depth-first, 4 alternatives per step, backtracking |
| Objective | integrated squared deviation + heading² + per piece + per degree | greedy score per step; hard corridor |
| Corners | rounded by the search | filleted first at the minimum radius |
| End | soft (end weight), tolerance 3.5 m | hard (tolerance), cap must land |
| Caps | optional | mandatory for roads |
| Straight merging | no | yes (tracks) |

Builder's approach is a principled cost and finds good chains. Studio's is faster but greedy, and needs its
anti-weaving rule.

**What Náves has today.** `roadfit.rs` already does a beam search over pieces with a windowed progress measure and
buckets (24 per 5 m bucket), and then solves the straight-run lengths so that the end lands on a given pose. That
replaces Studio's merging step for roads. Pieces come from memory points (`catalog.rs`). Náves has no
pre-filleting of corners and no anti-weaving rule, which it may not need.

---

## 2. Junctions in a road network (Builder)

Studio places no junction pieces. Everything below is Builder's.

### 2.1 Building the network from lines

**Input:** all lines of one shapefile, a **join distance** (default 1.5 m, at least 0.01 m).

1. **Clean** each line (drop repeats under 0.1 mm). Lines with fewer than two points are dropped.
2. **Find split points.** Each line's bounding box is enlarged by the join distance, to rule out pairs cheaply.
   - *T without a shared vertex.* For each end of each line, project it onto every other line. If it is within
     the join distance, and the foot point is more than the join distance from both ends of that other line, the
     other line gets a split at that chainage.
   - *X without a shared vertex.* For each pair of lines, intersect every pair of segments. A crossing counts
     only if both parameters are strictly between 0 and 1 (10⁻⁹ margin) and the crossing is more than the join
     distance from the ends of both lines. Both lines get a split there.
3. **Cut** each line at its splits, in chainage order. A split is kept only if it is more than 0.5 m from the
   previous cut and more than 0.5 m from the line's end.
4. **Nodes.** Every piece end goes to the first existing node within the join distance, or starts a new one. This
   is first-come, not clustering. Then each node is moved to the mean of the ends that joined it, and the ends of
   the pieces are moved onto their nodes. Edges shorter than 0.5 m after this are dropped.
5. **Overshoots.** An edge is removed if it is a dangling end (one node of degree 1, the other of degree 3 or
   more) and shorter than max(5 m, 3 × join distance). This removes a line drawn a few metres past another one.
6. Each edge remembers its source line and that line's total length.

### 2.2 What a node becomes

By the number of edge ends at the node:

| Ends | Result |
|---|---|
| 1 | road end; an end cap may go here |
| 2 | the two edges continue each other as one road |
| 3 | T-junction |
| 4 | X-crossroad |
| 5 or more | not supported: warning, the roads just end at the node |

### 2.3 Choosing the through road

For every arm (edge end) at a junction, the **arm direction** is the bearing from the node to the point
min(12 m, half the edge length) along the edge.

The **pair score** of two arms is 180° − |difference of their directions| (0 = exactly opposite, a straight line
through). Arms from different source lines get +10° added, which favours arms of the same input line.

- **T:** of the three pairs, the lowest score is the through road. The third arm is the side road.
- **X:** of the three ways to split four arms into two pairs, take the one with the lowest summed score. Then the
  straighter pair is the through road if their scores differ by more than 5°. Otherwise the pair whose source lines
  are longer in total wins (ties go to the first pair).

### 2.4 Chains (continuous roads)

Edges are joined into chains. Starting from an unused edge, the chain is extended forwards and backwards. At a
degree-2 node it continues into the other edge. At a junction it continues only from one through arm into the
other. It stops at a dead end, at a side arm, or at an edge already used.

A closed loop whose both ends meet at the same junction, as through arms of each other, is opened instead in the
middle of its longest edge. A new node is made there, so every junction lies inside the road.

Each chain records the junctions it passes as a through road. Each such junction records that chain.

### 2.5 Road types and which junction piece

Every chain starts with the selected road type. A junction piece is chosen for each junction from the pieces
whose name says that this road type is the through road and whose kind (T or X) matches.

**The naming rule** (a fact of the vanilla model names): `kr_t_<through>_<side>` and
`kr_x_<through>_<side>[_<through after>]`. The second token says T or X, the third is the through road's type, the
fourth is the side road's type, and an optional fifth is the type the through road continues as after the junction.
The tool checks this against the memory points: a piece with both arms is an X, with one arm a T, with none it is
not usable.

**Automatic choice** (if the user named none, or the named one is not for this type): pieces without a fifth token
first, then pieces whose side type equals the through type, then alphabetical. Pieces with a fifth token are
therefore only used when the user selects them, and the through road keeps its type after them.

**Side roads take the side type** (option, on by default). A chain that is not a through road at any placed
junction takes the side type of the first junction piece at its start or end. That only works if the library has
that type. Its allowed pieces are the selected type's ticked suffixes, if that type has those; otherwise all its
straights and curves.

If a type has no junction pieces, the junction gets a warning. Its through road is still built continuously, and
the side roads start at the node, without a piece.

### 2.6 Order of building

Chains are built one at a time. The next chain is the first one that has no side end waiting for an unplaced
junction. If none is free (a circular network), the chain with the fewest waiting ends is taken, and among those
the one that is a through road at the most junctions. It gets a warning to check its ends.

### 2.7 Building a through road with junction pieces

The chain's points are concatenated. For each active junction it passes, the junction's chainage is found by
projecting the node in a ±40 m window around its approximate position.

Each junction becomes a **waypoint** in the beam search (1.3). It is a variant inserted like any piece, with its
own length:

- It may be inserted from a state whose chainage is at least (junction chainage − half its length − 4 m). The 4 m
  is the **shift window**, the setting "how far a junction may move along the road".
- Inserting it adds 2 · (distance between the piece's centre and the node)² to the cost. The centre is taken as
  the midpoint between its entry and exit.
- Ordinary pieces may not go past (junction chainage − half length + 4 m) while that junction is the next one
  still to place. So a junction can never be skipped.
- A road only counts as finished when all its junctions are placed.

**Orientation of a T.** The side direction is the side arm's direction relative to the outgoing through arm
(negative = left). If the model's arm is on the left and the side road is on the left, the piece is placed forward.
Otherwise it is placed reversed, that is turned by 180°. Only that one orientation is offered. An X is offered in
both orientations.

**Spacing advice.** After building, the tool warns about each pair of neighbouring junctions on one road that are
closer than half the sum of their pieces' lengths. The search then shifts them apart. The README advises at least
about 20 m between junctions. Junctions that could not be placed at all get a warning.

**After the road is built**, each placed junction stores the world position and outward direction of its arms.
The arm centre is the midpoint of `LD`/`LH` or `PD`/`PH`. The outward direction of the left arm is the `LH`→`LD`
direction turned by −90°, and of the right arm the `PD`→`PH` direction turned by −90°. Both are then placed with
the piece. Side roads are matched to arms greedily. The side road whose best arm fits its direction most closely
chooses first, and it takes that arm.

### 2.8 Side roads

A chain is reversed when it has a placed arm only at its end, so that it starts at the arm. Start connections are
exact. (An option reverses free roads too.)

- **Start at an arm.** The start is the arm centre, the start heading is fixed to the arm's outward direction, and
  line points within (distance from node to arm + 1 m) of the start are dropped.
- **End at a second arm.** The end is that arm's centre and line points that close to the end are dropped. The
  desired end heading is the arm's outward direction + 180°. The end is "fixed", so the end weight rises to 40 per
  m² and the end-heading weight of 0.3 per degree² applies (1.3).
- If fewer than two points remain, the line becomes the straight from start to end.
- End caps are allowed only at real dead ends.

Pieces have fixed lengths, so a road between two fixed arms rarely closes exactly. The tool warns if the end gap
exceeds 0.3 m and suggests fixing the last piece by hand.

### 2.9 Five or more arms, and other limits

A node with five or more ends gets a warning and no piece. The roads simply end there (they are still chains, but
the node is neither a junction nor a continuation). A line crossing itself is not turned into a junction.

**What Náves has today.** `roadnet.rs` builds the graph from OSM ways, joining ways at degree-2 nodes. `roadplace.rs`
places junction pieces on through roads in rank order. A side road starts at the piece's port. The piece may shift
along the through road in 6.25 m steps up to 25 m (`JUNCTION_SHIFT_*`) within 3 m of the crossing. Náves has no
need to split crossing lines without shared vertices, since OSM shares nodes; overshoot removal belongs to the
network clean-up.

---

## 3. Routing a road across terrain (Studio)

Studio can find a road between user waypoints over an ASC heightmap. The road passes **exactly** through every
waypoint with a continuous heading, keeps to a maximum grade and a minimum radius, keeps clear of forbidden areas,
and minimises length plus a climb penalty plus earthwork. Then it fits real pieces to the result (1.4).

### 3.1 Terrain sampling

Heights are bilinear between cell centres of the ASC grid. Outside the outer cell centres, or if any of the four
corners is NODATA, there is no height and the test fails.

### 3.2 Obstacles and the distance map

Obstacles are drawn polygons, rivers (polylines with a width), and optionally **all terrain below a water level**
(default 0 m, off by default).

Without an ASC, obstacles are tested exactly: a segment is blocked if it touches a polygon (either end inside, or
any edge crossing it). With a clearance, it is also blocked if it comes closer to a polygon outline than the
clearance. It is blocked if it comes within half a river's width plus the clearance of a river segment.

With an ASC, obstacles are rasterised once:

- **Resolution**: half the ASC cell, clamped to 1–2.5 m. It is multiplied by 1.25 until the raster has at most
  40 million cells.
- **Water**: a cell is an obstacle if the terrain at its centre is below the water level.
- **Polygons**: each row is scanned at the cell centres (even-odd rule). Every cell that a polygon edge touches is
  also marked, using an exact grid traversal that visits every cell a segment touches (Amanatides–Woo).
- **Rivers**: every cell a river segment touches, plus every cell whose centre is within half the river width of
  the segment.
- **Distance map**: a two-pass chamfer transform with orthogonal and diagonal steps (weights: resolution, and
  resolution · √2), stored in decimetres in 16 bits, saturating.
- **Test of a segment with a clearance**: walk every cell it touches. It is blocked if any cell is an obstacle or
  its distance is below ((clearance + resolution) · 1.0824 + resolution · √2) (in metres; ×10 in decimetres). The
  1.0824 covers the chamfer error, and the extra terms cover the cell size, so a passing segment keeps at least
  the clearance from the real area. With zero clearance, only obstacle cells block.

The routing clearance is the **route tolerance**. Each waypoint must itself be clear of obstacles by that
clearance and lie on the terrain, or routing is refused.

### 3.3 The profile band: grade, cut and fill along a polyline

This is the core test of every step. Its inputs are a polyline, the maximum grade g (percent / 100), the maximum
**cut** c and **fill** f (metres, both 0 by default, each limited to 0–100 m), the clearance, and optionally the
interval of road heights reachable at the start.

1. Every segment must pass the obstacle test.
2. The polyline is sampled at spacing ASC cell / 2, clamped to 0.1–5 m.
3. At each sample with terrain height z, the allowed band is [z − c, z + f].
4. The reachable interval propagates. At the first sample with no start interval it is the band. After that it is
   [lo − g·ds, hi + g·ds], intersected with the band, where ds is the sample spacing. An empty interval (lo > hi by
   more than 10⁻⁹) means the step is impossible.
5. Along the way the tool tracks a *nominal* road height h: the terrain height clamped into the interval. It
   accumulates the **climb** Σ|h − h_prev| and the **earthwork** Σ|h − z| · ds (m²).
6. The result is the climb, the earthwork, and the final interval. The next step starts from that interval.

With c = f = 0, this is just the grade between terrain samples. Note that the propagation is forward only. The
interval says which heights can be reached from the start without breaking the grade. Whether a height can still
reach the end is settled later by the profile design (topic 5).

A route point is "permitted" from a to b when the band test with zero clearance passes. The fitter (1.4) uses it.

### 3.4 The coarse cost-to-go field

One field per leg (one per target waypoint), to guide the search:

- **Grid**: resolution = max(routing step, ASC cell), multiplied by 1.25 until there are at most 1 million cells.
  It covers the ASC extent.
- **Dijkstra from the target's cell.** Its start cost is the distance from the cell centre to the target. The grid
  is 8-connected.
- An edge between neighbouring cell centres is allowed if the band test passes **with the grade limit removed**
  (cut, fill, NODATA and obstacles still apply). The test runs from the neighbour towards the current cell, in
  the direction of travel.
- **Edge cost**: if cut + fill > 0, it is just the distance. Otherwise it is max(distance, climb / g) +
  slope weight · climb. The climb divided by the grade is the shortest road that can make that climb in any shape
  (serpentine, spiral, detour). That makes the field a useful lower bound on steep ground.
- **Estimate** at a point: the field value of its cell, divided by 1.0824 (octile paths are up to about 8 % longer
  than straight ones), minus one cell size, not below 0. Cells without a value give 0.

When bridges are possible, the field is built with the fill raised by 10⁻⁶ m. That moves it onto the
"distance only" branch, so a gap a bridge could cross is not treated as a wall. If bridges may cross forbidden
areas, the field is also built without obstacles.

### 3.5 The arc search for one leg

A hybrid A* from waypoint k to waypoint k+1.

**Settings**: routing step s (default 20 m for roads, 25 m for tracks), minimum radius R (default 25 m; 110 m for
tracks), maximum grade (default 12 %; 3 % for tracks), slope weight (default 4; 8 for tracks), route tolerance
(default 5 m; 3 m for tracks), earthwork weight (default 0.5 m of road per m²).

**Steps.** The maximum heading change per step is θ = min(s / R, 45°), so a step never curves tighter than R. Each
state has five successors: arcs of length s with heading change 0, ±θ/2, ±θ. An arc of length s and change δ has
radius s/δ, and is sampled about every 2° (at least one segment).

**Exact arrival.** If the state is within max(3·s, min(R, 300 m)) of the target, the tool also tries one circular
arc from the pose exactly through the target. It is allowed only if the angle α between the heading and the
direction to the target is under 90°, and the radius c / (2·sin|α|) is at least R (c is the distance). Its heading
change is 2α and its length r · 2|α|. Such a successor is marked "arrived" and its position is set exactly to the
target.

**State key** for duplicate detection: the arrived flag, position in cells of s/2, and heading in bins. The number
of bins is ⌈2π / (θ/4)⌉, clamped to 32–180. A new state is dropped if a stored state with the same key has a cost
that is not higher. Otherwise it replaces it as the key's best. Popped states that are stale (their priority is
above cost + weight · heuristic) are skipped.

**Cost of a step**: length + slope weight · climb + 0.1 · |δ| · R + earthwork weight · earthwork. The curvature
term prefers straight runs. Each step is tested with the band test from the parent's interval. Its final interval
is stored in the state.

**Heuristic** (0 for arrived states): the larger of two values.

- An analytic bound: the excess height Δ = max(0, |z_target − z_here| − c − f). The bound is
  max(distance, Δ / g) + slope weight · Δ.
- The field estimate (3.4).

**Start of the first leg.** The heading is free. The tool seeds one state heading straight at the target plus one
state per heading bin, all with cost 0 and no height interval.

**Arrivals.** An arrived state on the last leg ends the search. On an earlier leg, an arrival is kept only if the
road can leave from it: at least one of the five step arcs passes the band test from its interval. The tool keeps
the **cheapest arrival per 45° heading sector**, up to 8 arrivals. Once the first arrival exists, the search stops
when the popped priority exceeds twice the first arrival's cost plus 100, or when the stored states exceed 1.5 ×
(the count when it was found) + 50 000.

**Legs and headings.** Every arrival of leg k becomes a start of leg k+1, with its heading, its cost and its height
interval. The path that led to it is carried along. So the next leg picks the combination of arrival direction
and onward route that suits it best, and the heading is continuous through every intermediate waypoint. After the
last leg the cheapest complete path is taken. Consecutive duplicate points are removed.

**Exact search first, weighted after.** Each leg is tried with heuristic weight and state budget (1, 500 000), then
(1.5, 1 000 000), then (3, 1 500 000). A weighted search may find a route faster but not the optimum. The next
pair is tried only when the budget ran out. If a search ends because there are no states left, that proves no
route exists, and the tool stops at once.

**When nothing is found**, the error names the waypoint and the closest distance any state came to it. A special
message appears if the road could not leave an intermediate waypoint at all in the direction it arrived from (the
closest approach equals the leg length). The message for an exhausted budget is different.

### 3.6 From route to pieces, and the margin

The routed polyline is fitted with pieces without rounding its corners again (1.4). The fitted pieces are always
checked against the **full** settings. Several margins make that likely to succeed:

- **Radius.** The search uses the radius of the tightest usable curve piece of the road type (length / turn), but
  only among curves not tighter than the minimum radius. It is multiplied by 1.05 and kept at least the minimum
  radius. If the type has no curves, the minimum radius is used. A routed line tighter than any curve could not be
  followed.
- **Grade.** The search first runs with 90 % of the maximum grade, because the pieces may deviate from the line by
  up to the tolerance and so sample other ground. If fitting fails, it runs again with the full grade.
- **Bridges.** With bridges enabled, the route is first searched with bridges, then without (topic 4).

So the order is: bridges at 90 % grade, bridges at 100 %, no bridges at 90 %, no bridges at 100 %. The first
attempt that gives a fitted road wins. If all fail, the last error is returned.

**What Náves has today.** Nothing comparable. Náves follows OSM geometry and does not search for new alignments.
It has DEM sampling, a heightmap and a joint height solve over given lines (`profile.rs`), but no obstacle distance
map, cost field or arc search.

---

## 4. Bridges inside the route search (Studio)

### 4.1 When a bridge step is offered

A bridge step is tried from a state if all of these hold:

- a bridge model is available for the road type (4.4), and the maximum bridge length > 0;
- the state is not the start and was not itself reached by a bridge;
- the plain straight step of length s from this state fails the band test.

For tracks the last condition reads: no straight piece (ignoring short ones, see 6.3) fits.

### 4.2 Building the span

The **module** is the bridge model's length, the distance between its start and end points (4.4). No bridge is
built if the module is under 1 m, longer than the maximum bridge length, or the maximum bridge height is 0.

1. The deck height at the start is the ground there, clamped into the state's height interval.
2. For n = 1, 2, … up to ⌊max length / module⌋ (and up to the model's piece limit), the span is straight along the
   current heading with length L = n · module.
   - The ground at the end must exist, or the loop stops. The **landing height** is that ground.
   - |landing − deck| must be at most g · L. The deck keeps the maximum grade. If not, try the next n.
   - The road must be able to go on: one step from the end (the step length for roads, the longest piece for
     tracks), straight on, must pass the band test, starting from the landing height exactly.
   - If the span crosses a forbidden area (tested with the clearance), there is no bridge at all unless "bridges
     may cross forbidden areas" is set.
   - Sample along the span at spacing ASC cell / 2, clamped to 0.5–5 m. The deck height is linear from the start
     to the landing. Every inner sample must have terrain no higher than deck + 0.5 m (a bridge never cuts into
     higher ground). The depth below the deck must also be no more than the support reach: min(the model's pier
     depth, maximum bridge height).
   - The span must be needed: the deepest point below the deck must exceed the maximum fill, or the span must
     cross a forbidden area. Otherwise an embankment does the job and the next n is tried.
3. The first n that passes is the bridge. The search continues at its end with the heading unchanged and the height
   interval fixed to the landing height.

**Defaults**: maximum height 20 m, maximum length 150 m, bridge penalty 4 m of road per m of bridge, bridges may not
cross forbidden areas. Either limit at 0 turns bridges off; older projects load with 0.

**Cost** of a bridge step: L · bridge penalty + 2 · module. The fixed term discourages tiny bridges.

### 4.3 Splitting the road for output

The search returns the route and, per bridge, its two end points and piece count. Fitting then goes stretch by
stretch:

1. Fit the line from the start (or the previous bridge's exit) to the bridge's start point.
2. Lay the bridge from the **actual** end of the fitted pieces, aimed at the bridge's far point, with the planned
   piece count. The pieces are laid end to end along a straight line: each piece is turned so that its start→end
   axis points along the heading, and its start point sits on the previous piece's end.
3. The next stretch starts at the bridge's exit.

A route stores its parts and, per bridge, the number of parts before it, the deck height above the model origin,
and the bridge pieces. The stretches are the runs of parts between bridges.

- In the Road Tool export (7b) **each stretch becomes a separate road**. The tool refuses to export parts that do
  not connect, so a road without its bridge split would be rejected.
- Bridges are written as **objects** in the object import file (7a).
- Terrain shaping (topic 5) works per stretch and leaves the ground under a bridge as it is.

In 3D, a bridge's pieces are placed on the straight line between the two banks. The end of the stretch before
and the start of the stretch after are at their terrain (or track) heights. The slope is limited to a sine of ±0.5.
The pieces are tilted onto that line and lowered by the deck height, so the driving surface meets the banks.

### 4.4 Which bridge model, and what is measured

- **Roads**: the widest point of the road type's straights decides. The width is twice the largest |x| of their
  footprint vertices. Up to 5.5 m: `Bridge_Wood_25`. Up to 9.5 m: `Bridge_Stone_25`. Wider: `Bridge_Metal_25`. They
  are read from a `Bridges` folder next to the road parts folder. The wooden bridge is limited to one piece.
- **Tracks**: `Rail_Bridge_15` from a `Bridges` folder next to the track folder. Not for `Rail_TrackSmall`.
- Measured from the binarised model at run time:
  - Road bridges: the start is the midpoint of memory points `lb`/`pb` and the end the midpoint of `le`/`pe`. The
    deck height is the height of `lb`.
  - Rail bridges: the start and end are memory points `map1`/`map2`. The deck is the highest vertex of the
    roadway LOD (resolution 3·10¹⁵) minus a fixed offset between a track's origin and its roadway. Studio
    hard-codes that offset as a measurement of the vanilla track; Náves must measure it from the track models (the
    roadway LOD top against the origin).
  - The pier depth is the deck height minus the lowest vertex of the first (visual) LOD.

**What Náves has today.** Bridges are placed where OSM marks a bridge (`bridgefit.rs`, `bridgekind.rs`,
`bridgeground.rs`, research note 2026-11-18). There is no search that decides by itself where a bridge is cheaper
than an embankment. The splitting of a road around a bridge exists in Náves's own form.

---

## 5. Designing a height profile and grading terrain under a road (Studio)

### 5.1 Designing the profile

**Input**: the centre line of the fitted parts of one stretch, the maximum grade, cut and fill.

1. Sample the centre line at spacing ASC cell / 2, clamped to 0.5–2.5 m. Get the terrain height z_i and the step
   length ds_i at each sample. A sample without terrain is an error.
2. **Forward pass.** Start with lo_i = z_i − c and hi_i = z_i + f. For i = 1…n−1:
   lo_i = max(lo_i, lo_{i−1} − g·ds_i − 10⁻⁹) and hi_i = min(hi_i, hi_{i−1} + g·ds_i + 10⁻⁹).
   - If lo_i > hi_i by less than 5 cm, both are set to their mean. This absorbs the difference between routing's
     coarser sampling and this one.
   - If the gap is larger, it is an error that reports the missing metres.
3. **Backward pass.** For i = n−2 down to 0: lo_i = max(lo_i, lo_{i+1} − g·ds_{i+1}) and
   hi_i = min(hi_i, hi_{i+1} + g·ds_{i+1}). Now every height in [lo_i, hi_i] can reach both ends.
4. **Pick** a profile closest to a target t: h_0 = t_0 clamped into [lo_0, hi_0]. For each next i, h_i = t_i clamped
   into [max(lo_i, h_{i−1} − g·ds_i), min(hi_i, h_{i−1} + g·ds_i)]. This is always feasible after the two passes.
5. The first profile is picked with the terrain as the target.
6. **Smoothing** (only if cut + fill > 0): 30 rounds. In each round, take the current profile, apply three passes
   of a [¼, ½, ¼] filter to the inner samples (the ends stay), and pick again with that as the target. The result
   rounds the grade changes into gentle vertical curves without leaving the band or breaking the grade.

The report gives the largest fill (h − z) and the largest cut (z − h) on the centre line, and the length.

### 5.2 Setting the terrain under the footprint

**Input**: the footprint triangles of all parts of the stretch (the real visual-LOD triangles; degenerate ones
dropped), the profile, the side slope (default 1.5 m horizontal per metre vertical, allowed 0.5–10), and the
blending width (default 0 m, allowed 0–500 m).

- **Core**: cell / √2, half the cell diagonal. A grid point whose distance to the footprint is within the core gets
  the road height in full. The reason is that terrain heights are interpolated between grid points, so every grid
  point whose cell square touches the footprint shapes the road surface. A narrower radius leaves a checkerboard of
  graded and untouched points when the road is narrower than the cells.
- **Reach**: core + blending width + depth · side slope + one cell. The depth is max(c, f, largest |h − z|) and at
  least 3 m. Candidate grid points lie in the footprint's bounding box grown by the reach.
- For each candidate point with a terrain value:
  - **d** is the distance to the nearest footprint triangle (0 inside).
  - **Road height** is taken from the nearest **profile segment**: the nearest point on the sampled centre line,
    with the profile interpolated linearly there. Studio's changelog notes that this is what stopped serpentine legs
    sharing averaged cells. Each grid point takes the height of the one nearest part of the route, not a mean.
  - If d > reach, the point is left alone.
  - If d ≤ core, the new height is the road height.
  - Otherwise, with out = d − core:
    - if the road is above the terrain (embankment): side = max(terrain, road − out / side slope);
    - if it is below (cut): side = min(terrain, road + out / side slope).
    - The slope stops where it meets the terrain.
  - **Blend**: if the blending width w > 0 and out < w, with t = out / w, the new height is
    side + (road − side) · (1 − (3t² − 2t³)), a smoothstep from the road height to the slope.
- Only points that change by more than 10⁻⁹ m are written. NODATA points stay NODATA. If no point changes, it is an
  error that suggests checking the ASC's extent and resolution.
- To find the nearest triangle and segment quickly, both are put into 16 m bins. A point looks at all bins within
  ⌈reach / 16⌉ rings.

An older method (still in the code, with blending) graded only the footprint. It interpolated a shape-preserving
cubic Hermite profile between the terrain heights at the part joints. The joint slopes are the weighted harmonic
mean of neighbouring secant slopes, or zero at a local extremum (as in monotone cubic interpolation). It took the
strongest weight per cell. The new method above replaced it.

### 5.3 Layering, caching and undo

The source ASC is never changed. Each graded stretch is a **layer** with an order number (later wins), its parts,
blending width, maximum grade, cut, fill and side slope. Every stretch of a route between bridges is its own layer,
numbered after its route.

**Composition.** Start from the source heights. Apply the layers in order. Each layer is computed on the terrain
as it stands after the earlier layers, and its result is a **patch**: the list of changed cell indices and their new
heights. Later layers overwrite earlier ones where they overlap.

**Cache key of a patch**: a hash of the source (path, origin, cell size), the layer's parts (model, reversed,
position, rotation), its settings, and the keys of every earlier applied layer whose **bounds** overlap this layer's
bounds. The bounds are the parts' positions grown by blending width + (max(cut, fill) + 20) · side slope + 3 cells
+ 50 m. So removing a route reuses every patch it did not affect. Patches are stored as files. A layer whose models
are not loaded is skipped for now, and retried when they are. A layer that cannot be built is reported with the
reason and skipped. The composed terrain is also cached under a hash of the base and the applied keys, and older
composites are removed.

**Planning over adaptations.** A route is planned over the terrain with the other routes' layers but **without its
own previous layer**. So replanning does not route on its own embankments.

**Export** writes the composed heights as a new ASC. The NODATA value is −9999, or a value below the lowest
height if the terrain goes that low.

### 5.4 Comparison with routing

Routing only knows that a feasible height band exists (forward propagation, 3.3). Grading then designs an actual
profile inside that band with both passes and smoothing. Routing samples at ASC cell / 2 clamped to 0.1–5 m;
grading clamps to 0.5–2.5 m. The 5 cm tolerance in 5.1 bridges that difference.

**What Náves has today.** `profile.rs` solves all chain heights at once as a quadratic program with grade, grade
change, junction and clearance rules, which goes beyond Studio's per-stretch band and smoothing. `conform.rs`
shapes the terrain: vertices within half a road width and one and a half cells of the centre line take the road
height, outside that cut and embankment slopes, and bridges are handled separately. Náves rebuilds the whole
terrain per run, so it needs no layer cache.

---

## 6. Railways (Studio)

### 6.1 Which pieces can be chained

Track families are names of the form `Rail_Track…` (for example `Rail_Track`, `Rail_TrackE`, `Rail_TrackSmall`).
The name splits into the family (the first two `_`-separated tokens) and the piece name (the rest).

A piece is **routable** (chained automatically) if its piece name, in lower case, is:

- only digits (a straight of that nominal size), or
- digits followed by `m`, or digits followed by `deg`, or exactly `sp`, or
- `l` or `r` followed by digits, `_`, digits (a left or right curve),

and does not end in `_lc` or `_nolc`. Everything else is placed by hand: switches and turnouts, `Up_`/`Down_`
vertical pieces, type transitions, `Passing_*` level crossings, `_lc`/`_nolc` variants.

Pieces whose name starts with `passing_` (and do not end in `_lc`/`_nolc`) are loaded too, as **level-crossing
pieces**. They are never chained by the search.

The tool checks a routable piece's geometry: it is a curve if its turn exceeds 0.1°, otherwise a straight. Pieces
without both ends are skipped with a warning.

### 6.2 A track piece's centre line from a binarised model

Track models exist only binarised (ODOL). Studio reads ODOL version 53–60 (DayZ 1.30 writes 56). It reads only the
first LOD (visual) and the memory LOD (resolution 10¹⁵). For bridges it also reads the roadway LOD (3·10¹⁵). The
method:

1. **Direction of travel**: the memory points `map2` → `map1` (start → end). Their distance is the chord. If it is
   under 0.5 m, the model is refused. A model without `map1`/`map2` is not a track; non-track `rail_*` or `bridge_*`
   models get a box footprint from their first LOD.
2. **Why not just the memory points**: they can be off by centimetres. The rail heads in the visual LOD are exact.
3. **Rail heads**: the vertices of the first LOD within 2 cm of its highest vertex.
4. **Local frame**: origin at the midpoint of `map1`/`map2`. The axis u goes along the direction of travel, n is to
   its right. Each head vertex gets (along, side) coordinates.
5. **Centre samples**: sort the head vertices by "along". Group vertices that lie less than 0.3 m along from the
   first of the group. A group that spans more than 0.5 m across has both rails. Its sample is (mean along, mid-point of its
   lowest and highest side). Fewer than two samples is an error.
6. **Fit**: least squares of side as a quadratic in along (side = k₀ + k₁·s + k₂·s²). With fewer than four samples
   the fit is linear. The piece is **curved** if |k₂| > 2·10⁻⁶. If not, the quadratic term is dropped and folded into
   the offset as k₀ + k₂ · chord² / 12 (its mean over the chord).
7. **Ends**: start with s = ±chord/2. Four times, move each end outward by the largest projection of any rail head
   (relative to the current end point) onto the local direction at that end. The ends then sit at the true rail
   ends.
8. **Centre line**: sample the fitted curve between the ends about every 2.5 m (at least two steps). Its length is
   the sum of the sample distances. The ports are at the two ends. Their outward directions are the curve's tangent
   (backwards at the start, forwards at the end).
9. **Footprint width**: from first-LOD vertices within 0.5 m of the middle (along), the larger distance of the
   extreme side values from the centre offset. That is half the width, clamped to 0.5–10 m, or 1.5 m if no
   vertex is found. The footprint is a strip of that half width along the centre line.

The ODOL reader needs: finding the LOD offset table, decoding LZO1X-compressed arrays (an array of 1024 bytes or
more is compressed, smaller ones are stored), named selections with vertex indices, and the vertex positions. Náves
already has such a reader (`naves-formats/src/odol.rs`, from the public `bis-file-formats` layout and our own
observation). Studio finds the LOD table by scanning for a consistent set of start/end offsets after the LOD
resolutions. That is a heuristic; Náves should keep its own reader.

### 6.3 The track search

Tracks are searched **with the real pieces as steps**. There is no separate fitting afterwards. It is the same
hybrid A* as in 3.5, with these differences:

- **Steps**: every routable straight and curve of the family, and every curve reversed. Curves tighter than the
  minimum radius (length / turn) are left out. Each step's centre line, exit point and turn are precomputed in the
  piece's entry frame.
- **Short pieces**: pieces under 8 m (when the longest piece is at least 8 m) are used only within longest piece +
  reach of the target. Otherwise every cell fills with tiny steps.
- **State key**: position in cells of a quarter of the longest straight, clamped to 2–12 m, and the heading in 72
  bins. There is no arrived flag; arrival is by distance.
- **Arrival**: within the tolerance of the target, plus half the shortest piece on the last leg. An intermediate
  arrival must be able to leave: some piece must fit from it. It keeps one arrival per 45° sector, up to 8, with the
  same stopping rule as 3.5.
- **Step test**: the level-crossing rule (6.5) must allow the piece on its line, and then the band test (3.3) with
  the tolerance as clearance.
- **Step cost**: piece length + slope weight · climb + 0.1 · |turn| · R + earthwork weight · earthwork.
- **Bridge step**: offered when no straight (ignoring short ones) fits. The onward check uses the longest piece's
  length.
- **Budgets**: (1, 300 000), (1.5, 700 000), (3, 1 200 000). A search that ends without states proves there is no
  route.
- **Defaults** (track constraints): maximum grade 3 %, slope weight 8, step 25 m (used only for the coarse field),
  minimum radius 110 m, tolerance 3 m, cut and fill 0 m, earthwork weight 0.5.
- **Output**: the pieces are placed from the recorded poses. Straight runs are merged per stretch between bridges
  (1.7). The merged pieces are checked again for grade, obstacles and level crossings. Then level crossings are
  turned into crossing parts (6.5).

### 6.4 Laying rigid pieces on the terrain

Road parts follow the terrain in game. Track pieces do not: they are rigid objects. So each piece gets its own
pitch, roll and height, and the chain must stay connected in 3D.

1. For every piece, take its chord on the plan (entry port to exit port, as placed).
2. **Roll** (side slope): at the chord's midpoint, sample the terrain 2 m to the right and 2 m to the left of the
   track. roll = atan((right − left) / 4 m), in degrees, or 0 if a sample is missing.
3. **Limit roll between neighbours** to 1°. In a forward pass, clamp each roll into ±1° of the previous one. In a
   backward pass, clamp it into ±1° of the next one. (At the rails, 1° is about a centimetre.)
4. **Pitch and height**, piece by piece, along the chain:
   - The start is the plan entry plus the accumulated horizontal shift.
   - The start height is the previous piece's end height if this piece continues it (the plan points are within
     0.5 m). Otherwise it is the terrain at the start.
   - The slope is found by four rounds of fixed-point iteration. With cos = √(1 − sin²), take the end at
     start + direction · length · cos. Set sin = (terrain at that end − start height) / length, clamped to ±0.5.
   - The chord in 3D is (direction · cos, sin). The piece is a rigid body of fixed length, so its end lies at
     start + chord · length. It runs from the previous piece's end to the terrain at its own far end.
   - The side axis is the level side vector turned about the chord by the roll. The up axis is the cross product of
     side and chord.
   - Every model point is then moved: its offset from the plan entry is rotated from the level frame into the tilted
     frame and added to the new entry. The piece's origin follows.
   - The horizontal shift for the next piece is new end − plan end. Sloped pieces cover less ground, so the chain
     ends slightly short of its plan. The README gives about 0.5 m per km at a 3 % grade.
5. **Extra objects** (crossing decks) take the tilt of the nearest piece (by distance to the piece's chord
   midpoint).
6. **Bridges** take the straight line between the end of the stretch before and the start of the stretch after
   (4.3).
7. **Angles for export**: the yaw is the heading of the forward axis, clockwise from north. The pitch is the
   arcsine of the forward axis's vertical component (nose up positive). The roll is the angle of the side axis
   about the forward axis against the level side. The written height is the origin height minus the terrain at the
   origin. The signs of pitch and roll can be inverted by options, because the tool cannot verify Terrain Builder's
   convention.

The grading of the terrain under tracks (topic 5) uses the same method as for roads, with the track's footprint.
The README asks the user to export the ASC with the same adaptations, because the track heights are relative to it.

### 6.5 Level crossings

**Roads considered**: every road from the project and every road route, as centre lines with a half width. The
half width is the footprint area divided by the centre-line length, halved, clamped to 2–12 m, or 4 m if the
length is under 1 m. All parts of one road count as one road. Road segments are indexed in 50 m cells, each reaching
half width + 10 m around the segment.

**Hits**: a piece's centre line is intersected with the road segments in the cells it touches. Each crossing gives
the point, the distance along the piece, the angle between track and road (0–90°), and the road's half width. Hits
closer than 0.5 m to an existing hit are merged.

**Rules for a piece** (when there are roads):

- A piece that comes closer than half width + clearance to a road it does **not** cross is refused. Tracks must
  cross roads cleanly, not graze them.
- A piece without hits is fine.
- A piece with hits must be a straight, or the family's crossing piece itself. For standard track (see below), it
  must be exactly the straight that the crossing piece replaces.
- For every hit:
  - the angle must be at least **60°**;
  - the needed span along the track is 2 · half width / sin(angle) + 2 · 1 m;
  - the crossing kit must cover that span;
  - the hit must be far enough from both piece ends: at least max(cover / 2, (half width + clearance) /
    sin(angle) + 0.5 m), so neighbouring pieces stay out of the road.

**Crossing kits by family** (a naming rule plus what is found on disk):

| Family | Crossing |
|---|---|
| `Rail_Track` | the straight `Rail_Track_25` is **replaced** by `Rail_Track_Passing_25`, which has the same geometry. The span must fit within that piece's length. |
| `Rail_TrackE` (and any other family) | **deck blocks** `Rail_Crossing_Block_10` / `Rail_Crossing_Block_25` laid on a plain straight. The shortest block that covers the span is used. |
| `Rail_TrackSmall` | **tiles** of `Rail_Crossing_Block_Small`, as many as cover the span. |

Blocks are looked for in the track folder and in a `Misc` folder next to it. Their length and centre along the
track come from the first LOD's bounding box. A block is placed as an extra object centred on the hit, with the
piece's rotation. Tiles are laid one after another from −span. Barriers are not placed.

After the search the crossings are applied. A hit that is not allowed (on a curve, too oblique, no room) is an
error naming its position.

### 6.6 Rail bridges and the export

Rail bridges: see topic 4. Tracks are not part of the Road Tool network. Track pieces, crossing decks and the
bridges of all routes (for roads only the bridges) are written to the object import file (7a). They are imported
in Terrain Builder with File › Import › Objects.

**What Náves has today.** Nothing for railways. The research note `2026-11-03-dayz-osm-terrain-research.md`
(sections 2 and 3.4) lists the rail pieces and notes that rail grades are tiny and the terrain must give way more.
The memory list has railways as future work. Náves's ODOL reader already reads the memory LOD. It would need the
first LOD's vertices for the rail-head method.

---

## 7. Terrain Builder files

### 7a. The object import text format

Both tools write a text file that Terrain Builder imports with **File › Import › Objects** into the active object
layer. One object per line, fields separated by `;`:

| # | Field | Builder | Studio |
|---|---|---|---|
| 1 | template name in double quotes | the `.p3d` file name without extension | the same, lower-cased |
| 2 | X (easting, m) | 3 decimals | 6 decimals |
| 3 | Y (northing, m) | 3 decimals | 6 decimals |
| 4 | yaw, degrees, clockwise from north, in [0, 360) | 4 decimals | 6 decimals |
| 5 | pitch, degrees | always 0 | nose up positive (option to invert) |
| 6 | roll, degrees | always 0 | as in 6.4 (option to invert) |
| 7 | scale | 1 | 1 |
| 8 | Z, height **relative to the terrain**, m | 0 (configurable) | origin height minus terrain |
| 9 | underground flag | empty (line ends with `;`) | `0`, no trailing `;` |

- Coordinates are Terrain Builder coordinates. The usual DayZ frame has its origin at easting 200000, northing 0.
  Builder has offset options for lines without that offset.
- Builder writes `\r\n` line ends, UTF-8 without a byte order mark, and always `.` as the decimal separator.
- The template name must exist in a Terrain Builder library, or the import fails with "Wrong file format or source
  template not found".
- On import, the height option (under "Advanced") must be **relative**.
- **Position.** Builder writes the bounding-box centre over all LODs by default (the engine's autocenter
  reference), with an option for the model origin. Studio writes the model origin of binarised models. For
  binarised models the two may be the same, because binarisation centres the model; that is not determined.
- **Yaw.** Builder offers "invert yaw" and a yaw offset, because the convention "cannot be verified" without the
  user's setup.
- Studio's underground flag is always 0 for tracks. The 9th column is the per-object underground flag of Terrain
  Builder's import (also noted in research section 2 of `2026-11-03-dayz-osm-terrain-research.md`).

**To verify on our own files** before Náves relies on this format:

1. Whether Terrain Builder's import uses the model origin or the bounding-box centre for unbinarised and for
   binarised models.
2. The sign of yaw (clockwise from north) and the signs of pitch and roll.
3. Whether the 9th column is accepted empty, as 0 and as 1, and what 1 does.
4. Whether the name must match case.
5. The meaning of the relative height for tilted objects: whether it is measured at the origin.

**What Náves has today.** Nothing. Náves writes the world (`.wrp`, 8WVR) and the mission itself. It has no
Terrain Builder object text writer, and it does not need one for its own builds.

### 7b. Road Tool roads inside a `.tv4p` project

> **Kept for reference; not planned for implementation.** Náves will not write `.tv4p` files. Terrain Builder
> cannot be run here, and Náves is meant to replace it. This record is kept for reading other people's projects
> later.

**Certainty.** Both tools say that the format is undocumented and was worked out from real project files. Builder
resolves every tag by its **name** through the file's own name table. Studio uses **fixed tag numbers**. That both
work suggests the numbers are stable in practice. Treat everything below as observation, not specification.

#### Container (the "ALB1" format; Builder's reading)

- The file starts with `ALB1`. Builder requires at least 32 bytes. The first 16 bytes are a header whose meaning is
  not determined.
- From offset 16: **name tables**. Each one is a field with any tag and type byte `0x0F`, then a u32 count, then
  that many entries of u16 id, u16 length, and text (Latin-1). The first table holds **tag names**, the second
  **class names**. Builder requires at least two.
- Then the **root object**: a u16 class id and a u32 object id, followed by fields up to the end of the file.
- **A field** is a u16 tag, a u8 type, and a value. All values are little-endian.

| Type | Value |
|---|---|
| `0x01`, `0x09` | 1 byte |
| `0x20` | 3 bytes |
| `0x05` | i32 (used for indices and codes) |
| `0x06`, `0x07`, `0x08`, `0x0A` | 4 bytes |
| `0x0E` | 6 bytes |
| `0x13` | no value |
| `0x14` | f64 |
| `0x0B` | string: u16 length, then bytes |
| `0x12` | u8 count, then count × 4 bytes |
| `0x15` | f64 array: u8 count, then count × 8 bytes |
| `0x0C` | **list**: u32 byte length (counting from after this length field, so it includes the count), u32 count, then the items |
| `0x0D` | **object**: u32 byte length (from after the length field), u16 class id, u32 object id, then its fields |
| `0x0F` | pair: two fields |

- List items are objects with the tag named `item`. In practice that tag has id 6, so the items start with the
  bytes `06 00 0D`.
- Studio's own parser of road bodies reads type `0x0D` as a 4-byte value. That conflicts with the table. It is
  probably never reached inside a road record. Not determined.
- Builder checks that lists hold as many items as their count, and that objects end exactly where their length
  says.

#### Where the roads live

| Name (Builder) | Tag (Studio) | Meaning |
|---|---|---|
| `mroad` | `0x88` | list of **road type definitions** |
| `mcross` | `0x89` | list of **crossroad definitions** |
| `mroadGetRoads` | `0x8A` | list of **roads** (items of class `SKeyRoadElement`; Studio requires item class id `0x1A`) |

A road type definition has `name`, and lists `straight` (`0x78`), `curve` (`0x79`), `special` (`0x7A`, inferred
from the gap in the numbering, not confirmed), `terminator` (`0x7B`). Each item has `objectfilefilename` (`0x7C`,
the model path) and `name`. The **index of an item in its list** is how roads refer to it.

A crossroad definition has `name`, `objectfilefilename`, and `connecta`, `connectb`, `connectc`, `connectd`. Each of
those is the index of a road type, or −1 for none, for the end, start, left and right connection.

#### A road record (`SKeyRoadElement`)

| Name | Tag | Type | Meaning |
|---|---|---|---|
| `orientation` | `0x8C` | f64 | key part rotation, degrees, **clockwise** (Builder writes it wrapped to (−180, 180]) |
| `elevation` | `0x8D` | f64 | written as 0 |
| `mposition` | `0x8E` | f64 array (2) | world X, Y of the key part's **model origin** |
| `roadid` | `0x8F` | i32 | index of the road type in the definitions |
| `keypointtype` | `0x90` | i32 | element code of the key part (see below) |
| `keypointname` | `0x91` | string | model path of the key part |
| `directiona` | `0x92` | list | chain from the key's **end** edge (`LE`/`PE`) |
| `directionb` | `0x93` | list | chain from the key's **start** edge (`LB`/`PB`) |
| `directionc` | `0x94` | list | chain from the **left arm** (`LD`/`LH`) |
| `directiond` | `0x95` | list | chain from the **right arm** (`PD`/`PH`) |

Studio's test data also has an unknown 1-byte field `0xAA`. Studio keeps unknown fields by copying an existing
road as a template.

A chain element (`SRoadElement`; Studio's test data uses class `0x1B`) has `type` (`0x7F`, i32), `id` (`0x6C`, i32:
the part's index in its road type's list) and `name` (`0x33`, string: the model path).

**Element codes**: 2 = crossroad (key only), 3 = straight, 6 = terminator (end cap), 8 = curve entered through its
start edge, 7 = curve entered through its **end** edge (a reversed curve; turns the other way). Studio's checks say
that code 4 (the curve list) never appears in a record. Straights are always 3, and terminators always 6,
whichever way they are entered.

**Chains** are independent arms from the key, not consecutive sections. Each chain runs away from the key. Each
element attaches its entry edge to the previous element's exit (or to the key's port), with the outward normals
opposite.

- Builder: a part that comes *after* the key in driving order is entered at its end if the part was placed
  reversed. A part *before* the key (listed going away from the key) is entered at its end if it was **not**
  reversed. If the key itself is reversed, "after" goes to `directionb` and "before" to `directiona`.
- A crossroad key's `roadid` is its `connecta` type, or `connectb` if that is −1, or 0.

**Ids.** Every object has a u32 id. Builder gives new objects ids from the largest id in the file + 128, in steps
of 128. Studio's editor gives new ids from the largest id found + 1, in steps of 1. Its merge gives new roads ids
from the largest road id + 992, in steps of 992, and new elements ids in steps of 128. In all cases, ids already in
use are skipped. Studio finds the used ids by scanning the whole file for the `06 00 0D` item pattern. Whether
Terrain Builder needs the steps of 128 is not determined.

#### What must already exist in the project

- The road types and crossroads used, defined in the Road Tool. Builder refuses to write and lists every missing
  part. It also refuses parts defined as "special".
- The tag and class names. Builder asks the user to use the Road Tool once and save the project, so that the names
  exist.
- A road list (`mroadGetRoads`). Builder asks the user to draw one road.
- Studio additionally needs an existing road of the same road type to copy as a template, and at least one chain
  element anywhere to copy.

#### How each tool writes

- **Builder (adds new roads)**: every placed junction becomes the key of its own road. The through road continues
  in `directiona`/`directionb` and the side roads in `directionc`/`directiond`. Every other road gets its first
  straight (or crosswalk, or its first part) as the key. A segment that cannot be attached to a free chain of its
  junction becomes its own road. The new items are spliced at the end of the road list. Then the list's byte length
  and count are raised, and so is the byte length of **every enclosing list and object** up to the root.
- **Studio editor (adds, replaces, moves, deletes)**: it rebuilds the road list. Unchanged records are kept byte
  for byte. Moved records get new rotation and position fields. A new road is one record per stretch between
  bridges (4.3): the first part is the key and must not be reversed, all other parts go into `directiona`, and the
  other three chains are empty. Parts of different road types in one road are refused. Because the list's size
  changes, the byte lengths of **two enclosing objects** are adjusted, the ones with tags `0x3F` and `0x18` (type
  `0x0D`). Each must occur exactly once outside the road list, or the write is refused. This was a fix: ids inside
  road data can contain the same byte pattern.
- **Studio merge (between projects)**: A is the base and B the donor. The road type and crossroad definitions must
  be identical, compared byte by byte with every object id set to 0. A road from B is skipped if a road with the
  same normalised bytes already exists. The others are added with new ids. All non-road content comes from A.

#### Caveats both tools state

- Keep backups. The output must be a different file from every input.
- Builder parses its output again and checks the road count. It also rebuilds the geometry from the written file
  and compares it with its object export (error below 1 mm).
- Studio checks that the rebuilt road list is byte-identical. It checks that every new record refers to a defined
  road type, category, index and path. Paths are compared case-insensitively, without a drive letter, with `/` read
  as `\`. It refuses roads whose parts do not connect (1 mm, 0.001°).
- Open the result in Terrain Builder and check it before using it.

**Not verified.** None of this has been confirmed on Náves's own files. It is a record of what the two tools say
and do.

**What Náves has today.** Nothing for `.tv4p`, and nothing is planned.

---

## Ideas and the size of the work in Náves

| Idea | Size | Depends on |
|---|---|---|
| Builder's cost terms in the piece search (integrated squared deviation, heading², per piece, per degree) as a check against `roadfit.rs` | small | nothing |
| Anti-weaving rule (no curve that does not reduce heading error while already near the line) | small | `roadfit.rs` |
| Fillet pre-rounding of sharp OSM corners at a minimum radius | small | nothing |
| Splitting crossing lines without shared vertices; removing overshoots | small | network clean-up (`netclean`) |
| Builder's through-road choice (straightness + same source line, longer line on ties) compared with Náves's rank rule | small | `roadnet.rs` |
| A side road closing onto a second junction arm with a higher end weight and an end heading | medium | `roadplace.rs` closure |
| Obstacle raster with chamfer distance map (water, rivers, forbidden areas) | medium | DEM, OSM water |
| Profile band (reachable heights with cut/fill) as a feasibility test per step | small | DEM sampling |
| Coarse cost-to-go field with the climb / grade lower bound | medium | obstacle raster, profile band |
| Hybrid A* arc search with exact arrival, legs that carry several headings, and weighted retries | large | cost field, profile band |
| Fitting a routed line with search margins (radius × 1.05, 90 % grade first) | small | arc search, `roadfit.rs` |
| Automatic bridges inside the route search (span test, supports' reach, penalty) | medium | arc search, bridge catalogue |
| Two-pass band profile with smoothing inside the band, as a fallback or check for the QP | small | `profile.rs` |
| Side slopes and smoothstep blend beyond the footprint; nearest-profile-segment heights | small | `conform.rs` |
| Layered, cached terrain adaptations per route | medium | only useful for an interactive editor (GUI editing) |
| Railway pieces from ODOL: routable-name rule and the rail-head centre-line fit | medium | `naves-formats` ODOL reader (first LOD vertices) |
| Track search with pieces as steps | large | arc search, rail catalogue |
| Laying rigid track pieces with pitch and roll, limited roll between neighbours | medium | rail catalogue, heightmap |
| Level crossings (60° minimum, span rule, kits by family) | medium | track search, road network |
| Merging straight runs into fewest pieces (coin change) for tracks | small | rail catalogue |
| Terrain Builder object import text (7a) | small | only for interchange; not needed for Náves's own builds |
| `.tv4p` Road Tool format (7b) | notes — not to be implemented | — |

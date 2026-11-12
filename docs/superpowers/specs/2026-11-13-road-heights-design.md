<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Náves: road heights, junctions, earthworks and bridges

Date: 2026-11-13. Status: draft for approval. It changes how the roads level decides the height of a road and what
it does to the terrain, on top of `docs/superpowers/plans/2026-11-06-naves-roads.md` and
`2026-11-10-naves-road-alignment.md`. The measurements behind it are in section 8 and in
`docs/research/2026-11-10-road-alignment.md`, section 6. Decisions marked **(maintainer)** are the maintainer's;
those marked **(my call)** are the author's defaults and can be overruled.

## 1. Why

A full build of the Berchtesgaden example reports 8133 road violations. Measuring them showed that they are not a
matter of terrain resolution:

- Roads are solved one after another, and a road that ends on another is tied to wherever that road happens to be.
  At 44 % of the 1269 junctions the planned height is more than 2 m from the ground, at 54 of them more than 10 m,
  and every road tied there inherits that. Where a road cannot make up the difference within its grade limit, the
  whole difference lands in one piece: steps of 235 % and 294 %.
- Forest tracks are held to a car's grade limit and are dug up to 30 m into the hillside to keep it.
- A road that runs close to another at a different height loses the ground under it to the more important road:
  88 % of the pieces that miss the terrain by more than 0.5 m.
- A road crossing a dip stands on an embankment that runs 20 to 40 m down the slope.

What the maintainer wants to see when driving a mountain road: the road lies on a bench cut into the hillside, with a
rock face on the uphill side where the ground is steep; tracks are as steep as tracks are; roads meet at one height;
a dip is crossed by a bridge.

## 2. Goals, non-goals, anti-goals

**Goals**

1. No steps: where roads meet, they meet at one height, and no piece carries a height difference its neighbours
   could have shared.
2. A road stays as close to the ground as its grade and bend limits allow; where it cannot, it is cut in or raised
   within the allowance of section 5, and reported beyond it.
3. Tracks get limits that fit tracks (section 3).
4. Dips are crossed by bridges, OSM bridges are bridges in the game (section 6).
5. Roads close together at different heights each keep their own ground (section 5.3).
6. Same input, same output, as everywhere in Náves.

**Non-goals** (not in this design)

- Moving a road sideways off its OSM line. Measured: at the new limits it improves 25 of 455 track chains by more
  than 1 m, and the elevation model (one height per 15 m of map) cannot resolve a 5 to 10 m shift.
- The plan-view faults of placing: 332 stretch ends that miss what they meet, 737 crossings without a junction
  piece. They do not depend on heights and need their own design.
- Rocks with an opening over a stream (section 6.4), wall, gallery or tunnel objects.
- A finer elevation source than the 30 m model.
- Measuring vehicle limits in the game. All limits stay marked as estimates.

**Anti-goals** (a failure even if everything else works)

- A step in a road, anywhere.
- A deep cut or a high embankment on flat ground without a report.
- A road that is steeper than its class allows, without a report.
- Losing a manual edit when roads are regenerated (unchanged from the main design).

## 3. Road classes and grade limits

Three values per family, as today: the aim the profile tries to hold, the ceiling it may go to when holding the aim
would take it off the ground (0.9 of the hard limit), and the hard limit above which the check reports the road as
not drivable.

| Family | From OSM | Aim | Ceiling | Hard limit | Meant for |
|---|---|---|---|---|---|
| `asf1`, `asf2`, `asf3`, `city` | public roads | unchanged | 0.27 | 0.30 | a car |
| `grav` | `highway=track`, grade 1 and 2 or a firm surface | 0.18 | 0.36 | 0.40 | a car, at the top of its range |
| `mud` | `highway=track`, grade 3 and worse or unknown | 0.25 | 0.45 | 0.50 | a 4WD |

**(maintainer)** Tracks in the mountains are not expected to be drivable by car; a passenger car manages 30 to 40 %,
a 4WD 45 to 60 %, and those are acceptable values for bad gravel roads. **(my call)** the exact values inside those
ranges. The measurement (section 8.3) shows that raising `grav` from 0.30 to 0.40 gains little (cuts deeper than 5 m
on 12.1 % of the length instead of 13.2 %) and makes 4 % of the gravel length steeper than 30 %; keeping `grav` at
0.30 is a defensible alternative. Raising the `mud` aim was measured and is not worth it.

The classification of tracks into `grav` and `mud`, and the `drivable` flag, stay as they are. The cross-fall limit
and the smallest vertical radius stay as they are.

## 4. Junction heights

**(maintainer)** Every junction gets one height taken from the ground, all roads meeting there are tied to it, and
the terrain is changed to fit.

### 4.1 What a junction is

A place where roads meet, of any of the three kinds the build has today: a junction piece (478 on the example), a
join without a junction piece (681), and a chain end that lies on another road (110). Junctions whose points are
3 m or less apart along a road are one junction and share one height **(my call)**; measured, that merges 143
junctions into 65 groups.

### 4.2 The height

1. **Start:** the median of the original ground (before any road shapes it) on a 1 m grid inside a circle of 10 m
   around the junction point **(my call: median, 10 m)**. Measured: median, mean and the height at the point agree
   within 0.3 m at 90 % of the junctions, and the radius changes the height by 0.27 m at p90.
2. **Repair:** a link is the road between two neighbouring junctions. Where the grade needed between the two
   heights over the length of the link (junction edge to junction edge, at least 1 m) exceeds the link's ceiling,
   both junctions move toward each other by just enough. The move is shared by importance: the junction on the more
   important road moves less (the rank the profile solver already uses: `asf1` > `asf2` > `city` > `asf3` > `grav` >
   `mud`, by the most important road at the junction; each step of rank halves the share) **(my call)**. A fixed number of rounds in a fixed order; 50 rounds converge on
   the example and 500 give the same result. A link still over its ceiling after the rounds is a violation.
   Measured at the limits of section 3: 72 of 1535 links are over, 69 of them shorter than 15 m; 130 junctions
   move, 13 by more than 1 m, the largest by 2.83 m.
3. **Edits:** a junction height is derived, not stored. (A user override of a junction height is not part of this
   design.)

### 4.3 Roads and junctions

- Every road through or ending at a junction has that height there, **the through road included**. A through chain
  is then a sequence of links between fixed heights, and the result no longer depends on the order in which chains
  are solved.
- A junction piece is level along its own length at the junction height, as today; the pieces next to it keep the
  gentle meeting grade the alignment plan introduced. Where that meeting grade holds a road far off the ground
  (measured at the lower end of the Klingerweg: 9 m of cut), the builder may shorten or drop it for `grav` and
  `mud` **(my call)**.
- Free ends (dead ends, the map edge) are not tied.
- The terrain under a junction is levelled to its height. The 10 m circle is only where the height is sampled.
  **(my call)** the level patch is the footprint of the junction piece, or, at a join without one, a circle of half
  the width of the widest road meeting there plus one cell; around it the ordinary cut and fill of section 5 apply,
  and where the patch comes close to another road at a different height, section 5.3 applies to it as to a road.
  Measured: the ground inside the 10 m circle varies by 3.2 m at the median, by more than 10 m at 54 junctions.
- A junction at the end of a bridge takes its height from the ground at that abutment like any other. A junction on
  a bridge span has the deck's height there and no level patch **(my call)**.

## 5. Cut, fill and the ground beside the road

### 5.1 How far a road may leave the ground

**(maintainer)** A road below the ground is a cutting and is fine; so is raised ground. On flat ground up to 30 m of
either is acceptable; on the side of a steep mountain up to 100 m. A deep cut or a high embankment on flat ground
means something is off.

This is an **allowance that is reported when exceeded**, not a constraint on the solver and not a gate: the profile
still tries to stay near the ground (goal 2). **(my call)** the allowance by the side slope of the original ground:
30 m up to a slope of 20 %, rising in a straight line to 100 m at 60 % and above. A piece beyond its allowance gets
a new violation (`road_far_from_ground`, with the depth or height and the allowance).

**The side slope**, here and in 5.2 **(my call)**: at the middle of a piece, the absolute height difference of the
original ground between the points 10 m to the left and 10 m to the right of the centre line, over 20 m; then the
median of that value over the pieces within 25 m along the road, so that a single noisy sample of the elevation
model does not switch a rule on and off from piece to piece.

The ladder in the profile solver that gives up the grade aim when a road is "off the ground" (`RELAX_OFF_M`) stays;
its threshold is the builder's to tune with the measuring tool.

### 5.2 Faces and slopes

**(maintainer)** On steep ground the uphill side of a road may be a near-vertical rock face that nobody can walk
up; no wall or gallery objects, the mountain does it.

- Cut: on a hillside the face may be as steep as the grid allows (one cell wide). The rule that a player must be
  able to walk off the road on the uphill side no longer holds on steep ground. **(my call)** "steep" here is a
  side slope of the original ground above 35 %; below that the cut slopes stay as they are.
- Fill: slopes stay as they are. An embankment that would not reach the ground within the reach of the earthworks
  is what section 6 replaces by a bridge where it can; what is left is reported as today (`earthworks_capped`).
- The reach of the earthworks (60 m today) is the builder's to adjust with the faces in place.

### 5.3 Roads close together

Each road keeps the ground under its own width. Today the more important road holds a level strip of half its width
plus 1.5 cells and takes the vertices under a neighbour. **(my call)** where two roads (or two legs of one road)
at different heights come closer than that, the strip of each ends halfway between their edges and the height
difference becomes a face between them, as steep as the grid allows. Where their edges overlap, it remains a
conflict and is reported as today. This is where the finer grid of section 7 matters: the face is one cell wide.

## 6. Dips and bridges

**(maintainer)** Where OSM has a bridge, the game has a bridge. A road that crosses a dip without an OSM bridge gets
a bridge when one fits, or a chain of bridges over a long dip. A dry dip that no bridge fits is filled. Over water:
rocks with an opening, or the road is left in the air and marked for manual inspection.

### 6.1 Which bridge

- By surface: the wooden bridges for `grav` and `mud`, the paved ones (stone, metal) for the asphalt families and
  `city`. Among those, the narrowest model that is at least as wide as the road; today paved roads are offered the
  9 m stone bridge only, although the catalog has a 12 m one.
- **(maintainer)** A bridge that is not wide enough is scaled up. The scale is uniform, so the bridge also becomes
  longer and taller by the same factor; length and deck height come from the scaled model. **(my call)** at most
  1.5; beyond that the widest model is used at 1.5 and the build warns, as it does today.
- Models without connection points (the 50 m wooden bridges) stay out, as today.
- The world file's transform is a general 3×3 matrix and a position (`WorldObject::transform`), so a scale needs no
  format change: the placed object gets a scale and the rotation columns are multiplied by it. The catalog's
  `BridgeModel` has length, width, deck height and both ports; all of them scale by the same factor.
- Whether the game drives and collides correctly on a scaled bridge is **open until tested in the game**
  (section 9). **(my call)** if it does not, the fallback is today's behaviour: the widest model at scale 1 and a
  warning. The scaling therefore sits in one place where it can be switched off.

### 6.2 OSM bridges

An OSM bridge is built as a bridge: one model, or a chain of models end to end, covering at least its length, centred
on it **(my call)**. Today a bridge shorter than a model is laid as ordinary road; that ends. A bridge's deck takes
its height from the ground at its two ends (already so after the review fixes), and the ground under it is left
alone.

### 6.3 Dips without an OSM bridge

A dip is a run where the road, solved within its limits, would stand more than 3 m above the original ground
**(my call)**.

1. If one bridge model or a chain of them covers the run: bridge. A bridge fits when the run is at least half a
   model long, the models can be laid end to end over it without reaching a junction or the end of the road, and
   the road stays within 2 m of the straight line of each model **(my call)**.
2. Otherwise, if no water lies under the run: fill, as today.
3. Otherwise: the road is left as solved and reported for inspection (`road_over_water`, a new violation).

Bridges over dips are found after the heights are solved, so the order is fixed **(my call)**: solve the links;
find the dips; turn each into a bridge run whose deck takes its height from the ground at its two ends; solve the
links that got a bridge once more with the bridge ends fixed; no further rounds. A dip never contains a junction,
because junctions lie on the ground by construction. OSM bridges are known before the first solve.

A generated bridge is an object like any other: it can be moved, replaced or deleted in `edits/overrides.toml`,
with the fields an edit has today. **(my call)** its identity is the stretch it stands on and the number of the
bridge run on that stretch, counted from the stretch's start, so it does not depend on the grid or on small changes
of the heights. When a dip appears or disappears after the input changed, an edit on a bridge, or on the pieces of that
stretch from the bridge onward, becomes an orphan and is kept and reported as orphans are today; the pieces before
the first bridge of a stretch keep their identity. Existing overrides need no migration:
object identity comes from the placing, which does not depend on the grid or the heights (measured: identical at
1024, 2048 and 4096).

### 6.4 Water

The build knows lakes and reservoirs and nothing about streams. It starts reading `waterway=stream`, `river` and
`canal` lines from the OSM sources the project already names, for this test only; no new setting in the project
file. `ditch` and `drain` are left out **(my call)**: they are everywhere and would turn most dry dips into
reports. Water lies under a run when such a line crosses the road's footprint within the run, or a known lake
overlaps it. Rocks with an opening are a later step: they need rock models from the catalog and a look in the game.

## 7. The grid

The example moves from `terrain_size = 1024` to 2048 (3.125 m cells). Measured: a finer grid does not remove
violations by itself, but conflicts fall by a quarter, embankments get shorter, and a face is 3 m wide instead of
6 m. 4096 gains nothing more on these figures. **(maintainer)** A PBO of a few GB is fine. The build with binarize
and the PBO size at 2048 with roads have not been measured yet; that is part of the work. **(maintainer)** 2048 also
becomes the default `terrain_size` of a new project; existing project files keep the value they name. The manual
and the changelog say so. **(my call)** if the 2048 build does not get through binarize, the example and the default
stay at 1024 and the report says why; nothing else in this design depends on the grid.

## 8. What was measured

All on the Berchtesgaden example with `crates/naves-export/examples/road-analysis.rs`; raw outputs under
`/var/tmp/naves-grids.*`, `naves-shift.*`, `naves-junctions.*` and `naves-limits.*` (not kept forever: the figures
that matter are here and in the research note).

1. **Grids** (research note, section 6): violation counts at 1024, 2048, 4096; causes of the misses; cut against
   fill by side slope.
2. **Junctions:** 1269 junctions, 1535 links between them (138 km), 787 links to a free end (62 km). With both
   ends of a link on the repaired ground medians, the road has to leave the ground by 0.0 m at the median and 0.9 m
   at p90, and 90 % of the length is within 2 m; with today's junction heights 0.7 m, 6.3 m and 40 %. Not checked
   there: the vertical radius, cross fall where a side road meets a sloping through road, roads close together
   that are not junctions.
3. **Limits alone**, junctions as today (2048): `mud` length cut deeper than 10 m falls from 6.5 % to 2.2 %; the
   Klingerweg's 294 % step and 30 m cut are gone; `mud` length steeper than 30 % rises from 1.0 % to 7.1 %;
   `road_misses_terrain` rises by 6 % (small misses at piece joints); cross fall over 10 % from 5.4 to 6.0 km. The
   limits alone do not put the tracks on the ground (median distance still 2.4 m): about half of the remaining deep
   cuts sit at an end tied to another road, and the worst interior ones are held by the vertical-radius limit
   across a crest or hollow.
4. **Sideways shift:** see non-goals.

## 9. Order of work, and what stays open

Each step is measured with `road-analysis` before and after, at 1024 and 2048, and the figures go into the research
note. A step that makes a figure worse says so.

**What counts as done (my call).** The tests are structural, on synthetic terrain: roads meeting at a junction have
one height; a link between two junctions too close for their ground heights is brought within its ceiling; a road
next to a higher road keeps its ground; a dip gets a bridge, a chain, fill or a report by the rules of 6.3. On the
example, after step 2: no piece with a planned grade above its hard limit that is not reported, no grade above
100 % at all, and every junction within 3 m of its ground median. The other figures of section 8 are compared,
not gated. Violations never stop a build, as today; a link that the repair cannot bring within its ceiling is a
violation.

1. Grade limits (section 3).
2. Junction heights (section 4). The largest step; it changes how chains are solved.
3. Allowance, faces and roads close together (section 5).
4. Bridges and dips (section 6), with the world-file scale and the waterway lines.
5. The example at 2048, a full build through binarize, the manual and the changelog.

**Open until someone can supervise a game run:** whether binarize and the game place the road objects correctly
without the models below `P:` (binarize logs "Cannot open object" for all 102 road models and finishes); whether a
scaled bridge drives and collides; whether the game's vehicles manage 36 % and 45 %; how a near-vertical face next
to a road looks and behaves. Until then the manual says that roads have not been tested in the game.

**Left to the builder:** the data structures, how the profile solver takes interior fixed points, the tuning of
the meeting grade, the earthworks reach and the ladder threshold, the names of the new violations, and the tests.
`AGENTS.md` applies: test first, deterministic, documentation and changelog with each change.

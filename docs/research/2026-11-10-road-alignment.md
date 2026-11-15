<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Road alignment: what is wrong, why, and what the terrain can do about it

Measured on 2026-11-10 on the Berchtesgaden example (`examples/berchtesgaden`, a 6.4 km map at half scale,
202.6 km of drivable road in 19 973 checked pieces) at commit `4a5faed`. The tool is
`crates/naves-export/examples/road-analysis.rs` (optional argument: the terrain size; about 3 minutes per run). The
plan that follows from this note is `docs/superpowers/plans/2026-11-10-naves-road-alignment.md`.

**Limits of this note.** One map, one scale. The vehicle limits in section 3 come from the physics of real cars, on
the maintainer's assumption that DayZ's cars behave like them. They are **not measured in the game**: Task 1 of the
roads plan (drive ramps) is still open, and its result replaces these numbers when it exists.

## 1. Numbers

At `terrain_size = 1024` (6.25 m cells) unless a column says otherwise.

| | 1024 | 2048 | 4096 |
|---|---|---|---|
| Road that cannot be driven | 27.5 km | 21.4 km | 19.4 km |
| of that: cross fall alone | 25.6 km | 20.2 km | 18.4 km |
| of that: grade, alone or with cross fall | 1.9 km | 1.3 km | 1.0 km |
| Cross fall over 5 % | 27.4 km | 21.3 km | 19.2 km |
| Cross fall over 10 % | 12.9 km | 9.3 km | 8.0 km |
| Cross fall over 20 % | 5.8 km | 4.7 km | 3.9 km |
| Cross fall over 30 % | 3.8 km | 3.0 km | 2.4 km |
| Grade over 35 % (on the unshaped terrain) | 1.4 km (6.4 km) | 1.0 km (6.7 km) | 0.8 km (6.8 km) |
| Pieces more than 0.15 m off the terrain (unshaped) | 3786 (12 293) | 3234 (12 143) | 3613 (12 547) |
| Pieces more than 1 m off the terrain | 469 | 425 | 409 |
| Road conflicts | 2412 | 2166 | 1984 |
| Earthworks that stop at the 60 m cap (vertices) | 1904 | 3459 | 6363 |
| Map vertices changed by the earthworks | 18.5 % | 14.4 % | 12.3 % |

In plan view the chains are clean: no heading kink between consecutive pieces of a chain (19 604 joins, the largest
0.0°). 1150 joins have a distance of up to 6.25 m between one piece's end and the next one's start: straights that
overlap to make a length, as the fit intends. **Not clean:** 332 stretch ends miss what they have to meet (median
2.7 m, p90 9.7 m, worst 105.9 m), 737 crossings have no junction piece, 21 tunnels have no pieces. The 332 gaps are
**not analysed here** and are not part of the plan.

## 2. Causes

### 2.1 The cross-fall limit is a design value used as a hard limit

`MAX_CROSS_FALL = 0.05` makes a piece `NotDrivable`. 25.6 of the 27.5 km that cannot be driven are over this limit and
nothing else. A real road is built with up to 7 or 8 % across in a bend, and a car drives across far more (section 3).
With a hard limit of 20 % the figure is 5.8 km.

### 2.2 Where one road ends on another

Pieces with a cross fall over 5 % or more than 0.15 m off the terrain, by what is next to them (edge to edge, within
three cells):

| What is next to the piece | Pieces | Length | of that with cross fall over 5 % |
|---|---|---|---|
| Another road overlaps it, at a different height | 761 | 8.8 km | 5.3 km |
| Another road overlaps it, at the same height where they are closest | 887 | 10.0 km | 5.0 km |
| Another road within three cells, at a different height | 1519 | 16.2 km | 11.6 km |
| Another road within three cells, at the same height | 150 | 1.6 km | 0.8 km |
| Another part of the same road (a hairpin), at a different height | 236 | 2.3 km | 1.6 km |
| Nothing: an arc of radius 10 m or 25 m | 543 | 4.9 km | 4.1 km |
| Nothing: other | 151 | 2.0 km | 1.3 km |

Of the 22.9 km with a cross fall over 5 % next to another road, **19.4 km lie within 30 m of an end of one of the two
roads**: a meeting. Roads that run side by side account for 3.0 km, roads that cross or pass for 0.5 km.

The mechanism, from the code:

- `profile::end_joins` fixes **one** variable of a side road, its end, to the height of the road it meets.
- `conform::claim_cores` gives every vertex within half a width and 1.5 cells of the more important road that road's
  height. At 6.25 m cells that is 13 to 15 m from its centre line. The side road's first pieces therefore lie on the
  through road's surface, which falls along the through road, that is, across the side road. A through road at 10 %
  gives the side road 10 % across.
- From its second variable on, the side road's profile follows its own terrain. Over those 13 to 15 m it leaves the
  surface it actually lies on, and where the two cores meet there is a step. This is the "different height" row.

A finer grid shortens the zone (the "within three cells, different height" row goes 16.2, 7.7, 3.6 km at 1024, 2048,
4096) but the overlap rows stay at about 19 km: the grid is not the cause.

109.1 of the 202.6 km have another road within three cells of their edge, 72.9 km at a different height. At half scale
with full-size road pieces, roads are twice as close together as in the world.

### 2.3 Level junction pieces break the grade limit, unreported

`profile::chain_of` gives a junction piece one height along its 12.5 m. The road arrives at its grade, is level, and
leaves at its grade. The solver sees the junction as one variable at its centre and limits the grade to that point;
the piece before the junction then covers the same height over a shorter length.

- 496 junction pieces. 306 pieces next to one are planned steeper than their family's aim.
- Grade change into the level piece: 0.66 %/m at the median, 4.0 %/m at p90 (a crest radius of 25 m), 51 %/m at worst.
- `profile::excesses_of` works on the variables, not on the joints, so none of this is a `GradeExcess`.

### 2.4 The limit on the change of grade is per joint

`max_grade_change` is the same for a 4.36 m piece (`_10 25`) and a 25 m piece: 0.02 on `asf1` is a crest radius of
218 m on the one and 1250 m on the other. Planned profile, change of grade per metre:

| Family | p50 | p95 | p99 | Joints over 1 %/m |
|---|---|---|---|---|
| `asf1` | 0.09 | 1.73 | 7.75 | 61 of 847 |
| `asf2` | 0.04 | 0.63 | 1.52 | 18 of 684 |
| `asf3` | 0.08 | 0.89 | 6.31 | 102 of 2325 |
| `city` | 0.17 | 1.48 | 7.81 | 579 of 7052 |
| `grav` | 0.10 | 0.70 | 1.15 | 77 of 4555 |
| `mud` | 0.26 | 1.55 | 4.04 | 473 of 4141 |

1 %/m is a radius of 100 m; a car leaves the ground over a crest of radius `v² / g`, 79 m at 100 km/h.

### 2.5 Tight arcs on a grade

543 pieces with no other road near them are arcs of radius 10 m or 25 m. The profile, the earthworks and the check all
treat an arc as its chord. A `_60 10` piece is 1.34 m off its chord in the middle, and the check measures the fall at
right angles to the chord, 30° off the radius at the piece's ends, so it reads part of the grade as cross fall. Behind
that is a real effect: on the inside of a bend of radius `r` a road of width `w` is steeper than its centre line by
`r / (r - w/2)`, 1.5 times for `asf3` on a 10 m arc.

### 2.6 Why the profile leaves the terrain

At the piece centres the profile is 2.4 m from the unshaped terrain at the median, 7.4 m at p90, 24 m at p99 and 47 m
at worst. 4755 joints are more than 5 m off the ground:

| | Joints |
|---|---|
| The profile is at the grade aim of its family there | 2889 |
| The ground is rough (over 2.5 m off a line through the ground 50 m either side) | 228 |
| Both | 285 |
| Neither | 1353 |

Two thirds are places where the road is held to its aim while the ground is steeper. The DEM's roughness under the
roads is small (p90 1.85 m, p99 4.0 m). Over 200 m windows the ground under the roads is steeper than the family's aim
on 10.3 of 112.9 km, so much of this is short steep ground, not long climbs. The "neither" row was not looked into.

The earthworks do not depend on how steep the hillside is: the share of pieces with a cross fall or a miss is 10 to
16 % in every band of side slope from 0 to 100 %.

### 2.7 Embankments that never reach the ground

`FILL_SLOPE = 0.667` (34°). On a hillside of 34° or more an embankment never meets the ground and runs to
`MAX_EARTHWORK_M`. The median slope of the ground at the 1904 capped vertices is 34° (p10 24°, p90 48°).

### 2.8 Hairpins

3301 pieces have another part of their own road within 60 m. For 2040 of them the two legs differ by more in height
than the terrain rules allow (level for three cells between their edges, then 34°): by 3.7 m at the median, 9.6 m at
p90. At 2048 it is 1267 pieces, at 4096 it is 799. `profile::settle` then pulls the legs together by up to 5 m each,
which breaks the grades it does not report as broken (2.3). At half scale a real hairpin of radius 12 m is 6 m on the
map; the tightest piece has 10 m.

## 3. What a real car manages

For the maintainer's assumption that the game's cars are modelled on real ones. A car of wheelbase `L = 2.6 m`, centre
of gravity `h = 0.55 m` high and `0.4 L` behind the front axle, tyre grip `μ`. The steepest grade it climbs is
`μ·0.6L / (L + μh)` with front-wheel drive and `μ·0.4L / (L − μh)` with rear-wheel drive (weight moves to the rear on
a climb); with all-wheel drive it is `μ`.

| Surface | `μ` | Front | Rear | Worse of the two × 0.8 | Proposed hard limit | Now |
|---|---|---|---|---|---|---|
| Asphalt, dry | 0.8 | 0.41 | 0.39 | 0.31 | **0.30** | 0.35 |
| Gravel | 0.6 | 0.32 | 0.28 | 0.22 | **0.22** | 0.30 |
| Dirt track | 0.45 | 0.25 | 0.20 | 0.16 | **0.18** | 0.28 |

The factor 0.8 is the margin the roads plan uses for the measured value. The track value is rounded up: the vehicles
that use tracks in the game are mostly all-wheel drive.

**The aims per family** have to stay below the hard limits; `mud` (0.25) and `grav` (0.18) do not after this change.
Forest roads are built at 8 to 12 %, 15 to 18 % over short lengths. Proposed: `asf1` 0.08, `asf2` 0.10, `asf3` 0.12,
`city` 0.14, `grav` 0.15, `mud` 0.15. Section 2.6 shows that a low aim costs earth, so an aim must be allowed to give
way, up to 0.9 of the hard limit, where holding it puts the road far from the ground.

**Decided by the maintainer:** gravel and dirt keep their values, hard limits 0.30 and 0.28 and aims 0.18 and 0.25. The
proposal above is what a car manages, and not every gravel or dirt road in such an area can be driven by car: some
need chains or a tractor. The proposed limits and aims are used for asphalt and streets only.

**Decided by the maintainer (2026-11-13):** tracks in the mountains are not expected to be drivable by car; a
passenger car manages 30 to 40 %, a 4WD 45 to 60 %. Gravel tracks therefore get a hard limit of 40 % and dirt
tracks 50 % (ceilings 36 % and 45 %), replacing the 0.30 and 0.28 of the paragraph above; the aims stay 0.18 and
0.25. Estimates, not driven in the game.

**Across.** Roads are built with 2.5 % across on a straight and up to 7 or 8 % in a bend. A car slides sideways at a
fall of `μ` and tips at half its track width over `h` (above 1.0 for a car, 0.6 to 0.8 for a loaded truck). Proposed:
aim 0.08, hard limit 0.20.

**Crests and dips.** What matters is the radius, not the change per piece. A car leaves the ground at `v²/g`: 20 m at
50 km/h, 79 m at 100 km/h. Proposed smallest vertical radius: `asf1` 600 m, `asf2` 400 m, `asf3` 250 m, `city` 150 m,
`grav` 100 m, `mud` 60 m. (Design standards ask for far more, for sight distance; that is not needed here.)

**Bends.** A car turns in a radius of about 5.5 m. The 10 m arc is enough on every family; no change.

## 4. Experiments with the terrain

Each with one constant changed in `conform.rs` and the same run, 1024 grid.

| | Capped vertices | Conflicts | Vertices moved over 5 m | Cross fall over 5 % | Pieces over 0.15 m off |
|---|---|---|---|---|---|
| As it is | 1904 | 2412 | 65 518 | 27.4 km | 3786 |
| Embankments at 45° (`FILL_SLOPE = 1.0`) | 508 | 1947 | 55 343 | 27.2 km | 3777 |
| and cuttings at 63° (`CUT_SLOPE = 2.0`) | 47 | 1738 | 44 196 | 27.2 km | 3777 |
| Level strip 0.75 cells beyond the road's edge instead of 1.5 | 1563 | 1592 | 43 715 | 84.9 km | 14 418 |

- **Steeper earthworks work**, and cost the roads nothing. They are only needed where the hillside is steep, so the
  slope should follow the natural ground and not be steeper everywhere. A 63° cutting is above `WALK_LIMIT_DEG` (50°);
  the code asks for cuttings below it so that a player can leave the road. Where the hillside itself is near that
  limit, only the embankment side can stay walkable.
- **The level strip cannot be narrower in cells.** It can only be narrower in metres, with a finer grid: a 5 m track
  gets a level bench 23.75 m wide at 1024, 14.4 m at 2048, 9.7 m at 4096.
- **A finer grid** helps hairpins most (2.8) and costs build time and size (16 s and 75 MB at 1024, 38 s and 220 MB
  at 2048, see the example's README).

Not tried: hairpins widened to the 25 m arcs (legs 50 m apart), a wall between the legs as an object, the profile
taking its height from the downhill edge on steep ground (a road wholly in cutting).

## 5. After the fixes

Measured again on 2026-11-11 with the plan's fixes (Tasks 2 to 9), same map and tool, at `terrain_size = 1024` and
`2048`. The limits are still estimates from real vehicles, not measured in the game.

| | 1024 before | 1024 after | 2048 before | 2048 after |
|---|---|---|---|---|
| Road that cannot be driven | 27.5 km | 5.7 km | 21.4 km | 4.8 km |
| of that: cross fall alone | 25.6 km | 3.2 km | 20.2 km | 2.5 km |
| of that: grade, alone or with cross fall | 1.9 km | 2.5 km | 1.3 km | 2.3 km |
| Cross fall over 5 % | 27.4 km | 19.1 km | 21.3 km | 15.9 km |
| Cross fall over 10 % | 12.9 km | 7.5 km | 9.3 km | 5.5 km |
| Cross fall over 20 % | 5.8 km | 4.3 km | 4.7 km | 3.4 km |
| Cross fall over 30 % | 3.8 km | 3.2 km | 3.0 km | 2.7 km |
| Grade over 35 % (on the unshaped terrain) | 1.4 km (6.4 km) | 1.7 km (6.1 km) | 1.0 km (6.7 km) | 1.5 km (6.4 km) |
| Pieces more than 0.15 m off the terrain (unshaped) | 3786 (12 293) | 3522 (12 293) | 3234 (12 143) | 3303 (12 143) |
| Pieces more than 1 m off the terrain | 469 | 368 | 425 | 331 |
| Road conflicts | 2412 | 1979 | 2166 | 1729 |
| Earthworks that stop at the 60 m cap (vertices) | 1904 | 728 | 3459 | 1322 |
| Map vertices changed by the earthworks | 18.5 % | 17.7 % | 14.4 % | 13.5 % |

The grade over 35 % rises because a road now gives up its grade aim where holding it would leave the ground: the
road is steeper and stays on the terrain instead of hanging above it. The earthworks that stop at the cap fall
because cuttings and embankments follow the hillside; the 60 m cap and the 12.875 m level strip still leave some on
the steepest ground.

Pieces with a cross fall over 5 % or more than 0.15 m off the terrain, by what is next to them (edge to edge, within
three cells), after the fixes:

| What is next to the piece | 1024 pieces | 1024 length | 1024 cross fall over 5 % | 2048 pieces | 2048 length | 2048 cross fall over 5 % |
|---|---|---|---|---|---|---|
| Another road overlaps it, at a different height | 458 | 5.9 km | 3.0 km | 551 | 6.6 km | 3.4 km |
| Another road overlaps it, at the same height where they are closest | 514 | 6.3 km | 3.7 km | 609 | 7.3 km | 3.9 km |
| Another road within three cells, at a different height | 1111 | 12.5 km | 5.8 km | 542 | 5.8 km | 2.6 km |
| Another road within three cells, at the same height | 301 | 3.6 km | 1.9 km | 141 | 1.6 km | 0.7 km |
| Another part of the same road (a hairpin), at a different height | 304 | 3.0 km | 1.6 km | 155 | 1.3 km | 0.7 km |
| Nothing: an arc of radius 10 m or 25 m | 732 | 6.6 km | 3.2 km | 901 | 8.4 km | 3.9 km |
| Nothing: other | 591 | 7.8 km | 1.4 km | 775 | 9.1 km | 2.1 km |

What is left:

- **Hairpins.** 1345 pieces at 1024 and 618 at 2048 still differ by more than the step the ground between the legs
  can hold (short by 2.8 m at the median, 9.5 m at p90 at 1024; 1.5 m and 7.8 m at 2048). The finer grid helps most;
  widening the hairpins to 25 m arcs, or a wall between the legs, is still open.
- **The 332 stretch ends that miss what they have to meet** are unchanged and not analysed here; they need their own
  investigation. The 737 crossings without a junction piece and the 21 tunnels without pieces are also unchanged.
- **The 1353 joints far off the ground for neither known reason** (section 2.6) were not looked into; the fixes do not
  address them.
- **The limits** are estimates from real vehicles. Driving the ramps in the game (Task 1 of the roads plan) replaces
  them and may move every figure in this section.
- **The grid.** At 2048 the hairpin short count halves (1345 to 618), conflicts fall (1979 to 1729) and cross fall over
  5 % falls (19.1 to 15.9 km); the cost is build time and PBO size (38 s and 220 MB instead of 16 s and 75 MB, see
  `examples/berchtesgaden/README.md`).

## 6. The grid sizes compared, and where pieces miss the terrain (2026-11-12)

Measured with `road-analysis` at 1024, 2048 and 4096 on the tree with the review fixes (bridge ground, deck height,
cut ids, road bed). The 1024 column equals the `report.json` of a full build. Not measured: the build with binarize
and the PBO size at 2048 and 4096.

| Rule | 1024 | 2048 | 4096 |
|---|---|---|---|
| `road_misses_terrain` | 3569 | 3353 | 3872 |
| of these above 0.5 m | 817 | 735 | 792 |
| largest miss | 13.0 m | 22.0 m | 26.4 m |
| `road_conflict` | 2001 | 1743 | 1495 |
| largest conflict | 37.4 m | 39.9 m | 51.8 m |
| `road_grade` | 972 | 972 | 940 |
| `road_not_drivable` | 254 | 243 | 245 |
| not drivable | 5.7 km | 4.8 km | 4.1 km |
| `path_too_steep` | 89 | 91 | 73 |
| `earthworks_capped` | 143 | 122 | 108 |
| `road_gap`, `join_without_junction`, `tunnel_omitted` | 332, 737, 21 | same | same |

**A finer grid does not remove the violations.** It lowers the conflicts by a quarter and shortens the embankments
(below); the worst values grow, because a finer grid shows a step that a coarse one smears. The road pipeline costs
the same at every size (177 s, of which 168 s is placing; 1.4 to 1.7 GB).

**A miss above 0.5 m is a clash between roads.** The first cause that applies, pieces at 1024 / 2048 / 4096:

| Cause | 1024 | 2048 | 4096 |
|---|---|---|---|
| Another road within three cells, at a different height | 627 | 483 | 455 |
| Another leg of the same road, at a different height | 93 | 69 | 76 |
| A change of grade over 1 %/m at the piece's own joint | 57 | 144 | 215 |
| The end of a bridge | 22 | 32 | 37 |
| A bend | 15 | 2 | 2 |
| Other (same height, capped earthworks) | 3 | 5 | 7 |
| Unknown | 0 | 0 | 0 |

The other road's core holds vertices at or beside the piece's corners at its own height; printed for three pieces,
the vertex was 3 to 16 m off the piece's height and belonged to a higher-ranked road 11 to 14 m away. Shaping the
terrain harder cannot repair this: the two roads want different ground. The small misses (0.15 to 0.5 m; 2752 at
1024) are roads at different heights (1492), bends (528) and grade changes at a joint (420).

**The road is not cut into the hillside.** Finished minus original ground, median, at 1024:

| Side slope of the ground | Uphill edge | Centre | Downhill edge | Fill over 2 m / 5 m at the downhill edge | Fill runs (median) |
|---|---|---|---|---|---|
| 0 to 10 % | -0.1 m | 0.1 m | 0.3 m | 25 % / 7 % | 18 m |
| 20 to 35 % | -1.5 m | -0.4 m | 0.5 m | 34 % / 16 % | 27 m |
| 35 to 60 % | -2.5 m | -0.8 m | 0.7 m | 38 % / 19 % | 35 m |
| above 60 % | -4.8 m | -2.3 m | 0.3 m | 36 % / 18 % | 37 m |

Half of the pieces have their whole width in cut, in every band. A quarter of the pieces on flat ground stand on
more than 2 m of fill: that comes from a profile that does not follow the ground, not from the hillside. The fill runs
6 to 19 m at the median at 4096; the balance of cut and fill does not change with the grid.

**The extremes.** A road whose profile cannot follow the ground within its grade ceiling runs 20 to 38 m below it, and
where its end is fixed to another road's height the whole difference lands in one piece: 294 % over 9 m at
(3450, 3533), where a track needs 30 % on average and may have 25 %; 235 % at (4334, 5458) at 2048 and 4096. The
105.9 m gap at (6173, 5641) is a 7.7 m stretch between two junctions that gets a 24.5 m piece pointing elsewhere: a
fault of the placing. Not explained: the 37.4 m conflict at (1281, 3406), with one road within 45 m.

## 7. The road heights plan, task by task

### 7.1 Track limits (2026-11-13)

Gravel tracks now have a hard limit of 40 % and dirt tracks 50 % (ceilings 36 % and 45 %), before 30 % and 28 %
(section 3, "Decided by the maintainer (2026-11-13)"). Measured with `road-analysis` at 1024 and 2048 on the tree
with this change. The "before" columns are section 6; the same tool was run on the tree without the change and gave
the same counts (3569, 2001, 972, 254 at 1024 and 3353, 1743, 972, 243 at 2048). Raw outputs:
`/var/tmp/naves-task1.4SzKQM/` (`b*` before, `a*` after).

| Rule | 1024 before | 1024 after | 2048 before | 2048 after |
|---|---|---|---|---|
| `road_not_drivable` | 254 | 227 | 243 | 226 |
| not drivable | 5.7 km | 5.1 km | 4.8 km | 4.7 km |
| `road_grade` | 972 | 1035 | 972 | 957 |
| `road_misses_terrain` | 3569 | 3935 | 3353 | 3540 |
| largest miss | 13.0 m | 9.8 m | 22.0 m | 22.0 m |
| `road_conflict` | 2001 | 2007 | 1743 | 1702 |
| `path_too_steep` | 89 | 92 | 91 | 85 |
| `earthworks_capped` | 143 | 133 | 122 | 115 |
| `road_gap`, `join_without_junction`, `tunnel_omitted`, `road_in_water` | unchanged | | | |

**The Klingerweg** (`158289388:259090646-278531300`, strip 1027 at 2048): the largest planned grade is 45 %, the
ceiling of a dirt track; the joint grades along its 225 m are up to 45 %, and 41 to 45 % on seven of its 23 joints.

**Worse:** `road_misses_terrain` grows by 366 at 1024 and 187 at 2048, and `road_grade`, `road_conflict` and
`path_too_steep` at 1024 by 63, 6 and 3. The cause was not examined: a track that may be steeper gets a different profile, so its pieces meet
the ground and other roads differently.

Not measured: the share of `mud` length cut deeper than 10 m. The tool does not print it and this task does not
extend the tool.

### 7.2 Roads tied to the junction heights (2026-11-13)

`solve_profiles` takes the junctions of `find_junctions` and holds the heights of the roads that belong to a
junction (its stretches: they end there, cross there in the graph, or are the road a chain end lies on) at as few
joints as it can: both joints of a junction piece, one joint where a road passes a junction, the end where a road
ends and inward only along the other road's level strip. A junction moves at most 3 m from its ground median in the
repair; a link still over its ceiling is unresolved (`junction_too_steep`). The vertical radius is kept across a held
joint, and as a last resort the free joints between two held joints are blended toward the straight line until no
piece is over the hard limit. Measured with `cargo run --release -p naves-export --example road-analysis 1024`
(and `2048`); the "before" columns are the "after" columns of 7.1.

**What the first attempts showed:** with every joint of a junction held, `road_grade` was 2548 and 79 pieces had a
planned grade above 100 % (up to 1147 %); the causes were links measured edge to edge (the span the road holds is
shorter), piece grades measured over the length of a piece and not the run between joints, a smoothing that assumed
equal spacing, and the 6 % meeting grade between two junction heights. The review then found that the holds were
longer than needed, that unrelated roads within 13 m were held to a junction, and that junctions moved up to 7.5 m.

| Rule | 1024 before | 1024 after | 2048 before | 2048 after |
|---|---|---|---|---|
| `road_not_drivable` | 227 | 278 | 226 | 258 |
| not drivable | 5.1 km | 7.4 km | 4.7 km | 5.7 km |
| `road_grade` | 1035 | 1559 | 957 | 1336 |
| `road_misses_terrain` | 3935 | 3599 | 3540 | 2999 |
| largest miss | 9.8 m | 10.3 m | 22.0 m | 10.1 m |
| `road_conflict` | 2007 | 2148 | 1702 | 1882 |
| `path_too_steep` | 92 | 81 | 85 | 70 |
| `earthworks_capped` | 133 | 123 | 115 | 114 |
| `junction_too_steep` | n/a | 1 | n/a | 1 |
| `road_gap`, `join_without_junction`, `tunnel_omitted`, `road_in_water` | unchanged | | | |

**Junctions:** 1194 (452 pieces, 629 joins, 113 ends on roads) at both sizes, 1403 links at 1024 and 1420 at 2048,
one unresolved at each; the earlier study found 1269 and 1535. The links differ by definition (the span between held
joints); the 6 % fewer junctions was not examined. The membership check removed 86 holds at 1024 and 110 at 2048
(4604 and 3988 without it); 4518 and 3878 joints are held, in 1484 and 1159 runs of more than one joint (junction
pieces and ends), which is **15.7 km** (1024) and **10.5 km** (2048) of road held level.

**The three "done" criteria:**

1. Pieces with a planned grade above 100 %: **not met**: 1 at 1024 (332 % at (1021.0, 4505.6), stretch
   `216815452:21623801-2611724029`), 2 at 2048 (309 % there and 218 % at (841.7, 4818.7)). All have a held joint on
   one side only (a span from a held joint to a free end, where there is no line to blend to).
2. Pieces above the hard limit without a `road_grade` or `road_not_drivable` violation within 30 m: **met**: 0 (63 and
   48 pieces are above the limit, all reported).
3. Largest distance of a junction from its ground median: **met**: 3.00 m at both sizes (the cap; p50 and p90 0.00 m).

The largest `road_grade` is **332 %** (1024) and **309 %** (2048), at (1021.0, 4505.6), against 235 % in 7.1: worse.
The invariant of the heights (consecutive held joints of different junctions have a link, and the grade between
them is within its ceiling unless the link is unresolved): 1403 and 1420 pairs, none without a link, 2 over the ceiling
at each size, all on links of 1 m between held joints that lie 0 m apart (a link is at least 1 m long).

**The vertical bends:** `road_grade` is 1559 at 1024, of which 1180 are vertical-bend findings: **640 at a held joint,
220 next to one, 317 elsewhere**, 3 at a bridge deck (2048: 1019; 520, 201, 296, 2). Before the holds were cut down (7.2
first attempt) 1053 were at a held joint. Estimate of how far each junction height along a through chain would have
to move for the chain to keep the minimum vertical radius (the distance of the middle height from the straight line
through its two neighbouring held heights beyond what the radius allows, `(d1 + d2) / 2 / R / (1/d1 + 1/d2)`,
neighbours not moved): 798 triples at 1024, p50 0.00 m, p90 0.00 m, max 1.28 m, above 0.5 m 11, above 1 m 3, above
2 m 0 (2048: 804, max 1.42 m, 10, 1, 0). The heights themselves are nearly collinear, but 640 bends are still at held
joints, so something other than the heights makes them there: the solver's treatment next to a fixed height (the
change of grade across a held joint is limited in the clip, but the pieces beside it are short and the radius is
long). The estimate cannot show that, since it treats a run as a point.

**Worse than before 7.1:** `road_grade` (1035 to 1559 at 1024, 957 to 1336 at 2048), `road_not_drivable` (227 to
278, 226 to 258), `road_conflict` (+141, +180) and the largest `road_grade`. Better: `road_misses_terrain` (3935 to
3599, 3540 to 2999), `path_too_steep`, `earthworks_capped`, the largest miss at 2048.

### 7.3 Level patches, faces and roads close together (2026-11-13)

`conform` takes level patches and the side slope of the original ground at each road piece (`side_slopes`: at the
middle of the piece the difference of the ground 10 m to each side over 20 m, then the median over the pieces of the
chain within 25 m). A join and a chain end on a road get a patch of the junction height, radius half the width of
the widest road plus one cell; a junction piece is already a level segment and gets none. Where the side slope is
above 35 % the cut at the uphill side is a face (rise over run 50, one cell wide). A vertex inside a road's own
width belongs to that road, and a patch is never in conflict with a road. A vertex that two cores at heights more
than 0.3 m apart both want goes to the one whose edge is nearer (so to the one that has it inside its own width), an
exact tie to the higher rank, and the step between them is a face; it is a `road_conflict` only where the vertex is
within the width of both. Measured with `road-analysis 1024`
(and `2048`); the "before" columns are the "after" columns of 7.2, except the rows marked section 6, which were not
in 7.2.

**What the first version showed.** The first version gave a contested vertex to the road whose edge is nearer, and
let a patch compete with roads in the same way. `road_misses_terrain` rose from 3599 to 4552 (1024) and from 2999 to
4010 (2048), the misses above 0.5 m from 817 to 1543 and from 735 to 1466 (section 6 figures). The review found that
a patch took vertices inside a through road's width near its centre (the road is held at one joint only and slopes
through the patch): without patches the misses were 4351 and the conflicts 693, with them 4552 and 1291. That is
fixed: a patch takes only what lies outside every road's width.

**Which rule for the contested vertices.** (a) the road whose edge is nearer, a tie to the higher rank; (b) the
higher rank, the nearer edge only between equal ranks; (c) a road that has the vertex inside its own width, then
the higher rank, then the nearer edge. All with the patch fix. 1024 / 2048:

| Rule | misses above 0.5 m | above 2 m | `road_not_drivable` | `road_conflict` |
|---|---|---|---|---|
| (a) nearer edge | 1443 / 1378 | 122 / 123 | 236 / 232 | 742 / 1130 |
| (b) rank first | 1384 / 1211 | 159 / 148 | 258 / 237 | 723 / 1113 |
| (c) own width, then rank | 1587 / 1424 | 171 / 150 | 278 / 247 | 739 / 1126 |

(a) is the rule of the plan and of spec 5.3 (an edge not above zero is always nearer than one above zero, so each road
keeps its own width), and it has the fewest misses above 2 m; it was kept. (b) has fewer misses above 0.5 m (1384
against 1443, 1211 against 1378), but lets the higher road take a vertex inside the lower road's width without a word
and has more misses above 2 m (159 against 122, 148 against 123). (c) keeps each road's width as well, but is worse than
(a) on every miss figure (misses above 0.5 m 1587 against 1443 and 1424 against 1378, above 2 m 171 against 122 and 150
against 123, not drivable 278 against 236 and 247 against 232).

**Do roads lose vertices they did not have to lose?** Pieces missing by more than 0.5 m whose cause is another road at
a different height, with a vertex within one cell of their own width whose final height is not the height of any
piece of their own strip there, split by whether the vertex is also within one cell of the width of another road at
another height (contested) or not (taken without need). On the final rule (a), 1024 / 2048:

| | 1024 | 2048 |
|---|---|---|
| pieces of that cause | 1327 | 1292 |
| with a contested ring vertex | 1158 | 1217 |
| with a vertex taken without need | 40 | 111 |
| with only such vertices | 12 | 19 |
| of the needless, inside a level patch | 40 | 111 |

All the vertices taken without need are inside a level patch: a patch takes a ring vertex from a road where the
patch's edge is nearer, which the decision for patches allows. It is not a defect of the claim. Under (b) and (c) the
count is higher (577 and 476 at 1024) because the higher road takes ring vertices of the lower road that lie between
one and one and a half cells from its width, outside the one cell of the measure. A first version of this count
compared with the height of the one piece only and found about 900 pieces; those were vertices of the next piece of
the same strip.

**Final figures** (rule (a), patch rule):

| Rule | 1024 before | 1024 after | 2048 before | 2048 after |
|---|---|---|---|---|
| `road_not_drivable` | 278 | 236 | 258 | 232 |
| not drivable | 7.4 km | 5.5 km | 5.7 km | 5.3 km |
| `road_grade` | 1559 | 1559 | 1336 | 1336 |
| `road_misses_terrain` | 3599 | 4561 | 2999 | 3978 |
| of these above 0.5 m (section 6) | 817 | 1443 | 735 | 1378 |
| above 2 m | n/a | 122 | n/a | 123 |
| largest miss | 10.3 m | 6.7 m | 10.1 m | 10.1 m |
| `road_conflict` | 2148 | 742 | 1882 | 1130 |
| `path_too_steep` | 81 | 130 | 70 | 75 |
| `earthworks_capped` | 123 | 100 | 114 | 92 |
| `junction_too_steep` | 1 | 1 | 1 | 1 |

Misses above 0.5 m by cause (section 6 figures before), 1024 / 2048:

| Cause | 1024 section 6 | 1024 now | 2048 section 6 | 2048 now |
|---|---|---|---|---|
| Another road within three cells, at a different height | 627 | 1327 | 483 | 1292 |
| Another leg of the same road, at a different height | 93 | 72 | 69 | 38 |
| A change of grade over 1 %/m at the piece's own joint | 57 | 1 | 144 | 0 |
| The end of a bridge | 22 | 31 | 32 | 41 |
| A bend | 15 | 10 | 2 | 4 |

Pieces on a side slope above 35 % with fill over 2 m at the downhill edge: 35 % at 1024 and 37 % at 2048, against
38 % at 35 to 60 % in section 6: a little better at most; the face changes the uphill side, the fill on the
downhill side is set by the profile.

**`path_too_steep`** rose from 81 to 130 at 1024 (70 to 75 at 2048). With the faces on steep ground off the count at 1024
was 89, with the step faces between roads off 124, without the patches 131 (measured on the nearer-edge rule): the
rise is the faces on steep ground. A footpath beside a road on a hillside now runs along a near-vertical wall; the
check reports what is there. That is the intended face, not a defect of the shaping; whether paths should be moved
off a face is open.

**Worse than before 7.2:** `road_misses_terrain` (3599 to 4561 at 1024, 2999 to 3978 at 2048), the misses above 0.5 m
(817 to 1443, 735 to 1378, against section 6) and `path_too_steep` (81 to 130, 70 to 75). **Better:** `road_conflict`
(2148 to 742; 1882 to 1130), `road_not_drivable` and the length that cannot be driven, `earthworks_capped`, the
largest miss at 1024.

### 7.4 A road far from the ground (2026-11-13)

The new rule `road_far_from_ground` reports a piece whose planned height at its middle (the conformed terrain
there) differs from the original ground there by more than `earthwork_allowance_m` of the side slope of the original
ground: 30 m up to a side slope of 0.20, linear to 100 m at 0.60. A cut is a negative `off_ground_m`, an embankment a
positive one; bridges have no check and so are not measured, and a piece without ground data reports nothing. Like
`road_misses_terrain` it is one violation per piece, not per stretch. This step only reports: nothing in the shaping
changed. Measured with `road-analysis 1024` (and `2048`).

**Count of the new rule.** 11 at 1024 and 11 at 2048, all of them cuts, all on one dirt track (family `Mud`) in the
north-west of the map, positions (2442, 5579) to (2500, 5496), 11 consecutive pieces, cuts of 34 to 38 m on ground with
a side slope of 0.06 (allowance 30 m). The trial expected none on tracks; this is one track. No other family has a
piece over its allowance, on public roads or elsewhere, and there is no embankment over its allowance.

| Family | pieces | over the allowance (1024 / 2048) | p50 | p90 | max, of the absolute value (1024 / 2048) |
|---|---|---|---|---|---|
| Asf1 | 857 | 0 / 0 | 0.7 / 0.7 m | 4.1 / 3.8 m | 14.6 / 13.8 m |
| Asf2 | 709 | 0 / 0 | 0.9 / 0.9 m | 4.5 / 4.3 m | 10.8 / 10.0 m |
| Asf3 | 2363 | 0 / 0 | 1.3 / 1.1 m | 5.4 / 4.9 m | 22.1 / 22.0 m |
| City | 7264 | 0 / 0 | 0.9 / 0.8 m | 4.1 / 3.8 m | 17.4 / 16.7 m |
| Grav | 4610 | 0 / 0 | 1.6 / 1.5 m | 5.5 / 5.2 m | 17.1 / 17.2 m |
| Mud | 4170 | 11 / 11 | 2.1 / 1.8 m | 6.6 / 6.2 m | 37.5 / 37.7 m |

**Extremes.** The deepest cut is the Mud track above: 37.5 m at 1024 (37.7 m at 2048) at (2485, 5529), side slope 0.06,
allowance 30 m. The highest embankment is 22.1 m (22.0 m) of an `Asf3` road at (5943, 4655) on a side slope of 1.05
(allowance 100 m), well within what fits.

**Diagnosis of the 11 pieces: (e), a solver fault in `ramp_targets`.** The pieces are joints 62 to 72 of strip 968, a
Mud chain of 134 pieces with joints held at four places (`road-analysis` with `DIAG=2470,5537,100`: ways, strips,
holds, spans, ground and DEM near a place). Its ways are all `highway=track` (323475941, 323475937 and the track ways
1121665286, 1121665289 and 1121665290 under the 11 pieces; no `bridge`, `tunnel`, `layer` or `tracktype`), family Mud,
drivable, none a bridge or a tunnel; footpaths run beside it (1121665287, 1121665288) and no road passes above or below it.
Its holds: joint 0 to the patch of junction 211 (348.37 m), joints 10 and 11 to the piece of junction 198 (365.06 m),
joint 100 to junction 166 as an arm (555.19 m), joint 134 to junction 182 as the end (519.65 m). Both ends of the long
span are held at the ground median (junctions 198 and 166 were moved 0.00 m, so (b) is out); the link between them
(183) is 1053 m long with 190.13 m of height difference, a required grade of 0.181, under its ceiling of 0.45.
The ground is real (so (d) and (a) are out): a smooth DEM (raw samples 10 m apart rise 943 to 1052 m along y through
the place), no no-data cell and a largest step of 4.5 m between neighbouring 6.25 m cells within 50 m (a hillside,
not a fault); the map is built at `horizontal = 0.5`, so all the heights here are half the real ones. The ground
along the span climbs about 21 % on average to 561 m near the end (joint 92) and falls back to 555 m at the end
junction, a convex ramp with a plateau: a profile with the ground as target follows it within a few metres (the profile of the
sections held to the ground, after joint 100, does).

What cuts it is `ramp_targets` in `profile.rs`. For a span between two held heights whose required grade is at least
80 % of the span's grade limit, it replaces the ground targets of the span with the straight line between the two
heights ("has no room to follow the ground"). The limit of the span is the **smallest** limit of any piece in it, and a
bend lowers the limit of its piece by `(r - w/2) / r`, at most to 0.5. This span has one hairpin of radius 10 m (piece
37, near (2594, 5263)), which gives a factor of 0.75 and a span limit of 0.25 x 0.75 = 0.188 at the first of the three
base grades (aim 0.25, 0.35, ceiling 0.45); the required 0.181 is 96 % of it, so the whole kilometre becomes one
straight ramp at about 18 %. The grade the ramp needs is 72 % of the aim, so the ground itself asks for nothing the road cannot do. Then `profile_chain` accepts the first base grade
because its test for a profile that left the ground, `|height - target| > 4 m`, compares the heights with the
**replaced** targets: the maximum deviation was 3.3 m, though the heights are 37.5 m from the ground (a temporary probe
in `profile_chain`, removed again). The result is a profile that stays below the ground by up to 37 m wherever the
ground is above the line, while the ground is flat across (side slope 0.06), which is what the new rule is for. The
same path through the other bases would not have happened: at 0.35 and 0.45 the span is not tight (ratios 0.69 and
0.53).

**The smallest correct fix** (not made, it changes the shaping and belongs in its own task; owner: `ramp_targets` /
`profile_chain` in `crates/naves-core/src/profile.rs`): judge the span by what its pieces can do together, not by its
worst piece. Either test `required >= TIGHT_SPAN * limit` against the length-weighted mean limit of the span, or limit
the straight line to the stretch around the tight piece; and compare the heights with the ground, not with the replaced
targets, in the `off` test, so that a profile that has left the ground by more than `RELAX_OFF_M` goes on to the next
base grade. Other spans with a ratio near 0.8 and a single tight bend are likely to be flattened the same way; the 11
pieces here are the only ones over the allowance, others may be within it.

**The other counts did not change** (1024 / 2048, equal to the final figures of 7.3): `road_not_drivable` 236 / 232,
`road_grade` 1559 / 1336, `road_misses_terrain` 4561 / 3978, `road_conflict` 742 / 1130, `path_too_steep` 130 / 75,
`earthworks_capped` 100 / 92, `junction_too_steep` 1 / 1.

#### After the fix of the ramp targets (2026-11-13)

Two changes in `profile.rs`. (1) `ramp_targets` judges a span between two held heights by the mean of the limits of its
pieces, weighted by length, not by the lowest: `required >= 0.8 * mean limit` makes the straight line. Of the two ways
named in the diagnosis (the mean, or the straight line only around the tight piece) the mean is the simpler: one
number per span, the same test as before, and a span of one kind of piece (the case `ramp_targets` was written for)
gives the same answer, since the mean and the lowest limit are then equal. The straight line around a piece would need
a rule for how far around, which has no natural answer. (2) The test in `profile_chain` for a profile that has left the
ground (`RELAX_OFF_M`) compares the heights with the ground targets, not with the replaced ones, so a straight ramp that
is far from the ground makes the solver try the next grade limit. Tests: `one_tight_bend_does_not_turn_a_long_span_into_a_straight_ramp`
(a kilometre with one 10 m bend: 21.5 m from the ground before, within 4 m after) and
`a_profile_that_left_the_ground_for_a_straight_ramp_is_tried_again_at_a_looser_limit` (30 m before change (2), within
12 m after). No existing test changed.

**Count of the rule:** `road_far_from_ground` is 0 at 1024 and at 2048 (11 / 11 before); the Mud track is now at most
15.4 m (14.8 m) from the ground.

**Spans turned into straight ramps** (`road-analysis`, `RAMPSPANS`: the spans between two held joints, tested at the
first, aim, grade limit with the piece limits from the bends only, so a proxy for the solver's own test, which goes on
to the next limits when the profile has left the ground). 1024: 1079 spans, 362 (48.6 km) by the lowest limit, 288
(33.9 km) by the mean. 2048: 1131 spans, 376 (50.1 km) against 291 (33.8 km). So a fifth of those spans by number and three tenths by length follow the ground now.

**Distribution of the absolute `off_ground_m`** (p50 / p90 / max, m; before, from the table above, then after), 1024:

| Family | before | after |
|---|---|---|
| Asf1 | 0.7 / 4.1 / 14.6 | 0.7 / 4.1 / 14.6 |
| Asf2 | 0.9 / 4.5 / 10.8 | 0.9 / 4.5 / 10.4 |
| Asf3 | 1.3 / 5.4 / 22.1 | 1.3 / 5.2 / 22.1 |
| City | 0.9 / 4.1 / 17.4 | 0.9 / 4.0 / 17.4 |
| Grav | 1.6 / 5.5 / 17.1 | 1.6 / 5.3 / 17.1 |
| Mud | 2.1 / 6.6 / 37.5 | 2.0 / 5.9 / 15.4 |

2048:

| Family | before | after |
|---|---|---|
| Asf1 | 0.7 / 3.8 / 13.8 | 0.7 / 3.7 / 13.8 |
| Asf2 | 0.9 / 4.3 / 10.0 | 0.9 / 4.2 / 10.1 |
| Asf3 | 1.1 / 4.9 / 22.0 | 1.1 / 4.6 / 22.0 |
| City | 0.8 / 3.8 / 16.7 | 0.8 / 3.6 / 16.7 |
| Grav | 1.5 / 5.2 / 17.2 | 1.4 / 4.8 / 17.0 |
| Mud | 1.8 / 6.2 / 37.7 | 1.7 / 5.5 / 14.8 |

**All violations against the final figures of 7.3** (1024 before / after, 2048 before / after):

| Rule | 1024 before | 1024 after | 2048 before | 2048 after |
|---|---|---|---|---|
| `road_not_drivable` | 236 | 239 | 232 | 236 |
| not drivable | 5.5 km | 5.6 km | 5.3 km | 5.3 km |
| `road_grade` | 1559 | 1587 | 1336 | 1353 |
| `road_misses_terrain` | 4561 | 4588 | 3978 | 4004 |
| of these above 0.5 m | 1443 | 1457 | 1378 | 1374 |
| above 2 m | 122 | 123 | 123 | 130 |
| largest miss | 6.7 m | 6.7 m | 10.1 m | 10.1 m |
| `road_conflict` | 742 | 744 | 1130 | 1134 |
| `path_too_steep` | 130 | 130 | 75 | 77 |
| `earthworks_capped` | 100 | 101 | 92 | 87 |
| `junction_too_steep` | 1 | 1 | 1 | 1 |
| `road_far_from_ground` | 11 | 0 | 11 | 0 |

**What got worse:** a little of everything that depends on how close the profile stays to steep ground: `road_grade`
(1559 to 1587, 1336 to 1353), `road_not_drivable` (236 to 239, 232 to 236), `road_misses_terrain` (4561 to 4588,
3978 to 4004), the misses above 2 m at 2048 (123 to 130) and `road_conflict` by 2 and 4. A profile that follows steep ground
has the grades of the ground, a straight ramp had the gentlest grade that fits; that is the price of not cutting 37 m
through a hill. **Better:** the cut of the Mud track and the other extremes of the Mud family (37.5 m to 15.4 m) and
`earthworks_capped` at 2048 (92 to 87). The changes are small against the counts (under 2 %).

### 7.5 Bridges by surface and scaled (2026-11-13)

Bridge models are now chosen by the road's surface (wooden for `Grav` and `Mud`, every other model for the paved
families), the narrowest that is at least as wide as the road, and a model narrower than its road is scaled up to the
road's width, at most 1.5 times (`MAX_BRIDGE_SCALE`). Before, paved roads were offered the stone models only, and the
width warning was given once per bridge run. Measured with `road-analysis 1024` and `2048` (the bridge lines are new in
the example; placement does not depend on the size, so both sizes give the same bridges). **Scaled bridges have not been
tested in the game.**

**Bridge objects by model** (the same at 1024 and 2048): 24 objects in all.

| Model | objects | scaled |
|---|---|---|
| `bridge_metal_25` | 13 | 0 |
| `bridge_stone_25` | 9 | 0 |
| `bridge_wood_25` | 2 | 2 |

**Scale:** only the 2 wooden pieces are scaled, both by 1.18 (a 5 m deck for the 6 m of a `Grav` road); no bridge reached
the cap of 1.5, so nothing was held back. All 22 paved bridge objects fit their road unscaled: the metal model (12 m) now
serves the wide roads that were given the 9 m stone model before.

**Width warnings** (`bridge on ...`): 0, was 15.

**Wooden against paved, by road family** (objects): `Asf1` 0 / 6, `Asf2` 0 / 1, `Asf3` 0 / 8, `City` 0 / 7, `Grav` 2 / 0;
no bridge on a `Mud` track.

**All violations against the final figures of 7.4** (1024 before / after, 2048 before / after):

| Rule | 1024 before | 1024 after | 2048 before | 2048 after |
|---|---|---|---|---|
| `road_not_drivable` | 239 | 240 | 236 | 236 |
| not drivable | 5.6 km | 5.6 km | 5.3 km | 5.3 km |
| `road_grade` | 1587 | 1583 | 1353 | 1352 |
| `road_misses_terrain` | 4588 | 4589 | 4004 | 4001 |
| of these above 0.5 m | 1457 | 1455 | 1374 | 1373 |
| above 2 m | 123 | 123 | 130 | 131 |
| `road_conflict` | 744 | 743 | 1134 | 1132 |
| `path_too_steep` | 130 | 130 | 77 | 77 |
| `earthworks_capped` | 101 | 101 | 87 | 87 |
| `junction_too_steep` | 1 | 1 | 1 | 1 |
| `road_far_from_ground` | 0 | 0 | 0 | 0 |

**What got worse:** `road_not_drivable` at 1024 by 1 (239 to 240) and the misses above 2 m at 2048 by 1 (130 to 131);
`road_misses_terrain` at 1024 by 1. These are noise-level shifts from bridges of another length (a wider or a scaled model
changes where the run ends); no count moved by more than 4 (`road_misses_terrain` at 2048, 4004 to 4001, is a gain). The
real catalog is little affected because only 24 objects are bridges. What the step does not show is how a scaled
bridge behaves in the game: it is untested, and the fallback of spec 6.1 (the widest model at scale 1 with a warning) is
a one-line change in `choose_bridge`.

### 7.6 Every OSM bridge is a bridge (2026-11-13)

A bridge in OpenStreetMap that is shorter than one (scaled) bridge model used to be laid as ordinary road (below a
chord of 10 m it was never a bridge). Now `widen_bridges` grows it evenly into the stretches on both sides until one
model fits; each neighbour keeps at least 1 m, and a bridge that still does not fit stays an ordinary road with the
warning `bridge on <id>: no room for a bridge model between the junctions`. Measured with `road-analysis 1024` and
`2048` (the example now widens as the build does and prints the `OSMBRIDGE` lines). Both sizes give the same
bridges. **Scaled bridges have not been tested in the game.**

**OSM bridge stretches:** 66 (without tunnels). Built as a bridge before (chord of 10 m or more): 24. After: 47.

**Widened:** 46 stretches. By how much, in metres: p10 9.2, median 17.4, p90 30.9, largest 34.3. (Some of them were
longer than 10 m and were built before; they too grew to one model.)

**No room (19):** these stay ordinary road. Id, length, position in map metres.

| Stretch | length | at |
|---|---|---|
| `1093361022:510121087-10013276225` | 3.7 m | 5642, 2886 |
| `123928797:1380536726-2611738645` | 9.8 m | 548, 4488 |
| `130047131:1433545748-2612901794` | 9.9 m | 838, 4487 |
| `136213585:282908976-1494536585` | 7.3 m | 4583, 5445 |
| `150368334:282908960-282908962` | 8.6 m | 4557, 5423 |
| `172178577:475565028-1831110958` | 6.8 m | 3649, 3483 |
| `23449348:253952136-1046300707` | 13.8 m | 1697, 4200 |
| `240795460:253896556-2484891251` | 8.4 m | 4092, 5483 |
| `296178597:2611738595-2999699414` | 6.4 m | 635, 4508 |
| `39537999:282908976-1494536570` | 8.4 m | 4581, 5443 |
| `39782354:476903462-476903465` | 11.5 m | 5260, 6289 |
| `48391149:282908967-1494536518` | 13.0 m | 4587, 5409 |
| `48391150:282908960-1494536543` | 3.6 m | 4559, 5431 |
| `48391152:282908976-2041562015` | 8.0 m | 4590, 5437 |
| `48450084:21628088-615524621` | 17.2 m | 4447, 4880 |
| `598164112:5693507720-5693511921` | 2.8 m | 3332, 3371 |
| `74208396:877758539-877758550` | 3.7 m | 5118, 5781 |
| `74208422:877758668-877758670` | 5.3 m | 5130, 5763 |
| `953825999:8828196890-8828196891` | 3.3 m | 643, 4433 |

Six of them lie within 40 m of 4575, 5430 (a cluster of ramps and junctions).

**Bridge objects by model** (the same at both sizes): 47 in all, before 24.

| Model | objects | scaled |
|---|---|---|
| `bridge_metal_25` | 27 | 0 |
| `bridge_stone_25` | 12 | 0 |
| `bridge_wood_25` | 8 | 7 (all 1.18) |

By road family (wooden / paved): `Asf1` 0 / 9, `Asf2` 0 / 6, `Asf3` 0 / 6, `City` 0 / 18, `Grav` 7 / 0, `Mud` 1 / 0. Width
warnings: 0.

**All violations against the final figures of 7.5** (1024 before / after, 2048 before / after):

| Rule | 1024 before | 1024 after | 2048 before | 2048 after |
|---|---|---|---|---|
| `road_not_drivable` | 240 | 242 | 236 | 235 |
| not drivable | 5.6 km | 5.7 km | 5.3 km | 5.2 km |
| `road_grade` | 1583 | 1617 | 1352 | 1405 |
| `road_misses_terrain` | 4589 | 4598 | 4001 | 4010 |
| of these above 0.5 m | 1455 | 1465 | 1373 | 1366 |
| above 2 m | 123 | 140 | 131 | 135 |
| `road_conflict` | 743 | 746 | 1132 | 1125 |
| `path_too_steep` | 130 | 133 | 77 | 79 |
| `earthworks_capped` | 101 | 102 | 87 | 85 |
| `junction_too_steep` | 1 | 1 | 1 | 1 |
| `road_far_from_ground` | 0 | 0 | 0 | 0 |

**What got worse:** `road_grade` by 34 at 1024 (1583 to 1617) and by 53 at 2048 (1352 to 1405); the misses above 2 m by
17 at 1024 (123 to 140) and by 4 at 2048; `road_misses_terrain` by 9 at both sizes; `road_not_drivable` by 2 at 1024;
`path_too_steep` by 3 and 2; `road_conflict` by 3 at 1024. A bridge deck is level between its ends, and a bridge that
is now 25 m or more long (it was a road piece of a few metres) holds two joints further apart, so the roads beside it
probably carry the grade (not isolated by a separate run); the `road_grade` rise is the largest effect of this step. Better: `road_conflict` at 2048 by 7,
`earthworks_capped` at 2048 by 2, not drivable at 2048 by 0.1 km. The bridges are 23 more objects of 47 in a 200 km
network. Nothing was done about the 19 bridges without room; they show again as ordinary road, and the build's warning
names each.

### 7.7 Dips (2026-11-13)

A dip is a run where the planned road stands more than 3 m above the original ground. Each is bridged when a model
fits (a chain of models over a long dip), else filled when dry, else left and reported as `road_over_water`. When a
dip gets a bridge, the roads are placed a second time with the spans cut in; the dips that are left are not bridged
again. Measured with `road-analysis 1024` and `2048` (the example calls the same `plan_roads` as the build and prints
the `DIPS` lines). The numbers below are of the first placement, which has the dips as the solver left them; the second
placement is at the end. **Bridges over dips have not been tested in the game.**

**Dips found, first placement:** 480 at 1024, 468 at 2048.

| Action | 1024 | 2048 | of which over water |
|---|---|---|---|
| bridge | 93 | 95 | 0 |
| fill | 383 | 369 | 0 |
| report | 4 | 4 | 4 |

**By road family** (1024; 2048 in brackets):

| Family | bridge | fill | report |
|---|---|---|---|
| `Asf1` | 7 (6) | 1 (8) | 0 |
| `Asf2` | 4 (6) | 9 (4) | 0 |
| `Asf3` | 23 (23) | 35 (38) | 0 |
| `City` | 19 (26) | 90 (69) | 1 |
| `Grav` | 19 (15) | 124 (126) | 2 |
| `Mud` | 21 (19) | 124 (124) | 1 |

**Length and depth of the runs** (minimum / median / p90 / maximum, in metres, 1024; 2048 in brackets for the median
and maximum):

| Group | length | depth |
|---|---|---|
| bridge | 16 / 32 / 96 / 268 (28.1, 271.9) | 3.2 / 4.7 / 7.8 / 17.0 (4.4, 17.2) |
| fill | 2 / 24 / 88 / 208 (21.9, 209.4) | 3.0 / 4.4 / 7.6 / 22.5 (4.1, 22.5) |
| report | 6.4 / 40 / 112 / 112 (37.5, 100.0) | 3.5 / 4.2 / 6.9 / 6.9 (4.3, 5.7) |

Most dips are shallow (median 4 to 5 m) and most of them are filled, not bridged: a fill is short (median 24 m, the
half of a 25 m model is 12.5 m) or too close to a junction or in a bend. The longest bridge is a chain of 11 models.

**`road_over_water` (4, the same at both sizes)**, position in map metres and how far the road stands above the
ground (1024): (446, 2105) 4 m, (635, 4505) 3 m, (693, 2465) 4 m, (3498, 2663) 7 m. The stream lines of OpenStreetMap
make four of the 480 dips wet; none of them could take a bridge. (635, 4505) lies by the OSM bridge without room at
(635, 4508) of 7.6.

**Bridge objects by model** (before: the 47 of 7.6, 27 metal, 12 stone, 8 wooden):

| Model | 1024 after | 2048 after |
|---|---|---|
| `bridge_metal_25` | 93 | 90 |
| `bridge_stone_25` | 81 | 79 |
| `bridge_wood_25` | 71 (31 scaled, 1.18) | 60 (27 scaled, 1.18) |
| in all | 245 | 229 |

**After the second placement:** the placement finds 457 dips at 1024 and 458 at 2048, none to bridge: 453 / 454 to
fill and the same 4 to report. That is more than the 387 / 373 that were left after the 93 / 95 bridges: the second
placement finds some 70 to 80 new dips, probably the roads beside the bridges, whose ends now hold the ground height
and whose neighbours are solved again. I did not look at them one by one.

**Fill at the downhill edge** (share of the pieces with more than 2 m / more than 5 m of fill, 1024; section 6 was
25 / 7 % at 0 to 10 % and 38 / 19 % at 35 to 60 %, as the table at the top of section 6):

| Side slope | 1024 | 2048 |
|---|---|---|
| 0 to 10 % | 9 % / 2 % | 9 % / 1 % |
| 10 to 20 % | 17 % / 3 % | 17 % / 3 % |
| 20 to 35 % | 25 % / 6 % | 27 % / 6 % |
| 35 to 60 % | 33 % / 11 % | 35 % / 9 % |
| above 60 % | 41 % / 13 % | 41 % / 15 % |

This is the figure of the whole of 7.1 to 7.7, not of this step alone; I have no run of 7.6 with the same print.

**`earthworks_capped`:** 103 at 1024 and 93 at 2048 (7.6: 102 and 85).

**Wall time of the road placement** (placing, junction heights, profiles; one machine, release build): 1024: 146.9 s
with one round (the placement as it was before this step) and 273.2 s with dips (two rounds), 1.9 times. 2048: 267.3 s
with two rounds (I did not run it with one).

**All violations against the final figures of 7.6** (1024 before / after, 2048 before / after):

| Rule | 1024 before | 1024 after | 2048 before | 2048 after |
|---|---|---|---|---|
| `road_not_drivable` | 242 | 292 | 235 | 272 |
| not drivable | 5.7 km | 6.75 km | 5.2 km | 5.8 km |
| `road_grade` | 1617 | 1683 | 1405 | 1528 |
| `road_misses_terrain` | 4598 | 4750 | 4010 | 4192 |
| of these above 0.5 m | 1465 | 1571 | 1366 | 1481 |
| above 2 m | 140 | 176 | 135 | 190 |
| `road_conflict` | 746 | 758 | 1125 | 1125 |
| `path_too_steep` | 133 | 132 | 79 | 87 |
| `earthworks_capped` | 102 | 103 | 85 | 93 |
| `junction_too_steep` | 1 | 2 | 1 | 1 |
| `road_far_from_ground` | 0 | 0 | 0 | 4 |
| `road_over_water` | | 4 | | 4 |

**What got worse:** nearly everything except `road_conflict` and `path_too_steep`. `road_not_drivable` by 50 at 1024
(242 to 292, 1.05 km more road that cannot be driven) and by 37 at 2048; `road_grade` by 66 and 123; `road_misses_terrain`
by 152 and 182, the misses above 2 m by 36 and 55 (the largest miss is now 15.7 m at 1024 and 20.1 m at 2048); the
build takes about twice as long; and at 2048 four pieces are now `road_far_from_ground` (the highest embankment at 1024
is 38.7 m, at 2048 `max_fill_m` is 68.5 m and 1053 vertices are capped, which I did not compare with 7.6). The cause is
probably that the 245 bridges are decks level between two ground heights: the road beside a long span has to climb to
the deck or leave it within its grade limit, and a deck over a dip cannot follow the ground the way the road did, so
the piece at each end is steeper and its ground is shaped to a deck, not the other way round. I did not isolate this
(no run without the bridges over dips at the same sizes except for the time). What got better: 93 / 95 dips are bridged instead of embankments, and the share of pieces with fill over 2 m is
below the figures of section 6 in every band up to 35 % (not isolated to this step). The cause of the rise in the
other numbers is a guess: the decks are level between two ground heights, so the roads at their ends carry the
grade, as in 7.6; I did not check it.

### 7.8 The example through binarize at 2048 (2026-11-13)

`naves build` of the Berchtesgaden example at `terrain_size = 2048`, release binary, `NAVES_BINARIZE_TIMEOUT=3600`,
inside a headless sway, under `systemd-run --scope -p MemoryMax=32G`. No game was started.

- Exit 0 after 314.9 s for the whole build (binarize alone is not timed separately in the logs); peak memory of the
  children 1776 MiB. The default timeout of 30 min was not needed. The signed PBO is 223 MB (223 308 017 bytes),
  the world file holds the three bridge models (`bridge_metal_25`, `bridge_stone_25`, `bridge_wood_25`) and road
  models of `dz\structures\roads\parts`.
- binarize logged "Cannot open object" 104 times (one per model name, the road and bridge models under
  `dz\structures\roads`, which are not on `P:`): known, not a failure. The 1024 run gave 185 s, 1.7 GiB, 78 MB.
- `build/report.json`: 21 127 pieces in 2735 stretches (2367 closed, 368 open); `road_misses_terrain` 4192,
  `road_grade` 1528, `road_conflict` 1125, `join_without_junction` 741, `road_gap` 368, `road_not_drivable` 272,
  `earthworks_capped` 93, `path_too_steep` 87, `road_in_water` 24, `tunnel_omitted` 21, `road_far_from_ground` 4,
  `road_over_water` 4, `junction_too_steep` 1. 19 OSM bridges got "no room for a bridge model".
  The count of bridge objects over dips was not taken from this build.
- Done criteria of section 9: no unreported piece above its hard limit (met, `road_far_from_ground` 4 are
  reported); planned grade above 100 %: one piece (268 % against a limit of 6 %), not met; junctions further than
  3 m from the ground median: none (met, one `junction_too_steep` for a link).
- Not tested in the game: roads, bridges over dips and scaled bridges.

### 7.9 Bridges over dips only from 8 m (2026-11-14)

The maintainer first named 20 m and then left the value to the controller, who set the depth from which a dip gets a
bridge to 8 m on 2026-11-14 (`BRIDGE_DIP_M`). A run still counts as a dip from
3 m (`DIP_M`): below 8 m it is filled when dry and reported as `road_over_water` when a water line crosses it. The reason: at 3 m in 7.7 the bridged dips were 3.2 / 4.7 / 7.8 / 17.0 m deep (minimum / median / 90th percentile /
maximum), and a road crosses such a hollow on an embankment. Measured as in 7.7 with `road-analysis 1024` and `2048`, release build, one machine, after the
change. Nothing else changed in the code. **Bridges over dips have not been tested in the game.**

**Dips found, first placement:** 480 at 1024, 468 at 2048 (as in 7.7: the same roads are found; only the decision
differs).

| Action | 1024 | 2048 | of which over water |
|---|---|---|---|
| bridge | 8 (7.7: 93) | 9 (95) | 0 |
| fill | 468 (383) | 455 (369) | 0 |
| report | 4 (4) | 4 (4) | 4 |

The 8 and 9 bridged dips are 16 to 160 m (1024) and 25 to 159 m (2048) long, median 72 m and 69 m; their depth is
8.8 / 9.6 / 10.3 / 17.0 m (minimum / median / p90 / maximum, 1024) and 8.0 / 9.3 / 10.2 / 17.2 m (2048): the same
deepest dip as in 7.7. By family at 1024: 5 `Asf3`, 2 `City`, 1 `Grav` (2048: 5, 2, 2). The four `road_over_water`
are the same four places as in 7.7 (depths 4, 3, 4 and 7 m at 1024; 4, 3, 4 and 6 m at 2048), none deep enough for a
bridge.

**Bridge objects by model** (the 47 of 7.6 are the OSM bridges: 27 metal, 12 stone, 8 wooden; 7.7 in brackets):

| Model | 1024 | 2048 |
|---|---|---|
| `bridge_metal_25` | 34 (93) | 34 (90) |
| `bridge_stone_25` | 29 (81) | 30 (79) |
| `bridge_wood_25` | 9, 8 scaled by 1.18 (71, 31 scaled) | 12, 11 scaled (60, 27 scaled) |
| in all | 72 (245) | 76 (229) |

So the dips add about 25 bridge objects at 1024 and 29 at 2048 to the OSM bridges, instead of about 200.

**The roads are still placed twice** (the second placement is made whenever one dip gets a bridge): 286.3 s for the
placement with dips at 1024 and 285.7 s at 2048 (7.7: 273.2 s and 267.3 s; one round took 146.9 s at 1024). With 8 or 9
bridges the time is the same as with 95: the saving is not in the time. After the second placement the build finds 482 dips at 1024 and 470 at
2048 (7.7: 457 and 458), none to bridge, 478 / 466 to fill and the same 4 to report; the deepest fill is 28.3 m at 1024
and 36.7 m at 2048 (before the second placement 22.5 m).

**All violations** (1024 / 2048; 7.7 at 3 m in brackets; 7.6, before dips, in the last two columns):

| Rule | 1024 at 8 m | 2048 at 8 m | 7.7 (3 m) | 7.6 (no dips) |
|---|---|---|---|---|
| `road_not_drivable` | 246 | 235 | 292 / 272 | 242 / 235 |
| not drivable (km) | 5.64 | 5.05 | 6.75 / 5.8 | 5.7 / 5.2 |
| `road_grade` | 1640 | 1409 | 1683 / 1528 | 1617 / 1405 |
| `road_misses_terrain` | 4604 | 4015 | 4750 / 4192 | 4598 / 4010 |
| of these above 0.5 m | 1465 | 1358 | 1571 / 1481 | 1465 / 1366 |
| above 2 m | 136 | 135 | 176 / 190 | 140 / 135 |
| `road_conflict` | 749 | 1130 | 758 / 1125 | 746 / 1125 |
| `path_too_steep` | 133 | 78 | 132 / 87 | 133 / 79 |
| `earthworks_capped` | 105 | 93 | 103 / 93 | 102 / 85 |
| `junction_too_steep` | 2 | 1 | 2 / 1 | 1 / 1 |
| `road_far_from_ground` | 0 | 4 | 0 / 4 | 0 / 0 |
| `road_over_water` | 4 | 4 | 4 / 4 | |

Largest miss 8.19 m at 1024 and 20.11 m at 2048 (7.7: 15.7 and 20.1); highest embankment 38.1 m at 1024 (7.7: 38.7), at
2048 `max_fill_m` 68.9 m (68.5) with 1032 vertices capped (1053). The "not drivable" length is the `not_drivable_m` figure of `STATS`.

**What changed, in which direction:** against 7.7 every rule that got worse with 95 bridges went most of the way back:
`road_not_drivable` by 46 and 37 fewer, `road_grade` by 43 and 119 fewer, `road_misses_terrain` by 146 and 177 fewer, the
misses above 2 m by 40 and 55 fewer, the largest miss at 1024 by 7.5 m. `road_conflict` and `path_too_steep` are the
same within a few (749 against 758, 1130 against 1125). Against 7.6 (no bridges over dips) the figures are within a few
of each other: 4 more `road_not_drivable` at 1024 and none more at 2048, 23 and 4 more `road_grade`, 6 and 5 more misses,
and 1 more `junction_too_steep` at 1024; the 4 `road_far_from_ground` at 2048 are also in 7.7 and 7.6 did not have them.
That supports the guess of 7.7 that the decks of the many shallow bridges cost the neighbouring roads their grade. What
this step costs: about 460 dips of 3 to 8 m are filled, embankments, where 7.7 bridged 93 and 95 of them; the build time of the
roads is not lower.

## 8. A neighbourhood solver? (2026-11-14)

The question: would placing the roads of a whole neighbourhood jointly with a solver beat the current approach
(`roadfit` per line, `roadplace` through roads first, side roads closing on what is fixed), and is **optimizam
0.2.0** a sensible tool for it? Nothing in the code was changed for good; the measuring code (a reason tag on
every fit and every join, `BEAM_WIDTH` and `CLOSURE_CANDIDATES` read from the environment, a shift experiment) is
kept as a patch outside the repository. No game was started.

### 8.1 Baseline and wide beam

Berchtesgaden example, `terrain_size = 2048` (the project's value; 7.9 also measured 1024), release build, one
machine (24 threads), `road-analysis` as in 7.9. The unmodified tool gives, over the whole run: `road_gap` **343**,
`join_without_junction` **740**, placement 310.6 s for both rounds, 322 s wall (peak memory 1.3 GiB). The tool
prints no fit cost; the plan view is measured below by the distance of the pieces from the OSM line instead.

To compare cheaply, the three runs below stop at the second placement (the roads are placed twice, see 7.9); the
counts are those of the first round and agree with the full run (336 gaps against 343, 738 joins against 740).

| First placement round | `BEAM_WIDTH` 24, 300 candidates (now) | 240, 300 | 240, 3000 |
|---|---|---|---|
| Fits (stretch parts, 203.6 km) | 1810 | 1810 | 1810 |
| Fits that miss their goal by over 0.5 m (`road_gap`) | 336 | 337 | 337 |
| Sum of the misses | 1639 m | 1638 m | 1641 m |
| `join_without_junction` | 738 | 739 | 740 |
| Median over fits of the median distance piece to line | 0.482 m | 0.476 m | 0.482 m |
| Length-weighted mean of that distance | 0.81 m | 0.81 m | 0.83 m |
| Largest distance, p90 / p99 over fits | 8.3 / 17.1 m | 8.4 / 17.1 m | 8.4 / 17.9 m |
| Wall time to the second placement | 165 s | 354 s | 345 s |

**The search is not the weak point.** A ten times wider beam and ten times more closure candidates leave every
number where it was (one gap more, within noise) and cost twice the time. The misses and the line deviation come
from the model and the piece set, not from missing a better candidate. Search tuning is not worth doing.

### 8.2 Where the gaps and the missing junctions come from

Gaps by what the fit had to meet (first round, 24 / 300; "fits" are all fits of that kind, "gaps" those over
0.5 m, "over 5 m" the large part of them):

| Fit ends on | Fits | Gaps | Over 5 m | Median gap |
|---|---|---|---|---|
| nothing (free end) | 775 | 0 | | |
| nothing, but starts at a side road's port | 243 | 0 | | |
| the junction of a through road (goal on the OSM line) | 446 | 144 | 28 | 2.3 m |
| the junction of a side road, which starts at a port | 129 | 54 | 19 | 2.8 m |
| a port of a junction already placed (side road to port) | 123 | 89 | 34 | 4.1 m |
| a port of a junction, side road starting at a port | 61 | 40 | 14 | 3.5 m |
| a bridge | 33 | 9 | 0 | 2.5 m |

So 183 of the 336 gaps (54 %) are side roads that have to close against something already fixed, and 129 of them
(38 % of all) against a port that a through road decided; of the 123 plus 61 such fits, 70 % end in a gap, where
a free fit never does. 144 gaps (43 %) are through roads that do not hit the junction point on their own line.

`join_without_junction` (738), by the reason `roadplace` gives up on the junction piece:

| Reason | Joins |
|---|---|
| no T or X piece for the pair of families (Asf3 with City 117 + 7, Grav with Grav 67, Asf2 with City 63, Grav with Mud 52, City with Mud 45, City with Grav 27, Asf1 with Asf1 26, and the rest) | 454 |
| the junction piece would sit over 3 m from the crossing (the through road's fit ends too far from it) | 115 |
| the next junction would be less than 12.5 m from the last one | 79 |
| a crossing no chain passes through | 59 |
| an X with sides of different families, on one side, or three sides | 29 |
| beside a bridge | 2 |

61 % of the missing junctions are the piece set (the vanilla catalogue has no junction for that pair), which no
solver changes: the choice there is another piece (a T of a nearby family, or a plain butt joint that is
reported as now) and is a decision for the maintainer. 16 % are the through road missing the crossing, 11 % are
spacing.

**How much freedom a joint solve would have.** A rough upper bound of the main thing a joint solve adds, the
junction moving along its through road: each failing side road to port (89) and port to port (40) was refitted
with the port moved by up to 25 m along the through road (12 offsets, the fit and goal otherwise unchanged; not
checked against the through road's own fit, the other junctions on it, or the crossing it replaces):

| | Failing | Closed (under 0.5 m) | Gap more than halved | Median gap before / after |
|---|---|---|---|---|
| to port | 89 | 42 | 69 | 4.1 m / 0.6 m |
| port to port | 40 | 18 | 28 | 3.7 m / 0.8 m |

So at most 60 of 129 (47 %), 18 % of all gaps, could close by choosing the junction position better; the other 69
fail at any position (heading or distance that the arcs and straights cannot meet: piece set, or an OSM line that
cannot be driven with 10 m arcs). The same test for the 72 through-road approaches that miss by over 3 m (the
goal moved up to 6 m along the line): 11 come within 3 m, none within 0.5 m. The through road is not helped by a
joint solve in this sense.

### 8.3 optimizam

Checked on 2026-11-14 on crates.io, the crate file (read, built, not added to the workspace) and its repository
on GitHub.

| Question | Finding |
|---|---|
| What it is | `optimizam` 0.2.0 by Aaron Hillegass: LP, MILP (dual simplex, presolve, cuts, branch and bound), CP (lazy clause generation: `all_different`, `element`, `table`, intervals) and global nonlinear (spatial branch and bound, interior point). Library and CLI. It is what the maintainer described. |
| Age and use | First release 2026-11-09, 0.2.0 on 2026-11-11; 21 downloads; repository created 2026-11-04, 0 stars, 0 issues, one human author, some commits by Claude. Self-reported benchmarks (Apple M4 Max) put it level with HiGHS on small MILPs and behind on MIPLIB. No history to judge maintenance by. |
| Can it say this problem | Yes, as a selection problem, not as geometry: a set of candidate fits per road (from the beam search), binaries for the choice, `sum == 1`, pair variables for "this candidate of A meets that candidate of B", cost on the pairs. Continuous run lengths and the sine and cosine of the headings do not need its nonlinear engine: with the arcs fixed the end point is linear in the runs, and the rest are constants computed beforehand. A direct formulation (heading as a variable, `sin` in a constraint) would use the nonlinear engine and is hopeless here. |
| Determinism | `--threads=1` repeats exactly, by its own documentation; more than one thread (default up to 8) may return different, equally good optima. A time limit is the only limit (no node or work limit), so a run that hits it is not reproducible; the AGENTS.md rule "same input, same output" would need `threads(1)` and models small enough to finish. |
| Security | No `unsafe` in `src/`; no `build.rs`; dependencies of the library: `serde_json` only (already in our tree: `itoa`, `memchr`, `serde_core`, `zmij`); `clap` and `ctrlc` only with the default `cli` feature (use `--no-default-features`). About 159 `unwrap`/`expect`/`panic!` outside tests (`milp.rs` 30, `result.rs` 19, `nlp/global.rs` 13, `simplex/mod.rs` 12); its release profile sets `panic = "abort"`, which does not apply to a dependency. The models would be ours, not input, but a panic in the solver would still take down a build; it would have to be run behind `catch_unwind` or in a thread. No fuzzing seen. |
| Licence | MIT OR Apache-2.0: both on the `deny.toml` allow-list. |
| Build | Rust 1.88, edition 2024; builds with 1.99 on Linux in 15 s (library, no default features), 13 MB rlib. Pure Rust, no platform code in the library: no reason it fails on Windows, not tried. |
| Fitness for the review in AGENTS.md | Fails "maintenance" for now (a week old, one author, no track record) and cannot be shown to pass "security" without reading 39 000 lines. Not usable today without the maintainer's decision; the alternative that the old note rejected (MILP or CP-SAT as a C++ dependency) is also still out. |

**Does it scale to this problem?** A toy with the same structure and random costs (N junction sites with 10
candidate positions, one pair table of 30 % density per side road, tree plus extra edges, one thread; not our data):

| Sites | Positions | Side roads | Pair variables | Result | Time |
|---|---|---|---|---|---|
| 50 | 10 | 70 | 2 111 | optimal | 0.95 s |
| 150 | 10 | 210 | 6 357 | optimal | 12.7 s |
| 500 | 10 | 700 | 21 161 | infeasible (proved) | 53 s |
| 500 | 20 | 700 | 84 211 | no answer | 120 s limit |

Time grows faster than the size, so the solver has to be given **neighbourhoods**, not the map (the 493 junctions
of the example are a few tens of connected groups, usually small: not measured). For groups of a few tens of sites
it is milliseconds to seconds.

**The skill it ships** (`skills/optimizam/SKILL.md` in the repository, 14.9 KB; it is not in the crate, which
excludes `skills/`, `CLAUDE.md` and `tools/`; its `references/` are symlinks, so a plain copy of the file loses
them). It tells Claude to check that `optimizam` is on the `PATH`, to formulate a JSON problem file, to run
`optimizam --time=60 problem.json result.json` and to read the result; it needs only Bash and file writes, asks for
no network and no secrets, and has no hidden commands. Read in full on 2026-11-14: it is safe to install. It is
**useful for one-off modelling** (a user describing a scheduling or assignment problem), and not for this
project: our use would be a library call from Rust with a candidate table, not a JSON file made by an agent.
Not installed. (The repository also has a 85 KB `CLAUDE.md`, the author's working notes; not read.)

### 8.4 Verdict

- **Not now.** Wider beams do nothing; 61 % of the missing junctions are the piece set; and a joint solve can
  close at most about 18 % of the gaps (the side roads whose port can be moved), by a selection over candidate
  fits that a small local repair may already get.
- **optimizam is not a bad fit for that selection problem** (MILP and CP, pure Rust, one small dependency, a
  licence we accept), but it is a week old, has a time-limit-only stop and 159 panic sites, and the review of
  AGENTS.md would have to end in the maintainer's decision. If a solver is wanted later it is a candidate to
  benchmark on real neighbourhoods, next to a hand-written exact search (the neighbourhoods are small and the
  junction offsets are a handful of values, so enumeration may be enough and costs no dependency).
- **Cheapest next steps, in order:**
  1. The piece set: decide what a crossing with no junction piece gets (454 of 738 joins). This changes the
     largest number and needs no solver.
  2. A local repair in `roadplace` for side roads that fail to close: refit with the junction on the through road
     moved by a few offsets (the experiment above, steps of 6.25 m up to 25 m) and keep the best. It is a handful
     of extra `fit_line` calls only where `report_gap` fires (129 cases, about 1500 fits), needs no crate, and the
     experiment says up to 60 gaps. Its price is that the through road has to be refitted from the previous
     junction (the hierarchy is bent, not broken), and the junction must stay within reach of the OSM crossing.
  3. Only if gaps remain after 1 and 2: benchmark a neighbourhood selection solve (optimizam or exhaustive
     search) on the real groups, with `threads(1)` and a work limit.
- **Not measured:** the real size of the neighbourhoods (connected groups of junctions and side roads); whether a
  moved junction fits the through road's own fit; the share of the 69 unclosable side roads that is the OSM line
  itself; Windows builds; the fit cost of the beam search (the tool prints none, and the score is not stored).

Measurement code: `/var/tmp/naves-solver/instrumentation.patch` (not in the repository); run logs in the same
folder.

## 9. Roads that look right (2026-11-15)

Stage 2 of `docs/superpowers/specs/2026-11-15-road-look-design.md`. Every step is measured with `road-analysis` on
the Berchtesgaden example at `terrain_size = 2048` (release build, whole run), before and after.

### 9.1 Swapped X pieces

An X crossing whose through road and crossing road have no piece for that order of families takes the piece with
the roles swapped, turned by 90 degrees so that its through ports lie on the crossing arms (`roadplace::swapped`).
In the vanilla set this is `kr_x_asf1_city` for a street as through road crossed by an Asf1 road. T crossings are
never swapped.

| | Before | After |
|---|---|---|
| `join_without_junction` | 877 | 878 |
| `road_gap` | 388 | 388 |
| grade excesses | 1729 | 1729 |
| `road_conflict` (conflicts) | 1089 | 1091 |
| Wall time (analysis) | 367.6 s | 359.6 s |

No gain on this map. The one crossing that qualifies (street through, Asf1 across, at (4835, 7760)) is planned
with `kr_x_asf1_city`, with the model's ports lying symmetrically on the crossing arms, but the street's fit ends
3.28 m from the crossing, over the 3 m a junction piece may sit from it (`JUNCTION_TOLERANCE_M`, the 115 joins
"over 3 m" of 8.2), so it is dropped and stays a join, as before. The extra join and conflicts are knock-on
effects of the dropped plan (it takes part in the spacing and chain-order decisions). The rule is covered by
tests on the synthetic catalog (`an_x_crossing_uses_the_swapped_piece_when_only_that_exists`,
`a_t_crossing_is_never_swapped`).

### 9.2 The overlay rule (2026-11-15)

Where the game has no junction piece (the 454 joins of 8.2 and every join the spacing, the 3 m tolerance and the
bridges leave), one road lies on top of the other instead of the two meeting in a heap (`roadplace::overlay_at`).
The higher road is the one of the higher family rank (`roadnet::overlay_rank`: Asf1, Asf2, City, Asf3, Grav, Mud, then
the longer chain, then the lower stretch id). The placement stays hierarchical: only the lower road's end moves.

- A lower road that ends at a crossing stops under the higher road's edge: 2 m inside it, but not nearer than half
  the half width to its centre line (a street of 10 m: 3 m from the centre line). A start is placed exactly there; an
  arriving end cannot be steered that exactly (the pieces are 6.25 m and longer), so the cut is tried at the default
  depth and then at 1, 0.75, 0.5, 0.25 and 0.1 times the half width until the last piece ends between 0.1 m and the half width
  short of the crossing, measured along the road (a fit can overshoot its path; an end past the centre line counts as a
  miss), else the nearest miss is kept.
- A through road that is lower than both side roads of an X crossing is interrupted: two runs, each ending under one
  edge of the crossing road. A higher road that ends on a lower one that runs on is not cut; the lower road runs on.
- At the cut the lower road is held `OVERLAY_DROP_M` (5 cm) below the junction's height (`JunctionKind::Overlay`,
  `Junction::lower`, `profile::holds`); the higher road keeps the height. The level patch is the one of a join.
- A run left between two cuts that is shorter than half its width is not placed and a warning says so.
- `join_without_junction` stays only for crossings no through road passes (all roads end there).

| | Before | After |
|---|---|---|
| `join_without_junction` | 878 | 66 |
| `road_gap` | 388 | 392 |
| `road_conflict` (conflicts) | 1091 | 1077 |
| grade excesses (`road_grade`) | 1729 | 1677 |
| `road_misses_terrain` | 4812 | 4698 |
| `road_not_drivable` | 250 | 236 |
| `junction_too_steep` | 2 | 3 |
| Wall time (analysis) | 365.3 s | 533.9 s |

812 of the 878 joins became overlays; the 66 left are crossings where all roads end. The wall time is not
comparable: the second run shared the machine with builds and tests. The 4 extra gaps are probably knock-on effects of the
shortened side roads (a side road that ends under a road no longer starts at a place the neighbouring fit assumed).
The rule is covered by tests on the synthetic catalog (`a_mud_track_ends_under_the_edge_of_a_street`,
`a_crossing_without_piece_interrupts_the_lower_road`, `the_higher_road_ending_on_a_lower_one_lies_on_top`,
`equal_families_put_the_longer_road_on_top`, `the_lower_road_sits_five_centimetres_below_at_the_cut`,
`a_stub_shorter_than_half_the_width_is_dropped_and_reported`). The game check of the look comes after the last step
of stage 2.

### 9.3 Duplicate roads (2026-11-15)

Two roads along each other for at least 25 m, centre lines closer than half the narrower road's width and headings
within 15 degrees (a road and its reverse count as the same heading), are one road drawn twice in OpenStreetMap
(`roadnet::drop_duplicates`, before bridge widening and placement). Stretches are taken in the order of
`overlay_rank` and compared with what is left of the higher-ranked ones of other chains, at every metre; the lower road's
overlapping part is dropped and reported (`road_duplicate_dropped`, at its middle). Where it goes on, the kept stretch
is cut at the point where the lower road leaves (new node, `<id>` and `<id>/after<n>`) and the rest of the lower road
ends there, so that the overlay rule of 9.2 is what joins them. The cut pieces are named `<id>` and `<id>/rest<n>` (not
`/after<n>`, which the bridge cuts of the dips use). A bridge is compared only with bridges and a tunnel only with
tunnels, and is never cut: a bridge that runs along a kept one for 25 m is dropped whole.

| | Before | After |
|---|---|---|
| `road_duplicate_dropped` | - | 9 |
| `join_without_junction` | 66 | 66 |
| `road_gap` | 392 | 397 |
| `road_conflict` (conflicts) | 1077 | 1072 |
| grade excesses (`road_grade`) | 1677 | 1699 |
| `road_misses_terrain` | 4698 | 4676 |
| `road_not_drivable` | 236 | 236 |
| Wall time (analysis) | 528.9 s | 599.5 s |

The example has 9 dropped parts, 492 m in all. The five largest:

| Dropped (stretch) | Along (kept) | Length | At (map metres) |
|---|---|---|---|
| 206825784:428124318-1581812526 | 944576472:8745141196-8745141208 | 104 m | (7077, 7210) |
| 1458329776:13374507439-13374507449 | 258382705:2637465849-2637505139 | 88 m | (1645, 7140) |
| 715025603:21623809-6720613450 | 715025602:21623809-6720613451 | 62 m | (2835, 5999) |
| 275847468:2805118909-2805118944 | 275847447:2805118909-2805118944 | 45 m | (5474, 7396) |
| 39624158:474968784-474968794 | 39624159:474968784-474968794 | 43 m | (6161, 7055) |

The largest, 104 m at (7077, 7210), lies about 2981 m east and 3114 m north of the map centre; at the example's scale
of 0.5 (`real_extent_m` 16384 for an 8192 m map) that is roughly 5.96 km east and 6.23 km north of the area centre
47.5718 N, 12.9465 E, about 47.628 N, 13.026 E (assuming y grows to the north; not checked against the frame). The two
rows with the same two end nodes (715025603/715025602, 39624158/39624159, 275847468/275847447) are parallel ways
between the same two junctions, the classic OpenStreetMap duplicate.

A re-run after the bridge rule gives the same counts (the example has no duplicated bridge). The effect on the other counts is small and mixed (the wall times are not comparable, the second run shared the
machine with `make check`): the 9 dropped parts remove some pieces (`road_misses_terrain` 22 fewer), the cuts add 5
gaps and the profiles of the cut roads gain 22 grade excesses; none of it is more than 0.5 % of its count. The rule is
covered by tests (`two_roads_drawn_on_top_of_each_other_keep_the_higher_ranked`,
`a_parallel_road_further_than_half_the_width_is_kept`, `an_overlap_shorter_than_25_m_is_kept`,
`headings_more_than_15_degrees_apart_are_kept`, `the_lower_road_goes_on_after_the_overlap`,
`equal_family_and_length_drops_the_higher_stretch_id_every_time`).

### 9.4 Roundabouts on a tilted plane (2026-11-15)

A closed loop of `junction=roundabout|circular` stretches (`roadnet::rings`: every node of the loop has exactly two
ring stretches, one family, no bridge or tunnel) is placed as one closed chain of the family's arcs and straights
with no junction piece on it. The two ring arms at each of its nodes are always one through road, and the ring lies
on top of every road that joins it whatever the family, so the joining roads end under its edge by the overlay rule of
9.2 (`Placer::lies_over`). The ring has one more crossing than a chain of the same length: the node where it closes.

The ring and everything on it lie on **one plane** (`junctions::ring_plane`): a least-squares fit to the original
ground under the ring's band (centre line plus and minus half the width), sampled about 1 m apart, so that the work grows with the ring's length, the tilt cut to the grade ceiling of
the family. The junctions on the ring take their height from it instead of the ground median; every joint of the ring is
held at the plane (no relaxing to the aim grade, no settling), and the terrain under the band and one cell beside it is shaped to the
plane by one annular `LevelPatch` with a gradient and an inner radius (the ground inside a ring keeps its height) (`conform`: a vertex takes `height + gx dx + gy dy`) instead of one level
patch per junction. A ring whose radius is below the tightest arc of its family (`PieceCatalog::tightest_arc`: 10 m for
the vanilla set, the 60 degree arc) cannot be laid out and is a plain crossing (`roadnet::collapse_small_rings`, before
placement, in `generate::widened_graph`): the ring's stretches go and the roads that joined it meet at one node at the
ring's centre.

| | Before | After |
|---|---|---|
| `join_without_junction` | 66 | 65 |
| `road_gap` | 397 | 396 |
| `road_conflict` (conflicts) | 1072 | 1050 |
| grade excesses (`road_grade`) | 1699 | 1667 |
| `road_misses_terrain` | 4676 | 4661 |
| `road_not_drivable` | 236 | 227 |
| `junction_too_steep` | 3 | 3 |
| Wall time (analysis) | 544.3 s | 534.8 s |

Rings: OpenStreetMap has 6 closed roundabout loops in the plain graph of the example, and a seventh appears once
the bridge stretches of an OSM bridge that has no room for a model are ordinary roads again (`OSMBRIDGE no room`).
**Five of the seven are smaller than the 10 m tightest arc and became crossings; two were placed as rings.** The map is
at half scale, so a roundabout of 9 m radius is 4.6 m here: the radii of the seven are 4.6, 5.8, 6.8, 7.2, 8.1, 10.9
and 18.6 m (centre line, map metres). The two rings: (5308, 6768), Asf1, 10.9 m, tilt 6.0 % (the ceiling), and (6189, 7038),
Asf1, 18.6 m, tilt 1.9 %.

**Site 20** (OSM ways 257338907, 415138352, 422733023, at map (5944, 7070)) is a ring of 4.6 m radius: too small for any
arc, so it is a plain crossing of its joining roads at the centre, not a ring. The plane and the tilt there are
therefore not used; the benefit of the task on this map is on the two rings above and on the crossings of the five
small ones (no loop of cut ring stretches any more).

### 9.5 Gaps closed by moving the junction (2026-11-15)

Section 8.2 found that 54 % of the gaps are side roads that cannot close against a junction port a through road has
already fixed, and that moving the port up to 25 m along its through road closed 60 of 129 in a test. Task 8 builds
that into the placement (`roadplace.rs`: `Placer::repair_port`, `Placer::crossings`, `place_network`).

**How it works.** A placement is made as before. When a side road that ends at a junction port (`fixed_end`) misses
it by more than `GAP_TOLERATED_M` (0.5 m), the end of that road (the last fit, from the pose where it starts to the port)
is fitted again to the port the junction would have if it were 6.25 m further along its through road, then 6.25 m
back, 12.5 m further, 12.5 m back, and so on up to 25 m each way (`+` before `-`, in this order, for determinism). The
junction is taken at its place on the line of its through road with the entry at half its length before the crossing;
the placed junction follows that line to a fraction of a metre, which is why the trial is cheap: only the last fit of
the side road is made, not the through road again. The first shift that closes the road (gap at most 0.5 m) is noted
for the junction, else the one with the smallest gap if it is below the gap of the unmoved placement. After the
whole network is placed, **it is placed a second time with those shifts** (the through road is then refitted from its
previous junction as part of the ordinary placement, with the junction's goal and its tolerance circle moved along
the line), and the second network is kept only if it has fewer gaps (then a smaller sum) than the first; otherwise
the unmoved one is. A shift is dropped (the junction stays) where it would put the junction closer than one junction
length to another or less than a metre from the end of the run, or over a bridge, and a junction on a roundabout ring
is never shifted (the ring's joints lie on the ring plane). Overlay places (no junction piece) have no port, so they
are not in this repair; their gaps do not arise.

Why two placements and not a trial of the through road per candidate: the first version placed the whole network once
per candidate shift (all failing junctions at the same time, up to 8 rounds). Placement is the expensive part of the
analysis (518 s of 531 s, for two rounds with the dips), so that was not an option (it did not finish in 30 minutes).
A through road's refit cannot be tried locally either: it moves every later junction on it by a fraction of a metre and
with them the ports of their side roads, which are placed afterwards.

| | Before | After |
|---|---|---|
| `join_without_junction` | 65 | 65 |
| `road_gap` | 396 | 347 |
| `road_conflict` | 1050 | 1045 |
| `road_grade` (grade excesses) | 1667 | 1598 |
| `road_misses_terrain` | 4661 | 4614 |
| `road_not_drivable` | 227 | 217 |
| `junction_too_steep` | 3 | 5 |
| Placement with dips (2 rounds, wall) | 518.3 s | 1492.0 s |

The time triples (fix round 1: up to two extra placements per call): a second placement of the network costs as much as the first, and the trials of the side roads' ends
are cheap beside it. Memoising the fits that a shift does not change would cut it only where the through road and its side
roads upstream of the shifted junction are untouched; a shift changes every later fit on its through road by a fraction of a
metre, so this was not tried.

Where the figures come from: the example is placed twice (two rounds with the dips). In the first round the
unmoved placement had 389 gaps, the repair found a shift for 115 junctions and the placement with them had 341 (-48);
in the second round 396, 116 shifts and 347 (all shifts, before the check of the acceptance below). The `road_gap`
of the table is the analysis at its end (the second round's placement), 396 to 347. The grade and terrain counts fall as a side effect: a road that closes needs no
sharp bend to reach its port. `junction_too_steep` rose by two, because a moved junction sits on other ground.

**Acceptance (fix round 1).** The first version kept the moved network when it had fewer gaps, which could trade a
through road's gap, or a junction piece (a through fit missing a shifted junction by over 3 m drops it, and its
side roads then have no port to miss), for side gaps. Now the second placement is kept only if no junction piece is lost
and no join without a junction is added as well, and a through road's gap before a *shifted* junction must not grow (beyond
0.5 m): the shifts where it did are removed and the network is placed once more (two extra placements in all; the
unmoved network if the last still fails the check). A first version checked every junction and removed the shifts of the
whole chain; that rejected every move (the unshifted junctions after a move change by fractions of a metre), so it
is per shifted junction. In the example's first round 20 of the 115 shifts grew the through gap and the network with
all 115 had 5 junction pieces fewer (539 against 544): the loss the second check rules out. Without those 20 the
network has 95 shifts, 341 gaps (389 unmoved) and 547 junction pieces. A moved
junction is also checked against both neighbours at their final places (12.5 m), not only the one before. The
second placement's error is recorded in the warnings of the network (`naves-core` has no `tracing`
dependency).

**The through roads that miss their own junction point (43 % of the gaps).** Before building on it, the same shifts
were tried for the through roads (the fit to the junction goal, with the goal moved 6.25 to 25 m along the line, the
fit and the pose it starts from unchanged, and the shifts checked only against the end of the run). Of 879 through
fits that missed by more than 0.5 m in the four placements of the analysis (about 220 per placement), 604 (69 %) came
under 0.5 m at some shift and 797 (91 %) had their gap halved. **It was not applied.** The result is not the one that
section 8.2 expected (none within 0.5 m for a 6 m move), but it measures the wrong thing: a gap is the distance
to the goal, and a goal that is moved along the line is met more often only because the fit gets to choose where its
target is. The junction would then sit up to 25 m from the OpenStreetMap crossing where the through road has no
problem with it, its side roads would have to be built from a port that is that far from where OpenStreetMap has them
(and could fail to close, which the side-road repair above would have to repair again), and nothing keeps the
junction near the crossing. A real use needs the side roads of that junction in the test, as the repair of the side
roads has; it is the next step if the through gaps are worth it (about 220 per placement before this task).

**Left over.** Gaps that no shift closes (the remaining 347) are those where the heading or
distance cannot be met by arcs and straights at all (the other 69 of 129 in 8.2), where the through road was shifted
but the side road starts at a port too (`fixed_start`; only the end port is repaired), and gaps against bridges.
The existing height repair in `junctions::repair` (limit `MAX_MOVE_M`, 3 m) is a different thing (it moves heights, not
positions) and was not touched; it still does not exclude ring junctions, which the review of task 7 noted.

### 9.6 Bridges joined to their road (2026-11-15)

**Cause (written before the change).** `place_bridge` fitted the road before the deck to a fixed goal at the start of
the bridge link, heading along the chord of the link, and then laid the deck models from the pose where the fit
*ended*, not from the goal. The fit closes the position (the length of its straight runs is solved for it) but not the
heading: arcs come in fixed angles, so the heading at the end is off by what the arcs cannot make, up to a degree or
two. The deck was laid along that heading, so:

- the deck left its OSM line by `length * sin(error)`: on a synthetic river crossing (bend of 40 m sideways in the 100 m
  before a 100 m bridge, `bent_river_crossing` in `tests/roadplace.rs`) the approach ended at heading 88.0 degrees
  instead of 90.0, and the deck ended at (499.94, 543.49) for a line at y = 540: **3.5 m sideways** at the far end;
- the road behind the deck started from that deck end, so it began 3.5 m off its line and had to bend back (the next
  pieces: heading 98, 97, 89.5 degrees) or, with a junction behind the bridge, missed its fixed goal (a `road_gap`);
- the deck was never wrong against its own approach (it started at the approach's end pose), so a check of
  "approach pose equals deck port" passes; what failed was the deck against its line. The visible result is the
  sideways offset and the kink the spec describes.

Heights are not part of the cause: in the profile a deck is one variable, its two end joints are the end joints of
the neighbouring pieces, so the approach meets the deck at the deck's height by construction
(`the_deck_end_heights_equal_the_approach_heights` passes before and after).

**Choice.** Both options were weighed against this cause. Closing on the deck's port as a fixed end alone cannot
work: the fit already tries and ends 2 degrees off, since the arcs have no such angle. Moving the deck along its line
changes which port the approach has to reach, and a different position needs different arc/straight combinations, so one
usually closes. Both are combined, which is the minimal change: the deck is laid on its line (the heading of the
chord, never the fit's heading), the approach is fitted to the deck's start as a fixed end, and when it does not end
within `GAP_CLOSED_M` and `KINK_TOLERATED_DEG` the deck slides along its line (`BRIDGE_SLIDE_STEP_M` 2.5 m, four steps each way,
`+` before `-`, only forward as far as the next junction or the end of the run leaves room) and the fit is tried again.
The first slide that closes is used; if none does, the best fit is used and the deck starts where the approach ended (as
before), with the gap reported.

**Result at 2048** (same example, unchanged inputs; the "before" was re-measured on the commit before the change and
agrees with 9.5):

| | Before | After (slide up to 10 m) | After (slide up to 2.5 m) |
|---|---|---|---|
| `join_without_junction` | 65 | 65 | 65 |
| `road_gap` | 347 | 344 | 348 |
| `road_conflict` | 1045 | 1046 | 1051 |
| `road_grade` | 1598 | 1674 | 1691 |
| `road_misses_terrain` | 4614 | 4615 | 4629 |
| `road_not_drivable` | 217 | 230 | 236 |
| terrain misses over 0.5 m at a bridge end | 82 | 106 | 104 |
| wall (analysis) | 1545 s | 1585 s | 1550 s |

**What the numbers say, honestly.** The geometry is fixed (the synthetic crossing: deck on its line within 0.5 m instead
of 3.5 m off, both approaches within 0.1 m and 1 degree of the deck's ports), but on the real map the aggregate counts
do not improve: gaps move by 3, and grade, not-drivable and the misses at bridge ends grow (+76, +13, +24 for the
10 m slide). A shorter slide is worse, not better, so the growth does not come from sliding. What differs from before
is that the deck now lies on the OSM line where it used to follow the approach's end heading, and the roads behind a deck
start on the line; bridge ends over a dip (the second placement) are measured against the terrain there. I did not find
which of these is the cause of the growth in the time available (each analysis takes 26 minutes); it needs the
bridge sites looked at in the game or a per-site diff of the two analyses. Left for the review: whether the growth
is the deck now being where the OSM line is (and the terrain shaping at the abutments following the old position), or noise
from a changed placement.

**The cause of the growth (round 2, measured per bridge).** Each run of the analysis was repeated with a dump of every
violation (rule, position) and of every placed deck (stretch, ends, heading); a violation counts for a bridge when it lies
within 60 m of one of its deck ends, in either run. Findings:

- **The slide moved most decks by metres.** With 2.5 m steps the fit rarely closes at the deck's own place, so 28 of the
  65 OpenStreetMap decks moved: 10 by about 2.5 m, 18 by 5 to 10 m. When no slide closed, the best of the nine fits was
  kept, which is a slid one too. A bridge line is grown to the model length so that its ends sit on the banks; a deck
  slid by 10 m has one end over the gap and the other 10 m onto the ground beyond (deck ends up to 10 m past the end of
  their line in the dump; e.g. `23458920` start ground 290.8 → 287.4 m, `1467990031` 325.2 → 322.2 m, ground grade of
  the approach 30 → 44 %).
- **The road after a moved deck is a new fit, and that alone changes the counts.** Within 60 m of the decks the counts
  changed by grade +27, not drivable +10; the other +49 grade came from roads further along the chains (and from the
  second placement deciding a different dip bridge: `90238829…/bridge0` only before, `169999496…/bridge0` only after).
  Single sites went both ways: `299428041` +11, `130657211` +9, `419057266` +8, `495868617` +7; `830320923` −8,
  `23631314` −7, `23458060` −5, `39605281` −5.
- **The counts are below their own noise here.** A null run, the code before Task 9 with every deck moved 5 cm along its
  heading (nothing a player could see), gives road_grade 1628 (+30), road_not_drivable 227 (+10),
  road_misses_terrain 4657 (+43), road_conflict 1053 (+8), road_gap 348, join_without_junction 65. A 9 cm change of
  two decks at (2339, 6541), heading unchanged, re-fits the road behind them and adds 5 grade findings 40 m further on;
  a 0.27 m slide at `1299522645` adds 5. A difference of ±3 cannot be told from a re-rolled placement.
- **One systematic case: a road that turns into the bridge.** At the dip bridge `343477741…/bridge0` (four decks on a
  curve) the deck used to follow the road's heading, 18 degrees off the chord; on the chord the approach has to turn
  those 18 degrees right before the deck (grade 5 → 16 there). The synthetic `bridge_on_a_curve` shows the same: on a
  20 degree arc the fit to the deck's place arrives 3.5 degrees off the chord and is closed only by a tight last piece
  and a 1.5 m slide.

**Change.** Slides of 0.25 m, at most 2.5 m each way, the shortest first (`BRIDGE_SLIDE_STEP_M`, `BRIDGE_SLIDE_STEPS`):
on the synthetic crossing of 9.6 a 0.5 m slide closes where 2.5 m steps needed 2.5 m. When no slide closes, the deck stays
at its place and starts where the fit to that place ends, as before Task 9 (no "best slid" fit). When that fit arrives
more than `BRIDGE_TURN_MAX_DEG` (2.5 degrees) off the chord, no slide is tried: the road turns into the bridge, and the
deck follows the road. Tests: `a_deck_stays_where_its_line_puts_it` (bends of 4 to 60 m; the 8 m bend slid the deck
10 m before) and `a_deck_on_a_curve_keeps_its_place_and_follows_its_road` (20 degree arc; slid 1.5 m before).

**Result at 2048:**

| | Before Task 9 | b8f801e (slide ≤ 10 m) | 0.25 m steps ≤ 2.5 m | + turn limit (committed) | Null run (before, decks +5 cm) |
|---|---|---|---|---|---|
| `join_without_junction` | 65 | 65 | 65 | 65 | 65 |
| `road_gap` | 347 | 344 | 348 | 348 | 348 |
| `road_conflict` | 1045 | 1046 | 1051 | 1052 | 1053 |
| `road_grade` | 1598 | 1674 | 1638 | 1628 | 1628 |
| `road_misses_terrain` | 4614 | 4615 | 4597 | 4588 | 4657 |
| `road_not_drivable` | 217 | 230 | 225 | 224 | 227 |

The committed version is at the null run or better on every count; against the single "before" run it is worse by 30
grade, 7 not drivable and 7 conflict findings, which is what moving any deck by centimetres does. With it, 8 of the 75
deck runs (OpenStreetMap and dip bridges) slide, all by at most 2.5 m, the rest keep their place; far from the bridges
(more than 60 m) grade findings are 1404 against 1412 before. Left open: whether the criterion "no worse than before ±3" should be measured against a few
null runs instead of one, and the junction pair at (4183, 6660), where the 2.5 m slide of `302681746…/bridge0` changes the
start of the next dip bridge and its approach turns 18 degrees (grade 5 → 16).

### 9.7 Clearance under bridges over a road (2026-11-15)

**Question.** Bridges that cross another road without a shared node (spec 2.4): do they keep 4.5 m between the
surface of the road below and the underside of the deck? Before the change nothing looked at it; the deck followed the
ground at its abutments and the road below followed the ground, so a bridge over a road in a flat place lay in the road.

**Finding the crossings.** `roadnet::over_crossings`: stretch pairs whose lines cross, with no shared start or end
node, where one is a bridge (the upper one; of two bridges the higher `layer`) and neither is a tunnel. Berchtesgaden
at 2048 has **4** such crossings in the placed graph (the spec counted 20 road crossings, all tagged bridge or tunnel;
most of those are tunnels, share a node, or are cut out as duplicates before this point): (4689, 7321), (7995, 7320),
(2347, 6546) and (5101, 7913); the first two are the sites 51 (Silbergstrasse) and 50 (Kehlsteinstrasse) of the game
check.

**Deck thickness.** Measured per bridge model on its own Geometry LOD (resolution 10^13 exactly): the distance from
the deck surface (the mean height of the four memory points) down to the lowest vertex within 4 m below it
(`MAX_DECK_THICKNESS_M`; what hangs lower is a pier or an abutment, not the deck; the Geometry LODs of the game's
bridges have 230 to 600 vertices, the stone arch reaches down to the 4 m cap, the metal truss about 1.7 m). Reading
these LODs needed the compressed arrays of the ODOL format (LZO1X from 1024 bytes, `lzokay-native`, decompression
only): the first version of this change read the first geometry-type LOD that could be read, which failed silently on
the real Geometry LOD (its vertex array is compressed) and took a later small LOD (the roadway) at deck level, hence
about 0 m for the stone bridge. A model whose Geometry LOD cannot be read takes the thickest deck measured among the
models of its type (the word after `bridge_`: stone, wood, metal), else the thickest of all (a build warning names the
model and the value); if no bridge model can be read, the clearance is not checked and the build says so. The
thickness is scaled with the bridge's scale.

**Change.**

- `solve_clearing` (`profile.rs`): the deck at the crossing gets a lower bound of ground + 4.5 + thickness (the ground
  median of the crossing, the lower road being at the ground), only on the deck of the crossing's own upper stretch.
  The bound goes (a) into the profile of the deck's chain: a deck variable that comes out lower is held at it and the
  chain solved again, so the approaches rise within their limits, and (b) into the junctions: from the deck along its
  chain, every junction a joint is held to gets the bound less `grade_ceiling * distance` (`junctions::raise_floors`),
  which goes on along the links to the neighbours and lets `repair` lift a junction beyond the usual 3 m from the
  ground, but never more than 10 m (`MAX_RAISE_M`).
- A junction that the chains of the road below are held to is never raised, and carries no bound on (`frozen`); the
  road below keeps its height. This is checked on every solve: the road below is compared with the same solve without
  the lift; where it moved by more than 5 cm, that crossing is dropped from the lift and the profiles are solved again.
- Settling (the step that brings roads that run close to one height) leaves roads alone within 50 m of a crossing and
  the road below within 250 m of it. Without the second part, the road below was moved by settling against the lifted
  approach that runs along it: at Kehlsteinstrasse by up to 1.35 m over 280 m (measured: 1.35 m with settling, 0.00 m
  without), which dropped the crossing from the lift.
- The profiles are solved, the clearance measured on the solved strips (the bridge's and the lower road's own stretches,
  not what lies nearest), and what falls short is added to the bound; at most eight solves (the shortfall is closed by
  about half per round: 4.05, 4.33, 4.43 m at Kehlsteinstrasse). A bridge that still clears by less than 4.45 m is the
  finding `bridge_clearance` (message: reached and wanted).

**Deck slide of Task 9.** `upper_s` is only reported; the clearance is found by position (nearest point of the
bridge's and the lower road's own placed stretches to the crossing) and the bounds by the joints of the deck's chain, so
the deck's slide of at most 2.5 m changes nothing but where the crossing lies on the deck.

**Result at 2048** (clearance from the surface of the road below to the underside of the deck, real thicknesses):

| Crossing | Before | After |
|---|---|---|
| Kehlsteinstrasse over a road, (7995, 7320), site 50 | -4.02 m | **4.47 m** (within the 5 cm tolerance) |
| Silbergstrasse over a road, (4689, 7321), site 51 | -0.17 m | 0.02 m (finding) |
| (2347, 6546) | 1.77 m | 4.61 m |
| (5101, 7913) | -0.90 m | 4.50 m |

Counts: `bridge_clearance` 1; over-crossings 4. The six counts after the change (noise of the null run: grade up to
about 30, conflict about 10):

| | After Task 9 | After this change |
|---|---|---|
| `road_grade` | 1628 | 1645 |
| `road_not_drivable` | 224 | 237 |
| `road_misses_terrain` | 4588 | 4621 |
| `road_gap` | 348 | 347 |
| `road_conflict` | 1052 | 1072 |
| `join_without_junction` | 65 | 65 |

**Silbergstrasse (why it stays short, measured).** The deck is held by the junction piece at its start: junction 384
holds the joints 32 to 34 of the bridge's chain (rule `Piece`: the end of the junction piece, the first bridge joint
and the deck's variable), at 320.86 m, and the same junction holds the first joints of the road below (rules `End` and
`Inward`: the residential street ends in this junction). The crossing lies 4 m from the deck's start, in the junction.
The road below is at 319.15 m at the crossing, the deck at 320.86 m, 1.71 m above it, minus the deck's thickness 1.69 m
= 0.02 m. Lifting the junction would lift the road below with it, so it is frozen, the deck's variable is held by it
and cannot rise, and the finding stays. It is the data (the bridge starts in the junction of the road it crosses),
not the solve; the way out is a junction piece away from the crossing (the junction shift of Task 8 does not move it
because it is no gap).

**Not done.** The terrain under the bridge is shaped for the lower road as before (no new check of the cut); untagged
crossings and tunnels are out of scope (railroad design); the visual underside of an arch is not read (the Geometry
LOD is a collision body).

### 9.8 What makes the humps on steep roads (2026-11-15)

**Question.** Spec 2.5: `road_grade` (1729 at the start of the plan) and `road_not_drivable` (250) on steep stretches,
and the "ski jump" of the sites 21 and 23 of the game check. Does the hump come from vertical bends at fixed junction
heights, from short pieces between patches, from the terrain shaping, or from something else? This section only
diagnoses; nothing in the product code changed.

**Method.** `road-analysis` at `terrain_size = 2048` on the state after Task 10 (HEAD `78df272`), with temporary
instrumentation (kept outside the repository as a patch, `/var/tmp/naves-roadlook/t11/hump-dump.patch`) that dumps, for
every strip, each joint (planned height, original ground, finished terrain, the junction hold with its rule and
height), each piece (kind, run between its joints, model, start and end pose, the check's grade, cross fall and state)
and every `road_grade` excess of the profile and every `road_not_drivable` run. Two runs (the second added the model and
poses) give the same joints, pieces and findings (same checksum). Counts of that state: `road_grade` **1645** (411
grade excesses and 1234 vertical-bend excesses, one per joint, so a hump gives several), `road_not_drivable` **237**
runs of pieces. Each finding was matched to its joint and classified from the dump (script
`/var/tmp/naves-roadlook/t11/cons.py`). Counts are exact for this run; a different run of the same code moves every
deck a few centimetres and `road_grade` by about 30, so the classes are what counts, not single numbers. The test for
"too close for the rise" is a model, not a result of the solve: between two held heights `h1`, `h2` that lie `L`
metres apart (an end is *flat* when a run of two or more joints is held to one junction: the junction piece and its
patch), the shortest road with the family's minimum vertical radius `R` and 0.9 of its drive grade `c` is
`L = sqrt(2 n R dh)` (`n` flat ends, if the peak grade stays under `c`, else `dh / c + n R c / 2`); a span shorter than
that cannot be driven within the limits whatever the solver does, **with the heights held as they are**.

**Result: the 1645 `road_grade` findings by cause.**

| Class | Bends | Grades | Total | What it is |
|---|---|---|---|---|
| H2 held heights too close for the rise | 561 | 164 | **725** | Two junctions (level patches) so close that the road cannot change height between them within the minimum radius. Span between the two held runs: median 14 m, 10 to 90 percent 4 to 44 m; height difference median 2.3 m (0.5 to 7.9 m); 370 of the 561 bends are on `City` roads (R 150 m). The road has about 46 percent of the length it would need (median). |
| H3 held heights far enough apart | 261 | 77 | **338** | The same places, but the two heights are far enough apart for a curve; the solve does not use the room. Height to move to clear the bend: median 0.20 m, 90 percent 1.4 m (max 4.9 m). |
| H1 hold on one side only | 31 | 33 | **64** | A junction height on one side and a free chain end (dead end, end cap) on the other; the blend (`bound_spans`) needs two held ends. These include the three worst: 540 percent (2635, 6118), 187 percent (2454, 6431), 95 percent (7736, 8168). |
| F1 free joints, ground bends or is steeper than the limit | 253 | 68 | **321** | No hold nearby. 206 of the 321 have a piece of under 1 m at the joint (see F2/overlap below); bends clear with a move of median 0 cm, 90 percent 9 cm. The 68 grades are on ground that is itself steeper than the limit (Mud, Grav, City), 38 at a chain end. |
| F2 free joints, short pieces | 93 | 0 | **93** | No hold, straight ground, a piece under 12.5 m next to the joint; 72 of them under 1 m. Clears with 0 to 8 cm. |
| F3 free joints, other | 6 | 50 | **56** | Unexplained by these measures (grade excess on free joints with ground within the limit); not looked at one by one. |
| D bridge deck (and its ends) | 29 | 19 | **48** | At or next to a deck, 12 bends and 6 grades have no joint (the deck's single variable). |
| | **1234** | **411** | **1645** | |

The 50 worst grade excesses (grades from 30 to 540 percent): H2 15, H1 11, H3 6, F1 7, F3 4, deck 7. The earlier
finding (section 7.2: 640 of 1180 bends at a held joint, 220 next to one) holds: at HEAD 602 bends are at a held joint
and 251 next to one, **853 of 1234 (69 percent)**; of the grades 274 of 411. What was missing then is the cause there.

**Overlapped 6 m straights (F1, F2, part of the others).** 604 pieces of the chains have a run between their joints of
under 1 m (median 0.28 m, 6 of them at the end of a chain). All 604 are the 6.25 m straight of their family (`city_6`
236, `asf3_6` 118, `mud_6` 89, `asf1_6` 71, `asf2_6` 63, `grav_6` 27) placed over the end of the piece before it: its
start is a median 6.0 m before the previous piece's end (the "straights that overlap to make a length" of section 1; 1382
of 21 990 pieces start more than 0.5 m from the previous end). The profile has a joint at the end of each piece, so such
a piece is a run of 0.2 m between two joints, which the checks measure as it is (`max(0.5 m)`): a 5 cm difference of
height there is a 10 percent grade and a bend of 0.05 per metre, hundreds of times the limit. 350 of the 1234 bends have
such a run next to the joint (276 of the 351 free ones), and the move that clears them is under 5 cm in 333 of the 350.
**This is a hump of centimetres in the planned heights, but two pieces lie over each other there at different
heights**, which the game shows as a step or z-fighting, not as a ski jump. How the export sets the height and pitch
of an overlapped piece was not checked here.

**Not drivable (237 runs, 424 pieces).** 207 are cross fall only (grade within its limit, fall across the road over
20 percent): not humps, the side-slope problem of section 1, terrain shaping. 30 are grade humps: 15 at a held
joint (13 plus 2 beside a deck; planned grade over the hard limit, e.g. 95 percent over 4.4 m at (7736, 8168), planned 67 percent
at (5825, 5806), 46 percent at (5204, 5108), planned 535 percent at (2635, 6118)), 14 where the plan is within the
limit and the finished terrain is steeper in a 12.5 m window (11 plus 3 beside a deck: the earthworks of ground at 50
percent and more, e.g. Grav at (7316, 6280), base ground 51 percent, plan 18 percent, finished 79 percent) and 1 on
free joints. So terrain shaping is a small part of the grade humps (14 runs of 237), and not a cause of `road_grade`
at all, which counts the plan.

**The six sites of the game check** (Checkpoint A numbers; the plan has changed the roads since, so the measures are
those of HEAD at the same place):

| Site | Where | What the data shows | Class |
|---|---|---|---|
| 21 end cap | (7198, 6574) | Asf3 climbing a steady 17 to 18 percent (aim 12) for 65 m to the cap; no bend and no `road_grade` finding within 45 m. The photograph shows the cap detached from the last piece with a dark band between. | Not a profile excess: the end cap at the end of a steep road; look at its placement in the game check |
| 23 wooden bridge | (6101, 3040) | Mud road at 21 to 23 percent into a flat 37 m deck (562.41 m), whose far end is 3.5 m above the ground, then 15 percent down. The grade jumps from 21 percent to 0 at the deck start and from 0 to -15 percent at its end: the sag and the crest of the ski jump. The excess finder measures the change over the middle of the deck piece (about 25 m), 0.006 to 0.009 per metre, under the 0.0167 of `Mud`. | D |
| 36 `road_grade` | (2454, 6431) | Chain of two pieces: held at 401.60 m at junction 167, the free end 13 m under the ground (389.88 m) | H1 |
| 37 `road_grade` | (6743, 7377) | Two junctions 17.4 m apart with 2.1 m between their heights (25 m needed); behind the second, the free end drops 4 m in 6.2 m | H2 (and H1) |
| 42 `road_not_drivable` | (7736, 8168) | Free joints at the ground (295.18 m), then the hold of junction 1340 at 291.04 m, 3.2 m under the ground: 4.1 m down in 4.4 m (95 percent); 22 m of free road before it would take it at 19 percent | H1 |
| 43 `road_not_drivable` | (5204, 5108) | Grav planned at 36 to 46 percent on ground of 18 to 39 percent, 1.8 m over the ground at the top and held 3 m under it at the junction arm at the bottom (359.71 m); 64 percent over 13 m | H1 (a hold on one side) with steep ground |

Bridges (not only those two sites): 75 decks, 52 with a grade step of over 1/R across the approach piece alone, 30 with
an approach grade of 10 percent or more, 21 of 15 percent or more, up to 99 percent at (7682, 6073). The bend of a deck
is measured over the deck's length, which hides most of this.

**What the joint height solve (the plan that follows) would do with each class, and what needs its own fix now.**

| Class | Joint solve (one QP over all joints) | Own fix now (Task 12)? |
|---|---|---|
| H3 (338) | removes it: the room exists, the junction heights and the free joints are variables | no |
| F1, F2 (414) | removes what is not an artefact of overlapped pieces; the rest are cm | partly: see overlap |
| H1 (64) | removes it (a hold on one side, free joints to blend over; the QP has no "two held ends" condition) | optional: the three worst (540, 187, 95 percent) make hard-to-drive runs now |
| H2 (725) | **does not remove it**: the equality of the joints of a junction makes the patch level, and a level patch on a slope with the minimum radius needs `2 sqrt(R dh)` of road: with the junctions 14 m apart and 2.3 m of ground between them, no height assignment on the ground works; the QP trades the radius against the ground distance and leaves the road under or above the ground by metres (`road_misses_terrain`, earthworks) or breaks the bound | **yes** |
| Overlap (604 pieces, about 380 findings) | does not: the variables are the joints, and 0.2 m runs stay in it | **yes** |
| D (48, and 75 decks) | partly: a flat deck is one variable; the joint solve can bend the approach but not the deck, and the check must measure the bend at the deck ends | yes, with the check |

**Proposed fixes (Task 12 candidates, by size of effect).**

1. **Junction pieces follow the through road's grade instead of lying level (H2, up to 725 of 1645).** Hold the height
   of a junction only at its centre (or the crossing point on the through road's axis) and let the held joints of the
   through road, and the side roads' ports at the edge of the through road, lie on the through road's profile; the
   level terrain patch becomes a plane that follows the through road (as the ring plane of 9.4 does). If the junction
   joints were free of the flatness, the span between the two junctions would be a straight line of at most 0.9 times
   the drive grade in 518 of the 561 bends and 133 of the 164 grades; the rest (43 and 31) have junctions too close
   for any grade. Risk: a junction model pitched along a slope (the game's junction pieces are placed pitched?
   untested in the game); the side road's first piece must meet the through surface at its edge; the terrain patch and
   `junction_too_steep` change; it changes the junction height rules (`Piece`, `Inward`, `Arm`, `End`, `Patch`) and the
   heights of the level patch used by the earthworks. Needs the game check on the sites 28, 29, 37.
2. **The overlapped 6.25 m straights (604 pieces, about 380 findings).** Treat joints less than about 1 m apart as one
   variable (the profile has a `MIN_STENCIL_M` for the smoothing already, not for the limits or the excess finder), and
   give the overlapped piece the pose and pitch of the surface it overlaps; check how the export sets the height of
   such a piece. Effect: removes the F2 class and most of F1 (about 280 findings), and the centimetre steps between
   overlapped pieces. Risk: low for the check, medium for the export (the pose of the overlapped piece).
3. **A hold on one side only (H1, 64, including the three worst).** Extend `bound_spans` so that a span from a held
   joint to a free chain end blends away from the hold at the grade limit and the radius (a line of the limit's slope
   from the held height backwards over the free joints). Effect: the three pieces over 90 percent and most of the 15
   grade runs of `road_not_drivable` at a held joint. Risk: low; it moves free joints only. The joint solve does this too; do it now
   only if the joint solve is not near.
4. **Bridge decks (D, 48; 75 decks).** Measure the bend at the deck ends over the approach piece (not the middle of
   the deck) and let the approach lead into the deck over its own pieces; where an approach of over 10 percent meets a
   flat deck, either lengthen the ramp on the approach (the deck's variable is held) or pitch the deck along the road
   (a pitched bridge model; not tried in the game). Effect: the ski jump of site 23 and 30 decks with an approach of 10
   percent or more. Risk: medium (decks are held by clearance and junctions; the check changes `road_grade` upwards by
   the decks that were hidden: expect up to about 50 new findings).

H3 (338), F1 and F2 without the overlap, and the grade humps on steep ground of `road_not_drivable` (14 runs where the
plan is within the limit and the earthworks make it steeper) are for the joint solve and the terrain shaping of the
plan after this one; the 207 cross-fall runs are not humps. F3 (56) was not explained: grade excesses on free joints
whose ground is within the limit, to be looked at when the others are gone.

**Limits of this diagnosis.** The classes use the held heights as the solve left them: that H2 is "infeasible" is the
model above (radius and grade ceiling per family, flat ends where two joints are held to one junction), not a
different solve that could have shown a way; the second half of the table (what the joint solve does) is expectation,
not a run. The classes are exclusive and ordered (deck, then hold, then free); a hump at a deck end next to a junction
counts as a hold. Site 21 is not explained by the data. Nothing was driven or photographed in the game for this step.

**Result of fix 1 (junctions follow the through road's slope, 2026-11-15/16).** First implemented as proposed
(`Junction::gradient`, held joints and terrain patches on the tilted plane, links measured between the held joints;
tilt from two ground samples 6.25 m, or the patch radius, either side of the junction point along the through road's
heading, at most 0.9 of its grade ceiling; commit `1034db4`). That made things worse. Instrumented runs of both states
(the dump of `/var/tmp/naves-roadlook/t11/hump-dump.patch`, plus every junction with its tilt and every finding;
scripts in `/var/tmp/naves-roadlook/t12/an/`) found two causes:

- **The tilt did not match the grade the road takes.** The two samples were taken at the junction's point, which is
  the crossing node or the end of the side road and lies up to 9 m (90th percentile 2.5 m) off the through road's
  centre line, and they measured the ground over 12.5 m only. At a T piece at (1321, 7252) the point was 4 m beside the
  axis on a hillside: tilt 12.3 percent, ground along the axis 1.3 percent, and the road was held up to 3.4 m under the
  ground. Against the ground over 25 m either side, 203 of 505 tilted junction pieces got a tilt further from it than a
  level piece would have been; around those the bends rose, around the others they fell, so the H2 findings that went
  away (270 of 725) were paid for elsewhere.
- **Steep tilts break the roads that meet the through road.** The plane's fall along the through road is the fall
  across every road that crosses or ends on it there, and a road has no fall across of its own (its strip is level
  across in the terrain shaping): the two surfaces agree only along a line, and differ by the tilt times the half
  width of the meeting road. The new `road_conflict` findings next to tilted junctions differ by a median 3.5 m times
  the tilt (0.40 to 0.60 m at tilts of 12 to 20 percent); `road_misses_terrain` (the step between the two cores inside
  a piece's footprint) and "other road overlaps, different height" (1589 to 2000 pieces) rose with them. Findings rose
  almost only around junctions tilted by 8 percent or more; up to 3 percent they fell.

The fix (round 1): the tilt is the least-squares slope of the original ground sampled every metre **along the through
road's own line**, 25 m (or the patch radius) either side of the junction's station on it, and is limited to
`CROSS_FALL_AIM` (8 percent) as well as 0.9 of the grade ceiling (which is never the lower limit). Variants measured
at 2048 (same runs, deterministic): fit along the line over 12.5 m alone 1649 / 4903 / 1165 (grade / misses /
conflict); the 8 percent limit alone 1570 / 4773 / 1089; both, 12.5 m, 1545 / 4689 / 1070; both, 25 m, **1493 / 4612 /
1041**; a 4 percent limit 1627 / 4555 / 1039; a 6 percent limit 1599 / 4622 / 1055; a tilted terrain patch under
junction pieces too changed nothing (1545 / 4660 / 1096).

| Finding | Before (`78df272`) | `1034db4` | Round 1 |
|---|---|---|---|
| `road_grade` | 1645 (1234 bends, 411 grades) | 1732 (1299, 433) | **1493** (1132 bends, 361 grades) |
| `road_not_drivable` | 237 | 256 | 206 |
| `road_misses_terrain` | 4621 | 5025 | 4612 |
| `road_gap` | 347 | 346 | 347 |
| `road_conflict` | 1072 | 1186 | 1041 |
| `join_without_junction` | 65 | 65 | 65 |
| `junction_too_steep` | 7 | 6 | 7 |

Logs `/var/tmp/naves-roadlook/t12/ra-before.log`, `ra-after.log`, `ra-final.log`. `road_grade` falls by 152, well
beyond the noise of about 30; the others stay level or fall. The classes of 9.8, recounted with the same model (which
still counts a tilted run of held joints as a flat end, so it overstates H2 after the fix): H2 725 to 537, H3 338 to
380, H1 64 to 68, F1 to F3 470 to 464, D 48 to 44. What is left: around tilted junction pieces `road_misses_terrain` still rises (by about 100, made up by falls
around overlays and joins), the side roads' inward joints stay level across the through road, so H2 between a side
road's two junctions is untouched (most of the 455 H2 findings that remained after `1034db4` have an `Inward` hold
next to them), and a junction steeper than 8 percent keeps a bend at its ends. Not checked in the game.

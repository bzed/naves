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

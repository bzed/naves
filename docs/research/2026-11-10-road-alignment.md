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

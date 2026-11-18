<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# A step becomes a ramp

Status: design, 2026-11-18. Written by the assistant on the maintainer's rule of the same day; no approval gate.
Decisions the maintainer made are marked **(maintainer)**; everything else is the assistant's call and one constant
each.

## Why

**(maintainer)** "Short drivable ramps are always okay. Just no sudden very short changes on steepness, it still
needs to be drivable."

The height solve does the opposite next to a junction or a bridge end. At way 74208396 (Berchtesgaden, map
(6742, 7388)) a level deck ends at 319.71 m, 5.7 m from a junction whose plane is at 320.58 m. Closing the 0.76 m
needs a ramp of 16 to 20 %, well under the 27 % an asphalt road may have. The solve leaves a step instead
(`bridge_end_step`), for two reasons:

1. The crest-and-dip rule asks for a vertical radius of 400 m on this road class (60 to 600 m by family, an estimate
   for comfort at speed, never driven in the game). The change of grade a ramp needs at the deck's end is about four
   times what that allows, and its excess costs 12.5 times as much per unit as the slack of the junction hold.
2. The road has no height joint where it enters the junction's patch. The hold is a pin on the line between the
   deck's end and a joint 17 m on, so keeping it would lift that far joint about 2.4 m over the junction: a hump,
   not a ramp.

## Goal

Where a road has to change height over a short length next to a junction or a bridge end, it does so with a ramp
inside its family's grade limit, entered and left by changes of grade a vehicle takes at low speed. A step is left
only where even that is not possible, and is then reported as today.

## Non-goals

- The limits themselves are not measured here. Driving a test track in the game is its own task; these values stay
  estimates until then.
- Pitched bridge decks: a deck stays level.
- The comfort radius on open road is not changed.
- Heights on a centre line sampled at a fixed step (the larger rework) are not done here; this design adds joints
  only where a ramp needs them.

## Approaches

1. **Only change the weights** (a kept hold is worth more than a bend). Without a joint at the patch the hold
   then makes the hump described above. Rejected on its own.
2. **Pitch the deck.** It removes the step at 74208396 but is a different decision (how decks look, panel grids on a
   slope) and does not help a deck between two junctions at the same height difference. Not here.
3. **A joint where the road enters the patch, and a ramp rule next to junctions and deck ends (chosen).**

## Design

### A joint at the patch

Where a run of a road passes through the patch of a junction that names it and has no height joint inside the
patch, the solve gets a height variable for that road at the point where it enters the patch (and one where it
leaves, if it does). The junction's hold applies to these like to any joint in a patch (`Patch`). This replaces the
pin on the interpolated line (`Crossing`, added for runs that touch a deck) where a joint can be added; the pin
stays for the cases where it cannot.

**The pin at a deck end (fix round 1).** A deck's end whose junction's patch starts within `RAMP_JOINT_MIN_M` of it
along the run has no room for a joint between them, and a deck's own joint holds only one level. The deck's end is
then tied to that junction's plane where the run enters the patch (a `Crossing` at the patch's edge, a hard row),
even where the junction already holds another joint of the road (first version: the row was skipped, nothing in the
solve saw the step at 74208396). `bridge_end_step` measures the step at the same place. The deck or the junction
gives way: being off its ground costs a junction far less than a step costs (there is no threshold; the junction
gives way by as much as the row needs).

- It applies to every run, not only to runs that touch a deck, if the first measurement shows no harm; otherwise it
  is limited to runs that touch a deck and that is said.
- A new joint closer than `RAMP_JOINT_MIN_M` (4.0 m, one terrain cell at size 2048) to an existing joint is not
  added; the existing joint is used.
- What is built from joint heights (the ground under the road, the pieces' heights, the findings) reads the new
  joints like any other. The implementer checks first how each consumer takes a joint that is not a piece end, and
  stops with a report if one cannot.

### The ramp zone

Within `RAMP_ZONE_M` (25.0 m, one long road piece) of a junction's patch or of a bridge deck's end, measured along
the road, the rule for a change of grade is not the comfort radius:

- the change of grade at one joint may be up to `GRADE_BREAK_MAX` (0.10, about 5.7°);
- the limit used is the larger of this and what the family's radius allows over the run.

Outside the zone the rules do not change (the heights of roads near it do: the solve is one calculation).

What "within `RAMP_ZONE_M` of a junction's patch" means in the code: the zone is measured from every variable a
junction holds, not from the patch's edge. A road joint held by the `Arm` rule lies up to 13 m from the junction's
centre, and a junction holds up to eight joints inward, so the zone reaches about 25 m beyond a joint that is itself
up to 13 m off. At Berchtesgaden (2048) that is about 36 % of all road segments.

A change of grade above `GRADE_BREAK_HARD` (0.20) at one joint in a zone costs like a hard limit does outside a zone
(a second band, priced as a hold that is given up). Between 0.10 and 0.20 a break is a finding that a hold or a
clearance may cost; above 0.20 it is not given for them lightly. Without that band the break was the cheapest outlet
in an impossible case: breaks of 27 to 34 % at one joint appeared.

The value is an estimate: a car with ordinary ground clearance takes a 10 % break at walking speed without
grounding; two such breaks at least `RAMP_JOINT_MIN_M` apart give a 20 % ramp. **Known limit:** the solve adds no
joint between a level deck and the ramp, so a ramp steeper than `GRADE_BREAK_MAX` right at a deck's end is one break
and always a `road_grade` finding. It is one constant and will be
measured in the game.

### What gives way first

Next to a junction or a deck end the order is: keep the grade ceiling; keep the junction hold and the deck-end
join; keep the grade break under `GRADE_BREAK_MAX`; keep to the ground. A hold given up costs more than a grade break
over its limit of the same size in metres at the joint. The weights are set so, with one named constant per weight
and a test for the order.

### Findings

- `bridge_end_step` and the given-up-hold report stay as they are; they now mean "not even a ramp was possible".
- A grade break over `GRADE_BREAK_MAX` in a ramp zone is reported by `road_grade` with its own wording ("The grade
  changes by 14 % at one point, the most a vehicle takes there is 10 %"), not as a vertical radius.

### Constants

| name | value | meaning |
|---|---|---|
| `RAMP_ZONE_M` | 25.0 | how far from a junction's patch or a deck's end the ramp rule applies |
| `GRADE_BREAK_MAX` | 0.10 | the largest change of grade at one joint in a ramp zone (rise over run) |
| `GRADE_BREAK_HARD` | 0.20 | a change of grade over this at one joint in a ramp zone costs like a hold given up (an estimate, like `GRADE_BREAK_MAX`, twice it, about 11.3 degrees; not driven in the game) |
| `RAMP_JOINT_MIN_M` | 4.0 | new joints are not added closer than this to an existing one |

## How it is judged

Berchtesgaden at terrain size 2048 against the baseline on `main` at da88a23
(`/var/tmp/naves-clean/baseline/`): road_not_drivable 185, road_misses_terrain 4285, road_grade 4, road_gap 356,
road_kink 21, road_conflict 899, bridge_clearance 1, bridge_blocks_road 3, bridge_buried 6, bridge_end_step 3,
bridge_post_floats 0, join_without_junction 65.

- `bridge_end_step`: 74208396 gone or under 0.5 m; the other two (130047131 0.5 m, 39663890 0.6 m) named with what
  happened.
- No finding count worse without a reason that can be named; in particular `road_grade`, `road_not_drivable` and
  `road_misses_terrain`. Every finding that appears or disappears is listed.
- The largest grade and the largest grade break inside ramp zones, before and after.
- A game check of 74208396 and of the ten sites where heights moved most.

### Results (Berchtesgaden, terrain size 2048, the commit of the fix round)

| rule | `main` (da88a23) | ramps first version | ramps with the fix round |
|---|---|---|---|
| road_not_drivable | 185 | 182 | 182 |
| road_misses_terrain | 4285 | 4180 | 4175 |
| path_too_steep | 174 | 172 | 172 |
| road_grade | 4 | 11 | 11 |
| road_conflict | 899 | 848 | 845 |
| bridge_buried | 6 | 9 | 8 |
| bridge_end_step | 3 | 4 | 3 |
| all others | equal | equal | equal |

`bridge_end_step`: 74208396 is not gone but changed. The deck and the junction it faces (patch 0.7 m from the deck's
end) now meet (deck 320.12 to 320.39 m, the plane at the port 320.90 to 320.39 m, step 0.00 m); a step of 0.56 m
remains against the next junction (6764, 7378), 22 m on, measured where the road runs closest to it: that junction
holds one joint of the road outside its patch (the `Arm` rule), and the road's slope through the patch differs from the
tilt of the junction's plane. That is how any road passing a junction is held, not a matter of deck ends; it is left
as a follow-up. The
junction at the deck's far end is 0.79 m above its ground (0.52 m before). 39605281 (a new one in the first version)
is gone: step 0.00 m. 130047131 (0.52 m) and 39663890 (0.63 m) are chain ends and unchanged.

Findings: against the first version only `road_misses_terrain` (3 appear, 4 disappear), `road_conflict` (3 gone),
`bridge_buried` (one gone) and `bridge_end_step` (39605281 gone) differ. 157 of 26376 joints moved by more than 0.1 m
against the first version, none by more than 0.5 m (largest 0.38 m, at (5038, 6589)). Breaks of the grade over 10 %
near a junction or a deck end (a measure that is the same for any version, not the solver's zone): 35, one over
20 % (0.208 at (2356, 6551), a vertical-bend finding that was already there; the steepest grade in a zone is 0.450
at (6840, 6479), as before). The three vertical bends at (5302, 6761), (5308, 6778) and (5311, 6758) that are no
longer reported have the same heights as before; the zone limit now covers them, nothing was fixed. The zone
covers 9457 of 26376 segments (36 %).

Not done: a game check of 74208396 and of the sites where heights moved most.

<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Road alignment fixes — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Roads are judged by what a real car manages, lie on the ground where they meet each other, keep their grade limits at junctions, bridges and tight bends, and steep hillsides get earthworks that reach the ground.

**Architecture:** All vehicle limits move into one module, `naves_core::limits`, with real-car values. The profile solver gets a grade limit per segment and a vertical radius instead of a change per joint; on top of that sit the junction, meeting, arc and "aim gives way" rules. The terrain shaping takes its cut and fill slopes from the natural ground. Every task is measured on the Berchtesgaden example with one diagnostic.

**Tech Stack:** Rust 1.95, the existing crates `naves-core` and `naves-export`; `proptest` for the profile properties. No new dependency.

**Spec:** `docs/research/2026-11-10-road-alignment.md` (findings, numbers and the derivation of the limits). Background: `docs/superpowers/plans/2026-11-06-naves-roads.md` (Tasks 8 and 10), `AGENTS.md`.

## Global Constraints

- `AGENTS.md` applies in full. In particular: test first (red, then green); behaviour is tested through the public API in `crates/naves-core/tests/`; no `unwrap`, `expect`, `panic!`, unchecked indexing or truncating `as` in library code; lints are silenced only with `#[expect(lint, reason = "...")]`.
- Same input, same output: no `HashMap` iteration in results, fixed round counts, `proptest` regression files stay in the repo.
- Each commit passes `make check`, total coverage stays at or above 85 %, and carries its tests, its line in `CHANGELOG.md` under `## [Unreleased]` and its change to `docs/manual/` where behaviour a user sees changes.
- New files need an SPDX header (`SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>`, `AGPL-3.0-or-later`).
- A change to a text in the GUI starts with the `/frontend-design:frontend-design` skill (`AGENTS.md`, *User interface*).
- The limits are estimates from real cars, **not measured in the game**. Every constant's doc comment says so and points to section 3 of the spec. Task 1 of the roads plan (drive ramps) replaces them later.
- **The measurement.** `cargo run --release --locked -p naves-export --example road-analysis` in the repository root needs the example's data (`examples/berchtesgaden/README.md`) and the DayZ tools; it takes about 3 minutes. Tasks that say "measure" run it before and after and put these lines of both runs into the commit message: `total`, `not drivable because`, `cross fall > 5 %`, `cross fall > 20 %`, `miss > 0.15 m`, `miss > 1.00 m`, `junctions:`, `pieces with another part`, `vertices changed`, `capped earthworks`, `conflicts:`. The baseline is section 1 and 4 of the spec. A task whose measurement makes the figure it is about worse is not done: stop and report.

## Decisions for the maintainer

These are my calls; say so before the task starts if you want them different.

1. **Hard grade limits** 0.30 paved, 0.22 gravel, 0.18 track (now 0.35, 0.30, 0.28). **Aims** `grav` 0.15 and `mud` 0.15 (now 0.18 and 0.25); the others stay.
2. **Cross fall:** aim 0.08, hard limit 0.20 (now one limit, 0.05).
3. **Cuttings may be steeper than a player can walk** (up to 63°) where the natural hillside is steep. Embankments stay at or below 45°, so the downhill side of a road can always be left on foot. Today's code keeps both below `WALK_LIMIT_DEG`.
4. **Junction pieces are no longer level along their length.** They carry up to 6 % like any road near a meeting. This rests on the hypothesis that the engine drapes road pieces on the terrain, which is still not tested in the game.

## Not in this plan

- The 332 stretch ends that miss what they have to meet (spec section 1). The cause is not known; it needs its own investigation.
- Wider hairpins (25 m arcs instead of 10 m) and walls as objects between hairpin legs. Task 9 lets legs differ by a step; what is left after it decides whether the fit has to change.
- The earthworks follow an arc's chord (off by at most 1.34 m; inside the level strip at 6.25 m and 3.125 m cells).
- The 1353 joints far off the ground for neither known reason (spec 2.6).
- `render::WALK_LIMIT_DEG` (60°) and `checks::WALK_LIMIT_DEG` (50°) disagree. Not caused by this work.

## Review Focus

1. **A chain where every variable is fixed** (a short side road with both ends on other roads, Task 6): no panic, no NaN, heights are the pinned ones. Test in Task 6.
2. **Pieces that overlap or are very short** (straights overlap by up to 6.25 m, so the distance between two joints can be near zero): the radius limit must not divide by it. Test in Task 4.
3. **An arc whose radius is not larger than half the road's width** (a 12 m `asf1` on a 10 m arc has an inner radius of 4 m; a model with less must not give a zero or negative limit). Test in Task 5.
4. **Level ground, the map's edge and a heightmap with one row of roads** in the slope-dependent earthworks: the natural slope is zero or read at the border, and the result is exactly today's. Test in Task 8.
5. **Determinism of the ladder** (Task 7): two runs give identical profiles, and a chain that needed a looser limit does not change the chains solved before it. Test in Task 7.

---

### Task 1: The findings and the diagnostic in the repository

**Files:**
- Modify: `crates/naves-export/examples/road-analysis.rs` (exists, untracked)
- Add as they are: `docs/research/2026-11-10-road-alignment.md`, `docs/superpowers/plans/2026-11-10-naves-road-alignment.md`

**Interfaces:**
- Produces: the measurement command of *Global Constraints*.

- [ ] **Step 1: Make the example pass `make lint`.** Replace "NOT FOR COMMITMENT" in the header comment with a sentence that it is the measurement of the spec and needs the example's data and the tools. Add one file-level `#![expect(..., reason = "a measuring tool for developers; failing loudly is the point")]` that lists exactly the lints `make lint` reports for this file (an unfulfilled expectation is an error too), and remove the two `let _ =` leftovers. Do not change what it prints.
- [ ] **Step 2: Verify.** Run `make lint` and `make reuse`: both pass. Run the measurement: the `total` line reads `total 202.6 km: {"Fine": 156599.7, "NotDrivable": 27500.4, "Steep": 18483.9}`.
- [ ] **Step 3: Commit.** `docs: road alignment findings, the plan and their measuring tool`. Leave `render-crossfall.rs` untracked.

### Task 2: One module for the vehicle limits, with real-car values

**Files:**
- Create: `crates/naves-core/src/limits.rs`, `crates/naves-core/tests/limits.rs`
- Modify: `crates/naves-core/src/lib.rs`, `checks.rs`, `profile.rs`, `validate.rs`, `crates/naves-core/tests/checks.rs`, `tests/profile.rs`, `tests/validate.rs`, `crates/naves/src/gui/levels/roads.rs`, `crates/naves-export/examples/road-analysis.rs`, `docs/formats/road-objects.md`, `docs/manual/src/gui.md`, `docs/manual/src/troubleshooting.md`, `CHANGELOG.md`

**Interfaces:**
- Produces, in `naves_core::limits`:
  - `pub fn grade_aim(family: Family) -> f64`: Asf1 0.08, Asf2 0.10, Asf3 0.12, City 0.14, Grav 0.15, Mud 0.15.
  - `pub fn drive_grade(family: Family) -> f64`: Asf1, Asf2, Asf3, City 0.30; Grav 0.22; Mud 0.18.
  - `pub fn grade_ceiling(family: Family) -> f64`: `0.9 * drive_grade(family)`.
  - `pub fn min_vertical_radius(family: Family) -> f64` in metres: Asf1 600, Asf2 400, Asf3 250, City 150, Grav 100, Mud 60 (used from Task 4).
  - `pub const CROSS_FALL_AIM: f64 = 0.08; pub const CROSS_FALL_LIMIT: f64 = 0.20;`
- Removes: `profile::max_grade` (callers use `grade_aim`), `checks::DRIVE_GRADE_PAVED`, `DRIVE_GRADE_GRAVEL`, `DRIVE_GRADE_TRACK`, `checks::MAX_CROSS_FALL`. `profile::max_grade_change` stays until Task 4. `checks::WALK_LIMIT_DEG` and `MAX_MISS_M` stay where they are.
- `DriveState` keeps its variants. `Steep` now means: over the aim along **or across**, within what a vehicle manages. Update its doc comment.

- [ ] **Step 1: Write the failing tests.**

In `tests/limits.rs`:

```rust
#[test]
fn every_aim_is_below_its_ceiling_and_the_ceiling_below_the_limit() {
    for family in [Family::Asf1, Family::Asf2, Family::Asf3, Family::City, Family::Grav, Family::Mud] {
        assert!(grade_aim(family) < grade_ceiling(family), "{family:?}");
        assert!(grade_ceiling(family) < drive_grade(family), "{family:?}");
        assert!(min_vertical_radius(family) >= 60.0, "{family:?}");
    }
    assert!(CROSS_FALL_AIM < CROSS_FALL_LIMIT);
}

#[test]
fn the_limits_are_the_documented_ones() {
    // docs/research/2026-11-10-road-alignment.md, section 3
    assert_eq!([drive_grade(Family::Asf3), drive_grade(Family::Grav), drive_grade(Family::Mud)].map(|v| (v * 100.0).round()), [30.0, 22.0, 18.0]);
    assert_eq!([grade_aim(Family::Grav), grade_aim(Family::Mud)].map(|v| (v * 100.0).round()), [15.0, 15.0]);
}
```

In `tests/checks.rs`, with the helpers that are there (`terrain`, `chain`, `ramp`, `states`): replace `cross_fall_makes_a_road_not_drivable` by

```rust
#[test]
fn cross_fall_over_the_aim_is_steep_and_still_drivable() {
    let hm = terrain(100, 6.25, |x, _| 100.0 - 0.12 * x);
    let net = chain(Family::Asf3, true, (300.0, 100.0), (300.0, 400.0));
    assert!(states(&net, &hm).iter().all(|s| *s == DriveState::Steep));
}

#[test]
fn cross_fall_over_the_limit_is_not_drivable() {
    let hm = terrain(100, 6.25, |x, _| 100.0 - 0.24 * x);
    let net = chain(Family::Asf3, true, (300.0, 100.0), (300.0, 400.0));
    assert!(states(&net, &hm).iter().all(|s| *s == DriveState::NotDrivable));
}

#[test]
fn gravel_and_tracks_have_their_own_lower_limits() {
    for (family, grade, state) in [
        (Family::Grav, 0.20, DriveState::Steep),
        (Family::Grav, 0.25, DriveState::NotDrivable),
        (Family::Mud, 0.16, DriveState::Steep),
        (Family::Mud, 0.20, DriveState::NotDrivable),
        (Family::Asf3, 0.27, DriveState::Steep),
        (Family::Asf3, 0.33, DriveState::NotDrivable),
    ] {
        let net = chain(family, true, (300.0, 100.0), (300.0, 400.0));
        assert!(states(&net, &ramp(grade)).iter().all(|s| *s == state), "{family:?} at {grade}");
    }
}
```

In `tests/validate.rs`: a not-drivable run with a cross fall of 0.12 and a grade under the limit gives **no** `Rule::RoadNotDrivable`; with 0.24 it gives one whose message ends in `(and 24 % across)`.

- [ ] **Step 2: Run them and see them fail.** `cargo test -p naves-core --test limits --test checks --test validate`: the new tests fail to compile or fail.
- [ ] **Step 3: Implement.** `limits.rs` holds the functions and constants above; each doc comment gives the value's source in one sentence and the words "an estimate from real cars, not measured in the game: `docs/research/2026-11-10-road-alignment.md`, section 3". In `checks::check_roads_on_terrain` a drivable piece is `NotDrivable` if `grade > drive_grade(family)` or `cross_fall > CROSS_FALL_LIMIT`, else `Steep` if `grade > grade_aim(family)` or `cross_fall > CROSS_FALL_AIM`, else `Fine`. `validate::not_drivable` adds "(and N % across)" only above `CROSS_FALL_LIMIT`. Adapt the other callers and the existing tests that name the removed items (same assertions, new names; `a_road_at_the_limit_is_drivable` and its neighbours take their grades from `drive_grade`).
- [ ] **Step 4: The texts.** In `roads.rs` the label for `Steep` becomes "Yes, steeper or more tilted than aimed" (run the frontend-design skill first, as `AGENTS.md` asks). `docs/manual/src/gui.md`: amber means steeper or more tilted than the aim. `docs/manual/src/troubleshooting.md`: what "Not drivable" means now, with the three grade limits and the 20 % across. `docs/formats/road-objects.md`, part 2: the limits are estimates from real cars until the ramps are driven, with a link to the spec. `CHANGELOG.md`, under *Changed*: one line with the old and new limits.
- [ ] **Step 5: Verify and measure.** `make check` passes. Measure: `cross fall > 20 %` is unchanged at 5.80 km; `not drivable because` shows a cross-fall part of at most 5.8 km.
- [ ] **Step 6: Commit.** `feat(core): road limits from what real cars manage, cross fall with an aim and a limit`.

### Task 3: The check follows an arc, not its chord

**Files:**
- Modify: `crates/naves-core/src/checks.rs`, `crates/naves-core/tests/checks.rs`, `CHANGELOG.md`

**Interfaces:**
- Consumes: `PlacedRoad::start` and `end` (`Pose { x, y, heading }`, radians clockwise from north; the direction of travel is `(sin heading, cos heading)`).
- Produces: no new public item. `PieceCheck::grade` and `cross_fall` of an arc are measured along the arc and at right angles to it.

- [ ] **Step 1: Write the failing test.** A helper `arc(family, centre: (f64, f64), radius: f64, from_deg: f64, turn_deg: f64) -> PlacedNetwork` builds one `PlacedKind::Piece` whose `start` and `end` poses lie on the circle with the tangent headings.

```rust
#[test]
fn an_arc_on_a_spiral_ramp_has_no_cross_fall() {
    // a spiral ramp: 10 % along a circle of radius 10 m, level along every radius
    let centre = (300.0, 300.0);
    let hm = terrain(200, 3.125, |x, y| {
        100.0 + 0.10 * 10.0 * (x - centre.0).atan2(y - centre.1)
    });
    let net = arc(Family::Asf3, centre, 10.0, 20.0, 60.0);
    let check = &check_roads_on_terrain(&net, &hm)[0];
    assert!(check.cross_fall < 0.02, "{}", check.cross_fall);
    assert!((check.grade - 0.10).abs() < 0.02, "{}", check.grade);
}

#[test]
fn a_straight_is_measured_as_before() {
    let hm = terrain(100, 6.25, |x, y| 100.0 + 0.07 * y - 0.03 * x);
    let net = chain(Family::Asf3, true, (300.0, 100.0), (300.0, 400.0));
    let check = &check_roads_on_terrain(&net, &hm)[0];
    assert!((check.grade - 0.07).abs() < 0.005 && (check.cross_fall - 0.03).abs() < 0.005);
}
```

The arc stays away from the jump of `atan2` (it runs from 20° to 80°).

- [ ] **Step 2: Run them.** `cargo test -p naves-core --test checks an_arc_on_a_spiral_ramp`: fails with a cross fall of about 0.05.
- [ ] **Step 3: Implement in `measure`.** If the headings of `start` and `end` differ by more than 0.001 rad, the samples lie on the circle through both points that is tangent to both headings (radius `chord / (2 sin(turn / 2))`), one per metre of arc, and "right" at each sample is at right angles to the heading there. Distances for the grade are arc lengths. Otherwise nothing changes. `quad_miss` stays on the chord's four corners.
- [ ] **Step 4: Verify and measure.** `make check`. Measure: the row `alone, arc of radius 10 or 25 m` gets smaller.
- [ ] **Step 5: Commit.** `fix(core): measure the grade and cross fall of a bend along the bend`.

### Task 4: A vertical radius, a grade limit per segment, excesses on the joints

**Files:**
- Modify: `crates/naves-core/src/profile.rs`, `crates/naves-core/tests/profile.rs`, `crates/naves-export/examples/road-analysis.rs`, `CHANGELOG.md`

**Interfaces:**
- Consumes: `limits::grade_aim`, `limits::min_vertical_radius`.
- Removes: `profile::max_grade_change`.
- Produces (private to `profile.rs`, used by Tasks 5 to 7): `struct Limits { grade: Vec<f64>, radius: f64 }` with one grade per segment between two variables; `fn solve_chain(targets: &[f64], spacing: &[f64], fixed_height: &[Option<f64>], limits: &Limits) -> Vec<f64>`.
- `GradeExcess` keeps its fields. For a change of grade, `grade` is the change per metre and `limit` is `1 / radius`.

Rules:
- The change of grade between two neighbouring segments is at most `(l1 + l2) / 2 / radius`, with each length taken as at least 0.5 m.
- `spacing[i]` is **the length of the piece that joins variable `i` and `i + 1`**, not the distance between the places the variables stand for. (A bridge has one variable for its deck, placed at its middle; the road before it ends at the deck's start.)
- The excesses are measured on the strip's joints after settling: each segment against its grade limit, each joint between two segments that are not bridge decks against the radius.

- [ ] **Step 1: Write the failing tests** in `tests/profile.rs`, with the helpers there (`terrain`, `piece`, `straight`, `network`, `grades`).

```rust
#[test]
fn short_pieces_bend_no_more_per_metre_than_long_ones() {
    // the same crest once in 25 m pieces and once in 6.25 m pieces
    let hm = terrain(200, 6.25, |_, y| 100.0 - 0.004 * (y - 600.0).powi(2).min(40_000.0));
    for length in [25.0, 6.25] {
        let p = solve_profiles(&network(vec![pieces_of(Family::Asf3, (300.0, 200.0), (300.0, 1000.0), length)]), &hm);
        let g = grades(&p, 0);
        let radius = min_vertical_radius(Family::Asf3);
        for pair in g.windows(2) {
            assert!((pair[1] - pair[0]).abs() <= length / radius + 1e-6, "{length}: {pair:?}");
        }
    }
}

#[test]
fn the_piece_before_a_bridge_keeps_the_grade_limit() {
    // a 20 % ramp down to a 25 m bridge: the last piece before the deck is not steeper than the aim
    let hm = terrain(200, 6.25, |_, y| 100.0 + 0.20 * (y - 500.0).max(0.0));
    let mut roads = straight("up", Family::Asf3, (300.0, 900.0), (300.0, 525.0));
    roads.push(piece("deck", 0, PlacedKind::Bridge, Family::Asf3, (300.0, 525.0), (300.0, 500.0)));
    roads.extend(straight("on", Family::Asf3, (300.0, 500.0), (300.0, 300.0)));
    let p = solve_profiles(&network(vec![roads]), &hm);
    assert!(grades(&p, 0).iter().all(|g| g.abs() <= grade_aim(Family::Asf3) + 1e-6));
    assert!(p.excesses.is_empty(), "{:?}", p.excesses);
}

#[test]
fn overlapping_pieces_do_not_break_the_solver() {
    // two straights that overlap by all but 1 cm: the distance between two joints is nearly zero
    let hm = terrain(100, 6.25, |_, y| 100.0 + 0.05 * y);
    let roads = vec![
        piece("a", 0, PlacedKind::Piece, Family::Asf3, (300.0, 100.0), (300.0, 125.0)),
        piece("a", 1, PlacedKind::Piece, Family::Asf3, (300.0, 100.01), (300.0, 125.01)),
        piece("a", 2, PlacedKind::Piece, Family::Asf3, (300.0, 125.01), (300.0, 150.01)),
    ];
    let p = solve_profiles(&network(vec![roads]), &hm);
    assert!(p.strips[0].joints.iter().all(|j| j.2.is_finite() && (j.2 - (100.0 + 0.05 * j.1)).abs() < 1.0));
}
```

`pieces_of(family, a, b, length)` is a new helper next to `straight`: pieces of the given length from `a` to `b`. The helper `piece` gives every road a width of 7 m whatever its family; the tests of this plan count on that. Change the property test `grade_change_limit_holds` (or the part of `grade_limit_holds` that tests the change) to the per-metre rule, and `the_limits_get_looser_with_the_rank_of_the_road` to compare `min_vertical_radius`.

- [ ] **Step 2: Run them.** `cargo test -p naves-core --test profile`: the three new tests fail (the second with a grade of about 0.16 before the deck).
- [ ] **Step 3: Implement.** `Limits` as above, filled with `grade_aim(family)` for every segment for now. `clip` reads the segment's own grade limit and the radius rule. `chain_of` returns the spacings as defined above. `excesses_of` takes the strip and works on its joints. `settle` and `resolve` pass `&Limits` through. In the example, replace the per-joint statistics that used `max_grade_change`, if any, by the per-metre ones it already prints.
- [ ] **Step 4: Verify and measure.** `make check`. Measure: in `vertical curvature of the planned profile`, the count of joints over 1 %/m falls for every family; `grade excesses` may rise, because the steps at junctions are now seen.
- [ ] **Step 5: Commit.** `fix(core): limit the bend of a road's profile per metre and report what the joints do`.

### Task 5: Junctions carry a grade; gentle at meetings and in tight bends

**Files:**
- Modify: `crates/naves-core/src/profile.rs`, `crates/naves-core/tests/profile.rs`, `docs/manual/src/getting-started.md`, `CHANGELOG.md`

**Interfaces:**
- Consumes: `Limits { grade, radius }` of Task 4.
- Produces: `pub const MEETING_GRADE: f64 = 0.06;` in `profile.rs`. `fn segment_grades(net: &PlacedNetwork, roads: &[usize], base: f64) -> Vec<f64>` (private): the grade limit of each segment of a chain for a family limit `base`. Task 7 calls it with other values of `base`.

Rules for `segment_grades`:
- A `PlacedKind::Junction` piece has **its own variable at each end**, like a piece. Only a bridge deck keeps one height.
- A segment is **at a meeting** if it is a junction piece, or if the first or last point of **another** chain of `net.chains` lies within the segment's half width plus 12.5 m of it. Its limit is `min(base, MEETING_GRADE)`.
- A segment that is an arc (headings of `start` and `end` differ by more than 0.001 rad) of radius `r` (`chord / (2 sin(turn / 2))`) and width `w` has the limit `base * ((r - w / 2) / r).max(0.5)`, so that the inner edge is not steeper than `base`.
- Both rules can apply; the smaller limit holds.

- [ ] **Step 1: Write the failing tests.** Replace `a_junction_is_level_along_its_length` by:

```rust
#[test]
fn a_junction_on_a_slope_carries_the_meeting_grade_at_most() {
    let hm = terrain(200, 6.25, |_, y| 100.0 + 0.10 * y);
    let mut roads = straight("a", Family::Asf3, (300.0, 200.0), (300.0, 500.0));
    roads.push(piece("a|b", 0, PlacedKind::Junction, Family::Asf3, (300.0, 500.0), (300.0, 512.5)));
    roads.extend(straight("c", Family::Asf3, (300.0, 512.5), (300.0, 800.0)));
    let p = solve_profiles(&network(vec![roads]), &hm);
    let g = grades(&p, 0);
    let at_junction = g[12]; // 300 m of 25 m pieces come first
    assert!(at_junction > 0.03 && at_junction <= MEETING_GRADE + 1e-6, "{at_junction}");
    assert!(g.iter().all(|v| v.abs() <= grade_aim(Family::Asf3) + 1e-6));
    assert!(p.excesses.is_empty(), "{:?}", p.excesses);
}

#[test]
fn a_through_road_is_gentle_where_a_side_road_ends_on_it() {
    // no junction piece: a track ends on the road's edge at y = 500
    let hm = terrain(200, 6.25, |_, y| 100.0 + 0.11 * y);
    let through = straight("a", Family::Asf3, (300.0, 200.0), (300.0, 800.0));
    let side = straight("s", Family::Mud, (303.5, 500.0), (500.0, 500.0));
    let p = solve_profiles(&network(vec![through, side]), &hm);
    for (i, pair) in p.strips[0].joints.windows(2).enumerate() {
        let near = pair[0].1 <= 512.5 && pair[1].1 >= 487.5;
        let grade = (pair[1].2 - pair[0].2) / 25.0;
        if near { assert!(grade.abs() <= MEETING_GRADE + 1e-6, "segment {i}: {grade}"); }
    }
}

#[test]
fn a_tight_bend_is_gentler_than_a_straight() {
    // a 60 degree arc of radius 10 m in a 7 m road on a 12 % slope: at most 12 % * 6.5 / 10
    let p = solve_profiles(&network(vec![bend_on_a_ramp(Family::Asf3, 10.0, 60.0, 0.12)]), &ramp_terrain(0.12));
    let on_the_arc = arc_grade(&p);
    assert!(on_the_arc <= 0.12 * 0.65 + 1e-6, "{on_the_arc}");
}

#[test]
fn an_arc_narrower_than_the_road_keeps_half_the_limit() {
    // radius 3 m, road 7 m wide (the helper's width): (r - w/2) / r is negative; the limit is half the aim
    let p = solve_profiles(&network(vec![bend_on_a_ramp(Family::Asf1, 3.0, 60.0, 0.08)]), &ramp_terrain(0.08));
    let on_the_arc = arc_grade(&p);
    assert!(on_the_arc.is_finite() && on_the_arc > 0.0 && on_the_arc <= 0.04 + 1e-6, "{on_the_arc}");
}
```

`bend_on_a_ramp` (a straight, one arc piece with tangent poses, a straight), `ramp_terrain` and `arc_grade` (the grade of the arc's segment) are new helpers. Keep `a_side_road_ends_at_the_junction_port_height` and `side_road_starts_at_the_through_road_height` passing: the side road's end takes the height of the through road where it meets it, which on a graded junction is interpolated along the piece as `meeting_height` already does.

- [ ] **Step 2: Run them.** `cargo test -p naves-core --test profile`: the four fail.
- [ ] **Step 3: Implement.** `chain_of` treats a junction like a piece. `profile_chain` fills `Limits::grade` from `segment_grades(net, roads, grade_aim(family))`.
- [ ] **Step 4: Texts.** `getting-started.md`, where the road profile is described: a road is at most 6 % steep where another meets it and gentler in tight bends. Changelog under *Changed*.
- [ ] **Step 5: Verify and measure.** `make check`. Measure: on the `junctions:` line the neighbours over the aim go from 306 towards 0 and the p90 of the grade change from 4.04 %/m to below 1; `cross fall > 5 %` falls.
- [ ] **Step 6: Commit.** `fix(core): junctions carry a grade, roads are gentle where they meet and in tight bends`.

### Task 6: A side road lies on the road it meets as far as that road's ground reaches

**Files:**
- Modify: `crates/naves-core/src/profile.rs`, `crates/naves-core/tests/profile.rs`, `crates/naves-core/tests/checks.rs`, `CHANGELOG.md`

**Interfaces:**
- Consumes: `meeting`, `meeting_height`, `Solved`, `resolve` in `profile.rs`; `conform::claim_cores` gives a road's height to every vertex within `half_width + 1.5 * cell` of its centre line.
- Produces: `Solved::joins` becomes `Vec<(usize, Meeting)>` (variable, where it lies on the other road). No public change.

Rules:
- From each end of a chain that meets an earlier road (as today, within `half_width + ON_ROAD_M`), walk inwards: a variable is **also fixed to that same road's surface** while it lies within `owner.half_width + 1.5 * cell` of the owner's centre line (`cell` is `hm.cell_m`; on a junction segment within that distance beyond the piece's reach). Its height is the owner's at the nearest point.
- At most 8 variables per end. If the walks from both ends meet, every variable is fixed and the chain is not solved.
- `resolve` re-reads all of these from the owner's current heights, as it does for the two ends today.

- [ ] **Step 1: Write the failing tests.**

In `tests/checks.rs` (it has `conform` and `solve_profiles` in scope):

```rust
#[test]
fn a_side_road_starts_on_the_ground_of_the_road_it_leaves() {
    // the through road falls 5 % to the north; the side road leaves east, where the ground rises 10 %
    let base = terrain(160, 6.25, |x, y| 100.0 - 0.05 * y + 0.10 * (x - 300.0).max(0.0));
    let mut net = chain(Family::Asf3, true, (300.0, 100.0), (300.0, 900.0));
    append_chain(&mut net, Family::Grav, true, (303.5, 500.0), (600.0, 500.0));
    let profiles = solve_profiles(&net, &base);
    let shaped = conform(&base, &profiles.strips).unwrap().heightmap;
    for c in check_roads_on_terrain(&net, &shaped).iter().filter(|c| c.family == Family::Grav) {
        assert!(c.miss_m <= MAX_MISS_M, "miss {} at {:?}", c.miss_m, c.start);
        assert!(c.cross_fall <= CROSS_FALL_AIM, "cross fall {} at {:?}", c.cross_fall, c.start);
    }
}
```

`append_chain` is a new helper that adds a second chain of 12.5 m pieces to a `PlacedNetwork`.

In `tests/profile.rs`:

```rust
#[test]
fn a_short_road_between_two_others_is_all_fixed_and_finite() {
    // two parallel roads 20 m apart and a 20 m link between them
    let hm = terrain(100, 6.25, |x, y| 100.0 + 0.05 * x + 0.02 * y);
    let west = straight("w", Family::Asf3, (300.0, 100.0), (300.0, 500.0));
    let east = straight("e", Family::Asf3, (320.0, 100.0), (320.0, 500.0));
    let link = vec![
        piece("l", 0, PlacedKind::Piece, Family::Mud, (303.5, 300.0), (310.0, 300.0)),
        piece("l", 1, PlacedKind::Piece, Family::Mud, (310.0, 300.0), (316.5, 300.0)),
    ];
    let p = solve_profiles(&network(vec![west, east, link]), &hm);
    let (w, e) = (profile_height(&p, 0, 300.0), profile_height(&p, 1, 300.0));
    for joint in &p.strips[2].joints {
        assert!(joint.2.is_finite());
        assert!(joint.2 >= w.min(e) - 1e-6 && joint.2 <= w.max(e) + 1e-6, "{joint:?}");
    }
}
```

- [ ] **Step 2: Run them.** The first fails with a miss of the order of 1 m on the side road's first pieces.
- [ ] **Step 3: Implement** in `end_joins` (it needs `cell`) and `resolve`, by the rules above.
- [ ] **Step 4: Verify and measure.** `make check`. Measure: the rows `other road within 3 cells, different height` (16.19 km) and `other road overlaps, different height` (8.81 km) fall; `miss > 0.15 m` falls.
- [ ] **Step 5: Commit.** `fix(core): a road that ends on another lies on its ground as far as that reaches`.

### Task 7: The aim gives way where it would take the road off the ground

**Files:**
- Modify: `crates/naves-core/src/profile.rs`, `crates/naves-core/tests/profile.rs`, `docs/manual/src/getting-started.md`, `docs/manual/src/troubleshooting.md`, `CHANGELOG.md`

**Interfaces:**
- Consumes: `segment_grades(net, roads, base)` of Task 5; `limits::grade_aim`, `limits::grade_ceiling`.
- Produces: `pub const RELAX_OFF_M: f64 = 4.0;` in `profile.rs`.

Rule: a chain is solved with `base = grade_aim(family)`. If any variable that is not fixed is then more than `RELAX_OFF_M` from the terrain under it, the chain is solved again from the start with `base` halfway between the aim and `grade_ceiling(family)`, and if that is still so, with the ceiling. The last result stands. The limits a chain ended with are the ones `settle` and the excesses use. Pieces over the aim show as `Steep` in the check, as they do today.

- [ ] **Step 1: Write the failing tests.**

```rust
#[test]
fn a_road_on_ground_steeper_than_its_aim_gives_way() {
    let hm = terrain(200, 6.25, |_, y| 100.0 + 0.20 * y);
    let p = solve_profiles(&network(vec![straight("a", Family::Asf3, (300.0, 200.0), (300.0, 800.0))]), &hm);
    let steepest = grades(&p, 0).iter().fold(0.0_f64, |m, g| m.max(g.abs()));
    assert!(steepest > grade_aim(Family::Asf3) && steepest <= grade_ceiling(Family::Asf3) + 1e-6, "{steepest}");
    for joint in &p.strips[0].joints {
        assert!((joint.2 - (100.0 + 0.20 * joint.1)).abs() <= RELAX_OFF_M + 0.5, "{joint:?}");
    }
}

#[test]
fn a_road_that_stays_near_the_ground_keeps_its_aim() {
    // 14 % for 100 m in the middle of level ground: 2 m off at most with the aim of 12 %
    let hm = terrain(200, 6.25, |_, y| 100.0 + 0.14 * (y - 450.0).clamp(0.0, 100.0));
    let p = solve_profiles(&network(vec![straight("a", Family::Asf3, (300.0, 200.0), (300.0, 800.0))]), &hm);
    assert!(grades(&p, 0).iter().all(|g| g.abs() <= grade_aim(Family::Asf3) + 1e-6));
}

#[test]
fn giving_way_is_deterministic_and_leaves_earlier_roads_alone() {
    let hm = terrain(200, 6.25, |x, y| 100.0 + 0.20 * y + 0.01 * x);
    let first = straight("a", Family::Asf2, (200.0, 200.0), (800.0, 200.0));
    let steep = straight("b", Family::Asf3, (500.0, 300.0), (500.0, 900.0));
    let alone = solve_profiles(&network(vec![first.clone()]), &hm);
    let both = solve_profiles(&network(vec![first.clone(), steep.clone()]), &hm);
    assert_eq!(alone.strips[0], both.strips[0]);
    assert_eq!(both, solve_profiles(&network(vec![first, steep]), &hm));
}
```

Check that `grade_limit_holds` (the property test) now asserts against the ceiling where the terrain is steep, and against the aim where the chain did not give way; if it cannot tell, it asserts the ceiling.

- [ ] **Step 2: Run them.** The first fails: the road stays at 12 % and is about 24 m off at the ends of the ramp.
- [ ] **Step 3: Implement** in `profile_chain`.
- [ ] **Step 4: Texts.** `getting-started.md`: a road keeps to its family's grade where it can and is steeper, up to nine tenths of what vehicles manage, where it would otherwise be more than 4 m above or below the ground. `troubleshooting.md`: what an amber road means. Changelog under *Changed*.
- [ ] **Step 5: Verify and measure.** `make check`. Measure: `joints more than 5 m off the ground` (4755) falls, `vertices changed ... by more than 5 m` (65 518) falls, the `Steep` kilometres rise, `NotDrivable` does not rise by more than 1 km.
- [ ] **Step 6: Commit.** `feat(core): a road gives up its grade aim before it leaves the ground`.

### Task 8: Cuttings and embankments as steep as the hillside needs

**Files:**
- Modify: `crates/naves-core/src/conform.rs`, `crates/naves-core/tests/conform.rs`, `crates/naves-core/tests/checks.rs`, `docs/manual/src/getting-started.md`, `CHANGELOG.md`

**Interfaces:**
- Produces: `pub const CUT_SLOPE_MAX: f64 = 2.0; pub const FILL_SLOPE_MAX: f64 = 1.0;` in `conform.rs`. `CUT_SLOPE` (1.0) and `FILL_SLOPE` (0.667) stay and are now the gentlest slopes. Task 9 uses `FILL_SLOPE_MAX`.

Rule: each core vertex has a natural slope `s`, the slope of `base` there (`hypot(dz_dx, dz_dy)` of `Heightmap::sample` at the vertex). Its cutting rises at `(1.5 * s).clamp(CUT_SLOPE, CUT_SLOPE_MAX)` and its embankment falls at `(1.5 * s).clamp(FILL_SLOPE, FILL_SLOPE_MAX)`. A vertex outside the cores uses the slopes of its nearest core vertex (`Fields::nearest`) wherever today's code uses the constants: in `Fields::relax` the slopes of `from`'s nearest core, in `clamp_flooded` those of the vertex's own.

- [ ] **Step 1: Write the failing tests** in `tests/conform.rs`. Replace `a_valley_side_steeper_than_the_embankment_leaves_a_reported_step` by:

```rust
#[test]
fn an_embankment_on_a_40_degree_side_reaches_the_ground() {
    let slope = 40.0_f64.to_radians().tan();
    let base = terrain(200, 6.25, |x, _| 500.0 - slope * x);
    let out = conform(&base, &[road((600.0, 200.0), (600.0, 1000.0), 500.0 - slope * 600.0, 500.0 - slope * 600.0, 3.5, 3)]).unwrap();
    assert!(out.capped.is_empty(), "{} capped", out.capped.len());
    let downhill = steepest(&out.heightmap, |col, row| col > 100 && (40..150).contains(&row));
    assert!(downhill <= FILL_SLOPE_MAX + 0.05, "{downhill}");
}

#[test]
fn gentle_ground_keeps_the_gentle_slopes() {
    // a road 6 m above a 10 % slope: the embankment is 34 degrees, not steeper
    let base = terrain(200, 6.25, |x, _| 300.0 - 0.10 * x);
    let out = conform(&base, &[road((600.0, 200.0), (600.0, 1000.0), 246.0, 246.0, 3.5, 3)]).unwrap();
    let steep = steepest(&out.heightmap, |_, row| (40..150).contains(&row));
    assert!(steep <= FILL_SLOPE + 0.05, "{steep}");
}

#[test]
fn level_ground_and_the_map_edge_are_shaped_as_before() {
    // natural slope zero, and a road whose core touches the border
    let base = terrain(64, 6.25, |_, _| 100.0);
    let out = conform(&base, &[road((3.0, 0.0), (3.0, 393.0), 103.0, 103.0, 3.5, 3)]).unwrap();
    assert!(out.heightmap.h.iter().all(|h| h.is_finite()));
    let steep = steepest(&out.heightmap, |_, _| true);
    assert!(steep <= FILL_SLOPE + 0.05, "{steep}");
}
```

Check the signature of `steepest` and of `road` in the file before using these. In `tests/checks.rs`, `batter_slopes_are_walkable` becomes:

```rust
#[test]
fn a_road_can_always_be_left_downhill() {
    assert!(FILL_SLOPE_MAX.atan().to_degrees() < WALK_LIMIT_DEG);
    assert!(CUT_SLOPE.atan().to_degrees() < WALK_LIMIT_DEG);
}
```

`a_wall_next_to_the_road_stops_at_the_cap_and_is_reported` must still pass (a wall is steeper than `CUT_SLOPE_MAX`); if it does not, make its wall steeper than 63°, not the limit lower.

- [ ] **Step 2: Run them.** `cargo test -p naves-core --test conform an_embankment_on_a_40_degree_side`: fails with capped vertices.
- [ ] **Step 3: Implement** by the rule. `Core` gets `cut: f32` and `fill: f32`, set in `claim_cores` (it needs `base`).
- [ ] **Step 4: Texts.** `getting-started.md`, the paragraph on earthworks: slopes are 45° in a cutting and 34° on an embankment on gentle ground and get steeper with the hillside, up to 63° and 45°; a cutting in a steep hillside can be too steep to walk up. Changelog under *Changed*.
- [ ] **Step 5: Verify and measure.** `make check`. Measure: `capped earthworks` falls from 1904 to below 300, `conflicts` to below 2000, `vertices changed ... by more than 5 m` falls; `cross fall > 5 %` and `miss > 0.15 m` do not rise by more than 2 %.
- [ ] **Step 6: Commit.** `feat(core): earthworks as steep as the hillside they stand on needs`.

### Task 9: Hairpin legs may differ by a step

**Files:**
- Modify: `crates/naves-core/src/profile.rs`, `crates/naves-core/tests/profile.rs`, `CHANGELOG.md`

**Interfaces:**
- Consumes: `conform::FILL_SLOPE_MAX` of Task 8.

Rule, in `wanted_shifts` (it needs `cell`): two spots closer than `gap` (their half widths plus three cells) share vertices and may differ by `MATCH_M`, as today. From `gap` on they may differ by `MATCH_M + (distance - gap + cell) * FILL_SLOPE_MAX`: the cell between their level strips can hold a step. The doc comment of `settle` says so.

- [ ] **Step 1: Write the failing tests.**

```rust
#[test]
fn legs_whose_level_strips_do_not_touch_keep_their_heights() {
    // two legs on a 35 degree flank, 27 m apart; the helper's roads are 7 m wide, so their edges are 20 m
    // apart, 1.25 m more than three cells
    let slope = 35.0_f64.to_radians().tan();
    let hm = terrain(200, 6.25, |x, _| 400.0 - slope * (x - 300.0) * 0.3);
    let legs = hairpin_legs(Family::Mud, 300.0, 327.0, 200.0, 600.0);
    let p = solve_profiles(&network(vec![legs]), &hm);
    let apart = (leg_height(&p, 300.0, 400.0) - leg_height(&p, 327.0, 400.0)).abs();
    assert!(apart > 4.0, "{apart}");
    let out = conform(&hm, &p.strips).unwrap();
    assert!(out.conflicts.is_empty(), "{:?}", out.conflicts);
}
```

`hairpin_legs` builds one chain: up one leg, a link at the top, down the other; `leg_height` reads the profile at a place. Choose the terrain factor so that the legs differ by 5 to 6 m at y = 400 before settling, and say the number in a comment. Adjust `switchback_legs_leave_room_for_a_slope` to the new formula (its assertion uses `FILL_SLOPE`), and keep `roads_closer_than_their_footprints_share_a_height` unchanged and passing.

- [ ] **Step 2: Run it.** Fails: the legs are pulled to within about 1 m.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Verify and measure.** `make check`. Change the example's `room` to the same formula (`(c - margin + cell).max(0.0) * 1.0 + 0.1` for `c >= margin`). Measure: on `pieces with another part of their own road within 60 m`, the count that is short falls from 2040; `conflicts` and `miss > 1.00 m` do not rise.
- [ ] **Step 5: Commit.** `fix(core): hairpin legs keep their heights when a step fits between them`.

### Task 10: The result

**Files:**
- Modify: `docs/research/2026-11-10-road-alignment.md`, `examples/berchtesgaden/README.md`, `docs/manual/src/troubleshooting.md`, `CHANGELOG.md`

- [ ] **Step 1: Measure at 1024 and 2048** (`road-analysis` and `road-analysis 2048`).
- [ ] **Step 2: Add a section "5. After the fixes"** to the research note: the table of section 1 with a column for before and after at both sizes, the table of 2.2 after, and one paragraph on what is left and why (hairpins that are still short, the gaps, what the ramps in the game would change). Numbers only from the runs.
- [ ] **Step 3: The advice on the grid.** `troubleshooting.md` and the example's README: on steep ground with hairpins `terrain_size = 2048` leaves fewer roads in conflict, with the before-and-after numbers of step 1 and the cost from the README's table.
- [ ] **Step 4: Verify.** `make check` and `make docs`.
- [ ] **Step 5: Commit.** `docs: road alignment after the fixes`.

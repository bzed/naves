<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Roads that look right in the game: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The roads of a `naves build` are drawn in the game, and the look faults seen there (where roads meet, roundabouts, gaps, bridges, humps) are fixed automatically, each stage judged with the same sites in the game before and after.

**Architecture:** Stage 1 stages the game's road models on `binarize`'s `P:` drive during the build and adds a developer tool that photographs a fixed list of sites in the game. Stage 2 changes the road placement in `naves-core` (`roadplace`, `roadnet`, `junctions`, `profile`, `conform`, `validate`) step by step, each step measured on the Berchtesgaden example at 2048 and recorded in the research note.

**Tech Stack:** Rust 1.99 (edition 2024), the existing workspace crates; dayzmake ≥ 2.0.0; DayZ Tools `binarize.exe` under Proton; Enforce Script for the in-game helper mod; shell for the game-check runner.

**Spec:** `docs/superpowers/specs/2026-11-15-road-look-design.md` (approved 2026-11-15). Read it before any task; its section numbers (1.1 … 2.5) are referred to below.

## Global Constraints

- AGENTS.md in full: test first (red, then green); integration tests in `crates/*/tests/`; no `unwrap`/`expect`/`panic!`/unchecked indexing on untrusted data in library crates; `#[expect(lint, reason = "...")]`, never `#[allow]`; `thiserror` errors with context; no `println!` in libraries; `BTreeMap`/sorted output only (byte-for-byte reproducible builds); docs (`docs/manual/`) and a `CHANGELOG.md` `## [Unreleased]` line in the same commit as a user-visible change; `make check` passes before every commit; total coverage ≥ 85 %.
- Temp and build folders under `/var/tmp`, never `$HOME`; `/tmp` is a RAM tmpfs. Before `make check` after merging work from a worktree: `find crates fuzz/fuzz_targets -name '*.rs' -exec touch {} +` (shared target dir `/var/tmp/naves-target`).
- No game data in the repository, test fixtures or the built mod; game files are read from the user's install at build time. What `binarize` derives from the road models into the world file is allowed.
- Every figure in notes and plans: the Berchtesgaden example at `terrain_size = 2048` (8 192 m map, 4 m terrain cells), measured with `cargo run --release -p naves-export --example road-analysis 2048` or a `naves build`.
- Game runs: only inside headless sway; own server tree and non-default ports; one game at a time; never edit files in `<Steam>/steamapps/compatdata/221100/pfx/drive_c/users/steamuser/Documents/DayZ/`; never pass `-name=` or `-profiles=` to the client; remove the `@naves_*` symlink in the DayZ install afterwards. Never start the game in a task unless the task says so.
- Values fixed by the spec: overlay drop `0.05` m; duplicate roads `25.0` m overlap, centre lines closer than half the narrower width, headings within `15.0`°; bridge clearance `4.5` m (lower road surface to deck underside); junction shift `6.25` m steps up to `25.0` m each way; ranking Asf1 > Asf2 > City > Asf3 > Grav > Mud; for equal families the longer chain is higher.
- New rule ids: `road_duplicate_dropped`, `bridge_clearance`. Existing ids keep their names.
- Commits: conventional prefixes, trailer `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Implementers are Sonnet; a Sonnet implementer may ask an Opus subagent for one specific problem it cannot solve.

## Review Focus

1. **A lower road shorter than half the higher road's width** (a short stub ending under the higher road): the overlay cut leaves no piece of zero or negative length and does not panic; the stub is dropped and reported. Test in Task 5.
2. **A roundabout ring that is not closed or is cut by the map edge** (OSM ways missing, negative node ids): it falls back to ordinary roads, no panic, no endless loop. Test in Task 7.
3. **A bridge whose 4.5 m clearance cannot be reached within the grade limits** (short approaches, junction right beside it): a `bridge_clearance` finding, the solve terminates. Test in Task 10.
4. **Two duplicate roads of the same family and the same chain length**: exactly one is dropped, decided by stretch id, the same in every run. Test in Task 6.
5. **A build with no roads or without `structures_roads.pbo`** (no OSM, or the server install lacks the PBO): the binarize log check does not fail on the one nameless "Cannot open object" line, and a missing PBO is an error naming the file and the setting. Test in Task 1.

---

## Stage 1: the roads appear, and a repeatable game check

### Task 1: Road models on binarize's P: drive (spec 1.1)

**Files:**
- Modify: `crates/naves-export/src/catalog_read.rs` (factor the unpack out)
- Modify: `crates/naves-export/src/build.rs` (stage before `binarize_world`, remove after, check the log)
- Test: `crates/naves-export/tests/build.rs`, `crates/naves-export/tests/catalog_read.rs`
- Docs: `docs/manual/src/troubleshooting.md` ("Cannot open object" entry), `docs/formats/binarize-world.md`, `CHANGELOG.md`

**Interfaces:**
- Produces: `pub fn unpack_roads(tools: &Tools, pbo: &Path, dest: &Path) -> Result<(), CatalogReadError>` in `catalog_read.rs` (unpacks `structures_roads.pbo` so that `dest/dz/structures/roads/...` holds the models; `road_catalog` uses it too).
- Produces: `pub fn check_binarize_log(log: &str, world_has_roads: bool) -> Result<(), BuildError>` in `build.rs`, and `BuildError::RoadModelsMissing { models: Vec<String>, log: PathBuf }` and `BuildError::NoRoadNetwork { log: PathBuf }`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_road_model_binarize_cannot_open_is_an_error_naming_it() {
    let log = "Cannot open object dz\\structures\\roads\\parts\\asf1_25.p3d\nRoad 2 dirs: 0\n";
    let err = check_binarize_log(log, true).unwrap_err();
    assert!(err.to_string().contains("asf1_25.p3d"));
}
#[test]
fn the_nameless_cannot_open_line_of_every_build_is_ignored() {
    assert!(check_binarize_log("Cannot open object \nRoad 0 dirs: 0\n", false).is_ok());
}
#[test]
fn roads_but_no_road_directions_is_an_error() {
    assert!(matches!(check_binarize_log("Road 2 dirs: 0\nRoad 3 dirs: 0\n", true),
        Err(BuildError::NoRoadNetwork { .. })));
}
#[test]
fn road_models_are_staged_for_binarize_and_gone_afterwards() { /* fake dayzmake + fake binarize (testing.rs):
    the fake binarize records whether P/dz/structures/roads/parts exists when it runs; after the build that folder
    is gone, and no .p3d is inside the mod folder or the PBO's source folder */ }
#[test]
fn a_missing_structures_roads_pbo_is_an_error_naming_the_file() { /* server install without the PBO, project with OSM */ }
```

- [ ] **Step 2: Run them, expect FAIL** — `cargo test -p naves-export --test build --test catalog_read` (missing functions).
- [ ] **Step 3: Implement.** Stage with `unpack_roads(tools, &tools.dayz_server.join("addons/structures_roads.pbo"), &layout.staging.join("P"))` right before `binarize_world` when the world has road objects; remove `layout.staging.join("P/dz")` after it, also when `binarize_world` fails. Read `binarize.log`, call `check_binarize_log`. Whether to unpack only the used models is your call; measure the time either way and put it in the commit message.
- [ ] **Step 4: Run the tests, expect PASS; `make check`.**
- [ ] **Step 5: Real build.** `naves build examples/berchtesgaden` inside headless sway (`WLR_BACKENDS=headless sway -c <conf>`). Expected: exit 0, `logs/binarize.log` without road "Cannot open object", `Road 2 dirs` > 0, no `.p3d` under `build/@naves_berchtesgaden`.
- [ ] **Step 6: Commit** `fix: binarize gets the game's road models, so the roads are drawn`.

### Task 2: The game check tool (spec 1.2, 1.3)

**Files:**
- Create: `crates/naves-export/src/sites.rs` (site picking, pure), export in `lib.rs`
- Create: `crates/naves-export/examples/road-sites.rs` (writes `sites.json` for a built project)
- Create: `tools/ingame-check/` with `run.sh` (server + headless client + shots), `NavesCam/` (the helper mod's Enforce Script and `config.cpp`), `README.md`; REUSE headers
- Test: `crates/naves-export/tests/sites.rs`
- Docs: `AGENTS.md` ("Testing a built map in the game": point to the tool), `docs/manual/src/` only if users should know (they should not: it is a developer tool)

**Interfaces:**
- Produces: `pub struct Site { pub id: u32, pub category: String, pub x: f64, pub z: f64, pub yaw_deg: f64, pub numbers: BTreeMap<String, f64>, pub note: String }` (serde), and `pub fn pick_sites(report: &serde_json::Value, objects: &[PlacedRoad], fixed: &[Site]) -> Vec<Site>` in `sites.rs`.
- Consumes: `build/report.json`; the placed objects from `naves_export::generate` (`Generated`), as `road-analysis` already gets them.

Start from the screenshot agent's work in `/var/tmp/naves-ingame/` (`cammod/`, `analysis/sites.py`, the run scripts; read them fully — never reuse anything that edits the player's settings). Categories and fixed sites: spec 1.3; fixed sites in map metres: roundabout (4349.9, 5459.4 on the old 6.4 km map — recompute from 47.626173 N, 12.994013 E), Hammerstielstraße (47.604126 N, 12.949617 E), Kehlsteinstraße bridge (47.631554 N, 13.047906 E), Silbergstraße bridge (47.630057 N, 12.959948 E); convert with `Heightmap`'s `to_map`.

- [ ] **Step 1: Write the failing tests** in `tests/sites.rs`: `the_same_input_gives_the_same_sites` (byte-identical JSON twice), `every_category_of_the_spec_is_present_on_the_fixture`, `sites_lie_inside_the_map`, `two_sites_are_at_least_100_m_apart_unless_fixed`, `slight_angle_sites_have_an_interior_angle_between_150_and_180_degrees`.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement `pick_sites`** and the example.
- [ ] **Step 4: Implement `tools/ingame-check/run.sh`**: arguments `<build dir> <sites.json> <out dir>`; free ports (the `dayz-dev` skill's `free-ports.sh` logic), own server tree under the out dir, `verifySignatures = 2`, `BattlEye = 0`, noon and clear weather in the server's copy of `init.c` (AGENTS.md "Comparable screenshots"), `serverTimeAcceleration=1`; client `-mod=@naves_<world>,@NavesCam -connect=127.0.0.1 -port=<p>` (no `-name`, no `-profiles`); the helper mod signed with a key made by `dayzmake keygen` in the out dir; shots with `grim`, an `index.md` and contact sheets (`montage`); copy the player's settings file before, `diff` after and fail loudly on any difference other than `lastMPServer`; a `trap` that stops client, server and sway and removes every symlink.
- [ ] **Step 5: `make check`; shellcheck `run.sh` if installed.**
- [ ] **Step 6: Commit** `feat: a developer tool that photographs road sites in the game`.

### Checkpoint A (controller, not an implementer): the stage 1 game check

Build the example with Task 1, `road-sites`, run the tool. Pass: road pieces are drawn where the shots show roads. Keep `sites.json` and the shots under `/var/tmp/naves-roadlook/baseline/` as "before" for stage 2. The maintainer looks at the contact sheets.

---

## Stage 2: the look faults

Every task in this stage ends with: `road-analysis 2048` before and after, the counts in a new subsection of `docs/research/2026-11-10-road-alignment.md` section 9 ("Roads that look right", one subsection per task), `make check`, commit.

### Task 3: Swapped X pieces (spec 2.1, first bullet)

**Files:** Modify `crates/naves-core/src/roadplace.rs` (`plan_junction`); Test `crates/naves-core/tests/roadplace.rs`

**Interfaces:** Produces a private change only: in `plan_junction`, for two sides when `x_junction(through, side)` is `None`, try `x_junction(side, through)` and place it rotated by 90° so its through ports lie on the side arms. T junctions are never swapped (spec, maintainer).

- [ ] **Step 1: Failing test** `an_x_crossing_uses_the_swapped_piece_when_only_that_exists`: synthetic catalog with only `kr_x_asf1_city`; a City through road crossed by an Asf1 road; assert one junction object with model `kr_x_asf1_city` and no `join_without_junction`. Also `a_t_crossing_is_never_swapped`: only `kr_t_city_asf3`, Asf3 through, City side → no junction piece placed.
- [ ] **Step 2: FAIL. Step 3: implement. Step 4: PASS, measure, `make check`. Step 5: commit** `feat: an X crossing uses the junction piece with the roles swapped`.

### Task 4: Keep the OSM tags the later tasks need

**Files:** Modify `crates/naves-core/src/osm.rs` (`WAY_TAGS`), `crates/naves-core/src/features.rs`; Test `crates/naves-core/tests/features.rs`; fuzz seeds unchanged or extended

**Interfaces:**
- Produces: `FeatureKind::Road { family, drivable, bridge, tunnel, roundabout: bool, layer: i8 }` — `roundabout` is `junction=roundabout` or `junction=circular`; `layer` is the OSM `layer` tag clamped to −5..=5, default 0, unparsable → 0.
- Every `FeatureKind::Road { .. }` pattern in the workspace is updated.

- [ ] **Step 1: Failing tests** `roundabout_and_circular_set_the_flag`, `layer_is_read_and_clamped` (`"1"` → 1, `"-1"` → −1, `"9"` → 5, `"x"` → 0).
- [ ] **Step 2–4:** FAIL, implement (add `"junction"`, `"layer"` to `WAY_TAGS`), PASS, `make check`. **Step 5: commit** `feat: road features keep junction=roundabout and layer`.

### Task 5: The overlay rule (spec 2.1, second bullet)

**Files:** Modify `crates/naves-core/src/roadplace.rs`, `crates/naves-core/src/junctions.rs`, `crates/naves-core/src/profile.rs`, `crates/naves-core/src/conform.rs`, `crates/naves-core/src/validate.rs`; Test `crates/naves-core/tests/{roadplace,junctions,validate}.rs`; Docs `docs/manual/src/getting-started.md` (Roads: what happens where no junction piece exists), `CHANGELOG.md`

**Interfaces:**
- Produces: `JunctionKind::Overlay` in `junctions.rs` (roads that meet where the game has no junction piece; the higher road runs on, the lower one ends under it).
- Produces: `pub const OVERLAY_DROP_M: f64 = 0.05;` in `roadplace.rs`.
- Produces: `pub fn overlay_rank(a: &Chain, b: &Chain, graph: &RoadGraph) -> std::cmp::Ordering` in `roadnet.rs` (family rank as in `chains`, then longer chain, then lower stretch id).
- Placement stays hierarchical (spec): the higher road is fitted first as today; the lower road's fit ends at a goal pose on the higher road's edge (half its width from its centre line, plus the lower road's piece reaching under it by up to that half width) and starts again under the other edge where it goes on. No end caps at an overlay. Its height at the cut is the higher road's surface minus `OVERLAY_DROP_M`.
- An overlay is not a `join_without_junction` and its overlap is not a `road_conflict`.

- [ ] **Step 1: Failing tests:**
  - `a_mud_track_ends_under_the_edge_of_a_street` (T, City through, Mud side, no piece): the Mud road's last piece ends at a distance from the City centre line between `0` and `City::width/2`; no end cap on it; no `join_without_junction`.
  - `a_crossing_without_piece_interrupts_the_lower_road` (X, Grav through Mud): the Mud road has two parts, each ending under the Grav road; none of its pieces crosses the Grav centre line.
  - `the_higher_road_ending_on_a_lower_one_lies_on_top` (City ends on an Asf3 that runs on): the Asf3 is unbroken; the City end has no end cap.
  - `the_lower_road_sits_five_centimetres_below_at_the_cut`: profile heights at the cut differ by `OVERLAY_DROP_M` ± 1e-6.
  - `equal_families_put_the_longer_road_on_top`.
  - `an_overlay_is_not_a_conflict_or_a_join_without_junction` (validate).
  - Review Focus 1: `a_stub_shorter_than_half_the_width_is_dropped_and_reported` (no zero-length piece, no panic).
- [ ] **Step 2: FAIL. Step 3: implement. Step 4: PASS; measure (expect `join_without_junction` far below 877); `make check`. Step 5: commit** `feat: where no junction piece exists the lower road ends under the higher one`.

### Task 6: Duplicate roads (spec 2.1, third bullet)

**Files:** Modify `crates/naves-core/src/roadnet.rs`, `crates/naves-core/src/validate.rs`, `crates/naves-export/src/generate.rs` (call it, report); Test `crates/naves-core/tests/roadnet.rs`; Docs manual (Roads), `docs/manual/src/mcp.md` (rule list), `CHANGELOG.md`

**Interfaces:**
- Produces: `pub struct DroppedDuplicate { pub stretch: String, pub kept: String, pub from_m: f64, pub to_m: f64, pub at: (f64, f64) }` and `pub fn drop_duplicates(graph: &RoadGraph) -> (RoadGraph, Vec<DroppedDuplicate>)` in `roadnet.rs`; constants `DUPLICATE_MIN_M: f64 = 25.0`, `DUPLICATE_MAX_HEADING_DEG: f64 = 15.0`.
- Produces: `Rule::RoadDuplicateDropped` with id `"road_duplicate_dropped"`, one finding per dropped part at its middle.
- Consumes: `overlay_rank` (Task 5) to pick the kept road. Roads only (`FeatureKind::Path` untouched).

- [ ] **Step 1: Failing tests:** `two_roads_drawn_on_top_of_each_other_keep_the_higher_ranked`, `a_parallel_road_further_than_half_the_width_is_kept`, `an_overlap_shorter_than_25_m_is_kept`, `headings_more_than_15_degrees_apart_are_kept`, `the_lower_road_goes_on_after_the_overlap` (its rest is still in the graph and meets the kept road), Review Focus 4: `equal_family_and_length_drops_the_higher_stretch_id_every_time`.
- [ ] **Step 2–4:** FAIL, implement, PASS, measure, `make check`. **Step 5: commit** `feat: a road drawn twice in OpenStreetMap is placed once`.

### Task 7: Roundabouts on a tilted plane (spec 2.2)

**Files:** Modify `crates/naves-core/src/roadnet.rs` (ring detection), `crates/naves-core/src/roadplace.rs` (ring placement), `crates/naves-core/src/conform.rs` (sloped patch), `crates/naves-export/src/generate.rs` (`level_patches`), `crates/naves-core/src/junctions.rs`/`profile.rs` (heights from the plane); Test `crates/naves-core/tests/{roadnet,roadplace,conform}.rs`; Docs manual (Roads), `CHANGELOG.md`

**Interfaces:**
- Produces: `pub struct Ring { pub stretches: Vec<usize>, pub centre: (f64, f64), pub radius: f64, pub family: Family }` and `pub fn rings(graph: &RoadGraph) -> Vec<Ring>` in `roadnet.rs` (closed loops of `roundabout` stretches only).
- Produces: `LevelPatch` gains `pub gradient: (f64, f64)` (metres per metre east and north; `(0.0, 0.0)` is level, every existing constructor passes that). A vertex in the patch takes `height + gradient.0 * dx + gradient.1 * dy`.
- The plane: least squares over the original ground inside the ring (1 m grid), gradient magnitude clamped to the family's grade ceiling. The ring is placed as a closed loop of the family's arcs and straights (one height per point from the plane); joining roads end on the ring by the overlay rule (Task 5) at the plane's height. A ring whose radius is below the tightest arc of its family is treated as an ordinary crossing.

- [ ] **Step 1: Failing tests:** `a_closed_roundabout_is_one_ring`, Review Focus 2: `an_open_or_cut_ring_stays_ordinary_roads`, `a_ring_is_placed_as_a_closed_loop` (first piece's start equals last piece's end within `GAP_CLOSED_M`), `a_tilted_patch_follows_the_plane` (conform: vertices inside equal the plane ± 1e-3), `the_tilt_is_clamped_to_the_grade_ceiling`, `a_ring_smaller_than_the_tightest_arc_becomes_a_crossing`, `joining_roads_end_on_the_ring_at_the_plane_height`.
- [ ] **Step 2–4:** FAIL, implement, PASS, measure (also the roundabout site's render with `road-analysis`), `make check`. **Step 5: commit** `feat: roundabouts are placed as rings on a plane that follows the slope`.

### Task 8: Gap repair by moving the junction (spec 2.3)

**Files:** Modify `crates/naves-core/src/roadplace.rs` (`place_chain`/`place_unit`/`report_gap`); Test `crates/naves-core/tests/roadplace.rs`

**Interfaces:** Constants `JUNCTION_SHIFT_STEP_M: f64 = 6.25`, `JUNCTION_SHIFT_MAX_M: f64 = 25.0` in `roadplace.rs`. When a side road's fit misses its port by more than `GAP_TOLERATED_M`, try the junction at shifts `+6.25, −6.25, +12.5, −12.5, …, ±25` m along its through road (this order, for determinism), refitting the through road from its previous junction for each; keep the first shift whose worst gap is ≤ `GAP_TOLERATED_M`, else the smallest worst gap; never make the through road's own gaps worse. Then look at whether the same repair helps through roads that miss their own junction point (research section 8: 43 % of the gaps) and apply it if it does; write what you found in the research subsection.

- [ ] **Step 1: Failing tests:** `a_side_road_that_cannot_close_closes_after_the_junction_moves` (synthetic network built so that the port at the OSM position cannot be met but one 6.25 m further can), `the_shift_never_exceeds_25_m`, `the_through_road_gaps_do_not_get_worse`, `the_same_network_gives_the_same_shift`.
- [ ] **Step 2–4:** FAIL, implement, PASS, measure (`road_gap` below 388), `make check`. **Step 5: commit** `feat: a side road that cannot close moves its junction along the through road`.

### Task 9: Bridges joined to their road (spec 2.4, first bullet)

**Files:** Modify `crates/naves-core/src/roadplace.rs` (`place_bridge`, the approaches); Test `crates/naves-core/tests/roadplace.rs`

**Interfaces:** No new public items. First find out on the baseline sites with bridges (Checkpoint A: the bridge sites; `road-analysis` gives the pieces next to each deck) why approaches end offset sideways or with a step; write the cause in the research subsection before changing code. The fix makes the approach on each side close on the deck's port as a fixed end (position within `GAP_CLOSED_M`, heading within `KINK_TOLERATED_DEG`), or moves the deck along its line; your call, justified by the cause.

- [ ] **Step 1: Failing test** `both_approaches_meet_the_deck_ports` on a synthetic river crossing with a bend before the bridge: for each deck port, the adjoining piece's pose is within `GAP_CLOSED_M` and `KINK_TOLERATED_DEG`; and `the_deck_end_heights_equal_the_approach_heights` (profile).
- [ ] **Step 2–4:** FAIL, implement, PASS, measure (bridge gaps), `make check`. **Step 5: commit** `fix: bridge approaches meet the deck`.

### Task 10: Clearance under bridges over a road (spec 2.4, second bullet)

**Files:** Modify `crates/naves-core/src/roadnet.rs` (crossings without a shared node), `crates/naves-core/src/junctions.rs` and `crates/naves-core/src/profile.rs` (a lower bound on heights), `crates/naves-core/src/validate.rs`, `crates/naves-export/src/generate.rs`; Test `crates/naves-core/tests/{roadnet,profile,validate}.rs`; Docs manual (Roads), `docs/manual/src/mcp.md`, `CHANGELOG.md`

**Interfaces:**
- Produces: `pub struct OverCrossing { pub upper: usize, pub lower: usize, pub at: (f64, f64), pub upper_s: f64, pub lower_s: f64 }` and `pub fn over_crossings(graph: &RoadGraph) -> Vec<OverCrossing>` in `roadnet.rs`: stretch pairs whose lines cross with no shared node, where one is a bridge (`bridge` or a higher `layer`); the bridge is `upper`. Untagged crossings are out of scope (spec: moved to the railroad design).
- Produces: `pub const BRIDGE_CLEARANCE_M: f64 = 4.5;` and the deck thickness taken from the bridge model in the catalog (`BridgeModel`; add a field if it is not there, measured from the model's geometry, with a test on the synthetic model).
- Produces: `Rule::BridgeClearance` with id `"bridge_clearance"` and message naming the clearance reached.
- The lower road keeps its height (never lowered for this). The upper road's height at the crossing gets a lower bound `lower surface + BRIDGE_CLEARANCE_M + deck thickness`, which the junction holds and the profile solve respect, so approaches and nearby junctions may rise (maintainer). The terrain under the deck is shaped for the lower road.

- [ ] **Step 1: Failing tests:** `a_bridge_over_a_road_is_found_without_a_shared_node`, `roads_crossing_at_a_shared_node_are_not_an_over_crossing`, `the_bridge_rises_to_clear_the_road_below`, `the_road_below_keeps_its_height`, `a_junction_next_to_the_bridge_rises_with_it`, Review Focus 3: `clearance_out_of_reach_is_a_finding_and_the_solve_ends` (short approaches, grade limit binding; assert the finding and that the call returns).
- [ ] **Step 2–4:** FAIL, implement, PASS, measure (the two Berchtesgaden sites: clearance reached at Kehlsteinstraße and Silbergstraße), `make check`. **Step 5: commit** `feat: a bridge over a road keeps 4.5 m clearance`.

### Task 11: Humps on steep stretches — diagnosis (spec 2.5)

**Files:** `docs/research/2026-11-10-road-alignment.md` (subsection only); temporary measurement code only in a patch under `/var/tmp`

- [ ] **Step 1:** On the baseline sites with humps (the `road_grade` and `road_not_drivable` sites of Checkpoint A, the "ski jump" sites) and on the 50 worst `road_grade` findings, classify each: vertical bend at a fixed junction height, short pieces between patches, terrain shaping, something else. Numbers per class.
- [ ] **Step 2:** Write the classification and a proposed fix (what changes, expected effect, risk) into the research subsection. Commit `docs: what makes the humps on steep roads`.
- [ ] **Step 3:** Stop. The controller and reviewer decide the fix; it becomes Task 12 as an amendment to this plan, with its own tests.

### Task 12: Junctions follow the through road's slope (spec 2.5; research 9.8 class H2, fix 1)

Amended by the controller from Task 11 (research 9.8). Cause: every junction holds its joints at one height (a level
patch), so two junctions 14 m apart (median) on a slope leave no room for the rise (725 of 1645 `road_grade`). Pieces are
not pitched in the world file (`objects::piece_transform`: the engine drapes them on the terrain), so a tilted junction
needs only tilted holds and a tilted terrain patch; the junction models need no pitch. Rings keep their own plane (Task 7).

**Files:** Modify `crates/naves-core/src/junctions.rs`, `crates/naves-core/src/profile/holds.rs`,
`crates/naves-core/src/validate.rs` (message of `junction_too_steep`), `crates/naves-export/src/generate.rs` (patches);
Test `crates/naves-core/tests/{junctions,profile}.rs`, `crates/naves-export/tests/` (patch); Docs
`docs/research/2026-11-10-road-alignment.md` (9.8 result), manual (Roads: junctions on slopes), `CHANGELOG.md`

**Interfaces and the exact rule:**
- `Junction` gains `pub gradient: (f64, f64)` (rise per metre east, north) and `pub fn height_at(&self, point: (f64, f64)) -> f64`
  = `height + gradient.0 * (point.0 - at.0) + gradient.1 * (point.1 - at.1)` (same form as `RingPlane::height_at`).
  `height` stays the height at `at`; `repair`, `raise_floors` and the clearance floors keep moving `height` only (the
  tilt moves with it).
- `pub const JUNCTION_TILT_SHARE: f64 = 0.9;` The tilt, set once in `find_junctions` from the **original** ground `G`
  (`base.sample`):
  - the *through road* of a junction: `Piece` — the junction piece's own axis (start → end of the piece);
    `Overlay` — the top road (the stretch not in `lower`); `EndOnRoad` — the road the chain ends on; `Join` — none:
    `gradient = (0, 0)` (level, as today). A junction on a ring keeps `(0, 0)` (the ring plane holds it).
  - `t` = the unit heading of the through road at its point nearest `at`; `r = max(patch_m, 6.25)`;
  - `s = (G(at + r t) - G(at - r t)) / (2 r)`, clamped to `±JUNCTION_TILT_SHARE * grade_ceiling(family of the through road)`;
    a sample outside the map, or a non-finite one, gives `s = 0`;
  - `gradient = (s * t.0, s * t.1)`: the plane tilts along the through road only, it has no fall across it.
- `holds.rs::hold` gives each held variable `j.height_at(position of that variable)` (less `OVERLAY_DROP_M` for a lowered
  road, as today) instead of `j.height`. All five rules (`Patch`, `Piece`, `End`, `Inward`, `Arm`) go through it, so a
  junction piece's two variables now differ by `s` times their distance along `t`, and an arm's `Inward` joints lie on
  the through road's surface.
- `Link` gains `pub a_at: (f64, f64)` and `pub b_at: (f64, f64)` (the positions of the two held joints, from
  `held_links`). Everything that measures a link's rise (`repair`, the unresolved check, `junction_violations`) uses
  `|a.height_at(a_at) - b.height_at(b_at)|` over `length`; the excess `repair` corrects is still applied to `height`.
- `generate.rs` patches of `Join`, `Overlay` and `EndOnRoad` junctions get `gradient: j.gradient` (today `(0.0, 0.0)`);
  `LevelPatch` already shapes a plane (Task 7).

- [ ] **Step 1: Failing tests** (synthetic heightmap: a plane rising 10 % to the east, `City` roads):
  - `a_junction_on_a_slope_tilts_along_its_through_road` (junctions.rs): an X junction piece with its axis east–west
    → `gradient.0` within 1e-6 of `0.10`, `gradient.1` within 1e-6 of `0`; the same with the axis north–south →
    `gradient` `(0, 0)` within 1e-6.
  - `the_tilt_stops_at_the_share_of_the_grade_ceiling`: the plane at 40 % → `gradient.0 == 0.9 * grade_ceiling(City)`
    within 1e-9.
  - `a_join_of_ending_roads_stays_level`: three roads ending at one node (`Join`) on the 10 % plane → `(0, 0)`.
  - `two_junctions_14_m_apart_on_a_slope_leave_the_road_drivable` (profile.rs): through road east–west on the 10 %
    plane, two T junction pieces whose centres are 14 m apart (the median of 9.8) → no `road_grade` finding of the
    solve between them, and the held joints of each piece lie within 1 cm of the plane `height_at` their positions.
    RED today (level pieces: vertical bend over the limit).
  - `a_tilted_junction_patch_shapes_a_plane` (naves-export): the terrain patch of an `EndOnRoad` junction on the 10 %
    plane leaves the terrain inside the patch within 2 cm of the original plane.
  - `a_link_measures_the_rise_between_its_held_joints`: two junctions whose centres are level but whose held joints
    differ by 1 m over 5 m (tilts) → the link's rise is 1 m, not 0 (`junction_too_steep` for `City`).
- [ ] **Step 2:** Run them, capture RED (save the output under `/var/tmp/naves-roadlook/t12/`).
- [ ] **Step 3:** Implement as above.
- [ ] **Step 4:** PASS; `make check`; then `road-analysis` at 2048 once: the six counts plus `junction_too_steep`, and
  `road_grade` split into bends and grades. Expect `road_grade` to fall well beyond the ±30 noise (9.8: up to about 650
  of the H2 class); `road_misses_terrain`, `road_not_drivable` and `road_conflict` no higher than +50. If `road_grade`
  falls by less than 100, report DONE_WITH_CONCERNS with the classes of 9.8 recounted (the instrumentation patch is in
  `/var/tmp/naves-roadlook/t11/hump-dump.patch`). Write the counts into 9.8 as "Result of fix 1".
- [ ] **Step 5: Commit** `fix: junctions on a slope tilt with their through road`.

### Task 13: Joints less than a metre apart are one variable (research 9.8 class overlap, fix 2)

Cause: 604 overlapped 6.25 m straights put two joints a median 0.28 m apart; the profile solves them as two variables
and the checks read a 5 cm difference as a 10 % grade and a bend hundreds of times the limit (about 380 findings). The
pieces drape on the terrain (no pitch), so the fix is in the profile only.

**Files:** Modify `crates/naves-core/src/profile.rs` (strip building, the variable map); Test `crates/naves-core/tests/profile.rs`;
Docs 9.8 (result of fix 2), `CHANGELOG.md`

**Interfaces and the exact rule:**
- `pub const MERGE_JOINT_M: f64 = 1.0;` (equal to `MIN_STENCIL_M`; keep both names, the stencil one documents the smoothing).
- Building a chain's variables: walking the joints in order, a joint whose distance along the chain to the joint that
  opened the current variable is less than `MERGE_JOINT_M` maps to that variable (as the joints of a bridge deck share
  one variable today). A merge never crosses a bridge deck's variable or a joint held to a different junction than the
  variable's (then it opens a new variable). The strip keeps every joint, with the shared variable's height.
- Every consumer that reads the strip's joints in order (grade, bend, `road_grade` excess finder, `road_not_drivable`,
  terrain shaping) then sees equal heights at merged joints; the excess finder skips a run under `MERGE_JOINT_M`
  between two joints of one variable (no grade, no bend there) and measures the bend across the merged pair as one
  joint (the runs before and after).

- [ ] **Step 1: Failing tests:**
  - `an_overlapped_straight_gives_one_variable`: a chain of a 12.5 m straight then a 6.25 m straight starting 6.0 m before
    the first one's end (joints 0.25 m apart), on a 5 % plane → the two joints have equal heights (within 1e-9) and
    the solve reports no `road_grade` finding. RED today.
  - `joints_a_metre_apart_stay_two_variables`: the same with the overlap 5.25 m (joints exactly 1.0 m apart) → two
    variables (heights may differ).
  - `a_merge_does_not_cross_a_held_joint`: two joints 0.3 m apart held to two different junctions → two variables,
    each with its junction's height.
- [ ] **Step 2–4:** RED captured, implement, PASS, `make check`, `road-analysis` at 2048 once (same counts as Task 12).
  Expect the F2 class and most of F1 to go (9.8: about 280); every other count within noise.
- [ ] **Step 5: Commit** `fix: joints closer than a metre share one height`.

**Not in this plan (controller ruling, research 9.8):** H3, H1 (a hold on one side, incl. the 540/187/95 % sites),
F1/F3 on free joints and the bridge-deck class D (site 23's ski jump; measuring the bend at the deck ends) go to the
joint height solve plan that follows: the QP removes H3/H1/F1 and holds the deck as variables. The junction plane of
Task 12 becomes a variable there (height and tilt along the through road, bounded by the same share).

### Checkpoint B (controller): the stage 2 game check and the figures

Build the example, run the tool with `/var/tmp/naves-roadlook/baseline/sites.json`, put before and after side by side (contact sheets). Update `examples/berchtesgaden/README.md` (figures and the in-game results) and the CHANGELOG. The maintainer looks at the sheets.

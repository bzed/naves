<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Network Clean-up Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Clean the road graph once, before anything is built on it: remove sidepaths that disturb, stubs, merge
junctions the terrain cannot tell apart, give bridges an approach, then smooth the lines.

**Architecture:** A new module `crates/naves-core/src/netclean.rs` holds four steps, each a pure function from a
`RoadGraph` to a `RoadGraph` and a list of what it did. `naves-export`'s `generate::fitted_graph` calls them between
`drop_duplicates` and `fit_bridges`; the smoothing (`smooth.rs`, its own plan) follows as step 5. Each step is
measured on its own before the next is built.

**Tech Stack:** Rust 1.99; no new dependency planned (AGENTS.md's crate review applies if one seems needed: `geo`
and `rstar` are the first to check for nearest-point and spatial index work, and what the repo already uses comes
first).

**Spec:** `docs/superpowers/specs/2026-11-18-network-cleanup-design.md`

**Follows:** Tasks 2 to 5, the checkpoint and the final review of
`docs/superpowers/plans/2026-11-17-naves-road-smoothing.md` run after Task 6 of this plan, on the same branch. That
plan's Task 1 (baseline) is replaced by Task 1 here. Its constraint "junction nodes do not change" means: the
junctions as this plan leaves them.

**Implementers:** Sonnet. **Reviews:** Opus (maintainer, 2026-11-17).

## Global Constraints

- AGENTS.md binds: test first (RED then GREEN), `make check` green before every commit, coverage not below the value
  on the base commit, no panics on input, `#[expect(lint, reason)]` never `#[allow]`, no truncating casts, sort with
  `total_cmp`, deterministic output (no `HashMap` iteration order), docs and CHANGELOG in the behaviour commit, no
  game data in the repo.
- Every process gets a hard memory limit: `systemd-run --user --scope --quiet -p MemoryMax=12G -p MemorySwapMax=1G`
  for cargo and `make check`, 16G for `road-analysis`, server 8G, client 14G, `ingame-check` 24G. Never raise a cap;
  report an OOM kill.
- Temp and build folders under `/var/tmp` (never `/tmp`, never `$HOME`). Shared target dir `/var/tmp/naves-target`,
  never deleted.
- Never start the game outside the checkpoint task. Wine/Proton only inside headless sway. Never touch the user's
  DayZ profile folder. Remove `@naves*` symlinks from the DayZ install afterwards.
- Figures are quoted for Berchtesgaden at terrain size 2048.
- Constants, verbatim from the spec: `SIDEPATH_MAX_M = 15.0`, `SIDEPATH_MIN_RUN_M = 30.0`, `STUB_MAX_M = 6.0`,
  `SPECK_M = 0.5`, `ROUNDABOUT_KEEP_M = 10.0`, `JUNCTION_MERGE_M = 6.0`, `APPROACH_M = 8.0`. Each step has a `…_ENABLED: bool` constant.
- The number of connected components of the graph never grows in any step.
- New node ids are above all existing ids, handed out in sorted order; cut pieces are named `<id>/<word><n>` with a
  word no other stage uses (`/rest` and `/after` are taken).
- Every change is reported; nothing is removed or moved silently.
- A step that is switched off returns the graph unchanged, bit for bit.

## Review Focus

- A graph stretch that joins a sidepath in the middle of the removed run must still reach the road
  afterwards (the maintainer's caution at Bergwerkstraße, way 39674984).
- A sidepath that crosses its road, changes sides, or runs between two roads: not a one-sided run, must not be
  removed as one.
- A cluster of close junctions that contains two bridge ends, or a roundabout node: no merge, reported, no panic.
- A bridge shorter than `3 · APPROACH_M` with a T-junction on its end, and one with a road passing under the part
  that would be cut off: unchanged and reported as `bridge_end_at_junction`.
- Degenerate input in every step: zero-length stretches, duplicate points, NaN coordinates, a stretch whose two ends
  are the same node.

---

### Task 1: Baseline (done, 2026-11-18)

Done before this branch was made. The branch starts from `ramps` (ba6ce1b), so the rule counts to compare with are
those of the ramps run, `/var/tmp/naves-ramps2/ra-2048.log` and `rules.tsv`: road_not_drivable 182,
road_misses_terrain 4175, road_grade 11, road_gap 356, road_kink 21, road_conflict 845, bridge_clearance 1,
bridge_blocks_road 3, bridge_buried 8, bridge_end_step 3, join_without_junction 65, road_duplicate_dropped 9. The graph
figures were measured on `main` at da88a23 (`/var/tmp/naves-clean/baseline/baseline.md`; the ramps work does not change
the graph): 2959 stretches, 1694 nodes, 22 connected components, 257 797 m of roads of which 243 115 m drivable, 1405
junction nodes, 96 junction pairs joined by one stretch under 6 m, 152 bridge end nodes of which 7 have three or more
stretches; the smoothing figures are in the same file. The steps below are kept for the record.

#### As planned


**Files:** none in the repo. Output under `/var/tmp/naves-clean/baseline/`.

- [ ] **Step 1:** On `main` after the bridges merge (log `git rev-parse HEAD` and the `naves` binary's mtime), run
  `road-analysis` on Berchtesgaden at 2048 once (16G cap; `RULES_OUT=…/rules.tsv`), log `baseline/ra-2048.log`.
- [ ] **Step 2:** Apply `/var/tmp/naves-smooth/recon.patch` in a scratch worktree under `/var/tmp` (own target dir
  `/var/tmp/naves-clean-target`), re-run, and record the smoothing figures (bridge-end turns p50/p95/max,
  placed-joint distance from the line, corner statistics). Do not commit the patch.
- [ ] **Step 3:** Write `baseline/baseline.md`: every rule count, connected components of the graph, total length
  and drivable length, number of junction nodes, number of junction pairs joined by a stretch under 6 m, number of
  bridge end nodes with three or more stretches, run time, and the smoothing figures.

No commit.

### Task 2: Tags and the step frame (Sonnet)

**Files:** Modify `crates/naves-core/src/osm.rs` (`WAY_TAGS`), `crates/naves-core/src/roadnet.rs` (`Stretch`),
`crates/naves-core/src/lib.rs`; create `crates/naves-core/src/netclean.rs`, `crates/naves-core/tests/netclean.rs`;
modify `crates/naves-export/src/generate.rs` (`fitted_graph`, `FittedGraph`), `crates/naves-core/src/validate.rs`
(rules), the `road-analysis` example, `docs/manual/`, `CHANGELOG.md`.

**Interfaces — Produces:**

- `WAY_TAGS` also keeps `"foot"`.
- `Stretch::use_sidepath: bool` — true for a road tagged `foot=use_sidepath`. False by default; cut pieces inherit it
  from the original stretch. (`footway=sidewalk` is not read: it sits on painted paths outside the graph.)
- `netclean::CleanReport { sidepaths: Vec<DroppedSidepath>, stubs: Vec<DroppedStub>, merged: Vec<MergedJunction>,
  approaches: Vec<Approach>, roundabouts: Vec<Ring>, components_before: usize, components_after: usize }` and
  `netclean::clean(graph: &RoadGraph, widths: &dyn Fn(Family) -> f64, cell_m: f64) -> (RoadGraph, CleanReport)`,
  which calls the four steps in order (each still a stub returning the graph unchanged in this task).
- `netclean::components(graph: &RoadGraph) -> usize`.
- `FittedGraph::clean: CleanReport`; `fitted_graph` calls `clean` after `drop_duplicates`.
- Rules `road_sidepath_dropped`, `road_stub_dropped`, `road_junctions_merged`, `bridge_approach_made`
  (information) and `bridge_end_at_junction` (warning) exist in `Rule::ALL` with manual entries; `road-analysis`
  prints `CLEAN <rule> <count>` lines and `COMPONENTS <before> <after>`.

- [ ] **Step 1: Failing tests:** OSM input with `foot=use_sidepath` on a road sets
  the flag and nothing else changes (a real-fixture test pins that `foot` is kept); `components` of two separate roads is 2 and of a T is 1; `clean` on any
  graph with all steps as stubs returns an equal graph and an empty report with equal component counts.
- [ ] **Step 1b: Failing tests (roundabouts):** `roadnet::ROUNDABOUT_KEEP_M = 10.0`; `collapse_small_rings` keeps a
  ring only if its radius is at least `ROUNDABOUT_KEEP_M` and at least the family's tightest arc (a 9 m ring of a
  family whose tightest arc is 8 m is collapsed; a 12 m ring stays); it also returns the collapsed rings
  (`Vec<Ring>`), which `fitted_graph` puts into `CleanReport::roundabouts` and the rule
  `road_roundabout_collapsed` (information) reports.
- [ ] **Step 2:** RED. **Step 3:** implement. **Step 4:** GREEN, `make check`.
- [ ] **Step 5:** Commit `feat: the frame of the network clean-up`.

### Task 3: Stubs and specks, and close junctions (Sonnet)

Steps 2 and 3 of the spec. Built before the sidepath step because that step's connectors need them.

**Files:** `netclean.rs`, `tests/netclean.rs`, manual, CHANGELOG.

**Interfaces — Produces:** `netclean::drop_stubs(graph, widths) -> (RoadGraph, Vec<DroppedStub>)` with
`DroppedStub { stretch: String, length_m: f64, at: (f64, f64) }`;
`netclean::merge_junctions(graph) -> (RoadGraph, Vec<MergedJunction>, Vec<NotMerged>)` with
`MergedJunction { kept: i64, merged: Vec<i64>, removed: Vec<String>, at: (f64, f64) }` and
`NotMerged { nodes: Vec<i64>, reason: &'static str, at: (f64, f64) }`.

- [ ] **Step 1: Failing tests (stubs):** a 4 m dead-end track that is not drivable, off a road, is dropped and
  reported; a 20 m service road stays; a 4 m drivable dead end on a 6 m wide road is dropped, a 4 m one on a 3 m wide track stays; a 4 m
  stretch between two junctions (not a dead end) stays; points 0.2 m apart inside a stretch become one, the end
  nodes keep their positions; a stretch that is only two points 0.2 m apart is kept whole.
- [ ] **Step 2: Failing tests (junctions):** two T-junctions 4 m apart on a main road with one side road each
  become one four-way node at the node whose roads rank highest; the 4 m stretch is gone and reported; the side
  road's end is at the merged node and its line is straight for its last 6 m; two junctions 10 m apart stay; three
  junctions in a row 5 m apart (ends 10 m apart) merge, four in a row (ends 15 m apart) do not and are reported; a
  cluster with one bridge end merges onto the bridge end; a cluster with two bridge ends is reported and unchanged;
  roundabout nodes are never merged; components equal before and after in every case; the result is the same when
  the input stretches are given in reverse order.
- [ ] **Step 3:** RED. **Step 4:** implement both, wire into `clean`. **Step 5:** GREEN, `make check`.
- [ ] **Step 6:** Run `road-analysis` at 2048 (log `/var/tmp/naves-clean/t3/ra-2048.log`, `rules.tsv`) and write
  `t3/after.md`: the lists of what was dropped and merged, every rule count beside the baseline, components, run
  time. Do not tune.
- [ ] **Step 7:** Commit `feat: stubs are dropped and junctions too close to tell apart become one`.

### Task 4: Sidepaths (Sonnet; may use one Opus helper for the reconnection)

Step 1 of the spec.

**Files:** `netclean.rs` (or `netclean/sidepath.rs` if the file passes about 600 lines), `roadnet.rs` only to make
the duplicate drop's sampling and cutting helpers reusable (no behaviour change there; its tests stay green
untouched), `tests/netclean.rs`, manual, CHANGELOG.

**Interfaces — Produces:** `netclean::drop_sidepaths(graph, widths, cell_m) -> (RoadGraph, Vec<DroppedSidepath>,
Vec<KeptSidepath>)` with `DroppedSidepath { stretch: String, road: String, from_m: f64, to_m: f64, at: (f64, f64),
connectors: Vec<(String, (f64, f64))> }` and `KeptSidepath { stretch: String, road: String, reason: &'static str,
at: (f64, f64) }` (reasons: `"clear"`, `"would_disconnect"`, `"crosses"`).

Rules, from the spec: candidate = within 15 m on one side for at least 30 m, or any length where the road has `use_sidepath`; removed
only if the two earthworks (half width plus shoulder ring each; take the ring width the conform stage uses, do not
invent one) come closer than `cell_m`, or the run passes a bridge of the road; joining stretches get a straight
connector to the nearest point of the road with a new node there, or the piece of sidepath is kept if the connector
would cross a third road or be longer than 15 m; the two ends of the removed run are joined the same way.

- [ ] **Step 1: Failing tests:** a lower-family stretch 5 m beside a road for 100 m, joining it at both ends, is removed and
  reported, components unchanged; the same stretch at 14 m with narrow roads (earthworks a cell apart) stays with
  reason `"clear"`; at 14 m beside a bridge of the road it is removed along the bridge; a stretch 5 m beside for 20 m
  only stays, and is removed when the road has `use_sidepath`; a stretch that crosses the road halfway is not removed
  across the crossing; a graph stretch that joins the sidepath in the middle of the run ends on the road afterwards
  (a node on the road within 15 m of the old joint, connector straight) and the far end of that stretch is still
  reachable from the road; when a third road lies between, the piece of sidepath is kept instead; a gravel track
  beside a paved road is a candidate like any lower-family stretch, a paved road beside a gravel track is not removed; a sidepath
  between two parallel roads within 15 m of both is removed against the nearer; result independent of input order.
- [ ] **Step 2:** RED. **Step 3:** implement, wire into `clean` before `drop_stubs`. **Step 4:** GREEN,
  `make check`.
- [ ] **Step 5:** Run at 2048 (`/var/tmp/naves-clean/t4/`), write `t4/after.md` as in Task 3 plus: the full list of
  removed and kept candidates with reasons, and for way 39674984's road (Bergwerkstraße, map (6836, 7676)) what
  happened to the sidepath and to each stretch that joined it, with coordinates.
- [ ] **Step 6:** Commit `feat: sidepaths that disturb their road are removed`.

### Task 5: Bridge approaches (Sonnet)

Step 4 of the spec.

**Files:** `netclean.rs`, `tests/netclean.rs`, manual, CHANGELOG, the bridges spec (one sentence under "Changes
during implementation": a junction on a deck end is moved off it by the clean-up).

**Interfaces — Produces:** `netclean::make_approaches(graph) -> (RoadGraph, Vec<Approach>, Vec<NoApproach>)` with
`Approach { bridge: String, end: u8, kind: ApproachKind, moved: Vec<String>, at: (f64, f64) }`,
`ApproachKind::{SideRoadsMoved, BridgeShortened}`, `NoApproach { bridge: String, end: u8, reason: &'static str,
at: (f64, f64) }` (reasons: `"bridge_too_short"`, `"road_below"`, `"no_room"`).

- [ ] **Step 1: Failing tests:** a bridge whose road continues, with a side road on its end node: the side road now
  joins 8 m beyond, the end node has two stretches, the bridge stretch is unchanged; the same when the continuing
  stretch is only 5 m long before the next junction: `"no_room"`, unchanged; a 40 m bridge ending in a T: the
  bridge stretch is 32 m, an 8 m plain stretch of the same family and way joins it to the T, `bridge` false on it;
  a 20 m bridge ending in a T: `"bridge_too_short"`; a 40 m bridge ending in a T with a road passing under its last
  5 m: `"road_below"`; a bridge end with only two stretches: untouched, no report; both ends of one bridge handled
  independently; components unchanged; result independent of input order.
- [ ] **Step 2:** RED. **Step 3:** implement, wire into `clean` last. **Step 4:** GREEN, `make check`.
- [ ] **Step 5:** Run at 2048 (`/var/tmp/naves-clean/t5/`), write `t5/after.md` as in Task 3 plus the list of all
  bridge ends with three or more stretches and what happened to each, and `bridge_buried`, `bridge_end_step` and
  `road_conflict` at the named sites (39674984, 48450084, 240795460, 74208396, 299428041, 286196692, 130657216).
- [ ] **Step 6:** Commit `feat: a junction on a bridge's end is moved off the deck`.

### Task 6: Judge the four steps (controller with an Opus reviewer; Sonnet for changes)

- The controller reads `t3`, `t4`, `t5` against the baseline. A step whose run made a finding count worse without a
  reason that can be named is switched off (`…_ENABLED = false`, with its test for "switched off returns the graph
  unchanged") and listed for the maintainer. No tuning until numbers look right.
- **Checkpoint, the game:** build from this branch (rebuild the `naves` binary first; log HEAD and the binary's
  mtime), `tools/ingame-check` on: the seven named bridge sites, three removed sidepaths (one of them
  Bergwerkstraße with the stretch that joined the sidepath), three merged junctions, two shortened bridges. Compare with the shots in
  `/var/tmp/naves-bridges-cp2/shots/`. All caps apply; one game at a time; non-default ports; remove the mod
  symlinks afterwards. Send the pairs to the maintainer.
- If the shortened bridges look wrong in the game: the T case of step 4 is switched off; those sites stay reported.

Then continue with Task 2 of the smoothing plan.

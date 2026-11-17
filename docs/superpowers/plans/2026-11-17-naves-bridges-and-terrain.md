<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Bridges and Terrain Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every OpenStreetMap bridge is built from a part that fits its length, covers what passes below it, and stands on
terrain shaped for it.

**Architecture:** The catalog learns more parts (concrete panels, `bridge_wood2`, `bridge_concrete3`, the concrete pipe)
and, per part, what hangs below its deck. A pure function chooses what each bridge is built from by its length, width
and ground; the road graph grows a bridge only along a straight line. Placement lays panel rows and culverts besides
models. Bridges over roads are fitted to the road below. `conform` holds the ground under every deck below its
underside.

**Tech Stack:** Rust 1.99, the existing crates (`naves-formats`, `naves-core`, `naves-export`); no new dependency.

**Spec:** `docs/superpowers/specs/2026-11-16-bridges-and-terrain-design.md` (binding; read it in full before any task).
Measurements behind it: `/var/tmp/naves-bridgerecon/report.md`, `report2.md`, `bridges.csv`, `bridges2.csv`, and the
throwaway code that made them, `/var/tmp/naves-bridgerecon/recon.patch` (an ODOL reader that keeps faces, examples
`bridge-models.rs` and `bridge-dump.rs`): a source of method, not code to copy unreviewed.

## Global Constraints

- AGENTS.md in full. Test first (RED captured, then GREEN). `make check` before every commit, coverage ≥ 85 %.
- Library code never panics on input: no `unwrap`, `expect`, indexing; `#[expect(lint, reason)]`, never `#[allow]`.
  Model files are untrusted input: checked arithmetic, limits before allocation, a fuzz target for every parser change.
- Deterministic: same input, bit-identical output. No `HashMap` order.
- No game data in the repository, tests or docs: synthetic models only (`catalog.rs` has `synthetic_models*`).
- Every long process runs under `systemd-run --user --scope --quiet -p MemoryMax=<N>G -p MemorySwapMax=1G` (cargo and
  `make check` 12G, `road-analysis` 16G, a map build's sway 16G, `tools/ingame-check/run.sh` 24G). Never raise a cap;
  report an OOM kill. Wait on a PID or a marker file, never a `pgrep -f` loop.
- Temp files under `/var/tmp`, never `$HOME`, never `/tmp`. The cargo target dir is shared; never delete it.
- No DayZ, Wine or map build unless the task says so. No push. Other sessions' processes are left alone.
- Docs (`docs/manual`) and a `CHANGELOG.md` line in the commit that changes behaviour. Conventional commits.
- The goal behind every rule: roads that are drivable and usable. A rule of this plan that makes a road less usable is
  reported, not enforced.
- All lengths are metres on the map. Constants, verbatim from the spec: `FILL_MAX_M = 3.0`, `CULVERT_MAX_M = 6.0`,
  the pipe's size from the catalog (`Pipe::diameter`, 2.63 m measured; not a constant `PIPE_M`), `COVER_M = 0.5`, `MODEL_FILL = 2.0 / 3.0`, `BRIDGE_NARROWER_M = 1.0`, `GROW_TURN_MAX_DEG = 5.0`,
  `PASS_MARGIN_M = 1.0`, `BRIDGE_AIR_M = 1.0`, `MAX_DECK_THICKNESS_M = 4.0`, `BRIDGE_CLEARANCE_M = 4.5`; a panel is
  6.0 m across the road and 3.0 m along it at scale 1; a panel's scale is `W / 6.0`, never below 1. `MAX_BRIDGE_SCALE`
  is removed.
- Model paths: `dz\structures\roads\bridges\` (`bridge_metal_25`, `bridge_stone_25`, `bridge_wood_25`),
  `dz\structures\roads\panels\` (`panel_concrete_1`, `panel_concrete_2`), `dz\structures_sakhal\roads\bridges\`
  (`bridge_wood2`, `bridge_concrete3`; archive `sakhal/addons/structures_sakhal.pbo` beside the game's `addons`),
  `dz\structures\industrial\misc\misc_concretepipe.p3d` (archive `structures_industrial.pbo`).
- Figures are quoted for Berchtesgaden at `terrain_size = 2048`. Baseline (d7cb864): 84 OpenStreetMap bridges, 65
  built, 19 "no room"; 64 spans with ground above the underside; 8 solid parts on a road below; `road_grade` 3,
  `road_misses_terrain` 4317, `road_conflict` 900, `road_not_drivable` 189, `bridge_clearance` 0; `road-analysis`
  921 s.

## Review Focus

- A model file with a Geometry LOD of zero faces, or faces that index past the vertices: an error or an empty
  `underside`, never a panic; the part is then treated as open with no solid ends, with a warning.
- An install without the Sakhal folder or without the panels: a warning, and the ladder goes on with what is there
  (a bridge that would be a panel row becomes plain road with a finding, never nothing silently).
- A bridge of zero length, or whose two ends are the same node; a bridge whose both neighbours are junctions 1 m away:
  one panel, or a fill, never a deck grown through a junction.
- A road below that crosses at a very flat angle (`sin(angle)` near 0): a bounded interval and a finding, no division
  by zero, no interval longer than the map.
- A bridge at the edge of the map, and a deck narrower than one terrain cell: `conform` touches only vertices that
  exist and never raises ground.

## File Structure

| File | Responsibility |
|---|---|
| `crates/naves-formats/src/odol.rs` | keeps the faces of the Geometry LOD |
| `crates/naves-core/src/catalog.rs` | parts: bridges, panels, pipe; `underside`, `solid`, `retains`, `reach_down` |
| `crates/naves-export/src/catalog_read.rs` | reads the three archives and their prefixes; cache stamp per archive |
| `crates/naves-core/src/bridgekind.rs` (new) | what a bridge is built from: the ladder, pure |
| `crates/naves-core/src/roadnet.rs` | growth along a straight line only; nothing lost |
| `crates/naves-core/src/roadplace.rs` | lays models, panel rows; no scaling of bridges |
| `crates/naves-core/src/bridgefit.rs` (new) | what passes below a bridge, the interval it needs, the fit |
| `crates/naves-core/src/conform.rs` | the ground at a bridge |
| `crates/naves-core/src/validate.rs` | `bridge_blocks_road`, `bridge_buried`, `bridge_post_floats` |
| `crates/naves-export/src/generate.rs` | wiring, pipe objects, the report lines |

---

### Task 1: The parts in the game (controller, with a Sonnet game agent)

A throwaway check, nothing merged. On a scratch branch or with objects added through the project's
`edits/overrides.toml` (whichever places an object with a model path, position, heading and scale; read
`crates/naves-core/src/objects.rs`), put on roads of the example, each replacing or lying on a short piece of road:

- a row of 2 `panel_concrete_1` crosswise at scale 1.0 on a gravel road;
- a row of 2 at scale 2.0 on an Asf1 road, and one of 3 at scale 1.67 on a City road;
- a `bridge_wood2` on a gravel road and a `bridge_concrete3` on an Asf3 road, deck at road height;
- a `misc_concretepipe` (and three end to end) across under a road on a fill at least 3 m high.

- [ ] **Step 1:** Build the map (fresh binary, HEAD and mtime in the build log), photograph each from eye level along
  the road, from the side and from above (`tools/ingame-check/run.sh`), and drive a vehicle over each with
  `tools/ingame-check/drive.py` if it can be pointed at a site (read it; if not, say so).
- [ ] **Step 2:** Report per part: deck height that makes the joint to the road flush (the model's top above its
  origin times the scale), whether a vehicle/the player stands on it (the camera's height over the deck is evidence),
  whether a scaled panel's roadway still carries, gaps between panels, how the part looks, and the placement recipe
  (origin offset, rotation) that worked.
- [ ] **Step 3 (controller):** A kind that cannot be driven over or looks wrong leaves the spec's table (amend the spec
  and this plan; ledger ruling); the recipe goes into Task 5's dispatch.

### Task 2: The model reader keeps the faces of the Geometry LOD (Sonnet)

**Files:** Modify `crates/naves-formats/src/odol.rs`, its fuzz target under `fuzz/fuzz_targets/`, its tests.

**Interfaces — Produces:** `OdolModel` gains `geometry_faces: Vec<[u32; 3]>` (triangles into the Geometry LOD's
vertices; quads split in two) and `roadway: Vec<[f32; 3]>` (the vertices of the Roadway LOD, empty if none or
unreadable). Existing fields unchanged.

- [ ] **Step 1: Failing tests:** a synthetic LOD with two triangles and a quad gives 4 triangles with the right
  indices; a face index past the vertices is `OdolError`; a face count larger than the remaining bytes is `OdolError`
  before any allocation; a model without a Roadway LOD gives an empty `roadway`.
- [ ] **Step 2–4:** RED, implement (the reader skips faces today at `odol.rs:320`), GREEN, a fuzz seed with faces,
  `make check`.
- [ ] **Step 5: Commit** `feat: the model reader keeps the faces of the Geometry LOD and the roadway`.

### Task 3: The catalog knows more parts and what hangs below a deck (Opus)

**Files:** Modify `crates/naves-core/src/catalog.rs`, `crates/naves-export/src/catalog_read.rs`, `roadplace.rs`
(`bridge_for`), `crates/naves-export/src/generate.rs` (the thickness callers), tests, `docs/manual`, `CHANGELOG.md`.

**Interfaces — Produces:**
```rust
pub struct Underside { pub depth: Vec<f64>, pub half_width: Vec<f64> }      // 1 m slices from the first port
pub struct Solid { pub from: f64, pub to: f64, pub half_width: f64, pub depth: f64 }
// BridgeModel gains: underside: Underside, solid: Vec<Solid>, retains: (bool, bool), reach_down: f64
impl BridgeModel { pub fn thickness(&self, from: f64, to: f64) -> f64; pub fn opening(&self) -> (f64, f64); }
pub struct Slab { pub model: String, pub along: f64, pub across: f64, pub top: f64, pub thickness: f64 }   // a panel
pub struct Pipe { pub model: String, pub length: f64, pub diameter: f64 }
impl PieceCatalog { pub fn panels(&self) -> &[Slab]; pub fn pipe(&self) -> Option<&Pipe>; }
pub const BRIDGE_NARROWER_M: f64 = 1.0;
pub fn bridge_for(catalog: &PieceCatalog, family: Family) -> Vec<&BridgeModel>;   // every model that fits the width, shortest first; no scale
```
`deck_thickness` stays as the largest `underside` in the opening (so callers that are not yet moved keep working);
`MAX_BRIDGE_SCALE` and every scale of a bridge model go. `road_catalog` reads the three archives of Global
Constraints; a missing archive or part is a warning in the catalog (`PieceCatalog::warnings`), the cache stamp covers
every archive read.

- [ ] **Step 1: Failing tests** (synthetic models): a deck with end blocks gives `solid` at both ends, `retains`
  `(true, true)`, an opening between them and `thickness` over the opening equal to the slab; an arch gives a depth
  that grows from the crown to the springings and a `thickness` over a narrow middle interval smaller than over a wide
  one; posts give `retains (false, false)`; a model with no faces is open with a warning; a panel and a pipe are sized
  from their LODs; `bridge_for` returns a model 0.9 m narrower than the road and not one 1.1 m narrower, and never a
  scale; the catalog without the Sakhal archive has no `bridge_wood2` and a warning.
- [ ] **Step 2–4:** RED, implement, GREEN, `make check`. Old tests of scaling are ported or dropped with a line each.
- [ ] **Step 5: Commit** `feat!: the catalog reads panels, more bridges and the pipe, and what hangs below a deck`.

### Task 4: What a bridge is built from, and growth along a straight line (Opus)

**Files:** Create `crates/naves-core/src/bridgekind.rs`; modify `roadnet.rs` (`widen_bridges`, `bridge_room`,
`lost_bridges`), `generate.rs` (`widened_graph`), tests.

**Interfaces — Produces:**
```rust
pub enum BridgeKind { Fill, Culvert { pipes: usize }, Panels { count: usize, scale: f64 }, Model { model: String, count: usize } }
pub struct BridgeSite { pub length: f64, pub width: f64, pub family: Family,
    pub depth_at_ends: (f64, f64),            // original ground below the road at both ends of the span
    pub ground_under: Vec<f64>,               // original ground below the deck level, per metre along the span
    pub room: (f64, f64) }                    // straight room beyond each end
pub fn bridge_kind(catalog: &PieceCatalog, site: &BridgeSite) -> BridgeKind;       // the spec's table
pub fn straight_room(graph: &RoadGraph, stretch: usize) -> (f64, f64);            // to a junction, another bridge, or GROW_TURN_MAX_DEG of turn; 1 m kept
pub fn fit_bridges(graph: &RoadGraph, kinds: &dyn Fn(&BridgeSite) -> BridgeKind, ground: ...) -> (RoadGraph, Vec<(usize, BridgeKind)>);
```
`fit_bridges` replaces `widen_bridges`: it grows a bridge stretch to its kind's length within the straight room, falls
to the next shorter kind when it does not fit, marks a `Fill` and a `Culvert` as plain road (remembering the culvert),
and loses no bridge. `lost_bridges` goes with its warning.

- [ ] **Step 1: Failing tests:** the table's lines one by one (3.0 → `Fill`, 3.1 with shallow ground → `Panels 1`,
  5 m with 3 m of depth → `Culvert`, 12 m on a 6 m road → `Panels { count: 4, scale: 1.0 }`, 12 m on a 12 m road →
  `Panels { count: 2, scale: 2.0 }`, 13.2 m gravel → `bridge_wood2`, 14.1 m Asf3 → `bridge_concrete3`, 16.7 m Asf1 →
  `bridge_metal_25`, 22 m gravel with ground 4 m down → `bridge_wood2` not `bridge_wood_25`, 60 m Asf1 → a chain of
  3); a bridge between two bends grows 0 and becomes panels; room on one side only → the other side takes the rest; a
  grown line never turns more than 5°; a catalog without panels → a model if it fits, else `Fill` with a finding
  hook; every bridge of the input is in the output list.
- [ ] **Step 2–4:** RED, implement, GREEN, `make check`.
- [ ] **Step 5: Commit** `feat!: a bridge is built from the part that fits its length, grown only along a straight line`.

### Task 5: Laying panel rows, culverts and the new models (Sonnet)

**Files:** Modify `roadplace.rs` (`place_bridge`, `choose_bridge`), `objects.rs`, `generate.rs` (`place_once`,
`plan_roads`), `profile.rs` (a panel row is a deck), tests, `docs/manual`, `CHANGELOG.md`.

- Consumes Task 3 and 4, and Task 1's placement recipe (the controller puts it into the dispatch).
- A `Panels` bridge places `count` objects along the bridge's chord, crosswise, with `scale`, top at the deck level;
  it is one level deck for the height solve; its thickness is the panel's times the scale; it has no solid part.
- A `Culvert` is plain road; after the heights are solved, `pipes` pipe objects are laid end to end across under the
  road at the lowest original ground of the span, the row centred on the road, heading square to it, top at least
  `COVER_M` below the road; if the solved road is lower than that, no pipe is laid (plain road, a report line).
- The report (`build/report.json` and the build's summary) says for every OpenStreetMap bridge what it was built from.

- [ ] **Step 1: Failing tests:** a 9 m bridge on a gravel road gives 3 panel objects 3 m apart with the heading of the
  chord plus 90°, scale 1, their tops at the deck level, and no road piece under them; an Asf1 one has scale 2 and
  objects 6 m apart; the strip's bridge segments cover exactly the row; a culvert gives road pieces through and the
  pipes below them; a `Fill` gives plain road and a report line; no object of a bridge has a scale other than 1 except
  panels.
- [ ] **Step 2–4:** RED, implement, GREEN, `make check`.
- [ ] **Step 5: Commit** `feat: panel rows, culverts and the short bridge models are laid`.

### Task 6: A bridge over a road covers it (Opus)

**Files:** Create `crates/naves-core/src/bridgefit.rs`; modify `roadnet.rs` (`over_crossings`: also a road from the
bridge's end node that runs under the deck), `bridgekind.rs` / `fit_bridges` (the fit as a further constraint),
`profile/model.rs` (thickness from `underside` over the road below's interval), `validate.rs`
(`bridge_blocks_road`), tests, docs.

**Interfaces — Produces:**
```rust
pub struct Below { pub station: f64, pub half: f64 }                 // the interval a road below needs, along the span from its first port
pub fn below_interval(width: f64, angle: f64, part: &PartShape, clearance: f64) -> f64;   // the spec's footprint + margin + skew or wall run; bounded
pub fn fit_over(part: &PartShape, length: f64, room: (f64, f64), below: &[Below]) -> Fit;  // Slide(by) | Panels { count } | Blocked { missing }
```
- [ ] **Step 1: Failing tests:** the four measured crossings as synthetic cases (a 5 m track square to a 25 m truss at
  station 22.2 → slid by about −9.7 m within 11.2 m of room; a 10 m road at 45° under a truss with 1 m of room → a
  panel row if the straight room allows its length, else `Blocked` with the metres); `sin(angle)` of 0.01 → a bounded
  interval and `Blocked`; the height solve's clearance uses the arch's depth over the road's interval, not its crown.
- [ ] **Step 2–4:** RED, implement, GREEN, `make check`.
- [ ] **Step 5: Commit** `feat: a bridge over a road is placed so that the road passes through its opening`.

### Task 7: The ground at a bridge (Opus)

**Files:** Modify `crates/naves-core/src/conform.rs` (`bridge_shadow` goes), `validate.rs` (`bridge_buried`,
`bridge_post_floats`), `generate.rs`, tests, docs.

- The spec's section "The terrain at a bridge", rule by rule. A ceiling per vertex under each deck from the part's
  `underside`; the end ramp where the part does not retain; a road below keeps its core; never above the original
  ground; never a vertex of another road's core.

- [ ] **Step 1: Failing tests** (synthetic heightmaps): flat ground at deck level under a panel row → every vertex
  under the deck at most `deck − thickness − 1.0`, the cut open to both sides with the cutting slope, the vertices at
  the ports at road level; a truss with blocks → the ceiling also under the blocks; ground already lower stays; a road
  crossing under the deck keeps its height and the bridge's cut does not raise it; another road's core beside the
  bridge is not lowered and `bridge_buried` reports what stays; a wooden model over a 4 m dip → `bridge_post_floats`;
  a deck narrower than a cell and a bridge at the map's edge do not panic and change only existing vertices.
- [ ] **Step 2–4:** RED, implement, GREEN, `make check`.
- [ ] **Step 5: Commit** `feat!: the ground under a bridge is shaped for the bridge`.

### Task 8: Measure, document (Sonnet)

- [ ] **Step 1:** `road-analysis` at 2048 once (16G cap); extend the example to print per bridge: OSM length, kind,
  growth at each end and its turn, ground above the ceiling, solid parts on a road below; and the counts of the spec's
  "How it is judged" 2 and 3 against the baseline of Global Constraints.
- [ ] **Step 2:** Manual (what a bridge is built from and why, the three findings, what to do about each), research
  note (a new section with the figures), CHANGELOG. No tuning: a judged number that is worse is reported.
- [ ] **Step 3: Commit** `docs: bridges by length, over roads, and their ground, as measured`.

### Checkpoint (controller)

Game check with `tools/ingame-check` on the spec's "How it is judged" 4: the four bridges over roads from above and
below, way 495868617, the five worst buried spans of the first measurement, one bridge of each kind, a drive over each
kind. Fresh binary, memory caps, the maintainer's client settings untouched. A site that is worse than before, or a
bridge that cannot be driven, becomes a fix task before Task 9.

### Task 9: Figures (Sonnet)

`examples/berchtesgaden/README.md`: a dated section with what each of the 84 bridges became, the counts, the times and
the game results. Commit `docs: the bridges of the Berchtesgaden example`.

### Final review

Whole-branch review on the most capable model, then superpowers:finishing-a-development-branch.

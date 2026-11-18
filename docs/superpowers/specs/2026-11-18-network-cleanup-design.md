<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Cleaning the road network before it is built

Status: design, 2026-11-18. Written by the assistant; the maintainer agreed to the direction on 2026-11-18 ("Agreed"
to: merge bridges as it is, then one network clean-up stage that holds sidepath removal and smoothing together,
heights on a sampled centre line decided afterwards, pieces fitted last, the GUI for the rest). Decisions the
maintainer made are marked **(maintainer)**; everything else is the assistant's call and easy to change.

This design contains the smoothing design of 2026-11-17 (`2026-11-17-road-smoothing-design.md`) as its last step,
unchanged except where said. Figures are Berchtesgaden at terrain size 2048 and are re-measured on `main` after the
`bridges` merge, before any code changes.

## Why

OpenStreetMap is drawn for maps. What it holds is right on a map and cannot be built at our resolution: the terrain
cell is 4 m, roads are placed at full width on a map scaled 0.5, and every road shapes the ground beside it.

- A footpath or track drawn beside a road lies on or against that road. It holds terrain, makes conflicts, and under
  a bridge it holds the ground up into the deck (Bergwerkstraße, way 39674984).
- Junctions a few metres apart have overlapping junction patches. The height solve and the placement each need an
  exception for them.
- A junction drawn exactly on a bridge's end node leaves the bridge no approach: the deck's end, the junction plane
  and the side road's earthwork all want the same few cells (the T-junction of Bergwerkstraße with way 331358138;
  ways 48450084, 240795460, 299428041).
- Lines are point-to-point with kinks nobody sees on a map (the smoothing design).

So far each of these was met downstream by a rule in the placement, the height solve or the earthwork, and reported
by a finding when the rule could not help. This stage removes the cause from the graph once, before anything reads it.

## Goal

One cleaned road graph that every later stage shares. Roads stay drivable and usable; that is the only goal for
roads **(maintainer)**.

- Everything that could be driven or walked from A to B before can still be reached afterwards.
- Every change to the network is reported with the way it concerns and where.
- Each step can be judged on its own.

## Non-goals

- The height solve is not changed here. Whether heights move to a sampled centre line is decided after this stage is
  measured.
- No change to how pieces are chosen or fitted **(maintainer: geometry first, pieces later)**.
- No finer elevation data **(maintainer)**.
- No user option yet. Values are named constants in one place per step; the GUI editing tools come later.
- Tunnels, railways and the other OSM features stay on their own lists.

## Anti-goals

- Losing a connection: a path that met a removed sidepath and now ends in nothing (the hiking path beside the
  Bergwerkstraße bridge **(maintainer)**).
- Removing something that did no harm ("if it doesn't disturb us and is rendered without issues - leave it"
  **(maintainer)**).
- A step that moves a road visibly away from where OSM has it (more than the corridor of the smoothing).
- Output that depends on thread timing or map iteration order.
- A second graph: no later stage reads the uncleaned graph.

## Approaches

1. **Keep fixing downstream.** Each new site needs a new rule in two or three stages. This is what the bridges work
   showed: correct rules, and still four bridge ends that needed their own diagnosis.
2. **Clean the OSM data by hand (or ask the user to).** Right for single sites and what the GUI is for later, but
   Náves has to work anywhere from global data.
3. **A clean-up stage on the graph (chosen).** A fixed sequence of small steps between `drop_duplicates` and
   `fit_bridges`, each a pure function from a graph to a graph and a list of what it did. Map makers call this
   generalisation: reduce a network to what the target resolution can show.

## Design

### Where it runs and its shape

In `generate::fitted_graph`: `build_graph` → `drop_duplicates` → **clean-up steps 1 to 5** → `fit_bridges`.

- A new module `netclean` holds steps 1 to 4, `smooth` holds step 5.
- Every step is `fn(&RoadGraph, …) -> (RoadGraph, Vec<…Report>)`, deterministic, no panics, works on sorted ids.
- The reports become findings of severity "information", one rule per step, so they appear in the build report, the
  `road-analysis` example and the GUI.
- Each step has one constant that switches it off (`…_ENABLED`), used by the measurement and by tests.
- Stretch ids stay `<lowest way id>:<start node>-<end node>`. New nodes and cut pieces follow what
  `drop_duplicates` already does: a new node gets an id above all others, handed out in sorted order, and a cut piece
  is named `<id>/<step word><n>`, so the same input gives the same ids.

### Step 0: tags

`osm::WAY_TAGS` also keeps `foot` and `footway`. `foot=use_sidepath` on a road and `footway=sidewalk` on a path are
the strongest signals for step 1 **(maintainer: Bergwerkstraße is tagged `foot=use_sidepath`)**. `width` and `lanes`
are not read yet; they are added when a step uses them.

### Before the steps: small roundabouts

**(maintainer, 2026-11-18: "maybe no small roundabouts (I would keep very big ones though, like the big two lane one
in the town)")**

`collapse_small_rings` already turns a roundabout into a plain junction when its radius is below its family's
tightest curve piece (10 m on the map today). On Berchtesgaden that is five of six; the one that stays has a radius
of 10.9 m at map (5308, 6768).

- The rule gets its own constant, `ROUNDABOUT_KEEP_M` (10.0 m on the map, about 20 m in reality at scale 0.5): a
  roundabout stays only if its radius is at least this and at least the family's tightest curve piece.
- Each collapsed roundabout is reported (`road_roundabout_collapsed`), like every other change of this stage.
- Nothing changes on Berchtesgaden unless the constant is raised; it is there so that "small" is one number.

### Step 1: sidepaths

A sidepath is a part of a stretch that runs beside a road of higher rank (the rank the duplicate drop uses).

- **Candidate (maintainer's criteria):** the part stays within `SIDEPATH_MAX_M` (15 m) of the road, on one side of
  it, without crossing it, for at least `SIDEPATH_MIN_RUN_M` (30 m). A path of any length counts if the road is
  tagged `foot=use_sidepath` or the path `footway=sidewalk`.
- **Removed only if it disturbs:** somewhere along the run the two roads' earthworks (road width plus shoulder ring)
  overlap or leave less than one terrain cell between them, or the run passes a bridge of the road (within the
  bridge's length plus its end guards). A candidate that keeps its distance stays.
- **Connections are kept:** every node on the removed part where another stretch joins is connected to the road
  instead: the joining stretch is extended by a straight connector to the nearest point of the road, where a node is
  put into the road. If the connector would cross a third road or be longer than `SIDEPATH_MAX_M`, the piece of
  sidepath between that node and the sidepath's nearer end is kept instead.
- The two ends of the removed part are joined to the road the same way, so the rest of the path still leads
  somewhere.
- After the step, the number of connected components of the graph must not have grown; if it would, that removal is
  not made and is reported.

This is the duplicate drop (`drop_duplicates`, which removes a road lying on a higher road) with a wider distance and
the "disturbs" test; it reuses that code's sampling and leaving points where it fits.

Finding: `road_sidepath_dropped` (stretch, kept road, from/to along the stretch, where).

### Step 2: stubs and specks

- A dead-end stretch shorter than `STUB_MAX_M` (6 m) that is not drivable, or is drivable and shorter than its own
  width, is removed. (A driveway of 20 m stays.)
- Points of a stretch closer than `SPECK_M` (0.5 m) to the point before are dropped; the end nodes stay.

Finding: `road_stub_dropped`.

### Step 3: junctions too close to tell apart

Two junction nodes joined by a stretch shorter than `JUNCTION_MERGE_M` (6 m, one and a half terrain cells) become
one junction.

- Clusters are built by joining such pairs; a cluster is merged only if all its nodes lie within
  `2 · JUNCTION_MERGE_M` of each other.
- The merged node sits on the node whose roads rank highest (ties: the lowest node id), so the main road does not
  move.
- The short stretches between the cluster's nodes are removed; every other stretch that ended at a cluster node ends
  at the merged node, with its last `JUNCTION_MERGE_M` re-aimed in a straight line.
- A bridge end node or a tunnel end node is never moved; if one is in a cluster, it is the merged node. Two of them
  in one cluster: no merge, reported.
- Roundabouts and rings are left to `collapse_small_rings`.

Finding: `road_junctions_merged` (the node ids, where).

### Step 4: a bridge gets an approach

A bridge end node where three or more stretches meet is a junction on the bridge's end. The step makes room of
`APPROACH_M` (8 m, two terrain cells; the length of a deck end guard) between the deck end and the junction.

- **The road continues beyond the bridge** (one of the other stretches continues it, by the chain rule): the side
  roads are taken off the end node and joined to the continuing stretch `APPROACH_M` further on, at a new node. Each
  side road's last metres are re-aimed there.
- **The road ends at the junction** (a T: the bridge's road stops at a road passing across, as at Bergwerkstraße):
  the bridge is shortened by `APPROACH_M` at that end; the cut-off part becomes an ordinary stretch of the same road.
  This is done only if the bridge stays at least `3 · APPROACH_M` long and nothing passes under the cut-off part
  (`crossings_under`). Otherwise nothing changes and the site is reported.
- A bridge whose end already has `APPROACH_M` of its own road before the next junction is not touched.

This is the one step that moves what OSM calls the bridge. It is judged on its own in the game before it stays
switched on.

Findings: `bridge_approach_made` (what was done), `bridge_end_at_junction` (a warning: no room could be made).

### Step 5: smoothing

As in the smoothing design of 2026-11-17, with these changes:

- it runs after steps 1 to 4, so the nodes it holds fixed are the cleaned junctions and the bridge ends after
  step 4;
- the nodes made by steps 1, 3 and 4 are junctions like any other.

### Order

1 → 2 → 3 → 4 → 5. Sidepath removal makes stubs and close junctions (the connectors), so those steps follow it.
Step 4 needs the final junctions. Smoothing needs the final nodes.

### What a failed step does

A step that meets something it cannot handle (a degenerate line, NaN, a cluster it must not merge) leaves that part
of the graph as it was and reports it. Nothing panics.

### Effects on other stages

- **User edits** are keyed by stretch id, preset and slot. Removed stretches and moved ends orphan edits there; this
  is reported as today (`orphaned_edit`) and noted in the changelog.
- **Bridges.** The deck-end rules of the bridges work stay. With step 4 most of them have nothing left to do at the
  sites that needed them; they are not removed in this work.
- **The parked item** "roads that end at a bridge with no room" is this design's step 4 and step 1.

### Constants

| name | value | step | meaning |
|---|---|---|---|
| `ROUNDABOUT_KEEP_M` | 10.0 | before | smallest roundabout that stays a roundabout (map metres) |
| `SIDEPATH_MAX_M` | 15.0 | 1 | furthest a sidepath runs from its road **(maintainer)** |
| `SIDEPATH_MIN_RUN_M` | 30.0 | 1 | shortest parallel run without a tag that says sidepath |
| `STUB_MAX_M` | 6.0 | 2 | longest dead end that is dropped |
| `SPECK_M` | 0.5 | 2 | points closer than this are one point |
| `JUNCTION_MERGE_M` | 6.0 | 3 | junctions joined by less than this are one junction |
| `APPROACH_M` | 8.0 | 4 | room between a deck end and the next junction |

The smoothing constants are in the smoothing design.

## How it is judged

Berchtesgaden at terrain size 2048. One baseline run on `main`, then one run after each step is switched on, so that
every step shows what it alone changed:

- what the step did: how many sidepaths, stubs, merged junctions, approaches; the full list in the log;
- connected components before and after (must not grow); total drivable length (must not fall by more than the
  removed stubs);
- all findings, in particular `road_conflict`, `road_gap`, `road_kink`, `road_not_drivable`,
  `road_misses_terrain`, `road_grade`, `bridge_buried`, `bridge_end_step`, `bridge_clearance`, `bridge_blocks_road`;
- the named sites: Bergwerkstraße 39674984 (sidepath gone, the hiking path still connected, the T-junction off the
  deck end), 48450084, 240795460, 74208396, 299428041, 286196692, 130657216;
- run time.

A step whose run makes a finding count worse without a reason that can be named is switched off and reported to the
maintainer; it is not tuned until the numbers look right.

A game check with `tools/ingame-check` after steps 1 to 4 and again after step 5: the named bridge sites, three
removed sidepaths, three merged junctions, and the smoothing design's sites.

## Open points

- "Disturbs" in step 1 is the assistant's reading of "if it doesn't disturb us". The measurement shows how many
  candidates it removes and how many it leaves; the list goes to the maintainer.
- Step 4's shortened bridge is the least certain decision. If the game check shows the abutment badly, the T case is
  switched off and those sites stay reported.
- `JUNCTION_MERGE_M` and `APPROACH_M` follow from the 4 m cell at size 2048. At other terrain sizes they should
  follow the cell; for now they are constants.
- After this stage: decide whether the height solve moves to a centre line sampled at a fixed step.

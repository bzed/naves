<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Náves: the project's own OSM copy and the feature table

Date: 2026-11-28. Status: draft for approval. Extends `2026-11-04-naves-design.md` (sections 5.1 to 5.3) and
`2026-11-27-editing-design.md` (edits as a source layer next to OSM); where this document and those disagree, this one
wins. First of three designs agreed on 2026-11-28; the others are `2026-11-28-rail-and-water-design.md` and
`2026-11-28-ground-and-vegetation-design.md`, which build on this one.

## 1. Intent

Náves will place much more from OpenStreetMap than roads: railways, rivers and lakes, forests and hedges, and later
buildings, fences, power lines and single objects. Today every build re-reads the Geofabrik extracts listed in
`naves.toml` (for the example 1.07 GB: Oberbayern and Austria), in several passes, and each pass keeps only the few
tags it knows (`osm::read_land`, `osm::read_ways`, `WAY_TAGS`). Every new feature kind would add passes and tags, and a
newer download changes the map without anyone asking for it.

**The project gets its own copy of the OSM data for its area, and every build reads only that copy.** On top of it,
**one table classifies OSM elements into feature kinds**, so that later generators find their data ready, including the
kinds that have no generator yet. That is the groundwork for the rest of the design.

Maintainer's decisions (2026-11-28):

- **Keep a copy** in the project. "We basically want to work on static data."
- **Adding areas** (moving or enlarging the map) must be possible.
- **Store it as a cut-down `.osm.pbf`** (approach A, section 4).
- **No view-only GUI layers now** for the new kinds; each comes with the work on its feature.

## 2. Goals, non-goals, anti-goals

**Goals**
- `naves osm import` writes the copy; `naves build` and the GUI read only the copy.
- The copy holds **every** element in the area with all its tags, so a new feature kind needs no new import.
- A moved or enlarged area is completed by a further import that **adds** what is missing and changes nothing that is
  there.
- One classification table from OSM tags to feature kinds, covering every kind the design mentions (section 5.4).
- The build report counts the features of every kind; the MCP server can list them.
- Place names (`place=*` with a `name`) become the location names of the world (`class Names`), the one small output
  that comes with this work.
- Builds get faster: the example's copy is about 2 MB instead of 1.07 GB of extracts.

**Non-goals**
- Generators for the new kinds (railways, water, vegetation have their own designs; buildings, fences, power lines and
  single objects come later).
- GUI layers for the new kinds.
- Downloading extracts from Náves (the user downloads Geofabrik files, as today), and live OSM updates (Overpass, the
  OSM API, diffs).
- Changing the editing model: `edits/ways.toml` stays the user's layer on top of OSM, unchanged.

**Anti-goals** (would make this a failure even if it works)
- A build that quietly reads the extracts again, or quietly gets different data than the copy holds.
- An import that changes elements already in the copy without being asked to (`--refresh`).
- An area where the copy has nothing and the build carries on as if OSM had nothing there.
- Classification that drops tags: the copy is the source, the table is only a reading of it.
- A different map from the same copy and the same edits (determinism, `AGENTS.md`).

## 3. Constraints

- `AGENTS.md` applies: test first, coverage ≥ 85 %, strict lints, no panics on untrusted input (the copy and the
  extracts are both untrusted), checked arithmetic, no allocation from length fields, a fuzz target for every parser and
  for the writer's round trip, atomic writes, `BTreeMap` ordering, `thiserror` in libraries, user docs and changelog in
  the same commit.
- **Look for a crate first** for the PBF writer (section 5.2) and review it; the commit names the crates considered.
- The editing design relies on: OSM node and way ids as plain integers below `USER_ID_BASE = 2^40`, cut ends with
  negative ids `-(way * 4096 + n + 1)` (`features::cut_id`), and fingerprints of OSM ways (tags and node list) in
  `edits/ways.toml`. The copy must keep ids, tags and node lists exactly as in the extract.
- `osmpbf` (the reader in use) can panic on damaged input; the existing `catch_unwind` guard (`osm::guarded`) stays.
- Large data under `/var/tmp` on the development machine; tests write only under a `TempDir`.
- ODbL: the copy is a derived database; the mod's credits name the source files and their dates (spec section 9).

## 4. Approaches

Considered (maintainer picked A on 2026-11-28):

- **A. A cut-down `.osm.pbf` in the project.** The same format as the extracts: the existing reader keeps working,
  osmium and JOSM can open the file to check it, adding an area is a merge by id. Needs a PBF writer. **Picked.**
- **B. A text file** (one element per line, sorted). Diff-friendly, but about 20 times the size, slower to read, a new
  format with its own parser and fuzz target, and the data does not change, so diffs gain little.
- **C. SQLite.** Spatial queries and adding areas are easy, but the file is binary and not byte-reproducible, and the
  C library complicates the Windows build. A spatial index built in memory at load time does the same job.

## 5. The design

### 5.1 What the copy holds

The **import window** is the map's area in WGS84 (`GeoWindow` of the project, as today) grown by a margin on every
side, **1 km real** **[my call]** (converted to map metres with `scale.horizontal`, as `MapGeo::window` does for
today's 100 m map margins): enough that roads, rivers and railways leaving the map keep their next nodes, so the
cut-end logic and the tangents at the edge work as they do now.

Kept, following the usual "complete ways" rule (osmium's `complete_ways` strategy):

1. every node inside the window;
2. every way with at least one node inside the window, **with all its nodes** (also those outside);
3. every relation with at least one member kept by 1 or 2; for **area relations** (`type=multipolygon`,
   `type=boundary` is *not* one) all member ways and their nodes are added too, so a forest or lake multipolygon is
   whole. Other relations (routes, boundaries, restrictions) are kept with their full member list but only the members
   already kept are present. **[my call]**: a country boundary or a long hiking route must not pull in half a country.
4. every **area way and area relation whose bounding box overlaps the window**, even with no node inside it (a forest,
   lake or meadow larger than the map that encloses it), with all its members and nodes. Today's `read_land` keeps
   exactly these (`touches`, `add_group`); without them the copy would lose land cover the build uses now. This costs
   one more pass over the extract's area ways and relations;
5. every tag, every member role, the element's version if the extract has it. Changeset, user and timestamp are
   **dropped** **[my call]**: not needed, they make the file larger, and the user name is personal data.

Not kept: elements outside the window that nothing kept refers to; deleted elements; history.

**Size.** The example area (16.4 km real, plus the margin) cut from both extracts with osmium is about 2 MB.

### 5.2 Files

```
osm/
  copy.osm.pbf     the data (sorted: nodes, ways, relations, each by id)
  copy.toml        what it is (below)
```

`copy.toml` (text, diff-friendly):

```toml
format = 1
# every import, in order; the windows together are what the copy covers
[[import]]
window = { south = 47.489, west = 12.824, north = 47.654, east = 13.069 }
sources = [
  { path = "osm/oberbayern-latest.osm.pbf", replication_timestamp = "2026-11-05T20:21:03Z", sha256 = "…" },
  { path = "osm/austria-latest.osm.pbf", replication_timestamp = "2026-11-05T20:21:03Z", sha256 = "…" },
]
added = { nodes = 412345, ways = 51234, relations = 812 }
refresh = false
```

- `replication_timestamp` comes from the extract's header (`osmosis_replication_timestamp`) when present; otherwise the
  file's modification time is recorded and marked so (`osmpbf` exposes it as
  `HeaderBlock::osmosis_replication_timestamp` through `BlobReader`; `ElementReader` skips header blocks, so it is a
  short separate read). It feeds the ODbL credits ("OpenStreetMap data of
  2026-11-05").
- `sha256` lets the build tell the user that an extract has changed since the import (information only; the build
  still uses the copy).
- The `.osm.pbf` is written **byte-for-byte reproducibly**: fixed block size, fixed compression level, no timestamps in
  the header, elements in id order. Same extracts and windows → the same bytes.
- Both files are written atomically (temp file in `osm/`, then rename), the PBF first, the TOML last: a TOML that
  names an import is a promise that its data is in the PBF.

The path and name are **[my call]**; `osm/` already holds the extracts in the example, so the copy sits next to them.
The extracts stay listed in `[sources] osm` of `naves.toml` as today: they are where an import reads from.

**The writer.** `osmpbf` reads only. Candidates to review (crates.io, 2026-11-28): `osm-io`, `pbf-craft`, `osm-pbf`,
`vadeen_osm`. The review follows `AGENTS.md` (security, maintenance, standards). If none passes, a writer of our own
for the subset we write (dense nodes, ways, relations, string tables, zlib blobs) is acceptable: the format is public
(OSM wiki, `osmformat.proto`/`fileformat.proto`), and `prost` or a hand-written protobuf encoder is small. The fuzz
target writes a random element set and reads it back with `osmpbf`.

### 5.3 Import, extend, refresh

`naves osm import [--refresh]` (CLI; the GUI gets a button in the project settings next to the sources, and the MCP
server an `osm_import` tool **[my call]**, both calling the same library function in `naves-export`).

1. Compute the import window of the current area.
2. If there is no copy: read the extracts in the window (three passes as `read_land` does today: relations, ways,
   nodes, but keeping everything) and write the copy.
3. If there is a copy and the window lies inside the windows already imported: nothing to do; say so.
4. **Extend:** otherwise read only what the copy lacks: elements of the new window that are not in the copy, by id.
   **An element already in the copy is never replaced**, even if the extract has a newer version. A way already in the
   copy whose nodes lie partly outside the old windows already has them (complete ways); a relation already in the copy
   gets no new members. The new import is appended to `copy.toml`.
5. **`--refresh`:** re-read the whole window from the extracts and replace the copy. Afterwards the import lists every
   OSM way that `edits/ways.toml` modifies or deletes whose fingerprint no longer matches, and every moved node whose
   `base` position changed (the editing design's rules then apply at the next build: edits still win, the mismatch is
   reported).

A source file named in `naves.toml` that does not exist or does not overlap the window is an error that names the file.
Several extracts are merged as today (Oberbayern and Austria overlap at the border); the same id in two extracts is one
element, chosen **as today, independent of the order of the files** (sort by id, then by content, keep the first:
`osm.rs`, "the same one whatever the order of the sources").

**Passes.** The complete-ways rule needs more passes than today's `read_land`: window nodes → ways using them → area
ways and relations by bounding box (rule 4) → relations with kept members → member ways of area relations → the nodes
of all kept ways. The import may combine passes where the PBF order allows (nodes, ways, relations); it runs once per
import, not per build, so its time (minutes on 1.07 GB) is acceptable **[my call]**; the CLI shows progress.

**Extending from changed extracts.** An extend reads the extracts listed in `naves.toml` now. If a file's `sha256`
differs from the one recorded by an earlier import (a newer download), the extend **stops** and says so: mixing two
dates can produce a way split upstream that exists both as the old whole and as a new half. The user either restores
the old file or runs `--refresh` **[my call]**, following "we basically want to work on static data". A file added to
the list since the last import is read by the extend only for the new window (it has no history in the copy); a file
removed from the list changes nothing in the copy. The build only checks the windows; a changed source list is
reported as information (`osm_sources_changed`).

**Extending changes the copy by design, but not the data a map already used**: every element that was in the copy is
byte-identical afterwards. A map built before and after an extension differs only near the new parts (and where a
feature of the new part joins an old one).

### 5.4 The feature table

One module in `naves-core` (`osmkind`, name **[my call]**) maps an OSM element to zero or more **feature kinds**, with
the attributes the later generators need. It reads only tags (and the element type: node, way, closed way, area
relation); geometry is assembled by the readers as today.

| Kind | OSM (main tags) | Attributes kept | Used by |
|---|---|---|---|
| road, path | `highway=*` (today's `features::classify`, unchanged) | as today | roads (exists) |
| lot | car parks, camp sites (today's `lot_tags`) | as today | lots (exists) |
| railway | `railway=rail`, `light_rail`, `narrow_gauge`, `abandoned`, `disused`, `razed` | `usage`, `service`, `gauge`, `bridge`, `tunnel`, `layer`, `electrified` | rail design |
| rail point | `railway=switch`, `level_crossing`, `crossing`, `signal`, `buffer_stop`, `stop`, `station`, `halt` (nodes) | `railway:switch`, `railway:signal:position`, `railway:signal:direction`, `ref`, `name` | rail design |
| platform | `railway=platform`, `public_transport=platform` with `train=yes` (ways and areas) | `ref`, `name`, its geometry (length, side of the track) | rail design |
| waterway | `waterway=river`, `stream`, `canal`, `ditch`, `drain`, `brook` | `intermittent`, `seasonal`, `tunnel` (incl. `culvert`), `width`, `name`, `layer` | water design |
| water point | `waterway=waterfall`, `weir`, `dam`, `lock_gate`; `ford=*` on nodes and ways | `height` | water design |
| water area | `natural=water` (`water=lake`, `pond`, `river`, `reservoir`, `basin`, …), `landuse=reservoir`, `natural=wetland` | `water`, `intermittent`, `name` | water design |
| ground | `natural=bare_rock`, `scree`, `shingle`, `glacier`, `sand`, `beach`, `heath`, `grassland`, `fell`, `desert`; `landuse=meadow`, `grass`, `farmland`, `orchard`, `vineyard`, `quarry`, `cemetery`, `allotments`, `village_green`, `recreation_ground`; `leisure=park`, `garden`, `pitch`, `golf_course` | the tag and its subtags (`grassland=*`, `meadow=*`, `surface`, `intermittent`) | ground design |
| piste | `piste:type=downhill`, `nordic` (ways and areas) | `piste:type` | vegetation design (kept free) |
| woodland | `landuse=forest`, `natural=wood`, `natural=scrub` | `leaf_type`, `leaf_cycle`, `wood` (old tag) | vegetation design |
| tree line | `natural=tree_row`, `barrier=hedge` | `leaf_type`, `height` | vegetation design |
| tree | `natural=tree` (node) | `leaf_type`, `species`, `genus`, `height` | vegetation design |
| building | `building=*` (not `no`), `building:part` ignored **[my call]** | `building`, `building:levels`, `height`, `roof:shape`, `amenity`, `shop`, `name` | buildings (later) |
| settlement | `landuse=residential`, `commercial`, `retail`, `industrial`, `farmyard`, `military`, `military=*`, `place=*` areas | the tag | buildings, life (later) |
| place | `place=city`, `town`, `village`, `hamlet`, `isolated_dwelling`, `farm`, `locality` (node or area) with `name` | `name`, `population` | location names (this design) |
| barrier | `barrier=fence`, `wall`, `retaining_wall`, `guard_rail`, `gate`, `stile` | `fence_type`, `material`, `height` | fences (later) |
| power | `power=line`, `minor_line`, `tower`, `pole`, `substation` | `voltage`, `cables` | power lines (later) |
| aerialway | `aerialway=*` (lines), `aerialway=pylon`, `station` | `aerialway` | cable cars (later) |
| point object | `amenity=bench`, `waste_basket`, `shelter`, `hunting_stand`, `drinking_water`; `man_made=cross`, `water_well`, `mast`, `tower`, `water_tower`; `historic=wayside_cross`, `wayside_shrine`; `tourism=information` (`guidepost`, `board`); `highway=street_lamp`, `bus_stop` | the tag, `direction` | single objects (later) |

- One element can be several kinds (a `natural=wood` area that is also `leaf_type=needleleaved`: one kind with an
  attribute; a node that is `railway=level_crossing` and part of a road: rail point, and the road reads it too).
- **An element no row matches is still in the copy**; the table only decides what the generators see. A new row later
  needs no import.
- The table is data in code (a `match` per key like `features::classify`) **[my call]**, with a unit test per row; a
  shareable rule file is the spec's presets idea (5.2) and comes with the generators that need tuning.
- The existing readers move onto the copy and onto this table step by step: `read_ways` and `read_land` read
  `osm/copy.osm.pbf` and ask the table, so their output for the example stays the same (a regression test compares
  feature lists and the surface grid before and after).

### 5.5 What the build does with it

- **Read once.** A build reads the copy once into an in-memory `OsmData` (nodes, ways, relations in `BTreeMap`s by id,
  plus the per-kind index of the table). The road, lot and land readers take `&OsmData` instead of paths. With 2 MB of
  PBF this is fast; there is no need for a spatial index on disk.
- **Coverage check.** The build checks that the windows in `copy.toml` cover the map's area (the map's window, not the
  margin). If not, the build **stops** with an error that names the missing part and the command
  (`naves osm import`). A project with `[sources] osm = []` (a map without OSM) needs no copy and builds as today.
- **No copy yet, extracts listed.** The build stops with the same error **[my call]**; it never falls back to the
  extracts. (A project from before this change therefore needs one `naves osm import`; the manual and the changelog
  say so, and the error says so too.)
- **Changed extract.** If an extract's `sha256` differs from the import, the build reports `osm_source_changed`
  (information): the map still uses the copy; `--refresh` would bring the change in.
- **Report.** `build/report.json` gets `osm.kinds`: per kind the number of features on the map (and, for areas, the
  area in km², for lines the length in km). The MCP server's existing read tools gain `osm_kinds` (counts) and
  `osm_features` (the features of one kind in a rectangle, with id, tags and geometry in map metres), paged.
- **Location names.** Every `place` feature on the map becomes an entry in the world config's `class Names`, next to
  the existing `Center`: `name`, `position[]` in map metres, `type` from `place`: `city` → `NameCityCapital` or
  `NameCity`, `town` → `NameCity`, `village` → `NameVillage`, `hamlet`, `isolated_dwelling`, `farm` → `NameLocal`,
  `locality` → `NameLocal` **[my call]**, the vanilla type names (`[verify]` against Livonia's `class Names`, read with
  dayzmake from the user's install, and in game on the 2D map). The existing `Center` entry has `type = "Village"`;
  it gets whatever type the check shows to be right, so both use one scheme. An area place uses the area's centroid. Names keep
  their UTF-8 spelling (`Schönau am Königssee`) **[verify]** that the map shows umlauts; if not, transliterate. Entries
  are sorted by OSM id. The editing design's later "name" edits can override them; not part of this work.
- **Credits.** The ODbL line in the mod's credits uses the replication dates of the imports (the oldest and newest
  when they differ).

### 5.6 Edits

`edits/ways.toml` is applied on top of the copy as it is applied on top of the extracts today. Today's way reader keeps
only the 14 keys of `WAY_TAGS`, and every `base` fingerprint and every `modify` tag set in existing edit files was made
over that reduced set (`edits/ops.rs`, `edits/ways.rs::fingerprint`). So, **[my call]**:

- the fingerprint stays computed over the `WAY_TAGS` subset of the way's tags (and its node list), so existing edits keep
  matching; the subset becomes a named constant of the edit format (`FINGERPRINT_KEYS`), not a side effect of the
  reader;
- a `modify` edit's tags replace **only the keys in that subset** on the full-tag way; the way's other tags (railway,
  power, name, …) are kept. A later edit format that edits other kinds widens the subset with a format version and a
  migration;
- only `--refresh` can make fingerprints stop matching; it reports what it changed (5.3).

## 6. Testing

- Synthetic PBFs built in tests (as the existing `osm.rs` tests do), never game data or a real extract in the repo.
- Import: complete ways (a way with one node inside keeps all nodes); multipolygon members complete; a route relation
  keeps its member list but not the far members; dropped user/changeset/timestamp; reproducible bytes (two imports →
  identical files).
- Extend: elements already present are byte-identical afterwards even when the second extract has newer versions; new
  elements are added; `copy.toml` appends an import; a window inside the old ones is a no-op.
- Refresh: a changed way is replaced and the fingerprint mismatch with an edit is listed.
- Build: stops without a copy, stops when the area moved outside the copy, works with `osm = []`; the road features,
  lots and surface grid of a synthetic project are the same read from extracts (old code path, kept for the test until
  the switch) and from the copy.
- Classification: one test per table row; an element with no matching row is still in `OsmData`.
- Names: a synthetic village node becomes a `class Names` entry; the config snapshot test covers it.
- Fuzz: `osm_copy_read` (the reader on random bytes, seeds in `fuzz/corpus/`), `osm_copy_roundtrip` (write random
  elements, read back, compare).
- On the example (manual, documented in the plan): import from the two extracts, build, compare `report.json` road and
  lot numbers and the surface image with the last build from the extracts.

## 7. Left to the builder

- The writer crate or our own writer (after the review), block size and compression level.
- How `OsmData` is laid out in memory, and how the readers are moved onto it (order of the steps).
- The exact list of tags per kind beyond the table (more values may be added freely; none may be removed without a
  reason in the commit).
- MCP tool names and paging.
- Whether the GUI's import button runs in the background with progress like the build.

## 8. Designer's calls to check

1. Margin of 1 km real around the map (5.1).
2. Area relations complete, other relations only with the members already kept (5.1).
3. User, changeset and timestamp dropped from the copy (5.1).
4. Files `osm/copy.osm.pbf` and `osm/copy.toml` (5.2).
5. A duplicate id across extracts is resolved as today, independent of file order (5.3).
6. The build stops without a copy instead of reading the extracts (5.5); old projects need one import.
7. Place types mapped to `NameCity`, `NameVillage`, `NameLocal` (5.5).
8. The feature table as code, not a rule file, for now (5.4).
9. Areas larger than the map that enclose it are kept (5.1, rule 4).
10. An extend stops when an extract changed since the earlier import; `--refresh` or the old file (5.3).
11. Edit fingerprints and `modify` tags stay on the `WAY_TAGS` subset (5.6).

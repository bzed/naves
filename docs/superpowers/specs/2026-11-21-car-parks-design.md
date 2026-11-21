<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Car parks: a surface and a plane instead of a knot of roads

Status: design, 2026-11-21. Written by the assistant on the controller's brief, within the maintainer's decisions.
Decisions the maintainer made are marked **(maintainer)**, decisions of the controller **(controller)**; everything
else is the assistant's call and easy to change. Numbers marked **(judge)** are proposals that the judging runs
confirm or replace before the switch goes on.

Figures are Berchtesgaden at half scale (`scale.horizontal = 0.5`, `scale.height = 0.5`), terrain size 2048, 4 m
cells, on `road-planning` at bc5ea8a (the graph is the one `main` at 5137948 builds). The measurement program and its
output are in `/var/tmp/naves-parking/` (`measure/`, `out3.tsv`, `outcome.tsv`); they are scratch, not part of the
repo.

## Why

OpenStreetMap maps a car park as an `amenity=parking` area, usually with service roads inside. Aisles tagged
`service=parking_aisle` (and driveways) are already painted paths, not roads (`features::classify`). Everything
else inside, mostly plain `highway=service`, becomes a road of the `City` family and is built from road pieces. On a
scaled map that is a row of short, overlapping roads with junctions closer than a junction piece and corners no piece
can build.

What the example has (the measured table is below):

- 298 `amenity=parking` areas on the map. In 7 of them 16 graph stretches (549 m) lie wholly inside, with 15
  junction nodes inside. 64 more stretches enter a car park and end in it (access roads), 15 cross one.
- Most car parks have no road inside at all: their aisles are `parking_aisle` paths, already painted. Their problem is
  the ground: nothing flattens them, and nothing gives them a surface. 186 of the 298 lie on ground steeper than
  0.08 by a plane fitted to them.

**The site that prompted this is not a car park in OpenStreetMap.** Map position (5638, 5412) lies inside the camp
site *Grafenlehen* (way 169138470, `tourism=camp_site`), not inside any `amenity=parking` area. Its roads are ways
367615606 to 367615615 and 519730333 to 519730335, all plain `highway=service` with `surface=fine_gravel`: 29 graph
stretches, 536 m, wholly inside the camp site. The Königssee car park itself (way 23631323, `parking=surface`, P1,
12 583 m² on the map) is 240 m to the south-east at (5741, 5171); its aisles are paths, and only one road crosses
it. The motorhome car park P2 next to it (way 374744306) is tagged `tourism=caravan_site` with
`caravan_site:type=parking_lot`, not `amenity=parking`. Four camp and caravan sites on the map hold 70 stretches
(1484 m) wholly inside, more than all car parks together. This design keeps to `amenity=parking`
**(controller)**; whether camp and caravan sites follow is the first open point.

## Goal

Every car park that lies on the ground and can be built becomes one paved or grass area on one plane, which the
roads that lead to it meet without a step. Roads stay drivable and usable; that is the only goal for roads
**(maintainer)**.

- The roads inside a car park are not built as road pieces; the car park's area gets a surface instead, and only the
  access roads in and out stay roads **(maintainer)**.
- Paved if the roads that serve it are paved, grass if they are gravel or dirt **(maintainer)**.
- The car park is flattened **(maintainer)**: one plane, fitted to its ground, blended into the terrain around it.
- Map items keep the DayZ look: the surface is one the game already has **(maintainer)**.
- Scaled maps (1:2 and similar) are the primary case **(maintainer)**.
- Everything that is done, and everything that is left, is reported.

## Non-goals

- No terraces. A car park that one plane cannot hold within the limits stays as it is and is reported
  **(controller)**.
- No parking bays, markings, kerbs, lamps or other objects.
- No buildings: a multi-storey or underground car park is not built.
- No change to how road pieces are chosen or fitted, and no re-routing of the roads that stay (that is road planning,
  `2026-11-21-road-planning-design.md`; it re-routes tight bends of roads that stay, this stage removes roads and
  shapes an area).
- No new ground textures **(maintainer)**.
- No user option. The values are named constants; the switch is a constant.

## Anti-goals

- A car park nobody can drive into: every access road keeps its connection to the network and reaches the area.
- A road that only runs past a car park, crosses it or runs along its edge disappears.
- A step between an access road and the car park's surface.
- A flattened car park that reads as a hole or a dam: a cut or fill deeper than the limit.
- A change to the network that is not reported.
- Output that depends on thread timing or map iteration order.
- With the switch off, any change to any output.

## Approaches

1. **By area, with a surface and a plane (chosen).** Read the `amenity=parking` areas. Take every road out of the
   network that lies wholly inside a car park that can be built, whatever its tags; cut access roads back to the
   edge; paint the area; fit one plane and shape the ground to it like a junction's patch; tie the access roads to the
   plane in the height solve. It catches aisles mapped as plain service roads, gives the car parks without roads (the
   majority) a surface and level ground, and fails locally: a car park that cannot be built keeps its roads. It needs
   a new area reader, a polygon patch in the terrain shaping, a fifth ground layer, and a new kind of fixed junction.
2. **Only drop aisles by tag.** Widen the existing `parking_aisle` rule, for example to every `highway=service`
   without a `service` tag that is shorter than a few junction pieces, and paint those as paths. The smallest change.
   But it cannot tell an aisle from a short farm or yard road without the area, it does nothing for the ground or the
   surface (the maintainer's two decisions), and it would turn real service roads elsewhere into paths.
3. **A car park as one big junction patch.** Keep the roads inside, and give the car park one `LevelPatch` that the
   roads inside are held to, as a roundabout's plane is. It reuses the most code. But the pieces inside are exactly
   what does not fit (overlapping roads, junctions closer than a piece), so the findings stay; a disc does not follow
   a car park's outline; and there is still no surface.

By area is the only approach that meets both maintainer decisions (surface and flattening) and removes the cause
rather than holding it to a plane. It costs more code than the other two, but each part has a model in the code:
`drop_duplicates` and the sidepath step for removing and reporting, `ring_plane` for a fitted plane with a capped tilt,
`LevelPatch` and the claim rules of `conform` for the earthworks, and the ring junctions for a height the solve takes
as fixed.

## Design

### Which areas count, and how they are read

`osm::read_land` already reads areas in three passes (relations, ways, nodes), joins multipolygon members into rings
with `osm::assemble` and pairs holes with their outer ring. It gets a second classification: a way or a
`type=multipolygon` relation tagged `amenity=parking` is kept as a car park, with the tags `parking`, `surface`,
`layer`, `building`, `location` and `access`, in `OsmLand::car_parks`. A multipolygon with several outer rings gives
one car park per outer ring. `amenity=parking_space` (single bays; 9 areas on the example) is not read
**(controller)**. A car park mapped as a node has no area and is not read (one on the example).

The areas are turned into map metres with `MapGeo::to_map` and given stable ids: `osm/way/<id>` or
`osm/relation/<id>/<part>`. They are kept in id order.

Inside means even-odd inside the outer ring and not inside a hole. A self-intersecting ring is not rejected: every
test (inside, fill, ground samples, area) uses the even-odd rule, and the area is counted from the samples, not from
the shoelace formula (which a bow-tie cancels). A ring with fewer than three distinct corners or a non-finite point is
skipped and counted.

What each area becomes, first rule that applies:

| condition | outcome | on the example |
|---|---|---|
| `parking=underground`, `multi-storey` or `rooftop`, `building=*`, `location=underground` or `rooftop`, or `layer` other than 0 | **excluded** (`not_at_ground`): no surface, no plane, its roads stay **(controller)** | 0 (one underground car park is a node) |
| more than `CARPARK_WATER_MAX` (half) of its ground in water (`validate::water_test`) | **excluded** (`in_water`) | 0 |
| `parking=street_side`, `lane` or `layby` | **surface only** (`street_side`): the road beside already shapes that ground **(controller)** | 56 |
| area below `CARPARK_MIN_AREA_M2` (64 m², four terrain cells) | **surface only** (`tiny`) | 85 |
| within `features::CLIP_INSET_M` of the map edge | **surface only** (`map_edge`): roads there are cut ends that go on outside | 1 |
| a bridge or tunnel stretch passes over or under it | **surface only** (`under_bridge`, `over_tunnel`): the ground under a deck is the bridge's | 2 |
| its plane needs more than the limits allow | **not flattened**: surface only, roads stay, reported with how far out | 28 |
| otherwise | **built**: roads inside removed, access roads cut, surface, plane | 126 |

"Surface only" means the area is painted and nothing else changes: no road is removed or cut and the ground is not
shaped. It follows the maintainer's "all amenity=parking gets a paved surface" for areas that cannot or need not be
flattened (see *Departures* below).

`parking=carports` (one area) is treated as an ordinary car park: Náves places no roofs, and the ground under carports
is a car park.

### Groups: car parks that touch

Car parks whose areas touch or overlap (any two edges within `CARPARK_INSIDE_TOL_M`) form a group. A group gets one
plane, fitted to the ground of all its members, so that two car parks that share an edge meet at one height. Removal
and access are decided against the union of the group's members (inside any member). The surface is decided per car
park; a member with no road of its own takes the surface of its group, so a car park whose only access is through
another car park is paved or grass like the one it is reached through.

A group is built only if every member that is not surface-only for its own reason can be built; the limits are
checked on the group's plane. A group that fails is not split and retried in v1: every member is "not flattened",
with the group's numbers.

### Which roads go, and how access roads end

The step runs on the graph after `drop_duplicates` (see *Order*), for built car parks only. A stretch's line is
sampled every metre.

- **Inside.** A stretch lies inside a group if every sample is inside a member or within `CARPARK_INSIDE_TOL_M`
  (1 m, digitising) of a member's edge, **and** at least `CARPARK_INSIDE_SHARE` (half) of its length is inside by more
  than that tolerance. The second test keeps a road drawn on the car park's outline (OpenStreetMap often shares the
  nodes): it lies within the tolerance everywhere but never inside. A bridge or tunnel stretch is never inside (such
  a car park is surface only anyway). Every inside stretch is removed whatever its tags **(controller)**.
- **Access roads.** After the removal, a node inside a member by more than the tolerance that has exactly one
  stretch left, and is not a map-edge cut end (negative id), is a dead end inside the car park. Its stretch is cut
  where its line last enters the group (walking from that node outward, the first crossing of an edge), and is kept to
  `CARPARK_ENTRY_M` (4 m) beyond that crossing, inside. A new node ends it there, with an id above all others, handed
  out in sorted order; the cut piece keeps the stretch's id (the clean-up's rule for a moved end). If the stretch's
  inside part is shorter than `CARPARK_ENTRY_M`, it is not cut. The point where the stretch now ends is the **entry**
  of that access road.
- **Why 4 m inside, not at the edge.** The road's last joint must lie on the plane, not on the shoulder where the
  ground falls away: with the end one terrain cell (`limits::RAMP_JOINT_MIN_M`, 4 m) inside the edge, the end joint
  and the one-cell core beside it lie on flat ground, and the road bed overlaps the paint so no strip of grass shows
  between the road and the car park. Ending exactly at the edge would put the end of the last piece on the shoulder.
- **A road that enters, leaves and re-enters** is cut only at its last entry before its dead end; its parts that pass
  through the edge region stay. A stretch with both ends outside stays whole, however much of it lies inside.
- **Through roads.** A node inside where two or more stretches are left is not a dead end: the roads that meet there
  stay as they are. A road that crosses a car park, or a junction of roads that stay inside it, is not touched.
- **Ends that were already dead ends** inside a built car park (an access road that OpenStreetMap ends in the middle)
  are cut the same way.
- **Every access road stays.** The step never removes a stretch that is not wholly inside, so a car park's access is
  never lost. The stub step that follows must not drop an access road either: an entry node is passed to it as not a
  free end (a dead end at an entry is not a stub).
- **Connections.** A built group counts as one node for the connectivity check: every stretch that touches it (an
  entry, a road that crosses it, a node on its edge) is joined through it, because a vehicle drives across the area.
  The removal may not split a component counted this way (the sidepath step's `splits`, on the graph with one virtual
  node per group). If it would, nothing changes for that group, which is reported as not flattened with reason
  `would_split`. By construction this should not happen; the check is a guard.
- **Reported:** each removed stretch (id, car park, length, middle) and each cut access road (id, car park, the
  metres cut, the entry point).

Junction nodes inside a car park disappear with the stretches (on the example 9 junction nodes in the built car parks);
the report counts them.

### The surface

**(maintainer)** Paved if the roads that serve the car park are paved, grass if they are gravel or dirt.

- **Paved families** are those Náves treats as asphalt or city: `Asf1`, `Asf2`, `Asf3`, `City`. **Gravel and
  dirt** are `Grav` and `Mud`.
- **Serving roads** are the stretches removed from inside the car park and its access roads (the part inside before
  the cut), weighted by their length in metres inside **(controller)**. A car park with neither takes the roads whose
  earthwork touches its area: every stretch with a sample within `core_reach_m(half width, cell) + cell` of its edge
  (the clean-up's "earthworks touch" test), weighted by the metres within that reach. That is the frontage road a
  car park beside a street is reached from; on the example 98 of the 126 built car parks are decided this way.
- **Paved** if the paved metres are at least the gravel and dirt metres; a tie goes to paved (the higher-ranked
  family wins ties everywhere in Náves). **Grass** otherwise.
- **No road at all** (no serving road, no road within reach; 14 of the built car parks): grass. Only a paved road
  makes a paved car park, and a field car park without a road is a meadow in the game.
- A group member without serving roads takes its group's surface (the length-weighted decision over the group).
- An explicit `surface=*` tag on the car park is **not** read in v1 **(controller)**; whether it should override is an
  open point.

**Which ground layer.** Náves has four ground classes (`SurfaceClass`: grass, rock, forest, soil, layer slots 0 to
3); road beds are painted soil. None of them is paved. The game has a paved terrain surface: `en_tarmac` in
`surfaces_bliss` (`CfgSurfaces/en_tarmac`: `files = "en_tarmac_ca*"`, `vpSurface = "Asphalt"`, `soundEnviron =
"road"`, `impact = "Hit_Concrete"`), Livonia's own. So a vehicle on it gets asphalt friction and sound.

- A fifth class `SurfaceClass::Tarmac` with layer slot 4, textures `dz\surfaces_bliss\data\terrain\en_tarmac_nopx.paa`
  and `en_tarmac_ca.paa` (the names are checked in the install before they are written; the class's colour in the
  overlay is the average colour of `en_tarmac_ca`, read from the game file at development time).
- The mask colour of slot 4 is not known yet (`docs/formats/terrain-layers.md`: "5 or more: colours at alpha 128").
  The plan's surface task reads it from a Livonia tile whose material uses five or more layers (reading game files
  is allowed) and writes it into that document. If it cannot be established, paved car parks are painted soil, the
  class road beds use, and that is reported to the maintainer before the task continues.
- A tile material names the tarmac textures in slot 4 only if its mask uses slot 4. Tiles without tarmac are written
  exactly as today, so with the switch off every file is the same, byte for byte.
- Grass is `SurfaceClass::Grass` as it is.

**Painting.** The area (outer ring less holes, even-odd) is filled on the surface grid (1 m cells at 2048) with a
scanline fill like `OsmLayer::fill`. Order in `generate`: the decided ground, then grass car parks, then paths, then
paved car parks, then road beds. So in a grass car park the aisles stay soil tracks (a grass car park with gravel
aisles), in a paved car park the tarmac covers painted aisles and footways, and road beds of access roads lie on top of
both. Surface-only and not-flattened car parks are painted the same way.

### The plane

One plane per group, `z = h + gx·dx + gy·dy` about the group's centre.

- **Fit.** Least squares over the original ground (`Heightmap::sample` of the base heightmap) on a
  `CARPARK_SAMPLE_M` (1 m) grid of points inside the group's members and on the map, as `junctions::ring_plane` does
  for a roundabout. Fewer than three samples or a singular system: level at the median of the samples.
- **Slope limit.** The plane's steepest fall is at most `CARPARK_MAX_GRADE` = `limits::CROSS_FALL_AIM` = 0.08. A
  parked car stands on the plane in every direction, so its fall in any direction must be one a car takes across a
  road; `CROSS_FALL_AIM` is the fall across a road that roads are built with in a bend, and the steepest a road is
  aimed at sideways (estimate, `docs/research/2026-11-10-road-alignment.md`, section 3). It is under every family's
  `grade_aim` (lowest 0.08 for `Asf1`), so an access road can always meet it. A steeper fit is not refused: its tilt is
  cut down to the limit (direction kept) and the height re-fitted as the mean, as `ring_plane` caps a ring's tilt.
- **Depth limit.** The capped plane is accepted if the 95th percentile of `|ground − plane|` over the samples is at
  most `CARPARK_MAX_DEPTH_M` (1.5 m on the map, 3 m real at height scale 0.5). The percentile, not the maximum: the
  ground is a surface model, and single bumps (trees, cars, a building corner) are cut away by the earthwork anyway.
  The measured table chose it **(judge)**: 1.0 m would leave the flat but uneven Parkplatz Salzbergwerk (fall 0.01,
  95 % depth 1.12 m) unflattened; 1.5 m flattens 126 of 154 candidates; 2.0 m would add 12 car parks on hillsides that
  need a two-metre cut.
- **Not flattened** if the depth is over the limit. The report gives how far out it is: the free fit's fall against
  0.08, and the 95 % depth against 1.5 m (on the example 12 car parks at 1.5 to 2 m, 11 at 2 to 3 m, 5 at 3 m or more,
  the worst Parkplatz Zaunerbruck: fall 0.45, depth 4.65 m). These are the terrace candidates.
- **No size limit of its own.** A large car park on flat ground is fine (Königssee: 161 by 161 m, fall 0.047, 95 %
  depth 0.95 m); a large one on uneven ground fails the depth limit.

**In the terrain shaping.** The plane is a polygon patch, a sibling of `LevelPatch`:

- every terrain vertex inside a member (even-odd, not in a hole), or within `SHOULDER_CELLS` (1.5) cells of a member's
  outer edge or a hole's edge, is claimed at the plane's height, with its distance beyond the edge as the patch's
  `edge` (0 or less inside);
- the claim rules of `conform` stay as they are: a road's core wins inside its own width, compatible heights go to
  the higher rank (the car park has rank 0, below every road), contested heights to the nearer edge with a face
  between; a vertex under a bridge span is not claimed;
- beyond the shoulder the earthworks flood from it with the hillside cut and fill slopes, capped at
  `MAX_EARTHWORK_M`, as from a road. So the car park is blended into the terrain as a road's edge is
  **(controller)**;
- a hole keeps its own ground beyond the shoulder of its edge (a building island or a green inside the car park).

**The tie to access roads.** The plane is fixed before the height solve; the roads meet it, not the other way round.

- Each entry becomes a `Junction` of a new kind `JunctionKind::CarParkEntry` at the access road's placed end, with
  the access stretch in its `stretches`, `patch_m` = half the road's width, and the plane as its height and gradient.
- In the solve its height is a constant, exactly as a junction on a roundabout takes the ring's plane
  (`Source::Constant` in `profile::model`). The existing hold rules then do the rest: the chain's end joint is held
  (`HoldRule::End`) and the joints inward from it that lie over the car park (`HoldRule::Inward`, within the plane's
  area) are held to the plane's height there. So the road arrives at the height of the car park, with no step, and
  climbs or falls to it within its grade limits over the link to the next junction.
- A road that crosses a car park or runs along its edge is not held: it keeps its own profile and the plane yields
  inside its width (see open points).
- A car park reached only from a road beside it (frontage, no access stretch) has no entry and no hold; the
  shoulder meets the road's core as any patch does. Its difference to the plane at the edge is in the report.

### Neighbouring roads' earthworks

A road beside a car park shapes its own core and earthwork as before. Where its earthwork and the car park's shoulder
want one vertex, the claim rules decide (nearer edge, then rank). Where the heights differ by more than 0.3 m the
existing `road_conflict` reports it. The judging counts these within 50 m of a car park.

### Order in the pipeline

In `generate::fitted_graph_with`:

`build_graph` → `collapse_small_rings` → `drop_duplicates` → **car parks: assess (groups, exclusions, planes), drop
roads inside, cut access roads** → `netclean::clean` (sidepaths, stubs, merged junctions, approaches; the stub step
told the entry nodes) → road planning → smoothing → `fit_bridges`.

The car-park step comes before sidepaths, stubs and the junction merge **(controller)**, so that they do not work on
roads that are about to go. The assessment needs the graph (bridges and tunnels over the area) and the base heightmap
(the plane), both available there. The surface is decided right after the step (it needs what was removed) and
painted in `generate`. The planes go to `place_once`, which adds the entry junctions to the `Junctions` it finds, and
to `level_patches`'s caller, which gives them to the terrain shaping as polygon patches.

### Determinism

- Car parks in id order, groups ordered by their lowest member id, stretches in id order; new node ids above all
  others in sorted order; the cut piece keeps its id.
- Samples on a fixed grid from the group's south-west corner; sums in a fixed order; floats sorted with `total_cmp`;
  the percentile by sorting.
- No hash-map iteration. The step is sequential; it is small.
- With `CARPARKS_ENABLED` off the step is not called, no area is painted, no patch or entry junction is made, and
  every output is the same as without this work, bit for bit.

## The measured car parks of the example

`amenity=parking` areas read from the example's two extracts with the project's reader and `osm::assemble`; ground
from the base heightmap at 2048; graph after `drop_duplicates` and `netclean::clean` on `main`'s code; map metres and
map square metres (real area is four times as large). "Fall" is the steepest slope of the free least-squares plane;
"depth" the 95th percentile of `|ground − plane|` with the fall capped at 0.08. "Inside" are graph stretches wholly
inside (1 m tolerance), "access" stretches with one end inside, "through" stretches with both ends outside that pass
through.

**All areas on the map: 298** (304 touch the read window; 0 multipolygon relations, 0 with holes, 0 rings skipped).

| kind | count | note |
|---|---|---|
| `parking=surface` | 192 | |
| no `parking` tag | 49 | treated as surface |
| `parking=street_side` | 54 | 2189 m² together with `lane` |
| `parking=lane` | 2 | |
| `parking=carports` | 1 | treated as surface |
| underground, multi-storey, rooftop, `building`, `layer` ≠ 0 | 0 | |
| `amenity=parking_space` (not read) | 9 | |

**Sizes** (map m²): under 50: 112; 50 to 100: 57; 100 to 250: 69; 250 to 1000: 47; 1000 to 5000: 12; 5000 and more: 1
(Königssee, 12 583).

**Outcome with the proposed constants:**

| outcome | count | surface: paved / grass / grass (no road) |
|---|---|---|
| built (surface, plane, roads) | 126 | 109 / 3 / 14 |
| not flattened (depth over 1.5 m) | 28 | 15 / 5 / 8 |
| surface only: tiny (< 64 m²) | 85 | 65 / 2 / 18 |
| surface only: street side or lane | 56 | 45 / 0 / 11 |
| surface only: under a bridge | 2 | 2 / 0 / 0 |
| surface only: map edge | 1 | 1 / 0 / 0 |
| excluded | 0 | |

Of the 154 candidates (not street side, at least 64 m², not at the edge or under a bridge): the free fall is at most
0.05 for 35, 0.08 for 63, 0.10 for 77, 0.12 for 87, 0.15 for 101; median 0.098, 90th percentile 0.31, largest 0.63.
With the fall capped at 0.08, the 95 % depth is at most 0.5 m for 75, 1.0 m for 115, 1.5 m for 126, 2.0 m for 138,
3.0 m for 149.

**Roads:** in the 126 built car parks 9 stretches (351 m) are removed, with 9 junction nodes, and 43 access roads are
cut. The two other car parks with roads inside are not flattened and keep them: Hinterbrand (way 236789710, 5
stretches, 154 m, depth 3.13 m) and the Busparkplatz at Königssee (way 229154863, 2 stretches, 44 m, depth 2.56 m).
Through roads: 15 stretches cross a candidate car park.

**Named car parks:**

| car park | way | map | m² | fall | depth | inside | access | outcome |
|---|---|---|---|---|---|---|---|---|
| Königssee P1 | 23631323 | (5741, 5171) | 12 583 | 0.047 | 0.95 | 0 | 0 (1 through) | built, paved (frontage `City`) |
| Parkplatz Salzbergwerk | 23457631 | (6772, 7741) | 4244 | 0.010 | 1.12 | 0 | 2 | built, paved |
| Parkplatz „Schwarze Erde“ | 274929491 | (5117, 6966) | 3532 | 0.042 | 0.74 | 0 | 0 | built, paved (frontage) |
| P2 Hirschbichlstraße | 230425984 | (265, 5714) | 785 | 0.152 | 0.71 | 2 (98 m) | 5 | built, paved |
| P3 Hirschbichlstraße | 297153955 | (216, 5723) | 603 | 0.115 | 1.15 | 2 (79 m) | 2 | built, paved |
| (unnamed) | 51599660 | (6361, 7323) | 677 | 0.054 | 0.41 | 1 (84 m) | 2 | built, paved |
| (unnamed) | 255488300 | (2351, 6180) | 593 | 0.159 | 1.11 | 3 (81 m) | 3 | built, paved |
| Hinterbrand | 236789710 | (6967, 5309) | 1976 | 0.172 | 3.13 | 5 (154 m) | 3 | not flattened |
| Busparkplatz | 229154863 | (5660, 5156) | 747 | 0.191 | 2.56 | 2 (44 m) | 2 | not flattened |
| Wimbachbrücke | 180175387 | (3282, 5841) | 1663 | 0.036 | 0.89 | 0 | 3 | surface only (bridge) |
| Rathausplatz | 166805616 | (5613, 5937) | 861 | 0.049 | 0.78 | 0 | 0 | built, paved |

**Camp and caravan sites** (not `amenity=parking`, for the open point): Grafenlehen (169138470, 6605 m², fall 0.045,
29 stretches inside, 536 m), way 38834406 (15 962 m², 19, 405 m), way 174787106 (9902 m², 19, 399 m), way 174716692
(3503 m², 3, 144 m), the motorhome car park P2 Königssee (374744306, `caravan_site:type=parking_lot`, 1615 m², fall
0.22, 2 stretches, 127 m).

The surface decision used "serving" as designed, with the frontage reach measured as half the width plus 6 m rather
than `core_reach_m + cell` (half the width plus 10 m): Task 1 re-measures it with the code's own reach.

## Constants

All in `naves-core/src/carpark.rs`, `pub const`.

| name | value | meaning | from |
|---|---|---|---|
| `CARPARKS_ENABLED` | `false` until judged | the switch; off, the stage does nothing | controller; the maintainer switches it on after the game check |
| `CARPARK_INSIDE_TOL_M` | 1.0 m | digitising tolerance of "inside" and of "touching" for groups | controller ("a small tolerance"); OpenStreetMap's usual offset between an outline and a line drawn on it |
| `CARPARK_INSIDE_SHARE` | 0.5 | share of a stretch's length that must be inside by more than the tolerance | assistant: keeps a road drawn on the outline |
| `CARPARK_ENTRY_M` | 4.0 m | how far inside the edge a cut access road ends | `limits::RAMP_JOINT_MIN_M`, one terrain cell at 2048 |
| `CARPARK_MAX_GRADE` | 0.08 | steepest fall of a plane; a steeper fit is capped | `limits::CROSS_FALL_AIM` |
| `CARPARK_MAX_DEPTH_M` | 1.5 m | largest 95 % depth of ground against the capped plane | assistant, from the measured table **(judge)** |
| `CARPARK_DEPTH_QUANTILE` | 0.95 | the percentile of the depth test | assistant |
| `CARPARK_MIN_AREA_M2` | 64 m² | smaller car parks get a surface only | four terrain cells at 2048 |
| `CARPARK_WATER_MAX` | 0.5 | share of the ground in water above which a car park is excluded | controller ("mostly in water") |
| `CARPARK_SAMPLE_M` | 1.0 m | grid of ground samples for the fit and the depth | `ring_plane`'s 1 m |
| patch rank | 0 | the car park loses compatible ties to every road | assistant |
| shoulder | `conform::SHOULDER_CELLS` (1.5 cells) | ground beyond the edge held to the plane | reused |
| serving reach | `core_reach_m(half width, cell) + cell` | a road within this of the edge serves a car park without own roads | the clean-up's "earthworks touch" |
| paved families | `Asf1`, `Asf2`, `Asf3`, `City` | | maintainer, controller |

## How it is judged

Two runs of the `road-analysis` example on Berchtesgaden at terrain size 2048, the same binary, car parks off
against on. Compared:

- car parks by outcome (built, not flattened with reason, surface only with reason, excluded), and paved against
  grass;
- stretches and metres of road removed, access roads cut, junction nodes removed;
- connected components (with each built group as one node: must not grow) and total drivable length (must fall by
  no more than the removed metres and the cut metres);
- `road_gap` (count, total length, largest), `road_conflict`, `road_not_drivable`, `road_misses_terrain` and
  `join_without_junction`, map-wide and within 50 m of a built car park; every other rule count;
- the access roads: the height difference between each entry and the plane (must be under 0.05 m), the steepest
  grade on each access link;
- time of the stage.

Proposed bar for switching it on (the maintainer decides): no rule worse map-wide by more than 5 % without a reason
that can be named; within 50 m of built car parks, `road_gap` and `road_conflict` not worse; no entry with a step.

Then an in-game check with `tools/ingame-check`, off against on, of about six car parks: Königssee P1 (the large
flat one, with its through road), P2 and P3 Hirschbichlstraße (roads removed, on a slope, two car parks close
together), way 255488300 (three roads removed), Parkplatz Salzbergwerk (flat but bumpy), Hinterbrand (not flattened,
control) and, as the site that prompted this, Grafenlehen at (5638, 5412), which v1 does not change and so shows
what the camp-site open point is about.

## Effects on other stages

- **The clean-up.** Runs after the car-park step on a graph without the roads inside. The stub step is told the entry
  nodes and keeps access roads. The sidepath step may still judge an access road against a higher road beside it;
  that is rare and is reported as any sidepath.
- **Road planning.** Sees the graph without the car-park roads. An entry is a dead end: its position is fixed, its
  heading free, as for any dead end. No overlap: planning re-routes tight bends of roads that stay; this stage removes
  roads and shapes an area.
- **Bridges.** A car park under a bridge or over a tunnel is surface only; no deck end, deck or deck-end rule is
  touched. `fit_bridges` runs on the graph as before.
- **The height solve.** New: a junction kind whose height is a constant plane, as a ring junction's is. The access
  road's link to the next junction is checked against its ceiling as any link; a plane that a road cannot reach within
  its grade shows as `road_grade` or `junction_too_steep` there.
- **The terrain shaping.** New: a polygon patch beside the disc patches. Road cores still win inside their width.
- **Painted paths inside a car park.** Over a paved car park they are not seen (the tarmac is painted after them);
  over a grass car park they stay as soil tracks. `check_walkable` checks them on the shaped ground as before.
- **The ground layers.** A fifth layer (tarmac) on tiles that have a paved car park; everything else unchanged.
- **User edits.** Edits are keyed by stretch id, preset and slot. A removed stretch orphans its edits; a cut access
  road keeps its id but loses the slots of the cut part. Both are reported as today (`orphaned_edit`). Switching the
  stage on is a **breaking** change for such edits, and the changelog says so in the release that switches it on.
- **Sites.** `tools/ingame-check` takes car park sites as plain positions; no change to `naves_export::sites` is
  needed.

## Departures from the brief

- **Surface only, not "excluded", for tiny, street-side, map-edge and under-bridge car parks, and for the ones that
  are not flattened.** The maintainer said "all amenity=parking gets a paved surface if the roads are paved". Painting
  is harmless where flattening is not possible or not needed, so these get the surface and nothing else; their roads
  and ground are left as they are, which is what "excluded" and "left as it is" protect. Underground, multi-storey and
  water areas are excluded entirely.
- **Frontage roads serve a car park that has no road of its own.** The brief counts removed and access roads only;
  98 of the 126 built car parks have neither, so without the frontage rule almost every car park would be "no road",
  and grass, beside paved streets.
- **Groups.** Car parks that touch share one plane, so that two car parks with a common edge meet without a step.
  The brief asks for one plane per car park.
- **The switch lands off.** As road planning does, the stage is switched on after the judging and the game check,
  on the maintainer's word.

## Open points

- **Camp and caravan sites.** The site that prompted this work is a camp site, not a car park, and v1 does not change
  it. Four camp sites and one caravan site hold 70 stretches (1484 m) of plain service roads wholly inside, against 16
  stretches (549 m) in all car parks. Proposal for the maintainer: a `tourism=caravan_site` with
  `caravan_site:type=parking_lot` is a car park (the Königssee P2) and joins v1; camp sites get the road rule only
  (roads wholly inside are painted as tracks, like `parking_aisle`), no plane and no surface, because a camp site is a
  meadow with pitches, not a paved area.
- **`surface=*` on the car park.** Not read in v1 **(controller)**. Examples: Wimbachbrücke `surface=unpaved` but
  served by `Asf3` (paved by the rule), Rathausplatz `concrete:plates`, way 23555121 `gravel`. Also the serving roads'
  own `surface` tags: the Grafenlehen roads are `fine_gravel` but Náves builds every plain service road as `City`, so
  the rule would call them paved. Reading `surface` (on the car park first, then on its roads) is a small change once
  the maintainer decides.
- **Terraces.** 28 car parks need more than 1.5 m of cut or fill on one plane. A car park could be split into
  terraces along its fall, each a plane, joined by its aisles. Later work, from the report's list.
- **Through roads held to the plane.** A road that crosses a built car park keeps its profile, and the plane yields
  inside its width; where they differ the step is a face. Holding such a road's joints over the car park to the plane
  (as a road that crosses a junction's patch is held, `holds::crossings`) would make it lie flush. Decided after the
  judging (15 such stretches on the example).
- **Frontage without a hold.** A car park reached only from a road beside it has no entry and no hold. If the judging
  shows steps there, the road's nearest joint could be held, or the plane fitted with the road's height as a
  constraint (which needs the solve first).
- **Groups that fail.** A failed group is not split into its members in v1.
- **The slot-4 mask colour.** If it cannot be read from Livonia's data, paved car parks are painted soil until it is
  known.
- **Cell-size dependence.** `CARPARK_ENTRY_M` and `CARPARK_MIN_AREA_M2` follow the 4 m cell at 2048; at other
  terrain sizes they should follow the cell; for now they are constants.
- **Default of the switch.** The stage lands switched off and is switched on after the game check, on the
  maintainer's word.

<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Car parks, camp sites and similar: a surface and smooth ground instead of a knot of roads

Status: design, 2026-11-21, amended the same day after three decisions of the maintainer (the area kinds and "no
streets inside", the full surface, and "not bumpy"). Written by the assistant on the controller's brief, within the
maintainer's decisions. Decisions the maintainer made are marked **(maintainer)**, readings of the controller
**(controller)**; everything else is the assistant's call and easy to change. Numbers marked **(judge)** are
proposals that the judging runs confirm or replace before the switch goes on.

Figures are Berchtesgaden at half scale (`scale.horizontal = 0.5`, `scale.height = 0.5`), terrain size 2048, 4 m
cells, on the code of `main` at 5137948. The measurement programs and their output are in `/var/tmp/naves-parking/`
(`scan/` and `scan2.tsv`, `form.tsv` for this version; `measure/` and `outcome.tsv` for the first); they are
scratch, not part of the repo.

**Name.** The feature covers car parks, camp sites and similar areas. In the code and the manual it is a **lot**: the
module `lot`, the type `Lot`, the switch `LOTS_ENABLED`, the findings `lot_*`. The manual introduces it as "lots (car
parks, camp sites and similar areas)". The file name and the title of this spec stay.

## Why

OpenStreetMap maps a car park as an `amenity=parking` area and a camp site as a `tourism=camp_site` area, usually
with service roads inside. Aisles tagged `service=parking_aisle` and driveways are painted paths already
(`features::classify`). Everything else inside, mostly plain `highway=service`, becomes a road of the `City` family
and is built from road pieces. On a scaled map that is a row of short, overlapping roads with junctions closer than
a junction piece and corners no piece can build.

The site that prompted this work, map position (5638, 5412), lies inside the camp site *Grafenlehen* (way
169138470, `tourism=camp_site`), not in a car park. Its roads are ways 367615606 to 367615615 and 519730333 to
519730335, plain `highway=service` with `surface=fine_gravel`: 29 graph stretches, 536 m, 12 junction nodes, all
inside the camp site. The Königssee car park (way 23631323, P1) is 240 m south-east at (5741, 5171) and has no road
inside; the motorhome car park P2 next to it (way 374744306) is `tourism=caravan_site` with
`caravan_site:type=parking_lot`.

On the example the lots hold 85 stretches (2064 m) wholly inside, most of them in the four camp sites (69 stretches,
1481 m); car parks hold 14 (456 m). The ground inside them is a surface model with local bumps: the 95th percentile
of the local relief (defined below) is 0.31 m on average over the car parks and 0.50 m over the camp sites, up to
1.64 m (Campingplatz Simonhof).

## Goal

Every lot gets a full surface, paved or grass, and smooth ground; the roads inside it are not built. Roads stay
drivable and usable; that is the only goal for roads **(maintainer)**.

- "campsites / car parks and similar: full paved or grass, we don't need those extra streets" **(maintainer)**.
- "camp sites and car parks do not necessarily have to be totally flat, but also not bumpy" **(maintainer)**.
- "not bumpy = no local height changes, only larger areas have different heights" **(maintainer)**.
- Map items keep the DayZ look: the surface is one the game already has **(maintainer)**.
- Scaled maps (1:2 and similar) are the primary case **(maintainer)**.
- Only the access roads in and out stay roads; a road that passes through stays a road.
- Everything that is done, and everything that cannot be done, is reported.

## Non-goals

- No terraces and no facets in v1 (see *The smooth surface* and the open points).
- No bays, markings, kerbs, lamps, tents or other objects.
- No buildings: a multi-storey or underground car park is not built.
- No change to how road pieces are chosen or fitted, and no re-routing of the roads that stay (road planning,
  `2026-11-21-road-planning-design.md`, re-routes tight bends of roads that stay; this stage removes roads and shapes
  an area).
- No new ground textures **(maintainer)**.
- No user option. The values are named constants; the switch is a constant.

## Anti-goals

- A lot nobody can drive into: every access road keeps its connection to the network and reaches the lot.
- A road that only runs past a lot, crosses it or runs along its edge disappears.
- A step between an access road or a through road and the lot's ground.
- A hump or a hollow inside a lot; ground that goes up and down within a short distance.
- A change to the network that is not reported.
- Output that depends on thread timing or map iteration order.
- With the switch off, any change to any output.

## Approaches

1. **By area, with a surface and a low-order smooth surface (chosen).** Read the lot areas; take every road wholly
   inside a lot out of the network whatever its tags; cut access roads back to just inside the edge; paint the lot;
   replace its ground by a plane or a gently curved surface with no interior hump or hollow, fitted to the ground;
   tie access roads and through roads to that surface in the height solve. It removes the cause, gives the lots
   without roads (most car parks) a surface and smooth ground, and fails locally.
2. **Only drop aisles by tag.** Widen the `parking_aisle` rule to more service roads and paint them as paths. It
   cannot tell an aisle from a yard road without the area, leaves the ground and the surface alone, and the
   maintainer does not want the streets painted either.
3. **A lot as one big junction patch.** Keep the roads and hold them to one plane, as a roundabout's plane is held.
   It keeps exactly the pieces that do not fit, a disc does not follow an outline, and there is no surface.

For the smooth surface itself, a low-pass filter of the ground was measured and rejected: with a Gaussian of 8 m it
removed most of the local relief but kept the medium undulations the maintainer's definition rules out, it moved up
to 4.7 m of ground at its edges on a hillside, and it has no guaranteed property to test. A plane or a quadratic with
a capped curvature and no interior extremum is deterministic, cheap (a 6 × 6 system), and has a property that a test
can check exactly (see below).

## Design

### Which areas are lots, and how they are read

`osm::read_land` already reads areas in three passes and joins multipolygon members into rings
(`osm::assemble`). It gets a second classification. A way or a `type=multipolygon` relation is a lot if it has one
of these tags **(maintainer: "campsites / car parks and similar"; the list of similar tags is the assistant's, from
the measurement below)**:

| tag | why it is a lot | on the example |
|---|---|---|
| `amenity=parking` | a car park **(maintainer)** | 298 |
| `amenity=tourist_bus_parking` | a car park for coaches (a rarer tag for the same thing) | 1 (58 m², no road) |
| `highway=rest_area` | a car park beside a main road, with no services | 0 |
| `tourism=camp_site` | **(maintainer)** | 4 |
| `tourism=caravan_site` | **(maintainer)**; often a car park for motorhomes (`caravan_site:type=parking_lot`) | 1 |

`amenity=parking_space` (single bays) and `tourism=camp_pitch` (single pitches, all 14 inside camp sites on the
example) are not lots of their own. A lot mapped as a node has no area and is not read. Kept tags: `amenity`,
`tourism`, `highway`, `parking`, `surface`, `layer`, `building`, `location`, `access`, `caravan_site:type`.

The areas are turned into map metres with `MapGeo::to_map` and given stable ids: `osm/way/<id>` or
`osm/relation/<id>/<part>` (one per outer ring), in id order. Inside means even-odd inside the outer ring and not in
a hole; a self-intersecting ring is not rejected, every test uses the even-odd rule and the area is counted from
samples. An outer ring with fewer than three distinct corners, or a ring (outer or hole) with a non-finite point, skips the
lot and is counted as a skipped lot. A hole with fewer than three distinct corners and finite points has no area: the
hole is dropped, the lot stays, and the dropped holes are counted apart (`LotArea::holes_dropped`).

**Which tags were looked at.** Every area tag on the example was measured for graph stretches wholly inside it
(`scan2.tsv`; mean of the free-fit slope over all areas of the tag):

| tag | areas | with roads inside | stretches | metres | junction nodes | mean slope | verdict |
|---|---|---|---|---|---|---|---|
| `landuse=residential` | 362 | 71 | 849 | 34 626 | 498 | 0.18 | a road network: no |
| `landuse=forest` | 215 | 29 | 386 | 54 235 | 251 | 0.30 | a road network: no |
| `landuse=meadow` | 537 | 71 | 205 | 10 426 | 136 | 0.20 | no |
| `tourism=camp_site` | 4 | 4 | 69 | 1481 | 32 | 0.09 | **lot** |
| `landuse=military` | 5 | 3 | 68 | 3980 | 44 | 0.08 | open point (a barracks and a training ground) |
| `tourism=attraction` | 1 | 1 | 21 | 861 | 10 | 0.18 | open point (Eisarena Königssee) |
| `landuse=grass` | 63 | 1 | 20 | 1017 | 14 | 0.20 | no |
| `landuse=commercial` | 14 | 9 | 15 | 464 | 9 | 0.15 | open point (yards) |
| `amenity=parking` | 252 ≥ 20 m² | 5 | 14 | 456 | 13 | 0.15 | **lot** |
| `tourism=museum` | 1 | 1 | 12 | 265 | 7 | 0.11 | open point |
| `landuse=industrial` | 18 | 5 | 9 | 152 | 6 | 0.09 | open point (yards) |
| `landuse=farmyard` | 133 | 5 | 8 | 199 | 7 | 0.17 | no (a farm's own access) |
| `amenity=social_facility` | 2 | 1 | 7 | 371 | 6 | 0.10 | open point |
| `landuse=retail` | 2 | 1 | 6 | 127 | 4 | 0.08 | open point |
| `amenity=hospital`, `amenity=school` | 2, 7 | 1, 1 | 3, 3 | 110, 103 | 1, 2 | 0.06, 0.09 | no |
| `tourism=caravan_site` | 1 | 1 | 2 | 127 | 2 | 0.22 | **lot** |
| `highway=rest_area`, `highway=services` | 0 | | | | | | rest area **lot**; services open point |

A tag is a lot only if the place is clearly the same kind as a car park or a camp site: an open area people drive
around in on service roads that are not a road network of their own. Yards (`commercial`, `industrial`, `retail`),
institutions (`social_facility`, `museum`), `military` and `attraction` contain real internal road networks and
buildings; they are open points with these numbers.

What each lot becomes, first rule that applies:

| condition | outcome |
|---|---|
| `parking=underground`, `multi-storey` or `rooftop`, `building=*`, `location=underground` or `rooftop`, or `layer` other than 0 | **excluded** (`not_at_ground`): no surface, no smoothing, its roads stay **(controller, first brief)** |
| more than `LOT_WATER_MAX` (half) of its ground in water (`validate::water_test`) | **excluded** (`in_water`) |
| `parking=street_side`, `lane` or `layby` | **surface only** (`street_side`): a strip along a street, with no road inside; the street shapes that ground **(maintainer, via the controller)** |
| fewer than `LOT_MIN_VERTICES` (3) terrain vertices inside | **surface only** (`no_vertex`): too small for the 4 m grid to shape |
| otherwise | **smoothed**: roads inside removed, access roads cut, surface, smooth ground |

A bridge or tunnel no longer makes a lot surface only (it did in the first version): a bridge deck inside or over a lot
keeps its stretch and its own ground rules (the shaping never claims a vertex under a span, as for every patch), and a
tunnel is below the ground. That matters: the camp site Grafenlehen has a bridge stretch over its stream and would
otherwise not be smoothed. A lot at the map edge is smoothed on its part on the map; roads cut at the edge (negative
node ids) are never cut again.

### Lots that touch, overlap or contain each other

Lots whose areas touch or overlap (edges within `LOT_INSIDE_TOL_M`), of the same kind or not, form a group: a car
park inside a camp site, two car parks with a shared edge. A group gets one smooth surface over the union of its
members, so they meet without a step; roads are removed and access decided against the union. Every member keeps its
own surface (paved or grass, below); members are painted largest first, so a car park inside a camp site shows on top
of it. An excluded member is not part of any group.

### No streets inside

**(maintainer)** "we don't need those extra streets." For every lot that is not excluded, including the surface-only
ones:

- **Inside.** A stretch lies inside a group if every sample (1 m) is inside a member or within `LOT_INSIDE_TOL_M`
  (1 m) of a member's edge, and at least `LOT_INSIDE_SHARE` (half) of its length is inside by more than that. The
  second test keeps a road drawn on the outline (OpenStreetMap often shares nodes between an outline and a street).
  A bridge or tunnel stretch is never inside. Every inside stretch is removed, whatever its tags, and is **not**
  painted as a track either **(maintainer)**.
- **Painted paths.** A path that is a driveway or a parking aisle (`service=driveway`, `parking_aisle`; today a
  `FeatureKind::Path` of 3 m) inside a lot is a street and is not painted. A footway, path or cycleway through a lot is
  not a street; it stays painted, on top of the lot's surface **(controller)**.
- **Access roads.** After the removal, a node inside a member by more than the tolerance that has exactly one
  stretch left, and is not a map-edge cut end, is a dead end inside the lot. Its stretch is cut where its line last
  enters the group, and kept to `LOT_ENTRY_M` (4 m, one terrain cell, `limits::RAMP_JOINT_MIN_M`) beyond that
  crossing, inside; a new node ends it there (id above all others, sorted order), and the cut piece keeps the
  stretch's id. A stretch whose inside part is shorter than `LOT_ENTRY_M` is not cut. The end is the lot's **entry**
  for that road. Four metres inside, so that the road's last joint and its core lie on the smoothed ground, not on the
  shoulder, and the road bed overlaps the paint.
- **A road that enters, leaves and re-enters** is cut only at its last entry before its dead end. A stretch with both
  ends outside stays whole.
- **Through roads.** A node inside where two or more stretches are left is not a dead end; the roads there stay.
- **Every access road stays.** The step removes only stretches wholly inside, and the stub step of the clean-up is
  told the entry nodes: a dead end at an entry is not a stub. This matters most for a surface-only lot (`no_vertex`):
  its access road ends at the edge of ground that was not shaped, as a plain dead end with no height tie, and without
  the protection a short access piece would be dropped as a stub and the lot would lose its access.
- **Connections.** A group counts as one node for the connectivity check (a vehicle drives across a lot). The removal
  may not split a component counted this way; if it would (by construction it should not), nothing changes for that
  group and it is reported (`would_split`).
- **Reported:** each removed stretch and each cut access road.

### The surface

**(maintainer)** Full paved or grass, over the whole lot, for every lot that is not excluded.

**Paved or grass (controller).** Decided by the roads that serve the lot, by length:

- An explicit `surface=*` on the lot wins **(controller)**: a paved value gives paved, an unpaved value grass.
- Otherwise the serving roads: the stretches removed from inside it and its access roads (the part inside before the
  cut). Each metre counts by its way's `surface` tag where it has one; where the tag is missing or unknown, by its
  family (`Asf1`, `Asf2`, `Asf3`, `City` paved; `Grav`, `Mud` grass).
- A lot with neither takes the roads whose earthwork touches it (every stretch with a sample within
  `core_reach_m(half width, cell) + cell` of its edge, by the metres within that reach): the street a car park beside
  it is reached from (an extension of the controller's rule, see *Departures*).
- Paved if the paved metres are at least the grass metres (a tie goes to paved); grass otherwise; no road at all:
  grass. A group member without serving roads of its own takes its group's decision.
- Paved values: `asphalt`, `paved`, `concrete`, `concrete:plates`, `concrete:lanes`, `paving_stones`, `sett`,
  `cobblestone`, `unhewn_cobblestone`, `chipseal`, `bricks`, `metal`, `wood`. Unpaved values: `unpaved`, `gravel`,
  `fine_gravel`, `compacted`, `dirt`, `earth`, `ground`, `grass`, `grass_paver`, `mud`, `sand`, `pebblestone`, `rock`,
  `woodchips`. Anything else counts as untagged.

**What the reader has to add.** `osm::WAY_TAGS` already keeps `surface`, but `features::classify` reads it only to
choose the family of a track, and `FeatureKind::Road` and `Stretch` do not carry it. Added:
`FeatureKind::Road::paving: Option<Paving>` (`Paving::{Paved, Unpaved}`, from the lists above, `None` when the tag
is missing or unknown; `serde(default)`), and on `Stretch` the metres of its line whose way is tagged paved and
unpaved (`paved_m`, `unpaved_m`). They are not part of the attributes that cut a stretch, so the graph and every
stretch id stay as they are; a cut piece takes them in proportion to its length, that is the stretch's average
share, not the surface of the way the piece lies on, so the metres inside a lot are found by clipping the road
features (per way, each carries its `paving`) to the lot, not by reading the pieces. A line that is moved or shortened (junction merge, bridge approach, smoothing)
keeps its share of its new length, so the sum over pieces equals the original only for pure cuts; a line under
`EPS_M` has 0 metres, and a non-finite line has 0, never NaN. `Stretch::untagged_m()` is the length less both
figures, never below 0. `FeatureKind::Path` gets
`street: bool` (true for `service=driveway` and `parking_aisle`).

On the example: Grafenlehen is **grass** (404 m of its roads are tagged unpaved, 34 m paved, 132 m untagged
`City`); Camping Allweglehen (way 38834406) and Campingplatz Simonhof (way 174787106) are paved (asphalt roads, 287
against 118 and 263 against 189 m); Camping Mühlleiten (way 174716692) is paved by family (its roads are untagged);
the caravan site P2 is grass (155 m unpaved); the car park P2 Hirschbichlstraße is grass (98 m of unpaved against 6 m of paved road once every way of a stretch is counted). Of the 254 car parks of at least 20 m² that are not excluded, the surface is decided (Task 1's count) by their own
tag for 51, by their own roads for 11 (7 paved, 4 grass), by their group for 3, by the road beside them for 183 and
by nothing, so grass, for 6. A different count: 19 car parks have roads of their own (12 paved, 7 grass by those
roads), whatever decided them; do not add it to the others.

**Which ground layer.** Náves's four ground classes (grass, rock, forest, soil) come from `surfaces_bliss`
(`en_grass1`, `en_stones`, `en_forest_con`, `en_soil`); road beds are painted soil. The same PBO has two paved
surfaces, `en_tarmac` (`files = "en_tarmac_ca*"`) and `en_tarmacold` (`files = "en_tarmac_old*"`),
both with `vpSurface = "Asphalt"`, `soundEnviron = "road"`, `isDigable = 0`; Chernarus's `surfaces.pbo` has
`cp_concrete1`, `cp_concrete2` and `cp_gravel`. "lots of areas around buildings are paved and not part of the
building" **(maintainer)**: this is what the game uses for them.

- A fifth class `SurfaceClass::Tarmac` with layer slot 4, using `en_tarmacold` (texture `en_tarmac_old`). It is
  `en_tarmacold` and not `en_tarmac` because the game's `en_tarmac` class has no texture files anywhere in the install
  and Livonia never uses it; the files that exist are `en_tarmac_old_nopx.paa`, `en_tarmac_old_ca.paa` and
  `en_tarmac_old.rvmat` in the Livonia surface pack.
- The texture file names follow the pattern of the four layers (`dz\surfaces_bliss\data\terrain\<name>_nopx.paa`,
  `_ca.paa`); the exact names are read from the install in the plan's Task 1, not assumed.
- The fifth layer is selected by mask alpha 128 (RGB free; the value to write is (0, 0, 0, 128)); slot 5 would be
  alpha 0; slots 0 to 3 stay black, red, green, blue at alpha 255. Evidence: 71 Livonia masks fitted against their
  material variants with no violation in 3840 cells; not yet seen in the game, the game check confirms it. If the game
  check disproves it, paved lots are painted soil and the maintainer is told.
- A tile material names the tarmac textures in slot 4 only if its mask uses slot 4, so a build without lots writes
  every file as today.

**Painting.** The lot (outer ring less holes, even-odd) is filled on the surface grid (1 m cells at 2048) with a
scanline fill like `OsmLayer::fill`. Order in `generate`: the decided ground, the lots (largest first), the paths
that are not streets inside a lot, the road beds.

### The smooth surface

**(maintainer)** "not bumpy = no local height changes, only larger areas have different heights". **(controller)**
Inside a lot the ground has no local height variation; height changes only gradually across the lot as a whole,
never up and down within a short distance. Every lot that is smoothed gets this, whatever its size or slope **(controller)**.

**The form.** One surface per group, fitted by least squares to the original ground at the terrain vertices inside
the group's members (the terrain is its vertices; the fit is what the shaping writes):

1. **A plane** `z = h + gx·dx + gy·dy`, if the 95th percentile of `|ground − plane|` is at most `LOT_PLANE_FIT_M`
   (0.5 m). A car park that is nearly a plane anyway reads best perfectly plane. On the example 138 of the 191
   smoothed lots (the other 53 are quadratics).
2. **Otherwise a quadratic** `z = h + gx·dx + gy·dy + a·dx² + b·dx·dy + c·dy²` whose curvature is capped: both
   eigenvalues of its Hessian are clipped to `±1 / LOT_MIN_RADIUS_M`, and the plane part re-fitted with the curvature
   fixed. `LOT_MIN_RADIUS_M` = 600 m = `limits::min_vertical_radius(Asf1)`, the largest vertical radius any road
   needs: the lot is never more curved than the gentlest road. If the capped quadratic has its stationary point inside
   the lot and its Hessian is definite (an interior hump or hollow), the plane is used instead. On the example 53 lots,
   among them all four camp sites and the caravan site.

So the slope of the surface changes linearly and slowly, and it reverses at most once along a line, and then only in
a saddle or outside the lot.

**The property a test checks** ("no local height changes"):

- along every grid row, column and diagonal through the lot, the height sequence is monotonic or has one turning
  point, and no vertex inside (away from the edge) is higher or lower than all eight of its neighbours;
- the second difference of the height along a row, column or diagonal, `|h(i−1) − 2h(i) + h(i+1)|`, is at most
  `s² / LOT_MIN_RADIUS_M` for the spacing `s` (0.027 m at 4 m);
- the **local relief** is at most 0.12 m: for every vertex, the difference between its height and the plane fitted
  to the vertices within `LOT_RELIEF_LENGTH_M / 2` of it. `LOT_RELIEF_LENGTH_M` = 24 m (six terrain cells, about five
  car lengths); 0.12 m is the rise of a 600 m radius over 24 m (`L² / 8R`), so a capped quadratic meets it by
  construction and a plane has none. A tent pitch and a parked car need no more than that.

**How far the ground moves.** Measured with this form (`form.tsv`): over the 191 smoothed lots the 95th percentile of
the move is 0.25 m at the median lot, 0.94 m at the 90th percentile, 2.37 m at the most (Task 1 fitted each lot on its own, while the design fits one surface per group of touching lots, 29 groups of two or three, so these numbers may move again when groups are built). The largest move by lot:

| kind | under 1 m | 1 to 2 m | 2 to 3 m | 3 m or more |
|---|---|---|---|---|
| car parks (185) | 164 | 15 | 6 | 0 |
| camp sites (4) | 0 | 1 (Mühlleiten 1.75) | 1 (Grafenlehen 2.49) | 2 (Allweglehen 3.42, Simonhof 4.47) |
| caravan site (1) | 0 | 0 | 0 | 1 (3.29) |
| tourist bus parking (1) | 1 | 0 | 0 | 0 |

**A guard on the move.** `LOT_MAX_MOVE_M` = 3.0 m on the map (6 m real at height scale 0.5). A lot whose surface
would move any vertex more is not clipped vertex by vertex, which would bring back exactly the local height changes the
surface is there to remove. Instead the whole change is scaled: the result is `ground + λ·(surface − ground)` with
one `λ = LOT_MAX_MOVE_M / largest move` for the lot, and the lot is reported (`lot_move_limited`) with λ. Its local
relief is then at most `(1 − λ)` of what it was plus 0.12 m. On the example three lots: Simonhof (λ 0.67, relief
1.64 m before, 0.248 m at the 95th percentile and 0.526 m at most after), Allweglehen (λ 0.88) and the caravan site P2
Königssee (λ 0.911). These are where facets would help (open points).

**Overshoot.** A least-squares plane or quadratic can lie slightly outside the range of the original heights at a
corner of the lot. It is not clamped (clamping makes a kink); the move guard bounds it, and the report gives each
lot's lowest and highest height before and after.

**Steep lots.** A lot on ground too steep to drive stays steep: smoothing keeps the lie of the land. A lot whose
surface has a slope above `LOT_STEEP` = `limits::CROSS_FALL_LIMIT` (0.20, the largest fall across a road a vehicle can
take; a parked or turning car meets the slope sideways) at its 95th percentile is smoothed and painted like any other
and reported (`lot_steep`). On the example 44: 42 car parks, the caravan site P2 (0.33) and Camping Allweglehen
(0.23). They are the terrace candidates.

**In the terrain shaping.** The surface is a polygon patch, a sibling of `LevelPatch`:

- every terrain vertex inside a member (even-odd, not in a hole), or within `SHOULDER_CELLS` (1.5) cells of a
  member's outer edge or a hole's edge, is claimed at the surface's height (the surface is evaluated there too); its
  `edge` is its distance beyond the edge (0 or less inside);
- the claim rules of `conform` stay: a road's core wins inside its own width, compatible heights go to the higher rank
  (the lot has rank 0, below every road), contested ones to the nearer edge; a vertex under a bridge span is not
  claimed;
- beyond the shoulder the earthworks flood from it with the hillside cut and fill slopes, capped at
  `MAX_EARTHWORK_M`, as from a road: the lot blends into the terrain as a road's edge does;
- a hole keeps its own ground beyond the shoulder of its edge.

**Access roads.** Each entry becomes a `Junction` of a new kind `JunctionKind::LotEntry` at the access road's placed
end, with the access stretch in its `stretches`, `patch_m` half the road's width, and the lot's surface as its height
(and the surface's gradient there as its gradient). In the solve its height is a constant, as a junction on a
roundabout takes the ring's plane (`Source::Constant` in `profile::model`). The hold rules `End` and `Inward` hold the
road's end joint and the joints inward that lie over the lot to the surface's height there. So the road arrives at the
lot's height, with no step.

**Through roads.** A road that crosses a lot keeps its own earthworks, and the two must meet without a step
(**(controller)**: "the smooth surface meets them at the road's shoulder"). Composed in two parts:

- in the height solve, the ground target of every joint of a through road that lies over a smoothed lot is the lot's
  surface there instead of the original ground (`profile::model`'s `ground_targets`; a soft target, as the ground is
  today). The surface's curvature is at most that of the gentlest road (600 m), so a road can follow it; where its
  slope along the road is above the road's `grade_ceiling`, the road's own hard limits win and the target only pulls;
- in the shaping, the road's core and shoulder ring win as everywhere; the lot's vertices beyond the ring take the
  lot's surface. Where the road followed the surface, the two agree within `CONFLICT_M` (0.3 m) and there is no step;
  where it could not, the difference is a face, reported as `road_conflict` as today and counted by the judging within
  50 m of lots.

A lot reached only from a street beside it has no entry; the street is not pulled to the lot. The shoulder meets the
street's core as any patch does; the judging counts the steps there.

### Order in the pipeline

In `generate::fitted_graph_with`:

`build_graph` → `collapse_small_rings` → `drop_duplicates` → **lots: assess (groups, exclusions, surfaces to fit),
drop roads inside, cut access roads** → `netclean::clean` (sidepaths, stubs, merged junctions, approaches; the stub
step told the entry nodes) → road planning → smoothing → `fit_bridges`.

The lot step comes before sidepaths, stubs and the junction merge **(controller)**, so that they do not work on roads
that are about to go. The smooth surfaces need only the base heightmap, which is there. The surface class is decided
right after the step (it needs what was removed) and painted in `generate`. The surfaces go to `place_once`, which adds
the entry junctions and the through-road targets, and to the terrain shaping as polygon patches.

### Determinism

- Lots in id order, groups ordered by their lowest member id, stretches in id order; new node ids above all others,
  in sorted order; a cut piece keeps its id.
- Fits on the fixed terrain grid, sums in a fixed order, Gaussian elimination with a fixed pivot rule; percentiles by
  sorting with `total_cmp`.
- No hash-map iteration. The step is sequential.
- With `LOTS_ENABLED` off the step is not called, nothing is painted, no patch, entry or row is made, and every
  output is the same as without this work, bit for bit.

## The measured lots of the example

Lot areas read from the example's two extracts; ground from the base heightmap at 2048, at the terrain vertices;
graph after `drop_duplicates` and `netclean::clean`. Map metres and map square metres (real area is four times as
large). The scan covers every area, the 46 car parks under 20 m² too (surface only: street strips, or fewer than
three vertices inside; one way, 1548125225, is excluded). Task 1 fitted each lot on its own; the design fits one
surface per group of touching lots (29 groups of two or three), so the rows may move again when groups are built.

**Outcome by kind:**

| kind | areas | smoothed: plane | smoothed: quadratic | surface only: street side | surface only: no vertex | excluded | steep | move limited |
|---|---|---|---|---|---|---|---|---|
| `amenity=parking` | 298 | 137 | 48 | 56 | 56 | 1 | 42 | 0 |
| `tourism=camp_site` | 4 | 0 | 4 | 0 | 0 | 0 | 1 | 2 |
| `tourism=caravan_site` | 1 | 0 | 1 | 0 | 0 | 0 | 1 | 1 |
| `amenity=tourist_bus_parking` | 1 | 1 | 0 | 0 | 0 | 0 | 0 | 0 |
| `highway=rest_area` | 0 | | | | | | | |

**Roads.** Removed in total: 85 stretches, 2064 m, with 47 junction nodes; 32 access roads (dead ends after the removal), of which 21 are cut; 15 stretches pass
through a lot and stay. By kind: camp sites 69 stretches (1481 m, 32 junction nodes, 5 access roads), car parks 14
(456 m, 13, 25), caravan site 2 (127 m, 2, 2). (The first version counted 16 stretches, 549 m, in car parks; this
count drops bridge stretches and applies `LOT_INSIDE_SHARE`.)

**Local relief before** (95th percentile per lot, then averaged): car parks 0.31 m (90 of the 116 car parks large
enough to measure are above 0.12 m; the largest 1.61 m), camp sites 0.50 m (largest 1.64 m), caravan site 0.90 m.
After: at most 0.12 m by construction, except in the move-limited lots (camp sites 0.248 m at the 95th percentile,
0.526 m at most).

**Named lots:**

| lot | way | map | m² | form | move p95 / max | slope p95 | roads removed | surface | notes |
|---|---|---|---|---|---|---|---|---|---|
| Grafenlehen (camp site) | 169138470 | (5630, 5361) | 6605 | quadratic | 1.09 / 2.49 | 0.095 | 29 (536 m), 12 junction nodes, 1 access cut | grass | the site that prompted this |
| Campingplatz Simonhof | 174787106 | (1330, 7247) | 9902 | quadratic | 2.32 / 4.47 | 0.12 | 18 (396 m) | paved | move limited, λ 0.67 |
| Camping Allweglehen | 38834406 | (7684, 8194) | 15 962 | quadratic | 1.66 / 3.42 | 0.225 | 19 (405 m) | paved | at the map edge; steep; λ 0.88 |
| Camping Mühlleiten | 174716692 | (5739, 5623) | 3503 | quadratic | 1.12 / 1.75 | 0.17 | 3 (144 m) | paved (family) | |
| P2 Königssee (caravan site) | 374744306 | (5601, 5200) | 1615 | quadratic | 2.37 / 3.29 | 0.33 | 2 (127 m) | grass | steep; move limited, λ 0.911 |
| Königssee P1 | 23631323 | (5741, 5171) | 12 583 | quadratic | 0.61 / 2.09 | 0.070 | 0 (1 through road) | paved (street beside) | large, gentle |
| Parkplatz Salzbergwerk | 23457631 | (6772, 7741) | 4244 | quadratic | 0.71 / 1.04 | 0.063 | 0 | paved | flat but bumpy |
| P2 Hirschbichlstraße | 230425984 | (265, 5714) | 785 | plane | 0.44 / 0.56 | 0.148 | 2 (98 m) | grass | 98 m unpaved against 6 m paved |
| Hinterbrand | 236789710 | (6967, 5309) | 1976 | quadratic | 1.91 / 2.60 | 0.34 | 5 (154 m) | paved | steep |

## Constants

All in `naves-core/src/lot.rs`, `pub const`.

| name | value | meaning | from |
|---|---|---|---|
| `LOTS_ENABLED` | `false` until judged | the switch; off, the stage does nothing | controller; the maintainer switches it on after the game check |
| `LOT_INSIDE_TOL_M` | 1.0 m | digitising tolerance of "inside", and of "touching" for groups | controller ("a small tolerance") |
| `LOT_INSIDE_SHARE` | 0.5 | share of a stretch's length that must be inside by more than the tolerance | assistant: keeps a road drawn on the outline |
| `LOT_ENTRY_M` | 4.0 m | how far inside the edge a cut access road ends | `limits::RAMP_JOINT_MIN_M`, one cell at 2048 |
| `LOT_PLANE_FIT_M` | 0.5 m | a lot whose plane is within this of its ground (95th percentile) gets the plane | assistant, from the table (138 of 191) **(judge)** |
| `LOT_MIN_RADIUS_M` | 600 m | smallest radius of curvature of the surface | `limits::min_vertical_radius(Family::Asf1)` |
| `LOT_RELIEF_LENGTH_M` | 24 m | the "short distance" of "no local height changes" | six terrain cells, about five car lengths |
| relief target | 0.12 m | largest local relief over 24 m | `L² / (8 · LOT_MIN_RADIUS_M)` |
| `LOT_MAX_MOVE_M` | 3.0 m | largest move of a vertex; beyond it the change is scaled for the whole lot | assistant, from the table (3 lots) **(judge)** |
| `LOT_STEEP` | 0.20 | slope (95th percentile) above which a lot is reported steep | `limits::CROSS_FALL_LIMIT` |
| `LOT_MIN_VERTICES` | 3 | fewer terrain vertices inside: surface only | a plane needs three |
| `LOT_WATER_MAX` | 0.5 | share of the ground in water above which a lot is excluded | controller ("mostly in water") |
| the 1 m sample grid | 1.0 m | samples for the area (fixed in the area code, not a tunable); the water share is counted on a 1 m grid too | `ring_plane`'s 1 m |
| patch rank | 0 | the lot loses compatible ties to every road | assistant |
| shoulder | `conform::SHOULDER_CELLS` (1.5 cells) | ground beyond the edge held to the surface | reused |
| serving reach | `core_reach_m(half width, cell) + cell` | a street within this of the edge serves a lot without roads of its own | the clean-up's "earthworks touch" |
| paved families | `Asf1`, `Asf2`, `Asf3`, `City` | used where a road has no `surface` tag | maintainer, controller |

## How it is judged

Two runs of the `road-analysis` example on Berchtesgaden at 2048, the same binary, lots off against on. Compared:

- lots by kind and outcome (plane, quadratic, surface only with reason, excluded), paved against grass, steep and
  move-limited ones;
- stretches and metres removed (85, 2064 m), access roads (32 dead ends, 21 cut), junction nodes removed (47);
- connected components (each group one node: must not grow); drivable length (must fall by no more than the removed
  and cut metres);
- per lot kind, the local relief before and after (95th percentile and largest; after must be at most 0.12 m outside
  the move-limited lots), the largest and 95th-percentile move, and the slope of the result;
- `road_gap` (count, total length, largest), `road_conflict`, `road_not_drivable`, `road_misses_terrain` and
  `join_without_junction`, map-wide and within 50 m of a lot; every other rule;
- the access roads: the height difference between each entry and the surface (under 0.05 m), the steepest grade on
  each access link; for through roads, the largest difference between a joint over a lot and the surface;
- time of the stage.

Proposed bar for switching it on (the maintainer decides): no rule worse map-wide by more than 5 % without a reason
that can be named; within 50 m of lots `road_gap` and `road_conflict` not worse; no entry with a step; the relief
target met.

Then an in-game check with `tools/ingame-check`, off against on: Grafenlehen at (5638, 5412) (the prompting site),
Königssee P1 (large, gentle, with its through road), Camping Allweglehen (steep camp site) or, if its part on the map
is too small to photograph, Campingplatz Simonhof (move limited), the caravan site P2 Königssee (steep, grass), P2
Hirschbichlstraße (roads removed, a plane on a slope) and Parkplatz Salzbergwerk (flat but bumpy). The tarmac is
judged there.

## Effects on other stages

- **The clean-up.** Runs after the lot step on a graph without the roads inside; the stub step keeps access roads.
- **Road planning.** Sees the graph without the lot roads; an entry is a dead end (fixed position, free heading). No
  overlap.
- **Bridges.** A bridge in or over a lot keeps its stretch, its deck and its ground rules; the lot's patch never claims
  a vertex under a span. `fit_bridges` runs as before.
- **The height solve.** New: a junction kind with a constant height (as ring junctions have), and the lot's surface as
  the ground target of through roads over a lot. A surface that a road cannot reach within its grade shows as `road_grade` or
  `junction_too_steep` there.
- **The terrain shaping.** New: a polygon patch beside the disc patches.
- **Painted paths.** Driveways and parking aisles inside a lot are no longer painted; footways and paths through it
  are, on top of the lot's surface. `check_walkable` checks them as before.
- **The ground layers.** A fifth layer (tarmac) on tiles that have a paved lot.
- **User edits.** A removed stretch orphans its edits; a cut access road keeps its id but loses the slots of the cut
  part. Both are reported as today (`orphaned_edit`). Switching the stage on is a **breaking** change for such edits —
  on the example 85 stretches go and 21 are cut — and the changelog says so in the release that switches it on.

## Departures from the brief

- **Surface for the excluded classes of the first brief.** All lots that are not underground, multi-storey or in
  water get a surface, as the maintainer decided; tiny lots and street strips get only that.
- **No exception for bridges, tunnels and the map edge.** A bridge or tunnel in a lot, or the map edge, no longer
  makes it surface only: the bridge's ground rules and the clipping at the edge already do the right thing, and
  Grafenlehen would otherwise not be smoothed.
- **Streets beside a lot serve it** when it has no road of its own. The controller's rule counts removed and access
  roads only; on the example 183 car parks of at least 20 m² are decided by the road beside them (6 have no road at all), and
  without the rule they would all be grass beside paved streets.
- **The move guard scales the whole change** rather than limiting it vertex by vertex: limiting each vertex would
  reintroduce local height changes.
- **Through roads are pulled to the lot's surface** in the solve (the surface replaces the ground as their target
  over the lot), rather than only meeting it at their shoulder, so that neither leaves a step.
- **The switch lands off,** as road planning does.

## Open points

- **More tags as lots.** `highway=services` (none on the example; a fuel station and buildings with a car park),
  `landuse=commercial` (9 areas with 15 stretches, 464 m), `landuse=industrial` (5, 9, 152 m), `landuse=retail` (1,
  6, 127 m), `landuse=military` (3, 68, 3980 m: the Jägerkaserne barracks and the Silberg training ground),
  `tourism=attraction` (Eisarena Königssee, 21, 861 m), `amenity=social_facility` (1, 7, 371 m), `tourism=museum` (1,
  12, 265 m). They hold real internal roads and buildings; the assistant would leave them out.
- **Paved camp sites.** By the serving-roads rule three of the four camp sites are paved, because their roads are
  asphalt or untagged `City`. A camp site paved from edge to edge may read wrong; a camp site could be grass unless it
  is tagged paved, and only its car park (if mapped) paved. For the maintainer.
- **Facets for large lots.** Two camp sites (Simonhof 9902 m², Allweglehen 15 962 m²) and the caravan site P2 Königssee (1615 m²) need more than
  3 m of move for one surface and are scaled down. A few large facets, each a capped quadratic, joined without a step along their
  shared edges, would fit them. Needed where the largest move of one surface is above 3 m (three lots on this example); not in v1.
- **Terraces** for the 44 steep lots, from the report's list. Later work.
- **Through roads.** If the soft target is not enough (the judging counts faces within 50 m of lots), a hard hold of
  through-road joints over a lot could replace it where the surface's slope is under the road's ceiling.
- **The look of a large smoothed lot.** A 16 000 m² camp site on one gently curved surface may look artificially
  even in the game; the game check shows it. No terraces are added for it.
- **The slot-4 mask value.** Alpha 128 is fitted to Livonia's masks but not yet seen in the game; if the game check
  disproves it, paved lots are painted soil until it is known.
- **Cell-size dependence.** `LOT_ENTRY_M`, `LOT_RELIEF_LENGTH_M` and `LOT_MIN_VERTICES` follow the 4 m cell at
  2048; at other terrain sizes they should follow the cell.
- **Default of the switch.** The stage lands switched off and is switched on after the game check, on the
  maintainer's word.

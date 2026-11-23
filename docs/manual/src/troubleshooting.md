<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Troubleshooting

## Missing tools

**`dayzmake not found; set DAYZMAKE (hint: ...)`**
Install dayzmake 2.0.0 or newer (<https://github.com/bzed/dayzmake>) and put it on `PATH`, or set `DAYZMAKE` to the program.

**`dayzmake v1.x.y is too old; Náves needs dayzmake v2.0.0 or newer, get it at ...`**
The `dayzmake` that was found is older than 2.0.0. Install a newer one and point `DAYZMAKE` at it.

**`cannot tell the version of dayzmake ...`**
`dayzmake --version` printed nothing that looks like a version, so the program is probably not dayzmake. Check
the path in `DAYZMAKE`, in the settings or on `PATH`.

**`Steam not found; set STEAM_ROOT`**
Náves did not find a Steam folder. Set `STEAM_ROOT` to the folder that contains `steamapps/`.

**`DayZ Tools not found; set DAYZ_TOOLS`**
Install *DayZ Tools* from the Tools section of your Steam library, or set `DAYZ_TOOLS` to its folder
(the one that contains `Bin/Binarize`).

**`Proton not found; set PROTON`**
Install *Proton Hotfix* (or Proton Experimental) from Steam, or set `PROTON` to a `proton` script.

**`dayzmake: the path in the settings does not exist: ...`**
A path in the `[tools]` table of your `settings.toml` (see [Installation](installation.md#external-tools)) leads
nowhere. Correct it, or remove the line; an environment variable overrides the settings.

**`warning: the settings file is damaged and is ignored`**
`settings.toml` is not valid TOML or has a value of the wrong type. The message names the line. Náves does not
change the file; fix it or delete it.

## Binarize

**`binarize failed (exit N); log: ...; sandbox kept at ...`**
`binarize.exe` stopped with an error. Read the log named in the message; it is binarize's own output.
The sandbox folder is kept so that you can look at it, and is safe to delete.

**`binarize produced no OPRW world`**
The tool exited without an error but no world file came out. The log usually says which texture or
material it could not read.

**`binarize did not finish within N s`**
The run took longer than `NAVES_BINARIZE_TIMEOUT` (default 1800 s) and was stopped. Raise the limit for a
large map. The sandbox is kept.

**`not a valid prefix` / `not a valid world name`**
The world name or its prefix contains characters other than letters, digits and `_`. The `world` field in
`naves.toml` must match `^[a-z][a-z0-9_]*$`.

## Imagery

**`<file>: ... EPSG:<code> is not supported`**
Náves reads imagery in EPSG:4326, UTM (326xx, 327xx, ETRS89 258xx) and EPSG:3857. Convert the file with the
command the message gives: `gdalwarp -t_srs EPSG:<the UTM code of the map> in.tif out.tif`.

**`... bits per sample, not 8`** or **`it has 1 band(s), RGB needs 3`**
Imagery must be 8 bit with three colour bands (a fourth band is read as alpha). Convert with
`gdal_translate -ot Byte -scale in.tif out.tif`; separate band files need `gdal_merge.py -separate` first.

**`cannot decode GeoTIFF`** for a file with JPEG compression
Convert it with `gdal_translate -co COMPRESS=DEFLATE in.tif out.tif`.

**`the map needs a window of ... pixels`**
The file is much sharper than the map and has no overviews to read from. Add them with
`gdaladdo -r average in.tif 2 4 8 16 32`.

**`https://...: ...` when fetching**
A request failed. Náves tries three times with growing pauses and then reports the address and the reason. Check the
network and proxy variables (`https_proxy`, `no_proxy`). Only `https://` addresses are used, and a certificate that
the system or the bundled list does not trust is an error, not a warning.

**`... the service answered something that is not an image`**
The service sent an error page instead of a picture (the first words of it are in the message), for example because the
request was too large or the area is outside the service. Try another source (`naves imagery list`).

**`... this source has nothing for this area`**
Every pixel of the answer is empty or blank: the map lies outside the area the service covers.

**`<file>: the file is missing`**
A file listed in `sources.imagery` is not in the project folder, for example because a project was copied without its
`imagery` folder. When the name is one that `naves imagery fetch` writes, the message gives the command to get it
again; otherwise copy the file back or take it off the list.

**`overlaps no other image, its colours are not matched`**
A file touches the map but does not overlap any other image, so there is nothing to match its colours to. It is used
as it is. Add a file that covers both, or accept the step in colour.

## Roads

**`.../structures_roads.pbo not found; the road pieces are read from the DayZ Server install`**
The roads are built from the game's own pieces, which `naves build` reads from the DayZ Server install
(`addons/structures_roads.pbo`). Install *DayZ Server* from Steam, or set `DAYZ_SERVER` or the path under Settings,
External tools.

**`binarize could not open N road model(s) (...)`**
`binarize` looks for the road models below its `P:` drive, where `naves build` unpacks them from
`addons/structures_roads.pbo` and removes them again after the run. If it still cannot open one, the game's files are
not what Náves expects (a game update, a damaged PBO); the message names the models and the binarize log
(`build/logs/binarize.log`). The one `Cannot open object` line with an empty name that every build logs is harmless.

**`binarize found no road directions in a world with roads`**
Every `Road N dirs` line of the binarize log is 0, so the world file has no road network. Look at the log for why.

**`dayzmake unpack failed`**
dayzmake could not open the PBO; its output is in the message. The result of a successful read is cached in
`$XDG_CACHE_HOME/naves/catalog` (else `~/.cache/naves/catalog`); delete the folder to read the game again.

**`the model ... cannot be read`**
A model of the game is not in the layout Náves expects (ODOL version 56, as of DayZ 1.30). The game was probably
updated; the message names the model.

**`not_drivable`, `road_misses_terrain`, `road_grade` ... in the warnings**
Every line names the rule and a position in map metres. The same findings, with the numbers of the network, are in
`build/report.json`. A road is **not drivable** when it is steeper than a vehicle manages on its surface (30 % on
asphalt and streets; 40 % on gravel tracks, a car at the top of its range; 50 % on dirt tracks, a 4WD; estimates, not
driven in the game)
or more than 20 % tilted across. Between the family's aim and that limit it is amber: steep or tilted, but still
drivable. A road is steeper than its family's aim where the ground is steeper (it follows the ground within the limit
of its family, nine tenths of what vehicles manage); that is why a road on a steep hillside can be amber for a long
stretch. A road can be too steep because the real road is
steep and the map is small (a horizontal scale below 1 steepens everything): raise `[scale] horizontal`, or accept the
finding. `road_misses_terrain` and a road
that is not drivable because it leans sideways usually mean two roads closer than about 25 m at different heights
(a hairpin on a steep flank, parallel streets): the terrain grid cannot follow both. A larger `[scale] horizontal`
moves roads further apart in map metres; otherwise accept the finding. `road_grade` says either "Grade 31 % exceeds
the limit of 27 %" (a stretch that is steeper than the hard limit of its family, the ceiling, not its aim; usually where
a bridge deck, a junction that cannot give way or two roads at very different heights ask for more than the road can
climb) or "Crest or dip too
sharp: vertical radius 40 m, the minimum is 60 m" (the grade changes faster than a vehicle's suspension and view
allow over a crest or a dip) or "The grade changes by 14 % at one point, the most a vehicle takes there is 10 %" (next
to a junction or the end of a bridge deck, within 25 m of it along the road, the grade may change by 10 % at one joint
because a short ramp there is fine; the finding says that a ramp needed more, usually because the junction's plane or
the deck's level was kept and the height difference is large for the length between them. Both values are estimates,
not yet driven in the game, and so is the idea that a break of 14 % is a bump taken at low speed rather than a step). On steep ground with hairpins a finer grid
follows the roads more closely, but it does not remove the findings: on the Berchtesgaden example `road_misses_terrain`
is 4604 at `terrain_size = 1024` and 4015 at 2048, `road_conflict` 749 and 1130. The finer grid costs build time and PBO
size: with the roads the build takes 350 s, 1.8 GiB and gives a 223 MB PBO at 2048, against 325 s, 1.7 GiB and 78 MB at
1024 (without roads: 38 s and 220 MB against 16 s and 75 MB). These figures were measured on a 6.4 km map at 2048,
a grid the game client cannot show (cells of 3.1 m); a finer grid than 1024 needs a map of at least 8 192 m, see
[the grid](project-file.md#grid).

**`the road heights are not solved: <n> unknowns, <m> rows: <status>` or `the road heights cannot be solved: no ground data under the road at (x, y)`**
The build stops. The heights of all roads, with the clearance under bridges over roads, are one calculation; the
first message means the solver found no solution (the message names the size of the problem and the solver's
status), the second that the input is malformed, for example a road with no ground data under it.
There is no setting that changes this. A project with roads should always have a solution, so this is a fault of the
program: report it with the project (`naves.toml`, the edits and the OSM extract) and the message.

**`junction_too_steep`: "Junctions too close for their heights: 12.0 m over 50 m"**
Two junctions have ground heights that differ by more than the road between them may climb, and moving them toward
each other (which the build does, as far as it takes) was not enough, for instance because a roundabout's plane holds
one of them, or a bridge deck that has to clear a road holds the other. The
position is the middle between the two. Move one of the roads (a different OSM node for the side road), or accept
the finding: the road between them is then steeper than its limit and shows as `road_grade`.

**`road_far_from_ground`: "Cut 40.0 m deep; 30 m fit this ground" or "Embankment 45.0 m high; 30 m fit this ground"**
A piece of road is planned further from the original ground than fits its terrain. The limit is 30 m on gentle
ground and grows with the side slope of the ground across the road (0.20 and below: 30 m; 0.40: 65 m; 0.60 and
above: 100 m), since a road on a steep mountainside is cut into it or built out from it. The position is the middle
of the piece, which is measured at its middle against the ground before any road shaped it; bridges are not
measured. It usually means a road held to a height far from its ground, for instance by a junction on a
different slope, or a wrong node of the OSM data. Move the junction or fix the data, or accept the finding.

**`road_over_water`: "Road crosses water 6 m below it without a bridge; look at it"**
The road stands more than 3 m above the original ground over a run where a stream, a river, a canal or a lake lies
under it, and it gets no bridge: the dip is less than 8 m deep (a bridge is built from 8 m), the run is shorter than
half a bridge model, a model would reach a junction or the end of the road, or the road bends more than 2 m off the
straight line of a model. The road is left as it was solved, and the
ground under it as the build leaves it (a fill, as for any road); the position is the middle of the run and the number
is how far the road stands above the ground. Look at it in the game or the map: replace the pieces by a bridge in
`edits/overrides.toml`, move the OSM line, or accept it. Dips without water are filled and not reported.

**`bridge_clearance`: "A bridge clears the road below by only 1.6 m, the minimum is 4.5 m"**
A bridge crosses another road without a shared node and there are not 4.5 m between that road and the deck
(measured from the road's surface to the underside of the deck). To make the room, the build lifts the bridge's road
and lowers the road below, each as far as its grade limits and its vertical radius allow; the finding means that the
two together could not make 4.5 m. The usual cause: the bridge's road and the road below meet at a junction close to
the crossing (or one junction holds both), so the one cannot rise without the other, and the short road between
them would have to be steeper than its limit. Another cause: approaches too short for the deck to rise and a road
below that cannot dip far enough within its own limits. The position is where the two lines cross. Move the junction
away from the crossing in the data, or accept it.

**`bridge_blocks_road`: "Bridge ... lies on the road ... below it: 7.0 m of the 33.6 m the road needs are under a solid part or past the end of the deck"**
A bridge crosses another road and no part of the install's catalog lets that road pass: the road needs more room along the
bridge than its opening has, within the straight room the part may slide in, and the panel grid, lengthened into the road
beside the bridge by what each end lacks (at most 15 m, a metre before a junction, another bridge, a tunnel or a dead end),
has none either. The usual cause is a wide road crossing a short bridge at a small
angle (the road's footprint is its width over the sine of the angle), or a crossing close to the end of the bridge.
The position is where the two lines cross and the number is how much of what the road needs (its footprint, margins,
skew and, for panels and plain decks, the cutting's walls) is under a solid part or outside the deck. The build keeps
the placement with the least missing. Straighten the crossing in the data or accept it; bending the road below so that
it crosses squarely is a later task. A road below that the placement laid away from the deck (a bend too tight for the
pieces) is not reported; one whose placed pieces run under the deck is, wherever its OpenStreetMap line crosses.

**`bridge_buried`: "The ground under bridge ... is 1.3 m above the underside of its deck at 7 places, held by the road ..."**
After the terrain was shaped, ground in the deck of a bridge is higher than the underside of the deck (its height less
what hangs below it there; the metre of air the cut aims at and the foot of what stands are not counted) by more than
0.1 m, or, within the end guard of a free end of the bridge (a cell along the deck, a little more for a deck turned
against the grid; the whole of a deck shorter than two guards), higher than the deck's surface. The message says which
reference its number is above ("above the deck" in the guard, "above the underside of its deck" beyond it). The cut never changes the core of a road that is not the bridge's own, so a road that runs
under the deck (or an arm of a junction next to a lengthened deck end, or a level patch where roads meet) keeps its
ground and it sticks into the deck (inside the end guard only a core does: the shoulder ring of any road takes at most the deck's surface there). The message names the road or patch that holds it; "no road" means that nothing
holds it and the ground was not cut there. The position
is the vertex with the most ground above the reference. Move the other road in the data. Nothing in the project sets a
road's or a bridge's height, so where the holder is the bridge's own road, or the other road cannot be moved, the build
leaves it as it is: look at the place in the game and report it with the way id.

**`bridge_post_floats`: "The posts of bridge ... stand 4.0 m above the ground at the furthest (0.5 m is allowed)"**
A post, pier or wall foot of a bridge model is more than 0.5 m above the shaped ground, so the model hangs in the air
there. The choice of `bridge_wood_25` looks at the original ground; this finding looks at the ground after the shaping
under the deck as it was placed (with the height the road got). The position is the sample furthest from the ground.
Use a bridge that does not stand on the ground (a panel grid).

**`road_roundabout_collapsed`, `road_sidepath_dropped`, `road_stub_dropped`, `road_junctions_merged`, `road_junctions_not_merged`, `bridge_approach_made` (information)**
The build changed the road graph on purpose and says where: a roundabout smaller than 10 m on the map, or tighter than the
tightest arc of its surface, became a plain crossing, a part of a road that runs beside a road of higher rank, a short dead end or a junction next to another was
left out or merged, or a junction was moved off the end of a bridge. Nothing to do; look at the place if the result is
not what you want and change the data (OpenStreetMap) there. The position is the place of the change. For
`road_sidepath_dropped` the message names the stretch, the road it runs beside and the length of the part that was left out
(the `road-analysis` example also lists the roads that were joined to the road instead, and the candidates that stayed with
the reason `clear`, `crosses`, `would_disconnect`, `would_split` or `loops_back`); for
`road_stub_dropped` the message names the stretch and its length (a dead end shorter than 6 m that is not drivable, or
shorter than its own width); for `road_junctions_merged` it names the junction that stays and those that went into it (nodes
of the data, joined by a stretch shorter than 6 m, all within 12 m of each other). `road_junctions_not_merged` names the close
junctions that were left as they are and why (a roundabout among them, two bridge ends or junctions next to them, a longer
road between them, a straightened road that would cross another road, too wide a group, or junctions that a merge itself brought
closer than 6 m). The points of a line that were merged because they lay closer than 0.5 m are not a finding; the
`road-analysis` example lists each (`CLEAN speck`).

**`road_curve_widened` (information), `road_curve_too_tight` (warning)**
The smoothing of the road lines (switched off in this release, so these two findings do not appear) found a curve tighter than the tightest curve piece of its road type. For
`road_curve_widened` it was widened outward (an open corner, a turn of up to 120 degrees, may be cut inward instead)
to the radius of the piece; the message names the stretches, the radius before and after and the furthest move, which
can be several metres, and the position is the tightest point of the curve. A widened curve may end a little under
the wanted radius; the zone is then in `roads.smoothing.zones` of `build/report.json` with `reached: false`. For
`road_curve_too_tight` the curve could not be widened to 90 % of the wanted radius, or widening it would have put the
road in water (`blocked_by_water` in the report), so the road keeps the line OpenStreetMap draws and **cannot be built
as a curve**: the pieces will leave a kink or a gap there. Curves within 25 m of a bridge end are not widened at all.
Look at the place; change the data (OpenStreetMap) so that the curve is wider or has a junction.

**`lot_smoothed`, `lot_surface_only`, `lot_excluded` (information)**
The lot stage (switched off in this release, so these findings do not appear) says what became of every lot. `lot_smoothed`
names the form (a plane or a quadratic), the surface class and the largest move of the ground; `lot_surface_only` names
the reason: `street_side` (a strip along a street) or `no_vertex` (fewer than three terrain vertices inside, too small
for the grid to shape); `lot_excluded` names `not_at_ground` (underground, multi-storey, rooftop, a building, another
`layer`) or `in_water` (more than half of it in water). The position is the lot's centre. Nothing to fix.

**`lot_steep` (warning), `lot_move_limited` (warning)**
A smoothed lot whose slope at its 95th percentile is above `LOT_STEEP` (20 %, the largest fall across a road a vehicle
can take) keeps that slope: the surface follows the lie of the land, and the lot is painted like any other (terraces
are not built). A lot whose group's surface would move a terrain vertex by more than `LOT_MAX_MOVE_M` (3 m) is not
clipped vertex by vertex, which would bring back the local height changes the surface is there to remove; instead the
group's whole change is scaled by one factor (λ), which the message names, so the lot is smoothed less than it could
be. Both are places to look at in the game; nothing in the project sets heights.

**`road_in_lot_dropped` (information)**
A road stretch inside a lot is not built, or an access road is cut at the lot's edge; the message names the stretch,
the lot and the metres. A vehicle drives across the lot instead. An access road whose remainder behind the cut the
placement drops (shorter than half a road piece is wide, or ending under other roads) is reported as lost: the lot has
no entry there, the road is not built, and the build's lot line counts it (`1 entry lost`).

**What a lot can still show.** A road that lies on a lot reports `road_conflict` once the road's centre and the lot's
surface at the road's edge differ by more than 0.3 m; with a flat cross-section that is a cross slope above 0.6 m
divided by the road's width (about 6 % for a 10 m wide road, 8.6 % for a 7 m wide one), so a steep lot produces such
findings for nearly every road over it: they are real faces at the road's uphill edge, not mistakes in the data. The
road-lot finding is counted once per road, lot and 50 m square, at the worst place in the square, so one place next to
a square boundary can give two records and a long parallel road gives one per 50 m. A dead end that reaches less than
4 m into a lot is not cut: it keeps a step of about half the difference between the lot's surface and the ground, and
the place usually reports a `road_conflict` at least that large nearby (the square keeps its worst record, so a
boundary between the end and the lot's edge can take that record to the neighbouring square). A street directly beside
a lot is not pulled to the lot: where the lot's surface differs from the street by more than 0.3 m at the street's
edge, the difference stays as a face and is reported (`road_conflict`).

**Other road findings: `road_not_drivable`, `path_too_steep`, `road_in_water`, `earthworks_capped`, `orphaned_edit`, `join_without_junction`, `tunnel_omitted`, `road_duplicate_dropped`**
Each finding names its rule and a position in map metres. `road_not_drivable` is explained above (steeper or more tilted
than a vehicle manages) and `path_too_steep` is a path that is too steep to walk. `road_in_water` is a road piece
standing in water, `earthworks_capped` a place where the earthworks stop at their cap and leave a step (reported once
per square), `orphaned_edit` an edit that outlived its object (the road it was made on was removed, replaced or moved to
another stretch), `join_without_junction` roads that all end at one point without a junction piece, `tunnel_omitted` a
tunnel that got no pieces, and `road_duplicate_dropped` the part of a road that OpenStreetMap draws a second time under
a higher road and that is not placed. None of them stops the build.

**`road_gap` (warning)**
A road misses what it has to meet: the pieces end more than 0.5 m from a junction's port, a bridge's end or the road
it has to close on (the message gives the distance). A road on which **nothing could be placed** is reported too, also
when its end is free (a dead end or an end under another road): the message says "has no pieces", the distance is the
length of the part that has none, and the position is where the part starts. Typically the road turns away sharply
right behind a junction, so that no piece gets forward. A part shorter than the shortest piece is not reported. A part
that cannot be fitted no longer takes the rest of its road with it: the next part starts where the line is, so only the
part itself is reported. Look at the place; change the data (OpenStreetMap) so that the road does not turn away
within a few metres of a junction.

**`bridge_end_at_junction` (warning)**
A junction lies on the end of a bridge and the build could not move it off the deck (the bridge is too short, a road
passes below the part that would be cut off, or there is no room). The bridge is left as it is. Move the junction
away from the bridge's end in the data (OpenStreetMap).

**`bridge_end_step`: "The deck of bridge ... ends 1.1 m below the junction at (100, 200) it joins: nothing in the project sets heights, look at the site"**
A bridge's deck is level, and one end of it lies at a junction of the bridge's roads (or at the end of another road) that
the height solve put more than 0.5 m above or below it. The deck is held to one junction at the end that faces it; where
the junctions at its two ends want heights that differ (the ground under them differs, or the grade limits keep them
apart), the second end keeps what is left, and that is this finding; so does a deck that the clearance over a road below lifts
above the junction at its end. A vehicle meets a step, and the ground at the deck's end may stand in the lane. The position is the end of the deck;
the message says how far the deck is below or above, and names the junction (its position) or the road that ends there.
Where the road runs from a bridge deck through the patch of a junction of the bridge's roads, with no joint of its own there, the height solve gives it a joint where it enters the patch (and one where it leaves it) and holds those to the junction's plane, so the road climbs or descends from the level deck to the plane in a short ramp inside its family's grade ceiling (a break of the grade up to 10 % at a joint is allowed within 25 m of a deck end or a junction; a break over that is reported by `road_grade`). Only where no ramp is possible is there a step: the height difference is more than the ceiling allows over the length between the deck's end and the patch, or the joints the road has lie so near the patch (a joint within 4 m of where a new one would go is held to the plane instead) that only a bridge deck's end, which is level and cannot be held, is there (then the road is held to the plane by a pin on the line between the deck and its next joint, as before, and may give it up). The road is then below or above the plane where it crosses the patch: that is this finding, for every junction within 25 m of the patch that the road runs through, and its message names the junction (the message then reads "The road beyond the deck of bridge ... is 0.8 m below the plane of the junction at (6747, 7385) where it enters its patch"). Only a junction or a road that belongs to the bridge and ends at the deck's end is compared. A deck inside a chain of roads is compared with the junction's plane at its end if the junction is within its patch; if the junction lies a few metres beyond the patch (nearer this end than the deck's other end), the road beyond the deck is compared with the plane where it runs through the patch, and a ramp between the deck and the patch is not this finding (see `road_grade`). Nothing in the project sets
a road's or a bridge's height, so look at the site in the game and report it with the way id; or change the roads in the
data (OpenStreetMap): a bridge between two junctions on ground that differs by metres is the usual cause; moving one
junction away from the bridge's end gives the road room to take up the difference.

**`road_kink`: "Road ... meets its bridge 17.5 degrees off the deck's heading"**
The road reaches the end of an OpenStreetMap bridge, but its heading there differs from the deck's by more than
5 degrees, so a vehicle meets a corner at the bridge. The bridge's line turns at its node and the road before it had
no room for the straight lead that normally takes the turn (a junction, another bridge or a short road at the node).
Move the junction away from the bridge's end in the data, or accept it. Only the ends of bridges are checked.

**`bridge_not_built`: "Bridge ... (12 m) is plain road: the install has no concrete panels and no bridge model that fits it"**
An OpenStreetMap bridge longer than 3 m, or any bridge over a road, is built from a bridge model or a grid of concrete panels (see
[what a bridge is built from](getting-started.md#roads)). The DayZ Server install has no panels
(`dz\structures\roads\panels\` in `addons/structures_roads.pbo` could not be read, see the warnings) and no bridge
model of the road's surface that fits the bridge and its room, so the bridge is laid as an ordinary road. Check the
install (Steam: verify the files of DayZ Server). A bridge of at most 3 m that crosses no road is filled on purpose and is no finding.

**`bridge model ...: its Geometry LOD cannot be read ...`**
The thickness of a bridge deck is measured on the model; where a model cannot be read, the thickest deck of its type
(or of all bridge models) is used, and where no model can be read the clearance under bridges over roads is not
checked. Such a model is taken as open over its whole length, with no solid ends.

**`bridge model ...: its Geometry LOD has no faces` / `the faces of its Geometry LOD cannot be used (...)`**
What hangs below a deck is measured on the faces of the model's collision geometry. Without them the model is taken
as open over its whole length, as deep everywhere as its lowest point near the deck, with no solid ends.

**`bridge model ...: ... it is not used`**
A bridge model whose ports are missing, whose coordinates are not numbers or lie more than 1000 m out, whose ports
are less than 1 m or more than 200 m apart, or that cannot be read is left out; the roads get the other models.

**`... is not in the DayZ Server install: the bridge parts it holds are not used`, `the bridge part ... is not in the game's files`**
The Sakhal archive (`sakhal/addons/structures_sakhal.pbo`) or the industrial archive
(`addons/structures_industrial.pbo`) is missing, or a model in it; the build goes on without those parts.


## Server mission

**`source mission not found: .../dayzOffline.enoch`**
The mission is built from the `dayzOffline.enoch` folder of the DayZ server install. Install *DayZ Server*
from Steam (Tools) or set `DAYZ_SERVER` to its folder.

**`... already exists`**
The target mission folder is there already. A build removes its output folders first; if you call the
mission step yourself, remove the old folder.

## The game

**`a map of 6400 m with grid.terrain_size = 2048 has terrain cells of 3.125 m; the DayZ client crashes unless they are 4 to 16 m long ...`**
`naves build` refuses a grid whose terrain cells are shorter than 4 m or longer than 16 m: the game client would load
the world and crash a few seconds after the player enters it (an access violation reading address `0xa0`). Set the
`terrain_size` the message names, or make the map wider (`area.real_extent_m` or `scale.horizontal`); the limits per
grid are in [the grid](project-file.md#grid).

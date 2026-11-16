<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Náves: roads that look right in the game

Date: 2026-11-15. Status: approved by the maintainer on 2026-11-15. It builds on the road placement of
`docs/superpowers/specs/2026-11-13-road-heights-design.md` and the measurements in
`docs/research/2026-11-10-road-alignment.md` (section 8: the neighbourhood solver) and
`docs/research/2026-11-14-fabdem.md` (elevation sources). Decisions marked **(maintainer)** are the maintainer's;
everything else is a proposal of this document and says so.

## 1. Intent

**(maintainer)** "A playable map with roads that look right in game." The roads of the Berchtesgaden example are judged
in the game, not by the counts of the rules alone. A rule count that goes down while the road looks worse is a
failure; a count that stays while the road looks right is fine.

What the game showed on 2026-11-14 (screenshots of 42 sites, `/var/tmp/naves-ingame/shots/index.md`, a 1024 test
build with the road models made readable to `binarize` by hand):

- **No road piece is drawn at all** in a normal build: `binarize` cannot open the road models ("Cannot open object",
  104 times; `Road N dirs: 0`), because they are not on its `P:` drive. Bridges, terrain benches and the painted ground
  are there.
- **Look right:** bends, tight R10 arcs, a 180° hairpin, T and X junctions, end caps, a deep cutting, a high embankment,
  a rock face.
- **Look wrong:**
  - gaps between pieces, seen as ledges of 0.5 to 1 m and stubs hanging against a slope;
  - bridge decks not joined to their road, approaches offset sideways or ending in a step;
  - corners sticking out where roads meet without a junction piece;
  - humps and a "ski jump" on steep stretches;
  - no clearance under the two OSM bridges over another road (Kehlsteinstraße 0 m, Silbergstraße about 1.3 m);
  - a roundabout on a steep street in Berchtesgaden town (site 20; OSM ways 257338907, 415138352, 422733023) that comes
    out as confused loops and benches.

## 2. Goals, non-goals, anti-goals

**Goals**
1. The road pieces are drawn in the game from a normal `naves build`.
2. The look faults above are fixed automatically, in the order of how bad they look (section 5).
3. Every stage ends with the same sites photographed in the game before and after, so the result is judged by eye.

**Non-goals (later, each its own design)**
- Tunnels with the game's tunnel pieces, terrain holes and the navmesh.
- Railroads (track, bridges, signals, switches, platforms, road–rail level crossings).
- Importing all OSM features (woods, hedges, single trees, benches, sign posts) with matching objects.
- GUI tools to raise and lower terrain and to edit roads and paths with their terrain **(maintainer: after this plan)**.
- A neighbourhood solver (research section 8: the search is not the weak point).
- Another elevation source as default (FABDEM as an optional non-commercial source is a separate small change).
- Water, the navigation mesh, the code-structure cleanups.

**Anti-goals**
- Changing the player's client settings to make a world load or look right **(maintainer)**. A map must work with any
  settings (the user plays with view distance 10000).
- Game data in the repository, in a test fixture or in the built mod. Game files are read from the user's install at
  build time and only for the build.
- Fixing a count by hiding the fault (e.g. dropping roads that are hard to place, or loosening a rule's threshold).
- Non-reproducible output (AGENTS.md: same input, same output).

## 3. Constraints

- AGENTS.md in full: test first, safe Rust, no `unwrap` on untrusted input, `make check`, coverage ≥ 85 %, docs and
  CHANGELOG in the same commit, crate review before any new dependency.
- The example is an 8 192 m map at `terrain_size = 2048` (4 m terrain cells, 32 m land cells, 256 m surface tiles),
  half scale, so the slopes are steep **(maintainer: steep sections are what the road algorithm must get right)**.
  Terrain cells under 4 m crash the client; Náves refuses such grids.
- Figures in plans and notes are the 2048 figures of the example **(maintainer)**.
- Elevation: Copernicus GLO-30 stays the default. Náves must work anywhere in the world from global data
  **(maintainer)**; no regional elevation models.
- The external tool is dayzmake ≥ 2.0.0; `binarize.exe` of DayZ Tools runs under Proton.
- Game tests: only inside headless sway; own server tree and non-default ports; one game at a time; never edit the
  player's `Documents/DayZ` files; no `-name=` or `-profiles=` on the client; the mod symlink in the DayZ install is
  removed afterwards.
- Implementation by Sonnet subagents; review of their work by the dispatching model (Opus); a subagent may ask the next
  better model for one specific problem **(maintainer)**.

**Baseline** (8.2 km map at 2048, commit `1ddac13`, 2026-11-15): 24 815 road pieces in 2984 stretches, 388 open ends;
`road_misses_terrain` 4806 (1345 above 0.5 m, 145 above 2 m, largest 8.5 m), `road_grade` 1729, `road_conflict` 1089,
`join_without_junction` 877, `road_gap` 388, `road_not_drivable` 250 (5.43 km), `path_too_steep` 171,
`earthworks_capped` 93, `road_in_water` 25, `tunnel_omitted` 21, `road_over_water` 3, `junction_too_steep` 2. Build
396 s, 3.5 GiB, PBO 233 MB.

## 4. Approaches

- **A. One plan in two stages, a game check after each (chosen by the maintainer).** Stage 1 makes the roads appear
  and sets up a repeatable game check; stage 2 fixes the look faults in order of how bad they look. Every step is
  judged with the same sites before and after.
- **B. A separate plan per rule.** Small and quick to review, but many plan rounds and game runs, and the rules act on
  each other (the overlay rule changes gaps and conflicts). Not chosen.
- **C. The rules first, the drawing later.** Quicker to start, but nothing could be judged in the game until the end.
  Not chosen.

## 5. The design

### Stage 1: the roads appear, and a repeatable game check

**1.1 Road models for `binarize`.** During `naves build`, the game's road models are put on the staging `P:` drive
where the world references them (`P:\dz\structures\roads\…`, under `build/staging/P/`). Source: the DayZ Server install's
`addons/structures_roads.pbo`, unpacked with dayzmake as the catalog reader already does. They are removed again after
`binarize` and never reach the mod folder, the PBO or the repository. What `binarize` derives from them into the world
file (object references and its road network data) ships in the PBO, as with every DayZ terrain that uses vanilla
roads; that is allowed **(maintainer)**. The build checks the binarize log: a road model
that it cannot open is a build error naming the model (not 104 lines of noise), and `Road N dirs` must not be all 0
when the world has roads. A normal build already logs one "Cannot open object" with an empty name, also without roads;
the check ignores that one. Proposal: unpack only the models the placed pieces use; whether that is faster than the whole
PBO is for the builder to measure.

**1.2 The game check as a developer tool.** The camera and teleport helper that the screenshot agent wrote
(`/var/tmp/naves-ingame/cammod/`, a small helper mod) and its scripts become a developer tool in the repository, so
every stage can be checked the same way. It:
- takes a build and a list of sites (position, heading, height for the overhead shot), starts a test server and the
  headless client (both only inside headless sway), fixes the time to noon and clears the weather in the test server's
  copy of the mission (AGENTS.md "Comparable screenshots"), moves the camera through the sites and takes an eye-level
  and an overhead shot at each;
- writes the shots, an index and contact sheets under a folder given on the command line (default under `/var/tmp`);
- never touches the player's settings or profile, never passes `-name=` or `-profiles=`, removes its symlinks and stops
  everything also on failure, and compares the player's settings file before and after;
- is never run by `make check` or CI (it needs the game); it has its own `#[ignore = "needs DayZ"]`-style check or a
  documented manual command. Its own logic (site list, index) is tested with fakes like the other tool code.
Where it lives (a `tools/` script, an example binary, or a hidden CLI subcommand) is the builder's call; the helper mod
is our own Enforce Script, signed with a throwaway key made at run time, so it is no game data.

**1.3 The site list.** Sites are picked from the build's own data, not by hand: `build/report.json` (rule, position)
and the placed objects (piece, position, heading). Categories as in the first run: straight pieces meeting at a slight
angle (interior angle 150°–180°), the tightest arcs and hairpins, the worst findings of each road rule, a junction, an
end cap, OSM bridges of each material and a scaled one, a bridge over a dip, a high embankment, a deep cutting, a rock
face beside a road, a painted path and a steep one, plus fixed sites: the roundabout of site 20, Hammerstielstraße with
the dirt track (47.604126 N, 12.949617 E), and the two bridges over a road. The list is stored with the shots so that
"after" uses exactly the "before" sites; sites tied to a finding that a fix removes stay in the list by position.

**1.4 Stage 1 check.** Build the example, run the game check, keep the shots as the baseline of stage 2. Pass: road
pieces are drawn everywhere the shots show roads.

### Stage 2: the look faults, in order

Each step: test first, the 2048 counts before and after (a short section in `docs/research/2026-11-10-road-alignment.md`),
then the next step. The game check runs after the last step on the stage 1 site list (and after any step the reviewer
thinks needs a look).

**2.1 Where roads meet** (the 877 `join_without_junction`, the corners sticking out).
- **Swapped X pieces first (maintainer).** Where the game has an X junction piece with the roles swapped, use it: the
  one street/Asf1 X crossing uses `kr_x_asf1_city` (an X looks the same either way round). T crossings are **not**
  swapped **(maintainer)**: the 117 narrow-asphalt/street T crossings (Asf3 through, City side) would need either a
  3 m width step at every port of `kr_t_city_asf3` or a re-routed geometry; they take the overlay rule instead (the
  street, higher ranked, ends lying on the narrow asphalt road).
- **The overlay rule where no piece exists (maintainer).** Ranking Asf1 > Asf2 > City > Asf3 > Grav > Mud (the
  placement rank of `roadnet::chains`); for equal families the longer through road (the chain length `chains` already
  computes) is the higher. The lower road is placed first, the higher one over it. A lower road that crosses or ends at
  a higher one ends just under the higher road's edge and, where it goes on, starts again under the other edge, without
  end caps there. A higher road that ends on a lower one that runs on lies on top of it, without an end cap. The higher
  road keeps its surface height; the lower road meets it a few centimetres below (proposal: 5 cm) so the surfaces do
  not flicker. Overlaps made this way are intended and are not `road_conflict`. "Placed first" is about which surface
  lies on top, not the fitting order: the placement stays hierarchical as today (higher-ranked chains are fitted first
  and fix junctions and heights); only the lower road's end is shortened and lowered **(this document's call)**.
- **Duplicate roads (maintainer).** Two roads along each other for at least 25 m with centre lines closer than half the
  narrower road's width and headings within 15° are one road drawn twice in OSM: the higher-ranked (equal family: the
  longer) stays, the lower one's overlapping part is dropped; where it leaves, its rest follows the overlay rule. Each
  dropped part is a new finding (`road_duplicate_dropped`, proposal for the name). Roads only; paths (e.g. sidewalks)
  stay.

**2.2 Roundabouts** (the maintainer pointed at site 20). A ring tagged `junction=roundabout` (42 ways in the
Bavarian part of the map alone) is recognised as one closed loop and placed as a loop of arcs on **one tilted plane
that follows the slope (maintainer)**, not a level surface: a level ring on a steep street needs the large flat
cut-and-fill benches seen at site 20. The plane is fitted to the original ground under the ring (proposal: least
squares) and limited to what vehicles can drive across; this needs a new, sloped kind of patch in the terrain shaping
(today patches only level to one height). Roads that join the ring end on it by the overlay rule. A ring too small
for the tightest arc of its family becomes a plain crossing (junction piece or overlay rule). `junction=circular` is
treated the same.

**2.3 Gaps** (the 388 `road_gap`). Research section 8: 54 % are side roads that cannot close against a junction port a
through road already fixed; moving the port up to 25 m along its through road closed 60 of 129 in a test (an upper
bound). The repair: when a side road fails to close, try the junction a few steps along the through road (6.25 m steps
up to 25 m each way), refit the through road from its previous junction, keep the best result. The remaining 43 % are
through roads missing their own junction point; the builder looks at whether the same repair applies there.

**2.4 Bridges.**
- **Joined to their road.** The approaches meet the deck ends in position, heading and height (no sideways offset, no
  step). How (fitting the approach to the deck's port as a fixed end, or moving the deck along its line) is the
  builder's call.
- **Crossings at different heights (maintainer).** Roads that cross without a shared OSM node never join. A tagged
  bridge keeps 4.5 m from the lower road's surface to the underside of its deck. The lower road keeps its height and
  never dips into a cutting for it; the terrain under the bridge is shaped for the lower road. **The upper road, its
  approaches and the junctions near the bridge rise as far as needed (maintainer)**, as real bridges do: their fixed
  heights get a lower bound from the clearance, which goes through the junction holds and the profile solve. Only where
  even that cannot reach 4.5 m (within the grade limits) is it a new finding (proposal: `bridge_clearance`). Berchtesgaden
  has 20 such road crossings, all tagged bridge or tunnel (e.g. Kehlsteinstraße over Scharitzkehlstraße at site 41,
  Silbergstraße primary over Silbergstraße residential at site 42, with `kr_t_asf1_city` and `kr_t_city_city` right
  beside the bridge). Tunnels stay as they are.
- **Untagged crossings at different heights** are moved to the railroad design **(this document's call)**: Berchtesgaden
  has none, so nothing could be judged in the game, and railroads need them anyway.

**2.5 Humps on steep stretches** (`road_grade` 1729, `road_not_drivable` 250; the ski jump of sites 21 and 23). Not yet
diagnosed: the builder first finds out, on the sites of the game check, whether the humps come from vertical bends at
fixed junction heights (the earlier research found 640 of 1180 grade findings there), from short pieces between
patches, or from the terrain shaping, and proposes the fix in the plan's review before implementing it.

### After stage 2

The game check on the stage 1 site list, before and after side by side. The README of the example gets the new
figures; the research note gets the section with the counts of every step.

## 6. What is left to the builder

- The exact geometry of the overlay rule's cut (pieces have fixed lengths: how a lower road "ends just under the
  edge" with 6.25/12.5/25 m straights and arcs), and the depth below the higher surface (5 cm proposed).
- Where the game check tool lives and how it is invoked; the format of the site list.
- Whether 1.1 unpacks the whole PBO or only the used models.
- The order inside stage 2 may change when a step shows that another one comes first (e.g. the overlay rule changes
  the gaps); the reviewer agrees to such a change.
- Finding names (`road_duplicate_dropped`, `bridge_clearance`) and their messages, in the style of the existing ones.

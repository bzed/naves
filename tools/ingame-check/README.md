<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# In-game road check

A developer tool: it photographs a fixed list of road sites of a built map in the game, so that every stage
of a road change can be judged by looking at the same places before and after. `make check` and CI never run
it, because it needs the game.

1. Make the site list from a built project (reads `build/report.json` and places the roads again; needs the
   project's data and the DayZ tools like `road-analysis`):

   ```sh
   cargo run --release -p naves-export --example road-sites examples/berchtesgaden /var/tmp/roadlook/sites.json
   ```

   Sites are picked from the build's own data (`naves_export::sites`): straight pieces that meet at a slight
   angle (interior angle 150 to 180 degrees), the tightest arcs and hairpins, a junction, a dead end, a bridge
   of each material, a bridge over a dip (a bridge that OpenStreetMap does not have), the
   highest embankment, the deepest cutting and the steepest side slope (rock face) from the height profiles
   against the original ground, the steepest painted path and a long gentle one, the two worst findings of each
   road rule, and fixed sites of the Berchtesgaden example (the roundabout, Hammerstielstrasse, the two bridges
   over a road). Keep `sites.json`; a later run with the same file photographs the
   same places.

2. Run the game check on the project's `build/` folder:

   ```sh
   tools/ingame-check/run.sh examples/berchtesgaden/build /var/tmp/roadlook/sites.json /var/tmp/roadlook/before
   tools/ingame-check/run.sh examples/berchtesgaden/build /var/tmp/roadlook/sites.json /var/tmp/roadlook/test 1 2 3  # only sites 1 to 3
   ```

   The out folder gets `shots/`, `index.md`, `sheet1.png` ... (six sites per contact sheet), `sites.json`, the
   server tree with its logs, and `settings.diff`.

## What it does

- Builds the helper mod `NavesCam/` (a static camera for the client and a player teleport for the server, driven
  by small files in `$profile:`) and signs it with a key made at run time (`dayzmake keygen`).
- Makes its own server tree under the out folder with free ports, `verifySignatures = 2`, `BattlEye = 0`, the
  build's mission with noon and clear weather patched into the tree's copy of `init.c`.
- Starts the client only inside a headless sway (`dayz-client-headless.sh` of the `dayz-dev` skill; set
  `DAYZ_DEV_SCRIPTS` if the skill is somewhere else), with `-mod=@naves_<world>;@NavesCam` and no `-name=` and no
  `-profiles=`. The mod folders are symlinked into the DayZ install for the run.
- Takes an eye-level and an overhead shot at each site with `grim`.

## What it protects

- It refuses to start while a DayZ client runs, with less than 20 GB of free memory, or if it would overwrite
  a file it did not make.
- It copies the player's files in `Documents/DayZ/` before and compares `steamuser_settings.DayZProfile` after:
  any difference apart from the `lastMP*` lines that the game writes when it connects fails the run. It never
  edits, replaces or deletes those files.
- Its only files in the client's profile folder are `nc_cam.txt` and `nc_ack.txt`, removed at the end.
- A `trap` stops client, server and sway and removes the symlinks in the install on every exit, also on failure.

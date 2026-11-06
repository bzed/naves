<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Troubleshooting

## Missing tools

**`armake2 not found; set ARMAKE2 (hint: ...)`**
Install bzed's armake2 fork and put it on `PATH`, or set `ARMAKE2` to the program.

**`armake2 is not bzed's fork (no --proton-binarize); install https://github.com/bzed/armake2`**
The `armake2` that was found is the upstream program. Install the fork and point `ARMAKE2` at it.

**`Steam not found; set STEAM_ROOT`**
Náves did not find a Steam folder. Set `STEAM_ROOT` to the folder that contains `steamapps/`.

**`DayZ Tools not found; set DAYZ_TOOLS`**
Install *DayZ Tools* from the Tools section of your Steam library, or set `DAYZ_TOOLS` to its folder
(the one that contains `Bin/Binarize`).

**`Proton not found; set PROTON`**
Install *Proton Hotfix* (or Proton Experimental) from Steam, or set `PROTON` to a `proton` script.

**`armake2: the path in the settings does not exist: ...`**
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

## Server mission

**`source mission not found: .../dayzOffline.enoch`**
The mission is built from the `dayzOffline.enoch` folder of the DayZ server install. Install *DayZ Server*
from Steam (Tools) or set `DAYZ_SERVER` to its folder.

**`... already exists`**
The target mission folder is there already. A build removes its output folders first; if you call the
mission step yourself, remove the old folder.

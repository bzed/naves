<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Command line

```text
naves <COMMAND>
```

`naves --help` and `naves <command> --help` print the same information. `naves --version` prints the version.

## `naves new`

```text
naves new <DIR> --name <NAME> --center <LAT>,<LON> --extent <METRES>
```

Creates the project folder `<DIR>` with a [`naves.toml`](project-file.md) and a `.gitignore` that excludes
`build/` and `secrets/`. It stops with an error if `<DIR>/naves.toml` exists already, and never changes an
existing project.

| Option | Meaning |
|---|---|
| `<DIR>` | the folder to create |
| `--name <NAME>` | display name of the map; the identifier used in file names and config classes (`world`) is derived from it |
| `--center <LAT>,<LON>` | centre of the real-world area in degrees, for example `47.585,12.965` (a negative latitude is written `-33.9,18.4`) |
| `--extent <METRES>` | side length of the real-world square in metres |

The new project has no elevation data yet: add DEM files to `sources.dem` before you build.

## `naves build`

```text
naves build <DIR>
```

Builds the mod and the server mission of the project in `<DIR>`:

1. reads the project and the DEM files;
2. samples the scaled heightmap (an area that the DEM files do not cover is an error that names the position);
3. reads the land cover and OpenStreetMap files of the project, if any (a missing or unreadable file is an error
   that names it; a map that they only partly cover is a warning), and decides the ground of every map cell;
4. paints the satellite image, the surface mask and the tile materials;
5. converts the images, runs `binarize.exe` on the world, packs and signs the PBO;
6. assembles the server mission from the installed `dayzOffline.enoch`, and writes `ATTRIBUTION.txt` into the mod
   folder.

Progress goes to standard error; the last two lines on standard output are the folders to use:

```text
Mod:     <DIR>/build/@naves_<world>
Mission: <DIR>/build/missions/dayzOffline.<world>
```

Everything below `<DIR>/build/` is rebuilt from scratch each time. The signing key is created once in
`<DIR>/secrets/` (`naves_<world>.biprivatekey` and `.bikey`) and reused; **keep the private key out of version
control and do not share it**. The tools the build needs, and the variables that locate them, are listed in
[Installation](installation.md); [Troubleshooting](troubleshooting.md) explains the error messages.

| Variable | Meaning |
|---|---|
| `DAYZ_SERVER` | the DayZ Server install that provides `dayzOffline.enoch` (default: `<Steam>/steamapps/common/DayZServer`) |

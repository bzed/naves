<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Náves

*Náves – terrain generator and map editor for DayZ.*

Náves turns OpenStreetMap and elevation data into DayZ terrains: it reads a real area, builds the
heightmap and surface layers, and packs a signed mod that a DayZ server can load.

## What the name means

*Náves* is Czech for the village green: the open space in the middle of a village, which the houses,
roads and paths are arranged around. The tool works the same way. It starts from the real ground and
builds the place around it. The ASCII spelling `naves` is used for the program, the crates and the
file and PBO prefixes, because DayZ configs and file names only take plain ASCII.

## Status

Early development. The first milestone builds a walkable terrain (heightmap and procedural surfaces)
from a Copernicus elevation model. Roads, buildings, water and railways come in later milestones.

## Licence

Náves is free software under the GNU Affero General Public License v3.0 or later. It contains no
game data; the files it needs from your DayZ installation are read at run time.

<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Installation

## Náves

- **Debian 13 (trixie):** install the `.deb` from the release page with `apt install ./naves_<version>_amd64.deb`.
- **From source:** install Rust 1.95 (`rustup` reads `rust-toolchain.toml`) and run
  `cargo install --path crates/naves --locked`.

## External tools

Building a terrain needs three tools that Náves does not ship. It looks for them when a build starts and
stops with the name of the tool and the environment variable to set if one is missing.

| Tool | Used for | Where Náves looks | Variable |
|---|---|---|---|
| bzed's `armake2` fork | packing, signing, image conversion | `PATH` | `ARMAKE2` |
| DayZ Tools (Steam, *Tools*) | `binarize.exe`, which turns the world into the game's format | `<Steam>/steamapps/common/DayZ Tools` | `DAYZ_TOOLS` |
| Proton (Steam) | runs `binarize.exe` on Linux | `<Steam>/steamapps/common/Proton Hotfix/proton` | `PROTON` |

Other variables:

| Variable | Meaning |
|---|---|
| `STEAM_ROOT` | the Steam folder (the one that contains `steamapps/`); default: `~/.steam/steam`, `~/.local/share/Steam` or `~/.steam/debian-installation` |
| `DAYZ` | the DayZ game folder; default: `<Steam>/steamapps/common/DayZ` |
| `NAVES_BINARIZE_TIMEOUT` | seconds `binarize.exe` may run before it is stopped; default 1800 |

**armake2.** Use the fork at <https://github.com/bzed/armake2>. Náves tells it apart from the upstream
program by `--proton-binarize` in `armake2 -h`; the upstream version signs some files wrongly and is refused.

**Proton and temporary files.** `binarize.exe` runs in a throw-away sandbox with its own Wine prefix under
`/var/tmp`. The first run in a sandbox takes about half a minute. Náves never writes into the Steam
folders; they are only read. The sandbox is deleted after a successful run and kept after a failure, and
the error says where it is.

On Windows the tools run directly (`binarize_x64.exe` from DayZ Tools); this path is not tested yet.

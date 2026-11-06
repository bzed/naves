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
| `DAYZ_SERVER` | the DayZ Server folder the mission is built from; default: `<Steam>/steamapps/common/DayZServer` |
| `NAVES_BINARIZE_TIMEOUT` | seconds `binarize.exe` may run before it is stopped; default 1800 |

**Where a path comes from.** An environment variable wins. Without one, the path in your per-user settings is
used, and without that Náves searches (`PATH`, the Steam folders). The settings are the file `settings.toml` in
Náves' configuration folder (`$XDG_CONFIG_HOME/naves`, normally `~/.config/naves`); the `[tools]` table has the keys
`armake2`, `steam_root`, `dayz_tools`, `dayz`, `dayz_server` and `proton`:

```toml
[tools]
armake2 = "/home/me/bin/armake2"
steam_root = "/mnt/games/Steam"
```

The settings apply to `naves build`, `naves mcp` and the GUI, and never to a project's `naves.toml`. A path that does
not exist is reported by name. A damaged file is reported as a warning, left as it is, and ignored.

**armake2.** Use the fork at <https://github.com/bzed/armake2>. Náves tells it apart from the upstream
program by `--proton-binarize` in `armake2 -h`; the upstream version signs some files wrongly and is refused.

**Proton and temporary files.** `binarize.exe` runs in a throw-away sandbox with its own Wine prefix under
`/var/tmp`. The first run in a sandbox takes about half a minute. Náves never writes into the Steam
folders; they are only read. The sandbox is deleted after a successful run and kept after a failure, and
the error says where it is.

On Windows the tools run directly (`binarize_x64.exe` from DayZ Tools); this path is not tested yet.

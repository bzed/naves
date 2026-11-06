<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Settings

The **Settings** button at the right end of the dock, or **F2**, opens a window that dims the rest. What is in it
belongs to you, not to the map: **it applies to every project**. It is stored per user in
`settings.toml` in Náves' configuration folder (`$XDG_CONFIG_HOME/naves`, normally `~/.config/naves`; on Windows
the `naves` folder of the user's configuration folder), never in a project's `naves.toml`, so that a project
stays the same for everyone who opens it. A project's own values are in level 0. Every change is saved as soon as
you make it. **Esc** closes the window.

## Keyboard

Every shortcut of the [table of defaults](shortcuts.md), grouped by level in the order of the stack. Only what is
built is listed. The box at the top searches the names.

- Click the key of a row: the button reads **Press a key**. The next key, with its modifiers, is the new one.
  **Esc** cancels. No shortcut fires meanwhile.
- A key that is already in use is never taken silently: the window says "R is Scale in every level." and asks
  **Replace** or **Cancel**. Replace leaves the other action with no key (its row says **no key**). The same key in two
  different levels is no conflict, but a key used everywhere conflicts with all levels.
- A changed row says **changed** and has its own **Reset**; **Reset all to defaults** is at the bottom.
- **Esc**, **Enter** and **F1** cannot be changed, so that a cancel and the way to the help always work. **Tab**
  and **Shift+Tab** cannot be assigned, so that the way through the panels always works. Ctrl+Tab and every other key
  can be.
- Any key the keyboard can produce is allowed, but the window only hears keys that egui has a name for. On a German
  keyboard the keys **ö**, **ü**, **ä** and **ß** have none: the button stays at "Press a key". The defaults avoid
  such keys.

## Map

- **Invert the zoom direction of the wheel.**
- **Other levels are drawn at** (40 % by default): how strongly the levels you are not working on fade.
- **Interface scale**, from 0.5 to 3.
- **Unfold without animation**: the stack's layers unfold in a tenth of a second; this switches the animation off.
- **A project opens with**: the base map and the layers that are switched on.

## External tools

The paths of armake2, Steam, DayZ Tools, DayZ, DayZ Server and Proton. Each row shows the path in use and where it
comes from: **From an environment variable**, **From the settings**, **Found by searching** or **Not found**.
An environment variable wins over a path you type here, and a path here over the search. The paths are used by the
next build; see [Installation](installation.md#external-tools).

## The file

`settings.toml` holds only what differs from the default, so that a later version's new defaults still reach you.
Shortcuts are stored by a stable id and the key as text:

```toml
[shortcuts]
build = "F9"
"level.hide" = ""        # no key

[map]
invert_zoom = true
fade_percent = 30
interface_scale = 1.25
start_base = "slope"
start_layers = ["too_steep_to_walk"]

[tools]
armake2 = "/home/me/bin/armake2"
```

Ids and keys that this version does not know are kept when the file is saved, so that a file can be shared between
versions. A key name that is not understood keeps its default, and the window says so. Two actions on one key are
shown, and the first one in the table wins.

**A damaged file is not overwritten.** Náves starts with the defaults, the window says what is wrong, your changes hold
until you close the program, and nothing is saved. If the folder cannot be written the window says "Not saved:" and why.

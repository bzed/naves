<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Keyboard shortcuts

The defaults of the [GUI](gui.md); every key can be changed in the [settings](settings.md). Press **F1** in the editor for the keys that work in the level you are in.
Only what exists today does anything: a key whose action is not built yet is ignored. Letters are logical keys, so
a tool's letter is what the keycap says, on a German keyboard too; no default needs Shift or AltGr for a symbol.

While a text field has the keyboard, typing is typing: no single key fires. Only the F-keys, Esc, Enter,
Ctrl+B and Ctrl+Tab still work.

**The left hand works the keyboard, the right hand stays on the mouse.** Every default you use while working with the
mouse is on the left half of the keyboard (`1` to `6`, `Q W E R T`, `A S D F G`, `X C V B`, Esc, Tab, Space, Shift,
Ctrl and Alt). When the left half runs out, a whole set moves to the right half together: the problem keys, the level
visibility (`H`), the brush size (`,` and `.`), help and the settings. A second key for the same action has its own
row: the digits `7`, `8`, `9` and `0` go to the same levels as `Alt+1` to `Alt+4`, `Delete` is a second key for
Delete, `Enter` for Finish, `F5` for Regenerate.

**Undo, redo and rebuild:** `Ctrl+Z` and `Ctrl+Shift+Z` undo and redo the edits of road lines (see
[the GUI](gui.md#edits-undo-and-redo)); `Ctrl+R` and `F5` build the roads again. The keys do nothing while there is
nothing to undo or redo.

**Moving the view:** hold `W`, `A`, `S` or `D` to pan the map north, west, south or east, `Q` or `E` to zoom out or
in. Hold **Shift** with one of them to move three times faster, **Ctrl** for three times slower. They do nothing
while you type in a field. **Esc** is Select: it cancels what is in progress and goes back to Select, so Select has
no letter. The rail shows `Esc` over "Select".

**Tab** and **Shift+Tab** always move through the panels. The **middle mouse button** (drag) and **W A S D** pan
the map, the **wheel** zooms. **Space** finishes a line, so it does not pan.

| Key | Action | Where |
|---|---|---|
| `Alt+4` | Go to Project | Everywhere |
| `0` | Go to Project | Everywhere |
| `1` | Go to Terrain | Everywhere |
| `2` | Go to Ground | Everywhere |
| `3` | Go to Water | Everywhere |
| `4` | Go to Roads and paths | Everywhere |
| `5` | Go to Vegetation | Everywhere |
| `6` | Go to Buildings | Everywhere |
| `Alt+1` | Go to Objects | Everywhere |
| `7` | Go to Objects | Everywhere |
| `Alt+2` | Go to Life | Everywhere |
| `8` | Go to Life | Everywhere |
| `Alt+3` | Go to Economy | Everywhere |
| `9` | Go to Economy | Everywhere |
| `W` | Pan north (hold) | Everywhere |
| `A` | Pan west (hold) | Everywhere |
| `S` | Pan south (hold) | Everywhere |
| `D` | Pan east (hold) | Everywhere |
| `Q` | Zoom out (hold) | Everywhere |
| `E` | Zoom in (hold) | Everywhere |
| `G` | Move | Everywhere |
| `C` | Line | Everywhere |
| `V` | Area | Everywhere |
| `B` | Brush | Everywhere |
| `R` | Rotate | Buildings |
| `Shift+R` | Scale | Buildings |
| `T` | Place | Buildings |
| `R` | Smooth | Terrain |
| `T` | Flatten | Terrain |
| `Shift+R` | Ramp | Terrain |
| `Shift+C` | Hole | Terrain |
| `R` | Soften an edge | Ground |
| `R` | Cut | Roads |
| `T` | Join | Roads |
| `Shift+T` | Mark a bridge | Roads |
| `R` | Cut out | Vegetation |
| `T` | Place one tree | Vegetation |
| `Shift+C` | Make a composition | Buildings |
| `Shift+T` | Tag an entrance | Buildings |
| `Shift+B` | Hole for a bunker | Buildings |
| `R` | Rotate | Objects |
| `Shift+R` | Scale | Objects |
| `T` | Place | Objects |
| `Shift+C` | Make a composition | Objects |
| `T` | Place a group or a spawn | Life |
| `T` | Place an event | Economy |
| `H` | Hide the active level | Everywhere |
| `Shift+H` | Show the other levels in full | Everywhere |
| `Ctrl+Tab` | Switch between map and 3D view | Everywhere |
| `F` | Frame the selection | Everywhere |
| `Shift+F` | Frame the whole map | Everywhere |
| `Shift+V` | Pick the last earthwork shape again | Everywhere |
| `Shift+X` | World or local axes | Everywhere |
| `Shift+G` | Follow the ground | Everywhere |
| `X` | Delete | Everywhere |
| `Delete` | Delete | Everywhere |
| `Ctrl+G` | Duplicate | Everywhere |
| `Ctrl+Z` | Undo | Everywhere |
| `Ctrl+Shift+Z` | Redo | Everywhere |
| `Ctrl+F` | Search items | Everywhere |
| `Ctrl+P` | Command palette | Everywhere |
| `,` | Brush smaller | Everywhere |
| `.` | Brush larger | Everywhere |
| `Ctrl+,` | Brush weaker | Everywhere |
| `Ctrl+.` | Brush stronger | Everywhere |
| `Space` | Finish a line or area | Everywhere |
| `Enter` | Finish a line or area | Everywhere |
| `Esc` | Cancel and go back to Select | Everywhere |
| `Ctrl+R` | Regenerate the active level in the changed area | Everywhere |
| `F5` | Regenerate the active level in the changed area | Everywhere |
| `Ctrl+B` | Build | Everywhere |
| `F8` | Next problem | Everywhere |
| `Shift+F8` | Previous problem | Everywhere |
| `F1` | Show all shortcuts for this level | Everywhere |
| `F2` | Open the settings | Everywhere |

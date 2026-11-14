<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Would migrating the GUI to gpui-kit make sense?

Investigation, 2026-11-06. All crate facts from crates.io (registry API), all code facts from this
repository at the time of writing.

## What the GUI is today

- `eframe`/`egui` 0.37 with the `glow` (OpenGL) backend, single window (`ViewportBuilder::default()`),
  custom bottom dock, no `egui_dock`.
- About **5,900 lines** of production GUI code plus **3,900 lines** of `egui_kittest` UI tests
  (`crates/naves/src/gui/`).
- The heart is the map viewport (`viewport/map.rs`): background renders produce RGBA buffers that
  become one `egui::TextureHandle` per layer, plus `ui.painter()` overlays for selection frames,
  handles and cross-hairs. Pan/zoom is plain math on `egui` responses.
- A 961-line configurable keyboard shortcut system (`shortcuts.rs`) reading user bindings from the
  preferences file.
- Background work on `std::thread` + `mpsc`, drained each frame; idle repaint capped at 10 Hz
  (`request_repaint_after(100ms)`).
- The design doc chose egui explicitly (§5.9: "egui + wgpu; the same stack STAG.vision uses; proven
  to handle 3.6 M objects"). The GUI is days old.

## What gpui-kit is

Facts from crates.io (2026-11-06):

| Crate | Version | Publisher | Lines of Rust | Notes |
|---|---|---|---|---|
| `gpui-kit` | 0.9.0 (2026-11-02) | Longbridge (huacnlee) | small umbrella | 14 releases in 2 months, 278k downloads |
| `gpui-component` | 0.9.0 | Longbridge | 92,500 | the component library; exists since Feb 2025 (as `gpui-component`), 469k downloads |
| `gpui-base` | 0.9.0 | Longbridge | 107,000 | behavior/infrastructure layer |
| `gpui-pre` | 0.3.11 | Longbridge | 69,000 | **third-party snapshot of Zed's gpui** (`zed@f816e1f`), exact-pinned `=0.3.11`, re-snapshotted roughly weekly |
| `gpui` (official) | 0.2.2 (2025-10-22) | Zed | 63,000 | **stale for ~1 year** on crates.io |

- All Apache-2.0, all registry-sourced (no git deps — passes `cargo deny` sources and the licence
  allow-list).
- Linux: Vulkan only via `blade-graphics` (no OpenGL fallback), X11 + Wayland, `xkbcommon`,
  `cosmic-text`, `font-kit`. Windows: DirectX backend (HLSL in-tree), shipped in Zed's Windows
  release. Edition 2024; MSRV unlisted — must be verified against our pinned 1.95.
- Testing story exists: `#[gpui_kit::test]` UI integration tests (headless), comparable in intent to
  `egui_kittest`.
- Rich component catalog (inputs, tables, trees, command palette, settings panel, charts), but no
  docking system (we hand-roll our dock anyway).

## Advantages

1. **Component depth.** DataTable, Tree, Command palette, Settings, rich overlays out of the box;
   egui gives primitives and we hand-roll everything (dock, palette, prefs window, lists).
2. **Retained entity model.** Redraw only on change; better idle CPU than egui's frame loop even
   with the 100 ms cap. State changes are explicit (entities, subscriptions) instead of
   "re-run everything each frame".
3. **Actions and keybindings.** GPUI's action/key-context system is a better foundation for the
   961-line configurable shortcut layer than manual `InputState` inspection. (User-configurable
   bindings would still be custom code.)
4. **First-class async.** Workers map naturally to `cx.spawn`/`background_spawn` with proper
   cancellation, instead of threads + channels polled per frame.
5. **Text and polish.** GPU text via cosmic-text, mature theme/token system, shadcn-style design
   discipline — closer to the manual's palette than raw egui, though a custom theme is needed
   either way.
6. **Zed behind the core.** gpui powers a flagship editor on all three desktop platforms; gpui-kit
   has real adoption (469k downloads of gpui-component) and very active maintenance.

## Disadvantages

1. **A full rewrite, not a port.** ~10,000 lines (GUI + UI tests) change: new app shell, entity
   model for `NavesApp`, every panel re-authored, all `egui_kittest` tests rewritten, manual
   screenshots redone. Realistically a multi-week project, and the 85 % coverage floor must be
   defended throughout.
2. **The map viewport is the highest-risk part.** egui's texture + painter API is an unusually good
   fit for "background render → per-layer texture → overlay shapes". GPUI needs a custom `Element`
   with image entities and paint paths — more code, less convenient, and it is the heart of the app.
3. **Fails our dependency review on maintenance/stability grounds.** 0.x with weekly releases and
   breaking changes; the actual gpui arrives as `gpui-pre`, exact-pinned snapshots of Zed's monorepo
   republished by a third party (Longbridge), while Zed's own crates.io `gpui` has been stale for a
   year. Every `cargo update` can break the build; there is no semver contract to rely on.
4. **The `unsafe` audit is impractical.** ~270,000 lines across `gpui-pre`/`gpui-base`/
   `gpui-component` with substantial `unsafe` GPU/platform FFI. Our review standard ("every unsafe
   block justified and sound") cannot be met; adopting gpui-kit means trusting Zed + Longbridge
   wholesale — a deviation that needs the maintainer's explicit sign-off.
5. **Linux rendering regresses in weak-GPU setups.** Vulkan only, no OpenGL fallback: problems on
   VMs, X forwarding, older drivers. `eframe`/glow runs essentially everywhere. For a modder tool
   this is a real distribution risk.
6. **Debian packaging grows.** New build/runtime deps (`libxkbcommon`, fontconfig, Vulkan loader),
   a much larger vendored tree, slower builds, bigger binaries.
7. **Smaller knowledge base.** egui's documentation and example ecosystem are far larger; gpui
   documentation outside gpui-kit.com is thin. Accessibility on Linux/Windows is weaker than
   egui/AccessKit.
8. **Nothing is pushing us.** The GUI is two days old, was an explicit design decision, and no
   current requirement exceeds egui's capabilities.

## Verdict

**Do not migrate now.** The current GUI is new, tested and adequate; gpui-kit fails the project's
dependency review on the maintenance/stability criteria (0.x churn, third-party monorepo snapshots,
exact pins) and on the practicality of the `unsafe` audit; and the rewrite cost is weeks of work with
the coverage floor at stake. The one strong technical argument (rendering quality/perf) does not
apply — we render the map ourselves into textures either way.

Revisit if any of these become true:

- the GUI hits an egui wall (e.g. the object/economy editors need huge virtualized data views egui
  cannot deliver smoothly);
- gpui-kit reaches a stability plateau (1.x, or Zed publishing `gpui` to crates.io with real semver
  discipline) so upgrades stop being breaking;
- a concrete UX requirement emerges that egui cannot meet (complex docking, rich text, theming
  depth).

Until then, invest in the egui GUI itself.

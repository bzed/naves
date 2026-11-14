<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Rerun's design system for the naves GUI: kit fork vs upstream dependency

Investigation, 2026-11-07, following `2026-11-06-gpui-kit-migration.md`. Goal: a modern, polished look
without leaving egui — using Rerun's `re_ui` design system (the best-looking egui application in
existence), either through the third-party vendored `rerun_ui_kit` or directly from upstream.

## What was tested

1. **`rerun_ui_kit` 0.2.0** (crates.io, MIT OR Apache-2.0): `re_ui` 0.38.1 vendored as a standalone
   crate by one author, with the rerun data stack removed. Cloned to `/tmp` and reviewed.
2. **Upstream `re_ui` 0.42.0** (crates.io, MIT OR Apache-2.0, published 2026-11-04): probed as a
   dependency in a scratch crate on stable rustc 1.99.1.

## rerun_ui_kit review

- ~21,000 lines of Rust across 72 files, verbatim vendoring + documented minimal surgery
  (`PORTING.md` is a porting ledger: file mapping, dependency substitutions, pinned upstream tag).
- `unsafe_code = "forbid"`; the 33 `unwrap`/`expect` hits are in tests or init-time justified;
  59 unit tests pass (5 snapshot tests `#[ignore]d`, reference images not published).
- Dependencies are sane: egui + egui_extras + a dozen small pure-Rust crates (crossbeam, jiff,
  nucleo-matcher, ron, strum, parking_lot…). No eframe in the lib (dev-dep only). No wgpu, no protoc,
  no analytics.
- **Findings against using it as a dependency:**
  - Pinned to egui **0.36**; our GUI is on **0.37** (egui types are not interchangeable).
  - Published 0.2.0 **does not compile on stable 1.95**: `src/span.rs` (verbatim from `re_span`) uses
    unstable `core::range::Range`. Removing one 5-line `impl` makes the whole crate compile clean on
    stable. (Upstream rerun develops on nightly; verbatim vendoring inherits nightly-isms.)
  - 0 stars, 11 commits, 72 downloads, one author — fails the AGENTS.md maintenance review as an
    upstream. As a **fork we own**, that objection dissolves; the review then applies to the code,
    which is good.

## Upstream re_ui 0.42.0 as a dependency (probe results)

- Targets **egui 0.37** — exactly our version (also `egui_kittest` 0.37). MSRV **1.98**; our pinned
  1.95 is too old, stable 1.99.1 works.
- Compiles on stable 1.99.1.
- **Dependency tree: 570 unique crates vs 360 in our workspace today (+210).** It drags in:
  - the full **wgpu 30** stack (re_ui requires `eframe` with the `wgpu` feature; we render with
    `glow`, so this is compiled dead weight, and feature unification turns on `experimental`,
    `inspection`, `system_fonts` on our own eframe);
  - **16 `protoc-bin-vendored` crates** (via `re_build_tools`, rerun's gRPC data-model machinery);
  - **`re_analytics`** — rerun's telemetry crate, a hard dependency of re_ui 0.42.0: compiled into
    our binary even if never initialized;
  - `notify` (design-token hot reload), `wayland-client`/`wayland-protocols` (decoration probing);
  - 21 `re_*` crates total. (re_ui 0.41.0 was worse: it hard-depended on `re_entity_db`,
    `re_log_types`, `re_uri` — the full data model. 0.42 slimmed down but is still an internal
    viewer-support crate, not a product for external consumers.)
- Churn: re_ui is an internal crate of a fast-moving product, released weekly-ish, MSRV advancing
  with their toolchain; every update can demand another toolchain bump of ours.

## Comparison

| | Fork of the kit, re-ported to re_ui 0.42 | Upstream re_ui 0.42 as locked dependency |
|---|---|---|
| egui version | 0.37 after re-port | 0.37 now |
| Tree cost | +~15 small crates | +~210 crates (wgpu, protoc, analytics, wayland, 21 re_*) |
| Telemetry crate in binary | no | yes (`re_analytics`) |
| Toolchain | 1.95 fine for kit 0.2; 0.42 base likely needs the bump too | needs ≥1.98 |
| Maintenance | we own it; re-port from upstream when we want (ledger: PORTING.md) | lockfile-pinned; updates opt-in but MSRV treadmill is rerun's |
| Bus factor | ours; upstream (rerun) as design source | rerun the company — passes review |
| Effort to adopt | re-port 0.38.1→0.42.0 (three-way merge, documented recipe), SPDX headers, lint/coverage posture decision | near zero |
| Review verdict | no third-party upstream at all | tree "reasonable for what it gives us"? — the AGENTS.md test is strained |

## Recommendation

**Fork the kit and re-port it to `re_ui` 0.42.0.** The kit's own `PORTING.md` is the recipe: take its
0.38.1 port as the base, apply the upstream 0.38.1→0.42.0 diff (mostly mechanical, verbatim
vendoring), land on egui 0.37, and patch the few nightly-isms for stable (the span.rs fix proves the
class of problem is small). Vendor it as a workspace crate (e.g. `crates/naves-ui`), keep the
diff-vs-upstream ledger, add SPDX headers and an OFL exception for the Inter font in `deny.toml`.

Depending on upstream `re_ui` directly is the zero-effort option and rerun is a solid organisation,
but it couples a terrain tool's build to another product's viewer platform: +210 crates including a
dead-weight wgpu stack, vendored protoc binaries, and a telemetry crate in the binary. That fails the
"dependency tree reasonable for what it gives us" bar for a UI skin.

Either way the toolchain pin moves to ≥1.98 (the re-port's source base demands it too), and either
way the actual project after sourcing is the same: the design pass itself — our pale-green manual
palette on their token system, adopting their widgets (command palette, modals, list items, tab
bars, notifications) in the existing panels. The map viewport and all 3,900 lines of `egui_kittest`
tests stay untouched; this is a chrome change, not a framework migration.

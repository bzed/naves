<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# naves-ui: Rerun's Design System in the Náves GUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Vendor `rerun_ui_kit` as a workspace crate `crates/naves-ui`, re-port it to upstream `re_ui` 0.42.0 (egui 0.37), and adopt its design system in the GUI — theme first, then three bounded widget adoptions.

**Architecture:** The kit (a vendored, de-rerunned copy of Rerun's `re_ui`, ~21k lines, MIT OR Apache-2.0) becomes a workspace crate we own. A documented three-way merge (its own `PORTING.md` is the recipe) moves it from the 0.38.1/egui-0.36 base to upstream 0.42.0/egui-0.37, matching our GUI's egui version. The GUI then applies the kit's design tokens and adopts individual widgets; the map viewport and the `egui_kittest` test suite stay untouched.

**Tech Stack:** Rust 1.99 (toolchain bump from 1.95, required by `re_ui` 0.42.0 sources), egui/eframe/egui_kittest 0.37, `rerun_ui_kit` 0.2.0 sources as the fork base, upstream `rerun-io/rerun` tags 0.38.1 and 0.42.0 as the merge base and theirs.

**Spec:** `docs/research/2026-11-07-rerun-design-system.md` (the investigation and the fork-vs-upstream decision) and `docs/research/2026-11-06-gpui-kit-migration.md` (why we stay on egui). The plan argues from these; executors read both, plus AGENTS.md.

## Global Constraints

- Toolchain moves to **1.99**: `rust-toolchain.toml` `channel = "1.99"`, `[workspace.package] rust-version = "1.99"`, AGENTS.md toolchain line. Required by `re_ui` 0.42.0 (MSRV 1.98).
- egui stays **0.37** everywhere; after Task 3 the tree contains exactly one egui version.
- `naves-ui` is vendored third-party code, not naves code: no `lints.workspace = true` (keeps the port ledger diffable against upstream; its own `[lints.rust] unsafe_code = "forbid"` stays), licensing via a `REUSE.toml` entry (not per-file headers), an OFL exception in `deny.toml` for the Inter font. **This deviation from the workspace lint policy is deliberate and maintainer-approved.**
- Coverage: `make coverage-check` excludes `naves-ui` (`--exclude naves-ui`); AGENTS.md gains a sentence defining the vendored-crate policy. The exclusion applies to vendored code only, never to code we write.
- `make check` passes at every commit. Conventional prefixes; each commit carries its tests, doc updates and changelog line (AGENTS.md).
- Tasks 4–7 are UI tasks: the executor invokes the frontend-design skill before starting each of them (AGENTS.md rule).
- No git or path dependencies outside the workspace — the kit is *copied in*, not referenced.

## Review Focus

The five failure modes most likely to bite, each pinned to its owning task:

1. **Dark theme breaks our explicit colours** — the kit ships dark and light tokens; the GUI hardcodes `Visuals::light()` today and paints `GREEN`/`ERROR` on light backgrounds. If egui ever runs dark, those must still read. Task 4 pins: a kittest test asserts `contrast(GREEN, dark panel fill) >= 3.0` and the same for `ERROR` on the dark token set.
2. **Inter font silently fails to install** — `apply_style_and_install_loaders` embeds the font; a broken install falls back to egui defaults with no error. Task 4 pins: after apply, a kittest test asserts the context's font definitions contain the kit-installed Inter family.
3. **The command palette ignores user-configured shortcuts** — shortcuts.rs parses user bindings; hardcoding Ctrl+P breaks remapped configs. Task 7 pins: the palette opens via the binding resolved through `shortcuts::Bindings`, and a test opens it with a non-default binding.
4. **A modal does not block the map** — clicks/drags must not pan or pick through the preferences modal. Task 6 pins: a kittest test drags on the map while the modal is open and asserts the view did not pan.
5. **The coverage exclusion excludes too much** — `llvm-cov --exclude naves-ui` must not swallow our crates' lines. Task 2 pins: after the change, `make coverage-check` reports ≥ 85 % *and* the report's file list contains `crates/naves/src/gui/` files but no `crates/naves-ui/` files.

---

### Task 1: Toolchain bump to 1.99

**Files:**
- Modify: `rust-toolchain.toml` (channel line)
- Modify: `Cargo.toml` (`[workspace.package] rust-version`)
- Modify: `AGENTS.md` (the "Toolchain: Rust **1.95**" line in Repository layout)
- Modify: `CHANGELOG.md` (under `## [Unreleased]`)

**Interfaces:**
- Consumes: nothing.
- Produces: a workspace that builds on 1.99; nothing else changes.

- [ ] **Step 1: Change the pins**

`rust-toolchain.toml`: `channel = "1.99"`. `Cargo.toml`: `rust-version = "1.99"`. AGENTS.md toolchain line: `Rust **1.99**`. `rustup toolchain install 1.99` if not present (the environment already has 1.99.1 as `stable`).

- [ ] **Step 2: Rebuild and fix lint fallout**

Run: `cargo check --workspace --locked`, then `make lint`, then `make test`.
Expected: check passes; clippy across 4 minor rustc releases may raise new lints in *our* code — fix each at the site with `#[expect(lint, reason = "...")]` or a code change, never by relaxing the workspace set.

- [ ] **Step 3: Verify**

Run: `make check`.
Expected: all steps green (fuzz's nightly toolchain is separate and unaffected; the deb CI job installs the pinned toolchain via rustup, so it follows automatically — watch the next CI run).

- [ ] **Step 4: Changelog + commit**

Add under `## [Unreleased]`: `- Build: the Rust toolchain pin moved from 1.95 to 1.99 (required by the vendored design system).`

```bash
git add rust-toolchain.toml Cargo.toml AGENTS.md CHANGELOG.md
git commit -m "build: pin the Rust toolchain to 1.99"
```

---

### Task 2: Vendor the kit as `crates/naves-ui` (0.38.1 base) and set the policy

**Files:**
- Create: `crates/naves-ui/**` (copied from the kit clone, see steps)
- Create: `REUSE.toml` entry for `crates/naves-ui/**` (repo root `REUSE.toml` — create the file if absent)
- Modify: `deny.toml` (scoped OFL exception for the Inter font)
- Modify: `Makefile` (`coverage-check` line)
- Modify: `AGENTS.md` (Coverage section: vendored-crate sentence)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: the `rerun_ui_kit` 0.2.0 sources. Clone to `/tmp/opencode/` if the clone from the investigation is gone: `git clone --depth 1 https://github.com/jiangheng90-opensource/rerun_ui_kit`. Apply the one stable-fix known from the investigation: delete the `impl From<Span<Idx>> for core::range::Range<Idx>` block in `src/span.rs` (unstable `core::range::Range`; upstream develops on nightly).
- Produces: package `naves-ui`, lib `naves_ui`, exposing the kit's public API (`apply_style_and_install_loaders`, `design_tokens`, `modal`, `list_item`, `command_palette`, `icons`, `menu`, `notifications`, `UiExt`, …) on egui 0.36 for now. Tasks 4–7 consume this API; Task 3 re-bases it onto egui 0.37 / `re_ui` 0.42.0, where upstream may have renamed items — later tasks read the actual `crates/naves-ui` source, not this plan.

- [ ] **Step 1: Copy and rename**

Copy the whole kit tree (src/, data/, examples/, PORTING.md, both LICENSE files) to `crates/naves-ui/`. In `crates/naves-ui/Cargo.toml`: `name = "naves-ui"`, keep the kit's dependency list as-is (egui 0.36 for now — the tree briefly carries two egui versions; `bans multiple-versions = "warn"` tolerates it until Task 3). Rename `rerun_ui_kit::` to `naves_ui::` in all doctests and the example (PORTING.md rule R5).

- [ ] **Step 2: Licensing and policy paperwork**

`REUSE.toml` (root), one entry:

```toml
[[annotations]]
path = "crates/naves-ui/**"
SPDX-FileCopyrightText = "Rerun contributors; jiang heng"
SPDX-License-Identifier = "MIT OR Apache-2.0"
```

`deny.toml`: add a scoped exception modelled on the `epaint_default_fonts` one — `crate = "naves-ui"`, `allow = ["OFL-1.1"]` with a comment naming `data/Inter-Medium.otf` and `data/OFL.txt`.

`Makefile` `coverage-check`:

```make
coverage-check:
	$(CARGO) llvm-cov --workspace $(LOCKED) --exclude naves-ui --fail-under-lines $(COVERAGE_MIN)
```

AGENTS.md, Coverage section, add: "Vendored upstream crates (`crates/naves-ui`) are held to their upstream tests plus tests for our modifications, not to the 85 % floor; `make coverage-check` excludes them. The exclusion is for vendored third-party code only — never for code we wrote."

- [ ] **Step 3: Verify the crate on stable**

Run: `cargo test -p naves-ui --locked`.
Expected: the kit's 59 tests pass (5 snapshot tests stay `#[ignore]d` — their reference images are not published).

- [ ] **Step 4: Verify the workspace gates**

Run: `make check`.
Expected: green. Clippy lints `naves-ui` with default lints only (it does not opt into the workspace set) — fix any default-lint failures with minimal edits, keeping the diff-against-upstream ledger boring. If `make deny` complains about the two egui versions, confirm it is a `multiple-versions` **warn**, not an error.

- [ ] **Step 5: Verify the coverage exclusion (Review Focus #5)**

Run: `make coverage-check`, then inspect the report's file list (e.g. `cargo llvm-cov report --summary-only` paths).
Expected: total ≥ 85 %; report contains `crates/naves/src/gui/` lines and no `crates/naves-ui/` lines.

- [ ] **Step 6: Changelog + commit**

Add under `## [Unreleased]`: `- Build: vendor Rerun's design system (from rerun_ui_kit 0.2.0, MIT OR Apache-2.0) as the workspace crate naves-ui; not yet used by the GUI.`

```bash
git add crates/naves-ui REUSE.toml deny.toml Makefile AGENTS.md CHANGELOG.md
git commit -m "build: vendor rerun_ui_kit 0.38.1 as crates/naves-ui"
```

---

### Task 3: Re-port `naves-ui` to upstream `re_ui` 0.42.0 (egui 0.37)

**Files:**
- Modify: `crates/naves-ui/**` (three-way merge)
- Modify: `crates/naves-ui/Cargo.toml` (dependency versions)
- Modify: `crates/naves-ui/PORTING.md` (new pin, per its own §1)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: Task 2's vendored base (upstream pin 0.38.1) and the upstream rerun repo at tags 0.38.1 and 0.42.0.
- Produces: `naves_ui` with the `re_ui` 0.42.0 API on egui 0.37 — the exact API Tasks 4–7 code against (read the crate source at execution time; upstream renames are possible).

- [ ] **Step 1: Fetch the upstream sources**

```bash
git clone --filter=blob:none --no-checkout https://github.com/rerun-io/rerun /tmp/opencode/rerun
cd /tmp/opencode/rerun
git sparse-checkout set crates/viewer_support/re_ui crates/utils/re_format crates/utils/re_error crates/utils/re_span
```

- [ ] **Step 2: Measure the delta**

From the rerun clone: `git diff --stat 0.38.1 0.42.0 -- crates/viewer_support/re_ui crates/utils/re_format crates/utils/re_error crates/utils/re_span`. Record the file list and rough size in PORTING.md's §1 (the ledger). If a file's diff is mostly upstream-internal churn that our surgery already removed, note it and skip it — the ledger must say what was applied and what was deliberately not.

- [ ] **Step 3: Apply the merge**

Per PORTING.md's strategy ("verbatim vendoring + minimal surgery; the diff against upstream IS the port ledger"): for each upstream-changed file, bring our copy to the 0.42.0 content, then re-apply the surgery rules R1–R7 (path substitutions, cfg/feature deletions, `profile_*!` deletions, lint-attribute downgrades, doctest rename to `naves_ui::`) to the newly-arrived lines. Keep the shim modules (`format/`, `error.rs`, `span.rs`, `time_types.rs`, `ids.rs`, `async_runtime.rs`, `notifications.rs`) in their §5 shape — copy wholesale where §5.1 says verbatim, re-minimize where §5.3–5.5 say minimized, using the 0.42.0 source as the new base.

- [ ] **Step 4: Update the dependency versions**

`crates/naves-ui/Cargo.toml`, to match upstream 0.42.0's resolved set: `egui = "0.37"` (features `color-hex`), `egui_commonmark = "0.26"`, `egui_extras = "0.37"` (`image`, `svg`), `itertools = "0.15"`, `jiff = "0.2"`, `ron = "0.12"` (`integer128`), `parking_lot = "0.12"`, `smallvec = "1.15"` (`const_generics`, `union`), `strum = "0.26"` (`derive`), `num-traits = "0.2"`, `nucleo-matcher = "0.3.1"` (`unicode-casefold`, `unicode-normalization` — without these it fails to compile), `crossbeam = "0.8"`, `log`, `serde`, `anyhow`, `url`, `web-time`. Dev-deps: `eframe = "0.37"`, `egui_kittest = "0.37"` (`snapshot`, `eframe`), `egui_tiles = "0.18"`. Remove any dependency the 0.42.0 merge no longer needs.

- [ ] **Step 5: Stabilize on stable 1.99**

Run: `cargo test -p naves-ui --locked`.
Expected: compile errors first — fix nightly-isms (like the `span.rs` `core::range::Range` impl deleted in Task 2's base; if 0.42.0's `re_span` still carries it, strip it again and record the strip in PORTING.md), then all tests pass. The 59-test suite is the gate for merge correctness; if upstream renamed a tested API, port the test with it.

- [ ] **Step 6: Verify single-egui and the workspace gates**

Run: `cargo tree --workspace --prefix none | grep -E "^egui v" | sort -u` — expect exactly `egui v0.37.x` (one line). Run `make check`.
Expected: green; `make deny` no longer warns about duplicate egui.

- [ ] **Step 7: Changelog + commit**

Add under `## [Unreleased]`: `- Build: re-port naves-ui to Rerun's re_ui 0.42.0 on egui 0.37.`

```bash
git add crates/naves-ui CHANGELOG.md
git commit -m "build: re-port naves-ui to re_ui 0.42.0 (egui 0.37)"
```

---

### Task 4: Theme the GUI with the naves-ui design system

**UI task — invoke the frontend-design skill before starting.**

**Files:**
- Modify: `crates/naves/src/gui/theme.rs` (rewritten as a thin adapter)
- Modify: `crates/naves/src/gui/mod.rs:1114` area (the `apply_theme` call — unchanged signature), plus the usage sites of the removed constants (8 files, 13 sites: `dock.rs`, `levels/mod.rs`, `levels/terrain.rs`, `mod.rs`, `prefs_window.rs`, `rail.rs`, `stack.rs`, `tests/levels.rs`)
- Modify: `crates/naves/Cargo.toml` (add `naves-ui`)
- Test: `crates/naves/src/gui/tests/` (new theme tests + adapted `tests/levels.rs`)
- Modify: `docs/manual/src/gui.md` + `docs/manual/src/images/` (new screenshots)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: `naves_ui::apply_style_and_install_loaders(&egui::Context)` and `naves_ui::design_tokens()`.
- Produces: `theme::apply_theme(ctx: &egui::Context)` with the same signature (the `mod.rs:1114` call site stays untouched), and constants `GREEN`, `ERROR` (kept, they are our accent/error colours on top of the token system). `INK`, `GROUND`, `PANEL`, `SELECTED`, `EDGE` are removed — fills, text and selection come from the kit's tokens. `contrast(a, b)` stays.

- [ ] **Step 1: Write the failing tests**

New kittest tests (in the existing suite's style): (a) after `apply_theme`, the context's font definitions contain the kit's Inter family (Review Focus #2); (b) `contrast(GREEN, dark panel fill) >= 3.0` and `contrast(ERROR, dark panel fill) >= 3.0` using the dark token set (Review Focus #1); (c) the existing `tests/levels.rs` sites that reference removed constants are rewritten against token values (e.g. selected-row colour read from `design_tokens()`).

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p naves gui::tests --locked`
Expected: FAIL — fonts not installed, dark tokens not reachable, constants still the old ones.

- [ ] **Step 3: Implement the adapter**

`apply_theme` becomes: `naves_ui::apply_style_and_install_loaders(ctx)` (installs the light/dark token themes and Inter), nothing else — the kit owns fills, text, selection, spacing, fonts. Keep `GREEN`/`ERROR` as the manual-palette constants they are; delete the other five and fix the 13 usage sites (each site either drops the colour — the token system now decides — or switches to `GREEN`/`ERROR` where it painted an accent or an error). Add `naves-ui` to `crates/naves/Cargo.toml`. The pale-green manual look is superseded by the kit's theme in this milestone — the manual's palette survives in the accents and in the map rendering, which is unchanged.

- [ ] **Step 4: Run the tests and the whole suite**

Run: `cargo test -p naves gui::tests --locked`, then `make test`.
Expected: PASS — the 3,900 lines of existing GUI tests must stay green; if a test asserted an old colour value, rewrite it against the token it now comes from.

- [ ] **Step 5: Screenshots and manual**

Re-shoot the `docs/manual/src/images/` GUI screenshots against the new look; update `gui.md` where it describes colours or the look.

- [ ] **Step 6: Verify, changelog, commit**

Run: `make check`.
Add under `## [Unreleased]`: `- The GUI now uses Rerun's design system (Inter font, new light/dark theme, modern widgets) — Changed, look and feel.`

```bash
git add crates/naves docs/manual CHANGELOG.md
git commit -m "feat: theme the GUI with the naves-ui design system"
```

---

### Task 5: Adopt `list_item` for the palette

**UI task — invoke the frontend-design skill before starting.**

**Files:**
- Modify: `crates/naves/src/gui/palette.rs` (252 lines — row rendering)
- Test: `crates/naves/src/gui/tests/` (adapt palette/selection tests)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: `naves_ui::list_item::ListItem` (read the crate source for the 0.42.0 API — `list_item::list_item_scope`, `ListItem::new`, `.selected(bool)`, `.show_flat()` or the current equivalents).
- Produces: the palette's rows rendered by the kit's `list_item` (hover, selection and keyboard focus states for free); the search filter and click-to-pick behaviour unchanged.

- [ ] **Step 1: Write/adjust the failing tests**

The existing palette/selection kittest tests keep their assertions (search filters, click picks the tool, keyboard selection). Add one asserting the kit's hover state: hovering a row changes its visuals (the kit's `list_item` provides this; the test pins that we did not disable it).

- [ ] **Step 2: Verify they fail**

Run: `cargo test -p naves gui::tests --locked`
Expected: FAIL (palette still hand-renders rows).

- [ ] **Step 3: Implement**

Replace `palette.rs`'s hand-rolled row painting with `list_item` calls inside a `list_item_scope`; keep the fuzzy/search filtering, the `PaletteItem` model and the click/keyboard wiring exactly as they are.

- [ ] **Step 4: Verify, changelog, commit**

Run: `cargo test -p naves gui::tests --locked && make check`.
Expected: PASS. Changelog: `- The palette uses the design system's list items — Changed, look and feel.`

```bash
git add crates/naves/src/gui/palette.rs crates/naves/src/gui/tests CHANGELOG.md
git commit -m "feat: render the palette with naves-ui list items"
```

---

### Task 6: The preferences window becomes a modal

**UI task — invoke the frontend-design skill before starting.**

**Files:**
- Modify: `crates/naves/src/gui/prefs_window.rs` (529 lines — currently an `egui::Window`)
- Modify: `crates/naves/src/gui/mod.rs` (the `prefs_window: Option<PrefsWindow>` field's usage — open/close flow)
- Test: `crates/naves/src/gui/tests/prefs_window.rs` (703 lines — adapt)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: `naves_ui::modal::{ModalHandler, ModalWrapper}` (read the crate source for the 0.42.0 API).
- Produces: `prefs_window::PrefsWindow` with the same public surface the app state uses (open, close, the settings it edits); opening it shows a centered modal instead of a floating window.

- [ ] **Step 1: Write the failing tests**

Adapt the existing `tests/prefs_window.rs` to the modal flow, and add the modal-blocking test (Review Focus #3/#4): drag on the map while the modal is open → the view must not pan; Escape closes the modal and returns focus.

- [ ] **Step 2: Verify they fail**

Run: `cargo test -p naves gui::tests::prefs_window --locked`
Expected: FAIL (still a floating `egui::Window`).

- [ ] **Step 3: Implement**

Swap `egui::Window` for `ModalHandler` + `ModalWrapper` per the kit's documented modal pattern (the usage example is in the crate README and example gallery). The settings forms inside keep their logic; only the chrome changes. Escape-to-close comes from the kit's modal.

- [ ] **Step 4: Verify, changelog, commit**

Run: `cargo test -p naves gui::tests --locked && make check`.
Expected: PASS. Changelog: `- The settings window is now a modal dialog — Changed, look and feel.`

```bash
git add crates/naves/src/gui/prefs_window.rs crates/naves/src/gui/mod.rs crates/naves/src/gui/tests CHANGELOG.md
git commit -m "feat: show the settings window as a naves-ui modal"
```

---

### Task 7: A command palette (Ctrl+P)

**UI task — invoke the frontend-design skill before starting.**

**Files:**
- Create: `crates/naves/src/gui/command_palette.rs`
- Modify: `crates/naves/src/gui/mod.rs` (module decl, state field, the update loop's palette handling)
- Modify: `crates/naves/src/gui/shortcuts.rs` (a `CommandPalette` action bound through `Bindings`, default Ctrl+P)
- Test: `crates/naves/src/gui/tests/command_palette.rs` (new)
- Modify: `docs/manual/src/shortcuts.md`, `CHANGELOG.md`

**Interfaces:**
- Consumes: `naves_ui::command_palette` (read the crate source for the 0.42.0 API) and `shortcuts::Bindings` (the existing user-configurable binding machinery).
- Produces: `command_palette::CommandPalette` — a stateful handler owned by `NavesApp`, opened by the `CommandPalette` shortcut. Entries: the tools (`shortcuts::ToolId`), the levels (0–9), and the Build command. Selecting an entry performs exactly what the existing shortcut for that action does (no new code paths for the actions themselves).

- [ ] **Step 1: Write the failing tests**

New kittest tests: (a) opening via the configured binding (Review Focus #3 — configure a non-default binding in the test and open with it); (b) typing filters entries (fuzzy); (c) selecting the Build entry starts a build (assert through the existing build-test seam); (d) Escape closes it and no tool changed.

- [ ] **Step 2: Verify they fail**

Run: `cargo test -p naves gui::tests::command_palette --locked`
Expected: FAIL (module does not exist).

- [ ] **Step 3: Implement**

Follow the kit's command-palette pattern (crate example shows the handler wiring). Register entries with their icons (`naves_ui::icons`), labels and callbacks; each callback calls the same methods the equivalent shortcut calls today. Add the `CommandPalette` action to `shortcuts.rs` with default Ctrl+P and the usual parse/serialize support.

- [ ] **Step 4: Verify, changelog, commit**

Run: `cargo test -p naves gui::tests --locked && make check`.
Expected: PASS. Update `docs/manual/src/shortcuts.md` with the new binding. Changelog: `- A command palette (Ctrl+P) can open tools, levels and the build — Added.`

```bash
git add crates/naves/src/gui docs/manual/src/shortcuts.md CHANGELOG.md
git commit -m "feat: add a command palette for tools, levels and build"
```

---

## Follow-up (not in this plan)

The full design pass — every panel audited against the kit's widget set, the manual's palette revisited on the token system, dark-mode polish — is a separate plan once this milestone is on screen. The map viewport stays on its own rendering path in all of it.

# Porting Guide

How `naves_ui` is produced from upstream [`re_ui`](https://github.com/rerun-io/rerun/tree/main/crates/viewer_support/re_ui),
and how to re-port when a new upstream version is needed.

## 1. Port snapshot

| | |
|---|---|
| Upstream repo | `rerun-io/rerun` |
| Upstream pin | tag `0.38.1` (previous port: `0.30.2`) |
| Primary source | `crates/viewer_support/re_ui` (moved from `crates/viewer/re_ui` in 0.35–0.38) |
| Secondary sources | `crates/utils/re_format` (→ `src/format/`), `crates/utils/re_error` (→ `src/error.rs`), `crates/utils/re_span` (→ `src/span.rs`), `re_log_types`/`re_types_core` time types (→ `src/time_types.rs`), `re_log_types` ids + `re_uri` (→ `src/ids.rs`), `re_async` (→ `src/async_runtime.rs`), `re_log::LogMsg`/`FieldValue` (→ `src/notifications.rs`) |
| egui alignment | egui **0.36** (matches `bevy_egui` 0.42 for bevy 0.19) |

**Strategy: verbatim vendoring + minimal surgery.** Files are copied
file-for-file with identical names and module structure. The transformations
below are the only edit classes applied; **the diff against upstream *is* the
port ledger** — keep it small and boring.

The pinned upstream version is the *base* of the three-way merge on every
re-port: `upstream old-tag → new-tag` tells you what upstream changed; this
document tells you what we change. Without the pin, the ledger is unreadable.

## 2. File mapping

| This crate | Upstream |
|---|---|
| `src/*.rs`, `src/egui_ext/`, `src/list_item/`, `src/command/`, `src/re_form/` | `crates/viewer_support/re_ui/src/…` (1:1) |
| `src/format/{mod,duration,plural,time}.rs` | `crates/utils/re_format/src/{lib,duration,plural,time}.rs` |
| `src/error.rs` | `crates/utils/re_error/src/structured_error.rs` (verbatim) |
| `src/span.rs` | `crates/utils/re_span/src/lib.rs` (verbatim) |
| `src/time_types.rs` | hand-minimized port of `re_log_types` + `re_types_core::datatypes` |
| `src/ids.rs` | hand-minimized port of `re_log_types::{StoreId, EntryId}` + `re_uri::{Origin, Scheme}` |
| `src/async_runtime.rs` | hand-minimized port of `re_async::{AsyncRuntimeHandle, WasmNotSend}` |
| `data/` | `crates/viewer_support/re_ui/data/` verbatim, **except** `logo_*.png` and `icons/rerun_logo.png` / `icons/rerun_wordmark.svg` (rerun brand assets) |
| `examples/naves_ui_example/` | `crates/viewer_support/re_ui/examples/re_ui_example/` (adapted, see §7) |

## 3. Dependency substitution table

| Upstream | Replacement | How |
|---|---|---|
| `re_log` | `log` | `re_log::info!` → `log::info!` etc.; `re_log::Level::WARN` (tracing-style consts, upstream moved to tracing) → `log::Level::Warn`; `re_log::debug_warn_once!` → `Once`-guarded `log::warn!` (1 site: `list_item/scope.rs`) |
| `re_log::LogMsg`, `FieldValue` | `src/notifications.rs` | local ports (struct + enum, verbatim shape) |
| `re_mutex` | `parking_lot` | path substitution |
| `re_tracing` | — | delete every `profile_function!()` / `profile_scope!(…)` line |
| `re_format` | `src/format/` | vendored near-verbatim |
| `re_error` | `src/error.rs` | vendored verbatim (`StructuredError`, `format_with_details`) |
| `re_span` | `src/span.rs` | vendored verbatim (`Span<usize>` for `card_layout`) |
| `re_log_types` / `re_types_core` | `src/time_types.rs` | minimized rewrite (§5.3) |
| `re_log_types::{StoreId,EntryId}`, `re_uri::Origin` | `src/ids.rs` | minimized rewrite (§5.4) |
| `re_async` | `src/async_runtime.rs` | `AsyncRuntimeHandle` = spawn-closure adapter (§5.5) |
| `re_quota_channel` | `crossbeam` | `create_crossbeam_channel(1)` → `bounded(1)`, `send_crossbeam` → `try_send` (`requested_object.rs`) |
| `re_entity_db`, `re_log_types` (paths) | — | deleted: `syntax_highlighting.rs` impls for rerun data types (§6.2) |
| `re_analytics` | — | deleted: feature + `record(...)` call site in `ui_ext.rs` |
| `eframe` (lib) | `egui` | path substitution `eframe::emath/epaint` → `egui::…`; one API break (§6.1) |
| `getrandom` | — | `filter_widget.rs` session id: time + atomic counter |
| `ahash` | `std` | `ahash::HashMap` → `std::collections::HashMap` (`filter_widget.rs`, `egui_ext/garbage_collect.rs`) |
| `sublime_fuzzy` | `nucleo-matcher` | upstream switched; we follow (public crate, features `unicode-casefold`, `unicode-normalization` required — without them the crate fails to compile) |
| `notify` | — | deleted `#[cfg(hot_reload_design_tokens)]` branches |
| `raw-window-handle` | — | deleted with traffic-light measuring (§6.1) |
| `wayland-client`, `wayland-protocols`, `thiserror` | — | deleted with `wayland.rs` (§6.3) |
| `static_assertions` (dev) | — | dropped one meta-test in `requested_object.rs` |
| `rand` (dev) | — | example: `rand::random()` → atomic counter |

Kept as-is (public crates): `anyhow`, `crossbeam`, `egui`, `egui_commonmark`,
`egui_extras`, `itertools`, `jiff`, `log`, `nucleo-matcher`, `num-traits`,
`parking_lot`, `ron`, `serde`, `smallvec`, `strum`, `strum_macros`, `url`,
`web-time`.

## 4. Mechanical transformation rules

- **R1 — path substitution** (per §3): `re_log::` → `log::`,
  `re_format::` → `crate::format::`, `re_error::` → `crate::error::`,
  `re_span::` → `crate::span::`, `re_log_types::…` → `crate::time_types::…` or
  `crate::ids::…` (context-dependent), `re_uri::` → `crate::ids::`,
  `eframe::emath/epaint` → `egui::…`.
- **R2 — delete cfg/feature blocks**: `#[cfg(feature = "analytics")]`,
  `#[cfg(feature = "testing")] pub mod testing`, all
  `#[cfg(hot_reload_design_tokens)]` branches (keep the `OnceLock` path),
  `#[cfg(target_os = "macos")]` traffic-light code, `mod wayland`.
- **R3 — delete single lines**: `re_tracing::profile_*!();`,
  `egui_ctx.include_bytes("bytes://logo_…", …)`.
- **R4 — lint attribute downgrade**: `#[expect(clippy::…)]` →
  `#[allow(clippy::…)]`, including inner attributes `#![expect(…)]` (upstream's
  workspace clippy config is not vendored). Add
  `#[allow(clippy::too_many_arguments)]` where upstream's higher threshold
  hid the lint (`relative_time_range.rs::edit_boundary_ui`).
- **R5 — doctest rename**: `re_ui::…` → `naves_ui::…`.
- **R6 — remove identity conversions**: our simplified `TimeInt` makes some
  upstream `.into()` calls identity conversions (clippy `useless_conversion`);
  delete them (`relative_time_range.rs`).
- **R7 — explicit f32 literals**: `1.0` → `1.0_f32` if the toolchain's
  `float_literal_f32_fallback` lint fires.

## 5. Shim modules

### 5.1 `src/format/`, `src/error.rs`, `src/span.rs` — vendored verbatim

Treat as vendored code: re-copy wholesale on re-port, strip only what fails to
compile or is unused (`format_f16`, …).

### 5.2 `src/notifications.rs` — `LogMsg` + `FieldValue` ports

Local copies of `re_log::{LogMsg, FieldValue}` (rerun 0.38.1 shape: `level`,
`target`, `message`, `fields`). `pub use log::Level` stands in for
`re_log::Level` (which is tracing-based upstream).

### 5.3 `src/time_types.rs` — minimized rewrite

`TimeInt` is a plain `i64` newtype (`MIN = i64::MIN + 1`, no `STATIC`
sentinel); arrow/serde machinery removed. `Timestamp::now()` uses `web_time`.
0.38.1 note: upstream replaced `hide_today_date: bool` with the
`DateVisibility` enum — **ported** (enum + `with_date_visibility` +
format/parse logic). Consumers: `time.rs`, `time_drag_value.rs`,
`relative_time_range.rs`, `notifications.rs`.

### 5.4 `src/ids.rs` — `StoreId` / `EntryId` / `Origin`

Opaque identifiers for the `command` module; only `Clone + Debug + Eq + Hash +
Display` are exercised by re_ui. `ApplicationId`/`RecordingId` are `String`
newtypes; `EntryId` is a `u128` from a time/counter mix (upstream wraps
`re_tuid::Tuid`); `Origin` keeps the upstream shape (`Scheme` + `url::Host` +
`u16`).

### 5.5 `src/async_runtime.rs` — `AsyncRuntimeHandle` / `WasmNotSend`

`WasmNotSend`: cfg-gated trait alias (`Send` on native, empty on wasm), same
pattern as `re_async`. `AsyncRuntimeHandle`: a spawn-closure adapter —
consumers supply their runtime (tokio `Handle`, bevy `TaskPool`, …) via
`AsyncRuntimeHandle::new`, or use `AsyncRuntimeHandle::new_thread_per_task()`
(std-only `block_on`, native only). No tokio/wasm-bindgen dependency.

## 6. Intentional deviations (cannot be re-applied mechanically)

### 6.1 Break #1 — `top_bar_style` drops `eframe::Frame` (macOS traffic lights)

Upstream measures the native macOS traffic-light button area
(`eframe::Frame.window_handle()` → `raw_window_handle` →
`eframe::WindowChromeMetrics`; upstream moved the measuring code into eframe
itself at 0.33) so the top bar can share space with it in full-size windows.
The port cuts the chain at the entry point:

```rust
fn top_bar_style(&self, _style_like_web: bool) -> TopBarStyle // fixed height, indent: 0.0
```

This removes the crate's only `unsafe` code (`#![forbid(unsafe_code)]` stays)
and the `raw-window-handle` dependency. Cost shifted to consumers wanting
rerun-style full-size chrome on macOS (reserve ~78pt left indent themselves).

### 6.2 Break #2 — syntax highlighting for rerun data types removed

All `SyntaxHighlighting` impls for `EntityPath(Part)`, `Instance(Path)`,
`ComponentPath`, `ComponentDescriptor`, `ComponentIdentifier`, `ComponentType`,
`ArchetypeName`, and `InstanceInBrackets` deleted from
`syntax_highlighting.rs` (orphan impls on rerun's data model). Generic builder
and primitive impls kept.

### 6.3 Other behavioral deltas

- `wayland.rs` deleted; `custom_window_decorations_default()` returns `true`
  on Linux without probing the compositor (noted in the doc comment).
- `filter_widget.rs`: session id from `web_time` nanos ⊕ atomic counter.
- Brand assets (`logo_*.png`, `rerun_logo.png`, `rerun_wordmark.svg`) and
  their `Icon` constants / `include_bytes` registrations removed.
- Local feature on top of the port (commit `9a55e83`, re-applied each port):
  `modal.rs` slim title bar, non-selectable title. **Not** an upstream
  mapping — do not "fix" it back.
- Example: `re_log` logger → local `ChannelLogger` over `log`;
  `re_quota_channel` → `crossbeam` `try_send`; `rand` → atomic counter.

## 7. Tests

- `egui_kittest` snapshot tests are kept but marked
  `#[ignore = "snapshot reference images were not vendored from the rerun repo"]`
  (`alert.rs`, `combo_item.rs`, `relative_time_range.rs`, `tab_bar.rs` ×2).
- Upstream `src/testing.rs` (feature `testing`) is dropped; the one consumer
  (`combo_item.rs` snapshot test) inlines
  `SnapshotOptions::new().threshold(1.0).max_failed_pixels(10)`.

## 8. Re-port procedure (next upstream version)

1. Fetch the new upstream tree: `crates/viewer_support/re_ui/{src,data,examples}`
   plus `crates/utils/re_format/src`, `re_error/src/structured_error.rs`,
   `re_span/src/lib.rs` at the target tag. (Check whether `re_ui` moved again.)
2. Diff upstream old-pin → new-pin first; files unchanged upstream need no work.
3. `rsync -a --delete` upstream `src/` over ours, **excluding** `format/` and
   `time_types.rs`; same for `data/`, excluding brand assets. Delete
   `testing.rs`, `wayland.rs` if re-copied.
4. Re-apply rules **R1–R7** (§4). A global sed pass covers most of R1/R3/R4/R5.
5. Hand-check the touch-points: `context_ext.rs` (§6.1), `syntax_highlighting.rs`
   (§6.2), `lib.rs` (logos / hot-reload / module list), `icons.rs` (brand
   constants), `notifications.rs` (LogMsg/FieldValue/Level), `filter_widget.rs`,
   `requested_object.rs`, `command/*` (ids), `modal.rs` (re-apply local feature).
6. Re-sync `src/format/`, `src/error.rs`, `src/span.rs` wholesale.
7. Check the shims against upstream API drift: `time_types.rs` (§5.3),
   `ids.rs` (§5.4), `async_runtime.rs` (§5.5).
8. Update the version pin in `Cargo.toml` (`description`), `README.md`, `lib.rs`
   doc header and this file. Bump egui/eframe/egui_extras/egui_kittest/
   egui_tiles/egui_commonmark/nucleo-matcher to the versions upstream's
   workspace uses at that tag.
9. Verify: `cargo test`, `cargo build --example naves_ui_example`,
   `cargo check --target wasm32-unknown-unknown`, `cargo clippy --all-targets`.

## 9. License & attribution

Same license as upstream: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).
The bundled Inter font is OFL-1.1 (`data/OFL.txt`). Every shim/ported file
carries a header comment citing its upstream crate, the pinned version, and the
license — keep these headers accurate when bumping the pin.

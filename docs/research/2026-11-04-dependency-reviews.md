<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Dependency reviews

`AGENTS.md` requires a crate search and a review before a crate is used. This file records the result for
the dependencies of milestone 1, step 1. Commits that add a crate later put their review in the commit
message and may add a row here. Checks marked *scan* are `grep` for `unsafe`, build scripts and
panicking calls plus the crates.io metadata; *read* means the relevant code was read.

## Crates considered for the DEM reader

| Candidate | Result |
|---|---|
| `geotiff-reader` 0.11 (+ `geotiff-core`, `tiff-reader`, `tiff-core`, `ndarray`) | **used** |
| `tiff` 0.11 (image-rs) | rejected for the reader: it is only a TIFF decoder, so GeoKey and tie-point parsing would be ours (it stays as a dev-dependency to build synthetic test tiles) |
| `gdal` | rejected: needs the GDAL C library, which breaks the "builds on Windows without extras" rule |
| `wbraster` 0.2 | rejected: a whole raster-GIS toolbox, far more than needed |
| `srtm`, `srtm_reader` | rejected: `.hgt` files only, no GeoTIFF |
| `copernicus_explorer` 0.5 | not applicable: searches and downloads Sentinel products from CDSE, no DEM tiles. May be worth a look for the satellite imagery plan |

**`geotiff-reader`:** pure Rust, no build scripts, MIT OR Apache-2.0, needs Rust 1.88. `unsafe` exists only in the opt-in
mmap path (`open_mmap`), which Náves never calls (it uses `from_bytes` after reading at most 512 MiB). *Read:* the
remaining `unwrap` calls in non-test code are slice conversions guarded by an `ensure(n)?` bounds check. A 90 s
`cargo fuzz` run on our `dem_tile` target (37 k runs) found no crash. **Weak points:** the crate family is young
(first release March 2026, about 10 k downloads in the last 90 days, one organisation behind it) and pulls in
`ndarray` and `memmap2`. Mitigations: our own size and dimension limits before decoding, the fuzz target and its
seeds in `make test`/`make fuzz`, and 13 behaviour tests on real Copernicus tiles. If the crate turns out to be
unmaintained, swap back to the `tiff` crate; only `DemTile::from_reader` changes.

## Other crates

| Crate | Purpose | Alternatives considered | Review |
|---|---|---|---|
| `proj4rs` 0.3 | UTM conversion | `proj` (needs libproj, C), `utm` crates (only UTM, little maintained) | *scan:* no build script; `unsafe` in the NAD-grid catalogue (unused) and one `unwrap_unchecked` on a fixed-size array in the transverse-Mercator series (sound); actively released (Oct 2026), about 400 k recent downloads |
| `unicode-normalization` 0.1 | accent folding for `world_ident` | `deunicode`, `slug` (transliterate instead of decompose, change `ß`/`ø` handling and use `-`) | *scan:* `unsafe` only for `char::from_u32_unchecked` on values it just computed; part of the unicode-rs family, about 139 M recent downloads |
| `rayon` | row-parallel sampling | none needed | *scan:* heavy `unsafe` inside the scheduler, very widely audited (about 160 M recent downloads) |
| `serde`, `toml` | project file | `serde` + `toml` is the standard pairing | *scan:* `serde`'s build script only probes the compiler; `toml` has no `unsafe` |
| `thiserror`, `anyhow` | error types | `snafu`, `eyre` | *scan:* build scripts only probe the compiler |
| `tempfile` | atomic writes, tests | `atomic-write-file` (smaller, less used) | *scan:* three small `unsafe` blocks around OS calls; the de-facto standard |
| `clap`, `clap_mangen` | CLI, man page | `argh`, `pico-args` | *scan:* no `unsafe`, no build script |
| `image` 0.25 (feature `png` only, pulls `png`) | PNG satellite and mask images (and later the map render) | `png` directly (lower level; the render code later needs `RgbaImage`), `tiny-skia` (a 2D renderer) | *scan:* image-rs project, about 75 M recent downloads, updated March 2026, `png` about 107 M; pure Rust decoder/encoder, we only encode our own images in memory, so untrusted-input exposure is nil in this plan |
| `assert_cmd`, `libfuzzer-sys` | dev: CLI tests, fuzzing | none | *scan:* standard tools of their kind |

`cargo deny check` (RustSec advisories, licences, sources) is clean for all of them. The hand-written code that
remains has no crate that fits: the 8WVR reader and writer (`naves-formats`; a private binary layout), the
multi-tile bilinear height sampler and the heightmap grid (an `ndarray` grid would add a dependency for
`Vec<f32>` plus an index).

## Added with the world config and mission (Task 8)

| Crate | Purpose | Alternatives considered | Review |
|---|---|---|---|
| `quick-xml` 0.42 | edit three mission XML files (drop elements, replace spawn positions) | `roxmltree` (read-only), `xml-rs`/`xmltree` (older, slower), plain string edits (fragile on real files) | *scan:* `#![forbid(unsafe_code)]`, no build script, MIT, about 163 M recent downloads, updated Aug 2026; malformed input returns an error (tested) |
| `walkdir` 2.5 | copy the mission folder in a stable order | `std::fs::read_dir` recursion (more code for symlink and loop handling) | *scan:* no `unsafe`, no build script, Unlicense OR MIT (added `Unlicense` to the `deny.toml` allow-list: a public-domain dedication, AGPL-compatible), about 193 M recent downloads; last release March 2024, feature-complete |

## Added with the Proton bridge (Task 9)

| Crate | Purpose | Alternatives considered | Review |
|---|---|---|---|
| `wait-timeout` 0.2 | stop a child process after a time limit | `std` only (needs a polling loop with sleeps), `command-group`, `duct`, `subprocess` (bigger, more than needed) | *scan:* tiny, already in the tree through `assert_cmd`; one maintainer (alexcrichton, rust-lang), last release Feb 2025, MIT/Apache-2.0; its `unsafe` is the libc/Windows wait calls |
| `which` 8 | find `armake2` on `PATH` | hand-written `PATH` search (executable bit, `PATHEXT` on Windows) | *scan:* about 80 M recent downloads, updated Aug 2026, MIT; no build script |

## Added with the GUI (Task 11)

| Crate | Purpose | Alternatives considered | Review |
|---|---|---|---|
| `eframe` / `egui` 0.37 (features `glow`, `default_fonts`, `wayland`, `x11`) | the editor window | `iced`, `slint` (own toolkits, larger), `gtk4`/`qt` (system libraries), a web UI | *scan:* emilk/egui, the most used Rust immediate-mode GUI, requires Rust 1.95 like our toolchain, released this season; pure Rust except the platform windowing (`winit`, `glutin`) which load system libraries at run time. The `wgpu` backend is left out to keep the tree small. Licence: its embedded fonts (`epaint_default_fonts`: OFL-1.1 and Ubuntu-font-1.0) are covered by a **scoped** `deny.toml` exception, not a general allowance |
| `egui_kittest` 0.37 (dev) | widget tests through the accessibility tree | screenshot tests (need a GPU), manual testing | same project as egui; found missing accessible names in our form on its first run |

## Added with the MCP server (Task 12)

| Crate | Purpose | Alternatives considered | Review |
|---|---|---|---|
| `rmcp` 3.8 (features `server`, `macros`, `transport-io`; dev: `client`, `transport-child-process`) | MCP server over stdio | `rust-mcp-sdk`, `mcpr` and other third-party SDKs; writing JSON-RPC by hand | *scan:* the official SDK (`modelcontextprotocol/rust-sdk`), Apache-2.0, about 26 M recent downloads, updated Oct 2026, needs Rust 1.88; HTTP/OAuth features stay off, so there is no network code. Logging notifications are deprecated in 3.x, so `build` returns its progress lines in the result |
| `tokio` (`rt-multi-thread`, `macros`, `io-std`) | the async runtime that rmcp needs | `async-std`, `smol` (not supported by rmcp) | the de-facto runtime; already in the tree through rmcp |
| `base64` | image content of `render_topdown` | hand-written encoder | maintained by the rust-lang-adjacent `marshallpierce` project; no `unsafe` needed by us |
| `serde_json` | `project_set` values | none | the standard JSON crate |

## Added with imagery fetching (satellite imagery plan, Task 7)

| Crate | Purpose | Alternatives considered | Review |
|---|---|---|---|
| `ureq` 3.5 (`default-features = false`, `rustls`) | fetch imagery: WMS images, STAC search, scene files | `reqwest` (blocking; much larger tree with `hyper` and `tokio`, which the library crates do not otherwise need), `attohttpc` (smaller community, fewer releases), shelling out to `curl` (external tool, no size limits per request) | *source read:* `#![forbid(unsafe_code)]`, no build script, MIT OR Apache-2.0, blocking I/O so no async runtime in `naves-export`; a `Body` is read through `with_config().limit(n)`, so the response size can be capped while reading; `https_only`, redirect limit and a global timeout are configuration; proxy settings come from the environment (the only thing a user may want). *advisories:* none in RustSec for ureq 3.5, `rustls` 0.23 or `ring` (`cargo deny`). *maintenance:* algesten/ureq, one active maintainer plus contributors, releases through 2026. *tree:* `rustls`, `ring`, `webpki-roots`, `ureq-proto`, `base64`, `log`, `percent-encoding`, `utf8-zero`, `rustls-pki-types`. *Licence:* all on the allow-list except `webpki-roots` (see below) |
| `webpki-roots` 1.0 (through `ureq`) | the Mozilla CA list that TLS trusts | `rustls-platform-verifier` (system store; still pulls `webpki-root-certs`, same licence), `native-tls` (system OpenSSL: needs `libssl-dev` to build, a runtime dependency on `libssl3`) | Licence **CDLA-Permissive-2.0**, a permissive data licence that allows using and redistributing the list in a program; allowed for this crate only, as a scoped `[[licenses.exceptions]]` entry in `deny.toml` (the maintainer decided so on 2026-11-06). **A shipped CA list can go stale or contain a revoked CA**, so `make ca-bundle` fails when `Cargo.lock` holds an older `webpki-roots` than the newest release; CI runs it on every push, every week and before every release |
| `tiff` 0.11 (feature `deflate`; was a dev-dependency) | write the fetched imagery as GeoTIFF | `geotiff-writer` and similar (few users, young), `gdal` bindings (system library) | image-rs project, MIT OR Apache-2.0, no build script; its decoder is not used here, only the encoder, which writes to a buffer we own; already reviewed as a dev-dependency |
| `image` feature `jpeg` (`zune-jpeg` 0.5) | decode the JPEG that EOxCloudless answers even when PNG is asked for | `jpeg-decoder` (older), `mozjpeg` (C library) | `zune-jpeg` is pure Rust, MIT OR Apache-2.0 OR Zlib, the JPEG decoder of `image` 0.25. It contains `unsafe` for SIMD paths (guarded by runtime CPU detection); the responses are untrusted, so images are decoded with `image`'s `Limits` (side and allocation caps) and the decoded size must equal the requested size |

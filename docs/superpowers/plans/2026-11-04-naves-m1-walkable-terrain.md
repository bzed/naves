<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Náves M1, plan 1: walkable terrain end to end — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `naves new` + `naves build` turn a real area (Copernicus DEM) into a signed DayZ terrain mod that a local 1.30 server loads and a player can walk on; a minimal GUI shows the project and an MCP server can read it, render it top-down and rebuild it.

**Architecture:** A Cargo workspace with four crates: `naves-formats` (pure file formats), `naves-core` (project, DEM, projection, heightmap), `naves-export` (terrain layers, config, mission, Proton binarize bridge, packing) and `naves` (binary: CLI, GUI, MCP). Generation is a straight pipeline in this plan; the cached layer graph, the object data model and the undo history come with the plans that first need them (see "Later plans").

**Tech Stack:** Rust 1.95 (edition 2024), `tiff`, `proj4rs`, `image`, `serde` + `toml`, `clap`, `thiserror`/`anyhow`, `eframe`/`egui`, `rmcp`; external tools: bzed's armake2 fork (`armake2` in `PATH`), DayZ Tools `binarize.exe` under Proton, GDAL CLI (tests' fixture creation only).

**Spec:** `docs/superpowers/specs/2026-11-04-naves-design.md`. Background: `docs/research/2026-11-03-dayz-osm-terrain-research.md`.

This plan is milestone 1, internal step 1 of the design (section 8): "binarize bridge check with a heightmap-only world: walkable map", plus the first slices of the GUI (5.9) and MCP (5.10). It does not cover roads, houses, water, rails, scaling/thinning beyond the two scale factors, or satellite imagery.

## Global Constraints

- **`AGENTS.md` applies to every task**: tests first, total coverage ≥ 85 %, strict lints and the safe-Rust rules, user docs (`docs/manual/`) and the `CHANGELOG.md` line in the same commit as the change, `make check` passing before every commit. Where this plan and `AGENTS.md` disagree, `AGENTS.md` wins.
- Every parser of untrusted input gets a fuzz target in `fuzz/` whose seed corpus also runs in `make test` (Tasks 2, 3, 4, 12 name theirs).
- Licence: **AGPL-3.0** (`LICENSE` = the GNU AGPL v3 text; every crate `license = "AGPL-3.0-or-later"`).
- Name: product **Náves**, ASCII **`naves`** for crates, binary, PBO prefixes; "DayZ" never in a crate or binary name.
- Rust stable 1.95, edition 2024. Add dependencies with `cargo add` (latest stable), no git dependencies.
- Linux first. Windows-only code is limited to `naves-export/src/bridge.rs`'s non-Linux branch; everything else must build on Windows.
- **No game data in the repo or in test fixtures.** Tests use synthetic data and a DEM crop (Copernicus attribution in `tests/fixtures/README.md`). Game files are read from the user's install at run time only.
- **Never write into Steam directories.** Read-only access to the DayZ, DayZ Server and DayZ Tools installs.
- Temporary build folders only under `/tmp` or `/var/tmp` (`tempfile` crate or `mktemp -d`), never under `$HOME`.
- Clean-room: no code from BI tools; formats from public docs (bis-file-formats, MIT) and observation of game files and tool output on our own inputs.
- armake2 must be bzed's fork (`armake2 -h 2>&1 | grep -- --proton-binarize` matches); fail with that hint otherwise.
- Local test servers never use default ports (dayz-dev skill: `free-ports.sh`).
- Map coordinates: x = east, y = north, metres, origin at the south-west corner; heightmap and material rows are stored **south row first** (row 0 = y 0).

## Review Focus

1. **Area spans several DEM tiles** (Berchtesgaden town is at 13.00 °E, the GLO-30 tile boundary): heights must be continuous across the seam. Test in Task 4.
2. **Area not fully covered by the given DEM files**: `naves build` must stop with an error naming a missing lat/lon, not write a map with holes or zeros. Test in Task 5.
3. **Non-ASCII project names** ("Königssee Test", "Náves"): config class names, PBO prefixes and file names must be ASCII identifiers. Test in Task 3.
4. **Missing external tools** (no armake2 fork, no DayZ Tools, no Proton): a clear error naming the tool and the env var to set, before any long work starts. Test in Task 9.
5. **Rebuilding into an existing output folder**: stale tiles or PBOs from a previous build (e.g. a different grid size) must not end up in the new mod. Test in Task 10.

---

### Task 1: Workspace, tooling, docs, packaging and CI scaffold

Everything `AGENTS.md` refers to exists after this task, and `make check` plus the CI workflows pass on an almost empty workspace. Five commits.

**Files:**
- Create: `Cargo.toml` (workspace), `crates/naves-formats/`, `crates/naves-core/`, `crates/naves-export/`, `crates/naves/` (each `Cargo.toml` + `src/lib.rs`, `naves` has `src/main.rs`), `crates/naves/tests/version.rs`, `LICENSE`, `LICENSES/AGPL-3.0-or-later.txt`, `REUSE.toml`, `README.md`, `.gitignore`, `rust-toolchain.toml`, `clippy.toml`
- Create: `Makefile`, `deny.toml`, `codecov.yml`, `fuzz/Cargo.toml` (cargo-fuzz crate, excluded from the workspace)
- Create: `docs/manual/book.toml`, `docs/manual/src/SUMMARY.md` + chapter stubs, `CHANGELOG.md`
- Create: `debian/` (`control`, `rules`, `changelog`, `copyright`, `source/format`)
- Create: `.github/workflows/ci.yml`, `.github/workflows/codeql.yml`, `.github/workflows/pages.yml`, `.github/workflows/release.yml`
- Modify: the existing files under `docs/` (SPDX headers)

**Interfaces:**
- Produces: crates `naves_formats`, `naves_core`, `naves_export` (libs), binary `naves`; the `make` targets of `AGENTS.md` ("Before pushing"); the manual skeleton that later tasks fill in.

- [ ] **Step 1: Write the failing test** `crates/naves/tests/version.rs` (`assert_cmd`): `naves --version` exits 0 and prints `naves 0.1.0`. Run `cargo test -p naves` → FAIL (no binary yet).
- [ ] **Step 2: Workspace.** `resolver = "3"`, members `crates/*`, `exclude = ["fuzz"]`, shared `[workspace.package]` with `version = "0.1.0"`, `edition = "2024"`, `rust-version = "1.95"`, `license = "AGPL-3.0-or-later"`, `repository = "https://github.com/bzed/naves"`. `rust-toolchain.toml`: `channel = "1.95"`, `components = ["rustfmt", "clippy", "llvm-tools-preview"]`. `[profile.release] overflow-checks = true`.
  `[workspace.lints]`: `rust.unsafe_code = "forbid"`, `rust.missing_docs = "warn"`; `clippy.all` and `clippy.pedantic` at `warn` (priority -1), plus `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `print_stdout`, `print_stderr`, `allow_attributes_without_reason` at `warn` (CI's `-D warnings` makes them errors). Every crate has `[lints] workspace = true`. `clippy.toml`: the `allow-unwrap-in-tests`, `allow-expect-in-tests`, `allow-panic-in-tests`, `allow-indexing-slicing-in-tests` options. Integration-test files and the binary's output module silence what remains with `#![expect(..., reason = "...")]`.
- [ ] **Step 3: Licence and REUSE.** `LICENSE` = full GNU AGPL v3 text (https://www.gnu.org/licenses/agpl-3.0.txt), `LICENSES/AGPL-3.0-or-later.txt` via `reuse download`. SPDX headers (`SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>`, `SPDX-License-Identifier: AGPL-3.0-or-later`) on every source and doc file, including the existing ones under `docs/`; `REUSE.toml` covers files without comment syntax (`Cargo.lock`, later the binary fixtures). `README.md`: one paragraph with the tagline "Náves – terrain generator and editor for DayZ", status "early development", licence, links to the manual and the design. `.gitignore`: `/target`, `secrets/`, `**/build/`, `/vendor/`, `docs/manual/book/`.
- [ ] **Step 4:** `crates/naves/src/main.rs` with clap `#[command(version)]`. Run `cargo test` → PASS; `cargo clippy --workspace --all-targets -- -D warnings` clean. Commit `chore: cargo workspace, AGPL-3.0 licence`.
- [ ] **Step 5: Makefile and checks.** Targets exactly as in `AGENTS.md` ("Before pushing"): `fmt`, `fmt-check`, `lint`, `doc`, `reuse`, `deny`, `test`, `fuzz`, `cover` (`cargo llvm-cov --lcov --output-path lcov.info`), `coverage-check` (`cargo llvm-cov --fail-under-lines $(COVERAGE_MIN)`, `COVERAGE_MIN = 85`), `docs`, `miri`, `deb`, and `check` running all of them except `deb`/`miri`. The Makefile exports `TMPDIR ?= /tmp` so test temp dirs never land under `$HOME`. All cargo calls use `--locked`.
  `deny.toml`: licence allow-list of AGPL-3.0-compatible licences (MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0, MPL-2.0, BSL-1.0, CC0-1.0, LGPL-2.1-or-later, LGPL-3.0, GPL-3.0-or-later, AGPL-3.0-or-later); advisories deny; `sources.unknown-git = "deny"`, `unknown-registry = "deny"`; bans warn on duplicates.
  `codecov.yml`: `coverage.status.project.default: {target: auto, threshold: 0%}`, `coverage.status.patch.default: {target: auto, threshold: 5%}`.
  `fuzz/`: cargo-fuzz crate with no targets yet; `make fuzz` runs every target from `cargo +nightly fuzz list` for 10 s (`-max_total_time=10`) and succeeds with none. Seed corpora live in `fuzz/corpus/<target>/`; each crate that owns a target runs them in a `tests/fuzz_seeds.rs` (added with the first target).
  Run `make check` → PASS. Commit `build: Makefile, cargo-deny, coverage and fuzz setup`.
- [ ] **Step 6: Manual and changelog.** Invoke `/frontend-design:frontend-design` for the manual's theme (custom CSS on top of mdBook's default). `docs/manual/` mdBook (`book.toml`: title "Náves", `[output.html] git-repository-url`), chapters as stubs: Introduction (what Náves is, status, licence), Installation (Debian package, from source), Getting started, Project file (`naves.toml`), Command line, GUI, MCP server, Troubleshooting, Credits and data licences. `make docs` = `mdbook build docs/manual` followed by `lychee --offline docs/manual/book` (broken internal links fail). `CHANGELOG.md` in Keep a Changelog format with an empty `## [Unreleased]`. Commit `docs: user manual skeleton and changelog`.
- [ ] **Step 7: Debian packaging** (native, `3.0 (native)`, version `0.1.0`, distribution `UNRELEASED` until the first release): package `naves`, `Build-Depends: debhelper-compat (= 13)` (the toolchain comes from rustup, see `AGENTS.md`). `debian/rules` builds with `cargo build --release --locked --offline` against `vendor/` (created by `cargo vendor` before `dpkg-buildpackage`; `debian/rules` writes the matching `.cargo/config.toml` and fails with a hint if `vendor/` is missing), installs `/usr/bin/naves` and a man page generated from the clap definition (`clap_mangen`; lintian warns about binaries without one). Machine-readable `debian/copyright`. `make deb` runs `cargo vendor`, `dpkg-buildpackage -us -uc -b` and `lintian --fail-on error,warning`. Run it in a `debian:trixie` container (rustup with the pinned toolchain) → PASS. Commit `build: Debian packaging`.
- [ ] **Step 8: CI.** Pin every third-party action to a commit SHA with the version in a comment.
  - `ci.yml` (push and pull request to `main`): job `check` (ubuntu, toolchain from `rust-toolchain.toml`, `make check` without fuzz); job `coverage` (`make cover`, upload `lcov.info` with `codecov/codecov-action` **v7** or newer, `token: ${{ secrets.CODECOV_TOKEN }}`, `fail_ci_if_error: true`); job `fuzz` (nightly, `make fuzz`); job `windows` (`cargo build --workspace --locked` and `cargo test --workspace --locked`); job `deb` (container `debian:trixie`, rustup with the pinned toolchain, `make deb`).
  - `codeql.yml`: languages `rust` and `actions`, on push and pull request to `main` and a weekly schedule.
  - `pages.yml`: on push to `main`, `make docs` and deploy `docs/manual/book` to GitHub Pages.
  - `release.yml`: on tag `v*`, fail unless the tag, `[workspace.package] version`, the top of `debian/changelog` and a `## [x.y.z] - YYYY-MM-DD` section in `CHANGELOG.md` agree; then `make check`, `make deb` and attach the `.deb` to the GitHub release.
  Commit `ci: GitHub Actions, CodeQL, Codecov, Pages and release check`. Push, and check that all jobs are green. Repository settings the user does once: the `CODECOV_TOKEN` secret, Pages source "GitHub Actions", and branch protection on `main` requiring the CI, CodeQL and Codecov checks.

### Task 2: Editable world (8WVR) reader and writer

**Files:**
- Create: `crates/naves-formats/src/wrp8.rs`, `crates/naves-formats/src/binio.rs`
- Test: `crates/naves-formats/tests/wrp8.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct EditableWorld {
      pub land_range_x: u32, pub land_range_y: u32,       // material grid
      pub terrain_range_x: u32, pub terrain_range_y: u32, // heightmap grid
      pub cell_size: f32,               // metres per LAND cell
      pub elevation: Vec<f32>,          // terrain_range_x * terrain_range_y, south row first
      pub material_index: Vec<u16>,     // land_range_x * land_range_y, south row first; 0 = no material
      pub materials: Vec<String>,       // materials[0] == "" always
      pub objects: Vec<WorldObject>,
  }
  pub struct WorldObject { pub transform: [f32; 12], pub id: i32, pub model: String }
  impl EditableWorld {
      pub fn write<W: std::io::Write>(&self, w: &mut W) -> std::io::Result<()>;
      pub fn read<R: std::io::Read>(r: &mut R) -> Result<Self, FormatError>;
  }
  #[derive(Debug, thiserror::Error)] pub enum FormatError { BadMagic, Io(#[from] std::io::Error), Inconsistent(String) }
  ```

Layout (little endian; from bis-file-formats `EditableWrp.cs`): `"8WVR"`, `land_range_x u32`, `land_range_y u32`, `terrain_range_x u32`, `terrain_range_y u32`, `cell_size f32`, `elevation f32[tx*ty]`, `material_index u16[lx*ly]`, `material_count u32`, then per material: if non-empty `u32 len + ASCII bytes`, always followed by a `u32 0` terminator (so entry 0 is just `0u32`); then objects until EOF: `transform f32[12]` (3×3 rotation row by row, then position x, height, y), `id i32`, `model: u32 len + ASCII`. The writer always appends the terminator object (all 12 floats `NaN`, id `i32::MAX`, empty model; bis-file-formats' `EditableWrpObject.Dummy`) that Terrain Builder writes; the reader stops at and drops it.

- [ ] **Step 1: Write the failing tests**
  ```rust
  #[test] fn roundtrip_small_world() {
      let w = sample(); // 4x4 terrain, 2x2 land, materials ["", "naves\\x\\p_000-000.rvmat"], one object "a\\b.p3d"
      let mut buf = Vec::new(); w.write(&mut buf).unwrap();
      assert_eq!(&buf[0..4], b"8WVR");
      assert_eq!(EditableWorld::read(&mut &buf[..]).unwrap(), w);
  }
  #[test] fn empty_material_zero_is_only_a_terminator() {
      // bytes right after material_count: 00 00 00 00 (entry 0), then 0x17 00 00 00 + name + 00 00 00 00
  }
  #[test] fn terminator_object_written_and_dropped() {
      // buffer ends with 12 NaN floats, i32::MAX, u32 0; read() returns objects.len() == 1
  }
  #[test] fn wrong_length_is_inconsistent() { /* elevation.len() != tx*ty -> write() returns Err(InvalidInput) */ }
  #[test] fn truncated_input_is_error() { /* every prefix of the roundtrip buffer -> Err, never a panic */ }
  #[test] fn huge_header_is_rejected_before_allocating() { /* terrain_range 0xFFFF_FFFF x 2 -> Err(Inconsistent) */ }
  ```
- Untrusted input (`AGENTS.md`, Safe Rust): grid sizes use `checked_mul` and must not exceed 8192 × 8192 (terrain) and the matching land grid, `material_count` and string lengths have limits (65 536 materials, 1024 bytes per string), ASCII is checked. Nothing is allocated from a header value before these checks.
- [ ] **Step 2:** `cargo test -p naves-formats` → FAIL (types missing).
- [ ] **Step 3:** Implement in `wrp8.rs`; little-endian helpers in `binio.rs` (`read_u32`, `read_f32s(n)`, `write_ascii32`, …). Derive `Debug, Clone, PartialEq` (NaN-free in tests).
- [ ] **Step 4:** `cargo test -p naves-formats` → PASS.
- [ ] **Step 5: Fuzz target** `fuzz/fuzz_targets/wrp8_read.rs`: `EditableWorld::read` on arbitrary bytes returns `Ok` or `Err`, never panics. Seed corpus `fuzz/corpus/wrp8_read/` = the roundtrip test's buffer plus two truncated variants; `crates/naves-formats/tests/fuzz_seeds.rs` runs the seeds. `make fuzz` → no crash.
- [ ] **Step 6:** `make check` → PASS. Commit `feat(formats): 8WVR editable world reader/writer`.

### Task 3: Project file

**Files:**
- Create: `crates/naves-core/src/project.rs`, `crates/naves-core/src/ident.rs`
- Test: `crates/naves-core/tests/project.rs`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
  pub struct Project { pub name: String, pub world: String, pub area: Area, pub scale: Scale, pub grid: Grid, pub sources: Sources }
  pub struct Area { pub center_lat: f64, pub center_lon: f64, pub real_extent_m: f64 } // square, real-world side length
  pub struct Scale { pub horizontal: f64, pub height: f64 }                          // map m per real m; height multiplier
  pub struct Grid { pub terrain_size: u32, pub land_size: u32 }                       // heightmap and material grid sizes
  pub struct Sources { pub dem: Vec<std::path::PathBuf> }                             // relative to the project dir
  impl Project {
      pub fn new(name: &str, center_lat: f64, center_lon: f64, real_extent_m: f64) -> Project; // defaults below
      pub fn load(dir: &Path) -> Result<Project, ProjectError>;   // reads <dir>/naves.toml, then parse()
      pub fn parse(toml: &str) -> Result<Project, ProjectError>;  // deserialise + validate; fuzz entry point
      pub fn save(&self, dir: &Path) -> Result<(), ProjectError>; // atomic: temp file in dir, then rename
      pub fn map_size_m(&self) -> f64;        // real_extent_m * scale.horizontal
      pub fn terrain_cell_m(&self) -> f64;    // map_size_m / terrain_size
  }
  pub fn world_ident(name: &str) -> String;   // ASCII identifier, see tests
  ```
- Defaults in `Project::new` **[from the design's test map; values are the plan's call]**: `scale = {horizontal: 0.5, height: 0.5}`, `grid = {terrain_size: 1024, land_size: 256}`, `sources.dem = []`, `world = world_ident(name)`.
- `load` validates: `terrain_size` power of two in 256..=8192, `land_size` power of two and `terrain_size % land_size == 0`, `0 < horizontal <= 1`, `0 < height <= 2`, `world` matches `^[a-z][a-z0-9_]*$`; violations → `ProjectError::Invalid(String)` naming the field.

- [ ] **Step 1: Write the failing tests**
  ```rust
  #[test] fn ident_is_ascii() {
      assert_eq!(world_ident("Náves"), "naves");
      assert_eq!(world_ident("Königssee Test"), "konigssee_test");
      assert_eq!(world_ident("3 Gipfel"), "w3_gipfel");   // must start with a letter
  }
  #[test] fn roundtrip_toml() { let p = Project::new("Berchtesgaden", 47.585, 12.965, 12800.0); /* save to tempdir, load, assert_eq */ }
  #[test] fn derived_sizes() { let p = Project::new("x", 0.0, 0.0, 12800.0);
      assert_eq!(p.map_size_m(), 6400.0); assert_eq!(p.terrain_cell_m(), 6.25); }
  #[test] fn rejects_bad_grid() { /* terrain_size 1000 -> Invalid containing "terrain_size" */ }
  ```
- [ ] **Step 2:** Run `cargo test -p naves-core --test project` → FAIL.
- [ ] **Step 3:** Implement. `world_ident`: Unicode NFKD (`unicode-normalization`), drop combining marks, lowercase, runs of non-`[a-z0-9]` → `_`, trim `_`, prefix `w` if it does not start with a letter.
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5: Fuzz target** `fuzz/fuzz_targets/project_parse.rs` (`Project::parse` on arbitrary UTF-8 → `Ok`/`Err`, never a panic); seeds: the roundtrip TOML and the invalid-grid TOML; `crates/naves-core/tests/fuzz_seeds.rs` runs them.
- [ ] **Step 6: Docs.** `docs/manual/src/project-file.md`: every field of `naves.toml`, its default and its valid range. `CHANGELOG.md` → Added: "project file `naves.toml`".
- [ ] **Step 7:** `make check` → PASS. Commit `feat(core): project file`.

### Task 4: DEM reader (Copernicus GLO-30 GeoTIFF)

**Files:**
- Create: `crates/naves-core/src/dem.rs`, `tests/fixtures/README.md`, `tests/fixtures/dem/koenigssee.tif`, `tests/fixtures/dem/seam_w.tif`, `tests/fixtures/dem/seam_e.tif`
- Test: `crates/naves-core/tests/dem.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct Dem { /* tiles */ }
  pub struct DemTile { /* one GeoTIFF */ }
  impl DemTile { pub fn from_reader<R: std::io::Read + std::io::Seek>(r: R) -> Result<DemTile, DemError>; } // fuzz entry point
  impl Dem {
      pub fn open(paths: &[PathBuf]) -> Result<Dem, DemError>;   // DemTile::from_reader per file
      pub fn height(&self, lat: f64, lon: f64) -> Option<f32>; // bilinear; None outside every tile
  }
  ```
- Supported input: single-band Float32 GeoTIFF in EPSG:4326 (geo-referencing from `ModelTiepointTag` + `ModelPixelScaleTag`), tiled or striped, DEFLATE with predictor 3 (what GLO-30 COGs use). Anything else → `DemError::Unsupported(String)`. Pixel-is-area: pixel (col,row) centre is at `origin + (col + 0.5, row + 0.5) * scale`.
- Across tiles, bilinear sampling must take neighbours from the adjacent tile (keep a lookup that resolves any (lat,lon) pixel to the right tile).

- [ ] **Step 1: Create the fixtures** (document the commands in `tests/fixtures/README.md` with the Copernicus attribution "© DLR e.V. 2010-2014 and © Airbus Defence and Space GmbH 2014-2018 provided under COPERNICUS by the European Union and ESA; all rights reserved"):
  ```sh
  B=https://copernicus-dem-30m.s3.amazonaws.com
  T12=Copernicus_DSM_COG_10_N47_00_E012_00_DEM; T13=Copernicus_DSM_COG_10_N47_00_E013_00_DEM
  CO="-co TILED=YES -co BLOCKXSIZE=64 -co BLOCKYSIZE=64 -co COMPRESS=DEFLATE -co PREDICTOR=3"
  gdal_translate -q $CO -projwin 12.96 47.56 12.99 47.54 /vsicurl/$B/$T12/$T12.tif koenigssee.tif
  gdal_translate -q $CO -projwin 12.98 47.64 13.00 47.62 /vsicurl/$B/$T12/$T12.tif seam_w.tif
  gdal_translate -q $CO -projwin 13.00 47.64 13.02 47.62 /vsicurl/$B/$T13/$T13.tif seam_e.tif
  ```
  Each file is ~15 KB.
- [ ] **Step 2: Write the failing tests**
  ```rust
  #[test] fn koenigssee_surface() {   // gdallocationinfo gives 602 at this point
      let d = Dem::open(&[fx("koenigssee.tif")]).unwrap();
      assert!((d.height(47.5534, 12.9765).unwrap() - 602.0).abs() < 1.0);
  }
  #[test] fn outside_is_none() { assert!(Dem::open(&[fx("koenigssee.tif")]).unwrap().height(47.0, 12.0).is_none()); }
  #[test] fn seam_is_continuous() {
      let d = Dem::open(&[fx("seam_w.tif"), fx("seam_e.tif")]).unwrap();
      let step = 1.0 / 3600.0 / 4.0;            // quarter pixel
      let a = d.height(47.63, 13.0 - step).unwrap(); let b = d.height(47.63, 13.0 + step).unwrap();
      assert!((a - b).abs() < 5.0);              // no step or None at the seam
  }
  #[test] fn rejects_non_float() { /* an 8-bit PNG/TIFF -> Err(Unsupported) */ }
  ```
  Write the expected values of `seam_*` with `gdallocationinfo -wgs84 -valonly` before relying on the 5 m bound; adjust the bound only if the real terrain there is steeper.
- [ ] **Step 3:** Run `cargo test -p naves-core --test dem` → FAIL.
- [ ] **Step 4:** Implement with the `tiff` crate (decoder handles DEFLATE + floating-point predictor; verify, and if not, undo predictor 3 yourself: per row, byte-wise horizontal differencing over the shuffled byte planes, as in the TIFF Tech Note 3).
- Untrusted input: image dimensions are checked against a limit (GLO-30 tiles are 3600 × 3600; allow up to 16 384 × 16 384) before decoding; a corrupt TIFF is an `Err`, never a panic.
- [ ] **Step 5:** Run → PASS.
- [ ] **Step 6: Fuzz target** `fuzz/fuzz_targets/dem_tile.rs` (`DemTile::from_reader(Cursor::new(data))`); seeds: the three fixtures; `crates/naves-core/tests/fuzz_seeds.rs` runs them. **REUSE:** a `REUSE.toml` annotation for `tests/fixtures/dem/*.tif` with `LicenseRef-Copernicus-DEM` and the attribution above, licence text in `LICENSES/LicenseRef-Copernicus-DEM.txt`. **Docs:** "Credits and data licences" chapter gets the Copernicus attribution; `docs/manual/src/project-file.md` says which DEM files `sources.dem` accepts. `CHANGELOG.md` → Added: "Copernicus GLO-30 GeoTIFF DEM input".
- [ ] **Step 7:** `make check` → PASS. Commit `feat(core): GeoTIFF DEM reader`.

### Task 5: Projection and heightmap

**Files:**
- Create: `crates/naves-core/src/projection.rs`, `crates/naves-core/src/heightmap.rs`
- Test: `crates/naves-core/tests/heightmap.rs`

**Interfaces:**
- Consumes: `Project`, `Dem` (Tasks 3, 4).
- Produces:
  ```rust
  pub struct LocalProjection; // UTM zone of the area centre, via proj4rs
  impl LocalProjection { pub fn for_center(lat: f64, lon: f64) -> Self;
      pub fn to_geo(&self, e: f64, n: f64) -> (f64, f64); pub fn to_utm(&self, lat: f64, lon: f64) -> (f64, f64); }
  pub struct Heightmap { pub size: u32, pub cell_m: f32, pub h: Vec<f32> } // south row first
  impl Heightmap { pub fn at(&self, col: u32, row: u32) -> f32; pub fn min_max(&self) -> (f32, f32); }
  pub fn build_heightmap(p: &Project, dem: &Dem) -> Result<Heightmap, HeightmapError>;
  #[derive(Debug, thiserror::Error)] pub enum HeightmapError { #[error("no DEM data at {lat:.5},{lon:.5}")] MissingDem { lat: f64, lon: f64 } }
  ```
- Sampling rule: grid vertex (col,row) sits at map `(x,y) = (col, row) * cell_m`; real offset from the centre `= ((x, y) - map_size/2) / scale.horizontal`; UTM `= centre_utm + offset`; height `= dem(to_geo(utm)) * scale.height`. Parallel over rows (`rayon`).

- [ ] **Step 1: Write the failing tests** (use a synthetic in-memory `Dem` built from a tiny generated GeoTIFF or a test-only constructor `Dem::from_fn(bounds, f)`, add it behind `#[cfg(any(test, feature = "test-util"))]`):
  ```rust
  #[test] fn projection_roundtrip() { let p = LocalProjection::for_center(47.585, 12.965);
      let (e, n) = p.to_utm(47.585, 12.965); let (la, lo) = p.to_geo(e, n);
      assert!((la - 47.585).abs() < 1e-7 && (lo - 12.965).abs() < 1e-7); }
  #[test] fn height_scale_applies() { /* constant 1000 m DEM, height 0.5 -> every h == 500 */ }
  #[test] fn horizontal_scale_stretches() { /* DEM = east-west ramp 1 m per real metre; horizontal 0.5:
       neighbouring vertices differ by cell_m / 0.5 metres */ }
  #[test] fn south_row_first() { /* DEM increasing with latitude -> h[row 0] < h[last row] */ }
  #[test] fn missing_coverage_is_error() { /* DEM covering only the west half -> Err(MissingDem{..}) */ }
  ```
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit `feat(core): scaled heightmap from DEM`.

### Task 6: Binarize bridge experiment (gate for the rest of the plan)

Goal: establish the exact inputs with which BI's `binarize.exe` under Proton turns our 8WVR into an OPRW world, and pin them in `docs/formats/binarize-world.md`. Nothing after this task is worth building if this fails.

**Files:**
- Create: `docs/formats/binarize-world.md`, `docs/formats/terrain-layers.md`, `tools/bridge-experiment.sh`
- Create: `crates/naves-formats/examples/flat_world.rs` (writes a test 8WVR)

**Interfaces:**
- Consumes: `EditableWorld` (Task 2).
- Produces: the documented facts Tasks 7–9 implement (file layout under `P:`, binarize arguments, required config entries, tile size/overlap, satmap/mask naming).

- [ ] **Step 1: Pin the terrain layer geometry from Livonia** (observation of game files; read-only). Extract two neighbouring tile rvmats and the world config:
  ```sh
  A="$STEAM/steamapps/common/DayZ/Addons"; W=$(mktemp -d /var/tmp/naves-obs.XXXXXX)
  armake2 inspect "$A/worlds_enoch_data.pbo" > $W/inv.txt
  armake2 cat "$A/worlds_enoch_data.pbo" 'layers\P_000-000_L00_L01_L02_L10_L09_L04.rvmat' $W/p00.rvmat
  # plus the P_001-000_* and P_000-001_* files listed in inv.txt; armake2 derapify each
  armake2 cat "$A/worlds_enoch.pbo" config.bin $W/config.bin && armake2 derapify $W/config.bin $W/config.cpp
  ```
  Livonia tile 0,0 has `TexGen3.uvTransform aside = 0.0023437499`, `pos = {0.03125, 30.031248, 0}`. From neighbouring tiles derive: tile world size, overlap in pixels, satmap/mask tile pixel size (read `S_000_000_lco.paa` dimensions with `armake2 paa2img`), and which corner is tile 0,0. Write them into `docs/formats/terrain-layers.md` with the formulas, and add a test in Task 7 that reproduces Livonia's numbers from the formulas.
- [ ] **Step 2: Build a minimal addon by hand** with `flat_world.rs` (terrain 256×256, land 64×64, cell 25 m, height 100 m everywhere, all `material_index = 1`, `materials = ["", "naves\\worlds\\bridgetest\\data\\layers\\p_000-000.rvmat"]`), one rvmat copied in shape from Livonia's (Stage0 satmap tile, Stage1 mask tile, one grass layer using vanilla `dz\surfaces_bliss\data\terrain\en_grass1_*` texture paths), a uniform green satmap tile and a uniform mask tile (`armake2 img2paa`), and a `config.cpp` with `CfgPatches` (`requiredAddons[] = {"DZ_Data","DZ_Surfaces","DZ_Surfaces_Bliss"}` — verify names in the derapified Livonia config) + `CfgWorlds` (`class bridgetest: CAWorld`, `worldName = "naves\worlds\bridgetest\bridgetest.wrp"`, `centerPosition`, `class Grid`, `class OutsideTerrain`, `midDetailTexture`, `class Names {}`; copy the field list from Livonia, values ours) + `CfgWorldList { class bridgetest {}; }`.
- [ ] **Step 3: Run binarize in a Proton sandbox** following the recipe in the dayz-dev skill (`systems/mod-packaging.md`, "binarize.exe under Proton"): sandbox `$W` as `Z:`, `$W/P` as `P:` holding `naves/worlds/bridgetest/...`, current directory `P:\`, `STEAM_COMPAT_DATA_PATH` inside `$W`. Arguments to try first (ArmaRealMap's world call): `binarize.exe -always -textures=Z:\tex -binPath=P:\ P:\naves\worlds\bridgetest Z:\out`. If vanilla textures must resolve, extract the needed `dz\surfaces_bliss\data\terrain\*` and `dz\data\...` files from the user's install into `$W/P/dz/...` (sandbox only, deleted afterwards).
  Pass criterion: `$W/out/bridgetest.wrp` exists and starts with `OPRW` and version `32` (`xxd -l 8`). Record the exact working command, inputs, log lines and runtime.
- [ ] **Step 4: Load test.** Pack `$W/out` + config with `armake2 build` into `@bridgetest/addons/bridgetest.pbo`, sign it, and boot a local server with a mission copied from `dayzOffline.enoch` (only `init.c`, `db/`, `cfgeconomycore.xml`, `cfgplayerspawnpoints.xml` with one point at 3200/3200) per the dayz-dev skill's `testing/local-server.md`. Pass criterion: the RPT shows the world loading without error and the server reaches mission start. Then join with the headless client (`testing/client-testing.md`) and take one screenshot standing on the terrain.
- [ ] **Step 5: Write `docs/formats/binarize-world.md`**: working command, environment, required P: layout, required config fields, what failed on the way. Save `tools/bridge-experiment.sh` as the replayable script (no game data inside; it extracts from the install at run time).
- [ ] **Step 6: Decide.** If Step 3 or 4 fails after reasonable variations, stop the plan here and report to the user: the fallback (design 5.6) is a native OPRW v32 writer, which needs a new plan. Otherwise commit `docs: binarize world bridge, terrain layer geometry`.

### Task 7: Terrain layers (procedural satmap, mask, per-tile rvmats)

**Files:**
- Create: `crates/naves-export/src/layers.rs`, `crates/naves-export/src/rvmat.rs`
- Test: `crates/naves-export/tests/layers.rs`

**Interfaces:**
- Consumes: `Heightmap` (Task 5); geometry facts from `docs/formats/terrain-layers.md` (Task 6).
- Produces:
  ```rust
  pub struct TileGrid { pub tiles_x: u32, pub tiles_y: u32, pub tile_px: u32, pub overlap_px: u32, pub px_m: f64 }
  pub fn tile_grid(map_size_m: f64, land_size: u32) -> TileGrid;            // formulas from terrain-layers.md
  pub struct Surface { pub name: &'static str, pub nopx: &'static str, pub ca: &'static str, pub sat_rgb: [u8; 3] }
  pub const SURFACES: &[Surface];   // M1 step 1: "grass" (en_grass1), "rock" (en_rock or the closest vanilla bliss rock texture), "snow" if a vanilla terrain texture exists; paths verified against the install in Task 6
  pub fn classify(hm: &Heightmap) -> Vec<u8>;           // per heightmap vertex: index into SURFACES; rock where slope > 35°, snow above (height ≥ 1900 m × scale.height) if available
  pub struct LayerSet { pub materials: Vec<String>, pub material_index: Vec<u16>, pub files: Vec<(String, Vec<u8>)> } // P:-relative path -> PNG or rvmat text
  pub fn build_layers(hm: &Heightmap, land_size: u32, prefix: &str) -> LayerSet;
  ```
- File naming follows Livonia's: `data\layers\s_XXX_YYY_lco.png` (satmap), `m_XXX_YYY_lca.png` (mask), `p_XXX-YYY.rvmat`; PNG→PAA conversion happens in Task 10 with `armake2 img2paa`.
- Satmap colour: the surface's `sat_rgb`, shaded by a simple hillshade (sun azimuth 315°, altitude 45°) so relief is visible on the 2D map.

- [ ] **Step 1: Write the failing tests**
  ```rust
  #[test] fn livonia_tile_geometry() { /* tile_grid(12800.0, Livonia land_size) reproduces aside = 0.0023437499 and pos x = 0.03125 for tile 0,0 (values from terrain-layers.md) */ }
  #[test] fn every_land_cell_has_material() { /* build_layers on a 256 heightmap: material_index all in 1..materials.len() */ }
  #[test] fn steep_is_rock() { /* heightmap with a 60° wall: classify() marks the wall rock, flat ground grass */ }
  #[test] fn rvmat_shape() { /* p_000-000.rvmat contains PixelShaderID = "TerrainX", Stage0 texture ending s_000_000_lco.paa, Stage1 m_000_000_lca.paa */ }
  ```
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit `feat(export): procedural terrain layers`.

### Task 8: World config and mission

**Files:**
- Create: `crates/naves-export/src/config.rs`, `crates/naves-export/src/mission.rs`
- Test: `crates/naves-export/tests/config.rs`, `crates/naves-export/tests/mission.rs`

**Interfaces:**
- Consumes: `Project`, `Heightmap`, field list from `docs/formats/binarize-world.md`.
- Produces:
  ```rust
  pub fn prefix(world: &str) -> String;                                  // "naves\\worlds\\<world>"
  pub fn world_config(p: &Project, hm: &Heightmap) -> String;            // config.cpp text
  pub fn write_mission(server_mpmissions: &Path, out_dir: &Path, world: &str, spawn: (f32, f32)) -> Result<PathBuf, MissionError>; // returns out_dir/dayzOffline.<world>
  ```
- `world_config`: `CfgPatches { class naves_<world> { units[]={}; weapons[]={}; requiredVersion=0.1; requiredAddons[]=<from Task 6>; } }`, `CfgWorldList`, `CfgWorlds` with `class <world>: CAWorld` and the fields from Task 6; `centerPosition = {map/2, map/2, max height}`; `class Names` with one location (`project.name`) at the centre. The file is generated text; it contains no BI content.
- `write_mission` copies `dayzOffline.enoch` from the **server install** (read-only source) into `out_dir/dayzOffline.<world>` (never into Steam), then: `cfgplayerspawnpoints.xml` → one generator position at `spawn` (fresh, hop and travel sections the same point), empties `mapgrouppos.xml` and `cfgeventspawns.xml` entries, keeps everything else. The mission is assembled at build time, never committed.

- [ ] **Step 1: Write the failing tests**
  ```rust
  #[test] fn config_names_world() { let c = world_config(&p, &hm);
      assert!(c.contains("class berchtesgaden: CAWorld")); assert!(c.contains(r#"worldName = "naves\worlds\berchtesgaden\berchtesgaden.wrp";"#)); }
  #[test] #[ignore = "needs armake2"] fn config_rapifies() { /* write to tempdir, run `armake2 rapify` on it, exit status 0 */ }
  #[test] fn mission_spawn_point() { /* fake mpmissions dir with a minimal dayzOffline.enoch (synthetic XML written by the test);
       write_mission -> cfgplayerspawnpoints.xml has exactly one <pos x="3200" z="3200"/> per section */ }
  #[test] fn mission_never_writes_source() { /* source dir made read-only; write_mission succeeds */ }
  ```
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit `feat(export): world config and mission`.

### Task 9: Proton binarize bridge (code)

**Files:**
- Create: `crates/naves-export/src/bridge.rs`, `crates/naves-export/src/tools.rs`
- Test: `crates/naves-export/tests/bridge.rs`

**Interfaces:**
- Consumes: the recipe from `docs/formats/binarize-world.md`.
- Produces:
  ```rust
  pub struct Tools { pub armake2: PathBuf, pub binarize: BinarizeBridge }
  pub struct BinarizeBridge { pub steam_root: PathBuf, pub dayz_tools: PathBuf, pub dayz: PathBuf, pub proton: PathBuf }
  impl Tools { pub fn locate() -> Result<Tools, ToolError>; }   // env: ARMAKE2, STEAM_ROOT, DAYZ_TOOLS, DAYZ, PROTON (same names as armake2 where shared)
  impl BinarizeBridge {
      /// staging: a folder laid out as P: (our prefix tree). Returns the OPRW path in a temp output dir.
      pub fn binarize_world(&self, staging_p: &Path, prefix: &str, world: &str) -> Result<PathBuf, ToolError>;
  }
  #[derive(Debug, thiserror::Error)] pub enum ToolError {
      #[error("{tool} not found; set {env} (hint: {hint})")] Missing { tool: &'static str, env: &'static str, hint: &'static str },
      #[error("armake2 is not bzed's fork (no --proton-binarize); install https://github.com/bzed/armake2")] WrongArmake2,
      #[error("binarize failed (exit {code}); log: {log}")] Binarize { code: i32, log: PathBuf },
      #[error("binarize produced no OPRW world")] NoWorld { log: PathBuf },
      #[error("{tool} did not finish within {secs} s; log: {log}")] Timeout { tool: &'static str, secs: u64, log: PathBuf },
  }
  ```
- Sandbox in `/var/tmp` (`tempfile::Builder::new().prefix("naves-bin.").tempdir_in("/var/tmp")`), private Wine prefix inside it, `Z:` and `P:` confined as in the recipe; copy of DayZ Tools' `Bin/Binarize` into the sandbox; vanilla files the recipe found necessary are extracted from the install into the sandbox only. The log is kept next to the build output (`build/logs/binarize.log`); the sandbox is deleted on success and kept (path in the error) on failure. Never kill Wine processes by pattern; wait for the child and its `wineserver -w`. Every external call goes through `Command` with an argument list (no shell) and a timeout (binarize: 30 min default, `NAVES_BINARIZE_TIMEOUT` overrides); on timeout, stop the sandbox's own prefix with `wineserver -k` (scoped by `WINEPREFIX`, never by pattern) and return `ToolError::Timeout`.
- On non-Linux: `binarize_world` runs `binarize_x64.exe` directly (path from `DAYZ_TOOLS`); not tested in this plan.

- [ ] **Step 1: Write the failing tests**
  ```rust
  #[test] fn missing_armake2_is_named() { /* PATH and ARMAKE2 pointing at an empty dir -> Missing{tool:"armake2", env:"ARMAKE2"} */ }
  #[test] fn upstream_armake2_rejected() { /* ARMAKE2 = a test script printing help without --proton-binarize -> WrongArmake2 */ }
  #[test] fn missing_dayz_tools_is_named() { /* STEAM_ROOT = empty tempdir -> Missing{tool:"DayZ Tools", env:"DAYZ_TOOLS"} */ }
  #[test] fn hanging_tool_times_out() { /* stub tool that never exits, timeout 1 s -> Err(Timeout{..}) */ }
  #[test] #[ignore = "needs Steam, DayZ Tools and Proton"] fn binarizes_flat_world() {
      /* the Task 6 flat world through binarize_world -> file starts with b"OPRW" and u32 32 */ }
  ```
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** `cargo test -p naves-export --test bridge` → PASS; `cargo test -p naves-export --test bridge -- --ignored` on the dev machine → PASS.
- [ ] **Step 5: Docs.** `docs/manual/src/installation.md`: required external tools (bzed's armake2 fork, DayZ Tools, Proton) and the environment variables; `troubleshooting.md`: every `ToolError` message and what to do. `CHANGELOG.md` → Added: "binarize worlds with DayZ Tools under Proton".
- [ ] **Step 6:** `make check` → PASS. Commit `feat(export): Proton binarize bridge for worlds`.

### Task 10: Build pipeline and CLI

**Files:**
- Create: `crates/naves-export/src/build.rs`, `crates/naves/src/cli.rs`
- Modify: `crates/naves/src/main.rs`
- Test: `crates/naves/tests/cli.rs`

**Interfaces:**
- Consumes: Tasks 2–9.
- Produces:
  ```rust
  pub struct BuildOutput { pub mod_dir: PathBuf, pub mission_dir: PathBuf, pub warnings: Vec<String> }
  pub fn build(project_dir: &Path, tools: &Tools, progress: &dyn Fn(&str)) -> Result<BuildOutput, BuildError>;
  ```
  CLI:
  - `naves new <dir> --name <name> --center <lat>,<lon> --extent <m>`: creates `<dir>/naves.toml` (fails if it exists)
  - `naves build <dir>`: runs `build`; prints the mod and mission paths
  - `naves gui [<dir>]` (Task 11) and `naves mcp <dir>` (Task 12)
- `build` steps: load project → open DEM (`sources.dem`; error if empty: "add DEM files to sources.dem") → heightmap → layers → `EditableWorld` (terrain = heightmap, land = layer material grid, `cell_size = map_size / land_size`, no objects) → staging tree `build/staging/P/<prefix>/` (config.cpp, `<world>.wrp`, `data/layers/*`) with PNGs converted by `armake2 img2paa` (satmap `-t dxt1`, mask `-t dxt5`) → `binarize_world` → `armake2 build` of the prefix folder (OPRW replacing the 8WVR) into `build/@naves_<world>/addons/naves_<world>.pbo` → key `secrets/naves_<world>.biprivatekey` (create with `armake2 keygen` if missing; `.bikey` copied to `build/@naves_<world>/keys/`) → `armake2 sign` → mission into `build/missions/dayzOffline.<world>` (server path from `DAYZ_SERVER`, else the Steam library's `DayZServer`).
- `build/staging`, `build/@naves_<world>` and `build/missions` are removed before each build (Review Focus 5); `secrets/` is never removed.

- [ ] **Step 1: Write the failing tests**
  ```rust
  #[test] fn new_writes_project() { /* naves new tmp --name "Königssee Test" --center 47.55,12.97 --extent 4000 -> naves.toml with world = "konigssee_test" */ }
  #[test] fn new_refuses_existing() { /* second run -> non-zero exit, message contains "exists" */ }
  #[test] fn build_without_dem_explains() { /* -> non-zero exit, message contains "sources.dem" */ }
  #[test] fn rebuild_removes_stale_files() { /* create build/@naves_x/addons/old.pbo, run build with a stub Tools (feature "test-util": binarize copies the 8WVR, armake2 = a script that writes the PBO) -> old.pbo gone */ }
  #[test] #[ignore = "needs Steam, DayZ Tools, Proton, armake2"] fn builds_koenigssee_fixture() {
      /* project with the fixture DEM, extent 2000, terrain_size 256 -> mod PBO exists, `armake2 verify` OK */ }
  ```
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement (CLI with clap derive). **Step 4:** Run → PASS (`-- --ignored` on the dev machine too).
- [ ] **Step 5: Docs.** `docs/manual/src/cli.md` (every command and option, matching `--help`), `getting-started.md` (new → add DEM → build → load on a server); the man page follows from clap. `CHANGELOG.md` → Added: "`naves new` and `naves build`".
- [ ] **Step 6:** `make check` → PASS. Commit `feat: naves new/build`.

### Task 11: GUI basics

**Files:**
- Create: `crates/naves/src/gui/mod.rs`, `crates/naves/src/gui/map_view.rs`, `crates/naves-core/src/render.rs`
- Test: `crates/naves-core/tests/render.rs`

**Interfaces:**
- Consumes: `Project`, `build_heightmap`, `build` (Tasks 3, 5, 10).
- Produces:
  ```rust
  // naves-core::render, shared with MCP (Task 12)
  pub struct RenderRequest { pub x0: f64, pub y0: f64, pub x1: f64, pub y1: f64, pub px: u32, pub overlay: Overlay } // map metres, output width in pixels
  pub enum Overlay { Hillshade, Slope, Height }
  pub fn render_topdown(hm: &Heightmap, req: &RenderRequest) -> image::RgbaImage; // north up
  ```
- GUI (eframe): window title "Náves – <project name>"; left panel: project settings (name, centre, extent, both scales, grid sizes) editable and saved to `naves.toml`; central panel: top-down map from `render_topdown` with pan/zoom and overlay switch; bottom panel: build button running `build` on a worker thread with streamed progress lines and warnings/errors. No 3D view and no object editing in this plan (3D view comes with the buildings plan).

- [ ] **Step 1: Write the failing tests** (render only; GUI is checked by hand):
  ```rust
  #[test] fn north_up() { /* heightmap rising to the north, Overlay::Height: top image row brighter than bottom row */ }
  #[test] fn aspect_kept() { /* request 2000x1000 m at px 400 -> image 400x200 */ }
  #[test] fn slope_overlay_marks_steep() { /* 60° wall -> wall pixels red-dominant */ }
  ```
- **Before designing or writing any GUI code, invoke the `/frontend-design:frontend-design` skill** (`AGENTS.md`, User interface) and settle the visual direction there: layout, typography, colours, map-view styling.
- Coverage: keep `eframe` code thin. The settings form state (edit, validate, save), the build worker's message handling and the view's pan/zoom maths are plain structs with tests; widget wiring is tested with `egui_kittest` (settings field edit → `naves.toml` changes; build button → progress lines appear, using the stub `Tools` from Task 10).
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement render + GUI. **Step 4:** Run → PASS; `cargo run -p naves -- gui <fixture project>` shows the map, saving a setting changes `naves.toml`, build streams progress.
- [ ] **Step 5: Docs.** `docs/manual/src/gui.md` with a screenshot (taken from the fixture project, committed under `docs/manual/src/images/`). `CHANGELOG.md` → Added: "GUI: project settings, map view, build".
- [ ] **Step 6:** `make check` → PASS. Commit `feat: GUI basics (project, map view, build)`.

### Task 12: MCP server (read, render, settings, rebuild)

**Files:**
- Create: `crates/naves/src/mcp.rs`
- Test: `crates/naves/tests/mcp.rs`

**Interfaces:**
- Consumes: `Project`, `render_topdown`, `build`.
- Produces: `naves mcp <dir>` — MCP server over **stdio** (`rmcp`), local only. Tools:
  - `project_get` → the project as JSON
  - `project_set` `{path: "scale.height", value: 0.75}` → validates like `Project::load`, saves, returns the new project
  - `heightmap_stats` → `{min, max, size, cell_m, map_size_m}`
  - `render_topdown` `{x0,y0,x1,y1,px,overlay}` → PNG image content (`px` capped at 2048)
  - `build` → runs the build, returns mod/mission paths and warnings (long-running; progress via MCP logging notifications)
- The GUI-embedded server ("on by default", design 5.10) and the auto/approval modes come with the editor plans; in this plan `naves mcp` is the entry point and the README shows how to register it with an MCP client.

- [ ] **Step 1: Write the failing tests** (drive the server in-process over a duplex pipe with an `rmcp` client):
  ```rust
  #[tokio::test] async fn lists_tools() { /* names == ["build","heightmap_stats","project_get","project_set","render_topdown"] */ }
  #[tokio::test] async fn set_validates() { /* project_set scale.horizontal = 3 -> tool error mentioning "horizontal"; naves.toml unchanged */ }
  #[tokio::test] async fn render_returns_png() { /* image content, mime image/png, decodes to px width */ }
  ```
- MCP requests are untrusted input: `project_set` paths and values are validated like `Project::parse`, `render_topdown` caps `px` and rejects inverted or non-finite bounds, nothing panics. Blocking work (render, build) runs in `spawn_blocking`.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** Run → PASS.
- [ ] **Step 5: Fuzz target** `fuzz/fuzz_targets/mcp_project_set.rs`: arbitrary `{path, value}` JSON applied to a sample project → `Ok`/`Err`, never a panic; seeds from the `set_validates` test.
- [ ] **Step 6: Docs.** `docs/manual/src/mcp.md`: the tools, their arguments, and how to register `naves mcp <dir>` with an MCP client (moves out of the README, which links to it). `CHANGELOG.md` → Added: "MCP server `naves mcp`".
- [ ] **Step 7:** `make check` → PASS. Commit `feat: MCP server (project, render, build)`.

### Task 13: Berchtesgaden end to end

**Files:**
- Create: `examples/berchtesgaden/naves.toml`, `examples/berchtesgaden/README.md` (how to fetch the DEM tiles; tiles themselves not committed, `examples/*/dem/` in `.gitignore`)
- Modify: `README.md` (quick start, linking to the manual), `docs/manual/src/getting-started.md` (the Berchtesgaden walkthrough)

- [ ] **Step 1:** `naves new examples/berchtesgaden --name Berchtesgaden --center 47.585,12.965 --extent 12800`; download `Copernicus_DSM_COG_10_N47_00_E012_00_DEM.tif` and `…E013…` into `examples/berchtesgaden/dem/`, add them to `sources.dem`.
- [ ] **Step 2:** `naves build examples/berchtesgaden`. Expected: mod and mission paths printed, no error.
- [ ] **Step 3:** Boot a local server with the mod and mission (dayz-dev skill: `make-server-tree.sh`, `free-ports.sh 3`, `-mod=@naves_berchtesgaden`, `-mission=` the built mission). Expected: RPT without world/terrain errors, mission starts.
- [ ] **Step 4:** Join with the headless client (dayz-dev `dayz-client-test.sh`/`client-testing.md`; one game at a time, stop it right after), walk a few metres, take screenshots looking at the Watzmann from the north and over the Königssee basin. Expected: terrain visible with the procedural surfaces; the player can walk. Remove the client `@mod` symlink afterwards.
- [ ] **Step 5:** Through `naves mcp`, call `render_topdown` for the whole map with `Hillshade` and check by eye that the Königssee basin and the Watzmann are where they belong (Königssee south of the centre, Watzmann west of it).
- [ ] **Step 6:** Record results (screenshots outside the repo; paths and RPT excerpts in `examples/berchtesgaden/README.md`). `make check` → PASS. Commit `docs: Berchtesgaden walkable test map`. Push; CI, CodeQL and Codecov green.

---

## Later plans (milestone 1, in order; each gets its own plan)

2. **Roads:** catalog of vanilla road pieces (ODOL v56 reader for memory points), network solver (crossing to crossing, floating junctions, tolerance gaps), vertical alignment, terrain conforming solver, bridges; objects in the 8WVR; first object data model with stable identity and overrides.
3. **Houses:** building catalog (configs, footprints, doors), context-based type choice, entrances, building pads; seed tag file via MCP rendering; 3D view in the GUI; undo history.
4. **Water:** lakes (water planes, generated beds, swim depth, leak check), rivers.
5. **Railways and bridges.**
6. **Scaling and thinning** (overfull detection, priorities, dropped-for-scale objects), scaling preview.
7. **Satellite imagery and surface mask from land use** (replacing the procedural satmap).

Separate track in bzed's armake2 fork: whatever these plans need there (design 5.6).

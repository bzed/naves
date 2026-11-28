<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# dayzmake-core instead of dayzmake, and `naves navmesh` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Náves links the `dayzmake-core` crate for every PBO, PAA and key operation instead of running the
`dayzmake` program, and gets a `naves navmesh <project>` command that starts `DayZDiag_x64.exe` as a navmesh
data server under Proton and builds `<project>/navmesh/<world>.nm` with dayzmake-core's generator (no flare-navgen,
no `dayzmake` executable).

**Architecture:** Part 1 (Tasks 1–4) replaces each `dayzmake` subprocess call in `naves-export` (`img2paa`,
`keygen`, `build -k`, `inspect`, `cat`, `unpack`, `--version`) with the matching dayzmake-core call, then removes
the `dayzmake` tool from `Tools`, the settings, the GUI, the fakes and the docs. Part 2 (Tasks 5–8) adds a
`navmesh` module (generate from a data server or a `.dzgeo` capture, write the mesh atomically) and a
`navmesh_server` module (server tree in the sandbox, temporary mod symlink in the DayZ install, DayZDiag under
Proton, readiness probe, clean stop), wired into a new CLI subcommand.

**Tech Stack:** Rust 1.99 / edition 2024, `dayzmake-core` 0.3.1 (features `sign`, `image`, `navmesh`), clap,
`tempfile`, the existing Proton bridge in `crates/naves-export/src/bridge.rs`.

**Spec:** no separate design doc. This plan is the spec; it argues from `AGENTS.md`, the maintainer's decisions
below, `docs/manual/src/navmesh.md` (the manual steps it replaces) and dayzmake's navmesh guide
(`~/workspace/dayz/tools/dayzmake/docs/book/src/guides/navmesh.md`, the verified DayZDiag recipe).

Maintainer decisions (2026-11-16 and 2026-11-28):
- "use the dayzmake-core crate instead of calling dayzmake"; "use the navmesh generation code instead of running
  external programs".
- `naves navmesh` **starts DayZDiag itself** (temporary `@naves_<world>` symlink in the DayZ install, removed
  afterwards); `--server host:port` connects to a data server the user started instead.
- Missing items in dayzmake-core become GitHub issues in `bzed/dayzmake` (check for duplicates first with
  `gh issue list -R bzed/dayzmake --state all`). Already filed: #132 (`image` 0.24 vs 0.25), #133 (no published
  fake data server / synthetic world).

## Global Constraints

- Work in the worktree `/var/tmp/naves-dzcore/wt`, branch `dayzmake-core`, based on `origin/main` (88412f0).
- Read `/home/bzed/.claude/skills/linux-basics/SKILL.md` and follow it (its `references/` when a topic comes up).
  Every subagent brief repeats this line. In particular: no `sudo`/root; `GIT_PAGER=cat`, `GIT_EDITOR=true`;
  quote every pattern (the shell is zsh); `rg --color=never` with a path, and `-uu` before concluding something is
  absent; temp files only in `/tmp` or `/var/tmp` (big ones in `/var/tmp`); never `pgrep -f`/`pkill -f` on a bare
  name (use the tracked PID, `pgrep -x`, or `pgrep -af '[D]ayZDiag'`); SIGTERM before `kill -9`.
- Fake tool scripts in `testing.rs` are POSIX `sh` (dash): no bashisms; check each with `shellcheck -s sh`.
- Heavy jobs: start only when the 1-minute load average is below 20 (else wait in 5-minute steps, up to 60 min),
  and always as
  `CARGO_BUILD_JOBS=12 nice -n 19 ionice -c 3 systemd-run --user --scope --quiet -p CPUQuota=1200% -p MemoryMax=16G -p MemorySwapMax=1G make check`
  (same wrapper for `cargo test`/`cargo build`). One heavy job of this branch at a time.
- `dayzmake-core = "0.3.1"` added with `cargo add -p naves-export dayzmake-core@0.3.1` (default features =
  `sign`, `image`, `navmesh`; all three are used). No git or path dependency.
- `deny.toml`: add `"GPL-2.0-or-later"` to `[licenses] allow` (dayzmake-core's licence; AGPL-3.0-or-later can link
  it). The duplicate `image` 0.24 is a `multiple-versions = "warn"`, accepted until bzed/dayzmake#132.
- The commit that adds the crate carries the AGENTS.md review (security, maintenance, standards) in its message;
  the crate is the maintainer's own, the review is still required. Source: `~/.cargo/registry/src/*/dayzmake-core-0.3.1/`.
- dayzmake-core errors are `std::io::Error`; naves error variants wrap them as `source: std::io::Error` with the
  file and the operation as context.
- AGENTS.md rules all apply: test first, no panics on untrusted data, atomic writes, `tracing` not `println!` in
  libraries, `#[expect(..., reason)]`, coverage ≥ 85 %, every user-visible change updates `docs/manual/` and
  `CHANGELOG.md` (`## [Unreleased]`) in the same commit, clap help and `docs/manual/src/cli.md` agree.
- Build output stays byte-for-byte reproducible (same project, same PBO and `.bisign` bytes).
- Tests never touch `$HOME`, a Steam folder or a real DayZ install; game runs are manual (Task 8) or
  `#[ignore = "needs DayZ"]`.
- Wine/Proton programs started by agents run only inside a headless sway; DayZ under `MemoryMax`: server 8G.
  Never edit the user's DayZ profile or settings. Remove any symlink placed in the DayZ install when done.
- Commit prefixes `feat:`/`fix:`/`refactor:`/`test:`/`docs:`/`build:`; end messages with
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (or the implementing model's own line).

## Review Focus

1. **Reproducibility of the new packer:** two builds of the same project give the same `.pbo` and `.bisign`
   bytes (PBO timestamps 0, sorted entries, deterministic RSA PKCS#1 signature) — test in Task 2.
2. **An interrupted `naves navmesh` (Ctrl-C, crash) leaves `@naves_<world>` in the DayZ install:** the next run
   removes a symlink that points at this project's mod and goes on; a file, folder or symlink pointing elsewhere is
   never touched and is an error naming the path — test in Task 6.
3. **A partial mesh never replaces a good one:** failed or unfetched tiles write `<world>.partial.nm`, leave an
   existing `<world>.nm` untouched and fail with the first tiles named — test in Task 5.
4. **A game already running** (port 36533 taken) or no build yet (`build/@naves_<world>` or the mission missing)
   fails before anything is created, with the fix in the message — tests in Tasks 6 and 7.
5. **Old settings files** with `tools.dayzmake` or `tools.armake2`, and the `DAYZMAKE`/`ARMAKE2` variables, still
   load and are ignored; no tool lookup fails because dayzmake is not installed — test in Task 4.

---

### Task 1: dayzmake-core dependency; PNG → PAA in process

Model: **sonnet**.

**Files:**
- Modify: `crates/naves-export/Cargo.toml`, `Cargo.lock`, `deny.toml`
- Modify: `crates/naves-export/src/build.rs` (the `img2paa` block around the `pngs.par_iter()` loop, ~l. 380–420)
- Modify: `crates/naves-export/src/testing.rs` (drop the `img2paa` case of the fake script)
- Test: `crates/naves-export/tests/build.rs`

**Interfaces:**
- Produces: `fn png_to_paa(name: &Path, png: &[u8], format: PaaKind) -> Result<Vec<u8>, BuildError>` (private
  in `build.rs`), `enum PaaKind { Dxt1, Dxt5 }`; new `BuildError::Dayzmake { what: &'static str, path: PathBuf,
  source: std::io::Error }` (`#[error("{what} {}: {source}", path.display())]`), used by Tasks 2–3 too.

- [ ] **Step 1: Write the failing test** `terrain_textures_are_real_paas` in `tests/build.rs`: after a fake build,
  every `*.paa` in the packed world reads with `dayzmake_core::paa::Paa::read`; `s_*` files have compression
  `"DXT1"`, `m_*` `"DXT5"`; each has the taggs `AVGC` and `MAXC` (`Paa::tagg`), and masks have `FLAG`. Add
  `dayzmake-core` as a dev-dependency of the test (same crate entry suffices).
- [ ] **Step 2:** run `cargo test -p naves-export --test build terrain_textures_are_real_paas` → FAIL (the fake
  writes a stub PAA).
- [ ] **Step 3: Implement** `png_to_paa`: `image::load_from_memory` of the 0.24 crate re-exported as
  `dayzmake_core::image`, `Paa::from_dynamic_image(&img, PaaFormat::Dxt1|Dxt5)`, `Paa::write` into a `Vec`, then
  the existing `without_stray_lzo_bytes` and `with_colour_tags`; write the file atomically (temp + rename). The
  PNG bytes already in `layers.files` are used directly; the PNG need not be written first if nothing else reads it
  (check; keep it if the binarize sandbox needs it). Keep the `rayon` parallelism.
- [ ] **Step 4:** test passes; `make deny` passes (only the `image` duplicate warning).
- [ ] **Step 5: Commit** `build: dayzmake-core converts the terrain images to PAA` with the crate review and the
  `deny.toml` change in the message, plus `CHANGELOG.md` (Changed: images are converted in process).

### Task 2: keys, PBO packing and signing in process

Model: **sonnet**.

**Files:**
- Modify: `crates/naves-export/src/build.rs` (`ensure_keys`, `pack_mod`, the `dayzmake()` helper)
- Modify: `crates/naves-export/src/testing.rs` (drop `keygen` and `build` cases)
- Test: `crates/naves-export/tests/build.rs`

**Interfaces:**
- Consumes: `BuildError::Dayzmake` (Task 1).
- Produces: `fn ensure_keys(project_dir: &Path, world: &str) -> Result<(PathBuf, PathBuf), BuildError>` (no
  `tools`, no log); `fn pack_and_sign(pack_dir: &Path, pbo: &Path, private: &Path) -> Result<PathBuf, BuildError>`
  returning the `.bisign` path.

- [ ] **Step 1: Write failing tests** in `tests/build.rs`:
  - `the_mod_pbo_is_rapified_and_signed`: `PBO::read` of `build/@naves_<w>/addons/naves_<w>.pbo` succeeds; its
    header extension `prefix` is `naves\worlds\<w>`; it has `config.bin` and no `config.cpp`; the `.bisign` next to
    it verifies with the `.bikey` from `keys/` (`BIPublicKey::read`, `BISign::read`, `verify`).
  - `two_builds_give_the_same_pbo_and_signature`: build, copy the PBO and `.bisign` bytes, build again, `assert_eq!`.
  - `a_missing_key_pair_is_created_once`: first build creates `secrets/naves_<w>.{biprivatekey,bikey}`; a second
    build leaves their bytes unchanged.
  - adapt `a_project_navmesh_is_packed_where_the_config_points` and `without_a_navmesh_nothing_navmesh_is_packed`
    to list entries with `PBO::read` instead of the fake's text output.
- [ ] **Step 2:** run them → FAIL.
- [ ] **Step 3: Implement.** Keys: `BIPrivateKey::generate(1024, name)`, `to_public_key`, written atomically (same
  as dayzmake's `cmd_keygen`, `src/commands/sign.rs`). Pack: `PBO::from_directory(pack_dir, &PackOptions::build(),
  None, &mut Warnings::new())` (rapifies `config.cpp`, no binarizer); warnings go to `tracing::warn!`; write the PBO
  atomically; sign with `BIPrivateKey::sign(&pbo, BISignVersion::V3)` and write to `sign::signature_path(pbo, name)`
  (match what `dayzmake build -k` wrote: read `src/commands/pbo.rs`/`src/run.rs` in the dayzmake repo for the version
  and file name). Remove the `dayzmake()` helper and `DAYZMAKE_TIMEOUT` once nothing in `build.rs` uses them.
- [ ] **Step 4:** all `tests/build.rs` tests pass.
- [ ] **Step 5: Commit** `refactor: dayzmake-core packs and signs the mod` (+ CHANGELOG line if the message or log
  files change visibly: `build/logs/dayzmake-build.log` and `keygen.log` are gone — note it under Changed).

### Task 3: road catalog reads the game PBOs in process

Model: **sonnet**.

**Files:**
- Modify: `crates/naves-export/src/catalog_read.rs` (`dayzmake()`, `read_archive`, `parse_listing`,
  `unpack_roads`, `unpack_used`, `CatalogReadError`)
- Modify: `crates/naves-export/src/testing.rs`: the fake server's road PBOs become **real PBOs** packed with
  `PBO::from_directory(tree, &PackOptions::pack(), None, ..)` with the `prefix` header extension from the tree's
  `.prefix` file; drop the `inspect`, `cat`, `unpack` cases and the `dayzmake-calls.log`
- Test: `crates/naves-export/tests/catalog_read.rs`, `crates/naves-export/tests/build.rs:1309` (`unpack` error)

**Interfaces:**
- Produces: `unpack_roads(pbo: &Path, dest: &Path) -> Result<(), CatalogReadError>` and
  `unpack_used<'a>(server: &Path, models: impl IntoIterator<Item = &'a str>, dest: &Path)` (no `Tools`);
  `CatalogReadError::Dayzmake { what: &'static str, path: PathBuf, source: std::io::Error }` replaces the
  command/status/output variant; `NoPrefix` stays.

- [ ] **Step 1: Rewrite the tests** that asserted on the call log (`calls(&f)`): they assert the same behaviour on
  results instead — the catalog has the expected models with the expected prefixes; a model the catalog does not
  need is not in the cache; `unpack_used` with no Sakhal model creates no `dz/structures_sakhal` folder, with one it
  does; a PBO with an unsafe entry name (`../x.p3d`, `C:\x.p3d`) is skipped by `safe_entry`; a truncated PBO is an
  error naming the file (not a panic). New test `a_selected_entry_over_the_byte_limit_is_a_warning`.
- [ ] **Step 2:** run → FAIL (the fake no longer serves text listings).
- [ ] **Step 3: Implement:** listing and prefix from `PBO::read_headers` (`header_extension("prefix")`, entry
  names); the needed entries with `PBO::read_entries_tolerant(file, &wanted, &TolerantRead::new(MAX_BYTES))`
  (does not load the whole archive; pick `MAX_BYTES` = 512 MiB and note it); `SelectedEntry::is_over_limit` →
  warning; unpack with `dayzmake_core::pbo::unpack(file, out, &UnpackOptions::default(), &mut warnings)`. Remove
  `CAT_TIMEOUT`, `UNPACK_TIMEOUT`, `MAX_LISTING_BYTES`, `parse_listing` and its tests. If an API is missing for this
  (for example no prefix in `HeaderSummary`), file an issue in bzed/dayzmake and use the next best call.
- [ ] **Step 4:** `cargo test -p naves-export` passes.
- [ ] **Step 5: Commit** `refactor: dayzmake-core reads the road archives`.

### Task 4: remove the dayzmake tool

Model: **haiku** (mechanical; the list below is complete — `rg -n -i dayzmake` must show only dayzmake-core uses,
historic CHANGELOG lines and `docs/superpowers`/`docs/research` afterwards). Escalate to sonnet if a test fails for
a reason not on this list.

**Files:**
- Modify: `crates/naves-export/src/tools.rs` (remove `Tools::dayzmake`, `MIN_DAYZMAKE*`, `DAYZMAKE_URL`,
  `locate_dayzmake`, `check_version`, `parse_version`, `ToolError::DayzmakeTooOld`/`DayzmakeVersion`, the
  `DAYZMAKE` row of `TOOL_KEYS`, `with_legacy_env`; `tool_origins_with` has no dayzmake row)
- Modify: `crates/naves-core/src/prefs.rs` (`ToolPaths::dayzmake` stays deserializable for old files but is
  `#[serde(skip_serializing)]` and documented as ignored; the `armake2` migration only drops the key)
- Modify: `crates/naves/src/gui/prefs_window.rs` (no dayzmake row), `crates/naves/src/gui/worker.rs` (l. ~291, 297
  messages), `crates/naves/src/mcp.rs` (descriptions l. ~99, 284, 435, 584), `crates/naves/src/cli.rs` (help text if
  it names dayzmake)
- Modify: `crates/naves-export/src/testing.rs` (no `bin/dayzmake`, no `FakeEnv::dayzmake`), `generate.rs` doc l. 9,
  `catalog_read.rs` module docs
- Modify tests: `crates/naves-export/tests/tools_lookup.rs`, `crates/naves-core/tests/prefs.rs`,
  `crates/naves/tests/cli.rs` (l. ~404 uses `dayzmake inspect` → `PBO::read`), `crates/naves/tests/mcp.rs`,
  `crates/naves/src/gui/tests/{prefs_window,mod,roads,dock}.rs`; `crates/naves-formats/tests/odol.rs:442` (an
  ignored test running `dayzmake unpack` → `dayzmake_core::pbo::unpack`, dev-dependency)
- Modify docs: `docs/manual/src/{installation,settings,troubleshooting,gui,mcp,getting-started}.md`,
  `debian/control` (l. 19: "Building terrains needs DayZ Tools under Proton."), `CHANGELOG.md`

- [ ] **Step 1: Failing tests:** `tools_are_found_without_dayzmake` (`Tools::locate_with` with a `PATH` that has no
  dayzmake succeeds); `old_settings_with_dayzmake_and_armake2_still_load` (a `settings.toml` with
  `tools.dayzmake = "/x"` and `tools.armake2 = "/y"` loads; saving it writes neither key);
  `tool_origins_lists_no_dayzmake`.
- [ ] **Step 2:** run → FAIL (lookup fails without dayzmake).
- [ ] **Step 3:** remove everything listed; update the docs (installation no longer lists dayzmake as a
  requirement; settings and troubleshooting drop its rows; the `DAYZMAKE`/`ARMAKE2` variables are ignored).
- [ ] **Step 4:** full `make check` (wrapper from Global Constraints) passes;
  `rg --color=never -uu -n -i dayzmake crates docs/manual debian .github -g '!target'` shows only dayzmake-core.
- [ ] **Step 5: Commit** `feat: Náves no longer needs the dayzmake program` with CHANGELOG (Removed: the dayzmake
  requirement, `tools.dayzmake`, `DAYZMAKE`) and a packaging note under Unreleased for `debian/changelog`.

### Task 5: navmesh generation module

Model: **sonnet**.

**Files:**
- Create: `crates/naves-export/src/navmesh.rs` (register in `lib.rs`)
- Modify: `crates/naves-export/src/testing.rs`: `pub fn synthetic_capture(path: &Path) -> std::io::Result<()>`,
  a 2 × 2 world of 50 m tiles (flat walkable ground, `seed_position` in tile (0,0)) built from
  `WorldBlock::default()` + public fields, `TileInput`, `CaptureWriter::new`/`add` — modelled on dayzmake-core's
  `tests/support/navmesh_world.rs`, written fresh (only the flat ground, not the whole spec 13.2 world)
- Test: `crates/naves-export/tests/navmesh.rs`

**Interfaces:**
- Produces:
  ```rust
  pub enum NavmeshSource { Server(dayzmake_core::navmesh::client::Host), Capture(PathBuf) }
  pub struct NavmeshOptions { pub threads: Option<usize>,                // Default: None, None
      #[doc(hidden)] pub retry: Option<RetryPolicy> }
  pub struct NavmeshReport { pub path: PathBuf, pub built: usize, pub empty: usize, pub seconds: f64 }
  pub fn generate_navmesh(project_dir: &Path, world: &str, source: NavmeshSource,
      options: &NavmeshOptions, progress: &mut dyn FnMut(usize, usize)) -> Result<NavmeshReport, NavmeshError>;
  pub fn navmesh_path(project_dir: &Path, world: &str) -> PathBuf;     // <project>/navmesh/<world>.nm
  #[derive(thiserror::Error)] pub enum NavmeshError {
      Server { host: String, source: std::io::Error },    // "cannot reach the navmesh data server at {host}: ..."
      Incomplete { partial: PathBuf, failed: usize, first: Vec<(i32, i32, String)> }, // first ≤ 5
      Generate(std::io::Error), Io { path: PathBuf, source: std::io::Error },
      Start(String) /* used by Task 6 */ }
  ```

- [ ] **Step 1: Failing tests** in `tests/navmesh.rs`:
  - `a_capture_becomes_the_project_navmesh`: `generate_navmesh(.., Capture(synthetic), ..)` writes
    `navmesh/<w>.nm`; `NavMeshSet::read` of it has 4 tiles; `report.built == 4`; progress reached `(4, 4)`.
  - `the_mesh_does_not_depend_on_threads`: threads 1 and 3 give identical bytes.
  - `an_unreachable_server_fails_and_keeps_the_old_mesh`: write `navmesh/<w>.nm` = `b"OLD"`; `Server` on
    `127.0.0.1:<port of a listener bound then dropped>` with a short `RetryPolicy` (`NavmeshOptions::retry`) → `NavmeshError::Server`, file still
    `OLD`, no `.partial.nm`.
  - `a_bad_capture_is_an_error_not_a_panic`: a file of 100 random bytes (fixed seed) → `Err`.
- [ ] **Step 2:** run → FAIL (no module).
- [ ] **Step 3: Implement:** `Source::Servers(vec![host])` or `Source::CaptureFile(CaptureFile::open(..))` (range
  `None`, i.e. the capture's tiles); `generate(source, &opts, &mut |p| progress(p.done, p.total), &mut warnings)`
  with warnings to `tracing::warn!`; `report.stopped` → `Generate` error. Failed or fetch-error tiles → write
  `<w>.partial.nm` with `NavMeshSet::write_atomic` and return `Incomplete`; else `write_atomic` to `<w>.nm` and
  delete a stale `<w>.partial.nm`. A probe failure before any tile → `Server` (no file written).
- [ ] **Step 4:** tests pass (the `.nm`/`.dzgeo` parsers are dayzmake-core's and fuzzed there; no new fuzz target).
- [ ] **Step 5: Commit** `feat: naves-export generates a navmesh with dayzmake-core`.

### Task 6: the DayZDiag data server launcher

Model: **sonnet**. Step 0 needs the game: run it in a headless sway under `MemoryMax=8G`, follow
`AGENTS.md` *Testing a built map in the game* and the `dayz-dev` skill; remove every symlink you place.

**Files:**
- Create: `crates/naves-export/src/navmesh_server.rs`
- Modify: `crates/naves-export/src/bridge.rs` (make the Proton command builder and `stop_wineserver` `pub(crate)`
  for reuse; no behaviour change)
- Modify: `crates/naves-export/src/testing.rs` (fake `proton`: when argv contains `DayZDiag_x64.exe`, append argv
  and the `STEAM_COMPAT_*` env to `diag-calls.log`, write `profiles/fake.RPT`, then `exec tail -f /dev/null` so it
  runs until killed; a `FakeEnv` DayZ client folder with an empty `DayZDiag_x64.exe`)
- Test: `crates/naves-export/tests/navmesh_server.rs`

**Interfaces:**
- Consumes: `Tools` (`binarize.dayz`, `binarize.proton`, `binarize.steam_root`, `binarize.sandbox_base`),
  `NavmeshError::Start` (Task 5).
- Produces:
  ```rust
  pub struct ServerOptions { pub data_port: u16 /* 36533 */, pub start_timeout: Duration /* 300 s */,
                             pub poll: Duration /* 2 s */ }
  pub struct DataServer { /* child, sandbox TempDir, link guard */ }
  impl DataServer {
      pub fn start(tools: &Tools, project_dir: &Path, world: &str, opts: &ServerOptions) -> Result<DataServer, NavmeshError>;
      pub fn host(&self) -> Host;                                     // 127.0.0.1:<data_port>
      pub fn wait_ready(&mut self, probe: &mut dyn FnMut(&Host) -> std::io::Result<()>) -> Result<(), NavmeshError>;
      pub fn stop(self) -> Result<(), NavmeshError>;                  // Drop does the same, best effort
  }
  ```

- [ ] **Step 0 (manual spike, decides the Wine prefix):** with a built small test project, start DayZDiag as below
  once with a **fresh private prefix** in the sandbox (`STEAM_COMPAT_DATA_PATH=<sandbox>/prefix`, initialised like
  `binarize_proton` does) and probe it with `dayzmake navmesh probe` (the CLI is fine for a manual check). If it
  serves, the private prefix is the design (stop = kill child + `wineserver -k` on that prefix). If it does not,
  use the game's prefix `steamapps/compatdata/221100` as in the verified dayzmake guide, and stop by SIGTERM (then, after a deadline, SIGKILL) to the
  child's process group and to the `DayZDiag_x64.exe` it started only (found by PID, not `pkill -f`) — **never** `wineserver -k` on 221100. Write the
  result (command, log lines) into the commit message and `docs/formats/` (new `navmesh-data-server.md`).
- [ ] **Step 1: Failing tests:**
  - `start_links_the_mod_and_writes_the_server_tree`: after `start`, `<dayz>/@naves_<w>` is a symlink to
    `<project>/build/@naves_<w>`; the sandbox has `serverDZ_navmesh.cfg` with `verifySignatures = 0;`,
    `BattlEye = 0;`, `template="dayzOffline.<w>";`; `mpmissions/dayzOffline.<w>` is a full copy of
    `<project>/build/missions/dayzOffline.<w>` whose `init.c` is exactly `void main()\n{\n}\n`; `diag-calls.log`
    has `-server -startNavmeshDataServer`, `-mod=@naves_<w>`, `Z:\`-style `-config=`, `-mission=`, `-profiles=`, and
    a `-port=` other than 2302.
  - `stop_removes_the_link_and_the_process`: after `stop`, the symlink is gone and the fake process has exited
    (poll with a deadline helper, no sleep).
  - `a_stale_link_to_this_mod_is_replaced` and `a_foreign_entry_is_never_touched`: a pre-existing symlink to this
    project's mod → start succeeds; a folder, a file, or a symlink elsewhere at `@naves_<w>` → `Start` error naming
    the path, the entry unchanged, no process started.
  - `a_busy_data_port_fails_before_anything_is_created`: bind a listener on an ephemeral port, pass it as
    `data_port` → `Start` error ("port … is in use; is DayZ already running?"), no link, no sandbox left.
  - `wait_ready_retries_until_the_probe_answers`: probe fails twice, then succeeds → `Ok`, three calls;
    `poll = 1 ms`.
  - `wait_ready_gives_up_and_names_the_log`: probe always fails, `start_timeout = 50 ms` → `Start` error that
    contains the last lines of `profiles/*.RPT`.
  - `an_exited_server_is_reported_at_once`: fake proton variant that exits 3 → `wait_ready` returns `Start` with
    the exit status before the timeout.
- [ ] **Step 2:** run → FAIL.
- [ ] **Step 3: Implement.** Port for `-port=`: the first of 2402, 2412, … 2492 where UDP and TCP on `p..=p+2`
  bind on 127.0.0.1. Paths for the Windows program: `Z:` + the absolute path with `\`; check names with the
  bridge's `check_name`. Env as in the dayzmake guide (`STEAM_COMPAT_CLIENT_INSTALL_PATH`, `STEAM_COMPAT_DATA_PATH`
  per Step 0, `STEAM_COMPAT_INSTALL_PATH=<dayz>`, `SteamAppId`/`SteamGameId`/`STEAM_COMPAT_APP_ID=221100`),
  `current_dir(<dayz>)`, stdin null, stdout/stderr to `<sandbox>/diag.log`. Windows: run `DayZDiag_x64.exe`
  directly (as `binarize_native` does), mod link via a directory junction is out of scope — return
  `Start("naves navmesh starts DayZDiag on Linux only; use --server")` on Windows (`cfg!(windows)`), and say so
  in the manual. The link guard removes the link on `stop` and `Drop` only if it is still our symlink.
- [ ] **Step 4:** tests pass.
- [ ] **Step 5: Commit** `feat: naves-export starts DayZDiag as a navmesh data server`.

### Task 7: `naves navmesh` command and documentation

Model: **sonnet**.

**Files:**
- Modify: `crates/naves/src/cli.rs` (new subcommand), `crates/naves/src/main.rs` if dispatch lives there
- Modify: `docs/manual/src/navmesh.md` (rewrite), `docs/manual/src/cli.md`, `docs/manual/src/getting-started.md`
  (l. ~71, ~607 links still right), `docs/manual/src/troubleshooting.md` (new entries), `CHANGELOG.md`,
  `AGENTS.md` (see Step 3), `docs/superpowers/specs/2026-11-04-naves-design.md` l. 92 (navmesh line: dayzmake-core)
- Test: `crates/naves/tests/cli.rs`

**Interfaces:**
- Consumes: `generate_navmesh`, `NavmeshSource`, `NavmeshOptions`, `DataServer`, `ServerOptions`,
  `tools::locate_preferring`.
- Produces (CLI):
  ```text
  naves navmesh <DIR> [--server <HOST[:PORT]>] [--capture <FILE.dzgeo>] [--threads <N>] [--start-timeout <SECS>]
  ```
  `--server` and `--capture` conflict; without both, Náves starts DayZDiag (Task 6), waits, generates, stops.
  Progress lines `navmesh: <done>/<total> tiles` on stderr at most once a second; final line
  `wrote <path> (<built> tiles, <empty> empty, <s> s)`. Next step hint: `run naves build <DIR> to pack it`.

- [ ] **Step 1: Failing CLI tests** (`assert_cmd`, temp `HOME`/`XDG_*`, `FakeEnv::env`):
  - `navmesh_from_a_capture_writes_the_project_mesh` (synthetic capture → exit 0, file exists, stdout names it);
  - `navmesh_without_a_build_says_to_build_first` (no `build/@naves_<w>` → exit ≠ 0, stderr contains
    `naves build`, nothing linked);
  - `navmesh_with_an_unreachable_server_fails` (`--server 127.0.0.1:<closed>` → message names the host);
  - `navmesh_start_timeout_cleans_up` (fake env, `--start-timeout 1` → exit ≠ 0 within the test's deadline, the
    fake DayZ folder has no `@naves_<w>` afterwards, no fake process left);
  - `server_and_capture_conflict` (clap error);
  - `help_for_navmesh_matches_the_manual`: the option names of `naves navmesh --help` all appear in
    `docs/manual/src/cli.md`.
- [ ] **Step 2:** run → FAIL.
- [ ] **Step 3: Implement** the subcommand; the MCP and GUI entry points are out of scope (listed as follow-ups in
  the summary). `AGENTS.md`, project ground rules, the sentence "The `naves` program and the automated tests only
  read the installs" gets: "— except `naves navmesh`, which places the symlink `@naves_<world>` in the DayZ client
  install while DayZDiag runs and removes it afterwards (maintainer decision 2026-11-28)."
  Manual `navmesh.md`: one command (`naves navmesh <project>`), what it does (window may appear, needs DayZ client
  + Proton, about 20 tiles/s, one game at a time, port 36533), `--server` for a hand-started or remote DayZDiag
  (keep the start recipe there), `--capture`, the partial-mesh rule, the stale-link recovery, Linux only for the
  automatic start; no flare-navgen. CHANGELOG Added: `naves navmesh`.
- [ ] **Step 4:** `make check` (wrapper) passes, coverage ≥ 85 %; `make docs` passes.
- [ ] **Step 5: Commit** `feat: naves navmesh builds the navigation mesh with DayZDiag and dayzmake-core`.

### Task 8: game check (manual) and branch finish

Model: **sonnet** for the game run (follow the `dayz-dev` skill and `AGENTS.md` *Testing a built map in the game*;
headless sway; server `MemoryMax=8G`; never change client settings; remove links). The orchestrator (Opus) reviews.

- [ ] **Step 1:** build a small test project (`naves new … --extent 2000`, `naves build`), run
  `naves navmesh <project>` in the headless sway; it must finish, write the `.nm`, stop DayZDiag and remove the link
  (`ls -la <DayZ>/@naves_*` shows nothing; `pgrep -af '[D]ayZDiag'` prints nothing).
- [ ] **Step 2:** `naves build <project>` again; start a test DayZ server (own ports, `-mod=@naves_<w>` in the
  server tree) and check its RPT has no `Unable to load navmesh`; stop it.
- [ ] **Step 3:** the dayzmake CLI is not on `PATH` for the whole check (`env PATH=…` without `~/.bin`), proving
  Náves no longer needs it.
- [ ] **Step 4:** note the results (tiles, time, log lines) in the PR description; file bzed/dayzmake issues for
  anything missing in dayzmake-core found along the way.
- [ ] **Step 5:** `make check` (wrapper) once more on the branch head; then superpowers:finishing-a-development-branch
  (standing OK: finished, reviewed, green branches are merged to `main` and pushed).

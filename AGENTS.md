<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# AGENTS.md

Rules for anyone (human or agent) changing Náves. Náves turns OSM and DEM data into DayZ terrains; the
design is in `docs/superpowers/specs/2026-11-04-naves-design.md`, the background research in
`docs/research/`. Read the design before larger changes.

## Repository layout

| Path | Content |
|---|---|
| `crates/naves-formats` | pure file formats (no I/O policy, no game logic) |
| `crates/naves-core` | project, DEM, projection, heightmap, generation |
| `crates/naves-export` | terrain layers, config, mission, binarize bridge, packing |
| `crates/naves` | the binary: CLI, GUI, MCP server |
| `docs/manual/` | user documentation (mdBook), published to GitHub Pages |
| `docs/superpowers/` | design specs and implementation plans |
| `docs/research/` | research notes |
| `debian/` | Debian packaging (native package, trixie) |

Toolchain: Rust **1.95**, edition 2024, pinned in `rust-toolchain.toml`. `Cargo.lock` is committed and CI
builds with `--locked`.

## Project ground rules

- **No game data in the repo**, not even in test fixtures. Tests use synthetic data and openly licensed
  data (attribution in `tests/fixtures/README.md`). Game files are read from the user's install at run time.
- **Using the DayZ client, DayZ Server and DayZ Tools is allowed in every way** for development and manual
  testing: run them, read from them, start servers and clients, link a mod into an install when a test needs
  it. Prefer your own folders (a server tree, a mod folder passed with `-mod=`) over changes inside an
  install, never edit or delete game files, and remove what you linked when you are done. The `naves`
  program and the automated tests only read the installs (see *Tests*).
- **Clean-room:** no code from BI tools. Formats come from public documentation and from observing game
  files and tool output on our own inputs.
- Temporary and build folders go under `/tmp` or `/var/tmp` (`tempfile` crate or `mktemp -d`), never
  under `$HOME`.
- Local DayZ test servers never use the default ports.
- Linux first; everything except clearly marked platform code must also build on Windows.
- **Reuse before you write:** check for an existing crate before implementing anything, and review it
  (security, maintenance, our standards) before using it. See *Dependencies* below; this is mandatory.

## Tests

- **Test first.** A new feature or bug fix comes with a test that fails without it (red, then green).
- Test behaviour through the public API of a crate: integration tests in `crates/*/tests/`. Unit tests
  in `#[cfg(test)] mod tests` are for private helpers that hold real logic, not for every function.
- Tests are **deterministic**:
  - no `std::thread::sleep` / `tokio::time::sleep` to wait for state; poll with a deadline helper and a
    generous timeout, or inject a clock (`tokio::time::pause` in async tests);
  - random input uses a fixed seed; property tests (`proptest`) keep their regression files in the repo;
  - no network access.
- Tests write only under a `tempfile::TempDir`. They never touch `$HOME`, the user's config, a Steam
  directory or a running server. CLI tests (`assert_cmd`) set `HOME`, `XDG_CONFIG_HOME`,
  `XDG_CACHE_HOME` and `XDG_DATA_HOME` to temp dirs.
- Code that calls external tools (armake2, Proton, `binarize.exe`, DayZ) is tested in CI against fakes
  (a stub script on `PATH`, a trait seam). Tests that need the real tools or a DayZ install are
  `#[ignore = "needs <tool>"]` and run explicitly; they are not a substitute for the faked tests.
- Parsers for untrusted input (every file format, OSM, DEM, project files, MCP requests) get a fuzz
  target (see `make fuzz`). The fuzz corpus seeds also run as ordinary tests in `make test`.

## Coverage

- Total line coverage stays **at or above 85 %** (`COVERAGE_MIN` in the `Makefile`, measured with
  `cargo llvm-cov`). A change must not lower it.
- CI uploads the report to **Codecov** with `codecov/codecov-action` **v7 or newer**. `codecov.yml`:
  - `project`: `target: auto`, `threshold: 0%`. A pull request must not lower total coverage at all
    (and `make coverage-check` still enforces the 85 % floor);
  - `patch`: `target: auto`, `threshold: 5%`. The changed lines may fall slightly below the base
    coverage, but no further.
- Never lower the threshold, and never exclude code (`--ignore-filename-regex`, `#[coverage(off)]`,
  moving code into untested crates) to reach it.

## Safe Rust

These apply to all crates; the lints are configured once in `[workspace.lints]` in the root `Cargo.toml`
and each crate has `lints.workspace = true`.

**Lints**
- Strict by design: `cargo clippy` runs with `-D warnings`; `clippy::all` and `clippy::pedantic` are
  enabled. Don't relax the workspace lint set to get a change through.
- Silence a lint locally with `#[expect(lint, reason = "...")]`, not `#[allow]`
  (`clippy::allow_attributes_without_reason` is on). Never silence a lint crate-wide to make CI pass.

**`unsafe`**
- `unsafe_code = "forbid"`. If a crate ever really needs `unsafe` (FFI, a measured hot path), that needs
  the maintainer's approval first, then: its own module, a `// SAFETY:` comment on every block, and
  tests under Miri (`make miri`).

**Untrusted input never panics**
- All file contents, OSM, DEM, project files and MCP/CLI input are untrusted. Library code returns
  errors instead of panicking: no `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!` or unchecked
  slice indexing on such data (`clippy::unwrap_used`, `expect_used`, `panic`, `indexing_slicing` are
  denied in library crates; `clippy.toml` allows `unwrap`/`expect` in tests).
- Sizes and offsets read from files use checked arithmetic (`checked_add`, `checked_mul`) and
  `TryFrom` conversions. No truncating or sign-changing `as` casts (`cast_possible_truncation`,
  `cast_sign_loss`, `cast_possible_wrap` denied). The release profile keeps `overflow-checks = true`.
- Never allocate from a length field before checking it against the remaining input and a sane
  limit (`Vec::with_capacity(len_from_file)` is a bug). Stream large files instead of reading them whole.
- Floating point: no `==` on floats (`clippy::float_cmp`); check DEM no-data and NaN on input; sort
  with `total_cmp`.

**Files, paths and processes**
- Archive entry names (PBO and others) are validated before use: reject absolute paths, drive letters,
  `..` components and empty names; normalise `\` separators; only then join to an output directory.
- Write project files and build output atomically (temp file in the same directory, then rename).
- Run external tools with `std::process::Command` and an argument list, never through a shell
  (`sh -c`, `cmd /c`). Check the exit status, put stderr into the error, and give long-running tools
  (Proton, binarize) a timeout.
- Check for missing external tools up front and name the tool and the environment variable to set.

**Errors and output**
- Library crates define error types with `thiserror` and carry context (file, offset, object).
  `anyhow` is for the `naves` binary only.
- Don't swallow errors: `let _ = fallible()` and `.ok()` on a `Result` need a comment saying why.
- Libraries log with `tracing`, never `println!`/`eprintln!` (`clippy::print_stdout`,
  `print_stderr` denied outside the binary).
- **Same input, same output:** builds are byte-for-byte reproducible. No `HashMap`/`HashSet`
  iteration order in output (use `BTreeMap`, `IndexMap` or sort first), RNG seeds live in the project,
  no timestamps in generated files unless a format requires them.

**Concurrency**
- No `static mut`, no global mutable state; pass state explicitly. Prefer `rayon` and channels to
  shared locks.
- In async code (MCP server): don't hold a `std::sync::Mutex` across `.await`, and move blocking or
  CPU-heavy work to `spawn_blocking`.

**Secrets**
- Signing keys (`*.biprivatekey`) and other secrets never go into the repo, test fixtures, logs or
  error messages. `secrets/` is in `.gitignore`.

**Dependencies**
- **Look for a crate first (strict).** Before implementing a feature, parser, algorithm or helper, check
  whether an appropriate crate already exists (crates.io, lib.rs, RustSec, `cargo search`). Writing it
  ourselves is only allowed when no suitable crate exists or every candidate fails the review below; the
  commit message says which crates were considered and why they were rejected.
- **Review every crate before using it (strict).** Read its source, not just its README, and check:
  - *security*: open RustSec advisories and past CVEs, how it handles untrusted input (panics,
    unchecked indexing, allocation from length fields, unbounded recursion), every `unsafe` block is
    justified and sound, no build scripts or proc macros doing network access or other surprises;
  - *maintenance*: recent releases and commits, issues and pull requests get answered, not deprecated,
    archived or yanked, more than one maintainer or a healthy bus factor, sane release history;
  - *our standards*: has tests (ideally fuzzing for parsers), returns errors instead of panicking, builds
    on Linux and Windows with Rust 1.95, deterministic output, a licence on the `deny.toml` allow-list,
    and a dependency tree that is reasonable for what it gives us.
  A crate that fails the review is not used; if it is the only option, ask the maintainer first. The
  review result goes into the commit message that adds the crate.
- Add dependencies with `cargo add` (latest stable), with only the features needed. No git or path
  dependencies outside the workspace.
- Every new dependency is justified in the commit message. `cargo deny` checks licences against the
  AGPL-compatible allow-list in `deny.toml`, RustSec advisories, duplicates and sources; run
  `make deny` after any `Cargo.toml` change.

## Documentation

- User documentation lives in `docs/manual/` (mdBook) and is published to GitHub Pages: installation,
  getting started, CLI reference, the GUI, project files, troubleshooting and what the project is.
- **A commit that changes behaviour, the CLI, the GUI, config or project files updates the user
  documentation in the same commit.**
- Public items in library crates have rustdoc comments; `make doc` builds them with warnings as errors.
- CLI help (`clap`) and `docs/manual/` must agree; change both together.

## User interface

- Every UI task (GUI layout, widgets, map view, dialogs, the manual's look, any web page) starts by
  invoking the `/frontend-design:frontend-design` skill and follows its guidance. This applies to the
  `egui` GUI too, not only to web pages.

## Changelogs

- Every user-visible change (feature, fix, behaviour or config change, project-file format change,
  packaging, security fix) adds a line to `CHANGELOG.md` under `## [Unreleased]`
  ([Keep a Changelog](https://keepachangelog.com/) format), in the same commit. Pure refactors, tests and
  CI-internal changes do not.
- A release moves `Unreleased` to `## [x.y.z] - YYYY-MM-DD`, sets `version = "x.y.z"` in
  `[workspace.package]` and adds the same version to `debian/changelog` (`dch -v x.y.z`, distribution
  `trixie`) in the release commit. The tag is `vx.y.z`; `release.yml` fails if the tag, the workspace
  version, the top of `debian/changelog` and the `CHANGELOG.md` section disagree.
- Packaging changes (`debian/`, CI packaging jobs) also need a `debian/changelog` entry at the next
  release; collect them under `Unreleased`.

## Commits

- One logical change per commit, with `make check` passing.
- Conventional prefixes: `feat:`, `fix:`, `test:`, `docs:`, `build:`, `ci:`, `refactor:`, `chore:`.
- The commit includes its tests, the documentation update and the changelog line it needs.

## Before pushing

Run `make check`. It must pass, because CI runs the same steps and a push that fails them is wasted:

| Step | Target | Notes |
|---|---|---|
| formatting | `make fmt-check` | fix with `make fmt` (`cargo fmt`) |
| lint | `make lint` | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| rustdoc | `make doc` | `cargo doc --no-deps` with `RUSTDOCFLAGS="-D warnings"` |
| licences | `make reuse` | every new file needs an SPDX header (or a `REUSE.toml` entry) |
| dependencies | `make deny` | `cargo deny check`: licences, advisories, bans, sources |
| tests | `make test` | `cargo test --workspace --all-targets --locked` plus doctests; includes fuzz seeds |
| fuzzing | `make fuzz` | `cargo +nightly fuzz`, 10 s per target |
| coverage | `make coverage-check` | `cargo llvm-cov`; **total coverage must stay at or above 85 %** |
| user docs | `make docs` | `mdbook build docs/manual`, broken links fail the build |

CI additionally runs **CodeQL** (`.github/workflows/codeql.yml`, languages `rust` and `actions`) on
every push and pull request to `main` and weekly. New CodeQL alerts block the merge; fix them, or
dismiss them in GitHub with a written reason. Never just ignore them.

Also, when the change touches them:

- `debian/` or packaging: `dpkg-buildpackage -us -uc -b` and `lintian --fail-on error,warning`, as the
  `deb` CI job does. That job runs in `debian:trixie`, installs the pinned toolchain with rustup
  (trixie's rustc is older than 1.95) and builds offline from vendored crates.
- `unsafe` code: `make miri`.

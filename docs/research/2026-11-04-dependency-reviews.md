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

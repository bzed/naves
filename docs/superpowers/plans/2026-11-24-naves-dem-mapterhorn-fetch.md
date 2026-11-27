<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Naves dem fetch (Mapterhorn bare-earth DEM) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `naves dem fetch` downloads the bare-earth height model (DTM) for a project's map window from Mapterhorn's Terrarium tiles, writes it as one Float32 EPSG:4326 GeoTIFF that today's `sources.dem` reads unchanged, and lists it in `naves.toml`.

**Architecture:** One new module `naves-export/src/dem_fetch.rs`, a Float32 sibling of the existing GeoTIFF writer, and a thin CLI subcommand, all mapping the proven pattern of `imagery_fetch.rs` (ureq agent, injectable source, retries, atomic writes, toml upsert). `naves build` is untouched: it reads the produced GeoTIFF with the existing `dem.rs` reader.

**Tech Stack:** Rust (workspace, edition 2024), `image` 0.25 with the `webp` feature (pure-Rust `image-webp` 0.2.4, already in `Cargo.lock`; Mapterhorn tiles are lossless VP8L), `ureq` 3.5.1 with rustls, `tiff` 0.11 with deflate, rayon.

**Spec:** `docs/research/2026-11-23-1m-dem-sources.md` (§2 the service, §4 ingest path 1, §6 risks) and the spike answers `.superpowers/sdd/2026-11-24-naves-dem-mapterhorn-fetch/spike/answers.md` (measured values, verdict). All numbers below are measured, not estimated.

**Revision 2026-11-27 (review before implementation).** Changed against the first draft: the window comes from
`MapGeo::window` (the build's own projection, honours `scale.horizontal`) instead of a hand formula; the size
guard grows the pixel from the real grid (the UTM-rotated window is not square), not from `extent/8192`;
resampling works in bands of tile rows so memory stays near 150 MB instead of 2+ GB for decoded tiles; 404s
are cached as markers so re-runs never re-request them; `RealTiles` is tested against a local server; a fuzz
target for the tile decoder; the fixture's licence (REUSE, `tests/fixtures/README.md`); `docs/manual/src/cli.md`
and a CLI test; Claude commit trailers.

## Global Constraints

- Workspace: Rust 1.99, edition 2024. `unsafe_code = "forbid"`, `missing_docs = "warn"` (every public item needs a doc comment with an `# Errors` section where fallible), clippy `pedantic` and the workspace denials (`cast_possible_truncation`, `cast_sign_loss`, `cast_possible_wrap`, `float_cmp`, `indexing_slicing`, `unwrap_used`, `expect_used`, `panic`…): follow the lints of the surrounding code (`#[expect(…, reason = "…")]` where a cast is proven safe, as in `imagery_fetch.rs`); no `unwrap`/`expect` outside tests.
- Every new file starts with the house header: `SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>` and `SPDX-License-Identifier: AGPL-3.0-or-later`.
- No new crates. The only dependency change is adding `"webp"` to the `image` feature list in `crates/naves-export/Cargo.toml` (it enables `image-webp` 0.2.4, already in the lock file for another dependent; run `make deny`). The commit message of Task 3 says so.
- `cargo test` must pass with no network access. Tests use `TileGet` fakes, a local `TcpListener` server (as `tests/imagery_fetch.rs` does for `RealHttp`) and committed fixtures, never the real server.
- Test first (red, then green), tests through the public API in `crates/naves-export/tests/dem_fetch.rs`.
- Commit after every task, message style of the repo (`feat: …`, `test: …`, `docs: …`), ending with the trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. `make fmt-check` and `cargo clippy -p <crate> --all-targets -- -D warnings` clean before each commit.
- Cargo target dir: the worktree's own `target/`; wrap builds in `systemd-run --user --scope -p MemoryMax=24G`.
- Pin values (use exactly these constants in `dem_fetch.rs`):

| constant | value | meaning |
|---|---|---|
| `TILE_PX` | 512 | tile pixels per side |
| Terrarium | `h = R·256 + G + B/256 − 32768` | decode; R, G, B are `u8`, result `f64`, stored `f32` |
| `MARGIN_M` | 1000.0 | real metres around the map, `MapGeo::window(MARGIN_M * scale.horizontal)` |
| `DEFAULT_PIXEL_M` | 2.5 | target pixel in real metres |
| `MAX_PIXELS` | `1 << 26` | `dem.rs`'s limit, guarded before writing |
| `MAX_SIDE_PX` | 16_384 | `dem.rs`'s limit, guarded before writing |
| `TILE_JOBS` | 4 | parallel tile requests (be polite) |
| `MAX_TILE_BYTES` | `8 << 20` | answer size cap per tile |
| `Z_MIN`, `Z_MAX` | 12, 18 | fallback floor, probe ceiling; `Z_MIN` is also where Mapterhorn's world fallback (Copernicus GLO-30, 30 m, a DSM) is native |
| `TILE_BYTES_ESTIMATE` | 160_000 | bytes per tile for the size note (spike: 2116 tiles ≈ 340 MB) |
| retries | reuse `with_tries` (3 tries, one second more pause each) |
| cache | `dem/mapterhorn/{z}/{x}/{y}.webp`, a 404 as an empty `{y}.404` marker | inside the project |
| output file | `dem/mapterhorn_dtm.tif` | plus `dem/mapterhorn_dtm.attribution.txt` |

## Review Focus

1. **Server answers that are not tiles** (error pages, wrong dimensions, oversize): `terrarium` rejects anything that is not 512×512 8-bit RGB(A); `RealTiles` caps answers at `MAX_TILE_BYTES`; both surface as `FetchError` naming the tile, never a panic or a silent zero (Tasks 3, 5). A cached file that does not decode is an error naming the file (the user deletes it), not a silent re-fetch.
2. **Window a tile source cannot cover** (a 404 at every zoom): `resample` errors, naming the first missing tile id, instead of writing a hole that `build_heightmap` only reports at vertex level later (Tasks 4, 5).
3. **Maps bigger than `dem.rs` accepts**: the pixel grows until the real grid of the window fits `MAX_SIDE_PX` and `MAX_PIXELS`; the guard in `resample` errors clearly before anything is written (Tasks 4, 5).
4. **404 at native zoom is a fallback, not a failure** (macro-tile zoom borders, GLO-only regions): per-tile parent fallback down to `Z_MIN`; a sample whose bilinear quad touches a missing pixel falls back as a whole to the next coarser zoom (Tasks 3, 4, 5).
5. **Memory**: decoded tiles are held only for the band of output rows being sampled (about three tile rows), never the whole window (2116 tiles × 1 MiB = 2.2 GB for the example; more for bigger maps).
6. **Re-runs must not hammer the server or double-list the file**: cached tiles and cached 404s are never re-requested (counted in tests), and the `sources.dem` upsert never duplicates the entry (Task 5).
7. **A region Mapterhorn covers with nothing finer than the world fallback** (discovered zoom == `Z_MIN`): the fetch refuses by default, leaves `sources.dem` and the project untouched, tells the user that the current source stays in force — and that other sources exist but need their own implementation — and only `--force` writes the 30 m fallback (Tasks 5, 6).
8. **`sources.dem` order decides which file wins** (`Dem::height` picks the first tile containing the sample): the fetched file goes to the **front** of the list, ahead of 1° Copernicus tiles that would otherwise shadow it; an existing entry with the same path moves to the front, no entry is duplicated, no other entry is touched (Task 5).

---

### Task 1: Float32 GeoTIFF writer

**Files:**
- Modify: `crates/naves-export/src/geotiff_write.rs`
- Test: `crates/naves-export/tests/dem_fetch.rs` (new file, module doc + header like `tests/imagery_fetch.rs`)

**Interfaces:**

```rust
/// A one-band float raster: `width * height` heights, row 0 at the north.
#[derive(Debug, Clone, PartialEq)]
pub struct DemMosaic {
    /// Pixels per row.
    pub width: u32,
    /// Rows.
    pub height: u32,
    /// The heights.
    pub data: Vec<f32>,
}

/// Writes `dem`, which covers `bounds`, to `path` atomically … same contract as [`write_rgb`].
pub fn write_dem(path: &Path, dem: &DemMosaic, bounds: GeoWindow) -> Result<(), FetchError>
```

- [ ] **Step 1: Failing tests** in `tests/dem_fetch.rs`: (a) a 3×2 `DemMosaic` with distinct values written to a temp dir round-trips through `naves_core::dem::Dem::open`, and `Dem::height` at the first and last pixel *centre* (`west + 0.5·scale_x`, `north − 0.5·scale_y`, etc.) matches those values within `1e-4`; (b) a mosaic whose `data.len()` is not `width*height`, a zero-sized one, and bounds that are not a box (east ≤ west, non-finite) each error.
- [ ] **Step 2: Run to verify it fails** — `cargo test -p naves-export --test dem_fetch`.
- [ ] **Step 3: Implement** `DemMosaic` and `write_dem` next to `write_rgb`, sharing `write_rgb`'s checks and tags (factor a private helper for the size/box checks, the `scale`/`tie`/GeoKey tags `[1, 1, 0, 3, 1024, 0, 1, 2, 1025, 0, 1, 1, 2048, 0, 1, 4326]` and the temp-file-then-replace) rather than copying them; single band: `colortype::Gray32Float`, deflate, no `GDAL_NODATA` tag.
- [ ] **Step 4: Run the tests** — PASS.
- [ ] **Step 5: Commit** `feat: geotiff writer for one-band float rasters`.

### Task 2: Tile maths and the fetch window

**Files:**
- Create: `crates/naves-export/src/dem_fetch.rs` (module doc: what it fetches, from where, that nothing in `naves build` calls it — mirror the first paragraph of `imagery_fetch.rs`)
- Modify: `crates/naves-export/src/lib.rs` (add the module); `crates/naves-export/src/imagery_fetch.rs` (make `with_tries`, `transient`, `http_error`, `too_large`, `Limited` and `today` `pub(crate)`, no behavioural change)
- Test: `crates/naves-export/tests/dem_fetch.rs`

**Interfaces:**

```rust
/// The window a project's map occupies plus `MARGIN_M` real metres: `MapGeo::window(MARGIN_M * scale.horizontal)`.
pub fn fetch_window(project: &Project) -> Result<GeoWindow, FetchError>
/// Fractional world-pixel coordinates of a position at `z` (Web Mercator, `TILE_PX` pixels per tile):
/// `x = (lon + 180)/360 · 2^z · TILE_PX`, `y = (1 − asinh(tan φ)/π)/2 · 2^z · TILE_PX`.
pub fn world_px(lat: f64, lon: f64, z: u8) -> (f64, f64)
/// Tiles covering `window` at `z`, grown by one tile on each side (the bilinear halo) and clamped to
/// `0..2^z`: (x0, x1, y0, y1), inclusive.
pub fn tile_range(window: &GeoWindow, z: u8) -> (u32, u32, u32, u32)
```

- [ ] **Step 1: Failing tests**: (a) `fetch_window` of the example-like project (`Project::new("T", 47.571769, 12.946451, 16384.0)` with `scale.horizontal = 0.5`) equals `MapGeo::new(&p)?.window(500.0)?` exactly, and contains the window of `window(0.0)` with roughly 1 km real margin (north − north₀ ≈ 1000/111132 within 5 %); (b) `world_px` of the north-west corner of tile (16, 35130, 22901) (`lat = atan(sinh(π(1 − 2·22901/65536)))`, `lon = 35130/65536·360 − 180`) is `(35130·512, 22901·512)` within `1e-6`, and a point inside the Königssee fixture tile lands in it; (c) `tile_range` of a small window is the tiles of its corners grown by one, clamped at 0 and `2^z − 1` near the antimeridian/poles; for the example window at z16 the unhaloed count is about 2116 tiles (the spike's number; assert 2000..=2300).
- [ ] **Step 2: Verify failure, Step 3: implement** with `MARGIN_M`, `TILE_PX`, `Z_MIN`, `Z_MAX` consts (latitudes clamped to ±85.0511 before mercator), **Step 4: PASS**.
- [ ] **Step 5: Commit** `feat: fetch window and tile range maths for the dem source`.

### Task 3: Terrarium decode, the tile pyramid, the fuzz target

**Files:**
- Modify: `crates/naves-export/src/dem_fetch.rs`; `crates/naves-export/Cargo.toml` (`image` features: add `"webp"`)
- Create: `fuzz/fuzz_targets/dem_terrarium.rs`, `fuzz/corpus/dem_terrarium/{valid,truncated,small,garbage}`; add the `[[bin]]` to `fuzz/Cargo.toml`
- `REUSE.toml` and `tests/fixtures/README.md` already carry the fixture's licence and that of the `valid` and `truncated` seeds (copies of the fixture); the other seeds are the project's own
- Test: `crates/naves-export/tests/dem_fetch.rs`, `crates/naves-export/tests/fuzz_seeds.rs` (seed test like `imagery_tile_seeds`); fixture `tests/fixtures/dem/mapterhorn_z16_35130_22901.webp` (committed, a real z16 lake tile)

**Interfaces:**

```rust
/// Decodes one 512x512 Terrarium tile (webp bytes) to `TILE_PX*TILE_PX` heights, row 0 north.
pub fn terrarium(bytes: &[u8]) -> Result<Vec<f32>, FetchError>
/// Decoded tiles by (z, x, y), sampled with per-zoom fallback.
#[derive(Default)]
pub struct TilePyramid { /* BTreeMap<(u8, u32, u32), Vec<f32>> */ }
impl TilePyramid {
    pub fn insert(&mut self, z: u8, x: u32, y: u32, bytes: &[u8]) -> Result<(), FetchError>
    /// Bilinear height at a position; None if no zoom from `zoom_max` down to `Z_MIN` has all four pixels.
    pub fn sample(&self, lat: f64, lon: f64, zoom_max: u8) -> Option<f64>
}
```

- Pin `sample`: for `z` from `zoom_max` down to `Z_MIN`: `(wx, wy) = world_px(lat, lon, z)`, `fx = wx − 0.5`, `fy = wy − 0.5` (pixel centres, as `Dem::height`); the quad is pixels `(floor fx, floor fy)` … `(+1, +1)`, each looked up in the tile `(px / 512, py / 512)` at `z` (a quad may straddle tiles); if all four are present, interpolate and return; otherwise continue at `z − 1`.
- Pin `terrarium`: `image::load_from_memory_with_format(bytes, ImageFormat::WebP)` with `image::Limits` (max 512×512, alloc limit a few MiB); reject any other size; accept `Rgb8` and `Rgba8` (ignore alpha), reject other colour types.

- [ ] **Step 1: Failing tests**: (a) `terrarium` on the fixture: all heights within `[603.031, 603.407]`, pixel `(0,0)` is `603.15625`; (b) a synthetic tile encoded with `image::codecs::webp::WebPEncoder::new_lossless` from a height ramp (`v = h + 32768.0`; `r = floor(v / 256)`, `g = floor(v) mod 256`, `b = floor(fract(v) · 256)`) decodes back within `1/256`; (c) `terrarium` rejects a 256×256 webp and random bytes with `FetchError`; (d) `sample` uses the parent when the native-zoom quad is incomplete: a pyramid with one ramp tile at z16 and the z15 tile covering it, a point on the z16 tile's outer edge (whose quad needs the absent neighbour) returns the z15 value, a point in its interior the z16 value; (e) a point covered by nothing gives `None`.
- [ ] **Step 2: verify failures → Step 3: implement**, fuzz target (`terrarium(data)`, and on success one `TilePyramid` insert + sample), seeds, seed test (`valid` ok, others error) → **Step 4: PASS**, `make reuse` clean → **Step 5: commit** `feat: terrarium tile decode and sampling pyramid` (message: `image`'s `webp` feature enables `image-webp` 0.2.4, pure Rust, already in the lock; no new crate).

### Task 4: Resample the window in bands, guard the raster size

**Files:**
- Modify: `crates/naves-export/src/dem_fetch.rs`
- Test: `crates/naves-export/tests/dem_fetch.rs`

**Interfaces:**

```rust
/// Raw tile bytes by (z, x, y), as resampling reads them: the project's tile cache in production.
/// `Ok(None)`: the tile does not exist at that zoom.
pub trait TileStore: Sync {
    fn load(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, FetchError>;
}
/// Columns and rows of the grid of `window` at `pixel_m` real metres, growing `pixel_m` until it fits
/// `MAX_SIDE_PX` and `MAX_PIXELS`: (columns, rows, pixel_m). `requested` (default `DEFAULT_PIXEL_M`) is a floor.
pub fn dem_grid(window: &GeoWindow, requested: Option<f64>) -> Result<(u32, u32, f64), FetchError>
/// Samples the tiles at the centre of every pixel of the grid of `window` …
pub fn resample(store: &dyn TileStore, window: &GeoWindow, pixel_m: f64, zoom_max: u8)
    -> Result<DemMosaic, FetchError>
```

- Grid: at the window's middle latitude `φm`, `pixel_x = pixel_m / (111_320 · cos φm)` degrees, `pixel_y = pixel_m / 111_132`; `width = ceil((east − west)/pixel_x)`, `height = ceil((north − south)/pixel_y)`; the written bounds are `west, north` and `west + width·pixel_x`, `north − height·pixel_y` (so pixels stay square-ish and exact). `dem_grid` grows the pixel like `imagery_fetch::picture_grid` (factor from the longest side and from `sqrt(w·h / MAX_PIXELS)`, times 1.001, up to four rounds) and errors if it still does not fit or `requested` is not finite and positive. `resample` re-checks the limits and errors before allocating.
- Bands: output rows are processed in bands, each the rows whose centres fall in one tile row `ty` at `zoom_max`; for a band, a fresh `TilePyramid` is filled through `store.load` with tiles `x0..=x1` of rows `ty − 1 ..= ty + 1` at `zoom_max` and, for every position missing there, its ancestors down to `Z_MIN` (each tile loaded once per band); the band's rows are sampled in parallel with rayon and appended in order (deterministic); the pyramid is dropped before the next band. A pixel that `sample` cannot serve fails the whole call with `FetchError::Other` naming the pixel's tile at `zoom_max` (`z/x/y`) and its latitude and longitude.

- [ ] **Step 1: Failing tests** (a `TileStore` fake from a `BTreeMap`, counting loads): (a) one synthetic linear-ramp tile resamples a small window inside it to a mosaic whose corner pixels match the analytic ramp at those pixel centres within `1e-3`; (b) a window spanning two tile rows is continuous across the band border (no jump larger than the ramp's step); (c) the fake reports that no tile was loaded more than three times (bands overlap by at most a row on each side); (d) `dem_grid`: the example window gives `2.5`; a 24.5 km-extent project's window gives a pixel above 2.5 whose grid fits both limits and is within 1 % of the smallest that fits; `Some(0.0)`, `Some(NaN)` error; (e) `resample` with a pixel so small that the grid exceeds `MAX_PIXELS` errors without calling `store.load`; (f) a store missing a tile at every zoom errors naming that tile.
- [ ] **Step 2: verify failures → Step 3: implement → Step 4: PASS → Step 5: commit** `feat: dem window resampling in bands with raster size guards`.

### Task 5: The fetch itself — network, cache, discovery, toml

**Files:**
- Modify: `crates/naves-export/src/dem_fetch.rs`
- Test: `crates/naves-export/tests/dem_fetch.rs`

**Interfaces:**

```rust
/// A remote Terrarium-encoded XYZ tile source. Sources with the same shape plug in as new
/// constants; sources with other wire formats (COG, .hgt, WMS) get their own fetch later —
/// the imagery catalogue (`naves_core::imagery::catalog`) is the pattern that grows into.
pub struct TerrariumSource {
    /// The id used in file names and messages.
    pub id: &'static str,
    /// The human name.
    pub name: &'static str,
    /// The XYZ template with `{z}`, `{x}`, `{y}`.
    pub tile_url: &'static str,
    /// Where the source list and licences live.
    pub attribution_url: &'static str,
}

/// The only source v1 speaks.
pub const MAPTERHORN: TerrariumSource = TerrariumSource {
    id: "mapterhorn",
    name: "Mapterhorn",
    tile_url: "https://tiles.mapterhorn.com/{z}/{x}/{y}.webp",
    attribution_url: "https://mapterhorn.com/attribution",
};

/// Where tiles come from. `Ok(None)` is a 404 — absent at this zoom.
pub trait TileGet: Sync {
    fn tile(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, FetchError>;
}

/// Production source: a ureq agent like `RealHttp` (https only, UA `naves/<version>`, timeouts:
/// connect 30 s, global 2 min per tile), 404 → `Ok(None)`, other failures through `with_tries`,
/// answers capped at `MAX_TILE_BYTES` with `Limited`.
pub struct RealTiles { /* agent + &'static TerrariumSource */ }
impl RealTiles {
    pub fn new(source: &'static TerrariumSource) -> RealTiles
    /// For tests against a local server, which speaks plain `http`.
    #[cfg(any(test, feature = "test-util"))]
    pub fn plain_for_tests(source: &'static TerrariumSource) -> RealTiles
}

#[derive(Debug, Clone, Default)]
pub struct DemFetchRequest {
    pub zoom: Option<u8>,
    pub pixel_m: Option<f64>,
    /// Write the 30 m world fallback even when nothing finer covers the project.
    pub force: bool,
} // None/false mean "discover"/"2.5 m"/"refuse GLO-only"

/// What a fetch will do, for the user before anything is downloaded: zoom, tile count, how many are
/// cached, the expected download size (`TILE_BYTES_ESTIMATE` per missing tile), the grid and pixel.
pub fn describe_dem_fetch(project_dir: &Path, source: &'static TerrariumSource, request: &DemFetchRequest,
                          tiles: &dyn TileGet) -> Result<String, FetchError>

/// Fetches the bare-earth DEM of a project into `dem/<id>_dtm.tif` and lists it in `naves.toml`.
/// Sequence: discover zoom (probes, cached) -> refuse GLO-only unless forced -> fetch missing tiles
/// (TILE_JOBS parallel via a rayon pool, each written to the cache atomically) -> resample from the
/// cache -> write_dem atomically -> attribution sidecar -> upsert `sources.dem` -> `project.save`.
/// Returns the paths listed in `naves.toml` (relative, `/`-separated).
pub fn fetch_dem(project_dir: &Path, source: &'static TerrariumSource, request: &DemFetchRequest,
                 tiles: &dyn TileGet, note: &dyn Fn(&str)) -> Result<Vec<PathBuf>, FetchError>
```

- The cache is the `TileStore` for `resample` and sits in front of `TileGet`: a tile is requested only when neither `{y}.webp` nor `{y}.404` exists; a 200 is written as `{y}.webp` (temp file then rename) and a 404 as an empty `{y}.404`. A cached `.webp` that does not decode fails with an error naming the file.
- Zoom discovery when `request.zoom` is `None`: probe the tiles of the 4 window corners and the centre, `Z_MAX` down to `Z_MIN`; the highest `z` at which all five exist is the target zoom (through the cache). With `request.zoom = Some(z)`, `z` must be within `Z_MIN..=Z_MAX`.
- Per tile of `tile_range(window, zoom)`: fetch at the target zoom; on `None` fetch its parent, down to `Z_MIN` (parents shared by several children are fetched once). Count the tiles that are served only at `Z_MIN`: a non-zero count goes into a note line (`3 of 2116 tiles only have the 30 m world fallback`) and the sidecar; a tile absent at every zoom is left to `resample`'s error.
- The GLO-only refusal: when the discovered zoom is `Z_MIN` and `request.force` is `false`, `fetch_dem` writes nothing but the probe cache, passes this exact line to `note` and returns `FetchError::Other` with the same text: `Mapterhorn has nothing finer than Copernicus GLO-30 here; sources.dem is unchanged. Other sources exist (national portals, Sonny's 20 m, FABDEM if non-commercial), but none is implemented — TileGet sources slot into dem_fetch.rs, other formats need their own fetcher. Pass --force to fetch the 30 m world fallback anyway.`
- The upsert: `dem/<id>_dtm.tif` goes to the **front** of `sources.dem` (`Dem::height` picks the first tile that contains the sample; behind a 1° Copernicus tile it would be dead); an existing entry with that path is removed first (so it moves to the front, once); no other entry is touched. The project is re-loaded just before the upsert (as `imagery_fetch::fetch` does) and saved last; any earlier error leaves `naves.toml` byte-identical.
- The sidecar, exact copy (only the date, window, zoom and the optional fallback line vary; the date is `today()` as `YYYY-MM-DD`):

```text
Terrain elevation: Mapterhorn — https://mapterhorn.com
Source list and licences: https://mapterhorn.com/attribution
Retrieved: <YYYY-MM-DD>  Window: <south> <west> to <north> <east>  Native zoom: up to z<zoom>
The national data is bare-earth (CC BY 4.0, Etalab 2.0, DL-DE, OGL where present); the world fallback is Copernicus GLO-30.
[<n> of <m> tiles only have the 30 m world fallback.]
```

- [ ] **Step 1: Failing tests** (fake `TileGet`s serving synthetic ramp tiles, temp project dirs written with `Project::save`, no network): (a) end to end — z16 everywhere → `dem/mapterhorn_dtm.tif` that `Dem::open` reads, `height` at the map centre matching the ramp; (b) the cache — a second `fetch_dem` with a counting fake requests nothing, also when the first run met 404s; (c) the upsert — `sources.dem` lists `dem/mapterhorn_dtm.tif` exactly once after two runs, at position 0, and a pre-existing Copernicus entry is kept behind it; (d) 404 at z16, 200 at z15 → completes, values from z15; (e) the sidecar's four fixed lines; (f) a fake that answers only at `Z_MIN` → error with the refusal text (also seen by `note`), no `.tif`, `naves.toml` byte-identical; the same fake with `force: true` completes at z12; (g) a mixed fake (all probes at z16, one tile only at z12) completes, a note line contains `1 of <n> tiles only have the 30 m world fallback` and so does the sidecar; (h) a fake that answers garbage for one tile → error naming `z/x/y`, `naves.toml` unchanged; (i) `RealTiles::plain_for_tests` against a local `TcpListener` server (pattern of `tests/imagery_fetch.rs`): 200 with bytes → `Some`, 404 → `None`, an answer above `MAX_TILE_BYTES` → error saying "larger than"; (j) `describe_dem_fetch` names the zoom, the tile count and a size in MB.
- [ ] **Step 2: verify failures → Step 3: implement → Step 4: PASS** (`cargo test -p naves-export --test dem_fetch`) → **Step 5: commit** `feat: naves dem fetch, the mapterhorn dem source`.

### Task 6: CLI wiring and documentation

**Files:**
- Modify: `crates/naves/src/cli.rs` (new `Dem { action: DemAction }` subcommand mirroring `Imagery`: `dem fetch [DIR] --zoom <Z> --pixel-m <M> --force`, help text in the same voice; the `naves new` advice line becomes "Next: `naves dem fetch` writes a bare-earth height model into the project, or add your own files to `sources.dem` in `naves.toml`. Then run `naves build`.")
- Modify: `docs/manual/src/cli.md` (the `naves dem fetch` section: arguments, options, what it writes, the refusal), `docs/manual/src/getting-started.md` (after the Copernicus download paragraph: one short alternative paragraph naming `naves dem fetch`, about 340 MB for the example, the attribution sidecar, that the map remembers the file in `naves.toml`, and the fallback rules below), `docs/manual/src/project-file.md` (`dem` source: mention the fetched `dem/mapterhorn_dtm.tif` and that the first file wins), `docs/manual/src/credits.md` if it lists data sources, `CHANGELOG.md` (one line under `## [Unreleased]` / Added).
- Test: `crates/naves/tests/cli.rs` — `naves dem fetch --help` names `--zoom`, `--pixel-m`, `--force`; `naves new` prints the new advice line; `naves dem fetch` on a folder without `naves.toml` fails with a message naming the folder (no network reached).

- [ ] **Step 1: Failing CLI tests, then wire the subcommand**: `describe_dem_fetch` → `note` (the Mapterhorn credit, the zoom, tile count and expected MB, then this fixed reminder: `Other elevation sources exist (national portals, Sonny's 20 m, FABDEM if non-commercial); none is implemented. Terrarium XYZ sources slot into dem_fetch.rs, other formats need their own fetcher.`); then `fetch_dem(dir, &MAPTERHORN, &DemFetchRequest { zoom, pixel_m, force }, &RealTiles::new(&MAPTERHORN), &|line| output::note(line))`; a `result` per listed path, then "naves.toml lists the files; `naves build` uses them.".
- [ ] **Step 2: Manual**, including in `getting-started.md`: "Where Mapterhorn has nothing finer than Copernicus GLO-30 — most of the world outside Europe and the US — the command refuses and `sources.dem` keeps the Copernicus files; `--force` fetches the 30 m fallback. Tiles that only have the fallback inside a covered region are named in the output and in `dem/mapterhorn_dtm.attribution.txt`." Also that `earthworks_capped` may rise on a sharper DEM (real gorge walls), which is expected.
- [ ] **Step 3: `make check`** green (fmt, lint, doc, reuse, deny, test, fuzz, coverage ≥ 85 %, docs).
- [ ] **Step 4: Acceptance on the real example (network, one-off, not a test):** copy the example's untracked data from the main worktree (`examples/berchtesgaden/{dem,osm,landcover,imagery}`) into this worktree, then `naves dem fetch examples/berchtesgaden` (expect ~2 min, ~340 MB into the cache) and `naves build examples/berchtesgaden`; compare `build/report.json` with `/var/tmp/naves-mapterhorn-spike/report_mapterhorn.json`: `road_misses_terrain` about 3597 (the margin rule differs a little from the spike), `road_grade` about 9, `earthworks_capped` about 129. Note: the base of this branch has newer road code than the spike's `4dccd7c` binary only if it was rebased; compare against a GLO-30 build of the same binary as well. Nothing from `examples/berchtesgaden/dem/` is committed.
- [ ] **Step 5: Commit** `feat: naves dem fetch subcommand and manual chapter`.

## Notes for the implementer

- `naves build`, `dem.rs` and the height pipeline are **untouched** by this plan; if a task seems to need them, stop and ask — the point of path 1 is that the build reads an ordinary GeoTIFF.
- **Extensibility is a seam, not a framework.** A new Terrarium-XYZ source is a new `TerrariumSource` const plus tests. A source in another wire format (national COG over HTTP ranges, Sonny's `.hgt`, FABDEM zips) starts as a sibling fetcher — no plugin registry, the constant list is the catalogue.
- **The current source always stays available**: refusing GLO-only regions (and never touching `naves.toml` on any error) means `sources.dem` keeps working exactly as before; the fetched file is added at the front only on success.
- GUI and MCP access to the fetch are out of scope for v1 (imagery has both; DEM fetch can follow the same way later).
- The spike's scratch lives in `/var/tmp/naves-mapterhorn-spike/` while this machine stays up; it is evidence, not source.
- The measure of success is the recorded report deltas in the spike answers: road findings broadly down, `earthworks_capped` up because real gorge walls replace the DSM's smoothness. That is expected, disclosed in the manual, and **not** to be hidden by pre-smoothing the raster.

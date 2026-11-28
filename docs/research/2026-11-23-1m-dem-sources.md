<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# One-metre DEM sources: Mapterhorn and the rest

Explored on 2026-11-23, read-only, prompted by the evaluation of `glandais/osm-route-details`
whose exploration notes (`docs/exploration/04-sources-elevation.md`, `05-mapterhorn-emprises-et-usage.md`,
researched 2026-10-30 by its author against the live service) name Mapterhorn as the best current
open elevation aggregator. Question: **can the build read a 1 m DTM, from Mapterhorn or elsewhere, and
what would it buy us?** Everything below was re-verified where it was cheap, from the live servers,
the `mapterhorn/mapterhorn` repository (v0.0.13 plus the working tree of 2026-11-23), `sonny.4lima.de`,
crates.io and `attribution.json`/`download_urls.json` as published today. Provenance marks:
**[m]** measured today, **[r]** read in the upstream repositories, **[o]** only reported by the
osm-route-details notes, **[i]** inference to confirm.

## Short answer

Yes. Mapterhorn already serves the Bavarian DGM1 (1 m, CC BY 4.0) over Berchtesgaden as
lossless-WebP Terrarium tiles at zoom 16 (0.8 m/px), and the example's window is 2116 tiles ≈
340 MB at that zoom [m, spike of 2026-11-24]. The smaller ingest path needs no new reader at all:
a pre-fetch step turns the tiles into the Float32/EPSG:4326 GeoTIFFs that `sources.dem` accepts
today, keeping `naves build` offline. Whether anything of it is built is decided after a spike
measures what the 1 m DTM changes against the GLO-30 baseline of the FABDEM note (it has: see the
update section at the end). Nothing about the road planning changes: it already samples the
heightmap, and the heightmap simply becomes better ground.

## 1. What the build samples today, and what "1 m" buys

- `Dem::height` ([dem.rs](../crates/naves-core/src/dem.rs)) is a bilinear sample at a WGS84 position
  over single-band Float32 GeoTIFF tiles in EPSG:4326 — today only Copernicus GLO-30 (a **DSM**:
  roofs and canopy included) [r].
- `build_heightmap` ([heightmap.rs](../crates/naves-core/src/heightmap.rs)) samples the DEM once per
  heightmap vertex. On the example (extent 16 384 m, `scale.horizontal = 0.5`, `terrain_size 2048`)
  the vertex pitch is 8.0 real m [m]. **The map can never use a source finer than about half that
  pitch; "1 m resolution" is not for the extra pixels.** What it buys:
  1. **bare earth instead of canopy**: the diameter of the problem the FABDEM note measured
     (bare-earth GLO-30 variants cut `road_misses_terrain` by 6 to 9 %, `road_grade` by 6 to 13 %,
     `road_not_drivable` by 18 to 21 %, most in towns);
  2. **30× finer source cells**: GLO-30 cells (~23×30 m) are interpolated across 6.25 m samples;
     a 1 m DTM gives the true ground at every sample — stream beds, road cuts, ditches;
  3. **ground truth for the road planning's measures**: the drawn grade (mean and the reported 20 m
     window) and the feasibility band then measure the ground, not a 30 m DSM smear — the cleaner
     the terrain, the less the planner works against fiction.
- The zoom that matches the sampling is low: at lat 47.6, Mapterhorn's 512 px tiles give z14 ≈ 3.2 m,
  z15 ≈ 1.6 m, z16 ≈ 0.80 m per pixel [m, from tile geometry]; z16 is far finer than the 4 m the
  8 m pitch needs. The native zoom of the source is still the right one to read: below it Mapterhorn
  throws height precision away (see §2) and the aggregation overviews average.

## 2. Mapterhorn, verified

What it is: an NLnet-funded, Cloudflare-hosted aggregator of **151 open national DEM sources**
plus Copernicus GLO-30 as the world fallback, served as XYZ `tiles.mapterhorn.com/{z}/{x}/{y}.webp`
and as PMTiles archives for download; code BSD-3, 453 stars, releases roughly every 1 to 2 months,
current v0.0.13 (2026-09-11), one active maintainer (O. Wipfli) with repo rights shared with two
others [r,m,o].

| Aspect | Finding | Marks |
|---|---|---|
| Encoding | Terrarium `h = R·256 + G + B/256 − 32768`, 512 px tiles served as `image/webp`; `tilejson.json` confirms `encoding: terrarium`, `tileSize: 512` | [m,r] |
| Compression | **Lossless WebP** (`imagecodecs.webp_encode(rgb, lossless=True)` in `pipelines/utils.py`) — no lossy VP8 needed | [r] |
| Height quantization | Per zoom, `factor = min(1, 2^(19−z)/256)`: z16 → 3.1 cm, z15 → 6.25 cm, z12 → 0.5 m. Read at the source's native zoom | [r] |
| National sources | **DTM only** (bare earth); the world fallback GLO-30 is a DSM | [r,o] |
| Maxzoom rule | Smallest zoom whose Mercator pixel is finer than the source's; GLO-30 is pinned to z12. Above a tile's maxzoom the server 404s ("Tile not found", 14 B) — the client must climb to the parent | [r,m] |
| Merge | Sources sorted by maxzoom (ties by name), nodata filled by the next; a ~150 m Gaussian blur along seams (`MACROTILE_BUFFER_3857 = 150`) | [r] |
| Vertical datums | **No conversion**: each source keeps its national datum (DHHN2016, NGF-IGN69, NAP, …), GLO-30 is EGM2008. A per-source constant offset is invisible in a game map; a *mix* inside one map is a step, see §6 | [o,r] |
| Versioning | `download_urls.json` (version 0.0.13 today) lists every archive with md5; `attribution.json` the same for the raw source tarballs | [m] |
| Coverage index | `coverage-index.pmtiles` (1.5 MB, z4–z12: which sources are in a macrotile) and `coverage.pmtiles` (342 MB MVT: per-source polygons) to detect what a map actually touched | [o] |
| Usage terms | No published quota/ToS; the maintainer's answer in issue #317: tiles may be stored and redistributed indefinitely with attribution, self-host via `pmtiles extract` recommended for non-interactive use; 7-day `cache-control` | [o] |

Coverage of the inventory [m, from `attribution.json`]: 74 sources at 1 m, 19 at 0.5 m, 2 at 0.25 m
(Baden-Württemberg, Canton Zürich), the rest 2 m to 30 m. One metre or better covers: all 16 German
Länder (incl. **`debayern`, DGM1 1 m, CC BY 4.0**, producer Bayerische Vermessungsverwaltung),
France (LiDAR HD 0.5 m plus RGE ALTI 1 m incl. overseas), Switzerland 0.5 m, Austria 1 m
(sections), Benelux, Iberia 0.5 m, Nordics, PL/CZ/SK/SI/RO/EE, UK 1 m, US 3DEP 1 m (`us1*`),
Japan 1 m, NZ 1 m. Where none of these exists, Mapterhorn serves GLO-30 — i.e. exactly what we
read today.

Measured at Berchtesgaden (47.585 N, 12.965 E) [m]:

- z12 … z16 return `200 image/webp`, 129 to 165 kB per tile; z17 and above `404`. So Bavaria's DGM1
  is served at its native zoom 16 (0.80 m/px, 412 m per tile), right where the maxzoom rule puts a
  1 m source.
- A row of 17 z16 tiles along the map's western edge averaged 90 kB; the map's own window
  (16.4 km + margin) is 2116 z16 tiles ≈ 340 MB [m, spike].
- The z13+ archive covering the area is `6-34-22.pmtiles`, 166 GB (z13–z17), named by its z6 cell;
  the z0–z12 world base `planet.pmtiles` is 356 GB (z0-12) [m, from `download_urls.json`]. The
  archives are therefore only usable through `pmtiles extract --bbox`, through HTTP range requests
  (PMTiles v3 dirs), or through the 100 MB of XYZ tiles above — never as a whole download.
- The raw-source alternative `https://download.mapterhorn.com/sources/debayern.tar` (84 GB, the whole
  state's DGM1 GeoTIFFs) exists per source [m] — per-map that granularity is wrong.

## 3. The other candidates

- **Sonny's LiDAR DTMs of Europe** ([sonny.4lima.de](https://sonny.4lima.de/)) [m]: a one-person
  recompilation of national LiDAR DTMs, CC BY 4.0. Products: 1″ and 3″ `.hgt` (≈ 20×30 m and 60×90 m;
  **`.hgt` stores whole metres — 1 m vertical steps**, poor for grade windows), 20 m and 50 m
  GeoTIFF (0.1 m vertical), and 0.5″/10 m for Austria and Switzerland only. Updated fitfully
  (Germany v3b 2024-09, EUROPE v25 2025-08), hosted on Google Drive, and the author explicitly
  refuses finer grids ("download them from the original data providers"). **Not a 1 m source.**
  Its niche for us would be the 20 m GeoTIFFs as a bare-earth fallback for countries where Mapterhorn
  has nothing finer — the same countries are mostly 1 m-covered already, so this stays a footnote.
- **Direct national portals** (e.g. Bavaria's own `download1.bayernwolke.de/a/dgm/dgm1/<km>.tif`
  per-kilometre tiles: openly fetchable, 3.3 MB, Float32, CC BY 4.0 [m]) give the unquantised data in
  its native CRS (and vertical datum) — but then Náves must reproject per file (the `imagery` reader
  and its `proj4rs` `RasterSampler` already show the pattern) and every project's UX is a different
  portal. Mapterhorn has already paid exactly that cost for 151 sources; taking the raw road here
  buys at most 3 cm of quantization and some raster noise.
- **FABDEM, GEDTM30, ASTER**: already measured and judged in the
  [FABDEM note](2026-11-14-fabdem.md) — FABDEM is licence-blocked (NC, SA), the model-derived DTMs
  are noisy in mountains; nothing here changes that verdict. They are also still 1″ products with
  GLO-30's detail level, not 1 m class.

## 4. The ingest path

Two shapes fit the current architecture, both keeping `naves build` offline, deterministic and
`unsafe_code = forbid`:

1. **Repack offline, read as today (smaller step).** A helper (or a documented one-off with
   `pmtiles extract` + `gdal_translate`) fetches the native-zoom tiles of the project's window —
   2116 tiles ≈ 340 MB for the example [m, spike], one request per tile — decodes
   Terrarium, and writes standard single-band Float32 **EPSG:4326** GeoTIFFs that `sources.dem`
   accepts unchanged. At the example's 8 m sampling pitch, packing the z16 level at 2.5 m/px
   (≈7354² px ≈ 54 Mpx for the window plus margin) stays under `dem.rs`'s `MAX_PIXELS` (2^26)
   with room to spare. The decode runs in whatever tooling the helper likes (GDAL, ImageMagick);
   the workspace gains nothing.
2. **A tile reader in `naves-core` (larger step, no GDAL).** A `Dem` variant over PMTiles/XYZ
   Terrarium: crate `pmtiles` 0.28.1 (PMTiles v3, sync and async backends [m, crates.io]) and
   `image-webp` 0.2.4 (**pure Rust** WebP, lossless VP8L is what Mapterhorn encodes [m, r]) decode
   without C bindings. The per-position query stays `height(lat, lon)` bilinear as today; the
   reader climbs 404s to the parent tile and caches z/x/y on disk. This is the shape only if the
   repack proves annoying.

Determinism and attribution, either way: pin the mapterhorn version and the archive md5 (from
`download_urls.json`, as we pin an OSM extract), store the fetched data in the project, and record
in `report.json` the sources the map actually touched (from `coverage-index.pmtiles`, or simply the
entries of the zooms that were read). Licences are all attribution-class (CC BY 4.0, Etalab 2.0,
DL-DE, OGL…), commercial use fine — the same obligation class as the Copernicus attribution the
manual already requires, just a longer list.

## 5. What to expect of the result

- The FABDEM note already measured that DSM-vs-DTM matters on this example: with bare-earth
  30 m variants, `road_misses_terrain` fell 6 to 9 % (above 2 m: 21 to 28 %), `road_grade` 6 to
  13 %, `road_not_drivable` 18 to 21 % — and FABDEM/GE-DTM are *reconstructions* of the ground. A
  real LiDAR DTM at 1 m removes both the canopy bias and the reconstruction noise, and gives the
  road planning a terrain that its 2 m resamples, drawn grades and earthwork profiles measure on
  real ground.
- Water and bridges should improve for free: real stream beds instead of canopy-smeared ones for
  the dips and the water test, and bridge fitting that starts from the actual valley rather than a
  DSM deck ramp.
- This is expectation, not evidence [i]: the 1 m DTM has not been through the build. That is the
  spike's job.

## 6. Risks and open points

- **LiDAR speckle** in 1 m DTMs (classification noise, courtyards, scree): sampling at 6.25 m
  bilinearly may or may not be enough smoothing; the repack can average a 10×10 window [open].
- **Nodata holes** inside a 1 m region (France's ZICAD flight zones are the named case) fall back
  to GLO-30 inside the map — a visible 150 m-blurred seam of a different datum; the helper should
  check the coverage index and report [open].
- **Cross-border maps** mix datums with no conversion (§2); within one country the offset is
  constant and irrelevant [i].
- **`image-webp` on actual Mapterhorn tiles** (lossless VP8L with the extended transforms): decode
  one tile and confirm — it is the only technical unknown in path 2 [open].
- **`pmtiles` crate internals** (no `unsafe`, or feature-gated): audit like any dependency review
  if path 2 is taken [open]; the Go `pmtiles` CLI (asset naming tripped the first download attempt
  here) is the fallback extractor for path 1 [open, not retried].
- **Longevity**: one main maintainer, Cloudflare-sponsored hosting, no SLA — but data is meant to
  be extracted and kept, md5-pinned, which is exactly the offline-first posture above [o].

## 7. Verdict

Mapterhorn is the right vehicle for 1 m-class DEMs: right data model (bare-earth national DTMs),
right licence class (CC BY-style, unlike FABDEM), right scale of access (a 12 800 m map is 100 MB of
tiles or a small `pmtiles extract` out of a 166 GB regional archive — never a terabyte download),
and a decode path that fits the workspace's constraints (lossless WebP, pure-Rust crates exist).
Recommended next step, in the order of the FABDEM note: pull the z16 window for
`examples/berchtesgaden`, repack it to Float32/EPSG:4326 via path 1, rebuild the example, and
compare `report.json` (road findings, tight lengths, planning outcomes) against both GLO-30 and the
FABDEM figures already on file. The decision that follows — reader in `naves-core`, or repack
helper only, or nothing if the gains are small — is taken on those numbers, not on this note.

## 8. Update: the spike ran (2026-11-23)

The verdict's step was executed in the worktree `naves-spike-dem` (branch `spike/dem-mapterhorn`,
binary of `4dccd7c`; answers under `.superpowers/sdd/2026-11-24-naves-dem-mapterhorn-fetch/spike/answers.md`).
The example's extent is 16 384 m (it grew on `fbcc981`; the FABDEM note's 12.8 km is outdated), so
the vertex pitch is 8.0 real m and figures above were corrected. The fetch (window + ~1 km margin)
is 2116 z16 tiles ≈ 340 MB; the repack (gdal, 2.5 m/px bilinear, 54.7 Mpx Float32/EPSG:4326
deflate, 114 MB) passed `Dem::open` unchanged. The build comparison, GLO-30 → Mapterhorn DGM1:
`road_misses_terrain` 3752 → 3597, `road_not_drivable` 141 → 125 (3140 m → 2749 m), `road_grade`
14 → 9, `road_conflict` 797 → 735, `largest_miss_m` 7.48 → 6.62, `max_grade` 0.767 → 0.694; and
`earthworks_capped` 83 → 129 with max cut/fill 48/31 m → 78/64 m — the LiDAR ground has gorge and
bluff walls the DSM smeared away, and the height solve refuses (correctly) to plane over them.
Verdict: **build the fetch feature**; the plan is
`docs/superpowers/plans/2026-11-24-naves-dem-mapterhorn-fetch.md`.

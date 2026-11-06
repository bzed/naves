<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Imagery

The satellite image that the game shows from a distance can be made from real satellite or aerial images.
`naves imagery list` shows which sources exist for the area of your project and what each one allows. Images are
ordinary GeoTIFF files in the project folder, listed in `naves.toml`; `naves build` reads only those files and
never touches the network.

```sh
naves imagery list myproject
```

```text
Imagery for Berchtesgaden (12.88..13.05 E, 47.53..47.64 N)

ID                  COVERS     PIXEL  LICENCE                           NOTES
bayern-dop40        whole map  0.4 m  CC BY 4.0                         bounding box of Bavaria: ...
eox-cloudless-2017  whole map  10 m   CC BY 4.0                         cloud-free mosaic of 2017; ...
sentinel2-scenes    whole map  10 m   Copernicus Sentinel data, ...     single dates: ...
eox-cloudless-2025  whole map  10 m   CC BY-NC-SA 4.0  NON-COMMERCIAL   cloud-free mosaic of 2025; ...
```

## Choosing

| Source | Covers | Pixel | Licence | Use it when |
|---|---|---|---|---|
| `sentinel2-scenes` | the world | 10 m | Copernicus Sentinel data, free and open: commercial use allowed | you want a recent image; Náves picks the clearest scenes |
| `eox-cloudless-2017` | the world | 10 m | CC BY 4.0 | you want one cloud-free image without picking a date |
| `eox-cloudless-2025` | the world | 10 m | CC BY-NC-SA 4.0, **non-commercial** | you want the newest cloud-free mosaic and the map is non-commercial anyway |
| `bayern-dop40` | Bavaria | 0.4 m | CC BY 4.0 | the map lies in Bavaria and you want sharp aerial photographs |

The order of the list is the order of preference: sources that cover the whole map first, then those that allow
commercial use, then the sharper ones. A bounding box is all the catalogue knows of an area: where a service has
nothing (outside Bavaria, say) it answers blank, which Náves treats as no data and fills from another source.

A source marked **NON-COMMERCIAL** can be used. Most DayZ maps are non-commercial, and the resulting map says
so: `ATTRIBUTION.txt` opens with **NON-COMMERCIAL USE ONLY**, the mod's overview in the launcher repeats it and
`naves build` warns. Whether you may publish or sell a map is your responsibility; Náves states the licences of
the images, it does not give legal advice. CC BY-NC-SA is also share-alike: images derived from such a source stay
under that licence, and `ATTRIBUTION.txt` says so.

Google, Bing, Mapbox and Esri imagery is not offered: as far as we know their terms do not allow it to end up in a
redistributed map. The evidence for every entry, with the publisher's terms and the date they were read, is in
`docs/research/2026-11-06-imagery-sources.md` in the source tree.

## Several sources

Files can overlap. Náves measures their colours against each other where they overlap and corrects the exposure
with a smooth gain, so that a difference in brightness between two flights does not show as a seam and the detail
stays untouched. A change that is real (a mown field, snow in one image) is not corrected. Where several images
cover a spot, the sharpest is drawn on top, equals in the order of the list, and every image fades in over a few land
cells at its edge. The image with the most coverage keeps its colours, so a Sentinel-2 image of the whole map
under aerial photos needs no setting. The [GUI](gui.md) and the `imagery` overlay of the [MCP server](mcp.md) show
the result before you build.

## Your own files

Any 8-bit RGB GeoTIFF works (a fourth band is read as transparency, a `NoData` value as no data), in EPSG:4326,
UTM (EPSG:326xx, 327xx, 258xx) or EPSG:3857. Convert other files with GDAL:

```sh
# another coordinate system: the UTM code of the map is in the message of the error
gdalwarp -t_srs EPSG:32633 in.tif imagery/mine.tif
# 16 bit, or separate band files
gdal_translate -ot Byte -scale in.tif imagery/mine.tif
# JPEG compression inside the TIFF
gdal_translate -co COMPRESS=DEFLATE in.tif imagery/mine.tif
# a file much sharper than the map, so that Náves can read an overview instead of every pixel
gdaladdo -r average imagery/mine.tif 2 4 8 16 32
```

List the file in `naves.toml` with its credit and licence; a licence with `NC` in it needs `non_commercial = true`:

```toml
[[sources.imagery]]
path = "imagery/mine.tif"
attribution = "Orthophotos © Example Survey 2025"
licence = "CC BY 4.0"
```

## Adding sources to the catalogue

`$XDG_CONFIG_HOME/naves/imagery-sources.toml` (default `~/.config/naves/imagery-sources.toml`) can add sources or
replace one by its `id`. The file is not part of a project on purpose: opening someone else's project must never
make Náves fetch from an address that the project chose. Only `https://` addresses are accepted. The format is the
one of the built-in list, `crates/naves-core/data/imagery-sources.toml`:

```toml
[[source]]
id = "my-wms"
name = "My WMS"
kind = "wms"            # or "stac"
url = "https://example.org/wms"
layer = "orthophotos"
max_px = 2000           # the largest image the service allows
coverage = [5.9, 45.8, 10.5, 47.8]   # west, south, east, north; leave out for the world
native_m = 0.25
licence = "CC BY 4.0"
attribution = "Orthophotos © Example Survey"
non_commercial = false
```

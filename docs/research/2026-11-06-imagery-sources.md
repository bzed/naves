<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Imagery sources checked by hand (2026-11-06)

What each entry of `crates/naves-core/data/imagery-sources.toml` rests on. Every request was made on 2026-11-06 with
the user agent `naves/0.1` and the box 47.52..47.64 °N, 12.88..13.05 °E (the Berchtesgaden example) at 2000 pixels.
An entry that failed a check was not added.

## Sentinel-2 scenes (`sentinel2-scenes`)

- **Endpoint.** STAC API `https://earth-search.aws.element84.com/v1`, collection `sentinel-2-c1-l2a`, asset `visual`
  (true colour, 8 bit). `POST /search` with `intersects` (a polygon of the box), `datetime`
  `2025-05-01T00:00:00Z/2025-09-30T23:59:59Z` and `sortby` `properties.eo:cloud_cover` ascending answers without an
  account: `S2C_T32TQT_20250919T101514_L2A` (0.0003 % cloud), `S2C_T33TUN_20250919T101514_L2A` (0.0006 %),
  `S2B_T33TUN_20250921T100350_L2A` (0.38 %). Every feature has a `geometry` polygon and the properties
  `eo:cloud_cover` and `s2:snow_ice_percentage`, so the search can also prefer scenes without snow.
- **Files.** The `visual` asset lives on `e84-earth-search-sentinel-data.s3.us-west-2.amazonaws.com` (the only host
  the catalogue allows), a cloud-optimised GeoTIFF of 10 980 × 10 980 px in the UTM zone of the tile with overviews
  and `NoData 0`; `gdal_translate` crops it from the URL with a few range requests.
- **Licence.** Copernicus Sentinel data, free and open, commercial use included; the credit is "Contains modified
  Copernicus Sentinel data {year}". Terms: the Legal Notice on the use of Copernicus Sentinel Data and Service
  Information, <https://sentinels.copernicus.eu/documents/247904/690755/Sentinel_Data_Legal_Notice>.

## EOxCloudless (`eox-cloudless-2017`, `eox-cloudless-2025`)

- **Endpoint.** WMS `https://tiles.maps.eox.at/wms`, layers `s2cloudless-<year>` (EPSG:4326, the world). The
  capabilities list the layer `s2cloudless` (2016) and `s2cloudless-2017` to `-2025`. A `GetMap` of
  `s2cloudless-2017` and of `s2cloudless-2025` (WMS 1.3.0, `crs=EPSG:4326`, `bbox` in latitude, longitude order,
  2000 × 1400 px, `format=image/png`) answers **JPEG** (`content-type: image/jpeg`, 475 kB and 637 kB), never PNG,
  and has no transparency; the file is read by its content, not by the header.
- **Licence, from the capabilities of the service (the `Abstract` of each layer).** 2016 (`s2cloudless`) and 2017:
  "released under Creative Commons Attribution 4.0 International License". 2018 to 2025 (checked: 2018, 2019,
  2020 to 2025): "released under Creative Commons Attribution-NonCommercial-ShareAlike 4.0 International License.
  For commercial usage please see https://cloudless.eox.at". This corrects the first notes of the plan, which said
  that only 2016 is CC BY: **2017 is too**, so the catalogue offers 2017 as the newest year that allows commercial
  use.
- **Credit, as the layers state it.** "EOxCloudless https://cloudless.eox.at by EOX IT Services GmbH (Contains
  modified Copernicus Sentinel data 2017)", and with 2025 for the 2025 layer. The 2018 layer says "2017 & 2018",
  which is why layers are not offered by a template.
- **Redistribution.** Modified imagery may be redistributed as part of a work with its licence stated; sub-licensing
  under your own terms is not allowed (the share-alike of the NC years). The attribution must be visible or "under
  credits, data sources, or as a part of metadata": `ATTRIBUTION.txt` and `mod.cpp` do that.

## Bavaria aerial photographs (`bayern-dop40`)

- **Endpoint.** WMS `https://geoservices.bayern.de/od/wms/dop/v1/dop40`, layer `by_dop40c` (colour; `by_dop40g` is
  grey). Capabilities: title "WMS BY DOP40", ground pixel 40 cm, maximum 6000 × 6000 px, `Fees` "kostenfrei",
  `AccessConstraints` "CC BY 4.0 vgl. https://creativecommons.org/licenses/by/4.0/deed.de", CRS list with EPSG:4326, 25832,
  25833 and 3857, bounding box 8.891584..14.193229 °E, 46.980003..50.715246 °N. The `GetMap` (2000 × 1400 px,
  `transparent=true`) answers **RGBA PNG** that is **transparent outside Bavaria**, which Náves treats as no data.
- **Open question for the maintainer.** The service states the licence and an attribution link
  (<https://geoservices.bayern.de>), but the pages of the Landesamt für Digitalisierung, Breitband und Vermessung that
  I could read did not give the wording of the credit that the open-data terms prescribe. The catalogue uses
  "Bayerische Vermessungsverwaltung, https://geoservices.bayern.de (DOP40)" with CC BY 4.0. Please check the wording
  against the terms of the open geodata before a release.

## Not in the catalogue

The first notes of the plan also named the NRW `wms_nw_dop`, swisstopo `ch.swisstopo.swissimage`, IGN
`ORTHOIMAGERY.ORTHOPHOTOS`, PDOK `luchtfotorgb` and USGS NAIP. They were **not checked in this pass** (licence terms
and credit wording not read from the publishers), so they are not offered. The user file `imagery-sources.toml`
can add any of them. USGS NAIP gave an XML error on the first request (see the plan), so it needs a look in any case.

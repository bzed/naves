// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The imagery catalogue: what can be had for an area, with its licence.

#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::imagery::catalog::{
    CatalogError, CatalogSource, SourceKind, catalog, options_for, parse_sources,
};
use naves_core::landcover::GeoWindow;
use proptest::prelude::*;

fn window(south: f64, west: f64, north: f64, east: f64) -> GeoWindow {
    GeoWindow {
        south,
        west,
        north,
        east,
    }
}

/// The map of the Berchtesgaden example.
fn berchtesgaden() -> GeoWindow {
    window(47.527, 12.88, 47.643, 13.05)
}

fn ids(options: &[naves_core::imagery::catalog::ImageryOption]) -> Vec<&str> {
    options.iter().map(|o| o.source.id.as_str()).collect()
}

#[test]
fn builtin_catalogue_parses_and_is_valid() {
    let sources = catalog(None).unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for s in &sources {
        assert!(seen.insert(s.id.clone()), "duplicate id {}", s.id);
        assert!(s.native_m > 0.0 && s.native_m.is_finite(), "{}", s.id);
        assert!(
            !s.licence.is_empty() && !s.attribution.is_empty(),
            "{}",
            s.id
        );
        match &s.kind {
            SourceKind::Stac {
                url, asset_hosts, ..
            } => {
                assert!(url.starts_with("https://"), "{}: {url}", s.id);
                assert!(!asset_hosts.is_empty(), "{}", s.id);
            }
            SourceKind::Wms { url, max_px, .. } => {
                assert!(url.starts_with("https://"), "{}: {url}", s.id);
                assert!(*max_px >= 64, "{}", s.id);
            }
        }
        let says_nc = s
            .licence
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| w.eq_ignore_ascii_case("nc"));
        assert_eq!(says_nc, s.non_commercial, "{}: {}", s.id, s.licence);
    }
    for must in [
        "sentinel2-scenes",
        "eox-cloudless-2017",
        "eox-cloudless-2025",
        "bayern-dop40",
    ] {
        assert!(seen.contains(must), "{must} is part of the catalogue");
    }
    let by_id = |id: &str| sources.iter().find(|s| s.id == id).unwrap();
    assert!(by_id("eox-cloudless-2025").non_commercial);
    assert!(!by_id("eox-cloudless-2017").non_commercial);
    assert!(!by_id("sentinel2-scenes").non_commercial);
}

#[test]
fn options_are_filtered_by_area() {
    let sources = catalog(None).unwrap();
    let here = options_for(&sources, berchtesgaden());
    assert!(ids(&here).contains(&"bayern-dop40"), "{:?}", ids(&here));
    assert!(ids(&here).contains(&"sentinel2-scenes"));
    // Peru: Sentinel-2 and the world mosaics, not the Bavarian photographs
    let peru = options_for(&sources, window(-13.2, -72.1, -13.0, -71.9));
    assert!(!ids(&peru).contains(&"bayern-dop40"), "{:?}", ids(&peru));
    assert!(ids(&peru).contains(&"sentinel2-scenes"));
}

fn parse(text: &str) -> Vec<CatalogSource> {
    parse_sources(text).unwrap()
}

/// One source of a given kind of coverage, sharpness and licence.
fn entry(id: &str, native_m: f64, licence: &str, nc: bool, coverage: Option<[f64; 4]>) -> String {
    let coverage = coverage.map_or(String::new(), |c| format!("coverage = {c:?}\n"));
    format!(
        "[[source]]\nid = \"{id}\"\nname = \"{id}\"\nkind = \"wms\"\nurl = \"https://example.org/wms\"\nlayer = \"l\"\n\
         native_m = {native_m}\nlicence = \"{licence}\"\nattribution = \"A\"\nnon_commercial = {nc}\n{coverage}\n"
    )
}

#[test]
fn options_are_ordered() {
    let world = None;
    let half = Some([12.0, 47.0, 12.965, 48.0]);
    let text = [
        entry("zz-nc-sharp", 1.0, "CC BY-NC 4.0", true, world),
        entry("b-coarse", 10.0, "CC BY 4.0", false, world),
        entry("a-coarse", 10.0, "CC BY 4.0", false, world),
        entry("c-sharp", 0.5, "CC BY 4.0", false, world),
        entry("part", 0.1, "CC BY 4.0", false, half),
    ]
    .concat();
    let options = options_for(&parse(&text), berchtesgaden());
    // the whole map first; commercial use allowed first; sharper first; then the id
    assert_eq!(
        ids(&options),
        ["c-sharp", "a-coarse", "b-coarse", "zz-nc-sharp", "part"]
    );
}

#[test]
fn coverage_share_is_right() {
    // a box over the western half of the window (12.88..13.05 E)
    let text = entry(
        "half",
        1.0,
        "CC BY 4.0",
        false,
        Some([12.0, 47.0, 12.965, 48.0]),
    );
    let options = options_for(&parse(&text), berchtesgaden());
    assert_eq!(options.len(), 1);
    assert!(
        (options[0].coverage_share - 0.5).abs() < 1e-9,
        "{}",
        options[0].coverage_share
    );
    // outside the window: no option at all
    let far = entry(
        "far",
        1.0,
        "CC BY 4.0",
        false,
        Some([100.0, 0.0, 101.0, 1.0]),
    );
    assert!(options_for(&parse(&far), berchtesgaden()).is_empty());
}

#[test]
fn user_file_adds_and_replaces() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("imagery-sources.toml");
    let replaced = entry("bayern-dop40", 0.2, "CC BY 4.0", false, None)
        .replace("name = \"bayern-dop40\"", "name = \"My own Bavaria\"");
    std::fs::write(
        &file,
        format!("{replaced}{}", entry("my-wms", 3.0, "CC0", false, None)),
    )
    .unwrap();
    let builtin = catalog(None).unwrap();
    let merged = catalog(Some(&file)).unwrap();
    assert_eq!(merged.len(), builtin.len() + 1);
    assert!(merged.iter().any(|s| s.id == "my-wms"));
    let bavaria = merged.iter().find(|s| s.id == "bayern-dop40").unwrap();
    assert_eq!(bavaria.name, "My own Bavaria");
    // a missing user file is not an error
    assert_eq!(
        catalog(Some(&dir.path().join("nope.toml"))).unwrap().len(),
        builtin.len()
    );
}

#[test]
fn http_url_is_rejected() {
    let text = entry("plain", 1.0, "CC BY 4.0", false, None).replace("https://", "http://");
    let err = parse_sources(&text).unwrap_err();
    assert!(matches!(err, CatalogError::Invalid { .. }), "{err}");
    assert!(
        err.to_string().contains("plain") && err.to_string().contains("https"),
        "{err}"
    );
}

#[test]
fn bad_entries_are_rejected_with_their_id() {
    let good = entry("ok", 1.0, "CC BY 4.0", false, None);
    for (bad, why) in [
        (entry("../x", 1.0, "CC BY 4.0", false, None), "id"),
        (
            entry("nc", 1.0, "CC BY-NC 4.0", false, None),
            "non_commercial",
        ),
        (entry("zero", 0.0, "CC BY 4.0", false, None), "native_m"),
        (entry("nan", f64::NAN, "CC BY 4.0", false, None), "native_m"),
        (
            entry(
                "box",
                1.0,
                "CC BY 4.0",
                false,
                Some([12.0, 48.0, 11.0, 47.0]),
            ),
            "coverage",
        ),
        (good.clone(), "duplicate"),
    ] {
        let err = parse_sources(&format!("{good}{bad}"))
            .unwrap_err()
            .to_string();
        assert!(err.contains(why), "{why}: {err}");
    }
    assert!(parse_sources("not toml [[").is_err());
    assert!(parse_sources("").unwrap().is_empty());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Wherever the map is, some option covers all of it.
    #[test]
    fn every_area_has_an_option(
        south in -80.0_f64..80.0,
        west in -180.0_f64..179.0,
        height in 0.001_f64..2.0,
        width in 0.001_f64..2.0,
    ) {
        let sources = catalog(None).unwrap();
        let w = window(south, west, (south + height).min(84.0), (west + width).min(180.0));
        let options = options_for(&sources, w);
        prop_assert!(
            options.iter().any(|o| o.coverage_share > 0.999),
            "no option covers {w:?} whole: {:?}",
            ids(&options)
        );
    }
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Writes `sites.json` for a built project: the places `tools/ingame-check/` photographs. The sites come from
//! the project's `build/report.json` and from the road pieces as the build places them (it places them again
//! here, which needs the project's data and the DayZ tools, like `road-analysis`), plus fixed sites of the
//! Berchtesgaden example.
//!
//! `cargo run --release -p naves-export --example road-sites <project dir> <sites.json> [terrain_size]`

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a developer tool; it talks to its user"
)]

use std::collections::BTreeMap;
use std::path::Path;

use naves_core::dem::Dem;
use naves_core::features::FeatureKind;
use naves_core::features::road_features;
use naves_core::features::water_lines;
use naves_core::heightmap::{MapGeo, build_heightmap};
use naves_core::osm::read_ways;
use naves_core::project::Project;
use naves_core::surface::LoadedSources;
use naves_export::catalog_read::road_catalog;
use naves_export::generate::plan_roads;
use naves_export::sites::{Site, Terrain, pick_sites, yaw_near};
use naves_export::tools::Tools;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Fixed sites of the Berchtesgaden example: (category, latitude, longitude, note).
const FIXED: [(&str, f64, f64, &str); 4] = [
    ("fixed-roundabout", 47.626_173, 12.994_013, "the roundabout"),
    (
        "fixed-hammerstiel",
        47.604_126,
        12.949_617,
        "Hammerstielstrasse with the dirt track",
    ),
    (
        "fixed-bridge-kehlstein",
        47.631_554,
        13.047_906,
        "Kehlsteinstrasse bridge over a road",
    ),
    (
        "fixed-bridge-silberg",
        47.630_057,
        12.959_948,
        "Silbergstrasse bridge over a road",
    ),
];

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let (Some(dir), Some(out)) = (args.next(), args.next()) else {
        return Err("usage: road-sites <project dir> <sites.json> [terrain_size]".into());
    };
    let dir = Path::new(&dir);
    let mut project = Project::load(dir)?;
    if let Some(size) = args.next().and_then(|s| s.parse::<u32>().ok()) {
        project.grid.terrain_size = size;
        project.grid.land_size = size / 8;
    }
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("build/report.json"))?)?;
    let tools = Tools::locate()?;
    let dem_files: Vec<_> = project.sources.dem.iter().map(|p| dir.join(p)).collect();
    let dem = Dem::open(&dem_files)?;
    let base = build_heightmap(&project, &dem)?;
    let frame = MapGeo::new(&project)?;
    let window = frame.window(100.0)?;
    let files: Vec<_> = project.sources.osm.iter().map(|p| dir.join(p)).collect();
    let ways = read_ways(&files, window)?;
    let features = road_features(&ways, &frame, project.map_size_m());
    let catalog = road_catalog(&tools, &tools.cache_dir)?;
    let sources = LoadedSources::open(&project, dir, &|_| {})?;
    let lakes = sources
        .osm
        .as_ref()
        .map_or(&[][..], |o| o.polygons.as_slice());
    let streams = water_lines(&ways, &frame, project.map_size_m());
    let plan = plan_roads(&features, &catalog, &base, &streams, lakes, &frame, &|m| {
        eprintln!("{m}");
    })?;
    let objects = plan.placed.net.roads;
    let strips = plan.placed.profiles.strips;
    let paths: Vec<Vec<(f64, f64)>> = features
        .iter()
        .filter(|f| matches!(f.kind, FeatureKind::Path { .. }))
        .map(|f| f.points.clone())
        .collect();
    let osm_bridges: Vec<Vec<(f64, f64)>> = features
        .iter()
        .filter(|f| matches!(f.kind, FeatureKind::Road { bridge: true, .. }))
        .map(|f| f.points.clone())
        .collect();
    let ground = |x: f64, y: f64| f64::from(base.sample(x, y).height);
    let terrain = Terrain {
        strips: &strips,
        ground: &ground,
        paths: &paths,
        osm_bridges: &osm_bridges,
    };

    let size = project.map_size_m();
    let mut fixed = Vec::new();
    for (category, lat, lon, note) in FIXED {
        let (x, z) = frame.to_map(lat, lon)?;
        if !(0.0..=size).contains(&x) || !(0.0..=size).contains(&z) {
            eprintln!("{category} lies outside this map, left out");
            continue;
        }
        fixed.push(Site {
            id: 0,
            category: category.to_string(),
            x: (x * 10.0).round() / 10.0,
            z: (z * 10.0).round() / 10.0,
            yaw_deg: (yaw_near(&objects, x, z) * 10.0).round() / 10.0,
            numbers: BTreeMap::new(),
            note: note.to_string(),
        });
    }
    let sites = pick_sites(&report, &objects, &terrain, &fixed);
    std::fs::write(&out, serde_json::to_string_pretty(&sites)? + "\n")?;
    println!("{} sites written to {out}", sites.len());
    Ok(())
}

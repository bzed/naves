// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Sessions built from ways given in map metres, for the tests of hit-testing and the overlay.

use std::collections::BTreeMap;

use naves_core::edits::WayEdits;
use naves_core::heightmap::MapGeo;
use naves_core::osm::{OsmWay, OsmWays};
use naves_core::project::Project;
use naves_export::generate::Source;

use super::EditSession;

/// A session with one `highway=residential` way per entry: its raw id and its points in map metres. The node
/// ids are `id * 100 + index`.
///
/// # Panics
///
/// When a point is outside the map (it is a test helper).
pub fn session(ways: &[(i64, &[(f64, f64)])]) -> EditSession {
    let project = Project::new("Fixture", 47.55, 12.975, 1500.0);
    let frame = MapGeo::new(&project).unwrap();
    let ways: Vec<OsmWay> = ways
        .iter()
        .map(|(id, points)| OsmWay {
            id: *id,
            tags: BTreeMap::from([("highway".to_string(), "residential".to_string())]),
            nodes: (0..).map(|i| id * 100 + i).take(points.len()).collect(),
            points: points
                .iter()
                .map(|&(x, y)| frame.to_geo(x, y).unwrap())
                .collect(),
        })
        .collect();
    let extract = OsmWays { ways, skipped: 0 };
    let edits = WayEdits::default();
    let (ways, report) = naves_core::edits::apply_way_edits(&extract, &edits);
    EditSession::new(Source {
        frame,
        map_size_m: project.map_size_m(),
        extract,
        edits,
        ways,
        report,
    })
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Road and path features made from OSM ways.

#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use naves_core::catalog::Family;
use naves_core::features::{Feature, FeatureKind, Source, classify, road_features};
use naves_core::heightmap::MapGeo;
use naves_core::landcover::GeoWindow;
use naves_core::osm::{OsmWay, OsmWays, read_ways};
use naves_core::project::Project;
use proptest::prelude::*;

fn tags(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// The expected value of a table row.
#[expect(
    clippy::unnecessary_wraps,
    reason = "reads as the row's expected value"
)]
fn road(family: Family, drivable: bool) -> Option<FeatureKind> {
    Some(FeatureKind::Road {
        family,
        drivable,
        bridge: false,
        tunnel: false,
    })
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "reads as the row's expected value"
)]
fn path(width_m: f64) -> Option<FeatureKind> {
    Some(FeatureKind::Path { width_m })
}

/// Tags and what they classify as.
type Row<'a> = (Vec<(&'a str, &'a str)>, Option<FeatureKind>);

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one row per line of the plan's table"
)]
fn classification_table() {
    use Family::{Asf1, Asf2, Asf3, City, Grav, Mud};
    let cases: Vec<Row<'_>> = vec![
        (vec![("highway", "motorway")], road(Asf1, true)),
        (vec![("highway", "trunk")], road(Asf1, true)),
        (vec![("highway", "primary")], road(Asf1, true)),
        (vec![("highway", "primary_link")], road(Asf1, true)),
        (vec![("highway", "secondary")], road(Asf2, true)),
        (vec![("highway", "secondary_link")], road(Asf2, true)),
        (vec![("highway", "tertiary")], road(Asf3, true)),
        (vec![("highway", "unclassified")], road(Asf3, true)),
        (vec![("highway", "residential")], road(City, true)),
        (vec![("highway", "living_street")], road(City, true)),
        (vec![("highway", "service")], road(City, true)),
        (
            vec![("highway", "service"), ("service", "alley")],
            road(City, true),
        ),
        (
            vec![("highway", "service"), ("service", "driveway")],
            path(3.0),
        ),
        (
            vec![("highway", "service"), ("service", "parking_aisle")],
            path(3.0),
        ),
        (
            vec![("highway", "track"), ("tracktype", "grade1")],
            road(Grav, true),
        ),
        (
            vec![("highway", "track"), ("tracktype", "grade2")],
            road(Grav, true),
        ),
        (
            vec![("highway", "track"), ("surface", "asphalt")],
            road(Grav, true),
        ),
        (
            vec![("highway", "track"), ("surface", "compacted")],
            road(Grav, true),
        ),
        (
            vec![("highway", "track"), ("surface", "gravel")],
            road(Grav, true),
        ),
        // the tracktype wins over the surface
        (
            vec![
                ("highway", "track"),
                ("tracktype", "grade3"),
                ("surface", "asphalt"),
            ],
            road(Mud, true),
        ),
        (vec![("highway", "track")], road(Mud, true)),
        (
            vec![("highway", "track"), ("surface", "dirt")],
            road(Mud, true),
        ),
        (
            vec![("highway", "track"), ("tracktype", "grade3")],
            road(Mud, true),
        ),
        (
            vec![("highway", "track"), ("tracktype", "grade4")],
            road(Mud, false),
        ),
        (
            vec![("highway", "track"), ("tracktype", "grade5")],
            road(Mud, false),
        ),
        // smoothness beats the tracktype
        (
            vec![
                ("highway", "track"),
                ("tracktype", "grade1"),
                ("smoothness", "horrible"),
            ],
            road(Mud, false),
        ),
        (
            vec![("highway", "track"), ("smoothness", "very_horrible")],
            road(Mud, false),
        ),
        (
            vec![("highway", "track"), ("smoothness", "impassable")],
            road(Mud, false),
        ),
        (
            vec![("highway", "track"), ("smoothness", "bad")],
            road(Mud, true),
        ),
        (vec![("highway", "path")], path(1.6)),
        (vec![("highway", "footway")], path(1.6)),
        (vec![("highway", "cycleway")], path(1.6)),
        (vec![("highway", "bridleway")], path(1.6)),
        (vec![("highway", "steps")], path(1.6)),
        (vec![("highway", "pedestrian")], path(1.6)),
        (
            vec![("highway", "path"), ("sac_scale", "hiking")],
            path(1.6),
        ),
        (
            vec![("highway", "path"), ("sac_scale", "mountain_hiking")],
            path(1.6),
        ),
        (
            vec![
                ("highway", "path"),
                ("sac_scale", "demanding_mountain_hiking"),
            ],
            path(1.0),
        ),
        (
            vec![("highway", "path"), ("sac_scale", "alpine_hiking")],
            None,
        ),
        (
            vec![
                ("highway", "path"),
                ("sac_scale", "demanding_alpine_hiking"),
            ],
            None,
        ),
        (
            vec![
                ("highway", "path"),
                ("sac_scale", "difficult_alpine_hiking"),
            ],
            None,
        ),
        (vec![("highway", "path"), ("trail_visibility", "bad")], None),
        (
            vec![("highway", "path"), ("trail_visibility", "horrible")],
            None,
        ),
        (vec![("highway", "path"), ("trail_visibility", "no")], None),
        (
            vec![("highway", "path"), ("trail_visibility", "good")],
            path(1.6),
        ),
        (vec![("highway", "construction")], None),
        (vec![("highway", "proposed")], None),
        (vec![("highway", "raceway")], None),
        (vec![("highway", "platform")], None),
        (vec![("highway", "abandoned")], None),
        (vec![("highway", "razed")], None),
        (vec![("highway", "residential"), ("area", "yes")], None),
        (
            vec![("highway", "residential"), ("area", "no")],
            road(City, true),
        ),
        // not roads
        (vec![("highway", "bus_stop")], None),
        (vec![("highway", "crossing")], None),
        (vec![("highway", "via_ferrata")], None),
        (vec![("name", "x")], None),
        (vec![], None),
    ];
    for (pairs, expected) in cases {
        assert_eq!(classify(&tags(&pairs)), expected, "{pairs:?}");
    }
}

#[test]
fn access_tags_do_not_change_drivable() {
    for extra in [
        ("motor_vehicle", "no"),
        ("access", "private"),
        ("vehicle", "no"),
        ("access", "forestry"),
    ] {
        let t = tags(&[("highway", "track"), ("tracktype", "grade3"), extra]);
        assert_eq!(classify(&t), road(Family::Mud, true), "{extra:?}");
    }
}

#[test]
fn bridges_and_tunnels_are_flagged() {
    let kind = |extra: (&str, &str)| classify(&tags(&[("highway", "tertiary"), extra]));
    let flags = |k: Option<FeatureKind>| match k {
        Some(FeatureKind::Road { bridge, tunnel, .. }) => (bridge, tunnel),
        other => panic!("{other:?}"),
    };
    assert_eq!(flags(kind(("bridge", "yes"))), (true, false));
    assert_eq!(flags(kind(("bridge", "viaduct"))), (true, false));
    assert_eq!(flags(kind(("bridge", "no"))), (false, false));
    assert_eq!(flags(kind(("tunnel", "yes"))), (false, true));
    assert_eq!(flags(kind(("tunnel", "culvert"))), (false, true));
    assert_eq!(flags(kind(("tunnel", "no"))), (false, false));
}

proptest! {
    #[test]
    fn sac_scale_never_gives_a_road(
        highway in prop::sample::select(vec![
            "motorway", "trunk", "primary", "secondary", "tertiary", "unclassified", "residential",
            "service", "track", "path", "footway", "living_street", "primary_link",
        ]),
        sac in prop::sample::select(vec![
            "hiking", "mountain_hiking", "demanding_mountain_hiking", "alpine_hiking",
            "demanding_alpine_hiking", "difficult_alpine_hiking",
        ]),
    ) {
        let kind = classify(&tags(&[("highway", highway), ("sac_scale", sac)]));
        prop_assert!(matches!(kind, None | Some(FeatureKind::Path { .. })), "{kind:?}");
    }
}

// --- the map frame -------------------------------------------------------------------------------------------------

fn project(extent_m: f64, horizontal: f64) -> Project {
    let mut p = Project::new("Roads", 47.601, 12.985, extent_m);
    p.scale.horizontal = horizontal;
    p
}

#[test]
fn to_map_inverts_to_geo() {
    let p = project(2000.0, 1.0);
    let frame = MapGeo::new(&p).unwrap();
    for (x, y) in [(0.0, 0.0), (1000.0, 1000.0), (123.4, 1876.5), (2000.0, 0.0)] {
        let (lat, lon) = frame.to_geo(x, y).unwrap();
        let (bx, by) = frame.to_map(lat, lon).unwrap();
        assert!(
            (bx - x).abs() < 1e-6 && (by - y).abs() < 1e-6,
            "{x},{y} -> {bx},{by}"
        );
    }
}

#[test]
fn to_map_uses_the_map_scale() {
    let full = MapGeo::new(&project(2000.0, 1.0)).unwrap();
    let half = MapGeo::new(&project(2000.0, 0.5)).unwrap();
    // 400 m east of the centre in the real world
    let (lat, lon) = full.to_geo(1400.0, 1000.0).unwrap();
    let (x_full, _) = full.to_map(lat, lon).unwrap();
    let (x_half, _) = half.to_map(lat, lon).unwrap();
    assert!((x_full - 1400.0).abs() < 1e-6);
    // the half scale map is 1000 m across: 400 real metres are 200 map metres from its centre (500)
    assert!((x_half - 700.0).abs() < 1e-6, "{x_half}");
}

// --- clipping and ids ---------------------------------------------------------------------------------------------

const MAP: f64 = 2000.0;
const INSET: f64 = 13.0;

fn way_on_map(id: i64, nodes: &[i64], at: &[(f64, f64)]) -> OsmWay {
    let frame = MapGeo::new(&project(MAP, 1.0)).unwrap();
    OsmWay {
        id,
        tags: tags(&[("highway", "tertiary")]),
        nodes: nodes.to_vec(),
        points: at
            .iter()
            .map(|&(x, y)| frame.to_geo(x, y).unwrap())
            .collect(),
    }
}

fn features_of(ways: Vec<OsmWay>) -> Vec<Feature> {
    let frame = MapGeo::new(&project(MAP, 1.0)).unwrap();
    road_features(&OsmWays { ways, skipped: 0 }, &frame, MAP)
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn a_way_inside_the_map_is_kept_as_it_is() {
    let f = features_of(vec![way_on_map(
        7,
        &[1, 2, 3],
        &[(500.0, 500.0), (900.0, 700.0), (1200.0, 1500.0)],
    )]);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].nodes, vec![1, 2, 3]);
    assert_eq!(f[0].source, Source::Osm);
    assert_eq!(f[0].way_id, 7);
    assert!(near(f[0].points[1].0, 900.0) && near(f[0].points[1].1, 700.0));
    assert_eq!(f[0].points.len(), f[0].nodes.len());
}

#[test]
fn clipped_way_ends_at_the_inset_border() {
    // from far west of the map to far east, with a node inside
    let f = features_of(vec![way_on_map(
        9,
        &[10, 11, 12],
        &[(-500.0, 1000.0), (1000.0, 1000.0), (2500.0, 1000.0)],
    )]);
    assert_eq!(f.len(), 1);
    let feature = &f[0];
    assert_eq!(feature.nodes.len(), 3);
    assert!(
        feature.nodes[0] < 0 && feature.nodes[2] < 0,
        "{:?}",
        feature.nodes
    );
    assert_ne!(feature.nodes[0], feature.nodes[2]);
    assert_eq!(feature.nodes[1], 11, "interior nodes keep their ids");
    assert!(near(feature.points[0].0, INSET) && near(feature.points[0].1, 1000.0));
    assert!(near(feature.points[2].0, MAP - INSET));
}

#[test]
fn a_way_touching_only_outside_gives_nothing() {
    let f = features_of(vec![
        way_on_map(1, &[1, 2], &[(-900.0, -900.0), (-100.0, -50.0)]),
        // inside the map but outside the inset border
        way_on_map(2, &[3, 4], &[(2.0, 100.0), (8.0, 1900.0)]),
    ]);
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn a_way_that_leaves_and_comes_back_gives_two_features() {
    let f = features_of(vec![way_on_map(
        5,
        &[1, 2, 3, 4],
        &[
            (500.0, 500.0),
            (500.0, 2500.0),
            (1500.0, 2500.0),
            (1500.0, 500.0),
        ],
    )]);
    assert_eq!(f.len(), 2, "{f:?}");
    assert_ne!(f[0].id, f[1].id);
    assert_eq!((f[0].way_id, f[1].way_id), (5, 5));
    assert!(near(f[0].points[1].1, MAP - INSET));
    assert!(near(f[1].points[0].1, MAP - INSET));
    let ids: Vec<i64> = f.iter().flat_map(|x| x.nodes.iter().copied()).collect();
    let mut negative: Vec<i64> = ids.iter().copied().filter(|n| *n < 0).collect();
    negative.sort_unstable();
    negative.dedup();
    assert_eq!(negative.len(), 2, "each cut end has its own id: {ids:?}");
}

#[test]
fn features_come_in_way_id_order_and_are_reproducible() {
    let ways = vec![
        way_on_map(30, &[1, 2], &[(100.0, 100.0), (400.0, 400.0)]),
        way_on_map(10, &[3, 4], &[(-300.0, 900.0), (600.0, 900.0)]),
        way_on_map(20, &[5, 6], &[(1000.0, -300.0), (1000.0, 600.0)]),
    ];
    let a = features_of(ways.clone());
    let b = features_of(ways);
    assert_eq!(a, b);
    let order: Vec<i64> = a.iter().map(|f| f.way_id).collect();
    assert_eq!(order, vec![10, 20, 30]);
    // the ids of the cut ends do not repeat across features
    let mut negative: Vec<i64> = a
        .iter()
        .flat_map(|f| f.nodes.iter().copied())
        .filter(|n| *n < 0)
        .collect();
    let all = negative.len();
    negative.sort_unstable();
    negative.dedup();
    assert_eq!(negative.len(), all);
}

#[test]
fn ways_that_are_not_roads_or_paths_are_left_out() {
    let mut w = way_on_map(1, &[1, 2], &[(500.0, 500.0), (900.0, 900.0)]);
    w.tags = tags(&[("highway", "construction")]);
    assert!(features_of(vec![w]).is_empty());
}

// --- the village -------------------------------------------------------------------------------------------------

#[test]
fn the_fixture_has_the_main_road_a_bridge_and_paths() {
    let p = project(900.0, 1.0);
    let frame = MapGeo::new(&p).unwrap();
    let window: GeoWindow = frame.window(100.0).unwrap();
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/osm/schoenau.osm.pbf");
    let ways = read_ways(&[fixture], window).unwrap();
    let features = road_features(&ways, &frame, 900.0);

    let length = |f: &Feature| -> f64 {
        f.points
            .windows(2)
            .map(|p| (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1))
            .sum()
    };
    let main: f64 = features
        .iter()
        .filter(|f| {
            matches!(
                f.kind,
                FeatureKind::Road {
                    family: Family::Asf2,
                    ..
                }
            )
        })
        .map(length)
        .sum();
    assert!(main > 300.0, "only {main:.0} m of secondary road");
    assert!(features.iter().any(|f| matches!(
        f.kind,
        FeatureKind::Road {
            family: Family::City,
            ..
        }
    )));
    assert!(
        features
            .iter()
            .any(|f| matches!(f.kind, FeatureKind::Road { bridge: true, .. }))
    );
    assert!(
        features
            .iter()
            .any(|f| matches!(f.kind, FeatureKind::Path { .. }))
    );
    for f in &features {
        assert_eq!(f.points.len(), f.nodes.len());
        for &(x, y) in &f.points {
            assert!((INSET - 1e-6..=900.0 - INSET + 1e-6).contains(&x), "x {x}");
            assert!((INSET - 1e-6..=900.0 - INSET + 1e-6).contains(&y), "y {y}");
        }
    }
}

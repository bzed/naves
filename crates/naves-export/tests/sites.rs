// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The site picking of the in-game road check, on a synthetic road network.

use std::collections::BTreeMap;

use naves_core::catalog::Family;
use naves_core::profile::RoadStrip;
use naves_core::roadfit::Pose;
use naves_core::roadplace::{PlacedKind, PlacedRoad, Slot};
use naves_export::sites::{Site, Terrain, pick_sites, yaw_near};
use serde_json::json;

const MAP_M: f64 = 4096.0;

fn nth(k: usize) -> f64 {
    f64::from(u32::try_from(k).unwrap_or(0))
}

fn road(model: &str, kind: PlacedKind, start: Pose, end: Pose, scale: f64) -> PlacedRoad {
    PlacedRoad {
        slot: Slot {
            stretch: "s".to_string(),
            index: 0,
        },
        kind,
        family: Family::City,
        drivable: true,
        model: model.to_string(),
        origin: start,
        start,
        end,
        width: 7.0,
        scale,
        abreast: 1,
    }
}

fn pose(x: f64, y: f64, heading: f64) -> Pose {
    Pose { x, y, heading }
}

/// A straight piece of `len` metres from `from` along its heading.
fn straight(from: Pose, len: f64) -> PlacedRoad {
    let end = pose(
        from.x + len * from.heading.sin(),
        from.y + len * from.heading.cos(),
        from.heading,
    );
    road("straight", PlacedKind::Piece, from, end, 1.0)
}

/// An arc piece that turns `turn` radians at radius `r`.
fn arc(from: Pose, r: f64, turn: f64) -> PlacedRoad {
    let h = from.heading;
    let end = pose(
        from.x + r * (h.cos() - (h + turn).cos()),
        from.y + r * ((h + turn).sin() - h.sin()),
        h + turn,
    );
    road("arc", PlacedKind::Piece, from, end, 1.0)
}

fn fixture() -> Vec<PlacedRoad> {
    let mut o = Vec::new();
    // five kinks between straight pieces of different stretches, 400 m apart: all first pieces, then
    // all second ones, so that the two of a kink are not neighbours in the list
    let mut seconds = Vec::new();
    for (k, change) in [2.0_f64, 5.0, 9.0, 15.0, 25.0].into_iter().enumerate() {
        let x = 300.0 + 400.0 * nth(k);
        let a = straight(pose(x, 300.0, 0.0), 25.0);
        seconds.push(straight(pose(a.end.x, a.end.y, change.to_radians()), 25.0));
        o.push(a);
    }
    o.extend(seconds);
    // a hairpin: six arcs of 30 degrees at radius 10
    let mut at = pose(300.0, 1000.0, 0.0);
    for _ in 0..6 {
        let a = arc(at, 10.0, 30.0_f64.to_radians());
        at = a.end;
        o.push(a);
    }
    // two single tight arcs
    o.push(arc(pose(1200.0, 1000.0, 0.0), 10.0, 60.0_f64.to_radians()));
    o.push(arc(pose(2000.0, 1000.0, 0.0), 25.0, 45.0_f64.to_radians()));
    // a junction, an end cap
    o.push(road(
        "kr_t_city_city",
        PlacedKind::Junction,
        pose(300.0, 2000.0, 0.5),
        pose(300.0, 2000.0, 0.5),
        1.0,
    ));
    o.push(road(
        "city_konec",
        PlacedKind::EndCap,
        pose(700.0, 2000.0, 1.0),
        pose(700.0, 2000.0, 1.0),
        1.0,
    ));
    // bridges
    for (k, (model, scale)) in [
        ("bridge_metal_25", 1.0),
        ("bridge_stone_25", 1.0),
        ("bridge_wood_25", 1.0),
        ("bridge_wood_25", 1.18),
    ]
    .into_iter()
    .enumerate()
    {
        let x = 300.0 + 400.0 * nth(k);
        let from = pose(x, 3000.0, 0.0);
        let mut b = straight(from, 25.0);
        b.kind = PlacedKind::Bridge;
        b.model = model.to_string();
        b.scale = scale;
        o.push(b);
    }
    o
}

fn report() -> serde_json::Value {
    let mut v = Vec::new();
    for (k, gap) in [3.0, 8.0, 5.5].into_iter().enumerate() {
        v.push(json!({
            "rule": "road_gap",
            "x": 500.0 + 300.0 * nth(k),
            "y": 3500.0,
            "message": format!("Road 1:2-3 misses what it has to meet by {gap} m"),
        }));
    }
    v.push(json!({"rule": "road_grade", "x": 3000.0, "y": 600.0,
        "message": "Crest or dip too sharp: vertical radius 138 m, the minimum is 150 m"}));
    v.push(json!({"rule": "road_grade", "x": 3300.0, "y": 600.0,
        "message": "Crest or dip too sharp: vertical radius 60 m, the minimum is 150 m"}));
    v.push(
        json!({"rule": "join_without_junction", "x": 3600.0, "y": 1500.0,
        "message": "Roads meet here without a junction piece"}),
    );
    json!({"roads": {}, "violations": v})
}

fn fixed() -> Vec<Site> {
    // one fixed site right beside a kink, which must not make the kink disappear
    // and one that nothing else is near
    [(320.0, 320.0), (3800.0, 3800.0)]
        .into_iter()
        .map(|(x, z)| Site {
            id: 0,
            category: "fixed".to_string(),
            x,
            z,
            yaw_deg: 90.0,
            numbers: BTreeMap::new(),
            note: "fixed".to_string(),
        })
        .collect()
}

/// A strip of one 25 m segment at `x`, `y` with the road at height `z`.
fn strip(x: f64, y: f64, z: f64, bridge: bool, side_slope: f64) -> RoadStrip {
    RoadStrip {
        slots: vec![0],
        joints: vec![(x, y, z), (x + 25.0, y, z)],
        bridge: vec![bridge],
        junction: vec![false],
        side_slope: vec![side_slope],
        half_width: 3.5,
        rank: 1,
    }
}

/// The original ground is 100 m everywhere.
fn strips() -> Vec<RoadStrip> {
    vec![
        strip(300.0, 2500.0, 110.0, false, 0.1),  // embankment, 10 m
        strip(700.0, 2500.0, 85.0, false, 0.1),   // cutting, 15 m
        strip(1100.0, 2500.0, 100.0, false, 0.6), // rock face
        strip(1500.0, 2500.0, 130.0, true, 0.1),  // a bridge over a dip, 30 m
        strip(1900.0, 2500.0, 160.0, true, 0.1),  // an OpenStreetMap bridge, 60 m: not a dip
    ]
}

fn paths() -> Vec<Vec<(f64, f64)>> {
    vec![
        vec![(2600.0, 2500.0), (2750.0, 2500.0), (2900.0, 2500.0)], // long and gentle
        vec![(3600.0, 2500.0), (3610.0, 2520.0), (3620.0, 2540.0)], // steep
        vec![(3000.0, 2900.0), (3010.0, 2900.0)],                   // too short
    ]
}

fn with_terrain<T>(f: impl FnOnce(&Terrain<'_>) -> T) -> T {
    let (strips, paths) = (strips(), paths());
    let osm = vec![vec![(1900.0, 2500.0), (1925.0, 2500.0)]];
    let ground = |_: f64, _: f64| 100.0;
    f(&Terrain {
        strips: &strips,
        ground: &ground,
        paths: &paths,
        osm_bridges: &osm,
    })
}

fn sites() -> Vec<Site> {
    let mut r = report();
    if let Some(v) = r.get_mut("violations").and_then(|v| v.as_array_mut()) {
        v.push(json!({"rule": "path_too_steep", "x": 3610.0, "y": 2520.0,
            "message": "Too steep to walk: 58\u{b0} over 22 m"}));
    }
    with_terrain(|t| pick_sites(&r, &fixture(), t, &fixed()))
}

#[test]
fn the_same_input_gives_the_same_sites() {
    let a = serde_json::to_string(&sites()).unwrap();
    let b = serde_json::to_string(&sites()).unwrap();
    assert_eq!(a, b);
    assert_ne!(a, "[]");
}

#[test]
fn every_category_of_the_spec_is_present_on_the_fixture() {
    let sites = sites();
    let have: Vec<&str> = sites.iter().map(|s| s.category.as_str()).collect();
    for wanted in [
        "slight-angle",
        "tight-arc",
        "hairpin",
        "junction",
        "end-cap",
        "bridge-metal",
        "bridge-stone",
        "bridge-wood",
        "finding-road_gap",
        "finding-road_grade",
        "finding-join_without_junction",
        "high-embankment",
        "deep-cutting",
        "rock-face",
        "bridge-over-dip",
        "painted-path",
        "steep-path",
        "fixed",
    ] {
        assert!(have.contains(&wanted), "missing {wanted} in {have:?}");
    }
    let ids: Vec<u32> = sites.iter().map(|s| s.id).collect();
    assert_eq!(
        ids,
        (1..=u32::try_from(sites.len()).unwrap()).collect::<Vec<_>>()
    );
}

#[test]
fn the_worst_finding_of_a_rule_is_picked() {
    let sites = sites();
    let gap = sites
        .iter()
        .find(|s| s.category == "finding-road_gap")
        .unwrap();
    assert!((gap.numbers["measure"] - 8.0).abs() < 1e-9, "{gap:?}");
    // a smaller vertical radius is the worse one
    let grade = sites
        .iter()
        .find(|s| s.category == "finding-road_grade")
        .unwrap();
    assert!((grade.numbers["measure"] - 60.0).abs() < 1e-9, "{grade:?}");
}

#[test]
fn sites_lie_inside_the_map() {
    for s in sites() {
        assert!(
            (0.0..=MAP_M).contains(&s.x) && (0.0..=MAP_M).contains(&s.z),
            "{s:?}"
        );
        assert!((0.0..360.0).contains(&s.yaw_deg), "{s:?}");
    }
}

#[test]
fn two_sites_are_at_least_100_m_apart_unless_fixed() {
    let picked: Vec<Site> = sites()
        .into_iter()
        .filter(|s| s.category != "fixed")
        .collect();
    for (i, a) in picked.iter().enumerate() {
        for b in &picked[i + 1..] {
            assert!((a.x - b.x).hypot(a.z - b.z) >= 100.0, "{a:?} and {b:?}");
        }
    }
    // a kink 20 m from a fixed site is dropped in favour of the fixed one
    assert!(
        !picked
            .iter()
            .any(|s| (s.x - 320.0).hypot(s.z - 320.0) < 100.0)
    );
}

#[test]
fn slight_angle_sites_have_an_interior_angle_between_150_and_180_degrees() {
    let sites = sites();
    let slight: Vec<&Site> = sites
        .iter()
        .filter(|s| s.category == "slight-angle")
        .collect();
    assert!(slight.len() >= 3, "{slight:?}");
    for s in slight {
        let a = s.numbers["interior_angle_deg"];
        assert!((150.0..180.0).contains(&a), "{s:?}");
    }
}

#[test]
fn a_report_that_is_not_what_it_should_be_gives_no_findings_and_no_panic() {
    for report in [
        json!(null),
        json!([1, 2]),
        json!({"violations": "x"}),
        json!({"violations": [{"rule": 1}, {"rule": "r", "x": "a", "y": 1}, {"rule": "r", "x": 1, "y": null}]}),
    ] {
        let s = pick_sites(&report, &[], &Terrain::none(), &[]);
        assert!(s.is_empty(), "{s:?}");
    }
}

#[test]
fn yaw_near_is_the_direction_of_the_nearest_road() {
    let o = vec![straight(
        pose(100.0, 100.0, std::f64::consts::FRAC_PI_2),
        50.0,
    )];
    assert!((yaw_near(&o, 120.0, 105.0) - 90.0).abs() < 1e-6);
    assert!(yaw_near(&[], 0.0, 0.0).abs() < 1e-9);
}

fn site<'a>(sites: &'a [Site], category: &str) -> Option<&'a Site> {
    sites.iter().find(|s| s.category == category)
}

#[test]
fn the_terrain_faults_are_found_where_the_fixture_has_them() {
    let sites = sites();
    assert!(
        (site(&sites, "high-embankment").unwrap().numbers["road_above_ground_m"] - 10.0).abs()
            < 1e-9
    );
    assert!(
        (site(&sites, "deep-cutting").unwrap().numbers["road_below_ground_m"] - 15.0).abs() < 1e-9
    );
    assert!((site(&sites, "rock-face").unwrap().numbers["side_slope"] - 0.6).abs() < 1e-9);
    // the deeper bridge is an OpenStreetMap bridge and no dip
    let dip = site(&sites, "bridge-over-dip").unwrap();
    assert!((dip.x - 1512.5).abs() < 1e-9, "{dip:?}");
    assert!((dip.numbers["deck_above_ground_m"] - 30.0).abs() < 1e-9);
    let steep = site(&sites, "steep-path").unwrap();
    assert!((steep.numbers["slope_deg"] - 58.0).abs() < 1e-9);
    assert!(
        (0.0..360.0).contains(&steep.yaw_deg) && steep.yaw_deg > 0.0,
        "{steep:?}"
    );
    // the gentle path is the long one, not the steep or the short one
    let painted = site(&sites, "painted-path").unwrap();
    assert!((painted.x - 2750.0).abs() < 1e-9, "{painted:?}");
}

#[test]
fn without_a_dip_bridge_any_bridge_serves_and_without_terrain_there_are_no_terrain_sites() {
    let strips = vec![strip(1900.0, 2500.0, 160.0, true, 0.0)];
    let osm = vec![vec![(1900.0, 2500.0)]];
    let ground = |_: f64, _: f64| 100.0;
    let t = Terrain {
        strips: &strips,
        ground: &ground,
        paths: &[],
        osm_bridges: &osm,
    };
    let s = pick_sites(&json!({}), &[], &t, &[]);
    assert_eq!(s.len(), 1, "{s:?}");
    assert_eq!(s[0].category, "bridge-over-dip");
    assert_eq!(pick_sites(&json!({}), &[], &Terrain::none(), &[]).len(), 0);
}

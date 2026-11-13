// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The violations of the road build.

#![expect(
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::catalog::Family;
use naves_core::checks::{DriveState, PieceCheck, SteepPathSection};
use naves_core::conform::{Conformed, RoadConflict};
use naves_core::heightmap::{Heightmap, MapGeo};
use naves_core::junctions::{Junction, JunctionKind, Junctions, Link};
use naves_core::objects::{ObjectId, OrphanedEdit};
use naves_core::osm::LandPolygon;
use naves_core::profile::{GradeExcess, GradeExcessKind, Profiles};
use naves_core::project::Project;
use naves_core::roadfit::Pose;
use naves_core::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad, RoadGap, Slot};
use naves_core::surface::SurfaceClass;
use naves_core::validate::{Rule, Violation, check_roads};

fn road(index: u32, kind: PlacedKind, at: (f64, f64)) -> PlacedRoad {
    PlacedRoad {
        slot: Slot {
            stretch: "s".to_string(),
            index,
        },
        kind,
        family: Family::Asf3,
        drivable: true,
        model: "m".to_string(),
        origin: Pose {
            x: at.0,
            y: at.1,
            heading: 0.0,
        },
        start: Pose {
            x: at.0,
            y: at.1,
            heading: 0.0,
        },
        end: Pose {
            x: at.0,
            y: at.1 + 25.0,
            heading: 0.0,
        },
        width: 7.0,
    }
}

fn check(slot: usize, state: DriveState, grade: f64, miss: f64) -> PieceCheck {
    PieceCheck {
        slot,
        family: Family::Asf3,
        start: (100.0, 100.0 + 25.0 * slot as f64),
        end: (100.0, 125.0 + 25.0 * slot as f64),
        width: 7.0,
        grade,
        cross_fall: 0.0,
        grade_change: 0.0,
        miss_m: miss,
        state,
    }
}

fn conformed() -> Conformed {
    Conformed {
        heightmap: Heightmap {
            size: 2,
            cell_m: 1.0,
            h: vec![0.0; 4],
        },
        max_cut_m: 0.0,
        max_fill_m: 0.0,
        conflicts: Vec::new(),
        capped: Vec::new(),
    }
}

fn net(roads: Vec<PlacedRoad>) -> PlacedNetwork {
    PlacedNetwork {
        chains: vec![(0..roads.len()).collect()],
        roads,
        ..PlacedNetwork::default()
    }
}

fn frame() -> MapGeo {
    MapGeo::new(&Project::new("V", 47.55, 12.975, 1000.0)).unwrap()
}

fn run(
    net: &PlacedNetwork,
    profiles: &Profiles,
    conformed: &Conformed,
    pieces: &[PieceCheck],
    steep: &[SteepPathSection],
    orphans: &[OrphanedEdit],
    water: &[LandPolygon],
) -> Vec<Violation> {
    check_roads(
        net,
        profiles,
        &Junctions::default(),
        conformed,
        pieces,
        steep,
        orphans,
        water,
        &frame(),
    )
}

fn only(rule: Rule, violations: &[Violation]) -> Vec<&Violation> {
    violations.iter().filter(|v| v.rule == rule).collect()
}

#[test]
fn a_section_of_undrivable_pieces_is_one_violation() {
    let n = net((0..6)
        .map(|i| road(i, PlacedKind::Piece, (100.0, 100.0 + 25.0 * f64::from(i))))
        .collect());
    let pieces = vec![
        check(0, DriveState::Fine, 0.05, 0.0),
        check(1, DriveState::NotDrivable, 0.40, 0.0),
        check(2, DriveState::NotDrivable, 0.45, 0.0),
        check(3, DriveState::NotDrivable, 0.38, 0.0),
        check(4, DriveState::Fine, 0.05, 0.0),
        check(5, DriveState::NotDrivable, 0.5, 0.0),
    ];
    let v = run(
        &n,
        &Profiles::default(),
        &conformed(),
        &pieces,
        &[],
        &[],
        &[],
    );
    let nd = only(Rule::RoadNotDrivable, &v);
    assert_eq!(nd.len(), 2, "{nd:?}");
    assert!(nd[0].message.contains("75 m"), "{}", nd[0].message);
    assert!(nd[0].message.contains("45 %"), "{}", nd[0].message);
    assert!((nd[0].at.1 - 125.0).abs() < 1e-9);
}

#[test]
fn a_cross_fall_between_the_aim_and_the_limit_is_not_reported() {
    let n = net(vec![road(0, PlacedKind::Piece, (100.0, 100.0))]);
    let mut piece = check(0, DriveState::Fine, 0.10, 0.0);
    piece.cross_fall = 0.12;
    let v = run(
        &n,
        &Profiles::default(),
        &conformed(),
        &[piece],
        &[],
        &[],
        &[],
    );
    assert!(only(Rule::RoadNotDrivable, &v).is_empty());
}

#[test]
fn a_cross_fall_over_the_limit_says_so() {
    let n = net(vec![road(0, PlacedKind::Piece, (100.0, 100.0))]);
    let mut piece = check(0, DriveState::NotDrivable, 0.10, 0.0);
    piece.cross_fall = 0.24;
    let v = run(
        &n,
        &Profiles::default(),
        &conformed(),
        &[piece],
        &[],
        &[],
        &[],
    );
    let nd = only(Rule::RoadNotDrivable, &v);
    assert_eq!(nd.len(), 1, "{nd:?}");
    assert!(
        nd[0].message.ends_with("(and 24 % across)"),
        "{}",
        nd[0].message
    );
}

#[test]
fn a_piece_that_misses_the_terrain_is_reported() {
    let n = net(vec![road(0, PlacedKind::Piece, (100.0, 100.0))]);
    let v = run(
        &n,
        &Profiles::default(),
        &conformed(),
        &[check(0, DriveState::Fine, 0.0, 0.6)],
        &[],
        &[],
        &[],
    );
    let m = only(Rule::RoadMissesTerrain, &v);
    assert_eq!(m.len(), 1);
    assert!(m[0].message.contains("0.6"), "{}", m[0].message);
    let ok = run(
        &n,
        &Profiles::default(),
        &conformed(),
        &[check(0, DriveState::Fine, 0.0, 0.05)],
        &[],
        &[],
        &[],
    );
    assert!(only(Rule::RoadMissesTerrain, &ok).is_empty());
}

#[test]
fn a_path_that_is_too_steep_is_reported() {
    let steep = SteepPathSection {
        feature: "p".to_string(),
        from: (1.0, 2.0),
        to: (1.0, 32.0),
        length_m: 30.0,
        slope_deg: 61.0,
    };
    let v = run(
        &PlacedNetwork::default(),
        &Profiles::default(),
        &conformed(),
        &[],
        &[steep],
        &[],
        &[],
    );
    let p = only(Rule::PathTooSteep, &v);
    assert_eq!(p.len(), 1);
    assert!(
        p[0].message.contains("30 m") && p[0].message.contains("61"),
        "{}",
        p[0].message
    );
}

#[test]
fn the_other_rules_each_give_a_violation() {
    let profiles = Profiles {
        excesses: vec![GradeExcess {
            at: (5.0, 6.0),
            kind: GradeExcessKind::Grade,
            grade: 0.2,
            limit: 0.12,
        }],
        ..Profiles::default()
    };
    let mut n = net(vec![]);
    n.gaps.push(RoadGap {
        stretch: "7:1-2".to_string(),
        at: (10.0, 20.0),
        gap_m: 1.5,
        heading_error_deg: 3.0,
    });
    n.joins_without_junction.push((30.0, 40.0));
    n.tunnels_omitted.push(("9:3-4".to_string(), (50.0, 60.0)));
    let mut c = conformed();
    c.conflicts.push(RoadConflict {
        at: (70.0, 80.0),
        height_difference_m: 4.0,
    });
    c.capped = vec![(90.0, 100.0), (91.0, 100.0), (500.0, 500.0)];
    let orphan = OrphanedEdit {
        id: ObjectId {
            feature: "7:1-2".to_string(),
            preset: "road".to_string(),
            slot: 3,
        },
        at: (11.0, 12.0),
    };
    let v = run(&n, &profiles, &c, &[], &[], &[orphan], &[]);
    for rule in [
        Rule::RoadGrade,
        Rule::RoadGap,
        Rule::JoinWithoutJunction,
        Rule::TunnelOmitted,
        Rule::RoadConflict,
        Rule::OrphanedEdit,
    ] {
        assert_eq!(only(rule, &v).len(), 1, "{rule:?}");
    }
    // the capped places are reported once per region of 50 m
    assert_eq!(only(Rule::EarthworksCapped, &v).len(), 2);
    assert!(only(Rule::RoadGap, &v)[0].message.contains("7:1-2"));
    assert!(
        only(Rule::OrphanedEdit, &v)[0]
            .message
            .contains("road/7:1-2/3")
    );
}

#[test]
fn a_grade_and_a_bend_of_the_profile_are_worded_each_their_own_way() {
    let profiles = Profiles {
        excesses: vec![
            GradeExcess {
                at: (5.0, 6.0),
                kind: GradeExcessKind::Grade,
                grade: 0.2,
                limit: 0.12,
            },
            GradeExcess {
                at: (7.0, 8.0),
                // a radius of 40 m against a minimum of 60 m
                kind: GradeExcessKind::VerticalBend,
                grade: 1.0 / 40.0,
                limit: 1.0 / 60.0,
            },
        ],
        ..Profiles::default()
    };
    let v = run(&net(vec![]), &profiles, &conformed(), &[], &[], &[], &[]);
    let found = only(Rule::RoadGrade, &v);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].message, "Grade 20 % exceeds the limit of 12 %");
    assert_eq!(
        found[1].message,
        "Crest or dip too sharp: vertical radius 40 m, the minimum is 60 m"
    );
}

fn lake() -> LandPolygon {
    // about 450 m across, around the centre of the map
    let ring = vec![
        (47.548, 12.972),
        (47.548, 12.978),
        (47.552, 12.978),
        (47.552, 12.972),
        (47.548, 12.972),
    ];
    LandPolygon {
        class: SurfaceClass::Soil,
        priority: 3,
        outer: ring,
        holes: Vec::new(),
    }
}

#[test]
fn a_road_in_water_is_reported() {
    // the middle of the map is the middle of the lake
    let n = net(vec![
        road(0, PlacedKind::Piece, (250.0, 250.0)),
        road(1, PlacedKind::Piece, (50.0, 50.0)),
    ]);
    let v = run(
        &n,
        &Profiles::default(),
        &conformed(),
        &[],
        &[],
        &[],
        &[lake()],
    );
    let w = only(Rule::RoadInWater, &v);
    assert_eq!(w.len(), 1, "{v:?}");
    assert!((w[0].at.0 - 250.0).abs() < 1e-9);
}

#[test]
fn road_in_water_ignores_bridges() {
    let n = net(vec![road(0, PlacedKind::Bridge, (250.0, 250.0))]);
    let v = run(
        &n,
        &Profiles::default(),
        &conformed(),
        &[],
        &[],
        &[],
        &[lake()],
    );
    assert!(only(Rule::RoadInWater, &v).is_empty());
    // and only water counts (priority 3), not built-up ground
    let mut town = lake();
    town.priority = 1;
    let n = net(vec![road(0, PlacedKind::Piece, (250.0, 250.0))]);
    assert!(
        only(
            Rule::RoadInWater,
            &run(
                &n,
                &Profiles::default(),
                &conformed(),
                &[],
                &[],
                &[],
                &[town]
            )
        )
        .is_empty()
    );
}

#[test]
fn a_hole_in_a_lake_is_not_water() {
    let mut with_island = lake();
    with_island.holes.push(vec![
        (47.5495, 12.974),
        (47.5495, 12.976),
        (47.5505, 12.976),
        (47.5505, 12.974),
        (47.5495, 12.974),
    ]);
    let n = net(vec![road(0, PlacedKind::Piece, (250.0, 250.0))]);
    assert!(
        only(
            Rule::RoadInWater,
            &run(
                &n,
                &Profiles::default(),
                &conformed(),
                &[],
                &[],
                &[],
                &[with_island]
            )
        )
        .is_empty()
    );
}

#[test]
fn violations_are_sorted() {
    let mut n = net(vec![]);
    n.joins_without_junction = vec![(30.0, 40.0), (10.0, 90.0), (10.0, 20.0)];
    n.tunnels_omitted.push(("9:3-4".to_string(), (1.0, 1.0)));
    let v = run(&n, &Profiles::default(), &conformed(), &[], &[], &[], &[]);
    let keys: Vec<(Rule, f64, f64)> = v.iter().map(|x| (x.rule, x.at.0, x.at.1)).collect();
    let mut sorted = keys.clone();
    sorted.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.total_cmp(&b.1))
            .then(a.2.total_cmp(&b.2))
    });
    assert_eq!(keys, sorted);
    assert_eq!(v.len(), 4);
}

#[test]
fn the_rule_ids_are_distinct_snake_case_words() {
    let rules = [
        Rule::RoadNotDrivable,
        Rule::RoadMissesTerrain,
        Rule::PathTooSteep,
        Rule::RoadGrade,
        Rule::RoadGap,
        Rule::RoadInWater,
        Rule::RoadConflict,
        Rule::EarthworksCapped,
        Rule::OrphanedEdit,
        Rule::JoinWithoutJunction,
        Rule::TunnelOmitted,
        Rule::JunctionTooSteep,
    ];
    assert_eq!(rules, Rule::ALL);
    let mut ids: Vec<&str> = rules.iter().map(|r| r.id()).collect();
    assert!(
        ids.iter()
            .all(|i| i.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
    );
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), rules.len());
    assert_eq!(Rule::RoadNotDrivable.id(), "road_not_drivable");
}

#[test]
fn a_junction_pair_too_close_for_its_heights_is_a_violation() {
    let at = |x: f64, height: f64| Junction {
        at: (x, 40.0),
        kind: JunctionKind::Join,
        ground: height,
        height,
        rank: 3,
        patch_m: 3.5,
    };
    let junctions = Junctions {
        points: vec![at(100.0, 100.0), at(160.0, 112.0)],
        links: vec![Link {
            a: 0,
            b: 1,
            length: 50.0,
            ceiling: 0.12,
        }],
        unresolved: vec![0],
    };
    let v = check_roads(
        &net(Vec::new()),
        &Profiles::default(),
        &junctions,
        &conformed(),
        &[],
        &[],
        &[],
        &[],
        &frame(),
    );
    let found = only(Rule::JunctionTooSteep, &v);
    assert_eq!(found.len(), 1, "{v:?}");
    assert_eq!(found[0].rule.id(), "junction_too_steep");
    assert_eq!(found[0].at, (130.0, 40.0));
    assert_eq!(
        found[0].message,
        "Junctions too close for their heights: 12.0 m over 50 m"
    );
    // a repaired junction set makes none
    assert!(
        only(
            Rule::JunctionTooSteep,
            &run(
                &net(Vec::new()),
                &Profiles::default(),
                &conformed(),
                &[],
                &[],
                &[],
                &[]
            )
        )
        .is_empty()
    );
}

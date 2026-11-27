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
use naves_core::dips::{Dip, DipAction};
use naves_core::heightmap::{Heightmap, MapGeo};
use naves_core::junctions::{Junction, JunctionKind, Junctions, Link};
use naves_core::objects::{ObjectId, OrphanedEdit};
use naves_core::osm::LandPolygon;
use naves_core::profile::{GradeExcess, GradeExcessKind, Profiles, RoadStrip};
use naves_core::project::Project;
use naves_core::roadfit::Pose;
use naves_core::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad, RoadGap, RoadKink, Slot};
use naves_core::surface::SurfaceClass;
use naves_core::validate::{Rule, Violation, check_roads, water_test};

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
        scale: 1.0,
        abreast: 1,
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
        off_ground_m: 0.0,
        side_slope: 0.0,
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
        buried: Vec::new(),
        floating: Vec::new(),
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
        &[],
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
    assert_eq!(only(Rule::RoadNotDrivable, &v).len(), 0);
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
    assert_eq!(only(Rule::RoadMissesTerrain, &ok).len(), 0);
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
        nothing_placed: false,
    });
    n.kinks.push(RoadKink {
        stretch: "7:1-2".to_string(),
        at: (12.0, 22.0),
        heading_error_deg: 17.5,
    });
    n.joins_without_junction.push((30.0, 40.0));
    n.tunnels_omitted.push(("9:3-4".to_string(), (50.0, 60.0)));
    let mut c = conformed();
    c.conflicts.push(RoadConflict {
        bridge: false,
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
        Rule::RoadKink,
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
    // a road with nothing placed on it says so
    let mut unplaced = net(vec![]);
    unplaced.gaps.push(RoadGap {
        stretch: "8:3-4".to_string(),
        at: (1.0, 2.0),
        gap_m: 40.0,
        heading_error_deg: 0.0,
        nothing_placed: true,
    });
    let v2 = run(
        &unplaced,
        &Profiles::default(),
        &conformed(),
        &[],
        &[],
        &[],
        &[],
    );
    let gap = only(Rule::RoadGap, &v2)[0];
    assert!(
        gap.message.contains("8:3-4")
            && gap.message.contains("no pieces")
            && gap.message.contains("40.0"),
        "{}",
        gap.message
    );
    // a kink is its own finding, with the angle and the site, and no gap
    let kink = only(Rule::RoadKink, &v)[0];
    assert!(
        kink.message.contains("17.5") && kink.message.contains("7:1-2"),
        "{}",
        kink.message
    );
    assert_eq!(kink.at, (12.0, 22.0));
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
    assert_eq!(only(Rule::RoadInWater, &v).len(), 0);
    // and only water counts (priority 3), not built-up ground
    let mut town = lake();
    town.priority = 1;
    let n = net(vec![road(0, PlacedKind::Piece, (250.0, 250.0))]);
    assert_eq!(
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
        .len(),
        0
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
    assert_eq!(
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
        .len(),
        0
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
        Rule::RoadKink,
        Rule::RoadInWater,
        Rule::RoadConflict,
        Rule::EarthworksCapped,
        Rule::OrphanedEdit,
        Rule::JoinWithoutJunction,
        Rule::TunnelOmitted,
        Rule::JunctionTooSteep,
        Rule::RoadFarFromGround,
        Rule::RoadOverWater,
        Rule::RoadDuplicateDropped,
        Rule::BridgeClearance,
        Rule::BridgeBlocksRoad,
        Rule::BridgeNotBuilt,
        Rule::BridgeBuried,
        Rule::BridgePostFloats,
        Rule::BridgeEndStep,
        Rule::RoadRoundaboutCollapsed,
        Rule::RoadSidepathDropped,
        Rule::RoadStubDropped,
        Rule::RoadJunctionsMerged,
        Rule::RoadJunctionsNotMerged,
        Rule::BridgeApproachMade,
        Rule::BridgeEndAtJunction,
        Rule::RoadCurveWidened,
        Rule::RoadCurveTooTight,
        Rule::RoadCurvePlanned,
        Rule::RoadCurveNotPlanned,
        Rule::LotSmoothed,
        Rule::LotSurfaceOnly,
        Rule::LotExcluded,
        Rule::LotSteep,
        Rule::LotMoveLimited,
        Rule::RoadInLotDropped,
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
    assert_eq!(Rule::RoadDuplicateDropped.id(), "road_duplicate_dropped");
    assert_eq!(Rule::BridgeClearance.id(), "bridge_clearance");
    assert_eq!(Rule::BridgeNotBuilt.id(), "bridge_not_built");
    assert_eq!(Rule::BridgeBlocksRoad.id(), "bridge_blocks_road");
    assert_eq!(Rule::BridgeBuried.id(), "bridge_buried");
    assert_eq!(Rule::BridgePostFloats.id(), "bridge_post_floats");
    assert_eq!(Rule::BridgeEndStep.id(), "bridge_end_step");
    assert_eq!(Rule::RoadKink.id(), "road_kink");
    assert_eq!(
        Rule::RoadRoundaboutCollapsed.id(),
        "road_roundabout_collapsed"
    );
    assert_eq!(Rule::RoadSidepathDropped.id(), "road_sidepath_dropped");
    assert_eq!(Rule::RoadStubDropped.id(), "road_stub_dropped");
    assert_eq!(Rule::RoadJunctionsMerged.id(), "road_junctions_merged");
    assert_eq!(
        Rule::RoadJunctionsNotMerged.id(),
        "road_junctions_not_merged"
    );
    assert_eq!(Rule::BridgeApproachMade.id(), "bridge_approach_made");
    assert_eq!(Rule::BridgeEndAtJunction.id(), "bridge_end_at_junction");
    assert_eq!(Rule::RoadCurveWidened.id(), "road_curve_widened");
    assert_eq!(Rule::RoadCurveTooTight.id(), "road_curve_too_tight");
    assert_eq!(Rule::RoadCurvePlanned.id(), "road_curve_planned");
    assert_eq!(Rule::RoadCurveNotPlanned.id(), "road_curve_not_planned");
}

#[test]
fn a_widened_curve_and_a_curve_that_cannot_be_built_are_findings_at_their_place() {
    use naves_core::smooth::{SmoothReport, WidenedZone};
    let zone = |at: (f64, f64), after: f64, outward: f64, plate: bool| WidenedZone {
        at,
        stretches: vec!["7:1-2".to_string()],
        radius_before_m: 10.0,
        radius_after_m: after,
        radius_wanted_m: 25.0,
        max_outward_m: outward,
        max_inward_m: 0.0,
        reached: !plate,
        plate_candidate: plate,
        blocked_by_water: false,
    };
    let mut n = net(vec![]);
    n.smooth = Some(SmoothReport {
        chains: 1,
        kept: Vec::new(),
        skipped_short: 0,
        max_move_m: 0.0,
        tight_before_m: 0.0,
        tight_after_m: 0.0,
        per_stretch: Vec::new(),
        zones: vec![
            zone((1.0, 2.0), 25.0, 3.5, false),
            zone((3.0, 4.0), 12.0, 0.0, true),
            // a zone that did not move is no finding
            zone((5.0, 6.0), 25.0, 0.0, false),
            // a zone that widening would have put in water stays as drawn: it is too tight too
            {
                let mut z = zone((7.0, 8.0), 25.0, 0.0, false);
                z.blocked_by_water = true;
                z
            },
        ],
        max_outward_m: 3.5,
        widen_failed: Vec::new(),
        points_before: 0,
        points_fitted: 0,
        points_after: 0,
    });
    let v = run(&n, &Profiles::default(), &conformed(), &[], &[], &[], &[]);
    let widened = only(Rule::RoadCurveWidened, &v);
    assert_eq!(widened.len(), 1);
    assert_eq!(widened[0].at, (1.0, 2.0));
    assert!(
        widened[0].message.contains("7:1-2"),
        "{}",
        widened[0].message
    );
    let tight = only(Rule::RoadCurveTooTight, &v);
    assert_eq!(tight.len(), 2);
    assert_eq!(tight[1].at, (7.0, 8.0));
    assert!(tight[1].message.contains("water"), "{}", tight[1].message);
    assert_eq!(tight[0].at, (3.0, 4.0));
    assert!(
        tight[0].message.contains("12.0") && tight[0].message.contains("25.0"),
        "{}",
        tight[0].message
    );
    assert!(n.smooth.as_ref().unwrap().summary().contains("1 plate"));
}

/// A planned zone of `7:1-2` for the planning findings: radii and grades that the assertions pick up in
/// the messages.
fn reroute_zone(
    at: (f64, f64),
    planned_grade: f64,
    relax: usize,
) -> naves_core::reroute::PlannedZone {
    use naves_core::reroute::{AnchorKind, PlannedZone, ZoneOutcome};
    PlannedZone {
        id: format!("7:1-2#{relax}"),
        at,
        stretches: vec!["7:1-2".to_string()],
        legs: 1,
        anchors: (AnchorKind::Line, AnchorKind::Line),
        junction: None,
        partner: None,
        outcome: ZoneOutcome::Planned,
        radius_before_m: 4.0,
        radius_wanted_m: 10.0,
        radius_after_m: 10.5,
        length_before_m: 40.0,
        length_after_m: 42.0,
        max_stray_m: 1.5,
        drawn_grade: 0.05,
        drawn_grade_window: 0.09,
        planned_grade,
        grade_limit: 0.14,
        relax_steps: relax,
        climb_m: 1.0,
        earthwork_m2: 2.5,
        cut_max_m: 0.2,
        fill_max_m: 0.3,
        states: 42,
        weight: Some(1.0),
        budget_exhausted: false,
        original_feasible: true,
        candidate: None,
    }
}

#[test]
fn a_planned_curve_names_how_much_steeper_than_drawn_it_is() {
    use naves_core::reroute::RerouteReport;
    let mut relaxed = reroute_zone((3.0, 4.0), 0.20, 1);
    relaxed.drawn_grade = 0.12;
    relaxed.grade_limit = 0.22;
    // a relaxed route whose steepest step is under the drawn mean: the figure is cut at zero
    let mut clamped = reroute_zone((7.0, 9.0), 0.14, 3);
    clamped.drawn_grade = 0.20;
    let mut n = net(vec![]);
    n.reroute = Some(RerouteReport {
        zones: vec![reroute_zone((1.0, 2.0), 0.14, 0), relaxed, clamped],
        kept_zones: Vec::new(),
        tight_before_m: 60.0,
        tight_after_m: 20.0,
        max_stray_m: 1.5,
        steeper_than_drawn: 1,
        reverted: 0,
        kept_sampled_routes: 0,
        points_before: 30,
        points_after: 30,
        states: 42,
        skipped: Vec::new(),
    });
    let v = run(&n, &Profiles::default(), &conformed(), &[], &[], &[], &[]);
    let planned = only(Rule::RoadCurvePlanned, &v);
    assert_eq!(planned.len(), 3);
    assert_eq!(planned[0].at, (1.0, 2.0));
    assert!(
        planned[0].message.contains("4.0") && planned[0].message.contains("10.5"),
        "{}",
        planned[0].message
    );
    assert!(
        !planned[0].message.contains("steeper"),
        "a zone planned at its own limit is not steeper than drawn: {}",
        planned[0].message
    );
    assert_eq!(planned[1].at, (3.0, 4.0));
    assert!(
        planned[1].message.contains("8.0 points steeper than drawn"),
        "{}",
        planned[1].message
    );
    assert_eq!(planned[2].at, (7.0, 9.0));
    assert!(
        planned[2].message.contains("0.0 points steeper than drawn")
            && !planned[2].message.contains("-6.0"),
        "a relaxed route that is not steeper than drawn reads zero, never negative: {}",
        planned[2].message
    );
    assert!(
        only(Rule::RoadCurveNotPlanned, &v).is_empty(),
        "a planned zone keeps no warning"
    );
}

#[test]
fn a_kept_curve_names_its_reason_and_the_radius_it_keeps() {
    use naves_core::reroute::{Candidate, KeptReason, RerouteReport, ZoneOutcome};
    let mut held = reroute_zone((5.0, 6.0), 0.0, 0);
    held.outcome = ZoneOutcome::Kept {
        reason: KeptReason::Held,
        detail: "anchors_close".to_string(),
    };
    held.candidate = Some(Candidate::Plate);
    held.weight = None;
    // no leg of the zone moved: the radius it keeps is the drawn one
    held.max_stray_m = 0.0;
    // a kept zone with a planned leg: the drawn radius of the kept leg is the one that stays
    let mut partly = reroute_zone((9.0, 8.0), 0.14, 2);
    partly.outcome = ZoneOutcome::Kept {
        reason: KeptReason::Held,
        detail: "route_of 7:1-2#1".to_string(),
    };
    partly.max_stray_m = 1.5;
    partly.weight = None;
    let mut n = net(vec![]);
    n.reroute = Some(RerouteReport {
        zones: vec![held, partly],
        kept_zones: vec![("7:1-2#0".to_string(), "anchors_close".to_string())],
        tight_before_m: 60.0,
        tight_after_m: 20.0,
        max_stray_m: 1.5,
        steeper_than_drawn: 1,
        reverted: 0,
        kept_sampled_routes: 0,
        points_before: 30,
        points_after: 30,
        states: 42,
        skipped: Vec::new(),
    });
    let v = run(&n, &Profiles::default(), &conformed(), &[], &[], &[], &[]);
    let not_planned = only(Rule::RoadCurveNotPlanned, &v);
    assert_eq!(not_planned.len(), 2);
    assert_eq!(not_planned[0].at, (5.0, 6.0));
    assert!(not_planned[0].message.contains("keeps its drawn line"));
    assert!(
        not_planned[0].message.contains("held")
            && not_planned[0].message.contains("anchors_close")
            && not_planned[0].message.contains("4.0")
            && not_planned[0].message.contains("10.0"),
        "the radius of a zone that keeps its line is the drawn one: {}",
        not_planned[0].message
    );
    assert_eq!(not_planned[1].at, (9.0, 8.0));
    assert!(
        not_planned[1].message.contains("partly planned") && not_planned[1].message.contains("4.0"),
        "a kept zone with a planned leg names the tighter of its radii: {}",
        not_planned[1].message
    );
    assert!(
        !not_planned[1].message.contains("10.5"),
        "the radius of the planned leg hides the kept bend: {}",
        not_planned[1].message
    );
    assert!(
        only(Rule::RoadCurvePlanned, &v).is_empty(),
        "a kept zone keeps no information"
    );
}

#[test]
fn every_change_of_the_clean_up_is_a_finding_at_its_place() {
    use naves_core::catalog::Family;
    use naves_core::netclean::{
        Approach, ApproachKind, DroppedSidepath, DroppedStub, MergedJunction, NotMerged,
    };
    use naves_core::roadnet::Ring;
    let mut n = net(vec![]);
    n.clean.roundabouts.push(Ring {
        stretches: vec![0],
        centre: (1.0, 2.0),
        radius: 6.0,
        family: Family::City,
    });
    n.clean.sidepaths.push(DroppedSidepath {
        stretch: "7:1-2".to_string(),
        road: "8:3-4".to_string(),
        from_m: 10.0,
        to_m: 90.0,
        at: (3.0, 4.0),
        connectors: vec![],
    });
    n.clean.stubs.push(DroppedStub {
        stretch: "9:5-6".to_string(),
        length_m: 4.0,
        at: (5.0, 6.0),
    });
    n.clean.merged.push(MergedJunction {
        kept: 10,
        merged: vec![11],
        removed: vec![],
        at: (7.0, 8.0),
    });
    n.clean.not_merged.push(NotMerged {
        nodes: vec![20, 21],
        reason: "a roundabout is among the junctions",
        at: (11.0, 12.0),
    });
    n.clean.approaches.push(Approach {
        bridge: "12:1-2".to_string(),
        end: 1,
        kind: ApproachKind::SideRoadsMoved,
        moved: vec![],
        at: (9.0, 10.0),
    });
    n.clean.no_approach.push(naves_core::netclean::NoApproach {
        bridge: "13:1-2".to_string(),
        end: 0,
        reason: "no_room",
        at: (13.0, 14.0),
    });
    let v = run(&n, &Profiles::default(), &conformed(), &[], &[], &[], &[]);
    for (rule, at) in [
        (Rule::RoadRoundaboutCollapsed, (1.0, 2.0)),
        (Rule::RoadSidepathDropped, (3.0, 4.0)),
        (Rule::RoadStubDropped, (5.0, 6.0)),
        (Rule::RoadJunctionsMerged, (7.0, 8.0)),
        (Rule::RoadJunctionsNotMerged, (11.0, 12.0)),
        (Rule::BridgeApproachMade, (9.0, 10.0)),
        (Rule::BridgeEndAtJunction, (13.0, 14.0)),
    ] {
        let found = only(rule, &v);
        assert_eq!(found.len(), 1, "{rule:?}");
        assert_eq!(found[0].at, at);
    }
}

#[test]
fn an_osm_bridge_without_parts_is_a_finding_and_a_fill_is_not() {
    use naves_core::bridgekind::{BridgeFit, BridgeKind, Fallback, Lead, RoomEnd};
    let fit = |id: &str, fallback: Vec<Fallback>, at: (f64, f64)| BridgeFit {
        stretches: vec![id.to_string()],
        stretch: id.to_string(),
        at,
        family: Family::Asf1,
        length: 10.4,
        own_turn_deg: 0.0,
        kind: BridgeKind::Fill,
        part_length: 0.0,
        grown: (0.0, 0.0),
        part_start: 0.0,
        lead: (Lead::Unbuilt, Lead::Unbuilt),
        turn_deg: (0.0, 0.0),
        room: (0.0, 0.0),
        room_end: (RoomEnd::Junction, RoomEnd::Junction),
        fallback,
        slid: 0.0,
        below: Vec::new(),
    };
    let mut n = net(vec![]);
    n.bridge_fits = vec![
        fit("7:1-2", vec![Fallback::NoParts], (5.0, 6.0)),
        // filled because it is short: no finding
        fit("8:3-4", vec![], (1.0, 1.0)),
    ];
    let v = run(&n, &Profiles::default(), &conformed(), &[], &[], &[], &[]);
    let found = only(Rule::BridgeNotBuilt, &v);
    assert_eq!(found.len(), 1, "{v:?}");
    assert_eq!(found[0].at, (5.0, 6.0));
    assert!(
        found[0].message.contains("7:1-2") && found[0].message.contains("10 m"),
        "{}",
        found[0].message
    );
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one scenario: decks, roads below, fits and the findings"
)]
fn a_bridge_on_the_road_below_is_a_finding_with_the_missing_metres_where_the_road_is_placed() {
    use naves_core::bridgefit::{BelowFit, mark_placed};
    use naves_core::bridgekind::{BridgeFit, BridgeKind, Lead, RoomEnd};
    let below = |lower: &str, at: (f64, f64), missing: f64| BelowFit {
        lower: lower.to_string(),
        at,
        station: 10.0,
        interval: (2.0, 20.0),
        missing,
        placed: true,
    };
    let fit = |id: &str, below: Vec<BelowFit>| BridgeFit {
        stretches: vec![id.to_string()],
        stretch: id.to_string(),
        at: (0.0, 0.0),
        family: Family::Asf1,
        length: 25.0,
        own_turn_deg: 0.0,
        kind: BridgeKind::Fill,
        part_length: 0.0,
        grown: (0.0, 0.0),
        part_start: 0.0,
        lead: (Lead::Unbuilt, Lead::Unbuilt),
        turn_deg: (0.0, 0.0),
        room: (0.0, 0.0),
        room_end: (RoomEnd::Junction, RoomEnd::Junction),
        fallback: Vec::new(),
        slid: 0.0,
        below,
    };
    // the placed decks (a step of a grid each: 12 m wide, running north), and the roads below them: one part
    // split from "5:7-8" by the dip pass crossing under the deck of "9:1-2", the road "5:7-8" itself under the
    // deck of "4:3-4", and "6:7-8" laid 12 m beside the first deck, which it does not run under
    let piece = |stretch: &str, kind: PlacedKind, from: (f64, f64), to: (f64, f64), width: f64| {
        let mut r = road(0, kind, from);
        r.slot.stretch = stretch.to_string();
        r.start = Pose {
            x: from.0,
            y: from.1,
            heading: 0.0,
        };
        r.end = Pose {
            x: to.0,
            y: to.1,
            heading: 0.0,
        };
        r.width = width;
        r
    };
    let mut n = net(vec![
        piece(
            "9:1-2",
            PlacedKind::Bridge,
            (300.0, 30.0),
            (300.0, 50.0),
            12.0,
        ),
        piece(
            "4:3-4",
            PlacedKind::Bridge,
            (300.0, 20.0),
            (300.0, 35.0),
            12.0,
        ),
        piece(
            "5:7-8/after1",
            PlacedKind::Piece,
            (290.0, 40.0),
            (310.0, 40.0),
            10.0,
        ),
        piece(
            "5:7-8|9:1-2",
            PlacedKind::Junction,
            (290.0, 30.0),
            (310.0, 30.0),
            10.0,
        ),
        piece(
            "6:7-8",
            PlacedKind::Piece,
            (312.0, 20.0),
            (312.0, 60.0),
            10.0,
        ),
    ]);
    n.bridge_fits = vec![
        fit(
            "9:1-2",
            vec![
                below("5:7-8", (300.0, 40.0), 7.31),
                // nothing missing there
                below("5:7-8", (300.0, 41.0), 0.0),
                // the lower road is not under the deck
                below("6:7-8", (500.0, 40.0), 3.0),
                // a rounding sliver is no finding
                below("5:7-8", (300.0, 42.0), 0.01),
            ],
        ),
        fit("4:3-4", vec![below("5:7-8", (300.0, 30.0), 1.5)]),
    ];
    mark_placed(&mut n.bridge_fits, &n.roads);
    let placed: Vec<bool> = n.bridge_fits[0].below.iter().map(|b| b.placed).collect();
    assert_eq!(placed, [true, true, false, true]);
    let v = run(&n, &Profiles::default(), &conformed(), &[], &[], &[], &[]);
    let found = only(Rule::BridgeBlocksRoad, &v);
    // sorted by position, not by bridge
    assert_eq!(found.len(), 2, "{v:?}");
    assert_eq!(found[0].at, (300.0, 30.0));
    assert_eq!(found[1].at, (300.0, 40.0));
    assert!(
        found[1].message.contains("9:1-2")
            && found[1].message.contains("5:7-8")
            && found[1].message.contains("7.3 m")
            && found[1].message.contains("18.0 m"),
        "{}",
        found[1].message
    );
    // the same input gives the same list
    let again = run(&n, &Profiles::default(), &conformed(), &[], &[], &[], &[]);
    assert_eq!(v, again);
}

#[test]
fn buried_ground_and_floating_posts_are_findings_with_their_place_and_numbers() {
    use naves_core::conform::{Buried, Holder, PostFloat};
    let mut c = conformed();
    c.buried = vec![Buried {
        stretch: "9:1-2".to_string(),
        at: (300.0, 40.0),
        above_m: 1.26,
        vertices: 7,
        holder: Holder::Patch((300.0, 41.0)),
        guarded: false,
    }];
    c.floating = vec![
        PostFloat {
            stretch: "4:3-4".to_string(),
            at: (100.0, 20.0),
            by_m: 3.5,
        },
        PostFloat {
            stretch: "5:3-4".to_string(),
            at: (100.0, 10.0),
            by_m: 0.2,
        },
    ];
    let n = net(vec![]);
    let v = run(&n, &Profiles::default(), &c, &[], &[], &[], &[]);
    let buried = only(Rule::BridgeBuried, &v);
    assert_eq!(buried.len(), 1, "{v:?}");
    assert_eq!(buried[0].at, (300.0, 40.0));
    assert!(
        buried[0].message.contains("9:1-2")
            && buried[0].message.contains("1.3 m")
            && buried[0].message.contains("7 places")
            && buried[0]
                .message
                .contains("above the underside of its deck")
            && buried[0]
                .message
                .contains("held by a level patch at (300, 41)")
            && !buried[0].message.contains("could not be cut"),
        "{}",
        buried[0].message
    );
    // within the end guard the number is above the deck's surface; nobody holding it is said once
    c.buried[0].guarded = true;
    c.buried[0].holder = Holder::None;
    let v = run(&n, &Profiles::default(), &c, &[], &[], &[], &[]);
    let guarded = only(Rule::BridgeBuried, &v);
    assert!(
        guarded[0]
            .message
            .contains("1.3 m above the deck at 7 places, held by no road")
            && !guarded[0].message.contains("underside")
            && !guarded[0].message.contains("could not be cut"),
        "{}",
        guarded[0].message
    );
    c.buried[0].guarded = false;
    c.buried[0].holder = Holder::Patch((300.0, 41.0));
    let floats = only(Rule::BridgePostFloats, &v);
    // sorted by position
    assert_eq!(floats.len(), 2, "{v:?}");
    assert_eq!(floats[0].at, (100.0, 10.0));
    assert!(
        floats[1].message.contains("4:3-4") && floats[1].message.contains("4.0 m"),
        "{}",
        floats[1].message
    );
    let again = run(&n, &Profiles::default(), &c, &[], &[], &[], &[]);
    assert_eq!(again, run(&n, &Profiles::default(), &c, &[], &[], &[], &[]));
}

#[test]
fn a_dropped_duplicate_is_one_finding_at_its_middle() {
    let mut n = net(vec![]);
    n.duplicates.push(naves_core::roadnet::DroppedDuplicate {
        stretch: "7:1-2".to_string(),
        kept: "8:3-4".to_string(),
        from_m: 10.0,
        to_m: 90.0,
        at: (45.0, 5.0),
    });
    let v = run(&n, &Profiles::default(), &conformed(), &[], &[], &[], &[]);
    let found = only(Rule::RoadDuplicateDropped, &v);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].at, (45.0, 5.0));
    assert!(found[0].message.contains("7:1-2") && found[0].message.contains("8:3-4"));
    assert!(found[0].message.contains("80 m"), "{}", found[0].message);
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
        stretches: std::collections::BTreeSet::new(),
        lower: std::collections::BTreeSet::new(),
        gradient: (0.0, 0.0),
        tilt_axis: None,
        tilt_cap: 0.0,
    };
    let junctions = Junctions {
        rings: Vec::new(),
        lots: Vec::new(),
        points: vec![at(100.0, 100.0), at(160.0, 112.0)],
        links: vec![Link {
            a: 0,
            b: 1,
            length: 50.0,
            ceiling: 0.12,
            a_at: (0.0, 0.0),
            b_at: (0.0, 0.0),
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
        &[],
    );
    let found = only(Rule::JunctionTooSteep, &v);
    assert_eq!(found.len(), 1, "{v:?}");
    assert_eq!(found[0].rule.id(), "junction_too_steep");
    assert_eq!(found[0].at, (130.0, 40.0));
    assert_eq!(
        found[0].message,
        "Junctions too close for their heights: 12.0 m over 50 m"
    );
    // a junction set without unresolved links makes none
    assert_eq!(
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
        .len(),
        0
    );
}

fn far_check(slot: usize, off_ground_m: f64, side_slope: f64) -> PieceCheck {
    PieceCheck {
        off_ground_m,
        side_slope,
        ..check(slot, DriveState::Fine, 0.05, 0.0)
    }
}

fn far_from_ground(pieces: &[PieceCheck]) -> Vec<Violation> {
    let roads: Vec<PlacedRoad> = (0..pieces.len())
        .map(|i| road(u32::try_from(i).unwrap(), PlacedKind::Piece, (100.0, 100.0)))
        .collect();
    let v = run(
        &net(roads),
        &Profiles::default(),
        &conformed(),
        pieces,
        &[],
        &[],
        &[],
    );
    only(Rule::RoadFarFromGround, &v)
        .into_iter()
        .cloned()
        .collect()
}

#[test]
fn a_deep_cut_on_flat_ground_is_reported_and_the_same_cut_on_a_mountainside_is_not() {
    let far = far_from_ground(&[far_check(0, -40.0, 0.05), far_check(1, -40.0, 0.60)]);
    assert_eq!(far.len(), 1, "{far:?}");
    assert_eq!(far[0].rule.id(), "road_far_from_ground");
    assert_eq!(far[0].message, "Cut 40.0 m deep; 30 m fit this ground");
}

#[test]
fn an_embankment_over_the_allowance_is_reported_and_nan_or_a_small_height_is_not() {
    let far = far_from_ground(&[
        far_check(0, 70.0, 0.40),
        far_check(1, 64.0, 0.40),
        far_check(2, f64::NAN, 0.0),
        far_check(3, 30.0, f64::NAN),
    ]);
    assert_eq!(far.len(), 1, "{far:?}");
    assert_eq!(
        far[0].message,
        "Embankment 70.0 m high; 65 m fit this ground"
    );
    // the depth has one decimal: 30.4 m is over a 30 m allowance and says so
    let far = far_from_ground(&[far_check(0, -30.4, 0.0)]);
    assert_eq!(far[0].message, "Cut 30.4 m deep; 30 m fit this ground");
    // the cut at the allowance is within it
    assert_eq!(far_from_ground(&[far_check(0, -30.0, 0.0)]).len(), 0);
}

fn wet_dip(depth_m: f64, at: (f64, f64)) -> Dip {
    Dip {
        stretch: 0,
        from_m: 100.0,
        to_m: 130.0,
        depth_m,
        over_water: true,
        at,
    }
}

#[test]
fn a_dip_over_water_that_got_no_bridge_is_reported_where_it_is() {
    let dips = [
        (wet_dip(7.6, (250.0, 40.0)), DipAction::Report),
        // a bridge or a fill is nothing to look at
        (wet_dip(9.0, (10.0, 10.0)), DipAction::Bridge),
        (wet_dip(9.0, (20.0, 20.0)), DipAction::Fill),
    ];
    let v = check_roads(
        &net(Vec::new()),
        &Profiles::default(),
        &Junctions::default(),
        &conformed(),
        &[],
        &[],
        &[],
        &[],
        &frame(),
        &dips,
    );
    let found = only(Rule::RoadOverWater, &v);
    assert_eq!(found.len(), 1, "{v:?}");
    assert_eq!(found[0].rule.id(), "road_over_water");
    assert_eq!(found[0].at, (250.0, 40.0));
    assert_eq!(
        found[0].message,
        "Road crosses water 8 m below it without a bridge; look at it"
    );
}

#[test]
fn an_overlay_is_not_a_conflict_or_a_join_without_junction() {
    use naves_core::catalog::testing::synthetic_catalog;
    use naves_core::conform::conform;
    use naves_core::features::{Feature, FeatureKind, Source};
    use naves_core::heightmap::Heightmap;
    use naves_core::junctions::find_junctions;
    use naves_core::profile::solve_profiles;
    use naves_core::roadnet::build_graph;
    use naves_core::roadplace::place_network;
    use naves_core::sideslope::side_slopes;

    let feature = |way: i64, family: Family, nodes: &[i64], points: &[(f64, f64)]| Feature {
        id: format!("osm/way/{way}/0"),
        way_id: way,
        source: Source::Osm,
        kind: FeatureKind::Road {
            family,
            drivable: true,
            bridge: false,
            tunnel: false,
            roundabout: false,
            layer: 0,
            use_sidepath: false,
            paving: None,
        },
        points: points.to_vec(),
        nodes: nodes.to_vec(),
    };
    let graph = build_graph(&[
        feature(
            1,
            Family::City,
            &[1, 2, 3],
            &[(50.0, 200.0), (200.0, 200.0), (350.0, 200.0)],
        ),
        feature(2, Family::Mud, &[2, 4], &[(200.0, 200.0), (200.0, 350.0)]),
    ]);
    let n = place_network(&graph, &synthetic_catalog()).unwrap();
    assert_eq!(n.overlays.len(), 1);
    let hm = Heightmap {
        size: 128,
        cell_m: 3.125,
        h: vec![100.0; 128 * 128],
    };
    let junctions = find_junctions(&n, &graph, &hm);
    let profiles = solve_profiles(&n, &hm, &junctions, &side_slopes(&n, &hm)).unwrap();
    let conformed = conform(&hm, &profiles.strips, &[]).unwrap();
    let v = check_roads(
        &n,
        &profiles,
        &junctions,
        &conformed,
        &[],
        &[],
        &[],
        &[],
        &frame(),
        &[],
    );
    assert!(only(Rule::JoinWithoutJunction, &v).is_empty(), "{v:?}");
    assert!(only(Rule::RoadConflict, &v).is_empty(), "{v:?}");
}

#[test]
fn roads_that_all_end_at_a_crossing_are_still_a_join_without_junction() {
    use naves_core::catalog::testing::synthetic_catalog;
    use naves_core::features::{Feature, FeatureKind, Source};
    use naves_core::roadnet::build_graph;
    use naves_core::roadplace::place_network;

    let feature = |way: i64, family: Family, nodes: &[i64], points: &[(f64, f64)]| Feature {
        id: format!("osm/way/{way}/0"),
        way_id: way,
        source: Source::Osm,
        kind: FeatureKind::Road {
            family,
            drivable: true,
            bridge: false,
            tunnel: false,
            roundabout: false,
            layer: 0,
            use_sidepath: false,
            paving: None,
        },
        points: points.to_vec(),
        nodes: nodes.to_vec(),
    };
    // three roads of three families end at one node: no road passes through it
    let graph = build_graph(&[
        feature(1, Family::Asf3, &[1, 2], &[(50.0, 200.0), (200.0, 200.0)]),
        feature(2, Family::City, &[3, 2], &[(200.0, 350.0), (200.0, 200.0)]),
        feature(3, Family::Mud, &[4, 2], &[(350.0, 200.0), (200.0, 200.0)]),
    ]);
    let n = place_network(&graph, &synthetic_catalog()).unwrap();
    assert_eq!(n.joins_without_junction.len(), 1);
    assert_eq!(n.overlays.len(), 0);
}

#[test]
fn a_bridge_that_misses_its_clearance_is_a_finding_that_names_what_was_reached() {
    use naves_core::profile::Clearance;
    let profiles = Profiles {
        clearances: vec![
            Clearance {
                at: (10.0, 20.0),
                reached: 3.2,
                wanted: 4.5,
            },
            Clearance {
                at: (30.0, 40.0),
                reached: 4.48,
                wanted: 4.5,
            },
        ],
        ..Profiles::default()
    };
    let v = run(&net(vec![]), &profiles, &conformed(), &[], &[], &[], &[]);
    let found = only(Rule::BridgeClearance, &v);
    // 4.48 m is within the tolerance of 4.5 m
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].at, (10.0, 20.0));
    assert!(found[0].message.contains("3.2 m"), "{}", found[0].message);
    assert!(found[0].message.contains("4.5 m"), "{}", found[0].message);
}

#[test]
fn a_conflict_that_the_cut_under_a_bridge_causes_names_the_bridge() {
    let mut c = conformed();
    c.conflicts.push(RoadConflict {
        at: (10.0, 20.0),
        height_difference_m: 11.1,
        bridge: true,
    });
    c.conflicts.push(RoadConflict {
        at: (30.0, 40.0),
        height_difference_m: 2.0,
        bridge: false,
    });
    let v = run(&net(vec![]), &Profiles::default(), &c, &[], &[], &[], &[]);
    let found = only(Rule::RoadConflict, &v);
    assert_eq!(found.len(), 2, "{v:?}");
    assert!(
        found[0].message.contains("bridge") && found[0].message.contains("11.1 m"),
        "{}",
        found[0].message
    );
    assert!(
        found[1]
            .message
            .starts_with("Two roads want heights 2.0 m apart"),
        "{}",
        found[1].message
    );
}

/// A strip of the roads `slots` with its joints, bridge flags and half width 3.5.
fn strip(slots: &[usize], joints: &[(f64, f64, f64)], bridge: &[bool]) -> RoadStrip {
    RoadStrip {
        slots: slots.to_vec(),
        joints: joints.to_vec(),
        bridge: bridge.to_vec(),
        junction: Vec::new(),
        side_slope: Vec::new(),
        half_width: 3.5,
        rank: 3,
    }
}

/// A junction of roads that meet without a piece, at `at`, at `height`, of the stretches `stretches`.
fn join(at: (f64, f64), height: f64, stretches: &[&str]) -> Junction {
    Junction {
        at,
        kind: JunctionKind::Join,
        ground: height,
        height,
        rank: 3,
        patch_m: 3.0,
        stretches: stretches.iter().map(|s| (*s).to_string()).collect(),
        lower: std::collections::BTreeSet::new(),
        gradient: (0.0, 0.0),
        tilt_axis: None,
        tilt_cap: 0.0,
    }
}

/// The findings `bridge_end_step` of one deck, the road "s" from (100, 100) to (100, 125) at `deck`, with these
/// junctions and these other strips (each a road from the given joints).
fn end_steps(
    deck: f64,
    junctions: Vec<Junction>,
    others: &[[(f64, f64, f64); 2]],
) -> Vec<Violation> {
    end_steps_of(25.0, deck, junctions, others)
}

/// [`end_steps`] with the deck `length` metres long.
fn end_steps_of(
    length: f64,
    deck: f64,
    junctions: Vec<Junction>,
    others: &[[(f64, f64, f64); 2]],
) -> Vec<Violation> {
    let mut first = road(0, PlacedKind::Bridge, (100.0, 100.0));
    first.end.y = 100.0 + length;
    let mut roads = vec![first];
    let mut strips = vec![strip(
        &[0],
        &[(100.0, 100.0, deck), (100.0, 100.0 + length, deck)],
        &[true],
    )];
    for (i, joints) in others.iter().enumerate() {
        let mut other = road(
            1 + u32::try_from(i).unwrap(),
            PlacedKind::Piece,
            (joints[0].0, joints[0].1),
        );
        "t".clone_into(&mut other.slot.stretch);
        roads.push(other);
        strips.push(strip(&[roads.len() - 1], joints, &[false]));
    }
    let n = net(roads);
    let profiles = Profiles {
        strips,
        ..Profiles::default()
    };
    let junctions = Junctions {
        points: junctions,
        ..Junctions::default()
    };
    let v = check_roads(
        &n,
        &profiles,
        &junctions,
        &conformed(),
        &[],
        &[],
        &[],
        &[],
        &frame(),
        &[],
    );
    v.into_iter()
        .filter(|v| v.rule == Rule::BridgeEndStep)
        .collect()
}

#[test]
fn a_deck_that_ends_1_2_m_off_the_junction_it_joins_is_a_finding_with_the_step_and_the_junction() {
    let found = end_steps(105.0, vec![join((100.0, 126.0), 106.2, &["s"])], &[]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].at, (100.0, 125.0));
    assert!(
        found[0].message.contains("bridge s")
            && found[0].message.contains("1.2 m")
            && found[0].message.contains("junction at (100, 126)"),
        "{}",
        found[0].message
    );
}

#[test]
fn a_step_under_the_tolerance_is_no_finding() {
    for step in [0.0, 0.3, -0.4, 0.5] {
        let found = end_steps(105.0, vec![join((100.0, 125.0), 105.0 + step, &["s"])], &[]);
        assert!(found.is_empty(), "{step}: {found:?}");
    }
    let found = end_steps(105.0, vec![join((100.0, 125.0), 105.6, &["s"])], &[]);
    assert_eq!(found.len(), 1, "{found:?}");
}

#[test]
fn a_junction_that_is_not_the_bridges_or_is_out_of_reach_is_no_step() {
    // 1.2 m off, but the junction belongs to other roads; or the right roads but 10 m away
    let foreign = end_steps(105.0, vec![join((100.0, 126.0), 106.2, &["other"])], &[]);
    assert_eq!(foreign, Vec::new());
    let far = end_steps(105.0, vec![join((100.0, 135.0), 106.2, &["s"])], &[]);
    assert_eq!(far, Vec::new());
}

#[test]
fn a_road_that_ends_at_the_deck_end_at_another_height_is_a_finding_that_names_the_road() {
    let found = end_steps(
        105.0,
        Vec::new(),
        &[[(100.0, 125.5, 107.0), (100.0, 150.0, 107.0)]],
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].message.contains("2.0 m") && found[0].message.contains("road t"),
        "{}",
        found[0].message
    );
    // 30 m away it is not the deck's neighbour
    let far = end_steps(
        105.0,
        Vec::new(),
        &[[(100.0, 155.0, 107.0), (100.0, 180.0, 107.0)]],
    );
    assert_eq!(far, Vec::new());
}

#[test]
fn a_deck_between_two_junctions_at_different_heights_reports_the_end_that_has_the_step() {
    let found = end_steps(
        105.0,
        vec![
            join((100.0, 100.0), 105.0, &["s"]),
            join((100.0, 125.0), 110.0, &["s"]),
        ],
        &[],
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].at, (100.0, 125.0));
    assert!(found[0].message.contains("5.0 m"), "{}", found[0].message);
}

#[test]
fn heights_that_are_not_numbers_make_no_end_step() {
    for (deck, other) in [(f64::NAN, 105.0), (105.0, f64::NAN), (f64::INFINITY, 105.0)] {
        let found = end_steps(
            deck,
            vec![join((100.0, 126.0), other, &["s"])],
            &[[(100.0, 125.0, other), (100.0, 150.0, 105.0)]],
        );
        assert!(found.is_empty(), "{deck} {other}: {found:?}");
    }
}

#[test]
fn a_short_decks_far_end_is_not_compared_with_the_junction_at_its_near_end() {
    // a 6 m deck with a junction at its far end 3 m up: the near end is 6 m from it, within the reach, but the
    // junction is nearer the other end
    let found = end_steps_of(6.0, 105.0, vec![join((100.0, 106.0), 108.0, &["s"])], &[]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].at, (100.0, 106.0));
}

/// A chain of three roads "s": a piece, the bridge from (100, 100) to (100, 125), a piece, the strip through
/// `joints`, and the end steps of it over the tolerance with `junctions`.
fn chain_steps(
    joints: &[(f64, f64, f64)],
    junctions: Vec<Junction>,
) -> Vec<naves_core::validate::EndStep> {
    let mut roads = vec![
        road(0, PlacedKind::Piece, (100.0, 75.0)),
        road(1, PlacedKind::Bridge, (100.0, 100.0)),
        road(2, PlacedKind::Piece, (100.0, 125.0)),
    ];
    for r in &mut roads {
        "s".clone_into(&mut r.slot.stretch);
    }
    let n = net(roads);
    let profiles = Profiles {
        strips: vec![strip(&[0, 1, 2], joints, &[false, true, false])],
        ..Profiles::default()
    };
    let junctions = Junctions {
        points: junctions,
        ..Junctions::default()
    };
    naves_core::validate::bridge_end_steps(&n, &profiles, &junctions)
        .into_iter()
        .filter(|e| e.step.abs() > naves_core::validate::END_STEP_TOL_M)
        .collect()
}

/// The chain's joints with the strip at `h` beyond the deck, plus `extra` joints in between.
fn flat_chain(h: f64, extra: &[(f64, f64, f64)]) -> Vec<(f64, f64, f64)> {
    let mut joints = vec![
        (100.0, 75.0, 105.0),
        (100.0, 100.0, 105.0),
        (100.0, 125.0, 105.0),
    ];
    joints.extend_from_slice(extra);
    joints.push((100.0, 150.0, h));
    joints
}

#[test]
fn a_deck_inside_a_chain_is_compared_within_the_junctions_patch() {
    // the deck is the middle of three roads: held to a junction within its patch (3 m)
    let steps = chain_steps(
        &flat_chain(105.0, &[]),
        vec![join((100.0, 128.0), 107.0, &["s"])],
    );
    assert_eq!(steps.len(), 1, "{steps:?}");
    // a junction 15 m away is beyond the patch, and the strip that runs through its patch is compared with its
    // plane (the height solve may have given the hold up); one 65 m away is out of reach altogether
    let steps = chain_steps(
        &flat_chain(105.0, &[]),
        vec![join((100.0, 140.0), 107.0, &["s"])],
    );
    assert_eq!(steps.len(), 1, "{steps:?}");
    assert!((steps[0].step - 2.0).abs() < 1e-6, "{steps:?}");
    let steps = chain_steps(
        &flat_chain(105.0, &[]),
        vec![join((100.0, 190.0), 107.0, &["s"])],
    );
    assert_eq!(steps, Vec::new());
}

#[test]
fn a_deck_inside_a_chain_whose_strip_enters_the_junction_beyond_the_patch_off_its_plane_is_a_step()
{
    // the junction lies 6 m from the deck end, 3 m beyond its patch, and the strip passes through it 2 m below
    // the plane, held to nothing there: the break the deck's own joint does not show
    let steps = chain_steps(
        &flat_chain(105.0, &[]),
        vec![join((100.0, 131.0), 107.0, &["s"])],
    );
    assert_eq!(steps.len(), 1, "{steps:?}");
    assert_eq!(steps[0].end, 1);
    assert!((steps[0].step - 2.0).abs() < 1e-6, "{steps:?}");
    assert!(
        steps[0].what.contains("junction at (100, 131)"),
        "{steps:?}"
    );
}

#[test]
fn the_strip_is_read_backwards_at_the_first_end_of_a_deck_and_over_a_joint_that_repeats() {
    // the junction 6 m before the deck's first end, the strip 2 m under its plane there
    let steps = chain_steps(
        &flat_chain(105.0, &[]),
        vec![join((100.0, 94.0), 107.0, &["s"])],
    );
    assert_eq!(steps.len(), 1, "{steps:?}");
    assert_eq!(steps[0].end, 0);
    assert!((steps[0].step - 2.0).abs() < 1e-6, "{steps:?}");
    // a joint twice at one place (a segment without length) changes nothing
    let joints = flat_chain(105.0, &[(100.0, 125.0, 105.0)]);
    let steps = chain_steps(&joints, vec![join((100.0, 131.0), 107.0, &["s"])]);
    assert_eq!(steps.len(), 1, "{steps:?}");
    // the strip ends 4 m from the junction, outside its patch: it does not run through it, nothing to compare
    let ends = [
        (100.0, 75.0, 105.0),
        (100.0, 100.0, 105.0),
        (100.0, 125.0, 105.0),
        (100.0, 127.0, 105.0),
    ];
    let steps = chain_steps(&ends, vec![join((100.0, 131.0), 107.0, &["s"])]);
    assert_eq!(steps, Vec::new());
}

#[test]
fn a_strip_that_is_on_the_junctions_plane_where_it_enters_the_patch_is_no_step_however_it_ramps_before()
 {
    // a ramp of 1 m in 3 m from the deck to the edge of the patch (road_grade's business), then on the plane
    let joints = flat_chain(106.0, &[(100.0, 128.0, 106.0)]);
    let steps = chain_steps(&joints, vec![join((100.0, 131.0), 106.0, &["s"])]);
    assert_eq!(steps, Vec::new());
}

#[test]
fn a_junction_plane_extrapolated_beyond_its_patch_is_no_step() {
    // the plane rises 0.5 m per metre: 3 m below the junction it would be 1.5 m under the junction's height, and the
    // strip is on it (105.5 m) where it enters the patch, 3 m after the deck's end
    let mut j = join((100.0, 131.0), 107.0, &["s"]);
    j.gradient = (0.0, 0.5);
    let joints = flat_chain(107.0, &[(100.0, 128.0, 105.5)]);
    let steps = chain_steps(&joints, vec![j]);
    assert_eq!(steps, Vec::new());
}

#[test]
fn a_strip_that_is_on_the_plane_only_at_the_middle_of_a_patch_next_to_the_deck_is_a_step_at_its_edge()
 {
    // the patch (3 m) starts 3 m after the deck's end, so no joint fits between them and the solve ties the strip to
    // the plane where it enters the patch; the strip climbs from 105 m to the plane (107 m) at the junction's centre,
    // so it is 1 m under the plane at the patch's edge, where the step is measured (at the closest approach, the
    // centre, it would be none)
    let joints = flat_chain(107.0, &[(100.0, 131.0, 107.0)]);
    let steps = chain_steps(&joints, vec![join((100.0, 131.0), 107.0, &["s"])]);
    assert_eq!(steps.len(), 1, "{steps:?}");
    assert!((steps[0].step - 1.0).abs() < 1e-6, "{steps:?}");
}

#[test]
fn the_step_is_measured_at_the_patch_edge_over_several_joints_and_further_out_at_the_closest_approach()
 {
    // joints before the patch's edge do not change where it is: 105 m at 125 and 127, the patch (3 m around 131)
    // starts at 128, on the way from 127 (105 m) to 129 (107 m): 106 m there, 1 m under the plane
    let joints = flat_chain(107.0, &[(100.0, 127.0, 105.0), (100.0, 129.0, 107.0)]);
    let steps = chain_steps(&joints, vec![join((100.0, 131.0), 107.0, &["s"])]);
    assert_eq!(steps.len(), 1, "{steps:?}");
    assert!((steps[0].step - 1.0).abs() < 1e-6, "{steps:?}");
    // a patch that starts 5 m after the deck's end (a joint fits there) is compared where the strip passes nearest
    // the junction, as before: the strip is 105 m to 129 and then rises to 107 m at 150, 105.38 m at 133
    let joints = flat_chain(107.0, &[(100.0, 127.0, 105.0), (100.0, 129.0, 105.0)]);
    let steps = chain_steps(&joints, vec![join((100.0, 133.0), 107.0, &["s"])]);
    assert_eq!(steps.len(), 1, "{steps:?}");
    assert!(
        (steps[0].step - (107.0 - (105.0 + 2.0 * 4.0 / 21.0))).abs() < 1e-6,
        "{steps:?}"
    );
}

#[test]
fn a_deck_with_both_ends_in_one_patch_is_compared_at_both_ends() {
    // a 2 m deck with a junction between its ends, 1 m from each: neither end is "nearer"
    let found = end_steps_of(2.0, 105.0, vec![join((100.0, 101.0), 107.0, &["s"])], &[]);
    assert_eq!(found.len(), 2, "{found:?}");
}

#[test]
fn a_grade_break_in_a_ramp_zone_is_worded_as_a_change_of_grade() {
    let profiles = Profiles {
        excesses: vec![GradeExcess {
            at: (5.0, 6.0),
            kind: GradeExcessKind::GradeBreak,
            grade: 0.14,
            limit: 0.10,
        }],
        ..Profiles::default()
    };
    let v = run(&net(vec![]), &profiles, &conformed(), &[], &[], &[], &[]);
    let found = only(Rule::RoadGrade, &v);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].message,
        "The grade changes by 14 % at one point, the most a vehicle takes there is 10 %"
    );
}

#[test]
fn the_water_test_follows_the_rule_of_road_in_water() {
    let mut holed = lake();
    // a hole around the centre of the lake (about 110 m across)
    holed.holes = vec![vec![
        (47.5495, 12.9735),
        (47.5495, 12.9765),
        (47.5505, 12.9765),
        (47.5505, 12.9735),
        (47.5495, 12.9735),
    ]];
    let mut town = lake();
    town.priority = 1;
    // a ring that cannot be projected is left out, not a reason to lose the others
    let mut broken = lake();
    broken.outer = vec![(f64::NAN, 12.97), (47.55, 12.98), (47.56, 12.97)];
    let frame = frame();
    let in_lake = water_test(&[lake(), town.clone(), broken], &frame);
    assert!(in_lake((250.0, 250.0)), "the middle of the lake");
    assert!(!in_lake((50.0, 50.0)), "dry land");
    // built-up ground (priority 1) is no water
    assert!(!water_test(&[town], &frame)((250.0, 250.0)));
    // holes are no water
    let with_hole = water_test(&[holed], &frame);
    assert!(!with_hole((250.0, 250.0)), "inside the hole");
    // the map is half the size of the real ground: 80 map metres from the centre is within the lake
    assert!(with_hole((250.0 + 80.0, 250.0)), "between hole and shore");
}

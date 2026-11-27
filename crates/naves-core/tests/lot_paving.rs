// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Paved or grass: the surface class of every lot that is not excluded.

#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::catalog::Family;
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::lot::{
    Assessed, LotArea, LotKind, NO_GROUP, Outcome, Paving, Placement, surfaces, surfaces_with,
};
use naves_core::netclean::clean_with;
use naves_core::netclean::lot::groups;
use naves_core::roadnet::build_graph;
use naves_core::surface::SurfaceClass;

type Pt = (f64, f64);

/// The terrain cell, in metres.
const CELL: f64 = 4.0;

fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Pt> {
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]
}

/// The area of a rectangle given as a closed ring of four corners.
fn rectangle_area(ring: &[Pt]) -> f64 {
    ((ring[2].0 - ring[0].0) * (ring[2].1 - ring[0].1)).abs()
}

fn lot(id: &str, kind: LotKind, outer: Vec<Pt>, tag: Option<Paving>) -> Assessed {
    let area_m2 = rectangle_area(&outer);
    Assessed {
        area: LotArea {
            id: id.to_string(),
            kind,
            placement: Placement::Ground,
            outer,
            holes: Vec::new(),
            area_m2,
            surface_tag: tag,
            holes_dropped: 0,
        },
        group: NO_GROUP,
        outcome: Outcome::SurfaceOnly {
            reason: "no_vertex",
        },
        steep: false,
        move_p95_m: 0.0,
        move_max_m: 0.0,
        slope_p95: 0.0,
        relief_before_m: 0.0,
        relief_after_m: 0.0,
        range_before: (0.0, 0.0),
        range_after: (0.0, 0.0),
        water_share: 0.0,
    }
}

fn car_park(id: &str, outer: Vec<Pt>) -> Assessed {
    lot(id, LotKind::CarPark, outer, None)
}

/// A road of one way; the nodes are made from the way id.
fn road(way: i64, points: &[Pt], family: Family, paving: Option<Paving>) -> Feature {
    Feature {
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
            paving,
        },
        points: points.to_vec(),
        nodes: (0..points.len())
            .map(|i| way * 100 + i64::try_from(i).unwrap())
            .collect(),
    }
}

/// A road with the node ids given.
fn joined(
    way: i64,
    nodes: &[i64],
    points: &[Pt],
    family: Family,
    paving: Option<Paving>,
) -> Feature {
    Feature {
        nodes: nodes.to_vec(),
        ..road(way, points, family, paving)
    }
}

/// A horizontal road at `y` from `x0` to `x1`.
fn along(way: i64, y: f64, x0: f64, x1: f64, family: Family, paving: Option<Paving>) -> Feature {
    road(way, &[(x0, y), (x1, y)], family, paving)
}

/// The decision for `lots` with the roads `features`, the way the pipeline gets there: the graph, the lot step, the
/// decision from what it reports.
fn decide(lots: &[Assessed], features: &[Feature]) -> Vec<(String, SurfaceClass)> {
    let graph = build_graph(features);
    let (_, report) = clean_with(&graph, &|f: Family| f.width_m(), CELL, &groups(lots));
    surfaces(lots, &graph, &report, CELL)
}

fn class_of(decided: &[(String, SurfaceClass)], id: &str) -> Option<SurfaceClass> {
    decided.iter().find(|(i, _)| i == id).map(|(_, c)| *c)
}

const PAVED: SurfaceClass = SurfaceClass::Tarmac;
const GRASS: SurfaceClass = SurfaceClass::Grass;
const ASPHALT: Option<Paving> = Some(Paving::Paved);
const GRAVEL: Option<Paving> = Some(Paving::Unpaved);

/// A lot of 600 m a side, big enough for the long roads of the example.
fn big(id: &str) -> Assessed {
    car_park(id, square(0.0, 0.0, 600.0, 600.0))
}

#[test]
fn the_lots_tag_wins_over_its_roads() {
    let tagged = lot(
        "osm/way/1",
        LotKind::CarPark,
        square(0.0, 0.0, 600.0, 600.0),
        Some(Paving::Unpaved),
    );
    let decided = decide(
        &[tagged],
        &[along(10, 300.0, 10.0, 110.0, Family::Asf3, ASPHALT)],
    );
    assert_eq!(class_of(&decided, "osm/way/1"), Some(GRASS));
    let tagged = lot(
        "osm/way/1",
        LotKind::CarPark,
        square(0.0, 0.0, 600.0, 600.0),
        Some(Paving::Paved),
    );
    let decided = decide(
        &[tagged],
        &[along(10, 300.0, 10.0, 110.0, Family::Mud, GRAVEL)],
    );
    assert_eq!(class_of(&decided, "osm/way/1"), Some(PAVED));
}

#[test]
fn the_grafenlehen_roads_make_grass_by_metres_not_by_family() {
    // 404 m tagged fine gravel, 34 m asphalt and 132 m untagged City, all inside the lot
    let features = [
        along(10, 100.0, 10.0, 414.0, Family::Grav, GRAVEL),
        along(11, 200.0, 10.0, 44.0, Family::Asf3, ASPHALT),
        along(12, 300.0, 10.0, 142.0, Family::City, None),
    ];
    let decided = decide(&[big("osm/way/1")], &features);
    assert_eq!(class_of(&decided, "osm/way/1"), Some(GRASS));
}

#[test]
fn untagged_roads_count_by_their_family() {
    let decided = decide(
        &[big("osm/way/1")],
        &[along(10, 300.0, 10.0, 94.0, Family::City, None)],
    );
    assert_eq!(class_of(&decided, "osm/way/1"), Some(PAVED), "84 m City");
    let decided = decide(
        &[big("osm/way/1")],
        &[along(10, 300.0, 10.0, 94.0, Family::Mud, None)],
    );
    assert_eq!(class_of(&decided, "osm/way/1"), Some(GRASS), "84 m Mud");
}

#[test]
fn equal_metres_of_paved_and_grass_are_paved() {
    let features = [
        along(10, 100.0, 10.0, 35.0, Family::Asf3, None),
        along(11, 200.0, 10.0, 35.0, Family::Mud, None),
    ];
    let decided = decide(&[big("osm/way/1")], &features);
    assert_eq!(class_of(&decided, "osm/way/1"), Some(PAVED));
}

/// A lot 40 m a side with its south edge at y = 0.
fn small() -> Assessed {
    car_park("osm/way/1", square(0.0, 0.0, 40.0, 40.0))
}

#[test]
fn a_lot_without_roads_takes_the_street_beside_it() {
    let paved_street = along(10, -3.0, -50.0, 90.0, Family::Asf3, None);
    let decided = decide(&[small()], &[paved_street]);
    assert_eq!(class_of(&decided, "osm/way/1"), Some(PAVED));
    // a street within a metre of the edge has no road inside and none entering
    let at_the_edge = along(10, -0.5, -50.0, 90.0, Family::Asf3, None);
    let decided = decide(&[small()], &[at_the_edge]);
    assert_eq!(class_of(&decided, "osm/way/1"), Some(PAVED));
    let track = along(10, -3.0, -50.0, 90.0, Family::Mud, None);
    let decided = decide(&[small()], &[track]);
    assert_eq!(class_of(&decided, "osm/way/1"), Some(GRASS));
}

#[test]
fn only_a_road_whose_earthwork_reaches_the_lot_serves_it() {
    // an Asf3 road: half its width 3.5 m, the shoulder 1.5 cells and one cell: 13.5 m at a 4 m cell
    let near = along(10, -13.0, -50.0, 90.0, Family::Asf3, None);
    assert_eq!(
        class_of(&decide(&[small()], &[near]), "osm/way/1"),
        Some(PAVED)
    );
    let far = along(10, -15.0, -50.0, 90.0, Family::Asf3, None);
    assert_eq!(
        class_of(&decide(&[small()], &[far]), "osm/way/1"),
        Some(GRASS)
    );
    assert_eq!(class_of(&decide(&[small()], &[]), "osm/way/1"), Some(GRASS));
}

#[test]
fn the_metres_beside_are_counted_within_the_reach_only() {
    // a long paved street out of reach (20 m) against a short track along the edge: the track alone serves the lot
    let street = along(10, -20.0, -100.0, 140.0, Family::Asf3, None);
    let track = along(11, -3.0, 0.0, 10.0, Family::Mud, None);
    let decided = decide(&[small()], &[street, track]);
    assert_eq!(class_of(&decided, "osm/way/1"), Some(GRASS));
    // the same street, a track that is much shorter within reach than the street is: the street counts only
    // where it is within reach, 40 m of it against 10 m of track
    let street = along(10, -10.0, -100.0, 140.0, Family::Asf3, None);
    let track = along(11, -3.0, 0.0, 10.0, Family::Mud, None);
    let decided = decide(&[small()], &[street, track]);
    assert_eq!(class_of(&decided, "osm/way/1"), Some(PAVED));
}

#[test]
fn a_member_without_roads_takes_the_decision_of_its_group() {
    let a = car_park("osm/way/1", square(0.0, 0.0, 100.0, 100.0));
    let b = car_park("osm/way/2", square(100.0, 0.0, 140.0, 100.0));
    let c = car_park("osm/way/3", square(500.0, 500.0, 540.0, 540.0));
    let decided = decide(
        &[a, b, c],
        &[along(10, 50.0, 10.0, 60.0, Family::Asf3, ASPHALT)],
    );
    assert_eq!(class_of(&decided, "osm/way/1"), Some(PAVED));
    assert_eq!(class_of(&decided, "osm/way/2"), Some(PAVED), "same group");
    assert_eq!(class_of(&decided, "osm/way/3"), Some(GRASS), "alone");
}

#[test]
fn a_car_park_inside_a_camp_site_has_its_own_ground() {
    let camp = lot(
        "osm/way/1",
        LotKind::CampSite,
        square(0.0, 0.0, 300.0, 300.0),
        None,
    );
    let park = car_park("osm/way/2", square(100.0, 100.0, 160.0, 160.0));
    let features = [
        // gravel roads in the camp site, a paved road inside the car park
        along(10, 250.0, 10.0, 90.0, Family::Grav, GRAVEL),
        along(11, 130.0, 110.0, 150.0, Family::Asf3, ASPHALT),
    ];
    let decided = decide(&[camp, park], &features);
    assert_eq!(class_of(&decided, "osm/way/2"), Some(PAVED), "car park");
    assert_eq!(class_of(&decided, "osm/way/1"), Some(GRASS), "camp site");
}

#[test]
fn a_camp_site_is_grass_whatever_its_roads_are_unless_it_is_tagged_paved() {
    let features = [along(10, 150.0, 10.0, 200.0, Family::Asf3, ASPHALT)];
    let camp = |tag| {
        lot(
            "osm/way/1",
            LotKind::CampSite,
            square(0.0, 0.0, 300.0, 300.0),
            tag,
        )
    };
    assert_eq!(
        class_of(&decide(&[camp(None)], &features), "osm/way/1"),
        Some(GRASS),
        "asphalt roads"
    );
    assert_eq!(
        class_of(
            &decide(&[camp(Some(Paving::Paved))], &features),
            "osm/way/1"
        ),
        Some(PAVED),
        "tagged paved"
    );
    assert_eq!(
        class_of(
            &decide(&[camp(Some(Paving::Unpaved))], &features),
            "osm/way/1"
        ),
        Some(GRASS),
        "tagged unpaved"
    );
    // a camp site beside a paved street: grass as well
    let street = [along(10, -3.0, -50.0, 90.0, Family::Asf3, None)];
    let small_camp = lot(
        "osm/way/1",
        LotKind::CampSite,
        square(0.0, 0.0, 40.0, 40.0),
        None,
    );
    assert_eq!(
        class_of(&decide(&[small_camp], &street), "osm/way/1"),
        Some(GRASS)
    );
}

#[test]
fn caravan_sites_rest_areas_and_bus_parking_follow_their_roads() {
    for kind in [LotKind::CaravanSite, LotKind::RestArea, LotKind::BusParking] {
        let features = [along(10, 300.0, 10.0, 94.0, Family::City, None)];
        let lots = [lot("osm/way/1", kind, square(0.0, 0.0, 600.0, 600.0), None)];
        assert_eq!(
            class_of(&decide(&lots, &features), "osm/way/1"),
            Some(PAVED),
            "{kind:?}"
        );
    }
}

#[test]
fn a_street_strip_beside_a_city_road_is_paved() {
    let mut strip = car_park("osm/way/1", square(0.0, 5.0, 40.0, 7.0));
    strip.area.placement = Placement::StreetSide;
    strip.outcome = Outcome::SurfaceOnly {
        reason: "street_side",
    };
    let decided = decide(&[strip], &[along(10, 0.0, -50.0, 90.0, Family::City, None)]);
    assert_eq!(class_of(&decided, "osm/way/1"), Some(PAVED));
}

#[test]
fn an_access_road_counts_by_the_part_inside_the_lot() {
    // a street at y = -30, a track and an asphalt road from it into the lot, each ending at (x, 20)
    let street = along(10, -30.0, -60.0, 100.0, Family::Asf3, ASPHALT);
    let mud = road(11, &[(20.0, -30.0), (20.0, 20.0)], Family::Mud, GRAVEL);
    let decided = decide(&[small()], &[street.clone(), mud]);
    assert_eq!(class_of(&decided, "osm/way/1"), Some(GRASS), "a track in");
    let asphalt = road(11, &[(20.0, -30.0), (20.0, 20.0)], Family::Asf3, ASPHALT);
    let decided = decide(&[small()], &[street, asphalt]);
    assert_eq!(class_of(&decided, "osm/way/1"), Some(PAVED), "a road in");
}

#[test]
fn the_metres_inside_are_clipped_per_way_when_the_roads_are_given() {
    // one stretch of two ways from a street: 200 m of asphalt outside the lot, then 50 m of gravel of which 20 m
    // are inside the lot
    let street = joined(
        10,
        &[1, 2, 3],
        &[(-60.0, -230.0), (20.0, -230.0), (100.0, -230.0)],
        Family::Asf3,
        ASPHALT,
    );
    let way_a = joined(
        11,
        &[2, 4],
        &[(20.0, -230.0), (20.0, -30.0)],
        Family::Asf3,
        ASPHALT,
    );
    let way_b = joined(
        12,
        &[4, 5],
        &[(20.0, -30.0), (20.0, 20.0)],
        Family::Asf3,
        GRAVEL,
    );
    let features = [street, way_a, way_b];
    let lots = [small()];
    let graph = build_graph(&features);
    let (_, report) = clean_with(&graph, &|f: Family| f.width_m(), CELL, &groups(&lots));
    assert_eq!(report.lot_access.len(), 1, "{:?}", report.lot_access);
    // by the stretch's average the road is mostly paved
    assert_eq!(
        class_of(&surfaces(&lots, &graph, &report, CELL), "osm/way/1"),
        Some(PAVED)
    );
    // by the way that lies inside, it is gravel
    assert_eq!(
        class_of(
            &surfaces_with(&lots, &graph, &report, CELL, &features),
            "osm/way/1"
        ),
        Some(GRASS)
    );
}

#[test]
fn excluded_lots_get_no_class_and_the_rest_come_in_id_order() {
    let mut water = car_park("osm/way/5", square(300.0, 300.0, 340.0, 340.0));
    water.outcome = Outcome::Excluded { reason: "in_water" };
    let mut deck = car_park("osm/way/6", square(400.0, 300.0, 440.0, 340.0));
    deck.area.placement = Placement::NotAtGround;
    deck.outcome = Outcome::Excluded {
        reason: "not_at_ground",
    };
    let lots = [
        car_park("osm/way/9", square(0.0, 0.0, 40.0, 40.0)),
        water,
        deck,
        car_park("osm/way/2", square(100.0, 0.0, 140.0, 40.0)),
    ];
    let decided = decide(&lots, &[]);
    let ids: Vec<&str> = decided.iter().map(|(i, _)| i.as_str()).collect();
    assert_eq!(ids, ["osm/way/2", "osm/way/9"]);
}

#[test]
fn degenerate_input_decides_grass_and_does_not_panic() {
    let mut nan = car_park("osm/way/1", square(0.0, 0.0, 40.0, 40.0));
    nan.area.outer[1].0 = f64::NAN;
    let features = [along(10, -3.0, -50.0, 90.0, Family::Asf3, None)];
    // what a lot that is not a number gets is not specified, only that it gets something
    assert_eq!(decide(&[nan], &features).len(), 1);
    assert_eq!(decide(&[], &features).len(), 0);
    let graph = build_graph(&features);
    let lots = [small()];
    let (_, report) = clean_with(&graph, &|f: Family| f.width_m(), CELL, &groups(&lots));
    let decided = surfaces(&lots, &graph, &report, f64::NAN);
    assert_eq!(decided.len(), 1);
}

// ---- fix round 1 ----

#[test]
fn a_stretch_with_a_share_that_is_not_a_number_does_not_panic() {
    let lots = [small()];
    let features = [along(10, -3.0, -50.0, 90.0, Family::Asf3, ASPHALT)];
    let mut graph = build_graph(&features);
    for stretch in &mut graph.stretches {
        stretch.paved_m = f64::NAN;
        stretch.unpaved_m = f64::INFINITY;
    }
    let (_, report) = clean_with(&graph, &|f: Family| f.width_m(), CELL, &groups(&lots));
    assert_eq!(surfaces(&lots, &graph, &report, CELL).len(), 1);
}

#[test]
fn a_lot_that_is_not_a_number_is_grass() {
    let mut nan = car_park("osm/way/1", square(0.0, 0.0, 40.0, 40.0));
    nan.area.outer[1].0 = f64::NAN;
    let decided = decide(
        &[nan],
        &[along(10, -3.0, -50.0, 90.0, Family::Asf3, ASPHALT)],
    );
    assert_eq!(class_of(&decided, "osm/way/1"), Some(GRASS));
}

#[test]
fn a_piece_with_no_road_feature_counts_by_the_stretchs_shares() {
    // a dirt-track family tagged asphalt, features that do not hold the way: the stretch's own shares say paved
    let street = along(10, -30.0, -60.0, 100.0, Family::Asf3, ASPHALT);
    let access = road(11, &[(20.0, -30.0), (20.0, 20.0)], Family::Mud, ASPHALT);
    let lots = [small()];
    let graph = build_graph(&[street, access]);
    let (_, report) = clean_with(&graph, &|f: Family| f.width_m(), CELL, &groups(&lots));
    let elsewhere = [along(99, 900.0, 800.0, 900.0, Family::Mud, GRAVEL)];
    assert_eq!(
        class_of(
            &surfaces_with(&lots, &graph, &report, CELL, &elsewhere),
            "osm/way/1"
        ),
        Some(PAVED)
    );
}

#[test]
fn own_roads_come_before_the_streets_beside() {
    // a 20 m track inside, a long paved street along the edge: the track serves the lot
    let features = [
        along(10, 20.0, 10.0, 30.0, Family::Mud, GRAVEL),
        along(11, -3.0, -50.0, 90.0, Family::Asf3, ASPHALT),
    ];
    let decided = decide(&[small()], &features);
    assert_eq!(class_of(&decided, "osm/way/1"), Some(GRASS));
}

#[test]
fn the_group_comes_before_the_streets_beside() {
    let a = car_park("osm/way/1", square(0.0, 0.0, 100.0, 100.0));
    let b = car_park("osm/way/2", square(100.0, 0.0, 140.0, 100.0));
    let features = [
        along(10, 50.0, 10.0, 60.0, Family::Asf3, ASPHALT),
        along(11, -3.0, 110.0, 140.0, Family::Mud, GRAVEL),
    ];
    let decided = decide(&[a, b], &features);
    assert_eq!(class_of(&decided, "osm/way/2"), Some(PAVED));
}

#[test]
fn tagged_and_untagged_metres_count_each_by_their_own_kind() {
    // 30 m tagged gravel and 100 m of untagged City: the City metres are paved by their family
    let features = [
        along(10, 100.0, 10.0, 40.0, Family::Grav, GRAVEL),
        along(11, 200.0, 10.0, 110.0, Family::City, None),
    ];
    let decided = decide(&[big("osm/way/1")], &features);
    assert_eq!(class_of(&decided, "osm/way/1"), Some(PAVED));
}

#[test]
fn a_through_road_kept_over_the_lot_counts_as_a_road_that_serves_it() {
    // an asphalt road crosses the lot from west to east (80 m of it inside), and a 20 m gravel stub inside is removed
    let features = [
        joined(
            1,
            &[1, 2],
            &[(-50.0, 50.0), (10.0, 50.0)],
            Family::Asf2,
            ASPHALT,
        ),
        joined(
            2,
            &[2, 3],
            &[(10.0, 50.0), (90.0, 50.0)],
            Family::Asf3,
            ASPHALT,
        ),
        joined(
            3,
            &[3, 4],
            &[(90.0, 50.0), (150.0, 50.0)],
            Family::Asf2,
            ASPHALT,
        ),
        joined(
            4,
            &[5, 6],
            &[(20.0, 80.0), (40.0, 80.0)],
            Family::Mud,
            GRAVEL,
        ),
    ];
    let lots = [car_park("osm/way/1", square(0.0, 0.0, 100.0, 100.0))];
    let graph = build_graph(&features);
    let (_, report) = clean_with(&graph, &|f: Family| f.width_m(), CELL, &groups(&lots));
    assert_eq!(report.lot_through, ["2:2-3"], "{:?}", report.lot_roads);
    assert_eq!(report.lot_roads.len(), 1);
    assert_eq!(
        class_of(&surfaces(&lots, &graph, &report, CELL), "osm/way/1"),
        Some(PAVED)
    );
}

#[test]
fn a_group_without_roads_decides_by_the_streets_beside_all_its_members() {
    // a street beside the first car park only; the second is 30 m from it
    let a = car_park("osm/way/1", square(0.0, 0.0, 40.0, 40.0));
    let b = car_park("osm/way/2", square(40.0, 0.0, 80.0, 40.0));
    let street = along(10, -3.0, -50.0, 10.0, Family::Asf3, None);
    let decided = decide(&[a.clone(), b.clone()], std::slice::from_ref(&street));
    assert_eq!(class_of(&decided, "osm/way/1"), Some(PAVED));
    assert_eq!(class_of(&decided, "osm/way/2"), Some(PAVED), "same group");
    // alone, the second one is grass: nothing reaches it
    assert_eq!(
        class_of(
            &decide(std::slice::from_ref(&b), std::slice::from_ref(&street)),
            "osm/way/2"
        ),
        Some(GRASS)
    );
    // a member with a tag of its own follows its tag
    let mut tagged = b;
    tagged.area.surface_tag = Some(Paving::Unpaved);
    let decided = decide(&[a, tagged], &[street]);
    assert_eq!(class_of(&decided, "osm/way/1"), Some(PAVED));
    assert_eq!(class_of(&decided, "osm/way/2"), Some(GRASS), "its own tag");
}

#[test]
fn with_the_road_features_a_way_counts_by_its_own_tag_or_by_its_family() {
    let street = along(10, -30.0, -60.0, 100.0, Family::Asf3, ASPHALT);
    let lots = [small()];
    let class_with = |access: Feature| {
        let features = [street.clone(), access];
        let graph = build_graph(&features);
        let (_, report) = clean_with(&graph, &|f: Family| f.width_m(), CELL, &groups(&lots));
        class_of(
            &surfaces_with(&lots, &graph, &report, CELL, &features),
            "osm/way/1",
        )
    };
    let into = [(20.0, -30.0), (20.0, 20.0)];
    assert_eq!(
        class_with(road(11, &into, Family::Mud, ASPHALT)),
        Some(PAVED),
        "tagged paved on a track"
    );
    assert_eq!(
        class_with(road(11, &into, Family::City, None)),
        Some(PAVED),
        "untagged City"
    );
    assert_eq!(
        class_with(road(11, &into, Family::Mud, None)),
        Some(GRASS),
        "untagged Mud"
    );
}

#[test]
fn stretches_that_cannot_be_measured_count_nothing() {
    let features = [
        along(10, -3.0, -50.0, 90.0, Family::Asf3, None),
        along(11, -5.0, -50.0, 90.0, Family::Asf3, None),
    ];
    let mut graph = build_graph(&features);
    // one stretch with a point that is not a number, one of no length
    graph.stretches[0].points[0].0 = f64::NAN;
    graph.stretches[1].points = vec![(10.0, -5.0), (10.0, -5.0)];
    let lots = [small()];
    let (_, report) = clean_with(&graph, &|f: Family| f.width_m(), CELL, &groups(&lots));
    let decided = surfaces(&lots, &graph, &report, CELL);
    assert_eq!(class_of(&decided, "osm/way/1"), Some(GRASS));
}

#[test]
fn a_through_road_of_nested_lots_belongs_to_the_smallest_and_unknown_ids_are_ignored() {
    let features = [
        joined(
            1,
            &[1, 2],
            &[(-50.0, 50.0), (10.0, 50.0)],
            Family::Asf2,
            ASPHALT,
        ),
        joined(
            2,
            &[2, 3],
            &[(10.0, 50.0), (90.0, 50.0)],
            Family::Asf3,
            ASPHALT,
        ),
        joined(
            3,
            &[3, 4],
            &[(90.0, 50.0), (150.0, 50.0)],
            Family::Asf2,
            ASPHALT,
        ),
        // 60 m of gravel of the large lot's own, removed and outside the small lot
        along(20, 120.0, -10.0, 50.0, Family::Mud, GRAVEL),
    ];
    let lots = [
        car_park("osm/way/1", square(0.0, 0.0, 100.0, 100.0)),
        car_park("osm/way/2", square(-20.0, -20.0, 140.0, 140.0)),
    ];
    let graph = build_graph(&features);
    let (_, mut report) = clean_with(&graph, &|f: Family| f.width_m(), CELL, &groups(&lots));
    report.lot_through.push("no/such/stretch".to_string());
    let decided = surfaces(&lots, &graph, &report, CELL);
    // the smallest lot holds the road's middle: 80 m of asphalt against nothing, and the large lot's own gravel
    // leaves it grass. Were the largest lot the owner, it would be paved (80 m against 60 m).
    assert_eq!(class_of(&decided, "osm/way/1"), Some(PAVED));
    assert_eq!(class_of(&decided, "osm/way/2"), Some(GRASS), "the largest");
}

#[test]
fn a_feature_with_one_point_is_no_way_of_a_stretch() {
    let street = along(10, -30.0, -60.0, 100.0, Family::Asf3, ASPHALT);
    let access = road(11, &[(20.0, -30.0), (20.0, 20.0)], Family::Mud, GRAVEL);
    let stray = road(12, &[(20.0, 0.0)], Family::Mud, ASPHALT);
    let lots = [small()];
    let graph = build_graph(&[street.clone(), access.clone()]);
    let (_, report) = clean_with(&graph, &|f: Family| f.width_m(), CELL, &groups(&lots));
    let decided = surfaces_with(&lots, &graph, &report, CELL, &[street, access, stray]);
    assert_eq!(class_of(&decided, "osm/way/1"), Some(GRASS));
}

// ---- fix round 2 (the Task 6 leftovers) ----

/// A street-side strip, which the lot step leaves out of its groups.
fn strip(id: &str, outer: Vec<Pt>) -> Assessed {
    let mut lot = car_park(id, outer);
    lot.area.placement = Placement::StreetSide;
    lot.outcome = Outcome::SurfaceOnly {
        reason: "street_side",
    };
    lot
}

#[test]
fn a_street_side_strip_does_not_own_a_through_road() {
    // a car park with a through road from west to east and a 50 m gravel stub of its own; a street-side strip lies
    // over the middle of the through road and is smaller than the car park
    let features = [
        joined(
            1,
            &[1, 2],
            &[(-50.0, 50.0), (10.0, 50.0)],
            Family::Asf2,
            ASPHALT,
        ),
        joined(
            2,
            &[2, 3],
            &[(10.0, 50.0), (90.0, 50.0)],
            Family::Asf3,
            ASPHALT,
        ),
        joined(
            3,
            &[3, 4],
            &[(90.0, 50.0), (150.0, 50.0)],
            Family::Asf2,
            ASPHALT,
        ),
        along(20, 80.0, 20.0, 70.0, Family::Mud, GRAVEL),
    ];
    let park = car_park("osm/way/1", square(0.0, 0.0, 100.0, 100.0));
    // a street-side strip over the road's middle at (50, 50): 40 m by 10 m, smaller than the car park
    let small_strip = strip("osm/way/2", square(30.0, 45.0, 70.0, 55.0));
    let lots = [park, small_strip];
    let graph = build_graph(&features);
    let (_, report) = clean_with(&graph, &|f: Family| f.width_m(), CELL, &groups(&lots));
    assert_eq!(report.lot_through.len(), 1, "{:?}", report.lot_through);
    let decided = surfaces(&lots, &graph, &report, CELL);
    // the car park takes the asphalt through road (80 m) and its own 50 m of gravel: paved. The strip may not own
    // the road (it is no member of the lot step's groups), so without the fix the car park would be grass.
    assert_eq!(class_of(&decided, "osm/way/1"), Some(PAVED));
    assert_eq!(class_of(&decided, "osm/way/2"), Some(PAVED));
}

#[test]
fn a_lot_whose_hole_is_not_a_number_is_grass() {
    let mut park = small();
    park.area.holes.push(vec![
        (10.0, 10.0),
        (20.0, 10.0),
        (f64::NAN, 20.0),
        (10.0, 20.0),
        (10.0, 10.0),
    ]);
    let decided = decide(
        &[park],
        &[along(10, -3.0, -50.0, 90.0, Family::Asf3, ASPHALT)],
    );
    assert_eq!(class_of(&decided, "osm/way/1"), Some(GRASS));
}

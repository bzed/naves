// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What an OpenStreetMap bridge is built from, and how far it grows into the road beside it, on synthetic
//! catalogs and graphs.

#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::bridgekind::{
    BridgeFit, BridgeKind, BridgeSite, Choice, FILL_MAX_M, Fallback, GROW_KEEP_M,
    GROW_TURN_MAX_DEG, bridge_kind, choose, fit_bridges, straight_room,
};
use naves_core::catalog::testing::{synthetic_catalog, synthetic_models};
use naves_core::catalog::{Family, PANELS_PREFIX, PieceCatalog};
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::roadnet::{RoadGraph, Stretch, build_graph};

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// A bridge `length` long of `family` with `room` of straight road at each end (and as much to the junction),
/// on flat ground.
fn site(length: f64, family: Family, room: (f64, f64)) -> BridgeSite {
    site_on(length, family, room, &|_| 0.0)
}

/// The same on the ground `ground(along)`.
fn site_on(
    length: f64,
    family: Family,
    room: (f64, f64),
    ground: &dyn Fn(f64) -> f64,
) -> BridgeSite {
    let mut samples = Vec::new();
    let mut at = -room.0;
    while at <= length + room.1 + 1e-9 {
        samples.push((at, ground(at)));
        at += 1.0;
    }
    BridgeSite {
        length,
        family,
        room,
        reach: room,
        ground: samples,
    }
}

fn model(kind: &BridgeKind) -> Option<(&str, usize)> {
    match kind {
        BridgeKind::Model { model, count } => {
            let name = model.rsplit('\\').next().unwrap();
            Some((name.strip_suffix(".p3d").unwrap_or(name), *count))
        }
        _ => None,
    }
}

fn panels(count: usize, scale: f64) -> BridgeKind {
    BridgeKind::Panels { count, scale }
}

const ROOM: (f64, f64) = (100.0, 100.0);

// --- the table -------------------------------------------------------------------------------------------------

#[test]
fn three_metres_and_less_are_filled() {
    let c = synthetic_catalog();
    for length in [0.0, 1.0, FILL_MAX_M, f64::NAN, -2.0] {
        let choice = choose(&c, &site(length, Family::Asf1, ROOM));
        assert_eq!(choice.kind, BridgeKind::Fill, "{length}");
        assert!(
            choice.fallback.is_empty(),
            "{length}: {:?}",
            choice.fallback
        );
    }
}

#[test]
fn just_over_three_metres_is_a_panel_row() {
    let c = synthetic_catalog();
    // one panel at scale 2 on a 12 m road, two at scale 1 on a 6 m road
    assert_eq!(
        bridge_kind(&c, &site(3.1, Family::Asf1, ROOM)),
        panels(1, 2.0)
    );
    assert_eq!(
        bridge_kind(&c, &site(3.1, Family::Grav, ROOM)),
        panels(2, 1.0)
    );
    // there is no culvert: 5 m is a panel row too
    assert_eq!(
        bridge_kind(&c, &site(5.0, Family::Grav, ROOM)),
        panels(2, 1.0)
    );
}

#[test]
fn a_panel_row_is_scaled_to_its_road_and_covers_the_bridge() {
    let c = synthetic_catalog();
    let on_six = choose(&c, &site(12.0, Family::Grav, ROOM));
    assert_eq!(on_six.kind, panels(4, 1.0));
    assert!(near(on_six.length, 12.0, 1e-9));
    let on_twelve = choose(&c, &site(12.0, Family::Asf1, ROOM));
    assert_eq!(on_twelve.kind, panels(2, 2.0));
    // a row longer than the bridge is centred on it
    let longer = choose(&c, &site(10.0, Family::Grav, ROOM));
    assert_eq!(longer.kind, panels(4, 1.0));
    assert!(near(longer.grown.0, 1.0, 1e-9) && near(longer.grown.1, 1.0, 1e-9));
    // a 7 m road (asphalt 3) scales the 6 m panel by 7 / 6
    match bridge_kind(&c, &site(10.0, Family::Asf3, ROOM)) {
        BridgeKind::Panels { count, scale } => {
            assert!(near(scale, 7.0 / 6.0, 1e-9));
            assert_eq!(count, 3);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_model_is_used_from_two_thirds_of_its_length() {
    let c = synthetic_catalog();
    let cases = [
        (13.2, Family::Grav, "bridge_wood2"),
        (14.1, Family::Asf3, "bridge_concrete3"),
        (16.7, Family::Asf1, "bridge_metal_25"),
        (16.7, Family::Asf2, "bridge_stone_25"),
        (16.7, Family::City, "bridge_metal_25"),
    ];
    for (length, family, name) in cases {
        let choice = choose(&c, &site(length, family, ROOM));
        assert_eq!(model(&choice.kind), Some((name, 1)), "{length} {family:?}");
        assert_eq!(choice.fallback, Vec::new());
    }
    // just below two thirds: panels
    assert!(matches!(
        bridge_kind(&c, &site(13.0, Family::Grav, ROOM)),
        BridgeKind::Panels { .. }
    ));
    assert!(matches!(
        bridge_kind(&c, &site(16.5, Family::Asf1, ROOM)),
        BridgeKind::Panels { .. }
    ));
}

#[test]
fn a_model_on_posts_needs_ground_its_posts_reach() {
    let c = synthetic_catalog();
    // on flat ground the posts reach: the wooden 31.6 m bridge covers 22 m
    let flat = choose(&c, &site(22.0, Family::Grav, ROOM));
    assert_eq!(model(&flat.kind), Some(("bridge_wood_25", 1)));
    // the ground 4 m down under the middle: the 2.6 m posts do not reach it, so the plain deck
    let valley = |at: f64| if (2.0..20.0).contains(&at) { -4.0 } else { 0.0 };
    let deep = choose(&c, &site_on(22.0, Family::Grav, ROOM, &valley));
    assert_eq!(model(&deep.kind), Some(("bridge_wood2", 1)));
    // nor where the ground rises more than 0.5 m above the deck under a post
    let hump = |at: f64| if (8.0..14.0).contains(&at) { 1.0 } else { 0.0 };
    let high = choose(&c, &site_on(22.0, Family::Grav, ROOM, &hump));
    assert_eq!(model(&high.kind), Some(("bridge_wood2", 1)));
    // ground that is not known reaches nothing
    let unknown = choose(&c, &site_on(22.0, Family::Grav, ROOM, &|_| f64::NAN));
    assert_eq!(model(&unknown.kind), Some(("bridge_wood2", 1)));
    // a plain deck shorter than two thirds of the bridge is not used: a panel row instead
    let long = choose(&c, &site_on(40.0, Family::Grav, ROOM, &valley));
    assert!(matches!(long.kind, BridgeKind::Panels { .. }), "{long:?}");
}

#[test]
fn a_long_bridge_is_a_chain_of_the_family_model() {
    let c = synthetic_catalog();
    let choice = choose(&c, &site(60.0, Family::Asf1, ROOM));
    assert_eq!(model(&choice.kind), Some(("bridge_metal_25", 3)));
    assert!(near(choice.length, 75.0, 1e-9));
    assert!(near(choice.grown.0, 7.5, 1e-9) && near(choice.grown.1, 7.5, 1e-9));
    // asphalt 3: the stone bridge is longer than concrete3, so it is the chain
    let asf3 = choose(&c, &site(30.0, Family::Asf3, ROOM));
    assert_eq!(model(&asf3.kind), Some(("bridge_stone_25", 2)));
}

// --- room ------------------------------------------------------------------------------------------------------

#[test]
fn room_on_one_side_only_leaves_the_rest_to_the_other() {
    let c = synthetic_catalog();
    let choice = choose(&c, &site(16.7, Family::Asf1, (0.0, 20.0)));
    assert_eq!(model(&choice.kind), Some(("bridge_metal_25", 1)));
    assert!(near(choice.grown.0, 0.0, 1e-9));
    assert!(near(choice.grown.1, 8.3, 1e-9), "{:?}", choice.grown);
    let other = choose(&c, &site(16.7, Family::Asf1, (2.0, 20.0)));
    assert!(near(other.grown.0, 2.0, 1e-9) && near(other.grown.1, 6.3, 1e-9));
}

#[test]
fn a_model_without_room_is_a_panel_row_of_the_bridge_length() {
    let c = synthetic_catalog();
    let choice = choose(&c, &site(16.7, Family::Asf1, (1.0, 1.0)));
    // 3 panels at scale 2 are 18 m: 1.3 m more than the bridge, in the room
    assert_eq!(choice.kind, panels(3, 2.0));
    assert!(near(choice.grown.0, 0.65, 1e-9));
    assert!(matches!(
        choice.fallback.as_slice(),
        [Fallback::ModelNoRoom { model }] if model.ends_with("bridge_metal_25.p3d")
    ));
}

#[test]
fn a_panel_row_without_room_loses_panels_down_to_one() {
    let c = synthetic_catalog();
    // 13 m wants 5 panels (15 m); with 0.5 m of room at each end 4 fit, centred on the bridge
    let choice = choose(&c, &site(13.0, Family::Grav, (0.5, 0.5)));
    assert_eq!(choice.kind, panels(4, 1.0));
    assert!(near(choice.grown.0, -0.5, 1e-9) && near(choice.grown.1, -0.5, 1e-9));
    assert_eq!(
        choice.fallback,
        vec![Fallback::PanelsShortened { wanted: 5 }]
    );
}

#[test]
fn one_panel_past_the_straight_road_is_laid_if_it_stays_clear_of_the_junctions() {
    let c = synthetic_catalog();
    // a 6 m panel (scale 2) on a 4 m bridge with no straight room, 2 m to the junctions
    let mut s = site(4.0, Family::Asf1, (0.0, 0.0));
    s.reach = (2.0, 2.0);
    let choice = choose(&c, &s);
    assert_eq!(choice.kind, panels(1, 2.0));
    assert!(near(choice.grown.0, 1.0, 1e-9) && near(choice.grown.1, 1.0, 1e-9));
    assert_eq!(choice.fallback, vec![Fallback::OnePanelPastTheStraight]);
    // not if it reaches within a metre of a junction: plain road, and the report says why
    s.reach = (0.5, 2.0);
    let fill = choose(&c, &s);
    assert_eq!(fill.kind, BridgeKind::Fill);
    assert_eq!(fill.fallback, vec![Fallback::NoRoom]);
    assert_ne!(fill.fallback[0].to_string(), "");
}

/// The synthetic catalog without its panels.
fn without_panels() -> PieceCatalog {
    let mut models = synthetic_models();
    models.retain(|path, _| !path.starts_with(PANELS_PREFIX));
    PieceCatalog::from_models(&models).unwrap()
}

#[test]
fn without_panels_a_model_where_it_fits_else_plain_road_with_a_reason() {
    let c = without_panels();
    assert_eq!(c.panels().len(), 0);
    let fits = choose(&c, &site(16.7, Family::Asf1, ROOM));
    assert_eq!(model(&fits.kind), Some(("bridge_metal_25", 1)));
    let short = choose(&c, &site(10.0, Family::Asf1, ROOM));
    assert_eq!(short.kind, BridgeKind::Fill);
    assert_eq!(short.fallback, vec![Fallback::NoParts]);
    let no_room = choose(&c, &site(16.7, Family::Asf1, (1.0, 1.0)));
    assert_eq!(no_room.kind, BridgeKind::Fill);
    assert_eq!(no_room.fallback.last(), Some(&Fallback::NoParts));
    assert_eq!(no_room.fallback.len(), 2);
}

#[test]
fn rooms_that_are_not_numbers_are_no_room() {
    let c = synthetic_catalog();
    let mut s = site(16.7, Family::Asf1, (f64::NAN, f64::INFINITY));
    s.reach = (f64::NAN, -1.0);
    let choice = choose(&c, &s);
    assert!(
        matches!(choice.kind, BridgeKind::Panels { .. }),
        "{choice:?}"
    );
    assert!(choice.grown.0 <= 0.0 && choice.grown.1 <= 0.0);
}

// --- the graph -------------------------------------------------------------------------------------------------

fn feature(
    way: i64,
    family: Family,
    bridge: bool,
    layer: i8,
    nodes: &[i64],
    points: &[(f64, f64)],
) -> Feature {
    Feature {
        id: format!("osm/way/{way}/0"),
        way_id: way,
        source: Source::Osm,
        kind: FeatureKind::Road {
            family,
            drivable: true,
            bridge,
            tunnel: false,
            roundabout: false,
            layer,
        },
        points: points.to_vec(),
        nodes: nodes.to_vec(),
    }
}

fn road(way: i64, nodes: &[i64], points: &[(f64, f64)]) -> Feature {
    feature(way, Family::Asf3, false, 0, nodes, points)
}

fn len(s: &Stretch) -> f64 {
    s.points
        .windows(2)
        .map(|p| (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1))
        .sum()
}

fn total(g: &RoadGraph) -> f64 {
    g.stretches.iter().map(len).sum()
}

/// 100 m road, a bridge of `family` `bridge_m` long, 100 m road, in one line.
fn road_bridge_road(family: Family, bridge_m: f64) -> RoadGraph {
    build_graph(&[
        road(1, &[1, 2], &[(0.0, 0.0), (100.0, 0.0)]),
        feature(
            2,
            family,
            true,
            1,
            &[2, 3],
            &[(100.0, 0.0), (100.0 + bridge_m, 0.0)],
        ),
        road(
            3,
            &[3, 4],
            &[(100.0 + bridge_m, 0.0), (200.0 + bridge_m, 0.0)],
        ),
    ])
}

fn fit(graph: &RoadGraph) -> (RoadGraph, Vec<BridgeFit>) {
    let c = synthetic_catalog();
    fit_bridges(graph, &|s: &BridgeSite| choose(&c, s), &|_| 0.0)
}

fn bridge_of(g: &RoadGraph) -> &Stretch {
    g.stretches
        .iter()
        .find(|s| s.bridge_kind.is_some())
        .unwrap()
}

#[test]
fn the_straight_room_ends_a_metre_before_the_next_junction() {
    let g = road_bridge_road(Family::Asf1, 16.7);
    let i = g.stretches.iter().position(|s| s.bridge).unwrap();
    let (before, after) = straight_room(&g, i);
    assert!(near(before, 100.0 - GROW_KEEP_M, 1e-9), "{before}");
    assert!(near(after, 100.0 - GROW_KEEP_M, 1e-9), "{after}");
    // a stretch that is not there has none
    assert_eq!(straight_room(&g, 99), (0.0, 0.0));
}

#[test]
fn a_bridge_grows_evenly_along_a_straight_road() {
    let g = road_bridge_road(Family::Asf1, 16.7);
    let (out, fits) = fit(&g);
    let b = bridge_of(&out);
    assert!(b.bridge);
    assert_eq!(
        b.bridge_kind.as_ref().and_then(model),
        Some(("bridge_metal_25", 1))
    );
    assert!(near(len(b), 25.0, 1e-6), "{}", len(b));
    assert!(near(b.points[0].0, 95.85, 1e-6), "{:?}", b.points);
    assert!(near(total(&out), total(&g), 1e-6));
    // the ids stay
    let ids = |g: &RoadGraph| g.stretches.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&out), ids(&g));
    assert_eq!(fits.len(), 1);
    let f = &fits[0];
    assert!(near(f.length, 16.7, 1e-9));
    assert!(near(f.grown.0, 4.15, 1e-6) && near(f.grown.1, 4.15, 1e-6));
    assert!(near(f.turn_deg.0, 0.0, 1e-9) && near(f.turn_deg.1, 0.0, 1e-9));
    assert_eq!(f.stretches, vec![f.stretch.clone()]);
    assert_eq!(f.fallback, Vec::new());
}

#[test]
fn a_bridge_between_two_bends_grows_nothing_and_becomes_panels() {
    // the road turns by 30 degrees right at both ends of a 13.2 m gravel bridge
    let (s30, c30) = 30.0_f64.to_radians().sin_cos();
    let g = build_graph(&[
        road(1, &[1, 2], &[(-50.0 * c30, 50.0 * s30), (0.0, 0.0)]),
        feature(
            2,
            Family::Grav,
            true,
            1,
            &[2, 3],
            &[(0.0, 0.0), (13.2, 0.0)],
        ),
        road(3, &[3, 4], &[(13.2, 0.0), (13.2 + 50.0 * c30, 50.0 * s30)]),
    ]);
    let (out, fits) = fit(&g);
    let f = &fits[0];
    // wood2 has no room, 5 panels (15 m) neither: 4 panels, centred on the bridge
    assert_eq!(f.kind, panels(4, 1.0));
    assert!(f.grown.0 <= 0.0 && f.grown.1 <= 0.0, "{:?}", f.grown);
    assert!(matches!(
        f.fallback.as_slice(),
        [
            Fallback::ModelNoRoom { .. },
            Fallback::PanelsShortened { wanted: 5 }
        ]
    ));
    for s in &g.stretches {
        let o = out.stretches.iter().find(|o| o.id == s.id).unwrap();
        assert_eq!(o.points, s.points);
    }
    // until panel rows are laid it is plain road, but it knows what it is to be
    let b = bridge_of(&out);
    assert!(!b.bridge);
    assert_eq!(b.bridge_kind, Some(panels(4, 1.0)));
}

#[test]
fn a_grown_line_never_turns_more_than_five_degrees() {
    // after the bridge the road bends by 2 degrees every 2 m
    let mut after = vec![(16.7, 0.0)];
    let mut heading = 0.0_f64;
    for _ in 0..40 {
        let &(x, y) = after.last().unwrap();
        after.push((x + 2.0 * heading.cos(), y + 2.0 * heading.sin()));
        heading += 2.0_f64.to_radians();
    }
    let nodes: Vec<i64> = (3..3 + 41).collect();
    let g = build_graph(&[
        road(1, &[1, 2], &[(-100.0, 0.0), (0.0, 0.0)]),
        feature(
            2,
            Family::Asf1,
            true,
            1,
            &[2, 3],
            &[(0.0, 0.0), (16.7, 0.0)],
        ),
        road(3, &nodes, &after),
    ]);
    let i = g.stretches.iter().position(|s| s.bridge).unwrap();
    let (_, room) = straight_room(&g, i);
    // 0, 2 and 4 degrees over the first three segments: the fourth brings it to 6
    assert!(near(room, 6.0, 1e-6), "{room}");
    let (_, fits) = fit(&g);
    let f = &fits[0];
    assert_eq!(model(&f.kind), Some(("bridge_metal_25", 1)));
    assert!(f.turn_deg.1 <= GROW_TURN_MAX_DEG + 1e-9, "{:?}", f.turn_deg);
    assert!(f.grown.1 <= room + 1e-9);
    assert!(near(f.grown.0 + f.grown.1, 8.3, 1e-6));
}

#[test]
fn bridge_ways_that_follow_each_other_are_one_bridge() {
    // two bridge ways of 10 m (their layers differ, so they are two stretches): 20 m is a metal bridge
    let g = build_graph(&[
        road(1, &[1, 2], &[(0.0, 0.0), (100.0, 0.0)]),
        feature(
            2,
            Family::Asf1,
            true,
            1,
            &[2, 3],
            &[(100.0, 0.0), (110.0, 0.0)],
        ),
        feature(
            3,
            Family::Asf1,
            true,
            2,
            &[3, 4],
            &[(110.0, 0.0), (120.0, 0.0)],
        ),
        road(4, &[4, 5], &[(120.0, 0.0), (220.0, 0.0)]),
    ]);
    assert_eq!(g.stretches.iter().filter(|s| s.bridge).count(), 2);
    let (out, fits) = fit(&g);
    assert_eq!(fits.len(), 1);
    let f = &fits[0];
    assert_eq!(f.stretches.len(), 2);
    assert!(near(f.length, 20.0, 1e-9));
    assert_eq!(model(&f.kind), Some(("bridge_metal_25", 1)));
    let bridges: Vec<_> = out.stretches.iter().filter(|s| s.bridge).collect();
    assert_eq!(bridges.len(), 1);
    assert_eq!(bridges[0].id, f.stretch);
    assert!(f.stretches.contains(&f.stretch));
    assert!(near(len(bridges[0]), 25.0, 1e-6));
    assert_eq!(bridges[0].layer, 2);
    assert!(near(total(&out), 220.0, 1e-6));
    // every node meets two stretches
    assert!(out.crossings.iter().all(|c| c.stretches.len() == 2));
}

#[test]
fn a_bridge_between_two_junction_stubs_never_grows_through_a_junction() {
    // 1 m of road at each end, then 3-way crossings
    let g = build_graph(&[
        road(1, &[1, 2], &[(-1.0, 0.0), (0.0, 0.0)]),
        road(5, &[1, 6], &[(-1.0, 0.0), (-1.0, 50.0)]),
        road(6, &[1, 7], &[(-1.0, 0.0), (-1.0, -50.0)]),
        feature(2, Family::Asf1, true, 1, &[2, 3], &[(0.0, 0.0), (4.0, 0.0)]),
        road(3, &[3, 4], &[(4.0, 0.0), (5.0, 0.0)]),
        road(7, &[4, 8], &[(5.0, 0.0), (5.0, 50.0)]),
        road(8, &[4, 9], &[(5.0, 0.0), (5.0, -50.0)]),
    ]);
    let (out, fits) = fit(&g);
    assert_eq!(fits[0].kind, BridgeKind::Fill);
    assert_eq!(fits[0].fallback, vec![Fallback::NoRoom]);
    for s in &g.stretches {
        let o = out.stretches.iter().find(|o| o.id == s.id).unwrap();
        assert_eq!(o.points, s.points);
    }
    assert!(out.stretches.iter().all(|s| !s.bridge));
}

#[test]
fn every_bridge_of_the_input_is_in_the_output_list() {
    let (s, c) = 0.5_f64.sin_cos();
    let g = build_graph(&[
        // a fill, a panel row and a model on one road, and a bridge that is a loop
        road(1, &[1, 2], &[(0.0, 0.0), (100.0, 0.0)]),
        feature(
            2,
            Family::Asf1,
            true,
            1,
            &[2, 3],
            &[(100.0, 0.0), (102.0, 0.0)],
        ),
        road(3, &[3, 4], &[(102.0, 0.0), (200.0, 0.0)]),
        feature(
            4,
            Family::Asf1,
            true,
            1,
            &[4, 5],
            &[(200.0, 0.0), (210.0, 0.0)],
        ),
        road(5, &[5, 6], &[(210.0, 0.0), (300.0, 0.0)]),
        feature(
            6,
            Family::Asf1,
            true,
            1,
            &[6, 7],
            &[(300.0, 0.0), (320.0, 0.0)],
        ),
        road(7, &[7, 8], &[(320.0, 0.0), (400.0, 0.0)]),
        feature(
            8,
            Family::Grav,
            true,
            1,
            &[20, 21, 22, 20],
            &[
                (0.0, 500.0),
                (10.0 * c, 500.0 + 10.0 * s),
                (10.0, 500.0),
                (0.0, 500.0),
            ],
        ),
        // a bridge of zero length, and one with a coordinate that is not a number
        feature(
            9,
            Family::Asf1,
            true,
            1,
            &[30, 31],
            &[(0.0, 600.0), (0.0, 600.0)],
        ),
        feature(
            10,
            Family::Asf1,
            true,
            1,
            &[40, 41],
            &[(0.0, 700.0), (f64::NAN, 710.0)],
        ),
    ]);
    let ids: Vec<String> = g
        .stretches
        .iter()
        .filter(|s| s.bridge)
        .map(|s| s.id.clone())
        .collect();
    let (out, fits) = fit(&g);
    let listed: Vec<String> = fits.iter().flat_map(|f| f.stretches.clone()).collect();
    for id in &ids {
        assert!(listed.contains(id), "{id} missing from {listed:?}");
    }
    assert_eq!(listed.len(), ids.len());
    let kinds: Vec<&BridgeKind> = fits.iter().map(|f| &f.kind).collect();
    assert!(kinds.contains(&&BridgeKind::Fill));
    assert!(kinds.iter().any(|k| matches!(k, BridgeKind::Panels { .. })));
    assert!(kinds.iter().any(|k| matches!(k, BridgeKind::Model { .. })));
    // every stretch that was a bridge says what it is to be
    for id in &ids {
        let s = out.stretches.iter().find(|s| &s.id == id).unwrap();
        assert!(s.bridge_kind.is_some(), "{id}");
    }
    // the same input gives the same output (compared as text: a length that is not a number is not equal to
    // itself)
    assert_eq!(format!("{:?}", fit(&g)), format!("{:?}", (out, fits)));
}

#[test]
fn a_choice_closure_decides_and_the_graph_follows() {
    // whatever the closure says is laid: here a chain of two of a 20 m model on a 16.7 m bridge
    let g = road_bridge_road(Family::Asf1, 16.7);
    let choice = |s: &BridgeSite| Choice {
        kind: BridgeKind::Model {
            model: "x".to_string(),
            count: 2,
        },
        length: 40.0,
        grown: (s.room.0.min(11.65), 11.65),
        fallback: Vec::new(),
    };
    let (out, fits) = fit_bridges(&g, &choice, &|_| 0.0);
    assert!(near(len(bridge_of(&out)), 40.0, 1e-6));
    assert!(near(fits[0].part_length, 40.0, 1e-9));
}

#[test]
fn the_straight_room_stops_a_metre_before_a_junction_and_a_dead_end() {
    // the road before the bridge is 12 m long and ends at a 3-way crossing, the one after is a 7 m dead end
    let g = build_graph(&[
        road(1, &[1, 2], &[(0.0, 0.0), (12.0, 0.0)]),
        road(9, &[1, 8], &[(0.0, 0.0), (0.0, 50.0)]),
        road(7, &[1, 6], &[(0.0, 0.0), (0.0, -50.0)]),
        feature(
            2,
            Family::Asf3,
            true,
            1,
            &[2, 3],
            &[(12.0, 0.0), (20.0, 0.0)],
        ),
        road(3, &[3, 4], &[(20.0, 0.0), (27.0, 0.0)]),
    ]);
    let i = g.stretches.iter().position(|s| s.bridge).unwrap();
    let (before, after) = straight_room(&g, i);
    assert!(
        near(before, 11.0, 1e-9) && near(after, 6.0, 1e-9),
        "{before} {after}"
    );
}

#[test]
fn a_neighbour_is_never_used_up() {
    // whatever the choice asks for, the road beside the bridge keeps a metre
    let g = road_bridge_road(Family::Asf1, 8.0);
    let greedy = |_: &BridgeSite| Choice {
        kind: BridgeKind::Model {
            model: "x".to_string(),
            count: 16,
        },
        length: 400.0,
        grown: (196.0, 196.0),
        fallback: Vec::new(),
    };
    let (out, fits) = fit_bridges(&g, &greedy, &|_| 0.0);
    assert!(
        out.stretches
            .iter()
            .filter(|s| s.bridge_kind.is_none())
            .all(|s| len(s) >= GROW_KEEP_M - 1e-9),
        "{:?}",
        out.stretches.iter().map(len).collect::<Vec<_>>()
    );
    assert!(near(fits[0].grown.0, 99.0, 1e-9) && near(fits[0].grown.1, 99.0, 1e-9));
    assert!(near(total(&out), total(&g), 1e-6));
}

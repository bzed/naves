// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What an OpenStreetMap bridge is built from, and how far it grows into the road beside it, on synthetic
//! catalogs and graphs.

#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::bridgekind::{
    BridgeFit, BridgeKind, BridgeSite, Choice, FILL_MAX_M, Fallback, GROW_KEEP_M,
    GROW_TURN_MAX_DEG, Lead, PANEL_SCALE_MAX, PANEL_SCALE_MIN, RoomEnd, bridge_kind, choose,
    fit_bridges, panel_grid, straight_room,
};
use naves_core::catalog::testing::{synthetic_catalog, synthetic_models};
use naves_core::catalog::{Family, PANELS_PREFIX, PieceCatalog, Slab};
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::roadnet::{RoadGraph, Stretch, build_graph};

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn flat(_: f64, _: f64) -> f64 {
    0.0
}

/// A bridge `length` long of `family` with `room` of straight road at each end, on flat ground.
fn site(length: f64, family: Family, room: (f64, f64)) -> BridgeSite<'static> {
    site_on(length, family, room, &flat)
}

/// The same on the ground `ground(along, across)`.
fn site_on(
    length: f64,
    family: Family,
    room: (f64, f64),
    ground: &dyn Fn(f64, f64) -> f64,
) -> BridgeSite<'_> {
    BridgeSite {
        length,
        family,
        room,
        turn_deg: 0.0,
        bends: (Vec::new(), Vec::new()),
        at_bridge_node: false,
        node_ends: (false, false),
        ground,
        below: Vec::new(),
        reach: room,
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

/// The count and scale of a panel row.
fn row(kind: &BridgeKind) -> (usize, usize, f64) {
    match kind {
        BridgeKind::Panels {
            count,
            abreast,
            scale,
        } => (*count, *abreast, *scale),
        other => panic!("no panel row: {other:?}"),
    }
}

fn is_row(kind: &BridgeKind, count: usize, abreast: usize, scale: f64) -> bool {
    let (n, k, s) = row(kind);
    n == count && k == abreast && near(s, scale, 1e-3)
}

/// A row covers its bridge exactly and never grows.
fn covers(choice: &Choice, length: f64) {
    assert!(near(choice.length, length, 1e-9), "{choice:?}");
    assert_eq!(choice.grown, (0.0, 0.0), "{choice:?}");
}

const ROOM: (f64, f64) = (100.0, 100.0);

// --- the table -------------------------------------------------------------------------------------------------

#[test]
fn three_metres_and_less_are_filled() {
    let c = synthetic_catalog();
    for length in [0.0, 1.0, FILL_MAX_M, f64::NAN, -2.0] {
        let choice = choose(&c, &site(length, Family::Asf1, ROOM));
        assert_eq!(choice.kind, BridgeKind::Fill, "{length}");
        assert_eq!(choice.fallback, Vec::new(), "{length}");
    }
}

#[test]
fn a_panel_grid_matches_the_road_width_and_covers_the_bridge() {
    let slab = Slab {
        model: "p".to_string(),
        along: 3.0,
        across: 6.0,
        along_x: true,
        top: 0.14,
        thickness: 0.25,
    };
    // (length, road width, along, abreast, scale)
    for (length, width, n, k, s) in [
        (8.85, 12.0, 3, 2, 0.983), // 11.8 m wide
        (7.82, 10.0, 3, 2, 0.869), // 10.43 m
        (3.01, 10.0, 1, 2, 1.003), // 12.04 m: one panel abreast would be 6 m
        (8.42, 9.0, 2, 1, 1.403),  // 8.42 m
        (6.11, 6.0, 2, 1, 1.018),
        (34.98, 12.0, 6, 1, 1.943), // 11.66 m; 12 along and 2 abreast is as wide with more panels
        (12.0, 6.0, 4, 1, 1.0),
        (12.0, 12.0, 2, 1, 2.0), // 4 along and 2 abreast is as exact with more panels
    ] {
        let (count, abreast, scale) = panel_grid(length, width, &slab);
        assert_eq!((count, abreast), (n, k), "{length} on {width}");
        assert!(near(scale, s, 1e-3), "{length} on {width}: {scale}");
        assert!((PANEL_SCALE_MIN..=PANEL_SCALE_MAX).contains(&scale));
        assert!(near(
            f64::from(u32::try_from(count).unwrap()) * 3.0 * scale,
            length,
            1e-9
        ));
    }
    // no count gives a scale in range (2 m): the nearest count, and abreast to the road's width
    let (count, abreast, scale) = panel_grid(2.0, 12.0, &slab);
    assert_eq!((count, abreast), (1, 3));
    assert!(near(scale, 2.0 / 3.0, 1e-9));
    // absurd input stays bounded
    let (count, abreast, scale) = panel_grid(1e9, 6.0, &slab);
    assert!((1..=1000).contains(&count) && (1..=1000).contains(&abreast) && scale.is_finite());
    let (count, abreast, _) = panel_grid(10.0, f64::NAN, &slab);
    assert!(count >= 1 && abreast == 1);
    let (count, abreast, _) = panel_grid(f64::NAN, 10.0, &slab);
    assert!(count >= 1 && abreast >= 1);
}

#[test]
fn just_over_three_metres_is_one_panel() {
    let c = synthetic_catalog();
    // two abreast on a 12 m road, one on a 6 m road
    for (family, abreast) in [(Family::Asf1, 2), (Family::Grav, 1)] {
        let choice = choose(&c, &site(3.1, family, ROOM));
        assert!(
            is_row(&choice.kind, 1, abreast, 3.1 / 3.0),
            "{family:?} {choice:?}"
        );
        covers(&choice, 3.1);
    }
    // there is no culvert: 5 m is a panel row too
    assert!(matches!(
        bridge_kind(&c, &site(5.0, Family::Grav, ROOM)),
        BridgeKind::Panels { .. }
    ));
}

#[test]
fn a_panel_row_is_scaled_to_its_road_and_covers_the_bridge() {
    let c = synthetic_catalog();
    let on_six = choose(&c, &site(12.0, Family::Grav, ROOM));
    assert!(is_row(&on_six.kind, 4, 1, 1.0));
    covers(&on_six, 12.0);
    let on_twelve = choose(&c, &site(12.0, Family::Asf1, ROOM));
    assert!(is_row(&on_twelve.kind, 2, 1, 2.0));
    // a 7 m road (asphalt 3): three panels of 10 / 9
    let asf3 = choose(&c, &site(10.0, Family::Asf3, ROOM));
    assert!(is_row(&asf3.kind, 3, 1, 10.0 / 9.0), "{asf3:?}");
    covers(&asf3, 10.0);
}

#[test]
fn a_panel_row_needs_no_room() {
    let c = synthetic_catalog();
    let choice = choose(&c, &site(13.0, Family::Grav, (0.0, 0.0)));
    assert!(is_row(&choice.kind, 4, 1, 13.0 / 12.0), "{choice:?}");
    covers(&choice, 13.0);
    assert_eq!(choice.fallback, Vec::new());
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
    let flat_site = choose(&c, &site(22.0, Family::Grav, ROOM));
    assert_eq!(model(&flat_site.kind), Some(("bridge_wood_25", 1)));
    // the ground 4 m down under the middle: the 2.6 m posts do not reach it, so the plain deck
    let valley = |at: f64, _: f64| if (2.0..20.0).contains(&at) { -4.0 } else { 0.0 };
    let deep = choose(&c, &site_on(22.0, Family::Grav, ROOM, &valley));
    assert_eq!(model(&deep.kind), Some(("bridge_wood2", 1)));
    // nor where the ground rises more than 0.5 m above the deck under a post
    let hump = |at: f64, _: f64| if (8.0..14.0).contains(&at) { 1.0 } else { 0.0 };
    let high = choose(&c, &site_on(22.0, Family::Grav, ROOM, &hump));
    assert_eq!(model(&high.kind), Some(("bridge_wood2", 1)));
    // ground that is not known reaches nothing
    let unknown = |_: f64, _: f64| f64::NAN;
    let none = choose(&c, &site_on(22.0, Family::Grav, ROOM, &unknown));
    assert_eq!(model(&none.kind), Some(("bridge_wood2", 1)));
    // a plain deck shorter than two thirds of the bridge is not used: a panel row instead
    let long = choose(&c, &site_on(40.0, Family::Grav, ROOM, &valley));
    assert!(matches!(long.kind, BridgeKind::Panels { .. }), "{long:?}");
}

#[test]
fn posts_are_checked_where_they_stand_not_on_the_centre_line() {
    let c = synthetic_catalog();
    // a slope across the bridge, level along it: the centre line is at the deck's level, the posts 3.3 m to
    // either side stand 3.3 m above and below it
    let across = |_: f64, across: f64| across;
    let choice = choose(&c, &site_on(22.0, Family::Grav, ROOM, &across));
    assert_eq!(model(&choice.kind), Some(("bridge_wood2", 1)), "{choice:?}");
    // a gentle one (0.1 per metre: 0.33 m at the posts) they reach
    let gentle = |_: f64, across: f64| 0.1 * across;
    let choice = choose(&c, &site_on(22.0, Family::Grav, ROOM, &gentle));
    assert_eq!(
        model(&choice.kind),
        Some(("bridge_wood_25", 1)),
        "{choice:?}"
    );
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

#[test]
fn a_model_needs_a_straight_bridge() {
    let c = synthetic_catalog();
    let mut bent = site(16.7, Family::Asf1, ROOM);
    bent.turn_deg = 30.0;
    let choice = choose(&c, &bent);
    assert_eq!(model(&choice.kind), None, "{choice:?}");
    covers(&choice, 16.7);
    assert!(matches!(
        choice.fallback.as_slice(),
        [Fallback::ModelBends { deg, .. }] if near(*deg, 30.0, 1e-9)
    ));
    // the bridge's own turn and the turn of the road it grows into count together
    let mut grown = site(16.7, Family::Asf1, ROOM);
    grown.turn_deg = 3.0;
    grown.bends = (vec![(10.0, 1.0)], vec![(10.0, 1.5)]);
    assert!(model(&choose(&c, &grown).kind).is_none());
    grown.bends = (vec![(10.0, 1.0)], vec![(10.0, 0.5)]);
    assert_eq!(
        model(&choose(&c, &grown).kind),
        Some(("bridge_metal_25", 1))
    );
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
    // 3 panels of 16.7 / 9
    assert!(is_row(&choice.kind, 3, 1, 16.7 / 9.0), "{choice:?}");
    covers(&choice, 16.7);
    assert!(matches!(
        choice.fallback.as_slice(),
        [Fallback::ModelNoRoom { model }] if model.ends_with("bridge_metal_25.p3d")
    ));
    assert_ne!(choice.fallback[0].to_string(), "");
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
    let choice = choose(&c, &site(16.7, Family::Asf1, (f64::NAN, f64::INFINITY)));
    assert!(
        matches!(choice.kind, BridgeKind::Panels { .. }),
        "{choice:?}"
    );
    covers(&choice, 16.7);
}

#[test]
fn a_short_bridge_at_a_node_of_bridges_is_never_filled() {
    let c = synthetic_catalog();
    let mut s = site(2.0, Family::Asf1, (0.0, 0.0));
    s.at_bridge_node = true;
    s.node_ends = (true, false);
    let choice = choose(&c, &s);
    assert!(is_row(&choice.kind, 1, 3, 2.0 / 3.0), "{choice:?}");
    covers(&choice, 2.0);
    // one of no length is still nothing
    s.length = 0.0;
    assert_eq!(choose(&c, &s).kind, BridgeKind::Fill);
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
            sidepath_hint: false,
            use_sidepath: false,
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
    // wood2 has no room: 4 panels of 13.2 / 12, covering the bridge
    assert!(is_row(&f.kind, 4, 1, 1.1), "{f:?}");
    assert_eq!(f.grown, (0.0, 0.0));
    assert!(near(f.part_length, 13.2, 1e-9));
    assert!(matches!(
        f.fallback.as_slice(),
        [Fallback::ModelNoRoom { .. }]
    ));
    assert!(
        matches!(f.room_end.0, RoomEnd::KinkAtJoint { deg } if near(deg, 30.0, 1e-6)),
        "{:?}",
        f.room_end
    );
    // the bridge's own line is as it was; the roads beside it get a straight lead on its axis (30 degrees, so
    // 2 R tan 15 = 5.36 m with the 10 m radius of the tightest synthetic arc), and only within what is replaced
    for s in &g.stretches {
        let o = out.stretches.iter().find(|o| o.id == s.id).unwrap();
        if s.id.starts_with("2:") {
            assert_eq!(o.points, s.points);
        } else {
            assert_ne!(o.points, s.points, "{}", s.id);
        }
    }
    assert!(
        matches!(f.lead.0, Lead::Laid { turn_deg, .. } if near(turn_deg, 30.0, 1e-6)),
        "{:?}",
        f.lead
    );
    assert!(matches!(f.lead.1, Lead::Laid { .. }), "{:?}", f.lead);
    let before = out
        .stretches
        .iter()
        .find(|o| o.id.starts_with("1:"))
        .unwrap();
    // from the node outwards: the node, a point on the axis (to the west), then the road's own path
    let node = *before.points.last().unwrap();
    let axis = before.points[before.points.len() - 2];
    assert!(
        near(axis.1, 0.0, 1e-9) && axis.0 < node.0,
        "{axis:?} {node:?}"
    );
    assert!(near(node.0 - axis.0, 5.359, 0.01), "{axis:?}");
    // a panel grid is a real bridge
    let b = bridge_of(&out);
    assert!(b.bridge);
    assert!(is_row(b.bridge_kind.as_ref().unwrap(), 4, 1, 1.1));
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
    assert!(
        matches!(f.room_end.1, RoomEnd::TurnAfter { m, deg } if near(m, 6.0, 1e-6) && near(deg, 6.0, 1e-6)),
        "{:?}",
        f.room_end
    );
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
    // one panel of 4 / 3, the bridge's own length: no room is needed
    assert!(is_row(&fits[0].kind, 1, 2, 4.0 / 3.0), "{:?}", fits[0]);
    assert_eq!(
        fits[0].room_end,
        (RoomEnd::ShortNeighbour, RoomEnd::ShortNeighbour)
    );
    for s in &g.stretches {
        let o = out.stretches.iter().find(|o| o.id == s.id).unwrap();
        assert_eq!(o.points, s.points);
    }
    // a panel grid is a real bridge, and its stretch stays as it was
    assert!(out.stretches.iter().any(|s| s.bridge));
}

#[test]
fn the_room_says_why_it_ends() {
    // a bridge from a 3-way crossing to a dead end
    let g = build_graph(&[
        road(1, &[1, 2], &[(-50.0, 0.0), (0.0, 0.0)]),
        road(5, &[2, 6], &[(0.0, 0.0), (0.0, 50.0)]),
        feature(
            2,
            Family::Asf1,
            true,
            1,
            &[2, 3],
            &[(0.0, 0.0), (16.7, 0.0)],
        ),
    ]);
    let (_, fits) = fit(&g);
    assert_eq!(fits[0].room_end, (RoomEnd::Junction, RoomEnd::DeadEnd));
    assert_eq!(fits[0].room, (0.0, 0.0));
    for reason in [
        RoomEnd::Junction,
        RoomEnd::BridgeNode,
        RoomEnd::Bridge,
        RoomEnd::DeadEnd,
        RoomEnd::KinkAtJoint { deg: 8.0 },
        RoomEnd::TurnAfter { m: 3.0, deg: 6.0 },
        RoomEnd::ShortNeighbour,
        RoomEnd::Tunnel,
    ] {
        assert_ne!(reason.to_string(), "");
    }
}

#[test]
fn bridges_that_meet_at_a_node_of_bridges_reach_it_and_none_is_filled() {
    // a fork of three bridges at node 2: a 2 m one and two of 10 m, each with road at its other end
    let g = build_graph(&[
        road(1, &[1, 3], &[(-50.0, 0.0), (-2.0, 0.0)]),
        feature(
            2,
            Family::Asf1,
            true,
            1,
            &[3, 2],
            &[(-2.0, 0.0), (0.0, 0.0)],
        ),
        feature(
            3,
            Family::Asf1,
            true,
            1,
            &[2, 4],
            &[(0.0, 0.0), (10.0, 0.0)],
        ),
        feature(
            4,
            Family::Asf1,
            true,
            1,
            &[2, 5],
            &[(0.0, 0.0), (0.0, 10.0)],
        ),
        road(5, &[4, 6], &[(10.0, 0.0), (60.0, 0.0)]),
        road(6, &[5, 7], &[(0.0, 10.0), (0.0, 60.0)]),
    ]);
    let (out, fits) = fit(&g);
    assert_eq!(fits.len(), 3);
    for f in &fits {
        assert!(matches!(f.kind, BridgeKind::Panels { .. }), "{f:?}");
        assert!(near(f.part_length, f.length, 1e-9), "{f:?}");
        assert_eq!(f.grown, (0.0, 0.0));
        assert!(
            f.room_end.0 == RoomEnd::BridgeNode || f.room_end.1 == RoomEnd::BridgeNode,
            "{f:?}"
        );
    }
    assert!(near(total(&out), total(&g), 1e-9));
}

#[test]
fn a_filled_bridge_is_plain_road_for_the_room_beside_it() {
    let g = road_bridge_road(Family::Asf1, 2.0);
    let (out, fits) = fit(&g);
    assert_eq!(fits[0].kind, BridgeKind::Fill);
    // the road before it reaches into the filled stretch, keeping a metre of it
    let first = out
        .stretches
        .iter()
        .position(|s| s.id.starts_with("1:"))
        .unwrap();
    let (_, after) = straight_room(&out, first);
    assert!(near(after, 2.0 - GROW_KEEP_M, 1e-9), "{after}");
}

#[test]
fn a_model_is_never_laid_on_a_bent_bridge() {
    // a 17 m Asf1 bridge drawn as a 30 degree arc between straight roads
    let radius = 17.0 / 30.0_f64.to_radians();
    let arc: Vec<(f64, f64)> = (0..=10)
        .map(|k| {
            let a = 30.0_f64.to_radians() * f64::from(k) / 10.0;
            (radius * a.sin(), radius * (1.0 - a.cos()))
        })
        .collect();
    let end = *arc.last().unwrap();
    let out_dir = (30.0_f64.to_radians().cos(), 30.0_f64.to_radians().sin());
    let nodes: Vec<i64> = (2..=12).collect();
    let g = build_graph(&[
        road(1, &[1, 2], &[(-100.0, 0.0), (0.0, 0.0)]),
        feature(2, Family::Asf1, true, 1, &nodes, &arc),
        road(
            3,
            &[12, 13],
            &[end, (end.0 + 100.0 * out_dir.0, end.1 + 100.0 * out_dir.1)],
        ),
    ]);
    let (_, fits) = fit(&g);
    let f = &fits[0];
    // 27 degrees over its own nine vertices (and 1.5 at each joint)
    assert!(f.own_turn_deg > 25.0, "{f:?}");
    assert!(matches!(f.kind, BridgeKind::Panels { .. }), "{f:?}");
    assert!(matches!(
        f.fallback.as_slice(),
        [Fallback::ModelBends { .. }]
    ));
}

#[test]
fn the_joined_bridge_has_the_id_of_its_lower_outer_way_and_its_edits_still_apply() {
    use naves_core::objects::{
        ObjectId, Override, Overrides, PinnedEdit, apply_overrides, road_objects,
    };
    use naves_core::roadplace::place_network;
    // three bridge ways (their layers differ), the one in the middle with the lowest way id
    let g = build_graph(&[
        road(10, &[1, 2], &[(0.0, 0.0), (100.0, 0.0)]),
        feature(
            5,
            Family::Asf1,
            true,
            1,
            &[2, 3],
            &[(100.0, 0.0), (106.0, 0.0)],
        ),
        feature(
            1,
            Family::Asf1,
            true,
            2,
            &[3, 4],
            &[(106.0, 0.0), (112.0, 0.0)],
        ),
        feature(
            7,
            Family::Asf1,
            true,
            3,
            &[4, 5],
            &[(112.0, 0.0), (120.0, 0.0)],
        ),
        road(11, &[5, 6], &[(120.0, 0.0), (220.0, 0.0)]),
    ]);
    let (out, fits) = fit(&g);
    assert_eq!(fits.len(), 1);
    let f = &fits[0];
    assert_eq!(f.stretch, "5:2-3");
    assert_eq!(f.stretches.len(), 3);
    assert_eq!(model(&f.kind), Some(("bridge_metal_25", 1)));
    let catalog = synthetic_catalog();
    let net = place_network(&out, &catalog).unwrap();
    let objects = road_objects(&net);
    let deck = objects
        .iter()
        .find(|o| o.model.contains("bridge_metal_25"))
        .unwrap();
    assert_eq!(deck.id.feature, "5:2-3");
    let edit = |feature: &str| PinnedEdit {
        id: ObjectId {
            feature: feature.to_string(),
            preset: "road".to_string(),
            slot: deck.id.slot,
        },
        generated_model: deck.model.clone(),
        generated_at: (deck.x, deck.y),
        generated_yaw: deck.yaw,
        edit: Override::Replaced {
            model: "other".to_string(),
        },
    };
    // an edit on the first outer way finds the deck; one on another way of the bridge is orphaned
    let (placed, orphans) = apply_overrides(
        &objects,
        &Overrides {
            edits: vec![edit("5:2-3")],
        },
    );
    assert_eq!(orphans.len(), 0);
    assert!(placed.iter().any(|o| o.model == "other" && o.id.is_some()));
    let (_, orphans) = apply_overrides(
        &objects,
        &Overrides {
            edits: vec![edit("1:3-4")],
        },
    );
    assert_eq!(orphans.len(), 1);
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
        lead_radius: 15.0,
        slid: 0.0,
        below: Vec::new(),
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
        lead_radius: 15.0,
        slid: 0.0,
        below: Vec::new(),
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

#[test]
fn odd_sites_are_a_fill_or_a_row_and_say_so() {
    let c = synthetic_catalog();
    assert_eq!(
        bridge_kind(&c, &site(f64::INFINITY, Family::Asf1, ROOM)),
        BridgeKind::Fill
    );
    // an own turn that is not a number is no straight bridge
    let mut s = site(16.7, Family::Asf1, ROOM);
    s.turn_deg = f64::NAN;
    let choice = choose(&c, &s);
    assert!(matches!(
        choice.fallback.as_slice(),
        [Fallback::ModelBends { deg, .. }] if deg.is_infinite()
    ));
    assert!(format!("{s:?}").contains("BridgeSite"));
    for f in [
        Fallback::ModelNoRoom {
            model: "a\\b.p3d".to_string(),
        },
        Fallback::ModelBends {
            model: "a\\b.p3d".to_string(),
            deg: 7.0,
        },
        Fallback::NoParts,
    ] {
        assert_ne!(f.to_string(), "");
    }
}

#[test]
fn a_tunnel_or_the_same_road_at_both_ends_gives_no_room() {
    let mut tunnel = road(3, &[3, 4], &[(116.7, 0.0), (216.7, 0.0)]);
    if let FeatureKind::Road { tunnel: t, .. } = &mut tunnel.kind {
        *t = true;
    }
    let g = build_graph(&[
        road(1, &[1, 2], &[(0.0, 0.0), (100.0, 0.0)]),
        feature(
            2,
            Family::Asf1,
            true,
            1,
            &[2, 3],
            &[(100.0, 0.0), (116.7, 0.0)],
        ),
        tunnel,
    ]);
    let (_, fits) = fit(&g);
    assert_eq!(fits[0].room_end.1, RoomEnd::Tunnel);
    // a road that leaves the bridge's end and comes back to its start: it grows into it once
    let g = build_graph(&[
        road(
            1,
            &[1, 5, 6, 2],
            &[(0.0, 0.0), (0.0, 50.0), (16.7, 50.0), (16.7, 0.0)],
        ),
        feature(
            2,
            Family::Asf1,
            true,
            1,
            &[2, 1],
            &[(16.7, 0.0), (0.0, 0.0)],
        ),
    ]);
    let (_, fits) = fit(&g);
    assert_eq!(fits.len(), 1);
    assert!(
        fits[0].room_end.0 == RoomEnd::Junction || fits[0].room_end.1 == RoomEnd::Junction,
        "{:?}",
        fits[0]
    );
}

#[test]
fn bridges_are_joined_whichever_way_their_ways_run_and_a_ring_of_bridges_stays() {
    // the bridge with the lower id runs away from the shared node, the other towards it
    let g = build_graph(&[
        road(1, &[30, 40], &[(20.0, 0.0), (120.0, 0.0)]),
        feature(
            2,
            Family::Asf1,
            true,
            1,
            &[20, 30],
            &[(10.0, 0.0), (20.0, 0.0)],
        ),
        feature(
            3,
            Family::Asf1,
            true,
            2,
            &[10, 20],
            &[(0.0, 0.0), (10.0, 0.0)],
        ),
        road(4, &[5, 10], &[(-100.0, 0.0), (0.0, 0.0)]),
    ]);
    let (out, fits) = fit(&g);
    assert_eq!(fits.len(), 1, "{fits:?}");
    assert_eq!(fits[0].stretch, "2:20-30");
    assert!(near(fits[0].length, 20.0, 1e-9));
    let joined = out.stretches.iter().find(|s| s.id == "2:20-30").unwrap();
    assert!(joined.start < joined.end);
    assert!(near(total(&out), total(&g), 1e-6));
    // three bridges in a ring: each is a bridge of its own
    let ring = build_graph(&[
        feature(
            5,
            Family::Asf1,
            true,
            1,
            &[1, 2],
            &[(0.0, 0.0), (10.0, 0.0)],
        ),
        feature(
            6,
            Family::Asf1,
            true,
            2,
            &[2, 3],
            &[(10.0, 0.0), (5.0, 8.0)],
        ),
        feature(7, Family::Asf1, true, 3, &[3, 1], &[(5.0, 8.0), (0.0, 0.0)]),
    ]);
    let (_, fits) = fit(&ring);
    assert_eq!(fits.len(), 3, "{fits:?}");
    assert!(fits.iter().all(|f| f.stretches.len() == 1));
}

#[test]
fn a_short_model_is_centred_or_keeps_to_the_node_of_bridges_and_the_fit_says_where() {
    let c = synthetic_catalog();
    // the posts of bridge_wood_25 do not reach the ground in the middle: bridge_wood2 (19.7 m) on 25 m
    let valley = |at: f64, _: f64| if (2.0..20.0).contains(&at) { -4.0 } else { 0.0 };
    let gap = 25.0 - 19.7;
    let mut s = site_on(25.0, Family::Grav, ROOM, &valley);
    let choice = choose(&c, &s);
    assert_eq!(model(&choice.kind), Some(("bridge_wood2", 1)));
    assert!(near(choice.grown.0, -gap / 2.0, 1e-5) && near(choice.grown.1, -gap / 2.0, 1e-5));
    // it reaches the node of bridges at its start, or at its end
    s.node_ends = (true, false);
    let at_start = choose(&c, &s);
    assert!(
        near(at_start.grown.0, 0.0, 1e-5) && near(at_start.grown.1, -gap, 1e-5),
        "{at_start:?}"
    );
    s.node_ends = (false, true);
    let at_end = choose(&c, &s);
    assert!(
        near(at_end.grown.0, -gap, 1e-5) && near(at_end.grown.1, 0.0, 1e-5),
        "{at_end:?}"
    );
    // between two nodes of bridges it has no place: a panel grid, with the reason
    s.node_ends = (true, true);
    let between = choose(&c, &s);
    assert!(
        matches!(between.kind, BridgeKind::Panels { .. }),
        "{between:?}"
    );
    assert!(matches!(
        between.fallback.as_slice(),
        [Fallback::ModelBetweenBridges { model }] if model.ends_with("bridge_wood2.p3d")
    ));
    assert_ne!(between.fallback[0].to_string(), "");
}

#[test]
fn the_deck_thickness_is_the_panels_at_their_scale_or_the_model_measured() {
    use naves_core::bridgekind::deck_thickness;
    let c = synthetic_catalog();
    // a 9 m bridge on gravel: three panels at scale 1, the synthetic panel is 0.24 m thick
    let g = build_graph(&[
        road(1, &[1, 2], &[(-100.0, 0.0), (0.0, 0.0)]),
        feature(2, Family::Grav, true, 1, &[2, 3], &[(0.0, 0.0), (9.0, 0.0)]),
        road(3, &[3, 4], &[(9.0, 0.0), (109.0, 0.0)]),
    ]);
    let (out, _) = fit(&g);
    let panels = deck_thickness(&c, bridge_of(&out)).unwrap();
    assert!(near(panels, 0.24, 1e-5), "{panels}");
    // a long one: a chain of models, whose thickness is the one measured on the model
    let g = build_graph(&[
        road(1, &[1, 2], &[(-100.0, 0.0), (0.0, 0.0)]),
        feature(
            2,
            Family::Asf1,
            true,
            1,
            &[2, 3],
            &[(0.0, 0.0), (60.0, 0.0)],
        ),
        road(3, &[3, 4], &[(60.0, 0.0), (160.0, 0.0)]),
    ]);
    let (out, _) = fit(&g);
    let model = deck_thickness(&c, bridge_of(&out)).unwrap();
    assert!(model > 0.0, "{model}");
}

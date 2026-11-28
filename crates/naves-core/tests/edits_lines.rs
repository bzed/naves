// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Lines in map metres turned into node ids of the road edit set.

#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helper code; failing loudly is the point"
)]

use std::collections::BTreeMap;

use naves_core::edits::lines::{NodeIndex, check_line};
use naves_core::edits::{
    By, EditsError, ElementId, MAX_NODE_EDITS, MAX_WAY_NODES, NodeEdit, WayEdits,
};
use naves_core::heightmap::MapGeo;
use naves_core::osm::{OsmWay, OsmWays};
use naves_core::project::Project;

fn tags(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

struct Fixture {
    frame: MapGeo,
    ways: OsmWays,
    index: NodeIndex,
}

fn fixture() -> Fixture {
    // A real extent of 2000 m at the default scale of 0.5 is a map of 1000 m.
    let project = Project::new("T", 47.0, 12.0, 2000.0);
    let frame = MapGeo::new(&project).unwrap();
    let at = |x: f64, y: f64| frame.to_geo(x, y).unwrap();
    let way = |id: i64, nodes: Vec<i64>, pts: [(f64, f64); 3]| OsmWay {
        id,
        tags: tags(&[("highway", "residential")]),
        nodes,
        points: pts.iter().map(|&(x, y)| at(x, y)).collect(),
    };
    let ways = OsmWays {
        ways: vec![
            way(
                10,
                vec![1, 2, 3],
                [(100.0, 500.0), (500.0, 500.0), (900.0, 500.0)],
            ),
            way(
                20,
                vec![4, 2, 5],
                [(500.0, 100.0), (500.0, 500.0), (500.0, 900.0)],
            ),
        ],
        skipped: 0,
    };
    let index = NodeIndex::new(&ways, &frame);
    Fixture { frame, ways, index }
}

fn invalid_problem(result: Result<impl std::fmt::Debug, EditsError>) -> &'static str {
    match result {
        Err(EditsError::Invalid { problem, .. }) => problem,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn a_point_near_a_node_reuses_it() {
    let f = fixture();
    let mut edits = WayEdits::default();
    let ids = edits
        .line_nodes(&f.index, &f.frame, &[(500.004, 499.996)], By::Ai)
        .unwrap();
    assert_eq!(ids, vec![ElementId::Osm(2)]);
    assert_eq!(edits.nodes.len(), 0);
}

#[test]
fn a_point_off_a_node_is_a_new_node() {
    let f = fixture();
    let mut edits = WayEdits::default();
    let ids = edits
        .line_nodes(&f.index, &f.frame, &[(500.1, 500.0)], By::Ai)
        .unwrap();
    assert!(matches!(ids.as_slice(), [ElementId::User(_)]));
    assert_eq!(edits.nodes.len(), 1);
    assert_eq!(edits.nodes.first().map(|n| n.by), Some(By::Ai));
}

#[test]
fn a_closed_line_reuses_its_first_new_node() {
    let f = fixture();
    let mut edits = WayEdits::default();
    let ids = edits
        .line_nodes(
            &f.index,
            &f.frame,
            &[
                (100.0, 100.0),
                (200.0, 100.0),
                (200.0, 200.0),
                (100.0, 100.01),
            ],
            By::Ai,
        )
        .unwrap();
    assert_eq!(ids.len(), 4);
    assert_eq!(ids.first(), ids.get(3));
    assert_eq!(edits.nodes.len(), 3);
}

#[test]
fn add_line_needs_a_highway_tag() {
    let f = fixture();
    let mut edits = WayEdits::default();
    let before = edits.clone();
    let result = edits.add_line(
        &f.index,
        &f.frame,
        tags(&[("name", "x")]),
        &[(100.0, 100.0), (200.0, 200.0)],
        By::Ai,
    );
    assert_eq!(invalid_problem(result), "a road needs a highway tag");
    assert_eq!(edits, before);
}

#[test]
fn add_line_makes_a_way_joined_to_a_junction() {
    let f = fixture();
    let mut edits = WayEdits::default();
    let id = edits
        .add_line(
            &f.index,
            &f.frame,
            tags(&[("highway", "track")]),
            &[(500.0, 500.0), (600.0, 600.0)],
            By::Ai,
        )
        .unwrap();
    let way = edits.ways.iter().find(|w| w.id == id).unwrap();
    assert_eq!(way.nodes.first(), Some(&ElementId::Osm(2)));
    assert_eq!(edits.nodes.len(), 1);
}

#[test]
fn modify_line_keeps_an_unmoved_junction() {
    let f = fixture();
    let original = f.ways.ways.first().unwrap();
    let mut edits = WayEdits::default();
    edits
        .modify_line(
            &f.index,
            &f.frame,
            original,
            original,
            None,
            Some(&[(100.0, 505.0), (500.0, 500.0), (900.0, 500.0)]),
            By::Ai,
        )
        .unwrap();
    let way = edits.ways.first().unwrap();
    assert_eq!(way.nodes.get(1), Some(&ElementId::Osm(2)));
    assert_eq!(way.nodes.first(), Some(&ElementId::User(1)));
    assert_eq!(way.nodes.get(2), Some(&ElementId::Osm(3)));
}

#[test]
fn modify_line_drops_unused_user_nodes() {
    let f = fixture();
    let mut edits = WayEdits::default();
    let id = edits
        .add_line(
            &f.index,
            &f.frame,
            tags(&[("highway", "track")]),
            &[(100.0, 100.0), (200.0, 150.0), (300.0, 100.0)],
            By::Ai,
        )
        .unwrap();
    assert_eq!(edits.nodes.len(), 3);
    let way = edits.ways.first().unwrap().clone();
    let current = OsmWay {
        id: id.raw().unwrap(),
        tags: way.tags.clone(),
        nodes: way.nodes.iter().map(|n| n.raw().unwrap()).collect(),
        points: Vec::new(),
    };
    edits
        .modify_line(
            &f.index,
            &f.frame,
            &current,
            &current,
            None,
            Some(&[(100.0, 100.0), (300.0, 100.0)]),
            By::Ai,
        )
        .unwrap();
    assert_eq!(edits.nodes.len(), 2);
    let middle = way.nodes.get(1).unwrap();
    assert!(edits.nodes.iter().all(|n| &n.id != middle));
}

#[test]
fn modify_line_keeps_what_is_omitted() {
    let f = fixture();
    let original = f.ways.ways.first().unwrap();

    let mut edits = WayEdits::default();
    edits
        .modify_line(
            &f.index,
            &f.frame,
            original,
            original,
            Some(tags(&[("highway", "service")])),
            None,
            By::Ai,
        )
        .unwrap();
    let nodes: Vec<ElementId> = original.nodes.iter().map(|&n| ElementId::Osm(n)).collect();
    let edit = edits.ways.first().unwrap();
    assert_eq!(edit.nodes, nodes);
    assert_eq!(
        edit.tags.get("highway").map(String::as_str),
        Some("service")
    );

    let mut edits = WayEdits::default();
    edits
        .modify_line(
            &f.index,
            &f.frame,
            original,
            original,
            None,
            Some(&[(100.0, 505.0), (900.0, 500.0)]),
            By::Ai,
        )
        .unwrap();
    let mut expected = original.tags.clone();
    expected.insert("naves:keep".to_string(), "yes".to_string());
    assert_eq!(edits.ways.first().unwrap().tags, expected);
}

#[test]
fn modify_line_with_nothing_is_an_error() {
    let f = fixture();
    let original = f.ways.ways.first().unwrap();
    let mut edits = WayEdits::default();
    let result = edits.modify_line(&f.index, &f.frame, original, original, None, None, By::Ai);
    assert_eq!(invalid_problem(result), "nothing to change");
}

#[test]
fn check_line_limits() {
    let size = 1000.0;
    let problem = |points: &[(f64, f64)]| invalid_problem(check_line("line", points, size));
    assert_eq!(problem(&[(1.0, 1.0)]), "a way needs at least 2 nodes");
    let many = vec![(1.0, 1.0); MAX_WAY_NODES + 1];
    assert_eq!(problem(&many), "too many nodes");
    assert_eq!(
        problem(&[(-0.1, 5.0), (1.0, 1.0)]),
        "a point is outside the map"
    );
    assert_eq!(
        problem(&[(5.0, 1000.1), (1.0, 1.0)]),
        "a point is outside the map"
    );
    assert_eq!(
        problem(&[(f64::NAN, 5.0), (1.0, 1.0)]),
        "a point is not a finite number"
    );
    assert!(check_line("line", &[(0.0, 0.0), (1000.0, 1000.0)], size).is_ok());
}

#[test]
fn a_refused_line_changes_nothing() {
    let f = fixture();
    let mut edits = WayEdits::default();
    let before = edits.clone();
    let result = edits.line_nodes(
        &f.index,
        &f.frame,
        &[(100.0, 100.0), (200.0, 200.0), (5000.0, 5.0)],
        By::Ai,
    );
    assert!(result.is_err());
    assert_eq!(edits, before);
    let result = edits.add_line(
        &f.index,
        &f.frame,
        tags(&[("highway", "track")]),
        &[(100.0, 100.0), (200.0, 200.0), (5000.0, 5.0)],
        By::Ai,
    );
    assert!(result.is_err());
    assert_eq!(edits, before);
}

#[test]
fn a_failing_step_after_new_nodes_rolls_them_back() {
    let f = fixture();
    // One node edit is free: the first new point succeeds, the second hits the limit.
    let nodes = (0..MAX_NODE_EDITS - 1)
        .map(|i| NodeEdit {
            id: ElementId::Osm(i64::try_from(i).unwrap() + 1),
            lat: 47.0,
            lon: 12.0,
            base: None,
            by: By::User,
        })
        .collect();
    let mut edits = WayEdits {
        nodes,
        ..WayEdits::default()
    };
    let before = edits.clone();
    let result = edits.line_nodes(
        &f.index,
        &f.frame,
        &[(100.0, 100.0), (200.0, 200.0)],
        By::Ai,
    );
    assert_eq!(invalid_problem(result), "too many node edits");
    assert_eq!(edits, before);
}

fn index_of(nodes: &[(i64, f64, f64)], frame: &MapGeo) -> NodeIndex {
    let ways = OsmWays {
        ways: nodes
            .iter()
            .map(|&(id, x, y)| OsmWay {
                id: 1000 + id,
                tags: tags(&[("highway", "residential")]),
                nodes: vec![id],
                points: vec![frame.to_geo(x, y).unwrap()],
            })
            .collect(),
        skipped: 0,
    };
    NodeIndex::new(&ways, frame)
}

#[test]
fn near_picks_the_nearer_node_and_the_smaller_id_on_a_tie() {
    let f = fixture();
    let index = index_of(
        &[
            (7, 300.0, 300.0),
            (8, 300.03, 300.0),
            (9, 600.0, 600.0),
            (6, 600.0, 600.0),
        ],
        &f.frame,
    );
    assert_eq!(index.near(300.01, 300.0), Some(7));
    assert_eq!(index.near(300.025, 300.0), Some(8));
    assert_eq!(index.near(600.0, 600.0), Some(6));
    assert_eq!(index.near(300.5, 300.0), None);
}

#[test]
fn a_shared_node_is_found_and_a_rounded_point_snaps() {
    let f = fixture();
    // Node 2 is on both ways of the fixture.
    assert_eq!(f.index.near(500.0, 500.0), Some(2));
    // A 0.01 m rounding in both axes is 0.017 m off.
    assert_eq!(f.index.near(500.012, 499.988), Some(2));
    assert_eq!(f.index.near(500.04, 500.04), None);
}

#[test]
fn modify_line_refuses_bad_input_and_changes_nothing() {
    let f = fixture();
    let original = f.ways.ways.first().unwrap();
    let mut edits = WayEdits::default();
    let before = edits.clone();
    let result = edits.modify_line(
        &f.index,
        &f.frame,
        original,
        original,
        Some(tags(&[("name", "x")])),
        None,
        By::Ai,
    );
    assert_eq!(invalid_problem(result), "a road needs a highway tag");
    assert_eq!(edits, before);
    let result = edits.modify_line(
        &f.index,
        &f.frame,
        original,
        original,
        None,
        Some(&[(100.0, 100.0), (5000.0, 100.0)]),
        By::Ai,
    );
    assert_eq!(invalid_problem(result), "a point is outside the map");
    assert_eq!(edits, before);
}

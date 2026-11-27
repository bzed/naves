// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Hand-made edits of lots: closed ways with lot tags replace, add or delete the way lots of the extract.

use std::collections::BTreeMap;

use naves_core::edits::{
    By, EditIssue, EditReport, ElementId, NodeEdit, WayAction, WayEdit, WayEdits, apply_lot_edits,
    apply_way_edits,
};
use naves_core::osm::{LotPolygon, OsmWay, OsmWays};

fn tags(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn square(lat: f64, lon: f64, size: f64) -> Vec<(f64, f64)> {
    vec![
        (lat, lon),
        (lat, lon + size),
        (lat + size, lon + size),
        (lat + size, lon),
        (lat, lon),
    ]
}

fn lot(id: i64, relation: bool, part: usize) -> LotPolygon {
    LotPolygon {
        id,
        relation,
        part,
        tags: tags(&[("amenity", "parking")]),
        outer: square(47.0, 12.0, 0.001),
        holes: Vec::new(),
    }
}

fn lots() -> Vec<LotPolygon> {
    vec![lot(50, false, 0), lot(60, false, 0), lot(7, true, 0)]
}

fn no_ways() -> OsmWays {
    OsmWays::default()
}

fn node_edits(first: i64, ring: &[(f64, f64)]) -> Vec<NodeEdit> {
    ring.iter()
        .take(ring.len() - 1)
        .zip(first..)
        .map(|(&(lat, lon), id)| NodeEdit {
            id: ElementId::Osm(id),
            lat,
            lon,
            base: None,
            by: By::User,
        })
        .collect()
}

/// A closed way over the nodes `first..first + 4`, first node again at the end.
fn closed(id: ElementId, action: WayAction, first: i64, t: &[(&str, &str)]) -> WayEdit {
    let mut nodes: Vec<ElementId> = (first..first + 4).map(ElementId::Osm).collect();
    nodes.push(ElementId::Osm(first));
    WayEdit {
        id,
        action,
        tags: tags(t),
        nodes,
        base: None,
        by: By::User,
    }
}

const PARKING: &[(&str, &str)] = &[("amenity", "parking")];

#[test]
fn modify_replaces_the_way_lot_of_that_id() {
    let ring = square(48.0, 13.0, 0.002);
    let mut edits = WayEdits {
        nodes: node_edits(900, &ring),
        ..WayEdits::default()
    };
    edits.ways.push(closed(
        ElementId::Osm(50),
        WayAction::Modify,
        900,
        &[("amenity", "parking"), ("surface", "gravel"), ("name", "x")],
    ));
    let (out, report) = apply_lot_edits(&lots(), &no_ways(), &edits);
    assert_eq!(out.len(), 3);
    let replaced = out.iter().find(|l| l.id == 50).unwrap();
    assert_eq!(replaced.outer, ring);
    assert_eq!(
        replaced.tags,
        tags(&[("amenity", "parking"), ("surface", "gravel")]),
        "only the lot tags are kept"
    );
    assert!(replaced.holes.is_empty() && !replaced.relation && replaced.part == 0);
    assert_eq!(out.iter().find(|l| l.id == 60).unwrap(), &lot(60, false, 0));
    assert_eq!(report.issues.len(), 0);
}

#[test]
fn add_inserts_a_lot_in_id_order_before_the_relations() {
    let ring = square(48.0, 13.0, 0.002);
    let mut edits = WayEdits {
        nodes: node_edits(900, &ring),
        ..WayEdits::default()
    };
    edits.ways.push(closed(
        ElementId::User(1),
        WayAction::Add,
        900,
        &[("tourism", "camp_site")],
    ));
    let (out, report) = apply_lot_edits(&lots(), &no_ways(), &edits);
    let ids: Vec<(i64, bool)> = out.iter().map(|l| (l.id, l.relation)).collect();
    assert_eq!(
        ids,
        [
            (50, false),
            (60, false),
            (naves_core::edits::USER_ID_BASE + 1, false),
            (7, true)
        ]
    );
    assert_eq!(report.issues.len(), 0);
}

#[test]
fn delete_removes_the_way_lot_and_counts_it() {
    let mut edits = WayEdits::default();
    edits.ways.push(WayEdit {
        id: ElementId::Osm(60),
        action: WayAction::Delete,
        tags: BTreeMap::new(),
        nodes: Vec::new(),
        base: None,
        by: By::User,
    });
    let (out, report) = apply_lot_edits(&lots(), &no_ways(), &edits);
    assert_eq!(out.iter().map(|l| l.id).collect::<Vec<_>>(), [50, 7]);
    assert_eq!(report.applied, 1);
    assert_eq!(report.issues.len(), 0);
}

#[test]
fn a_modify_that_makes_the_way_no_lot_removes_the_lot() {
    let mut edits = WayEdits::default();
    edits.ways.push(closed(
        ElementId::Osm(50),
        WayAction::Modify,
        900,
        &[("highway", "service")],
    ));
    let (out, _) = apply_lot_edits(&lots(), &no_ways(), &edits);
    assert_eq!(out.iter().map(|l| l.id).collect::<Vec<_>>(), [60, 7]);
}

#[test]
fn relations_and_other_ways_are_left_alone() {
    let mut edits = WayEdits::default();
    // a road edit with the id of the relation, an add of a road, an open way with lot tags
    edits
        .ways
        .push(closed(ElementId::Osm(7), WayAction::Modify, 900, PARKING));
    edits.ways.push(closed(
        ElementId::User(2),
        WayAction::Add,
        900,
        &[("highway", "track")],
    ));
    let mut open = closed(ElementId::User(3), WayAction::Add, 900, PARKING);
    open.nodes.pop();
    edits.ways.push(open);
    edits.nodes = node_edits(900, &square(48.0, 13.0, 0.002));
    let (out, report) = apply_lot_edits(&lots(), &no_ways(), &edits);
    // the id 7 modify becomes a way lot 7, the relation 7 stays
    assert_eq!(
        out.iter().map(|l| (l.id, l.relation)).collect::<Vec<_>>(),
        [(7, false), (50, false), (60, false), (7, true)]
    );
    assert!(
        report
            .issues
            .iter()
            .all(|(i, _)| matches!(i, EditIssue::OriginalGone { .. }))
    );
}

#[test]
fn a_node_without_a_position_breaks_the_edit() {
    let mut edits = WayEdits::default();
    edits
        .ways
        .push(closed(ElementId::Osm(50), WayAction::Modify, 900, PARKING));
    let (out, report) = apply_lot_edits(&lots(), &no_ways(), &edits);
    assert_eq!(out.iter().map(|l| l.id).collect::<Vec<_>>(), [60, 7]);
    assert_eq!(report.issues.len(), 1);
    assert_eq!(
        report.issues[0].0,
        EditIssue::Broken {
            way: ElementId::Osm(50),
            node: ElementId::Osm(900)
        }
    );
}

#[test]
fn nodes_come_from_the_ways_when_no_node_edit_has_them() {
    let ring = square(48.0, 13.0, 0.002);
    let ways = OsmWays {
        ways: vec![OsmWay {
            id: 1,
            tags: tags(&[("highway", "service")]),
            nodes: vec![900, 901, 902, 903],
            points: ring[..4].to_vec(),
        }],
        skipped: 0,
    };
    let mut edits = WayEdits::default();
    edits
        .ways
        .push(closed(ElementId::User(1), WayAction::Add, 900, PARKING));
    let (out, report) = apply_lot_edits(&lots(), &ways, &edits);
    let added = out.last().filter(|l| !l.relation);
    assert!(out.iter().any(|l| l.outer == ring), "{added:?}");
    assert_eq!(report.issues.len(), 0);
}

#[test]
fn a_gone_lot_is_reported_once_by_the_two_reports_together() {
    let mut edits = WayEdits::default();
    // a delete of an id that is neither a lot nor a road, a modify of a lot, a modify of a road that is gone
    for id in [999, 50] {
        edits.ways.push(WayEdit {
            id: ElementId::Osm(id),
            action: WayAction::Modify,
            tags: tags(PARKING),
            nodes: Vec::new(),
            base: None,
            by: By::User,
        });
    }
    edits.ways.push(WayEdit {
        id: ElementId::Osm(998),
        action: WayAction::Delete,
        tags: BTreeMap::new(),
        nodes: Vec::new(),
        base: None,
        by: By::User,
    });
    let before = lots();
    let (_, mut report) = apply_way_edits(&no_ways(), &edits);
    let (_, lot_report) = apply_lot_edits(&before, &no_ways(), &edits);
    report.absorb_lots(lot_report, &before);
    let gone: Vec<&ElementId> = report
        .issues
        .iter()
        .filter_map(|(i, _)| match i {
            EditIssue::OriginalGone { way } => Some(way),
            _ => None,
        })
        .collect();
    assert_eq!(gone, [&ElementId::Osm(998), &ElementId::Osm(999)]);
}

#[test]
fn absorb_adds_the_counts() {
    let mut a = EditReport {
        applied: 2,
        issues: Vec::new(),
    };
    a.absorb_lots(
        EditReport {
            applied: 3,
            issues: Vec::new(),
        },
        &[],
    );
    assert_eq!(a.applied, 5);
}

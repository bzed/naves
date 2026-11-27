// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Element ids of the hand-made edits: OSM ids and user ids `u<n>`.

use naves_core::edits::{ElementId, USER_ID_BASE};
use serde::Deserialize;

#[test]
fn user_ids_read_and_write_as_u_n() {
    let id: ElementId = "u7".parse().unwrap();
    assert_eq!(id, ElementId::User(7));
    assert_eq!(id.to_string(), "u7");
    assert_eq!(id.raw(), Some(USER_ID_BASE + 7));
    assert_eq!(ElementId::from_raw(USER_ID_BASE + 7), Some(id));
}

#[test]
fn osm_ids_stay_plain() {
    assert_eq!("8421".parse::<ElementId>().unwrap(), ElementId::Osm(8421));
    assert_eq!(ElementId::Osm(8421).raw(), Some(8421));
    assert_eq!(ElementId::from_raw(8421), Some(ElementId::Osm(8421)));
}

#[test]
fn bad_ids_are_refused() {
    for text in [
        "",
        "u",
        "u0",
        "u-1",
        "x3",
        "0",
        "-5",
        "1099511627776",
        "u99999999999999999999",
    ] {
        assert!(text.parse::<ElementId>().is_err(), "{text}");
    }
}

#[test]
fn raw_refuses_what_does_not_fit() {
    assert_eq!(ElementId::User(0).raw(), None);
    assert_eq!(ElementId::User(u64::MAX).raw(), None);
    assert_eq!(ElementId::from_raw(0), None);
    assert_eq!(ElementId::from_raw(-3), None);
    assert_eq!(ElementId::from_raw(USER_ID_BASE), None);
}

#[test]
fn is_user_tells_the_kinds_apart() {
    assert!(ElementId::User(1).is_user());
    assert!(!ElementId::Osm(1).is_user());
    assert!(ElementId::Osm(5) < ElementId::User(1));
}

#[derive(Deserialize)]
struct Holder {
    id: ElementId,
}

#[test]
fn ids_read_from_toml_as_integer_or_u_string() {
    let osm: Holder = toml::from_str("id = 8421").unwrap();
    assert_eq!(osm.id, ElementId::Osm(8421));
    let user: Holder = toml::from_str("id = \"u7\"").unwrap();
    assert_eq!(user.id, ElementId::User(7));
}

#[test]
fn bad_ids_in_toml_are_refused() {
    for text in [
        "id = 0",
        "id = -1",
        "id = 1099511627776",
        "id = \"u0\"",
        "id = \"x\"",
        "id = 1.5",
    ] {
        assert!(toml::from_str::<Holder>(text).is_err(), "{text}");
    }
}

#[test]
fn ids_write_to_toml_as_integer_or_u_string() {
    #[derive(serde::Serialize)]
    struct Out {
        id: ElementId,
    }
    let osm = toml::to_string(&Out {
        id: ElementId::Osm(8421),
    })
    .unwrap();
    assert_eq!(osm, "id = 8421\n");
    let user = toml::to_string(&Out {
        id: ElementId::User(7),
    })
    .unwrap();
    assert_eq!(user, "id = \"u7\"\n");
}

// ---- edits/ways.toml ----

use std::collections::BTreeMap;

use naves_core::edits::{By, EditsError, NodeEdit, WayAction, WayEdit, WayEdits, fingerprint};

fn tags(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn user(n: u64) -> ElementId {
    ElementId::User(n)
}

fn way(id: ElementId, action: WayAction, nodes: Vec<ElementId>) -> WayEdit {
    WayEdit {
        id,
        action,
        tags: tags(&[("highway", "track")]),
        nodes,
        base: None,
        by: By::User,
    }
}

fn sample() -> WayEdits {
    let mut modify = way(
        ElementId::Osm(8421),
        WayAction::Modify,
        vec![ElementId::Osm(812), user(1)],
    );
    modify.base = Some("0123456789abcdef".to_string());
    WayEdits {
        format: 1,
        next_id: 3,
        nodes: vec![
            NodeEdit {
                id: user(1),
                lat: 47.5,
                lon: 12.9,
                base: None,
                by: By::User,
            },
            NodeEdit {
                id: ElementId::Osm(812),
                lat: 47.6,
                lon: 13.0,
                base: Some((47.59, 12.99)),
                by: By::Ai,
            },
        ],
        ways: vec![
            way(user(2), WayAction::Add, vec![user(1), ElementId::Osm(812)]),
            modify,
        ],
    }
}

#[test]
fn ways_file_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let edits = sample();
    edits.save(dir.path()).unwrap();
    let text = std::fs::read_to_string(dir.path().join("edits/ways.toml")).unwrap();
    assert!(text.contains("id = \"u1\""), "{text}");
    assert!(text.contains("nodes = ["), "{text}");
    assert_eq!(WayEdits::load(dir.path()).unwrap(), edits);
}

#[test]
fn missing_file_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let edits = WayEdits::load(dir.path()).unwrap();
    assert_eq!(edits, WayEdits::default());
    assert_eq!((edits.format, edits.next_id), (1, 1));
}

#[test]
fn duplicate_way_is_refused() {
    let mut edits = WayEdits::default();
    for _ in 0..2 {
        edits.ways.push(way(
            ElementId::Osm(8421),
            WayAction::Modify,
            vec![ElementId::Osm(1), ElementId::Osm(2)],
        ));
    }
    match edits.validate() {
        Err(EditsError::Invalid { what, .. }) => assert_eq!(what, "way 8421"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn repeated_node_is_refused_except_closing() {
    let with = |nodes: Vec<ElementId>| WayEdits {
        next_id: 10,
        ways: vec![way(user(3), WayAction::Add, nodes)],
        ..WayEdits::default()
    };
    assert!(with(vec![user(1), user(2), user(1)]).validate().is_ok());
    assert!(with(vec![user(1), user(1), user(2)]).validate().is_err());
    assert!(with(vec![user(1), user(2), user(2)]).validate().is_err());
}

#[test]
fn limits_are_checked() {
    let long: Vec<ElementId> = (1..=2001).map(ElementId::Osm).collect();
    let edits = WayEdits {
        ways: vec![way(ElementId::Osm(5), WayAction::Modify, long)],
        ..WayEdits::default()
    };
    assert!(edits.validate().is_err());

    let node = |lat: f64, lon: f64| WayEdits {
        nodes: vec![NodeEdit {
            id: ElementId::Osm(1),
            lat,
            lon,
            base: None,
            by: By::User,
        }],
        ..WayEdits::default()
    };
    assert!(node(90.0, 180.0).validate().is_ok());
    assert!(node(91.0, 0.0).validate().is_err());
    assert!(node(f64::NAN, 0.0).validate().is_err());
    assert!(node(0.0, 181.0).validate().is_err());
    assert!(
        WayEdits::parse("format = 1\nnext_id = 1\n[[nodes]]\nid = 1\nlat = nan\nlon = 0.0\n")
            .is_err()
    );

    let mut many_tags = way(
        ElementId::Osm(5),
        WayAction::Modify,
        vec![ElementId::Osm(1), ElementId::Osm(2)],
    );
    many_tags.tags = (0..65)
        .map(|i| (format!("k{i}"), "v".to_string()))
        .collect();
    let edits = WayEdits {
        ways: vec![many_tags],
        ..WayEdits::default()
    };
    assert!(edits.validate().is_err());

    let mut long_value = way(
        ElementId::Osm(5),
        WayAction::Modify,
        vec![ElementId::Osm(1), ElementId::Osm(2)],
    );
    long_value.tags = tags(&[("name", &"ä".repeat(256))]);
    let edits = WayEdits {
        ways: vec![long_value.clone()],
        ..WayEdits::default()
    };
    assert!(edits.validate().is_err());
    long_value.tags = tags(&[("name", &"ä".repeat(255))]);
    let edits = WayEdits {
        ways: vec![long_value],
        ..WayEdits::default()
    };
    assert!(edits.validate().is_ok());
}

#[test]
fn add_needs_user_id_and_modify_needs_osm_id() {
    let two = vec![ElementId::Osm(1), ElementId::Osm(2)];
    let wrong = [
        way(ElementId::Osm(5), WayAction::Add, two.clone()),
        way(user(1), WayAction::Modify, two.clone()),
        way(user(1), WayAction::Delete, vec![]),
    ];
    for w in wrong {
        let edits = WayEdits {
            next_id: 10,
            ways: vec![w],
            ..WayEdits::default()
        };
        assert!(edits.validate().is_err());
    }
}

#[test]
fn other_rules_are_checked() {
    let mut edits = sample();
    assert!(edits.validate().is_ok());
    edits.next_id = 2; // user way u2 is in use
    assert!(edits.validate().is_err());
    edits.next_id = 3;
    edits.format = 2;
    assert!(edits.validate().is_err());

    let mut edits = sample();
    edits.nodes[0].base = Some((1.0, 2.0)); // user node with a base
    assert!(edits.validate().is_err());

    let mut edits = sample();
    edits.nodes.push(edits.nodes[0].clone()); // node twice
    assert!(edits.validate().is_err());

    let mut edits = sample();
    edits.ways[0].nodes = vec![user(1)]; // too few nodes
    assert!(edits.validate().is_err());

    let mut edits = sample();
    edits.ways[0].tags = tags(&[("", "x")]); // empty key
    assert!(edits.validate().is_err());

    let del = |tags_: BTreeMap<String, String>, nodes: Vec<ElementId>| WayEdits {
        ways: vec![WayEdit {
            id: ElementId::Osm(9),
            action: WayAction::Delete,
            tags: tags_,
            nodes,
            base: None,
            by: By::User,
        }],
        ..WayEdits::default()
    };
    assert!(del(BTreeMap::new(), vec![]).validate().is_ok());
    assert!(del(tags(&[("a", "b")]), vec![]).validate().is_err());
    assert!(
        del(BTreeMap::new(), vec![ElementId::Osm(1)])
            .validate()
            .is_err()
    );

    // a user id used by a node but not below next_id
    let mut edits = sample();
    edits.next_id = 1;
    edits.ways.clear();
    assert!(edits.validate().is_err());
    // a user node id referenced only in a way's node list
    let edits = WayEdits {
        next_id: 4,
        ways: vec![way(
            ElementId::Osm(5),
            WayAction::Modify,
            vec![user(7), ElementId::Osm(1)],
        )],
        ..WayEdits::default()
    };
    assert!(edits.validate().is_err());
}

#[test]
fn fingerprint_is_stable() {
    let base = fingerprint(&tags(&[("highway", "track")]), &[1, 2]);
    assert_eq!(base, "36a3e00c2348512b");
    assert_eq!(base.len(), 16);
    assert_ne!(base, fingerprint(&tags(&[("highway", "path")]), &[1, 2]));
    assert_ne!(base, fingerprint(&tags(&[("highway", "track")]), &[1, 3]));
}

#[test]
fn damaged_file_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("edits")).unwrap();
    std::fs::write(dir.path().join("edits/ways.toml"), "format = [").unwrap();
    assert!(matches!(
        WayEdits::load(dir.path()),
        Err(EditsError::Parse(_))
    ));
}

#[test]
fn base_on_add_is_refused() {
    let mut edits = sample();
    edits.ways[0].base = Some("0123456789abcdef".to_string());
    assert!(matches!(
        edits.validate(),
        Err(EditsError::Invalid {
            problem: "an added way has no base",
            ..
        })
    ));
}

#[test]
fn bad_base_is_refused() {
    for bad in ["xyz", "0123456789abcdef0", "0123456789ABCDEF", ""] {
        let mut edits = sample();
        edits.ways[1].base = Some(bad.to_string());
        assert!(
            matches!(
                edits.validate(),
                Err(EditsError::Invalid {
                    problem: "base is not a fingerprint",
                    ..
                })
            ),
            "{bad}"
        );
    }
    assert!(sample().validate().is_ok());
}

mod apply {
    use std::collections::BTreeMap;

    use naves_core::edits::{
        By, EditIssue, ElementId, NodeEdit, USER_ID_BASE, WayAction, WayEdit, WayEdits,
        apply_way_edits, fingerprint,
    };
    use naves_core::osm::{OsmWay, OsmWays};

    fn tags(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn way(id: i64, nodes: &[i64]) -> OsmWay {
        OsmWay {
            id,
            tags: tags(&[("highway", "track")]),
            nodes: nodes.to_vec(),
            points: nodes
                .iter()
                .map(|&n| i32::try_from(n).unwrap_or(0))
                .map(|n| (47.0 + f64::from(n) * 0.001, 12.0 + f64::from(n) * 0.001))
                .collect(),
        }
    }

    fn osm() -> OsmWays {
        OsmWays {
            ways: vec![way(10, &[1, 5, 2]), way(20, &[5, 3, 4])],
            skipped: 3,
        }
    }

    fn node_edit(id: ElementId, lat: f64, lon: f64) -> NodeEdit {
        NodeEdit {
            id,
            lat,
            lon,
            base: None,
            by: By::User,
        }
    }

    fn way_edit(id: ElementId, action: WayAction, nodes: &[ElementId]) -> WayEdit {
        WayEdit {
            id,
            action,
            tags: if action == WayAction::Delete {
                BTreeMap::new()
            } else {
                tags(&[("highway", "service")])
            },
            nodes: nodes.to_vec(),
            base: None,
            by: By::User,
        }
    }

    fn osm_ids(ids: &[i64]) -> Vec<ElementId> {
        ids.iter().map(|&i| ElementId::Osm(i)).collect()
    }

    fn find(ways: &OsmWays, id: i64) -> Option<&OsmWay> {
        ways.ways.iter().find(|w| w.id == id)
    }

    #[test]
    fn moved_node_moves_in_every_way() {
        let mut edits = WayEdits::default();
        edits.nodes.push(node_edit(ElementId::Osm(5), 1.5, 2.5));
        let (out, report) = apply_way_edits(&osm(), &edits);
        assert_eq!(find(&out, 10).unwrap().points[1], (1.5, 2.5));
        assert_eq!(find(&out, 20).unwrap().points[0], (1.5, 2.5));
        assert_eq!(find(&out, 10).unwrap().points[0], (47.001, 12.001));
        assert_eq!(out.skipped, 3);
        assert_eq!(report.applied, 1);
        assert_eq!(report.issues.len(), 0);
    }

    #[test]
    fn modify_replaces_tags_and_nodes() {
        let mut edits = WayEdits::default();
        edits.ways.push(way_edit(
            ElementId::Osm(10),
            WayAction::Modify,
            &osm_ids(&[1, 3]),
        ));
        let (out, report) = apply_way_edits(&osm(), &edits);
        let w = find(&out, 10).unwrap();
        assert_eq!(w.nodes, vec![1, 3]);
        assert_eq!(w.tags, tags(&[("highway", "service")]));
        assert_eq!(w.points, vec![(47.001, 12.001), (47.003, 12.003)]);
        assert_eq!(report.applied, 1);
        assert_eq!(report.issues.len(), 0);
    }

    #[test]
    fn delete_removes_way() {
        let mut edits = WayEdits::default();
        edits
            .ways
            .push(way_edit(ElementId::Osm(10), WayAction::Delete, &[]));
        let (out, report) = apply_way_edits(&osm(), &edits);
        assert!(find(&out, 10).is_none());
        assert!(find(&out, 20).is_some());
        assert_eq!(report.applied, 1);
    }

    #[test]
    fn add_inserts_user_way_with_raw_ids() {
        let mut edits = WayEdits {
            next_id: 10,
            ..WayEdits::default()
        };
        edits.nodes.push(node_edit(ElementId::User(1), 48.0, 13.0));
        edits.nodes.push(node_edit(ElementId::User(2), 48.1, 13.1));
        edits.ways.push(way_edit(
            ElementId::User(1),
            WayAction::Add,
            &[ElementId::User(1), ElementId::User(2)],
        ));
        let (out, report) = apply_way_edits(&osm(), &edits);
        let w = find(&out, USER_ID_BASE + 1).unwrap();
        assert_eq!(w.nodes, vec![USER_ID_BASE + 1, USER_ID_BASE + 2]);
        assert_eq!(w.points, vec![(48.0, 13.0), (48.1, 13.1)]);
        assert_eq!(out.ways.last().unwrap().id, USER_ID_BASE + 1);
        assert_eq!(report.applied, 1);
    }

    #[test]
    fn added_way_joins_existing_node() {
        let mut edits = WayEdits {
            next_id: 10,
            ..WayEdits::default()
        };
        edits.nodes.push(node_edit(ElementId::User(1), 48.0, 13.0));
        edits.ways.push(way_edit(
            ElementId::User(1),
            WayAction::Add,
            &[ElementId::Osm(5), ElementId::User(1)],
        ));
        let (out, _) = apply_way_edits(&osm(), &edits);
        let w = find(&out, USER_ID_BASE + 1).unwrap();
        assert_eq!(w.points[0], (47.005, 12.005));
    }

    #[test]
    fn missing_node_is_broken() {
        let mut edits = WayEdits::default();
        edits.ways.push(way_edit(
            ElementId::Osm(10),
            WayAction::Modify,
            &osm_ids(&[1, 999]),
        ));
        let (out, report) = apply_way_edits(&osm(), &edits);
        assert!(find(&out, 10).is_none());
        assert!(find(&out, 20).is_some());
        assert_eq!(report.applied, 0);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(
            report.issues[0].0,
            EditIssue::Broken {
                way: ElementId::Osm(10),
                node: ElementId::Osm(999)
            }
        );
        assert_eq!(report.issues[0].1, (47.001, 12.001));
    }

    #[test]
    fn gone_original_still_applies() {
        let mut edits = WayEdits::default();
        edits.ways.push(way_edit(
            ElementId::Osm(77),
            WayAction::Modify,
            &osm_ids(&[1, 2]),
        ));
        let (out, report) = apply_way_edits(&osm(), &edits);
        assert!(find(&out, 77).is_some());
        assert_eq!(
            report.issues[0].0,
            EditIssue::OriginalGone {
                way: ElementId::Osm(77)
            }
        );
        assert_eq!(report.applied, 1);
    }

    #[test]
    fn gone_delete_changes_nothing() {
        let mut edits = WayEdits::default();
        edits
            .ways
            .push(way_edit(ElementId::Osm(77), WayAction::Delete, &[]));
        let (out, report) = apply_way_edits(&osm(), &edits);
        assert_eq!(out.ways.len(), 2);
        assert_eq!(report.applied, 0);
        assert_eq!(report.issues.len(), 1);
    }

    #[test]
    fn changed_original_is_reported() {
        let mut edits = WayEdits::default();
        let mut e = way_edit(ElementId::Osm(10), WayAction::Modify, &osm_ids(&[1, 2]));
        e.base = Some("0000000000000000".to_string());
        edits.ways.push(e);
        let (out, report) = apply_way_edits(&osm(), &edits);
        assert_eq!(find(&out, 10).unwrap().nodes, vec![1, 2]);
        assert_eq!(
            report.issues[0].0,
            EditIssue::OriginalChanged {
                way: ElementId::Osm(10)
            }
        );
        assert_eq!(report.applied, 1);
    }

    #[test]
    fn matching_base_is_not_reported() {
        let base = osm();
        let w = find(&base, 10).unwrap();
        let mut edits = WayEdits::default();
        let mut e = way_edit(ElementId::Osm(10), WayAction::Delete, &[]);
        e.base = Some(fingerprint(&w.tags, &w.nodes));
        edits.ways.push(e);
        let (_, report) = apply_way_edits(&base, &edits);
        assert_eq!(report.issues.len(), 0);
        assert_eq!(report.applied, 1);
    }

    #[test]
    fn order_of_file_does_not_matter() {
        let mut a = WayEdits {
            next_id: 10,
            ..WayEdits::default()
        };
        a.nodes.push(node_edit(ElementId::Osm(5), 1.5, 2.5));
        a.nodes.push(node_edit(ElementId::User(1), 48.0, 13.0));
        a.ways
            .push(way_edit(ElementId::Osm(20), WayAction::Delete, &[]));
        a.ways.push(way_edit(
            ElementId::User(1),
            WayAction::Add,
            &[ElementId::Osm(5), ElementId::User(1)],
        ));
        a.ways.push(way_edit(
            ElementId::Osm(10),
            WayAction::Modify,
            &osm_ids(&[1, 999]),
        ));
        let mut b = a.clone();
        b.nodes.reverse();
        b.ways.reverse();
        let mut reversed = osm();
        reversed.ways.reverse();
        let (out_a, rep_a) = apply_way_edits(&osm(), &a);
        let (out_b, rep_b) = apply_way_edits(&reversed, &b);
        assert_eq!(out_a, out_b);
        assert_eq!(rep_a, rep_b);
        let ids: Vec<i64> = out_a.ways.iter().map(|w| w.id).collect();
        assert!(ids.windows(2).all(|p| p[0] < p[1]));
    }

    #[test]
    fn no_osm_is_fine() {
        let mut edits = WayEdits {
            next_id: 10,
            ..WayEdits::default()
        };
        edits.nodes.push(node_edit(ElementId::User(1), 48.0, 13.0));
        edits.nodes.push(node_edit(ElementId::User(2), 48.1, 13.1));
        edits.ways.push(way_edit(
            ElementId::User(1),
            WayAction::Add,
            &[ElementId::User(1), ElementId::User(2)],
        ));
        let (out, _) = apply_way_edits(&OsmWays::default(), &edits);
        assert_eq!(out.ways.len(), 1);
    }

    #[test]
    fn broken_without_known_node_uses_node_edit_or_zero() {
        let mut edits = WayEdits {
            next_id: 10,
            ..WayEdits::default()
        };
        edits.nodes.push(node_edit(ElementId::User(1), 48.0, 13.0));
        edits.ways.push(way_edit(
            ElementId::User(1),
            WayAction::Add,
            &[ElementId::Osm(900), ElementId::User(1)],
        ));
        edits.ways.push(way_edit(
            ElementId::Osm(5),
            WayAction::Modify,
            &osm_ids(&[901, 902]),
        ));
        let (out, report) = apply_way_edits(&OsmWays::default(), &edits);
        assert_eq!(out.ways.len(), 0);
        let positions: Vec<(f64, f64)> = report
            .issues
            .iter()
            .filter(|i| matches!(i.0, EditIssue::Broken { .. }))
            .map(|i| i.1)
            .collect();
        // Sorted by way id: the OSM way first, then the user way.
        assert_eq!(positions, vec![(0.0, 0.0), (48.0, 13.0)]);
    }

    #[test]
    fn node_of_a_deleted_way_still_counts_as_known() {
        let mut edits = WayEdits::default();
        edits
            .ways
            .push(way_edit(ElementId::Osm(20), WayAction::Delete, &[]));
        edits.ways.push(way_edit(
            ElementId::Osm(10),
            WayAction::Modify,
            &osm_ids(&[1, 3]),
        ));
        let (out, report) = apply_way_edits(&osm(), &edits);
        assert_eq!(find(&out, 10).unwrap().points[1], (47.003, 12.003));
        assert_eq!(report.issues.len(), 0);
    }
}

// ---- edit operations ----

use naves_core::osm::OsmWay;

fn osm_way(id: i64) -> OsmWay {
    OsmWay {
        id,
        tags: tags(&[("highway", "track")]),
        nodes: vec![10, 11],
        points: vec![(47.0, 12.0), (47.1, 12.1)],
    }
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn ops_nodes(edits: &mut WayEdits, n: u32) -> Vec<ElementId> {
    (0..n)
        .map(|i| {
            edits
                .new_node(47.0 + f64::from(i) * 0.001, 12.0, By::User)
                .unwrap()
        })
        .collect()
}

#[test]
fn add_way_sets_keep() {
    let mut e = WayEdits::default();
    let n = ops_nodes(&mut e, 2);
    let id = e
        .add_way(tags(&[("highway", "track")]), n, By::User)
        .unwrap();
    let w = e.ways.iter().find(|w| w.id == id).unwrap();
    assert_eq!(w.tags.get("naves:keep").map(String::as_str), Some("yes"));
    assert_eq!(w.action, WayAction::Add);
    e.validate().unwrap();
}

#[test]
fn keep_no_is_respected() {
    let mut e = WayEdits::default();
    let n = ops_nodes(&mut e, 2);
    let id = e
        .add_way(
            tags(&[("highway", "track"), ("naves:keep", "no")]),
            n,
            By::User,
        )
        .unwrap();
    let w = e.ways.iter().find(|w| w.id == id).unwrap();
    assert_eq!(w.tags.get("naves:keep").map(String::as_str), Some("no"));
}

#[test]
fn modify_twice_keeps_first_base() {
    let mut e = WayEdits::default();
    let orig = osm_way(8421);
    let nodes = vec![ElementId::Osm(10), ElementId::Osm(11)];
    e.modify_way(&orig, tags(&[("highway", "path")]), nodes.clone(), By::User)
        .unwrap();
    let base = e.ways[0].base.clone();
    assert_eq!(base, Some(fingerprint(&orig.tags, &orig.nodes)));
    let mut changed = osm_way(8421);
    changed.nodes = vec![10, 12];
    e.modify_way(&changed, tags(&[("highway", "track")]), nodes, By::Ai)
        .unwrap();
    assert_eq!(e.ways.len(), 1);
    assert_eq!(e.ways[0].base, base);
    assert_eq!(e.ways[0].by, By::Ai);
    e.validate().unwrap();
}

#[test]
fn modify_user_way_updates_add() {
    let mut e = WayEdits::default();
    let n = ops_nodes(&mut e, 3);
    let id = e
        .add_way(tags(&[("highway", "track")]), n[..2].to_vec(), By::User)
        .unwrap();
    let raw = id.raw().unwrap();
    let mut o = osm_way(raw);
    o.nodes.clear();
    e.modify_way(&o, tags(&[("highway", "path")]), n.clone(), By::User)
        .unwrap();
    assert_eq!(e.ways.len(), 1);
    assert_eq!(e.ways[0].action, WayAction::Add);
    assert_eq!(e.ways[0].base, None);
    assert_eq!(e.ways[0].nodes, n);
    assert!(
        e.modify_way(&osm_way(USER_ID_BASE + 99), tags(&[]), n, By::User)
            .is_err()
    );
}

#[test]
fn delete_user_way_drops_its_nodes() {
    let mut e = WayEdits::default();
    let n = ops_nodes(&mut e, 3);
    let a = e.add_way(tags(&[]), vec![n[0], n[1]], By::User).unwrap();
    let _b = e.add_way(tags(&[]), vec![n[1], n[2]], By::User).unwrap();
    e.delete_way(&osm_way(a.raw().unwrap()), By::User).unwrap();
    assert_eq!(e.ways.len(), 1);
    let ids: Vec<_> = e.nodes.iter().map(|x| x.id).collect();
    assert_eq!(ids, vec![n[1], n[2]]);
    e.validate().unwrap();
}

#[test]
fn delete_osm_way_replaces_modify_and_modify_replaces_delete() {
    let mut e = WayEdits::default();
    let orig = osm_way(8421);
    let nodes = vec![ElementId::Osm(10), ElementId::Osm(11)];
    e.modify_way(&orig, tags(&[]), nodes.clone(), By::User)
        .unwrap();
    let base = e.ways[0].base.clone();
    e.delete_way(&orig, By::User).unwrap();
    assert_eq!(e.ways.len(), 1);
    assert_eq!(e.ways[0].action, WayAction::Delete);
    assert_eq!(e.ways[0].base, base);
    assert_eq!(e.ways[0].nodes.len() + e.ways[0].tags.len(), 0);
    e.validate().unwrap();
    e.modify_way(&orig, tags(&[]), nodes, By::User).unwrap();
    assert_eq!(e.ways[0].action, WayAction::Modify);
    assert_eq!(e.ways[0].base, base);
    let mut fresh = WayEdits::default();
    fresh.delete_way(&orig, By::Ai).unwrap();
    assert_eq!(
        fresh.ways[0].base,
        Some(fingerprint(&orig.tags, &orig.nodes))
    );
    fresh.validate().unwrap();
}

#[test]
fn reset_returns_to_osm() {
    use naves_core::edits::apply_way_edits;
    use naves_core::osm::OsmWays;
    let orig = osm_way(8421);
    let ways = OsmWays {
        ways: vec![orig.clone()],
        skipped: 0,
    };
    let mut e = WayEdits::default();
    let u = e.new_node(47.05, 12.05, By::User).unwrap();
    e.modify_way(
        &orig,
        tags(&[("highway", "path")]),
        vec![ElementId::Osm(10), u],
        By::User,
    )
    .unwrap();
    e.reset_way(ElementId::Osm(8421));
    assert_eq!(e.ways.len(), 0);
    assert_eq!(e.nodes.len(), 0);
    let (out, _) = apply_way_edits(&ways, &e);
    assert_eq!(out.ways, vec![orig]);
    e.reset_way(ElementId::Osm(1)); // no edit: nothing happens
}

#[test]
fn reset_user_way_is_delete() {
    let mut e = WayEdits::default();
    let n = ops_nodes(&mut e, 2);
    let id = e.add_way(tags(&[]), n, By::User).unwrap();
    e.reset_way(id);
    assert_eq!(e.ways.len() + e.nodes.len(), 0);
}

#[test]
fn ids_are_never_reused() {
    let mut e = WayEdits::default();
    let n = ops_nodes(&mut e, 2);
    let w = e.add_way(tags(&[]), n, By::User).unwrap(); // u3
    assert_eq!(w, user(3));
    e.reset_way(w);
    assert_eq!(e.new_node(47.0, 12.0, By::User).unwrap(), user(4));
}

#[test]
fn move_node_rules() {
    let mut e = WayEdits::default();
    let u = e.new_node(47.0, 12.0, By::User).unwrap();
    e.move_node(u, 47.5, 12.5, None, By::Ai).unwrap();
    assert_eq!(
        (e.nodes[0].lat, e.nodes[0].lon, e.nodes[0].by),
        (47.5, 12.5, By::Ai)
    );
    assert_eq!(e.nodes[0].base, None);
    let osm = ElementId::Osm(812);
    assert!(e.move_node(osm, 1.0, 1.0, None, By::User).is_err());
    e.move_node(osm, 47.1, 12.1, Some((47.0, 12.0)), By::User)
        .unwrap();
    e.move_node(osm, 47.2, 12.2, Some((9.0, 9.0)), By::User)
        .unwrap();
    let n = e.nodes.iter().find(|n| n.id == osm).unwrap();
    assert_eq!((n.lat, n.base), (47.2, Some((47.0, 12.0))));
    e.move_node(osm, 47.3, 12.3, None, By::User).unwrap();
    assert!(e.move_node(user(77), 1.0, 1.0, None, By::User).is_err());
    assert!(e.move_node(osm, f64::NAN, 1.0, None, By::User).is_err());
    assert!(
        e.move_node(ElementId::Osm(0), 1.0, 1.0, Some((1.0, 1.0)), By::User)
            .is_err()
    );
    e.validate().unwrap();
}

#[test]
fn operations_refuse_bad_input_and_leave_the_set_unchanged() {
    let mut e = WayEdits::default();
    let n = ops_nodes(&mut e, 2);
    let before = e.clone();
    assert!(e.new_node(100.0, 0.0, By::User).is_err());
    assert!(e.add_way(tags(&[]), vec![n[0]], By::User).is_err());
    assert!(e.add_way(tags(&[]), vec![n[0], n[0]], By::User).is_err());
    assert!(
        e.add_way(tags(&[]), vec![n[0], ElementId::User(0)], By::User)
            .is_err()
    );
    let many = (1..=65)
        .map(|i| (format!("k{i}"), "v".to_string()))
        .collect();
    assert!(e.add_way(many, n.clone(), By::User).is_err());
    assert!(e.delete_way(&osm_way(0), By::User).is_err());
    assert!(e.delete_way(&osm_way(USER_ID_BASE + 50), By::User).is_err());
    assert_eq!(e, before);
    let mut full = WayEdits {
        next_id: u64::MAX >> 2,
        ..WayEdits::default()
    };
    full.next_id = (1 << 63) + 1;
    assert!(full.new_node(1.0, 1.0, By::User).is_err());
    assert!(full.add_way(tags(&[]), n, By::User).is_err());
    assert_eq!(full.next_id, (1 << 63) + 1);
}

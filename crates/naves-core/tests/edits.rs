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

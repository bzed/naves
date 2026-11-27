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

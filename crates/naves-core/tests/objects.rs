// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Object identity, overrides and the transform of a placed piece.

use std::str::FromStr;

use naves_core::objects::{
    GeneratedObject, ObjectId, Override, Overrides, PinnedEdit, PlacedObject, apply_overrides,
    bridge_object_height, piece_transform, road_objects,
};
use naves_core::roadfit::Pose;
use naves_core::roadplace::{PlacedKind, PlacedNetwork, PlacedRoad, Slot};

fn id(stretch: &str, slot: u32) -> ObjectId {
    ObjectId {
        feature: stretch.to_string(),
        preset: "road".to_string(),
        slot,
    }
}

fn generated(stretch: &str, slot: u32, model: &str, x: f64, y: f64) -> GeneratedObject {
    GeneratedObject {
        id: id(stretch, slot),
        model: model.to_string(),
        x,
        y,
        yaw: 0.5,
        scale: 1.0,
    }
}

fn edit(stretch: &str, slot: u32, model: &str, at: (f64, f64), what: Override) -> PinnedEdit {
    PinnedEdit {
        id: id(stretch, slot),
        generated_model: model.to_string(),
        generated_at: at,
        generated_yaw: 0.5,
        edit: what,
    }
}

#[test]
fn the_id_round_trips() {
    for stretch in ["123:1-5", "9:-4--2", "10:3-7#2", "5:2-3|7:3-9"] {
        let original = id(stretch, 17);
        let text = original.to_string();
        assert!(text.starts_with("road/"), "{text}");
        assert_eq!(ObjectId::from_str(&text).unwrap(), original, "{text}");
    }
    assert_eq!(id("123:1-5", 4).to_string(), "road/123:1-5/4");
    assert!(ObjectId::from_str("nonsense").is_err());
    assert!(ObjectId::from_str("road/x/notanumber").is_err());
}

#[test]
fn piece_transform_matches_the_sample() {
    let object = PlacedObject {
        id: None,
        model: "m".to_string(),
        x: 100.0,
        y: 200.0,
        yaw: 120.0_f64.to_radians(),
        scale: 1.0,
    };
    let t = piece_transform(&object, 42.5);
    let expected = [
        -0.5,
        0.0,
        -0.866_025_4,
        0.0,
        1.0,
        0.0,
        0.866_025_4,
        0.0,
        -0.5,
        100.0,
        42.5,
        200.0,
    ];
    for (a, b) in t.iter().zip(expected) {
        assert!((f64::from(*a) - b).abs() < 1e-5, "{t:?}");
    }
}

#[test]
fn a_north_facing_piece_has_the_identity_rotation() {
    let object = PlacedObject {
        id: None,
        model: "m".to_string(),
        x: 1.0,
        y: 2.0,
        yaw: 0.0,
        scale: 1.0,
    };
    let t = piece_transform(&object, 3.0);
    assert_eq!(
        t.map(f32::to_bits),
        [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 3.0, 2.0].map(f32::to_bits)
    );
}

#[test]
fn objects_without_edits_are_placed_as_generated() {
    let g = vec![
        generated("a", 0, "m1", 1.0, 2.0),
        generated("a", 1, "m2", 3.0, 4.0),
    ];
    let (placed, orphans) = apply_overrides(&g, &Overrides::default());
    assert_eq!(placed.len(), 2);
    assert_eq!(orphans.len(), 0);
    assert_eq!(placed[1].id, Some(id("a", 1)));
    assert_eq!((placed[1].x, placed[1].y), (3.0, 4.0));
}

#[test]
fn moved_edit_is_kept() {
    let g = vec![generated("a", 0, "m1", 10.0, 20.0)];
    let o = Overrides {
        edits: vec![edit(
            "a",
            0,
            "m1",
            (10.0, 20.0),
            Override::Moved {
                x: 12.0,
                y: 21.0,
                yaw: 1.5,
            },
        )],
    };
    let (placed, orphans) = apply_overrides(&g, &o);
    assert_eq!(orphans.len(), 0);
    assert_eq!(placed.len(), 1);
    assert_eq!((placed[0].x, placed[0].y, placed[0].yaw), (12.0, 21.0, 1.5));
    assert_eq!(placed[0].id, Some(id("a", 0)));
}

#[test]
fn deleted_edit_is_kept() {
    let g = vec![
        generated("a", 0, "m1", 10.0, 20.0),
        generated("a", 1, "m1", 30.0, 20.0),
    ];
    let o = Overrides {
        edits: vec![edit("a", 0, "m1", (10.0, 20.0), Override::Deleted)],
    };
    let (placed, orphans) = apply_overrides(&g, &o);
    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].id, Some(id("a", 1)));
    assert_eq!(orphans.len(), 0);
}

#[test]
fn replaced_edit_is_kept() {
    let g = vec![generated("a", 0, "m1", 10.0, 20.0)];
    let o = Overrides {
        edits: vec![edit(
            "a",
            0,
            "m1",
            (10.0, 20.0),
            Override::Replaced {
                model: "other".to_string(),
            },
        )],
    };
    let (placed, orphans) = apply_overrides(&g, &o);
    assert_eq!(orphans.len(), 0);
    assert_eq!(placed[0].model, "other");
    assert_eq!((placed[0].x, placed[0].y, placed[0].yaw), (10.0, 20.0, 0.5));
}

#[test]
fn regeneration_keeps_edits() {
    let first = vec![
        generated("a", 0, "m1", 10.0, 20.0),
        generated("a", 1, "m1", 35.0, 20.0),
    ];
    let edits = Overrides {
        edits: vec![edit(
            "a",
            1,
            "m1",
            (35.0, 20.0),
            Override::Moved {
                x: 36.0,
                y: 22.0,
                yaw: 0.1,
            },
        )],
    };
    let (a, _) = apply_overrides(&first, &edits);
    // generated again from the same input
    let (b, _) = apply_overrides(&first.clone(), &edits);
    assert_eq!(a, b);
    assert!(
        b.iter()
            .any(|p| p.id == Some(id("a", 1)) && p.x.to_bits() == 36.0_f64.to_bits())
    );
}

#[test]
fn object_gone_makes_an_orphan() {
    let g = vec![generated("a", 0, "m1", 10.0, 20.0)];
    let o = Overrides {
        edits: vec![edit(
            "a",
            7,
            "m1",
            (80.0, 90.0),
            Override::Moved {
                x: 81.0,
                y: 91.0,
                yaw: 0.3,
            },
        )],
    };
    let (placed, orphans) = apply_overrides(&g, &o);
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].id, id("a", 7));
    assert_eq!(orphans[0].at, (81.0, 91.0));
    // the edit is kept as an object of its own, and the generated one is untouched
    assert_eq!(placed.len(), 2);
    assert!(
        placed
            .iter()
            .any(|p| p.id.is_none() && p.model == "m1" && p.x.to_bits() == 81.0_f64.to_bits())
    );
}

#[test]
fn changed_model_makes_an_orphan() {
    let g = vec![generated("a", 0, "different", 10.0, 20.0)];
    let o = Overrides {
        edits: vec![edit(
            "a",
            0,
            "m1",
            (10.0, 20.0),
            Override::Moved {
                x: 12.0,
                y: 21.0,
                yaw: 0.0,
            },
        )],
    };
    let (placed, orphans) = apply_overrides(&g, &o);
    assert_eq!(orphans.len(), 1);
    // the generated object stays where it was generated
    assert!(placed.iter().any(|p| p.id == Some(id("a", 0))
        && p.model == "different"
        && p.x.to_bits() == 10.0_f64.to_bits()));
}

#[test]
fn moved_far_makes_an_orphan() {
    let g = vec![generated("a", 0, "m1", 200.0, 20.0)];
    let o = Overrides {
        edits: vec![edit(
            "a",
            0,
            "m1",
            (10.0, 20.0),
            Override::Moved {
                x: 12.0,
                y: 21.0,
                yaw: 0.0,
            },
        )],
    };
    let (_, orphans) = apply_overrides(&g, &o);
    assert_eq!(orphans.len(), 1);
    // within 25 m is still the same object
    let g = vec![generated("a", 0, "m1", 30.0, 20.0)];
    let (_, orphans) = apply_overrides(&g, &o);
    assert_eq!(orphans.len(), 0);
}

#[test]
fn orphan_stays_exactly_where_it_was() {
    let o = Overrides {
        edits: vec![edit(
            "gone",
            3,
            "m1",
            (1.0, 2.0),
            Override::Moved {
                x: 5.5,
                y: 6.5,
                yaw: 2.25,
            },
        )],
    };
    let (placed, _) = apply_overrides(&[], &o);
    assert_eq!(placed.len(), 1);
    assert_eq!((placed[0].x, placed[0].y, placed[0].yaw), (5.5, 6.5, 2.25));
    assert_eq!(placed[0].id, None);
}

#[test]
fn a_deleted_edit_with_no_object_is_dropped_silently() {
    let o = Overrides {
        edits: vec![edit("gone", 3, "m1", (1.0, 2.0), Override::Deleted)],
    };
    let (placed, orphans) = apply_overrides(&[], &o);
    assert_eq!(placed.len(), 0);
    assert_eq!(orphans.len(), 0);
}

#[test]
fn overrides_file_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let o = Overrides {
        edits: vec![
            edit(
                "123:1-5",
                4,
                "dz\\a b.p3d",
                (10.5, 20.25),
                Override::Moved {
                    x: 1.0,
                    y: 2.0,
                    yaw: 3.0,
                },
            ),
            edit("5:2-3|7:3-9", 0, "j", (0.0, 0.0), Override::Deleted),
            edit(
                "9:-4--2",
                2,
                "k",
                (7.0, 8.0),
                Override::Replaced {
                    model: "other".to_string(),
                },
            ),
        ],
    };
    o.save(dir.path()).unwrap();
    assert!(dir.path().join("edits/overrides.toml").is_file());
    assert_eq!(Overrides::load(dir.path()).unwrap(), o);
}

#[test]
fn missing_overrides_file_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(Overrides::load(dir.path()).unwrap(), Overrides::default());
}

#[test]
fn damaged_overrides_are_errors() {
    for text in [
        "not toml at all [[[",
        "[[edits]]\nid = 5\n",
        "[[edits]]\nid = \"nonsense\"\ngenerated_model = \"m\"\ngenerated_at = [1.0, 2.0]\nedit = { kind = \"moved\" }\n",
    ] {
        assert!(Overrides::parse(text).is_err(), "{text}");
    }
    assert_eq!(Overrides::parse("").unwrap().edits.len(), 0);
}

fn bridge_road(index: u32) -> PlacedRoad {
    PlacedRoad {
        slot: Slot {
            stretch: "7:1-2".to_string(),
            index,
        },
        kind: PlacedKind::Bridge,
        family: naves_core::catalog::Family::Asf3,
        drivable: true,
        model: "bridge".to_string(),
        origin: Pose {
            x: 10.0,
            y: 20.0,
            heading: 0.75,
        },
        start: Pose {
            x: 10.0,
            y: 20.0,
            heading: 0.75,
        },
        end: Pose {
            x: 11.0,
            y: 21.0,
            heading: 0.75,
        },
        width: 7.0,
        scale: 1.0,
    }
}

#[test]
fn roads_become_objects_with_their_slot_as_identity() {
    let net = PlacedNetwork {
        roads: vec![bridge_road(0), bridge_road(1)],
        ..PlacedNetwork::default()
    };
    let objects = road_objects(&net);
    assert_eq!(objects.len(), 2);
    assert_eq!(objects[1].id.to_string(), "road/7:1-2/1");
    assert_eq!(
        (objects[0].x, objects[0].y, objects[0].yaw),
        (10.0, 20.0, 0.75)
    );
}

#[test]
fn a_bridge_sits_with_its_deck_at_the_profile_height() {
    use naves_core::catalog::testing::synthetic_catalog;
    let catalog = synthetic_catalog();
    let stone = catalog
        .bridges()
        .iter()
        .find(|b| b.model.contains("stone"))
        .unwrap();
    // the deck is 7.9 m above the model's origin: the origin goes that far below the road
    assert!((bridge_object_height(120.0, stone, 1.0) - (120.0 - stone.deck_height)).abs() < 1e-12);
}

const EDIT_HEAD: &str = "[[edits]]\nid = \"road/123:1-5/4\"\ngenerated_model = \"m\"\n";

#[test]
fn overrides_with_numbers_that_are_not_finite_are_rejected_by_name() {
    for (what, field) in [
        (
            "generated_at = [nan, 2.0]\n[edits.edit]\nkind = \"deleted\"\n",
            "generated_at",
        ),
        (
            "generated_at = [1.0, inf]\n[edits.edit]\nkind = \"deleted\"\n",
            "generated_at",
        ),
        (
            "generated_at = [1.0, 2.0]\ngenerated_yaw = nan\n[edits.edit]\nkind = \"deleted\"\n",
            "generated_yaw",
        ),
        (
            "generated_at = [1.0, 2.0]\n[edits.edit]\nkind = \"moved\"\nx = nan\ny = 1.0\nyaw = 0.0\n",
            "x",
        ),
        (
            "generated_at = [1.0, 2.0]\n[edits.edit]\nkind = \"moved\"\nx = 1.0\ny = -inf\nyaw = 0.0\n",
            "y",
        ),
        (
            "generated_at = [1.0, 2.0]\n[edits.edit]\nkind = \"moved\"\nx = 1.0\ny = 1.0\nyaw = inf\n",
            "yaw",
        ),
    ] {
        let text = format!("{EDIT_HEAD}{what}");
        let error = Overrides::parse(&text).unwrap_err().to_string();
        assert!(error.contains("road/123:1-5/4"), "{error}");
        assert!(error.contains(field), "{field}: {error}");
    }
}

#[test]
fn overrides_with_an_unusable_model_are_rejected_by_name() {
    for model in ["", "dz/stra\u{df}e.p3d"] {
        let text = format!(
            "{EDIT_HEAD}generated_at = [1.0, 2.0]\n[edits.edit]\nkind = \"replaced\"\nmodel = \"{model}\"\n"
        );
        let error = Overrides::parse(&text).unwrap_err().to_string();
        assert!(error.contains("road/123:1-5/4"), "{error}");
        assert!(error.contains("model"), "{error}");
    }
}

#[test]
fn an_orphaned_replacement_keeps_the_yaw_of_its_object() {
    let o = Overrides {
        edits: vec![PinnedEdit {
            generated_yaw: 1.25,
            ..edit(
                "1:1-2",
                0,
                "m",
                (10.0, 20.0),
                Override::Replaced {
                    model: "other".to_string(),
                },
            )
        }],
    };
    let (placed, orphans) = apply_overrides(&[], &o);
    assert_eq!(orphans.len(), 1);
    assert_eq!(placed.len(), 1);
    assert!((placed[0].yaw - 1.25).abs() < 1e-12);
    assert!((placed[0].x - 10.0).abs() < 1e-12);
}

#[test]
fn an_overrides_file_without_a_generated_yaw_still_loads() {
    let text = format!(
        "{EDIT_HEAD}generated_at = [1.0, 2.0]\n[edits.edit]\nkind = \"replaced\"\nmodel = \"other\"\n"
    );
    let o = Overrides::parse(&text).unwrap();
    assert!(o.edits[0].generated_yaw.abs() < 1e-12);
}

#[test]
fn the_transform_carries_the_scale() {
    let o = PlacedObject {
        id: None,
        model: "b".into(),
        x: 10.0,
        y: 20.0,
        yaw: 0.0,
        scale: 1.25,
    };
    let t = piece_transform(&o, 5.0);
    assert_eq!(
        [t[0], t[4], t[8]].map(f32::to_bits),
        [1.25_f32; 3].map(f32::to_bits)
    );
    assert_eq!(
        [t[9], t[10], t[11]].map(f32::to_bits),
        [10.0_f32, 5.0, 20.0].map(f32::to_bits)
    );
}

#[test]
fn a_scaled_bridge_stands_lower_by_its_scaled_deck_height() {
    let catalog = naves_core::catalog::testing::synthetic_catalog();
    let stone = catalog
        .bridges()
        .iter()
        .find(|b| b.model.contains("bridge_stone_25"))
        .unwrap();
    assert!((stone.deck_height - 7.9).abs() < 1e-4);
    assert!(
        (bridge_object_height(100.0, stone, 1.25) - (100.0 - 1.25 * stone.deck_height)).abs()
            < 1e-9
    );
}

#[test]
fn a_moved_bridge_keeps_its_scale_and_a_replaced_or_orphaned_one_loses_it() {
    let mut bridge = generated("7:1-2", 0, "bridge", 10.0, 20.0);
    bridge.scale = 1.3;
    let moved = Overrides {
        edits: vec![edit(
            "7:1-2",
            0,
            "bridge",
            (10.0, 20.0),
            Override::Moved {
                x: 12.0,
                y: 22.0,
                yaw: 0.1,
            },
        )],
    };
    let (placed, orphans) = apply_overrides(std::slice::from_ref(&bridge), &moved);
    assert_eq!(orphans.len(), 0);
    assert!((placed[0].scale - 1.3).abs() < 1e-12);
    assert!((placed[0].x - 12.0).abs() < 1e-12);

    let replaced = Overrides {
        edits: vec![edit(
            "7:1-2",
            0,
            "bridge",
            (10.0, 20.0),
            Override::Replaced {
                model: "other".into(),
            },
        )],
    };
    let (placed, _) = apply_overrides(std::slice::from_ref(&bridge), &replaced);
    assert!((placed[0].scale - 1.0).abs() < 1e-12);

    // the bridge is gone: the edit lives on as an object of its own, at scale 1
    let (placed, orphans) = apply_overrides(&[], &moved);
    assert_eq!(orphans.len(), 1);
    assert!((placed[0].scale - 1.0).abs() < 1e-12);
}

#[test]
fn a_road_object_takes_the_scale_of_its_road() {
    let mut road = bridge_road(0);
    road.scale = 1.2;
    let net = PlacedNetwork {
        roads: vec![road],
        ..PlacedNetwork::default()
    };
    assert!((road_objects(&net)[0].scale - 1.2).abs() < 1e-12);
}

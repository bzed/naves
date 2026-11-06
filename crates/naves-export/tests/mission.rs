// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Assembling the server mission from the installed `dayzOffline.enoch`.

#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]

use std::path::{Path, PathBuf};

use naves_export::mission::{MissionError, write_mission};

const SPAWNS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?>
<playerspawnpoints>
    <fresh>
        <spawn_params><min_dist_infected>30</min_dist_infected></spawn_params>
        <generator_params><grid_density>4</grid_density></generator_params>
        <generator_posbubbles>
            <group name="A">
                <pos x="1136.6" z="8039.5" />
                <pos x="1155.7" z="7771.2" />
            </group>
        </generator_posbubbles>
    </fresh>
    <hop>
        <generator_posbubbles>
            <group name="B"><pos x="10" z="20" /></group>
        </generator_posbubbles>
    </hop>
    <travel>
        <generator_posbubbles>
            <group name="C"><pos x="30" z="40" /></group>
        </generator_posbubbles>
    </travel>
</playerspawnpoints>
"#;

const GROUPS: &str = r#"<map>
    <group name="Land_A" pos="1 2 3" rpy="0 0 0" a="0" />
    <group name="Land_B" pos="4 5 6" rpy="0 0 0" a="0" />
</map>
"#;

const EVENTS: &str = r#"<eventposdef>
    <event name="StaticHeliCrash">
        <pos x="100" z="200" a="0" />
        <pos x="300" z="400" a="0" />
    </event>
    <event name="Other"><zone smin="0" smax="0" dmin="1" dmax="2" r="50" /></event>
</eventposdef>
"#;

/// A minimal `mpmissions` folder with `dayzOffline.enoch`.
fn fake_install(root: &Path) -> PathBuf {
    let mpmissions = root.join("mpmissions");
    let enoch = mpmissions.join("dayzOffline.enoch");
    std::fs::create_dir_all(enoch.join("db")).unwrap();
    std::fs::write(enoch.join("cfgplayerspawnpoints.xml"), SPAWNS).unwrap();
    std::fs::write(enoch.join("mapgrouppos.xml"), GROUPS).unwrap();
    std::fs::write(enoch.join("cfgeventspawns.xml"), EVENTS).unwrap();
    std::fs::write(enoch.join("init.c"), "void main() {}\n").unwrap();
    std::fs::write(
        enoch.join("cfgeffectarea.json"),
        r#"{"Areas":[{"AreaName":"Radunin","Data":{"Pos":[7500,0,6156]}}],"SafePositions":[[1,2]]}"#,
    )
    .unwrap();
    std::fs::write(enoch.join("db/types.xml"), "<types/>\n").unwrap();
    mpmissions
}

#[test]
fn mission_spawn_point() {
    let dir = tempfile::tempdir().unwrap();
    let src = fake_install(dir.path());
    let out = dir.path().join("out");
    let mission = write_mission(&src, &out, "x", (3200.0, 3200.0)).unwrap();
    assert_eq!(mission, out.join("dayzOffline.x"));
    let spawns = std::fs::read_to_string(mission.join("cfgplayerspawnpoints.xml")).unwrap();
    for section in ["fresh", "hop", "travel"] {
        let start = spawns.find(&format!("<{section}>")).unwrap();
        let end = spawns.find(&format!("</{section}>")).unwrap();
        let body = &spawns[start..end];
        assert_eq!(body.matches("<pos ").count(), 1, "{section}: {body}");
        assert!(
            body.contains("<group name=\"naves\">"),
            "points live in a group: {section}: {body}"
        );
        assert!(
            body.contains(r#"<pos x="3200" z="3200" />"#),
            "{section}: {body}"
        );
    }
    assert!(
        spawns.contains("<grid_density>4</grid_density>"),
        "other settings are kept"
    );
    assert!(!spawns.contains("1136.6"));
}

#[test]
fn mission_keeps_the_rest_and_empties_object_positions() {
    let dir = tempfile::tempdir().unwrap();
    let src = fake_install(dir.path());
    let mission = write_mission(&src, &dir.path().join("out"), "x", (1.5, 2.5)).unwrap();
    assert_eq!(
        std::fs::read_to_string(mission.join("init.c")).unwrap(),
        "void main() {}\n"
    );
    assert_eq!(
        std::fs::read_to_string(mission.join("db/types.xml")).unwrap(),
        "<types/>\n"
    );
    let groups = std::fs::read_to_string(mission.join("mapgrouppos.xml")).unwrap();
    assert!(
        groups.contains("<map>") && !groups.contains("<group"),
        "{groups}"
    );
    let events = std::fs::read_to_string(mission.join("cfgeventspawns.xml")).unwrap();
    assert!(
        events.contains("StaticHeliCrash") && events.contains("Other"),
        "{events}"
    );
    assert!(!events.contains("<pos "), "{events}");
    assert!(events.contains("<zone "), "other event content stays");
    let spawns = std::fs::read_to_string(mission.join("cfgplayerspawnpoints.xml")).unwrap();
    assert!(spawns.contains(r#"<pos x="1.5" z="2.5" />"#));
}

#[test]
fn mission_is_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let src = fake_install(dir.path());
    let a = write_mission(&src, &dir.path().join("a"), "x", (1.0, 2.0)).unwrap();
    let b = write_mission(&src, &dir.path().join("b"), "x", (1.0, 2.0)).unwrap();
    for name in [
        "cfgplayerspawnpoints.xml",
        "mapgrouppos.xml",
        "cfgeventspawns.xml",
        "init.c",
    ] {
        assert_eq!(
            std::fs::read(a.join(name)).unwrap(),
            std::fs::read(b.join(name)).unwrap()
        );
    }
}

#[cfg(unix)]
#[test]
fn mission_never_writes_source() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let src = fake_install(dir.path());
    let enoch = src.join("dayzOffline.enoch");
    let before = std::fs::read(enoch.join("cfgplayerspawnpoints.xml")).unwrap();
    // read-only files and directories, like an installed game
    for entry in [
        enoch.join("db/types.xml"),
        enoch.join("cfgplayerspawnpoints.xml"),
    ] {
        std::fs::set_permissions(entry, std::fs::Permissions::from_mode(0o444)).unwrap();
    }
    std::fs::set_permissions(enoch.join("db"), std::fs::Permissions::from_mode(0o555)).unwrap();
    std::fs::set_permissions(&enoch, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = write_mission(&src, &dir.path().join("out"), "x", (3200.0, 3200.0));
    std::fs::set_permissions(&enoch, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(enoch.join("db"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let mission = result.unwrap();
    assert_eq!(
        std::fs::read(enoch.join("cfgplayerspawnpoints.xml")).unwrap(),
        before
    );
    let copy = mission.join("cfgplayerspawnpoints.xml");
    assert!(
        std::fs::metadata(copy).unwrap().permissions().mode() & 0o200 != 0,
        "the copy is writable"
    );
}

#[test]
fn missing_source_mission_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let err = write_mission(dir.path(), &dir.path().join("out"), "x", (0.0, 0.0)).unwrap_err();
    assert!(matches!(err, MissionError::SourceMissing(_)), "{err}");
    assert!(err.to_string().contains("dayzOffline.enoch"), "{err}");
}

#[test]
fn existing_target_is_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let src = fake_install(dir.path());
    let out = dir.path().join("out");
    write_mission(&src, &out, "x", (0.0, 0.0)).unwrap();
    let err = write_mission(&src, &out, "x", (0.0, 0.0)).unwrap_err();
    assert!(matches!(err, MissionError::Exists(_)), "{err}");
}

#[test]
fn bad_world_name_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let src = fake_install(dir.path());
    for bad in ["", "../x", "a b", "Ä"] {
        let err = write_mission(&src, &dir.path().join("out"), bad, (0.0, 0.0)).unwrap_err();
        assert!(matches!(err, MissionError::BadWorld(_)), "{bad}: {err}");
    }
}

#[test]
fn non_finite_spawn_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let src = fake_install(dir.path());
    let err = write_mission(&src, &dir.path().join("out"), "x", (f32::NAN, 1.0)).unwrap_err();
    assert!(matches!(err, MissionError::BadSpawn), "{err}");
}

#[test]
fn malformed_xml_is_an_error_naming_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let src = fake_install(dir.path());
    std::fs::write(
        src.join("dayzOffline.enoch/mapgrouppos.xml"),
        "<map><group></map>",
    )
    .unwrap();
    let err = write_mission(&src, &dir.path().join("out"), "x", (0.0, 0.0)).unwrap_err();
    assert!(err.to_string().contains("mapgrouppos.xml"), "{err}");
}

/// Reads every event of `xml`; a mismatched or unclosed tag is an error.
fn assert_well_formed(name: &str, xml: &str) {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut failure = None;
    while failure.is_none() {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(_) => {}
            Err(e) => failure = Some(e.to_string()),
        }
    }
    assert!(
        failure.is_none(),
        "{name} is not well-formed: {failure:?}\n{xml}"
    );
}

#[test]
fn edited_files_stay_well_formed() {
    let dir = tempfile::tempdir().unwrap();
    let src = fake_install(dir.path());
    let mission = write_mission(&src, &dir.path().join("out"), "x", (3200.0, 3200.0)).unwrap();
    for name in [
        "cfgplayerspawnpoints.xml",
        "mapgrouppos.xml",
        "cfgeventspawns.xml",
    ] {
        let xml = std::fs::read_to_string(mission.join(name)).unwrap();
        assert_well_formed(name, &xml);
    }
    let spawns = std::fs::read_to_string(mission.join("cfgplayerspawnpoints.xml")).unwrap();
    assert_eq!(
        spawns.matches("</generator_posbubbles>").count(),
        3,
        "{spawns}"
    );
    assert!(
        spawns.trim_end().ends_with("</playerspawnpoints>"),
        "{spawns}"
    );
}

/// Gas zones at Livonia's coordinates would be at arbitrary places on another map.
#[test]
fn mission_has_no_gas_zones_of_the_source_map() {
    let dir = tempfile::tempdir().unwrap();
    let src = fake_install(dir.path());
    let mission = write_mission(&src, &dir.path().join("out"), "x", (3200.0, 3200.0)).unwrap();
    let areas = std::fs::read_to_string(mission.join("cfgeffectarea.json")).unwrap();
    assert!(
        !areas.contains("Radunin") && !areas.contains("[1,2]"),
        "{areas}"
    );
    assert!(
        areas.contains("\"Areas\"") && areas.contains("\"SafePositions\""),
        "{areas}"
    );
}

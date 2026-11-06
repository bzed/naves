// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where the tools come from: the environment, then the settings, then the search.

#![cfg(unix)]

use std::ffi::OsString;
use std::path::PathBuf;

use naves_core::prefs::ToolPaths;
use naves_export::testing::FakeEnv;
use naves_export::tools::{Origin, ToolError, locate_preferring_with, tool_origins_with};

fn env_of(pairs: Vec<(&'static str, PathBuf)>) -> impl Fn(&str) -> Option<OsString> {
    move |key| {
        pairs
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone().into_os_string())
    }
}

fn settings_of(fakes: &FakeEnv) -> ToolPaths {
    ToolPaths {
        armake2: Some(fakes.armake2.clone()),
        steam_root: Some(fakes.steam.clone()),
        dayz_server: Some(fakes.server.clone()),
        ..ToolPaths::default()
    }
}

#[test]
fn the_settings_beat_the_search() {
    let root = tempfile::tempdir().unwrap();
    let fakes = FakeEnv::new(root.path()).unwrap();
    // nothing in the environment, nothing on PATH: only the settings know where everything is
    let tools = locate_preferring_with(&settings_of(&fakes), &env_of(vec![]), None, None).unwrap();
    assert_eq!(tools.armake2, fakes.armake2);
    assert_eq!(tools.dayz_server, fakes.server);
}

#[test]
fn the_environment_beats_the_settings() {
    let root = tempfile::tempdir().unwrap();
    let fakes = FakeEnv::new(root.path()).unwrap();
    let other = tempfile::tempdir().unwrap();
    let second = FakeEnv::new(other.path()).unwrap();
    let env = env_of(vec![("ARMAKE2", second.armake2.clone())]);
    let tools = locate_preferring_with(&settings_of(&fakes), &env, None, None).unwrap();
    assert_eq!(
        tools.armake2, second.armake2,
        "the variable wins for its tool"
    );
    assert_eq!(
        tools.dayz_server, fakes.server,
        "the settings still serve the rest"
    );
}

#[test]
fn a_path_in_the_settings_that_does_not_exist_names_the_setting() {
    let root = tempfile::tempdir().unwrap();
    let fakes = FakeEnv::new(root.path()).unwrap();
    let mut paths = settings_of(&fakes);
    paths.armake2 = Some(root.path().join("gone/armake2"));
    let err = locate_preferring_with(&paths, &env_of(vec![]), None, None).unwrap_err();
    assert!(matches!(err, ToolError::BadSetting { .. }), "{err}");
    let text = err.to_string();
    assert!(
        text.contains("armake2") && text.contains("gone/armake2") && text.contains("settings"),
        "{text}"
    );
}

#[test]
fn without_a_setting_a_missing_tool_still_names_the_variable() {
    let err =
        locate_preferring_with(&ToolPaths::default(), &env_of(vec![]), None, None).unwrap_err();
    assert!(err.to_string().contains("ARMAKE2"), "{err}");
}

#[test]
fn origins_say_where_each_path_came_from() {
    let root = tempfile::tempdir().unwrap();
    let fakes = FakeEnv::new(root.path()).unwrap();
    let common = fakes.steam.join("steamapps/common");
    let paths = ToolPaths {
        steam_root: Some(fakes.steam.clone()),
        dayz_tools: Some(common.join("DayZ Tools")),
        ..ToolPaths::default()
    };
    let env = env_of(vec![
        ("ARMAKE2", fakes.armake2.clone()),
        ("DAYZ_SERVER", fakes.server.clone()),
    ]);
    let rows = tool_origins_with(&paths, &env, None, None);
    let from = |tool: &str| rows.iter().find(|r| r.tool == tool).unwrap().from;
    assert_eq!(from("armake2"), Origin::Environment);
    assert_eq!(from("DayZ Server"), Origin::Environment);
    assert_eq!(from("Steam"), Origin::Settings);
    assert_eq!(from("DayZ Tools"), Origin::Settings);
    // found under the Steam folder the settings name
    assert_eq!(from("Proton"), Origin::Found);
    assert_eq!(from("DayZ"), Origin::Found);
    let proton = rows.iter().find(|r| r.tool == "Proton").unwrap();
    assert_eq!(
        proton.path.as_deref(),
        Some(common.join("Proton Hotfix/proton").as_path())
    );
    assert_eq!(rows.len(), 6);
}

#[test]
fn a_tool_that_is_nowhere_is_missing() {
    let rows = tool_origins_with(&ToolPaths::default(), &env_of(vec![]), None, None);
    assert!(
        rows.iter()
            .all(|r| r.from == Origin::Missing && r.path.is_none()),
        "{rows:?}"
    );
}

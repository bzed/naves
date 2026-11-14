// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Proton binarize bridge, tested against fake `dayzmake` and `proton` scripts.

#![cfg(unix)]
#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use naves_export::tools::{BinarizeBridge, ToolError, Tools};
use naves_formats::wrp8::EditableWorld;

/// Serialises the tests of this file. Writing a script and executing it while another thread forks
/// fails with "Text file busy" (the child inherits the open write handle until it execs).
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn script(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A fake Steam root with DayZ Tools and a Proton whose `proton` script runs `proton_body`.
fn fake_steam(root: &Path, proton_body: &str) -> PathBuf {
    let steam = root.join("steam");
    let common = steam.join("steamapps/common");
    std::fs::create_dir_all(common.join("DayZ Tools/Bin/Binarize")).unwrap();
    std::fs::write(common.join("DayZ Tools/Bin/Binarize/binarize.exe"), "MZ").unwrap();
    std::fs::create_dir_all(common.join("DayZ")).unwrap();
    std::fs::create_dir_all(common.join("DayZServer/mpmissions")).unwrap();
    script(&common.join("Proton Hotfix/proton"), proton_body);
    script(&common.join("Proton Hotfix/files/bin/wineserver"), "exit 0");
    steam
}

fn fake_dayzmake(root: &Path) -> PathBuf {
    let p = root.join("bin/dayzmake");
    script(&p, "echo v2.0.0");
    p
}

fn env_of(pairs: Vec<(&'static str, PathBuf)>) -> impl Fn(&str) -> Option<OsString> {
    move |key| {
        pairs
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone().into_os_string())
    }
}

/// A proton stub: creates the prefix on `cmd.exe /c exit`, otherwise runs `on_binarize` in the sandbox.
fn proton_stub(on_binarize: &str) -> String {
    format!(
        r#"if [ "$4" = "exit" ]; then mkdir -p "$STEAM_COMPAT_DATA_PATH/pfx/dosdevices"; exit 0; fi
sandbox="$(dirname "$STEAM_COMPAT_DATA_PATH")"
dev="$STEAM_COMPAT_DATA_PATH/pfx/dosdevices"
[ -L "$dev/z:" ] && [ "$(readlink "$dev/z:")" = "$sandbox" ] || {{ echo "z: not confined" >&2; exit 9; }}
[ -L "$dev/p:" ] && [ "$(readlink "$dev/p:")" = "$(pwd -P)" ] || {{ echo "p: is not the cwd" >&2; exit 9; }}
[ -f "$sandbox/Binarize/binarize.exe" ] || {{ echo "no Binarize copy" >&2; exit 9; }}
case "$4" in *'P:\naves\worlds\w'*) ;; *) echo "bad command: $4" >&2; exit 9;; esac
{on_binarize}"#
    )
}

fn bridge(steam: &Path, base: &Path, timeout: Duration) -> BinarizeBridge {
    BinarizeBridge {
        steam_root: steam.to_path_buf(),
        dayz_tools: steam.join("steamapps/common/DayZ Tools"),
        dayz: steam.join("steamapps/common/DayZ"),
        proton: steam.join("steamapps/common/Proton Hotfix/proton"),
        sandbox_base: base.to_path_buf(),
        timeout,
    }
}

fn staging(root: &Path) -> PathBuf {
    let p = root.join("staging");
    std::fs::create_dir_all(p.join("naves/worlds/w")).unwrap();
    std::fs::write(p.join("naves/worlds/w/w.wrp"), "8WVR").unwrap();
    p
}

fn leftovers(base: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(base)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect()
}

#[test]
fn locates_everything_from_the_environment() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let steam = fake_steam(dir.path(), "exit 0");
    let dayzmake = fake_dayzmake(dir.path());
    let env = env_of(vec![
        ("DAYZMAKE", dayzmake.clone()),
        ("STEAM_ROOT", steam.clone()),
    ]);
    let tools = Tools::locate_with(&env, None, None).unwrap();
    assert_eq!(tools.dayzmake, dayzmake);
    assert_eq!(
        tools.binarize.dayz_tools,
        steam.join("steamapps/common/DayZ Tools")
    );
    assert_eq!(
        tools.binarize.proton,
        steam.join("steamapps/common/Proton Hotfix/proton")
    );
    assert_eq!(tools.dayz_server, steam.join("steamapps/common/DayZServer"));
}

#[test]
fn dayz_server_variable_wins_and_missing_server_is_named() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let steam = fake_steam(dir.path(), "exit 0");
    let dayzmake = fake_dayzmake(dir.path());
    let server = dir.path().join("my server");
    std::fs::create_dir_all(server.join("mpmissions")).unwrap();
    let env = env_of(vec![
        ("DAYZMAKE", dayzmake.clone()),
        ("STEAM_ROOT", steam.clone()),
        ("DAYZ_SERVER", server.clone()),
    ]);
    assert_eq!(
        Tools::locate_with(&env, None, None).unwrap().dayz_server,
        server
    );
    std::fs::remove_dir_all(steam.join("steamapps/common/DayZServer")).unwrap();
    let env = env_of(vec![("DAYZMAKE", dayzmake), ("STEAM_ROOT", steam)]);
    match Tools::locate_with(&env, None, None) {
        Err(ToolError::Missing { tool, env, .. }) => {
            assert_eq!((tool, env), ("DayZ Server", "DAYZ_SERVER"));
        }
        other => panic!("expected Missing DayZ Server, got {other:?}"),
    }
}

#[test]
fn explicit_variables_win() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let steam = fake_steam(dir.path(), "exit 0");
    let other_tools = dir.path().join("my tools");
    std::fs::create_dir_all(other_tools.join("Bin/Binarize")).unwrap();
    let env = env_of(vec![
        ("DAYZMAKE", fake_dayzmake(dir.path())),
        ("STEAM_ROOT", steam),
        ("DAYZ_TOOLS", other_tools.clone()),
    ]);
    assert_eq!(
        Tools::locate_with(&env, None, None)
            .unwrap()
            .binarize
            .dayz_tools,
        other_tools
    );
}

#[test]
fn missing_dayzmake_is_named() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let env = env_of(vec![]);
    match Tools::locate_with(&env, Some(empty.into_os_string()), None) {
        Err(ToolError::Missing { tool, env, .. }) => {
            assert_eq!((tool, env), ("dayzmake", "DAYZMAKE"));
        }
        other => panic!("expected Missing dayzmake, got {other:?}"),
    }
}

#[test]
fn dayzmake_is_found_on_the_path() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let steam = fake_steam(dir.path(), "exit 0");
    let dayzmake = fake_dayzmake(dir.path());
    let env = env_of(vec![("STEAM_ROOT", steam)]);
    let path = dayzmake.parent().unwrap().as_os_str().to_os_string();
    assert_eq!(
        Tools::locate_with(&env, Some(path), None).unwrap().dayzmake,
        dayzmake
    );
}

/// Locates with a `dayzmake` whose `--version` runs `body`.
fn locate_with_version_script(body: &str) -> Result<Tools, ToolError> {
    let dir = tempfile::tempdir().unwrap();
    let steam = fake_steam(dir.path(), "exit 0");
    let tool = dir.path().join("bin/dayzmake");
    script(&tool, body);
    let env = env_of(vec![("DAYZMAKE", tool), ("STEAM_ROOT", steam)]);
    Tools::locate_with(&env, None, None)
}

#[test]
fn an_old_dayzmake_is_rejected_with_version_minimum_and_url() {
    let _serial = serial();
    for body in ["echo v1.9.9", "echo 'dayzmake 1.0.0'", "echo v0.3.7 >&2"] {
        let err = locate_with_version_script(body).unwrap_err();
        assert!(matches!(err, ToolError::DayzmakeTooOld { .. }), "{err}");
        let text = err.to_string();
        assert!(
            text.contains("v2.0.0") && text.contains("github.com/bzed/dayzmake"),
            "{text}"
        );
    }
    let err = locate_with_version_script("echo v1.9.9").unwrap_err();
    assert!(err.to_string().contains("v1.9.9"), "{err}");
}

#[test]
fn newer_or_decorated_versions_are_accepted() {
    let _serial = serial();
    for body in [
        "echo v2.0.0",
        "echo 2.0.0",
        "echo v2.1.0-rc1",
        "echo 'dayzmake version v10.0.1 (abc)'",
        "echo v3.0.0 >&2",
    ] {
        assert!(locate_with_version_script(body).is_ok(), "{body}");
    }
}

#[test]
fn unparseable_version_output_is_an_error_not_a_panic() {
    let _serial = serial();
    for body in [
        "echo",
        "echo hello",
        "echo v2.0",
        "echo 'v2..0.1'",
        "exit 3",
    ] {
        let err = locate_with_version_script(body).unwrap_err();
        assert!(
            matches!(err, ToolError::DayzmakeVersion { .. }),
            "{body}: {err}"
        );
        assert!(err.to_string().contains("v2.0.0"), "{err}");
    }
}

#[test]
fn the_old_armake2_variable_still_finds_the_tool_and_the_new_one_wins() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let steam = fake_steam(dir.path(), "exit 0");
    let tool = fake_dayzmake(dir.path());
    let old = dir.path().join("old/dayzmake");
    script(&old, "echo v1.0.0");
    let env = env_of(vec![
        ("ARMAKE2", tool.clone()),
        ("STEAM_ROOT", steam.clone()),
    ]);
    assert_eq!(Tools::locate_with(&env, None, None).unwrap().dayzmake, tool);
    let env = env_of(vec![
        ("ARMAKE2", old),
        ("DAYZMAKE", tool.clone()),
        ("STEAM_ROOT", steam),
    ]);
    assert_eq!(Tools::locate_with(&env, None, None).unwrap().dayzmake, tool);
}

#[test]
fn missing_dayz_tools_is_named() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let empty_steam = dir.path().join("empty-steam");
    std::fs::create_dir_all(&empty_steam).unwrap();
    let env = env_of(vec![
        ("DAYZMAKE", fake_dayzmake(dir.path())),
        ("STEAM_ROOT", empty_steam),
    ]);
    match Tools::locate_with(&env, None, None) {
        Err(ToolError::Missing { tool, env, .. }) => {
            assert_eq!((tool, env), ("DayZ Tools", "DAYZ_TOOLS"));
        }
        other => panic!("expected Missing DayZ Tools, got {other:?}"),
    }
}

#[test]
fn missing_proton_is_named() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let steam = fake_steam(dir.path(), "exit 0");
    std::fs::remove_dir_all(steam.join("steamapps/common/Proton Hotfix")).unwrap();
    let env = env_of(vec![
        ("DAYZMAKE", fake_dayzmake(dir.path())),
        ("STEAM_ROOT", steam),
    ]);
    match Tools::locate_with(&env, None, None) {
        Err(ToolError::Missing { tool, env, .. }) => assert_eq!((tool, env), ("Proton", "PROTON")),
        other => panic!("expected Missing Proton, got {other:?}"),
    }
}

#[test]
fn missing_steam_is_named() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let env = env_of(vec![("DAYZMAKE", fake_dayzmake(dir.path()))]);
    // an empty home folder has no Steam in it
    match Tools::locate_with(&env, None, Some(dir.path().join("home"))) {
        Err(ToolError::Missing { tool, env, .. }) => {
            assert_eq!((tool, env), ("Steam", "STEAM_ROOT"));
        }
        other => panic!("expected Missing Steam, got {other:?}"),
    }
}

#[test]
fn binarizes_with_fake_proton() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let ok = r#"printf 'OPRW\040\000\000\000' > "$sandbox/out/w.wrp"; echo "fake log" > "$sandbox/binarize.log"; exit 0"#;
    let steam = fake_steam(dir.path(), &proton_stub(ok));
    let base = dir.path().join("sandboxes");
    std::fs::create_dir_all(&base).unwrap();
    let out = dir.path().join("out");
    let world = bridge(&steam, &base, Duration::from_secs(30))
        .binarize_world(&staging(dir.path()), "naves\\worlds\\w", "w", &out)
        .unwrap();
    assert_eq!(world, out.join("w.wrp"));
    assert_eq!(&std::fs::read(&world).unwrap()[..4], b"OPRW");
    assert_eq!(
        std::fs::read_to_string(out.join("binarize.log"))
            .unwrap()
            .trim(),
        "fake log"
    );
    assert!(
        leftovers(&base).is_empty(),
        "the sandbox is deleted on success"
    );
}

#[test]
fn failing_binarize_keeps_sandbox_and_log() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let steam = fake_steam(
        dir.path(),
        &proton_stub(r#"echo "boom" > "$sandbox/binarize.log"; exit 3"#),
    );
    let base = dir.path().join("sandboxes");
    std::fs::create_dir_all(&base).unwrap();
    let out = dir.path().join("out");
    let err = bridge(&steam, &base, Duration::from_secs(30))
        .binarize_world(&staging(dir.path()), "naves\\worlds\\w", "w", &out)
        .unwrap_err();
    match &err {
        ToolError::Binarize { code, log, sandbox } => {
            assert_eq!(*code, 3);
            assert_eq!(log, &out.join("binarize.log"));
            assert!(
                sandbox.starts_with(&base) && sandbox.exists(),
                "kept for inspection"
            );
        }
        other => panic!("expected Binarize, got {other:?}"),
    }
    assert!(err.to_string().contains("binarize.log"), "{err}");
    assert_eq!(
        std::fs::read_to_string(out.join("binarize.log"))
            .unwrap()
            .trim(),
        "boom"
    );
}

#[test]
fn success_without_a_world_is_an_error() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let steam = fake_steam(dir.path(), &proton_stub("exit 0"));
    let base = dir.path().join("sandboxes");
    std::fs::create_dir_all(&base).unwrap();
    let err = bridge(&steam, &base, Duration::from_secs(30))
        .binarize_world(
            &staging(dir.path()),
            "naves\\worlds\\w",
            "w",
            &dir.path().join("out"),
        )
        .unwrap_err();
    assert!(matches!(err, ToolError::NoWorld { .. }), "{err}");
}

#[test]
fn output_that_is_not_oprw_is_an_error() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let steam = fake_steam(
        dir.path(),
        &proton_stub(r#"printf '8WVR' > "$sandbox/out/w.wrp"; exit 0"#),
    );
    let base = dir.path().join("sandboxes");
    std::fs::create_dir_all(&base).unwrap();
    let err = bridge(&steam, &base, Duration::from_secs(30))
        .binarize_world(
            &staging(dir.path()),
            "naves\\worlds\\w",
            "w",
            &dir.path().join("out"),
        )
        .unwrap_err();
    assert!(matches!(err, ToolError::NoWorld { .. }), "{err}");
}

#[test]
fn hanging_tool_times_out() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let steam = fake_steam(dir.path(), &proton_stub("exec sleep 60"));
    let base = dir.path().join("sandboxes");
    std::fs::create_dir_all(&base).unwrap();
    let started = std::time::Instant::now();
    let err = bridge(&steam, &base, Duration::from_secs(1))
        .binarize_world(
            &staging(dir.path()),
            "naves\\worlds\\w",
            "w",
            &dir.path().join("out"),
        )
        .unwrap_err();
    assert!(matches!(err, ToolError::Timeout { .. }), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "the tool was stopped, not waited for"
    );
}

#[test]
fn unsafe_names_are_rejected_before_anything_runs() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let steam = fake_steam(dir.path(), "exit 99");
    let base = dir.path().join("sandboxes");
    std::fs::create_dir_all(&base).unwrap();
    for (prefix, world) in [
        ("naves\\worlds\\w & calc", "w"),
        ("naves\\worlds\\w", "w\"x"),
        ("", "w"),
    ] {
        let err = bridge(&steam, &base, Duration::from_secs(5))
            .binarize_world(&staging(dir.path()), prefix, world, &dir.path().join("out"))
            .unwrap_err();
        assert!(
            matches!(err, ToolError::BadName(_)),
            "{prefix} {world}: {err}"
        );
    }
    assert_eq!(leftovers(&base), [] as [std::path::PathBuf; 0]);
}

#[test]
#[ignore = "needs Steam, DayZ Tools and Proton"]
fn binarizes_flat_world() {
    let _serial = serial();
    let tools = Tools::locate().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let world = dir.path().join("P/naves/worlds/bridgetest");
    std::fs::create_dir_all(&world).unwrap();
    let w = EditableWorld {
        land_range_x: 64,
        land_range_y: 64,
        terrain_range_x: 256,
        terrain_range_y: 256,
        cell_size: 25.0,
        elevation: vec![100.0; 256 * 256],
        material_index: vec![1; 64 * 64],
        materials: vec![
            String::new(),
            "naves\\worlds\\bridgetest\\data\\layers\\p_000-000.rvmat".to_string(),
        ],
        objects: Vec::new(),
    };
    let mut bytes = Vec::new();
    w.write(&mut bytes).unwrap();
    std::fs::write(world.join("bridgetest.wrp"), bytes).unwrap();
    let config = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/bridge-experiment/config.cpp"),
    )
    .unwrap();
    std::fs::write(world.join("config.cpp"), config).unwrap();
    let layers = world.join("data/layers");
    std::fs::create_dir_all(&layers).unwrap();
    let rvmat = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/bridge-experiment/p_000-000.rvmat"),
    )
    .unwrap();
    std::fs::write(layers.join("p_000-000.rvmat"), rvmat).unwrap();
    let out = dir.path().join("out");
    let wrp = tools
        .binarize
        .binarize_world(
            &dir.path().join("P"),
            "naves\\worlds\\bridgetest",
            "bridgetest",
            &out,
        )
        .unwrap();
    let head = std::fs::read(wrp).unwrap();
    assert_eq!(&head[..4], b"OPRW");
    assert_eq!(&head[4..8], &32u32.to_le_bytes());
}

/// `target` as a path relative to the current directory.
fn relative_to_cwd(target: &Path) -> PathBuf {
    let cwd = std::env::current_dir().unwrap();
    let mut out = PathBuf::new();
    for _ in cwd.components().skip(1) {
        out.push("..");
    }
    out.join(target.strip_prefix("/").unwrap())
}

/// The tools run in other working directories, so a relative path from the environment has to be resolved early.
#[test]
fn relative_tool_paths_become_absolute() {
    let _lock = serial();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let steam = fake_steam(&root, "exit 0");
    let dayzmake = fake_dayzmake(&root);
    let proton = steam.join("steamapps/common/Proton Hotfix/proton");
    let env = env_of(vec![
        ("STEAM_ROOT", steam.clone()),
        ("DAYZMAKE", relative_to_cwd(&dayzmake)),
        ("PROTON", relative_to_cwd(&proton)),
        (
            "DAYZ_TOOLS",
            relative_to_cwd(&steam.join("steamapps/common/DayZ Tools")),
        ),
    ]);
    let tools = Tools::locate_with(&env, None, None).unwrap();
    assert!(tools.dayzmake.is_absolute(), "{}", tools.dayzmake.display());
    assert!(tools.binarize.proton.is_absolute());
    assert!(tools.binarize.dayz_tools.is_absolute());
    assert!(tools.dayz_server.is_absolute());
}

#[test]
fn steam_is_found_in_the_windows_program_folder() {
    let _lock = serial();
    let dir = tempfile::tempdir().unwrap();
    let programs = dir.path().join("Program Files (x86)");
    let steam = programs.join("Steam");
    std::fs::rename(fake_steam(&programs, "exit 0"), &steam).unwrap();
    let dayzmake = fake_dayzmake(dir.path());
    let env = env_of(vec![
        ("ProgramFiles(x86)", programs.clone()),
        ("DAYZMAKE", dayzmake),
    ]);
    let tools = Tools::locate_with(&env, None, None).unwrap();
    assert_eq!(tools.binarize.steam_root, steam);
}

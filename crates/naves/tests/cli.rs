// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The `naves new` and `naves build` commands, run as the real binary.

#![cfg(unix)]
#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use naves_export::testing::FakeEnv;

/// `naves` with `HOME` and the XDG folders in a temporary folder, so nothing outside it is touched.
fn naves(home: &Path) -> Command {
    let mut cmd = Command::cargo_bin("naves").unwrap();
    cmd.env_clear()
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("PATH", home.join("no-bin"));
    // keep coverage runs from writing profile data into the working directory
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        cmd.env("LLVM_PROFILE_FILE", profile);
    }
    cmd
}

fn with_fakes(cmd: &mut Command, fakes: &FakeEnv) {
    for (key, value) in fakes.env() {
        cmd.env(key, value);
    }
}

fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

fn stderr(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stderr.clone()).unwrap()
}

/// A project folder with the Königssee DEM, created through the CLI.
fn new_project(home: &Path) -> PathBuf {
    let proj = home.join("proj");
    naves(home)
        .args(["new"])
        .arg(&proj)
        .args([
            "--name",
            "Test",
            "--center",
            "47.55,12.975",
            "--extent",
            "1500",
        ])
        .assert()
        .success();
    std::fs::create_dir_all(proj.join("dem")).unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/dem/koenigssee.tif");
    std::fs::copy(fixture, proj.join("dem/koenigssee.tif")).unwrap();
    let toml = std::fs::read_to_string(proj.join("naves.toml")).unwrap();
    let toml = toml
        .replace("dem = []", "dem = [\"dem/koenigssee.tif\"]")
        .replace("terrain_size = 1024", "terrain_size = 256")
        .replace("land_size = 128", "land_size = 32");
    std::fs::write(proj.join("naves.toml"), toml).unwrap();
    proj
}

#[test]
fn new_writes_project() {
    let home = tempfile::tempdir().unwrap();
    let proj = home.path().join("koenigssee");
    let out = naves(home.path())
        .arg("new")
        .arg(&proj)
        .args([
            "--name",
            "Königssee Test",
            "--center",
            "47.55,12.97",
            "--extent",
            "4000",
        ])
        .assert()
        .success();
    let toml = std::fs::read_to_string(proj.join("naves.toml")).unwrap();
    assert!(toml.contains("world = \"konigssee_test\""), "{toml}");
    assert!(toml.contains("center_lat = 47.55"), "{toml}");
    assert!(toml.contains("real_extent_m = 4000.0"), "{toml}");
    let ignore = std::fs::read_to_string(proj.join(".gitignore")).unwrap();
    assert!(ignore.lines().any(|l| l == "secrets/") && ignore.lines().any(|l| l == "build/"));
    assert!(stdout(&out).contains("naves.toml"), "the new file is named");
}

#[test]
fn imagery_list_shows_licences_and_the_non_commercial_mark() {
    let home = tempfile::tempdir().unwrap();
    let proj = home.path().join("berchtesgaden");
    naves(home.path())
        .arg("new")
        .arg(&proj)
        .args([
            "--name",
            "Berchtesgaden",
            "--center",
            "47.585,12.965",
            "--extent",
            "12800",
        ])
        .assert()
        .success();
    // a user file with one more source, found through XDG_CONFIG_HOME
    let config = home.path().join(".config/naves");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("imagery-sources.toml"),
        "[[source]]\nid = \"my-wms\"\nname = \"My WMS\"\nkind = \"wms\"\nurl = \"https://example.org/wms\"\n\
         layer = \"l\"\nnative_m = 2.0\nlicence = \"CC0\"\nattribution = \"Me\"\n",
    )
    .unwrap();
    let out = naves(home.path())
        .args(["imagery", "list"])
        .arg(&proj)
        .assert()
        .success();
    let text = stdout(&out);
    assert!(text.contains("Berchtesgaden"), "{text}");
    for wanted in [
        "sentinel2-scenes",
        "bayern-dop40",
        "my-wms",
        "CC BY 4.0",
        "whole map",
        "naves imagery fetch",
    ] {
        assert!(text.contains(wanted), "{wanted} missing from:\n{text}");
    }
    let nc_line = text
        .lines()
        .find(|l| l.contains("eox-cloudless-2025"))
        .expect("the non-commercial mosaic is listed");
    assert!(
        nc_line.contains("CC BY-NC-SA 4.0") && nc_line.contains("NON-COMMERCIAL"),
        "{nc_line}"
    );
    let commercial = text
        .lines()
        .find(|l| l.contains("sentinel2-scenes"))
        .unwrap();
    assert!(!commercial.contains("NON-COMMERCIAL"), "{commercial}");
    // without the user file, my-wms is gone
    let plain = naves(home.path())
        .env("XDG_CONFIG_HOME", home.path().join("elsewhere"))
        .args(["imagery", "list"])
        .arg(&proj)
        .assert()
        .success();
    assert!(!stdout(&plain).contains("my-wms"));
}

#[test]
fn new_refuses_existing() {
    let home = tempfile::tempdir().unwrap();
    let proj = home.path().join("p");
    let run = || {
        naves(home.path())
            .arg("new")
            .arg(&proj)
            .args(["--name", "x", "--center", "1,2", "--extent", "1000"])
            .assert()
    };
    run().success();
    let before = std::fs::read(proj.join("naves.toml")).unwrap();
    let second = run().failure();
    assert!(stderr(&second).contains("exists"), "{}", stderr(&second));
    assert_eq!(std::fs::read(proj.join("naves.toml")).unwrap(), before);
}

#[test]
fn new_rejects_bad_numbers() {
    let home = tempfile::tempdir().unwrap();
    for args in [
        ["--center", "abc", "--extent", "1000"],
        ["--center", "91,0", "--extent", "1000"],
        ["--center", "1,2", "--extent", "-5"],
    ] {
        let assert = naves(home.path())
            .arg("new")
            .arg(home.path().join("bad"))
            .args(["--name", "x"])
            .args(args)
            .assert()
            .failure();
        assert!(!stderr(&assert).is_empty());
        assert!(!home.path().join("bad/naves.toml").exists(), "{args:?}");
    }
}

#[test]
fn build_without_dem_explains() {
    let home = tempfile::tempdir().unwrap();
    let fakes = FakeEnv::new(&home.path().join("fake")).unwrap();
    let proj = home.path().join("p");
    naves(home.path())
        .arg("new")
        .arg(&proj)
        .args([
            "--name",
            "x",
            "--center",
            "47.55,12.975",
            "--extent",
            "1500",
        ])
        .assert()
        .success();
    let mut cmd = naves(home.path());
    with_fakes(&mut cmd, &fakes);
    let out = cmd.arg("build").arg(&proj).assert().failure();
    assert!(stderr(&out).contains("sources.dem"), "{}", stderr(&out));
}

#[test]
fn build_names_missing_tools_before_doing_anything() {
    let home = tempfile::tempdir().unwrap();
    let proj = new_project(home.path());
    let out = naves(home.path())
        .arg("build")
        .arg(&proj)
        .assert()
        .failure();
    let err = stderr(&out);
    assert!(err.contains("armake2") && err.contains("ARMAKE2"), "{err}");
    assert!(!proj.join("build").exists(), "no work was started");
}

#[test]
fn build_prints_the_paths() {
    let home = tempfile::tempdir().unwrap();
    let fakes = FakeEnv::new(&home.path().join("fake")).unwrap();
    let proj = new_project(home.path());
    let mut cmd = naves(home.path());
    with_fakes(&mut cmd, &fakes);
    let out = cmd.arg("build").arg(&proj).assert().success();
    let text = stdout(&out);
    assert!(
        text.contains(proj.join("build/@naves_test").to_str().unwrap()),
        "{text}"
    );
    assert!(
        text.contains(
            proj.join("build/missions/dayzOffline.test")
                .to_str()
                .unwrap()
        ),
        "{text}"
    );
    assert!(!stderr(&out).is_empty(), "progress goes to stderr");
}

#[test]
fn rebuild_removes_stale_files() {
    let home = tempfile::tempdir().unwrap();
    let fakes = FakeEnv::new(&home.path().join("fake")).unwrap();
    let proj = new_project(home.path());
    let stale = proj.join("build/@naves_test/addons/old.pbo");
    std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
    std::fs::write(&stale, "old").unwrap();
    let mut cmd = naves(home.path());
    with_fakes(&mut cmd, &fakes);
    cmd.arg("build").arg(&proj).assert().success();
    assert!(!stale.exists());
    assert!(
        proj.join("build/@naves_test/addons/naves_test.pbo")
            .is_file()
    );
}

#[test]
#[ignore = "needs Steam, DayZ Tools, Proton, armake2"]
fn builds_koenigssee_fixture() {
    let home = tempfile::tempdir().unwrap();
    let proj = new_project(home.path());
    // the real tools: the real HOME is needed to find Steam, but nothing is written there
    let real_home = std::env::var_os("HOME").unwrap();
    let mut cmd = Command::cargo_bin("naves").unwrap();
    cmd.env("XDG_CACHE_HOME", home.path().join(".cache"));
    cmd.env("HOME", real_home);
    cmd.arg("build").arg(&proj).assert().success();
    let pbo = proj.join("build/@naves_test/addons/naves_test.pbo");
    assert!(pbo.is_file());
    let verify = std::process::Command::new("armake2")
        .arg("inspect")
        .arg(&pbo)
        .output()
        .unwrap();
    assert!(verify.status.success());
}

#[test]
fn gui_without_a_project_says_what_is_missing() {
    let home = tempfile::tempdir().unwrap();
    let empty = home.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let out = naves(home.path()).arg("gui").arg(&empty).assert().failure();
    assert!(stderr(&out).contains("naves.toml"), "{}", stderr(&out));
}

#[test]
fn build_accepts_a_relative_project_folder() {
    let home = tempfile::tempdir().unwrap();
    let fakes = FakeEnv::new(&home.path().join("fake")).unwrap();
    new_project(home.path());
    let mut cmd = naves(home.path());
    with_fakes(&mut cmd, &fakes);
    // run from inside the project's parent with a relative path, as a user would
    cmd.current_dir(home.path())
        .args(["build", "proj"])
        .assert()
        .success();
    assert!(
        home.path()
            .join("proj/build/@naves_test/addons/naves_test.pbo")
            .is_file()
    );
}

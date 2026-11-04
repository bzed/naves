// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `naves --version` smoke test.

use assert_cmd::Command;

#[test]
fn version_is_printed() {
    let out = Command::cargo_bin("naves")
        .unwrap()
        .arg("--version")
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert_eq!(text.trim(), "naves 0.1.0");
}

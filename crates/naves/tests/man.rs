// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The man page is generated from the clap definition.

use assert_cmd::Command;

#[test]
fn man_prints_roff() {
    let out = Command::cargo_bin("naves")
        .unwrap()
        .arg("man")
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(text.contains(".TH naves 1"), "{text}");
}

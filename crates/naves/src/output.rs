// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The binary's only place that prints: results to standard output, progress and errors to standard error.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "this module is the command line's output channel"
)]

/// Prints a result line to standard output.
pub fn result(line: &str) {
    println!("{line}");
}

/// Prints a progress or error line to standard error.
pub fn note(line: &str) {
    eprintln!("{line}");
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Náves – terrain generator and map editor for DayZ.

mod cli;
mod gui;
mod mcp;
mod output;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    match cli::run(cli::Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            output::note(&format!("error: {error:#}"));
            ExitCode::FAILURE
        }
    }
}

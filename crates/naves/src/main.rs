// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Náves – terrain generator and map editor for DayZ.

use clap::{CommandFactory, Parser, Subcommand};

/// Náves – terrain generator and map editor for DayZ.
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Print the man page (roff) to standard output; used by the packaging.
    #[command(hide = true)]
    Man,
}

#[expect(
    clippy::print_stdout,
    reason = "the binary's output module; stdout is its interface"
)]
fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if let Some(Command::Man) = cli.command {
        let mut buf = Vec::new();
        clap_mangen::Man::new(Cli::command()).render(&mut buf)?;
        print!("{}", String::from_utf8(buf)?);
    }
    Ok(())
}

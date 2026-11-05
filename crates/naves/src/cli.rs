// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The command line: `naves new` and `naves build`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use naves_core::project::{PROJECT_FILE, Project};
use naves_export::build::build;
use naves_export::tools::Tools;

use crate::output;

/// What `naves new` puts into `.gitignore`: generated output and the signing key.
const GITIGNORE: &str = "build/\nsecrets/\n";

/// Náves – terrain generator and map editor for DayZ.
#[derive(Parser)]
#[command(version, arg_required_else_help = true)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a project folder with a `naves.toml`.
    New {
        /// The folder to create (it must not contain a `naves.toml` yet).
        dir: PathBuf,
        /// Display name of the map.
        #[arg(long)]
        name: String,
        /// Centre of the real-world area as `<lat>,<lon>` in degrees.
        #[arg(long, value_parser = parse_center, allow_hyphen_values = true)]
        center: (f64, f64),
        /// Side length of the real-world square in metres.
        #[arg(long, allow_hyphen_values = true)]
        extent: f64,
    },
    /// Build the mod and the server mission of a project.
    Build {
        /// The project folder (the one with `naves.toml`).
        dir: PathBuf,
    },
    /// Open the graphical editor on a project.
    Gui {
        /// The project folder (default: the current folder).
        dir: Option<PathBuf>,
    },
    /// Print the man page (roff) to standard output; used by the packaging.
    #[command(hide = true)]
    Man,
}

/// Parses `<lat>,<lon>`.
fn parse_center(text: &str) -> Result<(f64, f64), String> {
    let (lat, lon) = text
        .split_once(',')
        .ok_or("expected <lat>,<lon>, for example 47.585,12.965")?;
    let number = |s: &str, what: &str| {
        s.trim()
            .parse::<f64>()
            .map_err(|_| format!("{what} {s:?} is not a number"))
    };
    Ok((number(lat, "latitude")?, number(lon, "longitude")?))
}

/// Runs the command line.
///
/// # Errors
///
/// Any error of the command, with context for the user.
pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::New {
            dir,
            name,
            center,
            extent,
        } => new_project(&dir, &name, center, extent),
        Command::Build { dir } => build_project(&dir),
        Command::Gui { dir } => crate::gui::run(&dir.unwrap_or_else(|| PathBuf::from("."))),
        Command::Man => {
            let mut buf = Vec::new();
            clap_mangen::Man::new(Cli::command()).render(&mut buf)?;
            output::result(String::from_utf8(buf)?.trim_end());
            Ok(())
        }
    }
}

fn new_project(dir: &Path, name: &str, center: (f64, f64), extent: f64) -> Result<()> {
    let file = dir.join(PROJECT_FILE);
    if file.exists() {
        bail!("{} exists already", file.display());
    }
    let project = Project::new(name, center.0, center.1, extent);
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    project.save(dir)?;
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, GITIGNORE)
            .with_context(|| format!("cannot write {}", ignore.display()))?;
    }
    output::result(&format!("Created {}", file.display()));
    output::result(
        "Next: add elevation files to sources.dem in naves.toml, then run `naves build`.",
    );
    Ok(())
}

fn build_project(dir: &Path) -> Result<()> {
    // Tools first: a missing tool must be reported before any long work starts.
    let tools = Tools::locate()?;
    let out = build(dir, &tools, &|line| output::note(line))?;
    for warning in &out.warnings {
        output::note(&format!("warning: {warning}"));
    }
    output::result(&format!("Mod:     {}", out.mod_dir.display()));
    output::result(&format!("Mission: {}", out.mission_dir.display()));
    Ok(())
}

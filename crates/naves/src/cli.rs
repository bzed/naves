// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The command line: `naves new` and `naves build`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use naves_core::heightmap::MapGeo;
use naves_core::imagery::catalog::{
    ImageryOption, WHOLE_MAP, catalog, find_source, options_for, user_catalog_path,
};
use naves_core::landcover::GeoWindow;
use naves_core::project::{PROJECT_FILE, Project};
use naves_export::build::build;
use naves_export::imagery_fetch::{FetchRequest, RealHttp, describe_fetch, fetch};
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
    /// Run the MCP server for a project on standard input and output.
    Mcp {
        /// The project folder.
        dir: PathBuf,
    },
    /// Find imagery for the area of a project.
    Imagery {
        #[command(subcommand)]
        action: ImageryAction,
    },
    /// Print the man page (roff) to standard output; used by the packaging.
    #[command(hide = true)]
    Man,
}

#[derive(Subcommand)]
enum ImageryAction {
    /// Fetch imagery for the area of a project into its `imagery` folder and list it in `naves.toml`.
    Fetch {
        /// The id of the source, from `naves imagery list`.
        id: String,
        /// The project folder (default: the current folder).
        dir: Option<PathBuf>,
        /// First day of the search for scenes, `YYYY-MM-DD` (needs `--to`).
        #[arg(long)]
        from: Option<String>,
        /// Last day of the search for scenes, `YYYY-MM-DD` (needs `--from`).
        #[arg(long)]
        to: Option<String>,
    },
    /// Show which imagery sources exist for the area of a project and what each one allows.
    List {
        /// The project folder (default: the current folder).
        dir: Option<PathBuf>,
    },
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
        Command::Mcp { dir } => run_mcp(&dir),
        Command::Imagery { action } => match action {
            ImageryAction::List { dir } => imagery_list(&dir.unwrap_or_else(|| PathBuf::from("."))),
            ImageryAction::Fetch { id, dir, from, to } => imagery_fetch(
                &dir.unwrap_or_else(|| PathBuf::from(".")),
                &id,
                &FetchRequest { from, to },
            ),
        },
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

fn run_mcp(dir: &Path) -> Result<()> {
    // Fail early and clearly if the folder is not a project; then serve until the client leaves.
    Project::load(dir)?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("cannot start the async runtime")?
        .block_on(crate::mcp::serve(dir))
}

/// `naves imagery list`: the sources that have imagery for the area of the project.
fn imagery_list(dir: &Path) -> Result<()> {
    let project = Project::load(dir)?;
    let window = MapGeo::new(&project)?.window(0.0)?;
    let sources = catalog(user_catalog_path().as_deref())?;
    let options = options_for(&sources, window);
    output::result(&format_options(&project, window, &options));
    Ok(())
}

/// The table of `naves imagery list`.
fn format_options(project: &Project, window: GeoWindow, options: &[ImageryOption]) -> String {
    let mut text = format!(
        "Imagery for {} ({:.2}..{:.2} E, {:.2}..{:.2} N)\n\n",
        project.name, window.west, window.east, window.south, window.north
    );
    let rows: Vec<[String; 5]> = options
        .iter()
        .map(|o| {
            let covers = if o.coverage_share >= WHOLE_MAP {
                "whole map".to_string()
            } else {
                format!("part (~{:.0}%)", o.coverage_share * 100.0)
            };
            let pixel = if o.source.native_m >= 1.0 {
                format!("{:.0} m", o.source.native_m)
            } else {
                format!("{} m", o.source.native_m)
            };
            let licence = if o.source.non_commercial {
                format!("{}  NON-COMMERCIAL", o.source.licence)
            } else {
                o.source.licence.clone()
            };
            [
                o.source.id.clone(),
                covers,
                pixel,
                licence,
                o.source.note.clone(),
            ]
        })
        .collect();
    let header = ["ID", "COVERS", "PIXEL", "LICENCE", "NOTES"].map(str::to_string);
    let widths: Vec<usize> = (0..5)
        .map(|c| {
            std::iter::once(&header)
                .chain(&rows)
                .map(|r| r.get(c).map_or(0, |cell| cell.chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    for row in std::iter::once(&header).chain(&rows) {
        let line: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        text.push_str(line.join("  ").trim_end());
        text.push('\n');
    }
    if rows.is_empty() {
        text.push_str("(no source has imagery for this area)\n");
    }
    text.push_str(
        "\nFetch with: naves imagery fetch <ID> [DIR]\n\
         Several can be combined: Náves evens out their colours and uses the sharpest where they overlap.\n\
         Or use your own files: see the manual, \"Imagery\".",
    );
    text
}

/// `naves imagery fetch`: downloads imagery for the project and lists it in `naves.toml`.
fn imagery_fetch(dir: &Path, id: &str, request: &FetchRequest) -> Result<()> {
    let project = Project::load(dir)?;
    let sources = catalog(user_catalog_path().as_deref())?;
    let source = find_source(&sources, id)?;
    // what it is and what it allows, before anything is downloaded
    output::note(&format!("{}: {}", source.name, source.licence));
    if source.non_commercial {
        output::note(
            "NON-COMMERCIAL: this source does not allow commercial use, and the finished map will say so.",
        );
    }
    output::note(&describe_fetch(&project, source)?);
    let files = fetch(dir, source, request, &RealHttp::new(), &|line| {
        output::note(line);
    })?;
    for file in &files {
        output::result(&format!("Added {} ({})", file.path.display(), file.licence));
    }
    output::result("naves.toml lists the files; `naves build` uses them.");
    Ok(())
}

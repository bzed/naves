// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The build pipeline: project, DEM and tools in; a signed mod and a server mission out.
//!
//! Layout below `<project>/build/`:
//!
//! ```text
//! staging/P/naves/worlds/<world>/   what binarize reads (8WVR, config.cpp, .paa, .rvmat)
//! staging/binarize/                 what binarize wrote (<world>.wrp, binarize.log)
//! staging/pack/naves/worlds/<world>/ what is packed (OPRW world replaces the 8WVR)
//! @naves_<world>/{addons,keys}/     the mod
//! missions/dayzOffline.<world>/     the server mission
//! logs/                             tool logs
//! ```
//!
//! `staging`, the mod folder and `missions` are removed before every build, so nothing of an earlier
//! build (a different grid size, an old PBO) ends up in the new mod. `<project>/secrets/` is never removed.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use naves_core::conform::ConformError;
use naves_core::dem::DemError;
use naves_core::heightmap::{Heightmap, HeightmapError};
use naves_core::imagery::{ImageryError, TileImagery, load_imagery};
use naves_core::objects::{OverridesError, piece_transform};
use naves_core::project::{ImageryFile, Project, ProjectError};
use naves_core::roadplace::PlaceError;
use naves_core::surface::SurfaceError;
use naves_formats::paa::{ColourTags, PaaError, with_colour_tags, without_stray_lzo_bytes};
use naves_formats::wrp8::{EditableWorld, WorldObject};
use rayon::prelude::*;

use crate::bridge::{Finished, copy_tree, run_supervised};
use crate::catalog_read::CatalogReadError;
use crate::config::{prefix, world_config_with};
use crate::generate::{Generated, generate};
use crate::layers::{LayerSet, build_layers};
use crate::mission::{MissionError, write_mission};
use crate::tools::{ToolError, Tools};

/// How long one `armake2` call may take.
const ARMAKE2_TIMEOUT: Duration = Duration::from_mins(30);
/// How much of a failed tool's output goes into the error message.
const ERROR_OUTPUT_CHARS: usize = 2000;

/// What a successful build produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOutput {
    /// The mod folder (`build/@naves_<world>`), to be loaded with `-mod=`.
    pub mod_dir: PathBuf,
    /// The server mission (`build/missions/dayzOffline.<world>`).
    pub mission_dir: PathBuf,
    /// Things worth knowing that did not stop the build.
    pub warnings: Vec<String>,
}

/// Errors from the build.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// The project file cannot be loaded.
    #[error(transparent)]
    Project(#[from] ProjectError),
    /// There is no elevation data to build from.
    #[error("the project has no elevation data: add DEM files to sources.dem in naves.toml")]
    NoDem,
    /// A DEM file cannot be read.
    #[error(transparent)]
    Dem(#[from] DemError),
    /// The DEM does not cover the area.
    #[error(transparent)]
    Heightmap(#[from] HeightmapError),
    /// The ground cannot be decided.
    #[error(transparent)]
    Surface(#[from] SurfaceError),
    /// The road pieces of the game cannot be read.
    #[error(transparent)]
    Catalog(#[from] CatalogReadError),
    /// The roads cannot be placed.
    #[error(transparent)]
    Place(#[from] PlaceError),
    /// The terrain cannot be shaped to the roads.
    #[error(transparent)]
    Conform(#[from] ConformError),
    /// The edits cannot be read.
    #[error(transparent)]
    Overrides(#[from] OverridesError),
    /// An imagery file cannot be read or merged.
    #[error(transparent)]
    Imagery(#[from] ImageryError),
    /// An external tool is missing or failed.
    #[error(transparent)]
    Tool(#[from] ToolError),
    /// The mission could not be assembled.
    #[error(transparent)]
    Mission(#[from] MissionError),
    /// The editable world could not be written.
    #[error("cannot write the editable world: {0}")]
    World(std::io::Error),
    /// `armake2` failed.
    #[error("armake2 {args} failed ({status}):\n{output}")]
    Armake2 {
        /// The command line without the program.
        args: String,
        /// Exit status or "timed out".
        status: String,
        /// The end of its output.
        output: String,
    },
    /// The private key exists but its public key does not.
    #[error(
        "{} exists but {} does not; restore the public key or delete the private key to create a new pair",
        private.display(), public.display()
    )]
    MissingPublicKey {
        /// The private key.
        private: PathBuf,
        /// The expected public key.
        public: PathBuf,
    },
    /// A texture `img2paa` wrote cannot be completed for the game.
    #[error("{}: {source}", path.display())]
    Paa {
        /// The PAA file.
        path: PathBuf,
        /// What is wrong with it.
        source: PaaError,
    },
    /// A file operation failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file or folder concerned.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> BuildError + '_ {
    move |source| BuildError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn narrow(v: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "map sizes are far inside the f32 range"
    )]
    let v = v as f32;
    v
}

fn remove_dir(path: &Path) -> Result<(), BuildError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err(path)(e)),
    }
}

/// Runs `armake2` with `args` (never through a shell); its output goes to `log`.
fn armake2(tools: &Tools, args: &[&OsStr], cwd: &Path, log: &Path) -> Result<(), BuildError> {
    let mut cmd = std::process::Command::new(&tools.armake2);
    cmd.args(args).current_dir(cwd);
    let shown = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    let status = match run_supervised(&mut cmd, log, ARMAKE2_TIMEOUT)? {
        Finished::Exited(status) if status.success() => return Ok(()),
        Finished::Exited(status) => status
            .code()
            .map_or("killed by a signal".to_string(), |c| format!("exit {c}")),
        Finished::TimedOut => "timed out".to_string(),
    };
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let skip = text.chars().count().saturating_sub(ERROR_OUTPUT_CHARS);
    Err(BuildError::Armake2 {
        args: shown,
        status,
        output: text.chars().skip(skip).collect(),
    })
}

/// The average colour (red, green, blue, alpha) of the PNG `name`.
fn average_colour(name: &Path, png: &[u8]) -> Result<[u8; 4], BuildError> {
    let img = image::load_from_memory(png).map_err(|e| BuildError::Io {
        path: name.to_path_buf(),
        source: std::io::Error::other(e),
    })?;
    let rgba = img.to_rgba8();
    let mut sum = [0u64; 4];
    for px in rgba.pixels() {
        for (s, c) in sum.iter_mut().zip(px.0) {
            *s += u64::from(c);
        }
    }
    let n = u64::from(rgba.width()) * u64::from(rgba.height());
    Ok(sum.map(|s| u8::try_from(s.checked_div(n).unwrap_or(0)).unwrap_or(u8::MAX)))
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), BuildError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_err(parent))?;
    }
    std::fs::write(path, bytes).map_err(io_err(path))
}

/// The signing key pair `secrets/naves_<world>.{biprivatekey,bikey}`; created if missing.
fn ensure_keys(
    tools: &Tools,
    project_dir: &Path,
    world: &str,
    log: &Path,
) -> Result<(PathBuf, PathBuf), BuildError> {
    let secrets = project_dir.join("secrets");
    std::fs::create_dir_all(&secrets).map_err(io_err(&secrets))?;
    let name = format!("naves_{world}");
    let private = secrets.join(format!("{name}.biprivatekey"));
    let public = secrets.join(format!("{name}.bikey"));
    if !private.is_file() {
        armake2(
            tools,
            &[OsStr::new("keygen"), OsStr::new("-f"), OsStr::new(&name)],
            &secrets,
            log,
        )?;
    }
    if !public.is_file() {
        return Err(BuildError::MissingPublicKey { private, public });
    }
    Ok((private, public))
}

/// Paths of one build below `<project>/build/`.
struct Layout {
    project_dir: PathBuf,
    world: String,
    prefix: String,
    staging: PathBuf,
    world_dir: PathBuf,
    mod_dir: PathBuf,
    missions: PathBuf,
    /// The mod and the missions while they are built.
    new_mod: PathBuf,
    new_missions: PathBuf,
    logs: PathBuf,
}

impl Layout {
    fn new(project_dir: &Path, world: &str) -> Layout {
        let build_dir = project_dir.join("build");
        let staging = build_dir.join("staging");
        let rel = Path::new("naves").join("worlds").join(world);
        Layout {
            project_dir: project_dir.to_path_buf(),
            world: world.to_string(),
            prefix: prefix(world),
            world_dir: staging.join("P").join(rel),
            new_mod: staging.join("mod"),
            new_missions: staging.join("missions"),
            staging,
            mod_dir: build_dir.join(format!("@naves_{world}")),
            missions: build_dir.join("missions"),
            logs: build_dir.join("logs"),
        }
    }

    /// Removes the staging folder of an earlier build. Its mod and mission stay until a new build has finished.
    fn clean(&self) -> Result<(), BuildError> {
        remove_dir(&self.staging)?;
        std::fs::create_dir_all(&self.logs).map_err(io_err(&self.logs))
    }

    /// Replaces the mod and the missions of an earlier build with the finished ones from staging.
    fn install(&self) -> Result<(), BuildError> {
        for (new, old) in [
            (&self.new_mod, &self.mod_dir),
            (&self.new_missions, &self.missions),
        ] {
            remove_dir(old)?;
            std::fs::rename(new, old).map_err(io_err(old))?;
        }
        Ok(())
    }
}

fn has_extension(name: &str, extension: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(extension))
}

/// Writes the 8WVR, the config, the tile materials and the converted images below `P:`.
fn stage_world(
    layout: &Layout,
    tools: &Tools,
    project: &Project,
    heightmap: &Heightmap,
    layers: &LayerSet,
    objects: Vec<WorldObject>,
) -> Result<(), BuildError> {
    let world_dir = &layout.world_dir;
    let has_roads = !objects.is_empty();
    let editable = EditableWorld {
        land_range_x: project.grid.land_size,
        land_range_y: project.grid.land_size,
        terrain_range_x: heightmap.size,
        terrain_range_y: heightmap.size,
        cell_size: narrow(project.map_size_m() / f64::from(project.grid.land_size)),
        elevation: heightmap.h.clone(),
        material_index: layers.material_index.clone(),
        materials: layers.materials.clone(),
        objects,
    };
    let mut wrp = Vec::new();
    editable.write(&mut wrp).map_err(BuildError::World)?;
    write_file(&world_dir.join(format!("{}.wrp", layout.world)), &wrp)?;
    write_file(
        &world_dir.join("config.cpp"),
        world_config_with(project, heightmap, has_roads).as_bytes(),
    )?;

    let png_dir = layout.staging.join("png");
    for (name, bytes) in &layers.files {
        let target = if has_extension(name, "rvmat") {
            world_dir
        } else {
            &png_dir
        };
        write_file(&target.join(name), bytes)?;
    }
    let pngs: Vec<&String> = layers
        .files
        .iter()
        .map(|(n, _)| n)
        .filter(|n| has_extension(n, "png"))
        .collect();
    pngs.par_iter()
        .map(|name| {
            let kind = if name.contains("/s_") { "dxt1" } else { "dxt5" };
            let source = png_dir.join(name);
            let target = world_dir.join(name.replace(".png", ".paa"));
            // each call logs to its own file so that parallel calls do not interleave
            let log = layout
                .logs
                .join(format!("img2paa-{}.log", name.replace('/', "_")));
            let args = [
                OsStr::new("img2paa"),
                OsStr::new("-f"),
                OsStr::new("-t"),
                OsStr::new(kind),
                source.as_os_str(),
                target.as_os_str(),
            ];
            armake2(tools, &args, &layout.staging, &log)?;
            let png = layers
                .files
                .iter()
                .find(|(n, _)| n == *name)
                .map_or(&[][..], |(_, b)| b.as_slice());
            let tags = ColourTags {
                average: average_colour(&source, png)?,
                // Livonia's masks carry FLAG 1, its satellite images no FLAG
                flag: (kind == "dxt5").then_some(1),
            };
            let paa = std::fs::read(&target).map_err(io_err(&target))?;
            let tagged = without_stray_lzo_bytes(&paa)
                .and_then(|fixed| with_colour_tags(&fixed, &tags))
                .map_err(|source| BuildError::Paa {
                    path: target.clone(),
                    source,
                })?;
            write_file(&target, &tagged)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(())
}

/// Packs the binarized world with its config and images and signs the PBO; returns nothing but writes the mod.
fn pack_mod(
    layout: &Layout,
    tools: &Tools,
    binarized_world: &Path,
    mission_dir: &Path,
) -> Result<(), BuildError> {
    let pack_dir = layout
        .staging
        .join("pack")
        .join("naves")
        .join("worlds")
        .join(&layout.world);
    copy_tree(&layout.world_dir.join("data"), &pack_dir.join("data"))?;
    // The client reads the environment config from the world's ceFiles folder; it comes from the
    // installed server mission (read at build time, never committed).
    let ce_dir = pack_dir.join("ce");
    std::fs::create_dir_all(&ce_dir).map_err(io_err(&ce_dir))?;
    let env_file = mission_dir.join("cfgenvironment.xml");
    std::fs::copy(&env_file, ce_dir.join("cfgenvironment.xml")).map_err(io_err(&env_file))?;
    copy_tree(&mission_dir.join("env"), &ce_dir.join("env"))?;
    std::fs::copy(
        layout.world_dir.join("config.cpp"),
        pack_dir.join("config.cpp"),
    )
    .map_err(io_err(&pack_dir))?;
    std::fs::copy(
        binarized_world,
        pack_dir.join(format!("{}.wrp", layout.world)),
    )
    .map_err(io_err(&pack_dir))?;
    write_file(&pack_dir.join("$PBOPREFIX$"), layout.prefix.as_bytes())?;
    // A navmesh made with DayZDiag (see the manual) goes where the config's navmeshName points.
    let navmesh = layout
        .project_dir
        .join("navmesh")
        .join(format!("{}.nm", layout.world));
    if navmesh.is_file() {
        let target = pack_dir
            .join("navmesh")
            .join(format!("{}.nm", layout.world));
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir).map_err(io_err(dir))?;
        }
        std::fs::copy(&navmesh, &target).map_err(io_err(&navmesh))?;
    }

    let (private, public) = ensure_keys(
        tools,
        &layout.project_dir,
        &layout.world,
        &layout.logs.join("keygen.log"),
    )?;
    let addons = layout.new_mod.join("addons");
    let keys = layout.new_mod.join("keys");
    for dir in [&addons, &keys] {
        std::fs::create_dir_all(dir).map_err(io_err(dir))?;
    }
    let pbo = addons.join(format!("naves_{}.pbo", layout.world));
    let args = [
        OsStr::new("build"),
        OsStr::new("-f"),
        OsStr::new("-k"),
        private.as_os_str(),
        pack_dir.as_os_str(),
        pbo.as_os_str(),
    ];
    armake2(
        tools,
        &args,
        &layout.staging,
        &layout.logs.join("armake2-build.log"),
    )?;
    let bikey = keys.join(format!("naves_{}.bikey", layout.world));
    std::fs::copy(&public, &bikey).map_err(io_err(&bikey))?;
    Ok(())
}

/// The notice that opens `ATTRIBUTION.txt` when a used imagery file does not allow commercial use.
const NON_COMMERCIAL_NOTICE: &str = "NON-COMMERCIAL USE ONLY\n\
    The satellite images of this map come from sources that do not allow commercial use (marked below).\n\
    Do not use this map commercially without a licence from those sources.\n";

/// Added under the notice when a used licence is share-alike.
const SHARE_ALIKE_LINE: &str =
    "Images derived from a share-alike source stay under that licence (marked below).\n";

/// Whether the licence text contains the word `token` (case does not matter).
fn licence_has(licence: &str, token: &str) -> bool {
    licence
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word.eq_ignore_ascii_case(token))
}

/// The text of `ATTRIBUTION.txt`: only the sources this build used.
fn attribution(land_cover: bool, osm: bool, imagery: &[ImageryFile]) -> String {
    let mut text = String::new();
    if imagery.iter().any(|f| f.non_commercial) {
        text.push_str(NON_COMMERCIAL_NOTICE);
        if imagery.iter().any(|f| licence_has(&f.licence, "SA")) {
            text.push_str(SHARE_ALIKE_LINE);
        }
        text.push('\n');
    }
    text.push_str(
        "This map was made with Náves from the following data. Keep this attribution with the map.\n\n\
         Elevation: Copernicus DEM GLO-30\n\
         (c) DLR e.V. 2010-2014 and (c) Airbus Defence and Space GmbH 2014-2018 provided under COPERNICUS by the\n\
         European Union and ESA; all rights reserved\n",
    );
    if land_cover {
        text.push_str(
            "\nLand cover: ESA WorldCover, CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/)\n\
             © ESA WorldCover project 2021 / Contains modified Copernicus Sentinel data (2021) processed by ESA\n\
             WorldCover consortium\n",
        );
    }
    if osm {
        text.push_str(
            "\nMap data: © OpenStreetMap contributors, ODbL 1.0, https://www.openstreetmap.org/copyright\n\
             This map contains OSM-derived data. The Open Database Licence requires that the derived database is\n\
             shared alike.\n",
        );
    }
    // one entry per distinct credit: five scenes of one year are one line
    let mut seen: Vec<(&str, &str, bool)> = Vec::new();
    for file in imagery {
        let key = (
            file.attribution.as_str(),
            file.licence.as_str(),
            file.non_commercial,
        );
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let mark = if file.non_commercial {
            " (non-commercial)"
        } else {
            ""
        };
        for part in [
            "\nSatellite images: ",
            file.attribution.as_str(),
            "\nLicence: ",
            file.licence.as_str(),
            mark,
            "\n",
        ] {
            text.push_str(part);
        }
    }
    text
}

/// `mod.cpp`: the name and the overview that the launcher shows.
fn mod_cpp(project: &Project, imagery: &[ImageryFile]) -> String {
    let name: String = project
        .name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let restriction = if imagery.iter().any(|f| f.non_commercial) {
        " Non-commercial use only: see ATTRIBUTION.txt."
    } else {
        ""
    };
    format!(
        "name = \"{}\";\noverview = \"Made with Náves.{restriction}\";\n",
        name.replace('"', "\"\"")
    )
}

/// Imagery coverage of the map below this gets no warning.
const IMAGERY_UNCOVERED_WARN_SHARE: f64 = 0.01;

/// A navigation mesh made for the terrain from before the roads is out of date.
fn navmesh_warning(layout: &Layout, generated: &Generated) -> Option<String> {
    let navmesh = layout
        .project_dir
        .join("navmesh")
        .join(format!("{}.nm", layout.world));
    (!generated.objects.is_empty() && navmesh.is_file()).then(|| {
        "the terrain has changed (roads and their earthworks): the navigation mesh of the project was made \
         for the old terrain and should be regenerated, see the manual page on the navigation mesh"
            .to_string()
    })
}

/// The objects of the world file, numbered from 1 in the order of their ids.
fn world_objects(generated: &Generated) -> Vec<WorldObject> {
    generated
        .objects
        .iter()
        .zip(&generated.heights)
        .enumerate()
        .map(|(index, (object, height))| WorldObject {
            transform: piece_transform(object, *height),
            id: i32::try_from(index + 1).unwrap_or(i32::MAX),
            model: object.model.clone(),
        })
        .collect()
}

/// How many violations go into the warnings, one line each; the rest is in the report.
const MAX_VIOLATION_LINES: usize = 50;

/// The violations as warnings, one line each, and a last line for what does not fit.
fn violation_warnings(generated: &Generated) -> Vec<String> {
    let mut lines: Vec<String> = generated
        .violations
        .iter()
        .take(MAX_VIOLATION_LINES)
        .map(|v| {
            format!(
                "{}: {} (at {:.0}, {:.0})",
                v.rule.id(),
                v.message,
                v.at.0,
                v.at.1
            )
        })
        .collect();
    let more = generated
        .violations
        .len()
        .saturating_sub(MAX_VIOLATION_LINES);
    if more > 0 {
        lines.push(format!("... and {more} more, see build/report.json"));
    }
    lines
}

/// `build/report.json`: the numbers and every violation.
fn write_report(layout: &Layout, generated: &Generated) -> Result<(), BuildError> {
    let violations: Vec<serde_json::Value> = generated
        .violations
        .iter()
        .map(|v| {
            serde_json::json!({
                "rule": v.rule.id(),
                "x": v.at.0,
                "y": v.at.1,
                "message": v.message,
            })
        })
        .collect();
    let report = serde_json::json!({ "roads": generated.stats, "violations": violations });
    let text = serde_json::to_string_pretty(&report).map_err(|e| BuildError::Io {
        path: layout.project_dir.join("build/report.json"),
        source: std::io::Error::other(e),
    })?;
    write_file(
        &layout.project_dir.join("build/report.json"),
        text.as_bytes(),
    )
}

/// Builds the mod and the mission of the project in `project_dir`.
///
/// `progress` is called with a short line before every step.
///
/// # Errors
///
/// Any [`BuildError`]; a DEM that does not cover the area gives [`BuildError::Heightmap`] naming the
/// first uncovered position, and no output is written in that case.
pub fn build(
    project_dir: &Path,
    tools: &Tools,
    progress: &dyn Fn(&str),
) -> Result<BuildOutput, BuildError> {
    progress("Loading the project");
    // External tools run in other working directories, so every path they get must be absolute.
    let project_dir = &std::path::absolute(project_dir).map_err(io_err(project_dir))?;
    let project = Project::load(project_dir)?;
    // All sources are read before anything is cleaned or any tool runs, so a bad file costs nothing.
    let generated = generate(project_dir, &project, tools, progress)?;
    let heightmap = &generated.heightmap;

    let imagery = load_imagery(&project, project_dir, progress)?;

    progress("Painting the surface layers");
    let layout = Layout::new(project_dir, &project.world);
    let tile_imagery = imagery.as_ref().map(|i| TileImagery {
        composite: &i.composite,
        project: &project,
    });
    let layers = build_layers(
        heightmap,
        project.grid.land_size,
        &layout.prefix,
        &generated.surfaces,
        tile_imagery.as_ref(),
    )?;
    let used_imagery: &[ImageryFile] = imagery.as_ref().map_or(&[], |i| i.used.as_slice());
    let mut warnings = generated.warnings.clone();
    warnings.extend(violation_warnings(&generated));
    warnings.extend(navmesh_warning(&layout, &generated));
    if let Some(imagery) = &imagery {
        warnings.extend(imagery.warnings.iter().cloned());
        if !imagery.used.is_empty() && 1.0 - layers.imagery_share > IMAGERY_UNCOVERED_WARN_SHARE {
            warnings.push(format!(
                "{:.0} % of the map has no satellite imagery; the class colours are drawn there",
                (1.0 - layers.imagery_share) * 100.0
            ));
        }
    }
    let non_commercial: Vec<&str> = used_imagery
        .iter()
        .filter(|f| f.non_commercial)
        .map(|f| f.attribution.as_str())
        .collect();
    if !non_commercial.is_empty() {
        warnings.push(format!(
            "the satellite images use non-commercial sources ({}): the map may only be used non-commercially, \
             see ATTRIBUTION.txt",
            non_commercial.join("; ")
        ));
    }
    layout.clean()?;

    progress("Writing the world and converting the images");
    stage_world(
        &layout,
        tools,
        &project,
        heightmap,
        &layers,
        world_objects(&generated),
    )?;

    progress("Binarizing the world (this takes a while)");
    let binarized = layout.staging.join("binarize");
    let oprw = tools.binarize.binarize_world(
        &layout.staging.join("P"),
        &layout.prefix,
        &layout.world,
        &binarized,
    )?;
    let log = layout.logs.join("binarize.log");
    std::fs::copy(binarized.join("binarize.log"), &log).map_err(io_err(&log))?;

    progress("Assembling the server mission");
    let half = narrow(project.map_size_m() / 2.0);
    let new_mission = write_mission(
        &tools.dayz_server.join("mpmissions"),
        &layout.new_missions,
        &layout.world,
        (half, half),
    )?;

    progress("Packing and signing the mod");
    pack_mod(&layout, tools, &oprw, &new_mission)?;
    let credits = attribution(generated.used_land_cover, generated.used_osm, used_imagery);
    write_file(&layout.new_mod.join("ATTRIBUTION.txt"), credits.as_bytes())?;
    // server owners install the mission separately from the mod
    write_file(&new_mission.join("ATTRIBUTION.txt"), credits.as_bytes())?;
    write_file(
        &layout.new_mod.join("mod.cpp"),
        mod_cpp(&project, used_imagery).as_bytes(),
    )?;
    layout.install()?;
    write_report(&layout, &generated)?;
    let mission_dir = layout
        .missions
        .join(format!("dayzOffline.{}", layout.world));
    Ok(BuildOutput {
        mod_dir: layout.mod_dir,
        mission_dir,
        warnings,
    })
}

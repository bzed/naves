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

use naves_core::dem::{Dem, DemError};
use naves_core::heightmap::{Heightmap, HeightmapError, build_heightmap};
use naves_core::project::{Project, ProjectError};
use naves_formats::wrp8::EditableWorld;
use rayon::prelude::*;

use crate::bridge::{Finished, copy_tree, run_supervised};
use crate::config::{prefix, world_config};
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
            staging,
            mod_dir: build_dir.join(format!("@naves_{world}")),
            missions: build_dir.join("missions"),
            logs: build_dir.join("logs"),
        }
    }

    /// Removes what an earlier build left behind; `secrets/` stays.
    fn clean(&self) -> Result<(), BuildError> {
        for dir in [&self.staging, &self.mod_dir, &self.missions] {
            remove_dir(dir)?;
        }
        std::fs::create_dir_all(&self.logs).map_err(io_err(&self.logs))
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
) -> Result<(), BuildError> {
    let world_dir = &layout.world_dir;
    let editable = EditableWorld {
        land_range_x: project.grid.land_size,
        land_range_y: project.grid.land_size,
        terrain_range_x: heightmap.size,
        terrain_range_y: heightmap.size,
        cell_size: narrow(project.map_size_m() / f64::from(project.grid.land_size)),
        elevation: heightmap.h.clone(),
        material_index: layers.material_index.clone(),
        materials: layers.materials.clone(),
        objects: Vec::new(),
    };
    let mut wrp = Vec::new();
    editable.write(&mut wrp).map_err(BuildError::World)?;
    write_file(&world_dir.join(format!("{}.wrp", layout.world)), &wrp)?;
    write_file(
        &world_dir.join("config.cpp"),
        world_config(project, heightmap).as_bytes(),
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
            armake2(tools, &args, &layout.staging, &log)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(())
}

/// Packs the binarized world with its config and images and signs the PBO; returns nothing but writes the mod.
fn pack_mod(layout: &Layout, tools: &Tools, binarized_world: &Path) -> Result<(), BuildError> {
    let pack_dir = layout
        .staging
        .join("pack")
        .join("naves")
        .join("worlds")
        .join(&layout.world);
    copy_tree(&layout.world_dir.join("data"), &pack_dir.join("data"))?;
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

    let (private, public) = ensure_keys(
        tools,
        &layout.project_dir,
        &layout.world,
        &layout.logs.join("keygen.log"),
    )?;
    let addons = layout.mod_dir.join("addons");
    let keys = layout.mod_dir.join("keys");
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
    if project.sources.dem.is_empty() {
        return Err(BuildError::NoDem);
    }
    progress("Reading the elevation data");
    let dem_files: Vec<PathBuf> = project
        .sources
        .dem
        .iter()
        .map(|p| project_dir.join(p))
        .collect();
    let dem = Dem::open(&dem_files)?;
    progress("Sampling the heightmap");
    let heightmap = build_heightmap(&project, &dem)?;

    progress("Painting the surface layers");
    let layout = Layout::new(project_dir, &project.world);
    let layers = build_layers(&heightmap, project.grid.land_size, &layout.prefix);
    layout.clean()?;

    progress("Writing the world and converting the images");
    stage_world(&layout, tools, &project, &heightmap, &layers)?;

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

    progress("Packing and signing the mod");
    pack_mod(&layout, tools, &oprw)?;

    progress("Assembling the server mission");
    let half = narrow(project.map_size_m() / 2.0);
    let mission_dir = write_mission(
        &tools.dayz_server.join("mpmissions"),
        &layout.missions,
        &layout.world,
        (half, half),
    )?;
    progress("Done");
    Ok(BuildOutput {
        mod_dir: layout.mod_dir,
        mission_dir,
        warnings: Vec::new(),
    })
}

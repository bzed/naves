// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reads the catalog of road pieces from the game's `structures_roads.pbo`.
//!
//! The PBO is unpacked with armake2 into a temporary folder, only the models the catalog uses are
//! parsed, and the folder is removed again. The parsed result is cached per user as JSON; the
//! models themselves are game data and are never kept or distributed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use naves_core::catalog::{
    BRIDGES_PREFIX, CatalogError, PARTS_PREFIX, PieceCatalog, is_catalog_model,
};
use naves_formats::odol::{OdolError, OdolModel, read_odol};
use serde::{Deserialize, Serialize};

use crate::bridge::{Finished, run_supervised};
use crate::tools::{ToolError, Tools};

/// Bumped when the cached shape changes.
const CACHE_FORMAT: u32 = 1;
/// How long unpacking the PBO may take.
const UNPACK_TIMEOUT: Duration = Duration::from_mins(5);
/// How much of armake2's output goes into an error message.
const ERROR_OUTPUT_CHARS: usize = 2000;

/// Errors from reading the catalog.
#[derive(Debug, thiserror::Error)]
pub enum CatalogReadError {
    /// The road PBO is not in the DayZ Server install.
    #[error(
        "{} not found; the road pieces are read from the DayZ Server install (set DAYZ_SERVER or Settings, External tools)",
        .0.display()
    )]
    MissingPbo(PathBuf),
    /// `armake2 unpack` failed.
    #[error("armake2 unpack failed ({status}):\n{output}")]
    Armake2 {
        /// Exit status or "timed out".
        status: String,
        /// The end of its output.
        output: String,
    },
    /// A model that the catalog needs cannot be read.
    #[error("the model {model} cannot be read: {source}")]
    Model {
        /// The file name.
        model: String,
        /// Why not.
        source: OdolError,
    },
    /// The models do not make a complete catalog.
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    /// Starting or supervising armake2 failed.
    #[error(transparent)]
    Tool(#[from] ToolError),
    /// A file operation failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file or folder concerned.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The cache file cannot be written.
    #[error("cannot write the catalog cache {}: {source}", path.display())]
    Cache {
        /// The cache file.
        path: PathBuf,
        /// The underlying error.
        source: serde_json::Error,
    },
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> CatalogReadError + '_ {
    move |source| CatalogReadError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// What identifies the PBO a cache was made from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    size: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    format: u32,
    stamp: Stamp,
    catalog: PieceCatalog,
}

fn stamp(pbo: &Path) -> Result<Stamp, CatalogReadError> {
    let meta = std::fs::metadata(pbo).map_err(io(pbo))?;
    let modified = meta
        .modified()
        .map_err(io(pbo))?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Ok(Stamp {
        size: meta.len(),
        modified_secs: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
    })
}

/// The catalog of the road pieces of the installed game.
///
/// The result is cached in `cache_dir` and rebuilt when the PBO's size or modification time
/// changes (a game update).
///
/// # Errors
/// [`CatalogReadError::MissingPbo`] naming the file, [`CatalogReadError::Armake2`] with the tool's
/// output, [`CatalogReadError::Model`] naming a model that cannot be read, or a
/// [`CatalogError`] if the models do not make a catalog.
pub fn road_catalog(tools: &Tools, cache_dir: &Path) -> Result<PieceCatalog, CatalogReadError> {
    let pbo = tools.dayz_server.join("addons/structures_roads.pbo");
    if !pbo.is_file() {
        return Err(CatalogReadError::MissingPbo(pbo));
    }
    let stamp = stamp(&pbo)?;
    let cache_file = cache_dir.join("catalog").join("roads.json");
    if let Some(catalog) = cached(&cache_file, &stamp) {
        return Ok(catalog);
    }
    let models = read_models(tools, &pbo)?;
    let catalog = PieceCatalog::from_models(&models)?;
    write_cache(&cache_file, &stamp, &catalog)?;
    Ok(catalog)
}

/// The cached catalog if it was made from this very PBO. A missing, unreadable or outdated cache is
/// not an error: the catalog is rebuilt from the game files, which are the source of truth.
fn cached(file: &Path, stamp: &Stamp) -> Option<PieceCatalog> {
    let text = std::fs::read_to_string(file).ok()?;
    let cache: CacheFile = serde_json::from_str(&text).ok()?;
    (cache.format == CACHE_FORMAT && cache.stamp == *stamp).then_some(cache.catalog)
}

fn write_cache(file: &Path, stamp: &Stamp, catalog: &PieceCatalog) -> Result<(), CatalogReadError> {
    let dir = file.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    let text = serde_json::to_string(&CacheFile {
        format: CACHE_FORMAT,
        stamp: stamp.clone(),
        catalog: catalog.clone(),
    })
    .map_err(|source| CatalogReadError::Cache {
        path: file.to_path_buf(),
        source,
    })?;
    // temp file in the same folder, then rename: a reader never sees half a file
    let tmp = tempfile::NamedTempFile::new_in(dir).map_err(io(dir))?;
    std::fs::write(tmp.path(), text).map_err(io(tmp.path()))?;
    tmp.persist(file).map_err(|e| io(file)(e.error))?;
    Ok(())
}

/// Unpacks the PBO into a temporary folder and parses the models the catalog uses.
fn read_models(tools: &Tools, pbo: &Path) -> Result<BTreeMap<String, OdolModel>, CatalogReadError> {
    let work = tempfile::Builder::new()
        .prefix("naves-roads-")
        .tempdir()
        .map_err(io(&std::env::temp_dir()))?;
    let out = work.path().join("content");
    std::fs::create_dir(&out).map_err(io(&out))?;
    let log = work.path().join("armake2.log");
    let mut cmd = std::process::Command::new(&tools.armake2);
    cmd.arg("unpack").arg(pbo).arg(&out);
    let failure = match run_supervised(&mut cmd, &log, UNPACK_TIMEOUT)? {
        Finished::Exited(status) if status.success() => None,
        Finished::Exited(status) => Some(
            status
                .code()
                .map_or("killed by a signal".to_string(), |c| format!("exit {c}")),
        ),
        Finished::TimedOut => Some("timed out".to_string()),
    };
    if let Some(status) = failure {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        let skip = text.chars().count().saturating_sub(ERROR_OUTPUT_CHARS);
        return Err(CatalogReadError::Armake2 {
            status,
            output: text.chars().skip(skip).collect(),
        });
    }
    let mut models = BTreeMap::new();
    for (folder, prefix) in [("parts", PARTS_PREFIX), ("bridges", BRIDGES_PREFIX)] {
        for file in model_files(&out, folder)? {
            let name = file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !is_catalog_model(&name) {
                continue;
            }
            let bytes = std::fs::read(&file).map_err(io(&file))?;
            let model = read_odol(&bytes).map_err(|source| CatalogReadError::Model {
                model: name.clone(),
                source,
            })?;
            let stem = name.strip_suffix(".p3d").unwrap_or(&name);
            let stem = stem.strip_suffix(".P3D").unwrap_or(stem);
            models.insert(format!("{prefix}{}.p3d", stem.to_ascii_lowercase()), model);
        }
    }
    Ok(models)
}

/// The `.p3d` files directly in the sub-folder of `root` whose name matches `folder` in any case,
/// sorted by name.
fn model_files(root: &Path, folder: &str) -> Result<Vec<PathBuf>, CatalogReadError> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(root).map_err(io(root))? {
        let dir = entry.map_err(io(root))?.path();
        let matches = dir.is_dir()
            && dir
                .file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(folder));
        if !matches {
            continue;
        }
        for entry in std::fs::read_dir(&dir).map_err(io(&dir))? {
            let path = entry.map_err(io(&dir))?.path();
            let is_model = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("p3d"));
            if is_model && path.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

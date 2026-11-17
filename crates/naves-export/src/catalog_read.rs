// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reads the catalog of road pieces and bridge parts from the game's archives.
//!
//! Three archives of the DayZ Server install are read: `addons/structures_roads.pbo` (the road pieces, the
//! bridges and the concrete panels), `sakhal/addons/structures_sakhal.pbo` (`bridge_wood2`, `bridge_concrete3`)
//! and `addons/structures_industrial.pbo` (the concrete pipe). Folder names are found in any case (`Addons` on a
//! client install). Each archive is listed with `dayzmake inspect`, which also gives its prefix (the model
//! paths are the prefix and the path inside the archive), and only the models the catalog uses are taken out of
//! it with `dayzmake cat`, into a temporary folder that is removed again. The parsed result is cached per user
//! as JSON; the models themselves are game data and are never kept or distributed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use naves_core::catalog::{CatalogError, PieceCatalog, is_catalog_model};
use naves_formats::odol::{OdolError, OdolModel, read_odol};
use serde::{Deserialize, Serialize};

use crate::bridge::{Finished, run_supervised};
use crate::tools::{ToolError, Tools};

/// Bumped when the cached shape changes.
const CACHE_FORMAT: u32 = 6;
/// How long unpacking the PBO may take.
const UNPACK_TIMEOUT: Duration = Duration::from_mins(5);
/// How long listing an archive or taking one file out of it may take.
const CAT_TIMEOUT: Duration = Duration::from_mins(1);
/// How much of dayzmake's output goes into an error message.
const ERROR_OUTPUT_CHARS: usize = 2000;
/// The most bytes of `dayzmake inspect` output that are read.
const MAX_LISTING_BYTES: u64 = 16 << 20;

/// Errors from reading the catalog.
#[derive(Debug, thiserror::Error)]
pub enum CatalogReadError {
    /// The road PBO is not in the DayZ Server install.
    #[error(
        "{} not found; the road pieces are read from the DayZ Server install (set DAYZ_SERVER or Settings, External tools)",
        .0.display()
    )]
    MissingPbo(PathBuf),
    /// A dayzmake command failed.
    #[error("dayzmake {command} failed ({status}):\n{output}")]
    Dayzmake {
        /// The subcommand (unpack, inspect, cat).
        command: &'static str,
        /// Exit status or "timed out".
        status: String,
        /// The end of its output.
        output: String,
    },
    /// `dayzmake inspect` does not name the archive's prefix.
    #[error("dayzmake inspect names no prefix for {}", .0.display())]
    NoPrefix(PathBuf),
    /// A road piece that the catalog needs cannot be read.
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
    /// Starting or supervising dayzmake failed.
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

/// What of an archive the catalog takes, by the path inside it (lower case, backslashes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Need {
    /// Not used.
    No,
    /// A road piece: one that cannot be read fails the catalog.
    Piece,
    /// A bridge part: one that cannot be read is a warning.
    Part,
}

/// One archive the catalog reads.
struct Archive {
    /// The folders below the server install and the file name, matched in any case.
    path: &'static [&'static str],
    /// Whether the catalog cannot be built without it.
    required: bool,
    /// What it holds that the catalog uses.
    needs: fn(&str) -> Need,
}

fn roads_needs(path: &str) -> Need {
    let Some((folder, file)) = path.split_once('\\') else {
        return Need::No;
    };
    let is_model = Path::new(file)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("p3d"));
    if file.contains('\\') || !is_model {
        return Need::No;
    }
    match folder {
        "parts" if is_catalog_model(file) => Need::Piece,
        "bridges" => Need::Part,
        "panels" if matches!(file, "panel_concrete_1.p3d" | "panel_concrete_2.p3d") => Need::Part,
        _ => Need::No,
    }
}

fn sakhal_needs(path: &str) -> Need {
    match path {
        "roads\\bridges\\bridge_wood2.p3d" | "roads\\bridges\\bridge_concrete3.p3d" => Need::Part,
        _ => Need::No,
    }
}

fn industrial_needs(path: &str) -> Need {
    if path == "misc\\misc_concretepipe.p3d" {
        Need::Part
    } else {
        Need::No
    }
}

/// The archives, in the order they are read.
const ARCHIVES: [Archive; 3] = [
    Archive {
        path: &["addons", "structures_roads.pbo"],
        required: true,
        needs: roads_needs,
    },
    Archive {
        path: &["sakhal", "addons", "structures_sakhal.pbo"],
        required: false,
        needs: sakhal_needs,
    },
    Archive {
        path: &["addons", "structures_industrial.pbo"],
        required: false,
        needs: industrial_needs,
    },
];

/// The file below `root` whose path components match `parts` in any case; an exact match is preferred, else
/// the first in name order.
fn find_ci(root: &Path, parts: &[&str]) -> Option<PathBuf> {
    let mut at = root.to_path_buf();
    for part in parts {
        let exact = at.join(part);
        if exact.exists() {
            at = exact;
            continue;
        }
        let mut names: Vec<PathBuf> = std::fs::read_dir(&at)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(part))
            })
            .collect();
        names.sort();
        at = names.into_iter().next()?;
    }
    at.is_file().then_some(at)
}

/// What identifies an archive a cache was made from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    size: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    format: u32,
    /// Per archive (its path in the install as `ARCHIVES` names it), its stamp, `None` if it is absent.
    stamps: Vec<(String, Option<Stamp>)>,
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

/// An archive as found: its file, its stamp and a warning about it.
type Found = (Option<PathBuf>, Option<Stamp>, Option<String>);

/// The archive `file` (if it was found) with its stamp. An optional archive whose size and time cannot be read
/// is treated as absent, with a warning that starts with its `label`.
///
/// # Errors
/// [`CatalogReadError::Io`] if a `required` archive cannot be stated.
fn archive_stamp(
    file: Option<PathBuf>,
    required: bool,
    label: &str,
) -> Result<Found, CatalogReadError> {
    match file.as_deref().map(stamp).transpose() {
        Ok(stamped) => Ok((file, stamped, None)),
        Err(e) if !required => Ok((
            None,
            None,
            Some(format!(
                "{label} cannot be read: the bridge parts it holds are not used: {e}"
            )),
        )),
        Err(e) => Err(e),
    }
}

/// The catalog of the road pieces and bridge parts of the installed game.
///
/// The result is cached in `cache_dir` and rebuilt when one of the archives appears, goes or changes its size
/// or modification time (a game update). A missing optional archive, or a bridge part that is missing or
/// cannot be read, is a warning in [`PieceCatalog::warnings`].
///
/// # Errors
/// [`CatalogReadError::MissingPbo`] naming the road archive, [`CatalogReadError::Dayzmake`] with the tool's
/// output, [`CatalogReadError::Model`] naming a road piece that cannot be read, or a [`CatalogError`] if the
/// models do not make a catalog.
pub fn road_catalog(tools: &Tools, cache_dir: &Path) -> Result<PieceCatalog, CatalogReadError> {
    let mut found = Vec::new();
    let mut stamps = Vec::new();
    let mut early = Vec::new();
    for archive in &ARCHIVES {
        let label = archive.path.join("/");
        let file = find_ci(&tools.dayz_server, archive.path);
        if file.is_none() && archive.required {
            return Err(CatalogReadError::MissingPbo(
                tools.dayz_server.join(archive.path.join("/")),
            ));
        }
        let (file, stamped, warning) = archive_stamp(file, archive.required, &label)?;
        early.extend(warning);
        stamps.push((label.clone(), stamped));
        found.push((archive, label, file));
    }
    let cache_file = cache_dir.join("catalog").join("roads.json");
    if let Some(mut catalog) = cached(&cache_file, &stamps) {
        for warning in early {
            catalog.warn(warning);
        }
        return Ok(catalog);
    }
    let mut models = BTreeMap::new();
    let mut warnings = early;
    for (archive, label, file) in found {
        let Some(pbo) = file else {
            if !warnings.iter().any(|w| w.starts_with(&label)) {
                warnings.push(format!(
                    "{label} is not in the DayZ Server install: the bridge parts it holds are not used"
                ));
            }
            continue;
        };
        match read_archive(tools, &pbo, archive.needs, &mut models, &mut warnings) {
            Err(e) if !archive.required => warnings.push(format!(
                "{label} cannot be read: the bridge parts it holds are not used: {e}"
            )),
            other => other?,
        }
    }
    let mut catalog = PieceCatalog::from_models(&models)?;
    for warning in warnings {
        catalog.warn(warning);
    }
    write_cache(&cache_file, &stamps, &catalog)?;
    Ok(catalog)
}

/// The cached catalog if it was made from these very archives. A missing, unreadable or outdated cache is
/// not an error: the catalog is rebuilt from the game files, which are the source of truth.
fn cached(file: &Path, stamps: &[(String, Option<Stamp>)]) -> Option<PieceCatalog> {
    let text = std::fs::read_to_string(file).ok()?;
    let cache: CacheFile = serde_json::from_str(&text).ok()?;
    (cache.format == CACHE_FORMAT && cache.stamps == stamps).then_some(cache.catalog)
}

fn write_cache(
    file: &Path,
    stamps: &[(String, Option<Stamp>)],
    catalog: &PieceCatalog,
) -> Result<(), CatalogReadError> {
    let dir = file.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    let text = serde_json::to_string(&CacheFile {
        format: CACHE_FORMAT,
        stamps: stamps.to_vec(),
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

/// Runs dayzmake with `args`, its output in `log`; an error with the end of the output if it fails.
fn dayzmake(
    tools: &Tools,
    command: &'static str,
    args: &[&std::ffi::OsStr],
    log: &Path,
    timeout: Duration,
) -> Result<(), CatalogReadError> {
    let mut cmd = std::process::Command::new(&tools.dayzmake);
    cmd.arg(command).args(args);
    let failure = match run_supervised(&mut cmd, log, timeout)? {
        Finished::Exited(status) if status.success() => None,
        Finished::Exited(status) => Some(
            status
                .code()
                .map_or("killed by a signal".to_string(), |c| format!("exit {c}")),
        ),
        Finished::TimedOut => Some("timed out".to_string()),
    };
    if let Some(status) = failure {
        let text = std::fs::read_to_string(log).unwrap_or_default();
        let skip = text.chars().count().saturating_sub(ERROR_OUTPUT_CHARS);
        return Err(CatalogReadError::Dayzmake {
            command,
            status,
            output: text.chars().skip(skip).collect(),
        });
    }
    Ok(())
}

/// Unpacks the road PBO with dayzmake so that `dest/dz/structures/roads/` holds the game's road models,
/// where `binarize` looks for them below its `P:` drive.
///
/// # Errors
/// [`CatalogReadError::MissingPbo`] naming the file, [`CatalogReadError::Dayzmake`] with the tool's output.
pub fn unpack_roads(tools: &Tools, pbo: &Path, dest: &Path) -> Result<(), CatalogReadError> {
    if !pbo.is_file() {
        return Err(CatalogReadError::MissingPbo(pbo.to_path_buf()));
    }
    let out = dest.join("dz").join("structures").join("roads");
    std::fs::create_dir_all(&out).map_err(io(&out))?;
    let scratch = tempfile::Builder::new()
        .prefix("naves-unpack-")
        .tempdir()
        .map_err(io(&std::env::temp_dir()))?;
    let log = scratch.path().join("dayzmake.log");
    dayzmake(
        tools,
        "unpack",
        &[pbo.as_os_str(), out.as_os_str()],
        &log,
        UNPACK_TIMEOUT,
    )
}

/// What `dayzmake inspect` says about an archive: its prefix (lower case, backslashes, ending in one) and the
/// paths of its files as the archive spells them.
#[derive(Debug, Default, PartialEq)]
struct Listing {
    prefix: String,
    files: Vec<String>,
}

/// Reads the output of `dayzmake inspect <pbo>`: a header with a `- prefix=` line, then a table whose rows after
/// a line of `=` are a path (which may hold spaces) and three numbers.
fn parse_listing(text: &str) -> Listing {
    let mut listing = Listing::default();
    let mut in_table = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if !in_table {
            if let Some(prefix) = trimmed.strip_prefix("- prefix=") {
                let prefix = prefix.trim().trim_matches('\\').replace('/', "\\");
                if !prefix.is_empty() {
                    listing.prefix = format!("{}\\", prefix.to_ascii_lowercase());
                }
            } else if !trimmed.is_empty() && trimmed.chars().all(|c| c == '=') {
                in_table = true;
            }
            continue;
        }
        let mut rest = trimmed;
        let mut numbers = 0;
        while numbers < 3 {
            let Some((head, tail)) = rest.rsplit_once(char::is_whitespace) else {
                break;
            };
            if tail.is_empty() || !tail.chars().all(|c| c.is_ascii_digit()) {
                break;
            }
            rest = head.trim_end();
            numbers += 1;
        }
        if numbers == 3 && !rest.is_empty() {
            listing.files.push(rest.to_string());
        }
    }
    listing
}

/// A path inside an archive that is safe to name: relative, no `..`, no drive.
fn safe_entry(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(['\\', '/'])
        && !name.contains(':')
        && name
            .split(['\\', '/'])
            .all(|part| !part.is_empty() && part != "..")
}

/// Lists `pbo` and parses the models of it that `needs` names into `models`, keyed by the archive's prefix
/// and their path in it (lower case). A road piece that cannot be read is an error, a bridge part a warning.
fn read_archive(
    tools: &Tools,
    pbo: &Path,
    needs: fn(&str) -> Need,
    models: &mut BTreeMap<String, OdolModel>,
    warnings: &mut Vec<String>,
) -> Result<(), CatalogReadError> {
    let work = tempfile::Builder::new()
        .prefix("naves-roads-")
        .tempdir()
        .map_err(io(&std::env::temp_dir()))?;
    let log = work.path().join("inspect.log");
    dayzmake(tools, "inspect", &[pbo.as_os_str()], &log, CAT_TIMEOUT)?;
    let mut text = String::new();
    {
        use std::io::Read;
        std::fs::File::open(&log)
            .map_err(io(&log))?
            .take(MAX_LISTING_BYTES)
            .read_to_string(&mut text)
            .map_err(io(&log))?;
    }
    let listing = parse_listing(&text);
    if listing.prefix.is_empty() {
        return Err(CatalogReadError::NoPrefix(pbo.to_path_buf()));
    }
    let cat_log = work.path().join("cat.log");
    let target = work.path().join("model.p3d");
    for name in listing.files.iter().filter(|n| safe_entry(n)) {
        let path = name.replace('/', "\\").to_ascii_lowercase();
        let need = needs(&path);
        if need == Need::No {
            continue;
        }
        let key = format!("{}{path}", listing.prefix);
        let read = dayzmake(
            tools,
            "cat",
            &[
                pbo.as_os_str(),
                std::ffi::OsStr::new(name),
                target.as_os_str(),
            ],
            &cat_log,
            CAT_TIMEOUT,
        )
        .and_then(|()| std::fs::read(&target).map_err(io(&target)));
        let model = read.and_then(|bytes| {
            read_odol(&bytes).map_err(|source| CatalogReadError::Model {
                model: name.clone(),
                source,
            })
        });
        // the next file is written to the same place
        std::fs::remove_file(&target).ok(); // a file that was never written is fine
        match (model, need) {
            (Ok(model), _) => {
                models.insert(key, model);
            }
            (Err(e), Need::Piece) => return Err(e),
            (Err(e), _) => warnings.push(format!("{key}: cannot be read, it is not used: {e}")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_gives_the_prefix_and_paths_with_spaces() {
        let text = "Header extensions:\n- product=dayz\n- prefix=DZ\\structures\\roads\n- version=1\n\n# Files: 2\n\nPath     Method  Original  Packed\n            Size      Size\n=====\nParts\\asf1_10 100.p3d     0     0    2929\nBridges\\Bridge_Metal_25.p3d   0   0   20263\nnot a row\n";
        let l = parse_listing(text);
        assert_eq!(l.prefix, "dz\\structures\\roads\\");
        assert_eq!(
            l.files,
            vec!["Parts\\asf1_10 100.p3d", "Bridges\\Bridge_Metal_25.p3d"]
        );
        assert_eq!(parse_listing(""), Listing::default());
    }

    #[test]
    fn entries_that_leave_the_archive_are_not_named() {
        for bad in [
            "",
            "\\a.p3d",
            "/a.p3d",
            "c:\\a.p3d",
            "a\\..\\b.p3d",
            "a\\\\b.p3d",
        ] {
            assert!(!safe_entry(bad), "{bad}");
        }
        assert!(safe_entry("Parts\\asf1_10 100.p3d"));
    }

    #[test]
    fn only_the_parts_the_catalog_uses_are_needed() {
        assert_eq!(roads_needs("parts\\asf1_25.p3d"), Need::Piece);
        assert_eq!(roads_needs("parts\\runway_main.p3d"), Need::No);
        assert_eq!(roads_needs("bridges\\bridge_wood_50.p3d"), Need::Part);
        assert_eq!(roads_needs("bridges\\data\\x.p3d"), Need::No);
        assert_eq!(roads_needs("panels\\panel_concrete_1.p3d"), Need::Part);
        assert_eq!(roads_needs("panels\\panel_concrete_dam.p3d"), Need::No);
        assert_eq!(roads_needs("panels\\panel_concrete_1_legacy.p3d"), Need::No);
        assert_eq!(roads_needs("readme"), Need::No);
        assert_eq!(sakhal_needs("roads\\bridges\\bridge_wood2.p3d"), Need::Part);
        assert_eq!(
            sakhal_needs("roads\\bridges\\bridge_wood2_snow.p3d"),
            Need::No
        );
        assert_eq!(
            sakhal_needs("roads\\bridges\\bridge_wood2_pillar.p3d"),
            Need::No
        );
        assert_eq!(industrial_needs("misc\\misc_concretepipe.p3d"), Need::Part);
        assert_eq!(
            industrial_needs("misc\\misc_concretepipe_gate.p3d"),
            Need::No
        );
    }

    #[test]
    fn an_optional_archive_that_cannot_be_stated_is_absent_with_a_warning() {
        // found by its name and gone (or unreadable) when its size and time are read
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("structures_sakhal.pbo");
        let label = "sakhal/addons/structures_sakhal.pbo";
        let (file, stamped, warning) = archive_stamp(Some(gone.clone()), false, label).unwrap();
        assert_eq!((file, stamped), (None, None));
        let warning = warning.unwrap();
        assert!(
            warning.starts_with(label) && warning.contains("cannot be read"),
            "{warning}"
        );
        // the road archive is needed: the same is an error
        assert!(archive_stamp(Some(gone), true, label).is_err());
        // an archive that is there is stamped, one that is not is no stamp, both without a warning
        let there = dir.path().join("there.pbo");
        std::fs::write(&there, b"pbo").unwrap();
        let (file, stamped, warning) = archive_stamp(Some(there.clone()), false, label).unwrap();
        assert_eq!(file, Some(there));
        assert_eq!(stamped.map(|s| s.size), Some(3));
        assert_eq!(warning, None);
        assert_eq!(
            archive_stamp(None, false, label).unwrap(),
            (None, None, None)
        );
    }
}

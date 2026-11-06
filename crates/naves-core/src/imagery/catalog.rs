// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The catalogue of imagery sources: where imagery for an area can be had and on what terms.
//!
//! The built-in list is `data/imagery-sources.toml`. A user file, `imagery-sources.toml` in the Náves folder of
//! the user's configuration, adds entries or replaces one by its id. It is deliberately not part of a project:
//! opening someone's project must never make Náves fetch from a URL that the project chose. Only `https://`
//! URLs are accepted.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::landcover::GeoWindow;

/// The built-in catalogue.
const BUILTIN: &str = include_str!("../../data/imagery-sources.toml");

/// Largest user file that is read.
const MAX_FILE_BYTES: u64 = 1 << 20;

/// Longest URL, credit or note that is accepted.
const MAX_TEXT: usize = 2000;

/// Longest id; it becomes part of file names.
const MAX_ID: usize = 64;

/// Wherever a source covers at least this share of a map it counts as covering all of it.
pub const WHOLE_MAP: f64 = 0.999;

/// Errors of the catalogue.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    /// The file is not valid TOML for a catalogue.
    #[error("cannot parse the imagery sources: {0}")]
    Parse(String),
    /// An entry is not acceptable; the message names it.
    #[error("imagery source {id:?}: {why}")]
    Invalid {
        /// The id of the entry.
        id: String,
        /// What is wrong.
        why: String,
    },
    /// The user file cannot be read.
    #[error("{}: {why}", path.display())]
    Io {
        /// The file.
        path: PathBuf,
        /// What went wrong.
        why: String,
    },
}

/// How a source is fetched.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceKind {
    /// A STAC API that lists scenes; the scenes are cloud-optimised GeoTIFF files.
    Stac {
        /// The API root.
        url: String,
        /// The collection to search.
        collection: String,
        /// The asset that holds the colour image.
        asset: String,
        /// The only hosts a scene's files may come from.
        asset_hosts: Vec<String>,
    },
    /// A Web Map Service (version 1.3.0) that answers images of a bounding box.
    Wms {
        /// The service URL.
        url: String,
        /// The layer to ask for.
        layer: String,
        /// The largest image side in pixels the service allows.
        max_px: u32,
        /// A colour that the service paints where it has nothing (besides transparency).
        blank: Option<[u8; 3]>,
    },
}

/// One imagery source.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogSource {
    /// Short name for the command line; letters, digits and `-`.
    pub id: String,
    /// Name for people.
    pub name: String,
    /// How it is fetched.
    pub kind: SourceKind,
    /// West, south, east and north of what it covers in degrees; `None` is the world.
    pub coverage: Option<[f64; 4]>,
    /// Metres per pixel.
    pub native_m: f64,
    /// The licence in words.
    pub licence: String,
    /// The credit line; may hold `{year}`.
    pub attribution: String,
    /// The licence does not allow commercial use.
    pub non_commercial: bool,
    /// A hint for the user.
    pub note: String,
}

/// A source that has something for an area.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageryOption {
    /// The source.
    pub source: CatalogSource,
    /// Share of the area that the source covers, `0 < share <= 1`.
    pub coverage_share: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    source: Vec<RawSource>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSource {
    id: String,
    name: String,
    kind: String,
    url: String,
    collection: Option<String>,
    asset: Option<String>,
    #[serde(default)]
    asset_hosts: Vec<String>,
    layer: Option<String>,
    max_px: Option<u32>,
    blank: Option<[u8; 3]>,
    coverage: Option<[f64; 4]>,
    native_m: f64,
    licence: String,
    attribution: String,
    #[serde(default)]
    non_commercial: bool,
    #[serde(default)]
    note: String,
}

fn invalid<T>(id: &str, why: impl Into<String>) -> Result<T, CatalogError> {
    Err(CatalogError::Invalid {
        id: id.to_string(),
        why: why.into(),
    })
}

/// `text` if it is a plain one-line string of acceptable length.
fn plain(id: &str, field: &str, text: &str, may_be_empty: bool) -> Result<(), CatalogError> {
    if text.chars().any(char::is_control) {
        return invalid(
            id,
            format!("{field} must be one line without control characters"),
        );
    }
    if text.chars().count() > MAX_TEXT {
        return invalid(id, format!("{field} is longer than {MAX_TEXT} characters"));
    }
    if !may_be_empty && text.trim().is_empty() {
        return invalid(id, format!("{field} must not be empty"));
    }
    Ok(())
}

/// An `https://` URL without whitespace.
fn https(id: &str, field: &str, url: &str) -> Result<(), CatalogError> {
    plain(id, field, url, false)?;
    if !url.starts_with("https://") || url.chars().any(char::is_whitespace) || url.len() <= 8 {
        return invalid(
            id,
            format!("{field} must be an https:// URL (plain http is not accepted), not {url:?}"),
        );
    }
    Ok(())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
}

/// A coverage box must be `[west, south, east, north]` in degrees, with the west before the east.
fn check_coverage(id: &str, coverage: Option<[f64; 4]>) -> Result<(), CatalogError> {
    let Some([west, south, east, north]) = coverage else {
        return Ok(());
    };
    let sane = [west, south, east, north].iter().all(|v| v.is_finite())
        && (-180.0..=180.0).contains(&west)
        && (-180.0..=180.0).contains(&east)
        && (-90.0..=90.0).contains(&south)
        && (-90.0..=90.0).contains(&north)
        && west < east
        && south < north;
    if sane {
        Ok(())
    } else {
        invalid(
            id,
            "coverage must be [west, south, east, north] in degrees with west < east and south < north",
        )
    }
}

fn convert(raw: RawSource) -> Result<CatalogSource, CatalogError> {
    let id = raw.id.as_str();
    if !valid_id(id) {
        return invalid(
            id,
            "id must be lower-case letters, digits and - (at most 64, not starting with -)",
        );
    }
    plain(id, "name", &raw.name, false)?;
    plain(id, "licence", &raw.licence, false)?;
    plain(id, "attribution", &raw.attribution, false)?;
    plain(id, "note", &raw.note, true)?;
    https(id, "url", &raw.url)?;
    if !(raw.native_m.is_finite() && raw.native_m > 0.0) {
        return invalid(id, "native_m must be positive and finite");
    }
    check_coverage(id, raw.coverage)?;
    let says_nc = raw
        .licence
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word.eq_ignore_ascii_case("nc"));
    if says_nc != raw.non_commercial {
        return invalid(
            id,
            format!(
                "non_commercial must be {says_nc}: the licence {:?} {} NC",
                raw.licence,
                if says_nc { "says" } else { "does not say" }
            ),
        );
    }
    let kind = match raw.kind.as_str() {
        "stac" => {
            let (Some(collection), Some(asset)) = (raw.collection, raw.asset) else {
                return invalid(id, "a stac source needs collection and asset");
            };
            plain(id, "collection", &collection, false)?;
            plain(id, "asset", &asset, false)?;
            if raw.asset_hosts.is_empty() {
                return invalid(
                    id,
                    "a stac source needs asset_hosts: the hosts its files may come from",
                );
            }
            for host in &raw.asset_hosts {
                plain(id, "asset_hosts", host, false)?;
                if host.contains(['/', ':', '@', ' ']) {
                    return invalid(
                        id,
                        format!("asset_hosts entry {host:?} must be a host name"),
                    );
                }
            }
            SourceKind::Stac {
                url: raw.url,
                collection,
                asset,
                asset_hosts: raw.asset_hosts,
            }
        }
        "wms" => {
            let Some(layer) = raw.layer else {
                return invalid(id, "a wms source needs a layer");
            };
            plain(id, "layer", &layer, false)?;
            let max_px = raw.max_px.unwrap_or(2048);
            if !(64..=8192).contains(&max_px) {
                return invalid(id, "max_px must be in 64..=8192");
            }
            SourceKind::Wms {
                url: raw.url,
                layer,
                max_px,
                blank: raw.blank,
            }
        }
        other => return invalid(id, format!("kind must be stac or wms, not {other:?}")),
    };
    Ok(CatalogSource {
        id: raw.id,
        name: raw.name,
        kind,
        coverage: raw.coverage,
        native_m: raw.native_m,
        licence: raw.licence,
        attribution: raw.attribution,
        non_commercial: raw.non_commercial,
        note: raw.note,
    })
}

/// Parses and validates the text of a catalogue file (the entry point of the fuzz target).
///
/// # Errors
///
/// [`CatalogError::Parse`] for text that is not a catalogue, [`CatalogError::Invalid`] naming the first
/// entry that is not acceptable (a duplicate id, a URL that is not `https://`, a non-commercial licence that
/// is not marked, ...).
pub fn parse_sources(text: &str) -> Result<Vec<CatalogSource>, CatalogError> {
    let raw: RawFile = toml::from_str(text).map_err(|e| CatalogError::Parse(e.to_string()))?;
    let mut out: Vec<CatalogSource> = Vec::new();
    for source in raw.source {
        let converted = convert(source)?;
        if out.iter().any(|s| s.id == converted.id) {
            return invalid(&converted.id, "duplicate id in the same file");
        }
        out.push(converted);
    }
    Ok(out)
}

/// The place of the user's catalogue file: `imagery-sources.toml` in the `naves` folder of the configuration
/// folder (`$XDG_CONFIG_HOME`, else `~/.config`, else `%APPDATA%`).
#[must_use]
pub fn user_catalog_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })
        .or_else(|| {
            std::env::var_os("APPDATA")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })?;
    Some(base.join("naves").join("imagery-sources.toml"))
}

/// The built-in sources, then those of `user_file` (if it exists); an entry of the user file with the id of a
/// built-in one replaces it.
///
/// # Errors
///
/// [`CatalogError`] if the user file cannot be read or is not acceptable; the built-in list is part of the
/// tests and does not fail.
pub fn catalog(user_file: Option<&Path>) -> Result<Vec<CatalogSource>, CatalogError> {
    let mut sources = parse_sources(BUILTIN)?;
    let Some(path) = user_file else {
        return Ok(sources);
    };
    let io = |why: String| CatalogError::Io {
        path: path.to_path_buf(),
        why,
    };
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(sources),
        Err(e) => return Err(io(e.to_string())),
    };
    if meta.len() > MAX_FILE_BYTES {
        return Err(io(format!("larger than {MAX_FILE_BYTES} bytes")));
    }
    let text = std::fs::read_to_string(path).map_err(|e| io(e.to_string()))?;
    for user in parse_sources(&text)? {
        match sources.iter_mut().find(|s| s.id == user.id) {
            Some(slot) => *slot = user,
            None => sources.push(user),
        }
    }
    Ok(sources)
}

/// The share of `window` that the coverage box overlaps; 0 for a window without area.
fn share_of(coverage: Option<[f64; 4]>, window: GeoWindow) -> f64 {
    let area = (window.north - window.south) * (window.east - window.west);
    if !(area.is_finite() && area > 0.0) {
        return 0.0;
    }
    let Some([west, south, east, north]) = coverage else {
        return 1.0;
    };
    let across = (window.east.min(east) - window.west.max(west)).max(0.0);
    let up = (window.north.min(north) - window.south.max(south)).max(0.0);
    (across * up / area).clamp(0.0, 1.0)
}

/// The sources that have something for `window`, best first: those that cover the whole map, then those that
/// allow commercial use, then the sharper ones, then by id. No source without any overlap.
#[must_use]
pub fn options_for(sources: &[CatalogSource], window: GeoWindow) -> Vec<ImageryOption> {
    let mut options: Vec<ImageryOption> = sources
        .iter()
        .filter_map(|source| {
            let coverage_share = share_of(source.coverage, window);
            (coverage_share > 0.0).then(|| ImageryOption {
                source: source.clone(),
                coverage_share,
            })
        })
        .collect();
    options.sort_by(|a, b| {
        let partial = |o: &ImageryOption| o.coverage_share < WHOLE_MAP;
        partial(a)
            .cmp(&partial(b))
            .then(a.source.non_commercial.cmp(&b.source.non_commercial))
            .then(a.source.native_m.total_cmp(&b.source.native_m))
            .then(a.source.id.cmp(&b.source.id))
    });
    options
}

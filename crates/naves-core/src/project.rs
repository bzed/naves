// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The project file `naves.toml`.

use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ident::world_ident;

/// File name of the project file inside a project directory.
pub const PROJECT_FILE: &str = "naves.toml";

/// A Náves project.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Project {
    /// Display name.
    pub name: String,
    /// ASCII world identifier (class name, PBO prefix); matches `^[a-z][a-z0-9_]*$`.
    pub world: String,
    /// The real-world area.
    pub area: Area,
    /// Scale factors from the real world to the map.
    pub scale: Scale,
    /// Heightmap and material grid sizes.
    pub grid: Grid,
    /// Input data.
    pub sources: Sources,
    /// How the satellite images are drawn.
    #[serde(default, skip_serializing_if = "ImagerySettings::is_default")]
    pub imagery: ImagerySettings,
}

/// The brightness factor applied to imagery when a project does not set one.
pub const DEFAULT_BRIGHTNESS: f64 = 1.3;

/// How imagery is drawn into the satellite images.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ImagerySettings {
    /// Factor on the image colours, `0.25..=4`. Satellite images are dark for a terrain.
    #[serde(default = "default_brightness")]
    pub brightness: f64,
}

fn default_brightness() -> f64 {
    DEFAULT_BRIGHTNESS
}

impl Default for ImagerySettings {
    fn default() -> Self {
        ImagerySettings {
            brightness: DEFAULT_BRIGHTNESS,
        }
    }
}

impl ImagerySettings {
    fn is_default(&self) -> bool {
        *self == ImagerySettings::default()
    }
}

/// One imagery GeoTIFF of the project and the terms it comes with.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ImageryFile {
    /// The file, relative to the project directory.
    pub path: PathBuf,
    /// The credit line the source asks for; it is written into the finished map.
    pub attribution: String,
    /// The licence in words, for example `CC BY 4.0`.
    pub licence: String,
    /// The licence does not allow commercial use; the finished map says so.
    #[serde(default)]
    pub non_commercial: bool,
}

/// A square real-world area.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Area {
    /// Centre latitude in degrees (WGS84).
    pub center_lat: f64,
    /// Centre longitude in degrees (WGS84).
    pub center_lon: f64,
    /// Side length of the square in real metres.
    pub real_extent_m: f64,
}

/// Scale factors.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Scale {
    /// Map metres per real metre, `0 < horizontal <= 1`.
    pub horizontal: f64,
    /// Height multiplier, `0 < height <= 2`.
    pub height: f64,
}

/// Terrain cells per land cell. The game client needs 8 (Livonia and Chernarus have it): with fewer it
/// overflows a fixed 128-height buffer while it builds the land segments and crashes.
pub const LAND_RATIO: u32 = 8;

/// Grid sizes.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Grid {
    /// Heightmap vertices per side (power of two, 256..=8192).
    pub terrain_size: u32,
    /// Land (material) grid cells per side: always `terrain_size / LAND_RATIO`, as on Livonia.
    pub land_size: u32,
}

/// Input data files.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Sources {
    /// DEM GeoTIFF files, relative to the project directory.
    pub dem: Vec<PathBuf>,
    /// ESA `WorldCover` GeoTIFF files (land cover classes), relative to the project directory.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub land_cover: Vec<PathBuf>,
    /// OpenStreetMap `.osm.pbf` extracts, relative to the project directory.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub osm: Vec<PathBuf>,
    /// Satellite or aerial images (GeoTIFF) with their licences, in priority order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imagery: Vec<ImageryFile>,
}

/// Errors loading, validating or saving a project.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    /// Reading or writing the file failed.
    #[error("{path}: {source}")]
    Io {
        /// The file concerned.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The file is not valid TOML for a project.
    #[error("cannot parse project file: {0}")]
    Parse(#[from] toml::de::Error),
    /// The project cannot be written as TOML.
    #[error("cannot serialise project: {0}")]
    Serialize(#[from] toml::ser::Error),
    /// A field is out of range; the message names it.
    #[error("invalid project: {0}")]
    Invalid(String),
}

impl ProjectError {
    fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// Gives the temporary file (private by default) the mode of the file it replaces, or an ordinary mode for a new one.
#[cfg(unix)]
fn keep_file_mode(tmp: &std::fs::File, path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let permissions = match std::fs::metadata(path) {
        Ok(old) => old.permissions(),
        Err(_) => std::fs::Permissions::from_mode(0o644), // no old file: nothing to keep
    };
    tmp.set_permissions(permissions)
}

#[cfg(not(unix))]
fn keep_file_mode(_tmp: &std::fs::File, _path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn invalid<T>(msg: impl Into<String>) -> Result<T, ProjectError> {
    Err(ProjectError::Invalid(msg.into()))
}

fn is_world_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl Project {
    /// A project with the default scale and grid and no DEM files.
    #[must_use]
    pub fn new(name: &str, center_lat: f64, center_lon: f64, real_extent_m: f64) -> Project {
        Project {
            name: name.to_string(),
            world: world_ident(name),
            area: Area {
                center_lat,
                center_lon,
                real_extent_m,
            },
            scale: Scale {
                horizontal: 0.5,
                height: 0.5,
            },
            grid: Grid {
                terrain_size: 1024,
                land_size: 1024 / LAND_RATIO,
            },
            sources: Sources {
                dem: Vec::new(),
                land_cover: Vec::new(),
                osm: Vec::new(),
                imagery: Vec::new(),
            },
            imagery: ImagerySettings::default(),
        }
    }

    /// Reads `<dir>/naves.toml` and validates it.
    ///
    /// # Errors
    ///
    /// [`ProjectError::Io`] if the file cannot be read, otherwise as [`Project::parse`].
    pub fn load(dir: &Path) -> Result<Project, ProjectError> {
        let path = dir.join(PROJECT_FILE);
        let text = std::fs::read_to_string(&path).map_err(|e| ProjectError::io(&path, e))?;
        Project::parse(&text)
    }

    /// Parses and validates project TOML.
    ///
    /// # Errors
    ///
    /// [`ProjectError::Parse`] for malformed input, [`ProjectError::Invalid`] (naming the field)
    /// for out-of-range values.
    pub fn parse(toml: &str) -> Result<Project, ProjectError> {
        let project: Project = toml::from_str(toml)?;
        project.validate()?;
        Ok(project)
    }

    /// Writes `<dir>/naves.toml` atomically (temporary file in `dir`, then rename).
    ///
    /// # Errors
    ///
    /// [`ProjectError::Invalid`] if the project does not validate, [`ProjectError::Io`] if writing
    /// fails.
    pub fn save(&self, dir: &Path) -> Result<(), ProjectError> {
        self.validate()?;
        let text = toml::to_string_pretty(self)?;
        let path = dir.join(PROJECT_FILE);
        let mut tmp = tempfile::Builder::new()
            .prefix(".naves-")
            .suffix(".tmp")
            .tempfile_in(dir)
            .map_err(|e| ProjectError::io(dir, e))?;
        tmp.write_all(text.as_bytes())
            .and_then(|()| tmp.flush())
            .and_then(|()| keep_file_mode(tmp.as_file(), &path))
            .and_then(|()| tmp.as_file().sync_all())
            .map_err(|e| ProjectError::io(tmp.path(), e))?;
        tmp.persist(&path)
            .map_err(|e| ProjectError::io(&path, e.error))?;
        Ok(())
    }

    /// Side length of the map in metres: `real_extent_m * scale.horizontal`.
    #[must_use]
    pub fn map_size_m(&self) -> f64 {
        self.area.real_extent_m * self.scale.horizontal
    }

    /// Distance between heightmap vertices in metres.
    #[must_use]
    pub fn terrain_cell_m(&self) -> f64 {
        self.map_size_m() / f64::from(self.grid.terrain_size)
    }

    /// A copy of the project with the field at the dotted `path` (for example `scale.height`) set to
    /// `value`, validated like [`Project::parse`]. Only existing fields can be set. This is the entry
    /// point for `project_set` requests of the MCP server, so `path` and `value` are untrusted.
    ///
    /// # Errors
    ///
    /// [`ProjectError::Invalid`] naming the path for an unknown field, a value of the wrong type or an
    /// out-of-range value.
    pub fn with_value(
        &self,
        path: &str,
        value: serde_json::Value,
    ) -> Result<Project, ProjectError> {
        let bad = |why: &str| ProjectError::Invalid(format!("{path}: {why}"));
        let mut document = serde_json::to_value(self).map_err(|e| bad(&e.to_string()))?;
        // Fields that are left out of the file while empty or default can be set all the same.
        if let Some(root) = document.as_object_mut() {
            root.entry("imagery")
                .or_insert_with(|| serde_json::json!({ "brightness": DEFAULT_BRIGHTNESS }));
            if let Some(sources) = root.get_mut("sources").and_then(|s| s.as_object_mut()) {
                for key in ["land_cover", "osm", "imagery"] {
                    sources.entry(key).or_insert_with(|| serde_json::json!([]));
                }
            }
        }
        let mut node = &mut document;
        let mut keys = path.split('.').peekable();
        if path.is_empty() {
            return Err(bad("empty path"));
        }
        while let Some(key) = keys.next() {
            let serde_json::Value::Object(map) = node else {
                return Err(bad("not a section of the project"));
            };
            let Some(next) = map.get_mut(key) else {
                return Err(bad(&format!("unknown field {key:?}")));
            };
            if keys.peek().is_none() {
                if next.is_object() != value.is_object() {
                    return Err(bad("a whole section cannot be replaced by a single value"));
                }
                *next = value;
                break;
            }
            node = next;
        }
        let mut project: Project =
            serde_json::from_value(document).map_err(|e| bad(&e.to_string()))?;
        if path == "grid.terrain_size" {
            // the land grid is not a free choice, it follows the terrain grid
            project.grid.land_size = project.grid.terrain_size / LAND_RATIO;
        }
        project.validate()?;
        Ok(project)
    }

    fn validate(&self) -> Result<(), ProjectError> {
        let g = &self.grid;
        if !(256..=8192).contains(&g.terrain_size) || !g.terrain_size.is_power_of_two() {
            return invalid("grid.terrain_size must be a power of two in 256..=8192");
        }
        if g.land_size.checked_mul(LAND_RATIO) != Some(g.terrain_size) {
            return Err(ProjectError::Invalid(format!(
                "grid.land_size must be grid.terrain_size / {LAND_RATIO} (DayZ needs {LAND_RATIO} terrain cells per \
                 land cell); for terrain_size = {} use land_size = {}",
                g.terrain_size,
                g.terrain_size / LAND_RATIO
            )));
        }
        let s = &self.scale;
        if !(s.horizontal > 0.0 && s.horizontal <= 1.0) {
            return invalid("scale.horizontal must be in (0, 1]");
        }
        if !(s.height > 0.0 && s.height <= 2.0) {
            return invalid("scale.height must be in (0, 2]");
        }
        if !is_world_ident(&self.world) {
            return invalid("world must match ^[a-z][a-z0-9_]*$");
        }
        let a = &self.area;
        if !(-90.0..=90.0).contains(&a.center_lat) {
            return invalid("area.center_lat must be in -90..=90");
        }
        if !(-180.0..=180.0).contains(&a.center_lon) {
            return invalid("area.center_lon must be in -180..=180");
        }
        if !(a.real_extent_m.is_finite() && a.real_extent_m > 0.0) {
            return invalid("area.real_extent_m must be positive and finite");
        }
        let src = &self.sources;
        for (name, list) in [
            ("dem", &src.dem),
            ("land_cover", &src.land_cover),
            ("osm", &src.osm),
        ] {
            for p in list {
                check_inside_project(name, p)?;
            }
        }
        for file in &src.imagery {
            check_inside_project("imagery", &file.path)?;
            file.validate()?;
        }
        let b = self.imagery.brightness;
        if !(b.is_finite() && (0.25..=4.0).contains(&b)) {
            return invalid("imagery.brightness must be in 0.25..=4");
        }
        Ok(())
    }
}

/// The longest credit line; it ends up in `mod.cpp` and `ATTRIBUTION.txt`.
const MAX_ATTRIBUTION_CHARS: usize = 500;

impl ImageryFile {
    fn validate(&self) -> Result<(), ProjectError> {
        let one_line = |field: &str, text: &str| -> Result<(), ProjectError> {
            if text.trim().is_empty() {
                return invalid(format!("sources.imagery {field} must not be empty"));
            }
            if text.chars().any(char::is_control) {
                return invalid(format!(
                    "sources.imagery {field} must be one line without control characters"
                ));
            }
            if text.chars().count() > MAX_ATTRIBUTION_CHARS {
                return invalid(format!(
                    "sources.imagery {field} is longer than {MAX_ATTRIBUTION_CHARS} characters"
                ));
            }
            Ok(())
        };
        one_line("attribution", &self.attribution)?;
        one_line("licence", &self.licence)?;
        let says_nc = self
            .licence
            .split(|c: char| !c.is_alphanumeric())
            .any(|token| token.eq_ignore_ascii_case("nc"));
        if says_nc && !self.non_commercial {
            return invalid(format!(
                "sources.imagery non_commercial must be true for {}: its licence {:?} says NC",
                self.path.display(),
                self.licence
            ));
        }
        Ok(())
    }
}

fn check_inside_project(name: &str, p: &Path) -> Result<(), ProjectError> {
    // A rooted path ("/etc/x") is not absolute on Windows (no drive), so check the root too.
    let escapes = p.is_absolute()
        || p.has_root()
        || p.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::Prefix(_) | Component::RootDir
            )
        });
    if escapes || p.as_os_str().is_empty() {
        return invalid(format!(
            "sources.{name} entries must be relative paths inside the project"
        ));
    }
    Ok(())
}

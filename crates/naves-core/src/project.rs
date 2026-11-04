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
}

/// A square real-world area.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
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
pub struct Scale {
    /// Map metres per real metre, `0 < horizontal <= 1`.
    pub horizontal: f64,
    /// Height multiplier, `0 < height <= 2`.
    pub height: f64,
}

/// Grid sizes.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Grid {
    /// Heightmap vertices per side (power of two, 256..=8192).
    pub terrain_size: u32,
    /// Material grid cells per side (power of two dividing `terrain_size`).
    pub land_size: u32,
}

/// Input data files.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Sources {
    /// DEM GeoTIFF files, relative to the project directory.
    pub dem: Vec<PathBuf>,
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
                land_size: 256,
            },
            sources: Sources { dem: Vec::new() },
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

    fn validate(&self) -> Result<(), ProjectError> {
        let g = &self.grid;
        if !(256..=8192).contains(&g.terrain_size) || !g.terrain_size.is_power_of_two() {
            return invalid("grid.terrain_size must be a power of two in 256..=8192");
        }
        if !(32..=4096).contains(&g.land_size)
            || !g.land_size.is_power_of_two()
            || g.land_size > g.terrain_size
            || !g.terrain_size.is_multiple_of(g.land_size)
        {
            return invalid(
                "grid.land_size must be a power of two in 32..=4096 that divides grid.terrain_size",
            );
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
        for p in &self.sources.dem {
            let escapes = p.is_absolute()
                || p.components()
                    .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)));
            if escapes || p.as_os_str().is_empty() {
                return invalid("sources.dem entries must be relative paths inside the project");
            }
        }
        Ok(())
    }
}

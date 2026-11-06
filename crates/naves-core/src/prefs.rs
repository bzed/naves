// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The per-user settings: `settings.toml` in Náves' configuration folder.
//!
//! They belong to the person, not to the map, so they never go into `naves.toml`. Only what differs from the
//! default is written, so that a later version's new defaults still reach you. Keys and shortcut ids this version
//! does not know are kept when the file is saved, for a file shared between versions. A damaged file is an
//! error and is never overwritten by the program that failed to read it.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Smallest and largest interface scale.
const SCALE_RANGE: (f32, f32) = (0.5, 3.0);

/// Why the settings cannot be read or written.
#[derive(Debug, thiserror::Error)]
pub enum PrefsError {
    /// The file cannot be read or written.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file or folder.
        path: PathBuf,
        /// What went wrong.
        source: std::io::Error,
    },
    /// The file is not valid TOML or has a value of the wrong type.
    #[error("{}: {message}", path.display())]
    Syntax {
        /// The file.
        path: PathBuf,
        /// What is wrong, with the line.
        message: String,
    },
    /// The settings cannot be written as TOML.
    #[error("cannot write the settings: {0}")]
    Write(String),
}

/// The folder of Náves' per-user files: `naves` in `$XDG_CONFIG_HOME`, else `~/.config`, else `%APPDATA%`.
#[must_use]
pub fn config_dir() -> Option<PathBuf> {
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
    Some(base.join("naves"))
}

/// The settings file in the configuration folder `config_dir` (see [`config_dir`]).
#[must_use]
pub fn default_path(config_dir: &Path) -> PathBuf {
    config_dir.join("settings.toml")
}

/// The settings of a person.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// Shortcuts by stable id, as text such as `Ctrl+B`; `""` is no key. Only what differs from the default.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub shortcuts: BTreeMap<String, String>,
    /// How the map behaves.
    #[serde(skip_serializing_if = "MapPrefs::is_default")]
    pub map: MapPrefs,
    /// Where the external tools are.
    #[serde(skip_serializing_if = "ToolPaths::is_default")]
    pub tools: ToolPaths,
    /// Tables and keys of a newer version.
    #[serde(flatten)]
    pub other: BTreeMap<String, toml::Value>,
}

/// The Map tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MapPrefs {
    /// The wheel zooms the other way.
    #[serde(skip_serializing_if = "is_false")]
    pub invert_zoom: bool,
    /// How strongly the levels you are not working on are drawn, in percent.
    #[serde(skip_serializing_if = "is_default_fade")]
    pub fade_percent: u8,
    /// The scale of the whole interface.
    #[serde(skip_serializing_if = "is_default_scale")]
    pub interface_scale: f32,
    /// The stack unfolds without animation.
    #[serde(skip_serializing_if = "is_false")]
    pub no_unfold_animation: bool,
    /// The base map a project opens with: `hillshade`, `slope`, `height`, `surface` or `imagery`.
    #[serde(skip_serializing_if = "is_default_base")]
    pub start_base: String,
    /// The map layers a project opens with, by id.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub start_layers: Vec<String>,
    /// Keys of a newer version.
    #[serde(flatten)]
    pub other: BTreeMap<String, toml::Value>,
}

impl Default for MapPrefs {
    fn default() -> Self {
        Self {
            invert_zoom: false,
            fade_percent: 40,
            interface_scale: 1.0,
            no_unfold_animation: false,
            start_base: "hillshade".to_string(),
            start_layers: Vec::new(),
            other: BTreeMap::new(),
        }
    }
}

impl MapPrefs {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The External tools tab: a path here is used when the environment variable of the tool is not set.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolPaths {
    /// bzed's armake2 fork.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub armake2: Option<PathBuf>,
    /// The Steam folder (the one with `steamapps/`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steam_root: Option<PathBuf>,
    /// DayZ Tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dayz_tools: Option<PathBuf>,
    /// The DayZ game.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dayz: Option<PathBuf>,
    /// DayZ Server, which the mission is built from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dayz_server: Option<PathBuf>,
    /// The `proton` script.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proton: Option<PathBuf>,
    /// Keys of a newer version.
    #[serde(flatten)]
    pub other: BTreeMap<String, toml::Value>,
}

impl ToolPaths {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_false(v: &bool) -> bool {
    !*v
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_default_fade(v: &u8) -> bool {
    *v == MapPrefs::default().fade_percent
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_default_scale(v: &f32) -> bool {
    (*v - 1.0).abs() < f32::EPSILON
}

fn is_default_base(v: &String) -> bool {
    *v == MapPrefs::default().start_base
}

impl Preferences {
    /// Reads the file; a missing file gives the defaults.
    ///
    /// # Errors
    ///
    /// [`PrefsError::Io`] if the file exists but cannot be read, [`PrefsError::Syntax`] if it is damaged.
    pub fn load(path: &Path) -> Result<Preferences, PrefsError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Preferences::default()),
            Err(source) => {
                return Err(PrefsError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        Self::parse(&text).map_err(|e| PrefsError::Syntax {
            path: path.to_path_buf(),
            message: e.to_string(),
        })
    }

    /// Reads settings from text. Out-of-range values are clamped.
    ///
    /// # Errors
    ///
    /// The TOML error for a damaged file or a value of the wrong type.
    pub fn parse(text: &str) -> Result<Preferences, toml::de::Error> {
        let mut prefs: Preferences = toml::from_str(text)?;
        prefs.clamp();
        Ok(prefs)
    }

    fn clamp(&mut self) {
        self.map.fade_percent = self.map.fade_percent.min(100);
        let scale = self.map.interface_scale;
        self.map.interface_scale = if scale.is_nan() {
            1.0
        } else {
            scale.clamp(SCALE_RANGE.0, SCALE_RANGE.1)
        };
    }

    /// Writes only what differs from the default, atomically (a temporary file in the same folder, then a rename),
    /// creating the folder. Keys this version does not know are written back.
    ///
    /// # Errors
    ///
    /// [`PrefsError::Io`] if the folder or file cannot be written; the old file is then still there.
    pub fn save(&self, path: &Path) -> Result<(), PrefsError> {
        let mut clamped = self.clone();
        clamped.clamp();
        let text =
            toml::to_string_pretty(&clamped).map_err(|e| PrefsError::Write(e.to_string()))?;
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source| PrefsError::Io { path, source }
        };
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir).map_err(io(dir))?;
        let mut tmp = tempfile::Builder::new()
            .prefix(".settings-")
            .suffix(".tmp")
            .tempfile_in(dir)
            .map_err(io(dir))?;
        tmp.write_all(text.as_bytes())
            .and_then(|()| tmp.flush())
            .and_then(|()| tmp.as_file().sync_all())
            .map_err(io(tmp.path()))?;
        tmp.persist(path).map_err(|e| PrefsError::Io {
            path: path.to_path_buf(),
            source: e.error,
        })?;
        Ok(())
    }
}

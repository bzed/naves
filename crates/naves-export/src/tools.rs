// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The external tools Náves drives: bzed's armake2 fork and DayZ Tools' `binarize.exe` under Proton.
//!
//! All tools are located up front, so that a missing one is reported by name, with the environment
//! variable that fixes it, before any long work starts.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// How long `binarize.exe` may run before it is stopped.
const DEFAULT_BINARIZE_TIMEOUT: Duration = Duration::from_mins(30);

/// Errors from locating or running the external tools.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// A tool or folder was not found.
    #[error("{tool} not found; set {env} (hint: {hint})")]
    Missing {
        /// Name of the tool.
        tool: &'static str,
        /// Environment variable that points at it.
        env: &'static str,
        /// Where to get it.
        hint: &'static str,
    },
    /// The `armake2` found is the upstream one.
    #[error(
        "armake2 is not bzed's fork (no --proton-binarize); install https://github.com/bzed/armake2"
    )]
    WrongArmake2,
    /// `binarize.exe` exited with an error.
    #[error("binarize failed (exit {code}); log: {}; sandbox kept at {}", log.display(), sandbox.display())]
    Binarize {
        /// Exit code.
        code: i32,
        /// The binarize log.
        log: PathBuf,
        /// The sandbox, kept for inspection.
        sandbox: PathBuf,
    },
    /// `binarize.exe` finished but produced no `OPRW` world.
    #[error("binarize produced no OPRW world; log: {}; sandbox kept at {}", log.display(), sandbox.display())]
    NoWorld {
        /// The binarize log.
        log: PathBuf,
        /// The sandbox, kept for inspection.
        sandbox: PathBuf,
    },
    /// A tool did not finish in time and was stopped.
    #[error("{tool} did not finish within {secs} s; log: {}; sandbox kept at {}", log.display(), sandbox.display())]
    Timeout {
        /// The tool.
        tool: &'static str,
        /// The limit in seconds.
        secs: u64,
        /// The log so far.
        log: PathBuf,
        /// The sandbox, kept for inspection.
        sandbox: PathBuf,
    },
    /// A world name or prefix contains characters that must not reach the Wine command line.
    #[error("not a valid {0}")]
    BadName(String),
    /// A file operation failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file or folder concerned.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
}

impl ToolError {
    pub(crate) fn io(path: &Path) -> impl FnOnce(std::io::Error) -> ToolError + '_ {
        move |source| ToolError::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// The installed tools.
#[derive(Debug, Clone)]
pub struct Tools {
    /// bzed's armake2 fork.
    pub armake2: PathBuf,
    /// `binarize.exe` through Proton.
    pub binarize: BinarizeBridge,
}

/// Where DayZ Tools and Proton live, and how binarize is run.
#[derive(Debug, Clone)]
pub struct BinarizeBridge {
    /// The Steam root (`STEAM_COMPAT_CLIENT_INSTALL_PATH`).
    pub steam_root: PathBuf,
    /// The DayZ Tools install.
    pub dayz_tools: PathBuf,
    /// The DayZ game install (read-only).
    pub dayz: PathBuf,
    /// The `proton` script.
    pub proton: PathBuf,
    /// Folder in which the sandbox is created (`/var/tmp` by default).
    pub sandbox_base: PathBuf,
    /// Limit for one binarize run.
    pub timeout: Duration,
}

fn missing(tool: &'static str, env: &'static str, hint: &'static str) -> ToolError {
    ToolError::Missing { tool, env, hint }
}

impl Tools {
    /// Locates the tools from the process environment (`ARMAKE2`, `STEAM_ROOT`, `DAYZ_TOOLS`, `DAYZ`,
    /// `PROTON`, `NAVES_BINARIZE_TIMEOUT`), `PATH` and the Steam folders under `$HOME`.
    ///
    /// # Errors
    ///
    /// [`ToolError::Missing`] naming the first missing tool and the variable to set, or
    /// [`ToolError::WrongArmake2`].
    pub fn locate() -> Result<Tools, ToolError> {
        Self::locate_with(
            &|key| std::env::var_os(key),
            std::env::var_os("PATH"),
            std::env::var_os("HOME").map(PathBuf::from),
        )
    }

    /// [`Tools::locate`] with the environment, `PATH` and home folder passed in.
    ///
    /// # Errors
    ///
    /// As [`Tools::locate`].
    pub fn locate_with(
        env: &dyn Fn(&str) -> Option<OsString>,
        path: Option<OsString>,
        home: Option<PathBuf>,
    ) -> Result<Tools, ToolError> {
        let armake2 = locate_armake2(env, path)?;
        let steam_root = locate_steam(env, home)?;
        let common = steam_root.join("steamapps/common");
        let var = |key: &str| env(key).map(PathBuf::from);
        let dayz_tools = var("DAYZ_TOOLS").unwrap_or_else(|| common.join("DayZ Tools"));
        if !dayz_tools.join("Bin/Binarize").is_dir() {
            return Err(missing(
                "DayZ Tools",
                "DAYZ_TOOLS",
                "install DayZ Tools from Steam (Library, Tools)",
            ));
        }
        let proton = var("PROTON").unwrap_or_else(|| common.join("Proton Hotfix/proton"));
        if !proton.is_file() {
            return Err(missing(
                "Proton",
                "PROTON",
                "install Proton Hotfix or Proton Experimental from Steam, or point PROTON at a `proton` script",
            ));
        }
        let timeout = env("NAVES_BINARIZE_TIMEOUT")
            .and_then(|v| v.to_str().and_then(|s| s.parse::<u64>().ok()))
            .map_or(DEFAULT_BINARIZE_TIMEOUT, Duration::from_secs);
        Ok(Tools {
            armake2,
            binarize: BinarizeBridge {
                dayz: var("DAYZ").unwrap_or_else(|| common.join("DayZ")),
                dayz_tools,
                proton,
                steam_root,
                sandbox_base: default_sandbox_base(),
                timeout,
            },
        })
    }
}

fn default_sandbox_base() -> PathBuf {
    if cfg!(target_os = "linux") {
        PathBuf::from("/var/tmp")
    } else {
        std::env::temp_dir()
    }
}

fn locate_armake2(
    env: &dyn Fn(&str) -> Option<OsString>,
    path: Option<OsString>,
) -> Result<PathBuf, ToolError> {
    const HINT: &str = "install bzed's fork, https://github.com/bzed/armake2";
    let found = if let Some(explicit) = env("ARMAKE2") {
        let p = PathBuf::from(explicit);
        p.is_file().then_some(p)
    } else {
        let cwd = std::env::current_dir().unwrap_or_default();
        which::which_in("armake2", path, cwd).ok()
    };
    let armake2 = found.ok_or_else(|| missing("armake2", "ARMAKE2", HINT))?;
    if is_fork(&armake2)? {
        Ok(armake2)
    } else {
        Err(ToolError::WrongArmake2)
    }
}

/// bzed's fork is recognised by `--proton-binarize` in its help text.
fn is_fork(armake2: &Path) -> Result<bool, ToolError> {
    let output = Command::new(armake2)
        .arg("-h")
        .output()
        .map_err(ToolError::io(armake2))?;
    let text = [output.stdout, output.stderr].concat();
    Ok(String::from_utf8_lossy(&text).contains("--proton-binarize"))
}

fn locate_steam(
    env: &dyn Fn(&str) -> Option<OsString>,
    home: Option<PathBuf>,
) -> Result<PathBuf, ToolError> {
    const HINT: &str = "install Steam, or point STEAM_ROOT at its folder (the one with steamapps/)";
    if let Some(root) = env("STEAM_ROOT") {
        return Ok(PathBuf::from(root));
    }
    let candidates = home.into_iter().flat_map(|h| {
        [
            h.join(".steam/steam"),
            h.join(".local/share/Steam"),
            h.join(".steam/debian-installation"),
        ]
    });
    candidates
        .into_iter()
        .find(|c| c.join("steamapps").is_dir())
        .ok_or_else(|| missing("Steam", "STEAM_ROOT", HINT))
}

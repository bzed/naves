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

use naves_core::prefs::ToolPaths;

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
    /// A path in the settings does not lead to the tool.
    #[error(
        "{tool}: the path in the settings does not exist: {}; change it under Settings, External tools (tools.{setting} in settings.toml)",
        path.display()
    )]
    BadSetting {
        /// Name of the tool.
        tool: &'static str,
        /// The key in the `[tools]` table.
        setting: &'static str,
        /// The path that was set.
        path: PathBuf,
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
    /// The DayZ Server install (read-only); the mission is built from its `mpmissions`.
    pub dayz_server: PathBuf,
    /// Where Náves keeps what it learns from the game files (the catalog of road pieces): per user, never
    /// part of a project and never distributed.
    pub cache_dir: PathBuf,
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
        let cache_dir = cache_dir(env, home.as_deref());
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
        if proton_missing(&proton, cfg!(windows)) {
            return Err(missing(
                "Proton",
                "PROTON",
                "install Proton Hotfix or Proton Experimental from Steam, or point PROTON at a `proton` script",
            ));
        }
        let dayz_server = var("DAYZ_SERVER").unwrap_or_else(|| common.join("DayZServer"));
        if !dayz_server.join("mpmissions").is_dir() {
            return Err(missing(
                "DayZ Server",
                "DAYZ_SERVER",
                "install DayZ Server from Steam (Library, Tools); the mission is built from its dayzOffline.enoch",
            ));
        }
        let timeout = env("NAVES_BINARIZE_TIMEOUT")
            .and_then(|v| v.to_str().and_then(|s| s.parse::<u64>().ok()))
            .map_or(DEFAULT_BINARIZE_TIMEOUT, Duration::from_secs);
        // The tools run in other working directories, so relative paths have to be resolved here.
        Ok(Tools {
            armake2: absolute(armake2),
            dayz_server: absolute(dayz_server),
            cache_dir: absolute(cache_dir),
            binarize: BinarizeBridge {
                dayz: absolute(var("DAYZ").unwrap_or_else(|| common.join("DayZ"))),
                dayz_tools: absolute(dayz_tools),
                proton: absolute(proton),
                steam_root: absolute(steam_root),
                sandbox_base: default_sandbox_base(),
                timeout,
            },
        })
    }
}

/// Where a tool's path comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// An environment variable.
    Environment,
    /// The per-user settings.
    Settings,
    /// Found by searching `PATH` and the Steam folders.
    Found,
    /// Not found.
    Missing,
}

/// One tool, the path in use and where it comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOrigin {
    /// The tool's name, as in the error messages.
    pub tool: &'static str,
    /// The path in use; for a missing tool the path that was set, if any.
    pub path: Option<PathBuf>,
    /// Where it comes from.
    pub from: Origin,
}

/// The environment variable of each tool, its name in the settings and the name of the tool.
const TOOL_KEYS: [(&str, &str, &str); 6] = [
    ("ARMAKE2", "armake2", "armake2"),
    ("STEAM_ROOT", "steam_root", "Steam"),
    ("DAYZ_TOOLS", "dayz_tools", "DayZ Tools"),
    ("DAYZ", "dayz", "DayZ"),
    ("DAYZ_SERVER", "dayz_server", "DayZ Server"),
    ("PROTON", "proton", "Proton"),
];

fn setting<'a>(paths: &'a ToolPaths, var: &str) -> Option<&'a PathBuf> {
    match var {
        "ARMAKE2" => paths.armake2.as_ref(),
        "STEAM_ROOT" => paths.steam_root.as_ref(),
        "DAYZ_TOOLS" => paths.dayz_tools.as_ref(),
        "DAYZ" => paths.dayz.as_ref(),
        "DAYZ_SERVER" => paths.dayz_server.as_ref(),
        "PROTON" => paths.proton.as_ref(),
        _ => None,
    }
}

/// The environment first, then the settings: the lookup of [`Tools::locate_with`] sees both as one environment.
fn layered<'a>(
    paths: &'a ToolPaths,
    env: &'a dyn Fn(&str) -> Option<OsString>,
) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |key| env(key).or_else(|| setting(paths, key).map(|p| p.clone().into_os_string()))
}

/// [`Tools::locate`] with the settings between the environment and the search: an environment variable
/// wins over the settings, and the settings over the search.
///
/// # Errors
///
/// As [`Tools::locate`], and [`ToolError::BadSetting`] when the path in the settings does not exist.
pub fn locate_preferring(paths: &ToolPaths) -> Result<Tools, ToolError> {
    locate_preferring_with(
        paths,
        &|key| std::env::var_os(key),
        std::env::var_os("PATH"),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

/// [`locate_preferring`] with the environment, `PATH` and home folder passed in.
///
/// # Errors
///
/// As [`locate_preferring`].
pub fn locate_preferring_with(
    paths: &ToolPaths,
    env: &dyn Fn(&str) -> Option<OsString>,
    path: Option<OsString>,
    home: Option<PathBuf>,
) -> Result<Tools, ToolError> {
    // a Steam folder from the settings is not checked by the lookup itself, so a bad one would show up as a
    // missing DayZ Tools; say what is wrong instead
    if env("STEAM_ROOT").is_none()
        && let Some(root) = &paths.steam_root
        && !root.join("steamapps").is_dir()
    {
        return Err(ToolError::BadSetting {
            tool: "Steam",
            setting: "steam_root",
            path: root.clone(),
        });
    }
    Tools::locate_with(&layered(paths, env), path, home).map_err(|error| match error {
        ToolError::Missing { tool, env: var, .. } if env(var).is_none() => {
            match (setting(paths, var), TOOL_KEYS.iter().find(|k| k.0 == var)) {
                (Some(path), Some((_, key, _))) => ToolError::BadSetting {
                    tool,
                    setting: key,
                    path: path.clone(),
                },
                _ => error,
            }
        }
        other => other,
    })
}

/// Every tool with the path in use and where it comes from, for the settings window.
#[must_use]
pub fn tool_origins(paths: &ToolPaths) -> Vec<ToolOrigin> {
    tool_origins_with(
        paths,
        &|key| std::env::var_os(key),
        std::env::var_os("PATH"),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

/// [`tool_origins`] with the environment, `PATH` and home folder passed in.
#[must_use]
pub fn tool_origins_with(
    paths: &ToolPaths,
    env: &dyn Fn(&str) -> Option<OsString>,
    path: Option<OsString>,
    home: Option<PathBuf>,
) -> Vec<ToolOrigin> {
    let all = layered(paths, env);
    let armake2 = locate_armake2(&all, path).ok();
    let steam = locate_steam(&all, home).ok();
    let common = steam.as_ref().map(|s| s.join("steamapps/common"));
    // an explicit path, else the usual place below the Steam folder
    let chosen = |var: &str, below: &str| -> Option<PathBuf> {
        all(var)
            .map(PathBuf::from)
            .or_else(|| common.as_ref().map(|c| c.join(below)))
    };
    let proton = chosen("PROTON", "Proton Hotfix/proton");
    let found = |tool: &'static str, var: &str, path: Option<PathBuf>, ok: bool| {
        let given = env(var).is_some() || setting(paths, var).is_some();
        let from = match (ok, env(var).is_some(), given) {
            (false, _, _) => Origin::Missing,
            (true, true, _) => Origin::Environment,
            (true, false, true) => Origin::Settings,
            (true, false, false) => Origin::Found,
        };
        ToolOrigin {
            tool,
            path: if ok || given { path } else { None },
            from,
        }
    };
    let dayz_tools = chosen("DAYZ_TOOLS", "DayZ Tools");
    let dayz = chosen("DAYZ", "DayZ");
    let server = chosen("DAYZ_SERVER", "DayZServer");
    vec![
        found("armake2", "ARMAKE2", armake2.clone(), armake2.is_some()),
        found("Steam", "STEAM_ROOT", steam.clone(), steam.is_some()),
        found(
            "DayZ Tools",
            "DAYZ_TOOLS",
            dayz_tools.clone(),
            dayz_tools
                .as_ref()
                .is_some_and(|p| p.join("Bin/Binarize").is_dir()),
        ),
        found(
            "DayZ",
            "DAYZ",
            dayz.clone(),
            dayz.as_ref().is_some_and(|p| p.is_dir()),
        ),
        found(
            "DayZ Server",
            "DAYZ_SERVER",
            server.clone(),
            server
                .as_ref()
                .is_some_and(|p| p.join("mpmissions").is_dir()),
        ),
        found(
            "Proton",
            "PROTON",
            proton.clone(),
            proton
                .as_ref()
                .is_some_and(|p| !proton_missing(p, cfg!(windows))),
        ),
    ]
}

/// `path` made absolute; an unusable path stays as it is and fails with a named error when it is used.
fn absolute(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}

/// On Windows the DayZ Tools run directly, so only other systems need Proton.
fn proton_missing(proton: &Path, windows: bool) -> bool {
    !windows && !proton.is_file()
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
    let windows = env("ProgramFiles(x86)").map(|p| PathBuf::from(p).join("Steam"));
    candidates
        .chain(windows)
        .find(|c| c.join("steamapps").is_dir())
        .ok_or_else(|| missing("Steam", "STEAM_ROOT", HINT))
}

/// `naves` in `$XDG_CACHE_HOME`, else `~/.cache`, else the temporary folder.
fn cache_dir(env: &dyn Fn(&str) -> Option<OsString>, home: Option<&Path>) -> PathBuf {
    let base = env("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.map(|h| h.join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("naves")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proton_is_only_needed_off_windows() {
        let absent = Path::new("/nonexistent/proton");
        assert!(proton_missing(absent, false));
        assert!(!proton_missing(absent, true));
    }
}

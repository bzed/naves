// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Running BI's `binarize.exe` on a world: under Proton on Linux, directly on Windows.
//!
//! The recipe is in `docs/formats/binarize-world.md`: binarize runs in a sandbox that becomes Wine's `Z:`,
//! with the staging folder as `P:` and as the working directory. Everything the tool needs from DayZ
//! Tools is copied into the sandbox, which is deleted on success and kept (its path is in the error) on
//! failure. Nothing is written into Steam directories.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;

use wait_timeout::ChildExt;
use walkdir::WalkDir;

use crate::tools::{BinarizeBridge, ToolError};

/// Steam app id of DayZ Tools, which Proton uses to pick the prefix settings.
const DAYZ_TOOLS_APP_ID: &str = "830640";
/// How long the wineserver of a finished run may take to shut down.
const WINESERVER_WAIT: Duration = Duration::from_mins(1);

/// How long a spawn retries while the program file is open for writing elsewhere.
const BUSY_DEADLINE: Duration = Duration::from_secs(1);

/// Spawns `cmd`, retrying while the program file is still open for writing.
///
/// Linux can refuse to start a program that was written moments earlier with "text file busy": the
/// kernel releases a closed file's write permission only a moment after `close`. A tool that is
/// busy for longer than [`BUSY_DEADLINE`] reports the error of its last attempt.
pub(crate) fn spawn_retrying_busy(cmd: &mut Command) -> std::io::Result<Child> {
    let start = std::time::Instant::now();
    loop {
        match cmd.spawn() {
            Ok(child) => return Ok(child),
            Err(e)
                if e.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && start.elapsed() < BUSY_DEADLINE =>
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(e) => return Err(e),
        }
    }
}

/// The outcome of a supervised process.
#[derive(Debug)]
pub(crate) enum Finished {
    Exited(ExitStatus),
    TimedOut,
}

/// Runs `cmd` with its output in `log`, stopping it after `timeout`.
pub(crate) fn run_supervised(
    cmd: &mut Command,
    log: &Path,
    timeout: Duration,
) -> Result<Finished, ToolError> {
    let out = File::create(log).map_err(ToolError::io(log))?;
    let err = out.try_clone().map_err(ToolError::io(log))?;
    let mut child: Child = spawn_retrying_busy(
        cmd.stdin(Stdio::null())
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err)),
    )
    .map_err(ToolError::io(Path::new(cmd.get_program())))?;
    let waited = child
        .wait_timeout(timeout)
        .map_err(ToolError::io(Path::new(cmd.get_program())))?;
    if let Some(status) = waited {
        return Ok(Finished::Exited(status));
    }
    // The child is the Proton launcher; the wineserver of the sandbox is stopped by the caller.
    child.kill().ok(); // already exited is fine: all that matters is that it is gone
    child.wait().ok(); // reaps the process; its exit status is irrelevant after a kill
    Ok(Finished::TimedOut)
}

/// Only plain identifiers may reach the Wine command line.
fn check_name(what: &str, value: &str, allow_backslash: bool) -> Result<(), ToolError> {
    let ok = !value.is_empty()
        && !value.starts_with('\\')
        && !value.ends_with('\\')
        && !value.contains("\\\\")
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || (allow_backslash && c == '\\'));
    if ok {
        Ok(())
    } else {
        Err(ToolError::BadName(what.to_string()))
    }
}

#[cfg(unix)]
fn link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn link(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Wine drive links need a Unix system",
    ))
}

pub(crate) fn copy_tree(from: &Path, to: &Path) -> Result<(), ToolError> {
    for entry in WalkDir::new(from).sort_by_file_name() {
        let entry = entry.map_err(|e| ToolError::Io {
            path: from.to_path_buf(),
            source: e.into(),
        })?;
        let relative = entry.path().strip_prefix(from).unwrap_or(entry.path());
        let dest = to.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest).map_err(ToolError::io(&dest))?;
        } else {
            std::fs::copy(entry.path(), &dest).map_err(ToolError::io(&dest))?;
        }
    }
    Ok(())
}

impl BinarizeBridge {
    /// Binarizes the world `world` (an 8WVR plus `config.cpp`) found at `<staging_p>\<prefix>` into
    /// `out_dir/<world>.wrp`, and keeps the tool's log as `out_dir/binarize.log`.
    ///
    /// `staging_p` is a folder laid out as the `P:` drive (the world below its in-game prefix, for
    /// example `naves\worlds\berchtesgaden`). Returns the path of the `OPRW` world.
    ///
    /// # Errors
    ///
    /// [`ToolError::BadName`] for a prefix or world name that is not a plain identifier,
    /// [`ToolError::Timeout`] if the tool exceeds [`BinarizeBridge::timeout`],
    /// [`ToolError::Binarize`] for a non-zero exit, [`ToolError::NoWorld`] if no `OPRW` file came out,
    /// [`ToolError::Io`] for file errors. After a failure the sandbox is kept and named in the error.
    pub fn binarize_world(
        &self,
        staging_p: &Path,
        prefix: &str,
        world: &str,
        out_dir: &Path,
    ) -> Result<PathBuf, ToolError> {
        check_name("prefix", prefix, true)?;
        check_name("world name", world, false)?;
        std::fs::create_dir_all(out_dir).map_err(ToolError::io(out_dir))?;
        if cfg!(windows) {
            return self.binarize_native(staging_p, prefix, world, out_dir);
        }
        self.binarize_proton(staging_p, prefix, world, out_dir)
    }

    fn binarize_proton(
        &self,
        staging_p: &Path,
        prefix: &str,
        world: &str,
        out_dir: &Path,
    ) -> Result<PathBuf, ToolError> {
        let sandbox = tempfile::Builder::new()
            .prefix("naves-bin.")
            .tempdir_in(&self.sandbox_base)
            .map_err(ToolError::io(&self.sandbox_base))?;
        let root = sandbox.path().to_path_buf();
        let wine_prefix = root.join("prefix");
        for dir in ["out", "tex", "prefix"] {
            std::fs::create_dir_all(root.join(dir)).map_err(ToolError::io(&root))?;
        }
        copy_tree(
            &self.dayz_tools.join("Bin/Binarize"),
            &root.join("Binarize"),
        )?;

        let log = out_dir.join("binarize.log");
        let proton = |args: &[&str]| {
            let mut cmd = Command::new(&self.proton);
            cmd.arg("run")
                .args(args)
                .env("STEAM_COMPAT_CLIENT_INSTALL_PATH", &self.steam_root)
                .env("STEAM_COMPAT_DATA_PATH", &wine_prefix)
                .env("SteamAppId", DAYZ_TOOLS_APP_ID)
                .current_dir(staging_p);
            cmd
        };
        let keep = |sandbox: tempfile::TempDir| sandbox.keep();

        // The first run creates the Wine prefix; its exit status does not matter, the folders do.
        let init_log = root.join("prefix-init.log");
        let mut init = proton(&["cmd.exe", "/c", "exit"]);
        if matches!(
            run_supervised(&mut init, &init_log, self.timeout)?,
            Finished::TimedOut
        ) {
            self.stop_wineserver(&wine_prefix);
            return Err(ToolError::Timeout {
                tool: "Proton",
                secs: self.timeout.as_secs(),
                log: init_log,
                sandbox: keep(sandbox),
            });
        }
        let dosdevices = wine_prefix.join("pfx/dosdevices");
        std::fs::create_dir_all(&dosdevices).map_err(ToolError::io(&dosdevices))?;
        for (drive, target) in [("z:", root.as_path()), ("p:", staging_p)] {
            let l = dosdevices.join(drive);
            std::fs::remove_file(&l).ok(); // a fresh prefix has none; any other failure shows up at link()
            link(target, &l).map_err(ToolError::io(&l))?;
        }

        // The command line goes through Wine's cmd.exe only to capture binarize's console output; the
        // prefix and world name were checked above to be plain identifiers.
        let command = format!(
            "Z:\\Binarize\\binarize.exe -always -textures=Z:\\tex -binPath=P:\\ P:\\{prefix} Z:\\out > Z:\\binarize.log 2>&1"
        );
        let mut run = proton(&["cmd.exe", "/c", &command]);
        let proton_log = root.join("proton.log");
        let finished = run_supervised(&mut run, &proton_log, self.timeout)?;
        // Keep binarize's own log next to the build output, whatever happened.
        let tool_log = root.join("binarize.log");
        if tool_log.is_file() {
            std::fs::copy(&tool_log, &log).map_err(ToolError::io(&log))?;
        } else {
            std::fs::copy(&proton_log, &log).map_err(ToolError::io(&log))?;
        }
        match finished {
            Finished::TimedOut => {
                self.stop_wineserver(&wine_prefix);
                Err(ToolError::Timeout {
                    tool: "binarize",
                    secs: self.timeout.as_secs(),
                    log,
                    sandbox: keep(sandbox),
                })
            }
            Finished::Exited(status) if !status.success() => Err(ToolError::Binarize {
                code: status.code().unwrap_or(-1),
                log,
                sandbox: keep(sandbox),
            }),
            Finished::Exited(_) => {
                let produced = root.join("out").join(format!("{world}.wrp"));
                if !is_oprw(&produced) {
                    return Err(ToolError::NoWorld {
                        log,
                        sandbox: keep(sandbox),
                    });
                }
                self.wait_wineserver(&wine_prefix);
                let result = out_dir.join(format!("{world}.wrp"));
                std::fs::copy(&produced, &result).map_err(ToolError::io(&result))?;
                Ok(result)
            }
        }
    }

    /// Windows: `binarize_x64.exe` runs directly, with the staging folder as the project root.
    fn binarize_native(
        &self,
        staging_p: &Path,
        prefix: &str,
        world: &str,
        out_dir: &Path,
    ) -> Result<PathBuf, ToolError> {
        let exe = self.dayz_tools.join("Bin/Binarize/binarize_x64.exe");
        let log = out_dir.join("binarize.log");
        let mut cmd = Command::new(&exe);
        cmd.arg("-always")
            .arg(format!("-binPath={}", staging_p.display()))
            .arg(staging_p.join(prefix))
            .arg(out_dir)
            .current_dir(staging_p);
        match run_supervised(&mut cmd, &log, self.timeout)? {
            Finished::TimedOut => Err(ToolError::Timeout {
                tool: "binarize",
                secs: self.timeout.as_secs(),
                log,
                sandbox: out_dir.to_path_buf(),
            }),
            Finished::Exited(status) if !status.success() => Err(ToolError::Binarize {
                code: status.code().unwrap_or(-1),
                log,
                sandbox: out_dir.to_path_buf(),
            }),
            Finished::Exited(_) => {
                let result = out_dir.join(format!("{world}.wrp"));
                if is_oprw(&result) {
                    Ok(result)
                } else {
                    Err(ToolError::NoWorld {
                        log,
                        sandbox: out_dir.to_path_buf(),
                    })
                }
            }
        }
    }

    /// The `wineserver` of this Proton, if it has one.
    fn wineserver(&self) -> Option<PathBuf> {
        let candidate = self.proton.parent()?.join("files/bin/wineserver");
        candidate.is_file().then_some(candidate)
    }

    /// Stops the wineserver of one prefix (scoped by `WINEPREFIX`, never by process name).
    fn stop_wineserver(&self, wine_prefix: &Path) {
        if let Some(server) = self.wineserver() {
            // Best effort after a timeout: the sandbox is kept and named in the error either way.
            Command::new(server)
                .arg("-k")
                .env("WINEPREFIX", wine_prefix.join("pfx"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .ok();
        }
    }

    /// Waits for the wineserver of one prefix to exit after a normal run.
    fn wait_wineserver(&self, wine_prefix: &Path) {
        let Some(server) = self.wineserver() else {
            return;
        };
        let mut cmd = Command::new(&server);
        cmd.arg("-w").env("WINEPREFIX", wine_prefix.join("pfx"));
        let spawned = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = spawned else {
            return;
        };
        if matches!(child.wait_timeout(WINESERVER_WAIT), Ok(None)) {
            child.kill().ok(); // gave up waiting: the sandbox is deleted anyway
            child.wait().ok(); // reaps it
        }
    }
}

/// Whether `path` is a file that starts with the `OPRW` magic.
fn is_oprw(path: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0_u8; 4];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && &magic == b"OPRW"
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// Under `naves mcp` the stdin of this process is the protocol stream, which tools must not read.
    #[test]
    fn tools_do_not_inherit_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("log");
        let mut cmd = Command::new("readlink");
        cmd.arg("/proc/self/fd/0");
        run_supervised(&mut cmd, &log, Duration::from_mins(1)).unwrap();
        assert_eq!(std::fs::read_to_string(log).unwrap().trim(), "/dev/null");
    }

    /// A tool whose file was written moments earlier can be refused by the kernel with "text file
    /// busy": Linux releases a closed file's write permission a moment after `close`. The write
    /// handle here closes while the spawn is already retrying, which is that moment.
    #[test]
    fn a_tool_written_moments_earlier_still_starts() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("tool");
        let mut held = std::fs::File::create(&script).unwrap();
        held.write_all(b"#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        // 20 ms stand in for the kernel's deferred release after close.
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            drop(held);
        });
        let mut cmd = Command::new(&script);
        let log = dir.path().join("log");
        let finished = run_supervised(&mut cmd, &log, Duration::from_secs(10)).unwrap();
        release.join().unwrap();
        assert!(
            matches!(&finished, Finished::Exited(status) if status.success()),
            "{finished:?}"
        );
    }

    /// A tool that is still being written when the busy deadline passes is the original error,
    /// not a hang.
    #[test]
    fn a_tool_that_stays_busy_fails_with_the_error() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("tool");
        let mut held = std::fs::File::create(&script).unwrap();
        held.write_all(b"#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut cmd = Command::new(&script);
        let log = dir.path().join("log");
        let result = run_supervised(&mut cmd, &log, Duration::from_secs(10));
        drop(held);
        let error = result.expect_err("a busy tool must fail");
        assert!(
            error.to_string().contains("Text file busy"),
            "the original spawn error: {error}"
        );
    }
}

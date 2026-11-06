// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fake external tools for tests (feature `test-util`, Unix only): an `armake2` that lists what it
//! packs, a `proton` that "binarizes" by writing an `OPRW` file, a DayZ Server with a tiny
//! `dayzOffline.enoch` and a Steam folder around them.
//!
//! The CLI tests of the `naves` crate use [`FakeEnv::env`] to run the real binary against these fakes.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::tools::{BinarizeBridge, Tools};

const ARMAKE2: &str = r#"case "$1" in
  -h) echo "armake2 build [--proton-binarize] <source>" ;;
  img2paa)
    shift; [ "$1" = "-f" ] && shift
    # a PAA header like the real tool writes (type, an OFFS tag pointing at the first mipmap, no palette),
    # then the request as a stand-in for the mipmap data
    { printf '\001\377GGATSFFO\100\000\000\000\120\000\000\000'
      head -c 60 /dev/zero; printf '\000\000'
      printf 'PAA:%s\n' "$2"; cat "$3"; } > "$4" ;;
  keygen)
    shift; [ "$1" = "-f" ] && shift
    : > "$1.biprivatekey"; : > "$1.bikey" ;;
  build)
    shift; key=""
    while [ $# -gt 2 ]; do case "$1" in -k) key="$2"; shift 2;; *) shift;; esac; done
    { echo "key=$key"; (cd "$1" && find . -type f | sort); } > "$2"
    : > "$2.$(basename "$key" .biprivatekey).bisign" ;;
  *) echo "unexpected armake2 call: $*" >&2; exit 2 ;;
esac"#;

/// The fake `proton`: creates the prefix, then writes an `OPRW` world for every world below `P:`.
const PROTON: &str = r#"if [ "$4" = "exit" ]; then mkdir -p "$STEAM_COMPAT_DATA_PATH/pfx/dosdevices"; exit 0; fi
sandbox="$(dirname "$STEAM_COMPAT_DATA_PATH")"
for d in "$(pwd -P)"/naves/worlds/*/; do
  w="$(basename "$d")"
  [ -f "$d$w.wrp" ] || { echo "no $w.wrp in $d" >&2; exit 7; }
  printf 'OPRW\040\000\000\000' > "$sandbox/out/$w.wrp"
done
echo "fake binarize log" > "$sandbox/binarize.log"
exit 0"#;

const SPAWNS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?>
<playerspawnpoints>
    <fresh>
        <generator_params><grid_density>4</grid_density></generator_params>
        <generator_posbubbles>
            <group name="A"><pos x="1136.6" z="8039.5" /></group>
        </generator_posbubbles>
    </fresh>
    <hop>
        <generator_posbubbles>
            <group name="B"><pos x="10" z="20" /></group>
        </generator_posbubbles>
    </hop>
    <travel>
        <generator_posbubbles>
            <group name="C"><pos x="30" z="40" /></group>
        </generator_posbubbles>
    </travel>
</playerspawnpoints>
"#;

/// A set of fake tools below one folder.
#[derive(Debug, Clone)]
pub struct FakeEnv {
    /// The folder everything lives in.
    pub root: PathBuf,
    /// The fake `armake2` script.
    pub armake2: PathBuf,
    /// The fake Steam root.
    pub steam: PathBuf,
    /// The fake DayZ Server install.
    pub server: PathBuf,
}

fn write_script(path: &Path, body: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        format!("#!/bin/sh\nPATH=\"$PATH:/usr/bin:/bin\"\n{body}\n"),
    )?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

impl FakeEnv {
    /// Creates the fakes below `root`.
    ///
    /// # Errors
    ///
    /// Any I/O error while writing the files.
    pub fn new(root: &Path) -> std::io::Result<FakeEnv> {
        let steam = root.join("steam");
        let common = steam.join("steamapps/common");
        let server = common.join("DayZServer");
        let armake2 = root.join("bin/armake2");
        write_script(&armake2, ARMAKE2)?;
        std::fs::create_dir_all(common.join("DayZ Tools/Bin/Binarize"))?;
        std::fs::write(common.join("DayZ Tools/Bin/Binarize/binarize.exe"), "MZ")?;
        std::fs::create_dir_all(common.join("DayZ"))?;
        write_script(&common.join("Proton Hotfix/proton"), PROTON)?;
        write_script(&common.join("Proton Hotfix/files/bin/wineserver"), "exit 0")?;
        let enoch = server.join("mpmissions/dayzOffline.enoch");
        std::fs::create_dir_all(enoch.join("db"))?;
        std::fs::write(enoch.join("cfgplayerspawnpoints.xml"), SPAWNS)?;
        std::fs::write(
            enoch.join("mapgrouppos.xml"),
            "<map><group name=\"x\"/></map>\n",
        )?;
        std::fs::write(enoch.join("cfgeventspawns.xml"), "<eventposdef/>\n")?;
        std::fs::write(enoch.join("init.c"), "void main() {}\n")?;
        std::fs::write(enoch.join("cfgenvironment.xml"), "<env/>\n")?;
        std::fs::create_dir_all(enoch.join("env"))?;
        std::fs::write(
            enoch.join("env/bear_territories.xml"),
            "<territory-type/>\n",
        )?;
        std::fs::write(enoch.join("db/types.xml"), "<types/>\n")?;
        std::fs::create_dir_all(root.join("sandboxes"))?;
        Ok(FakeEnv {
            root: root.to_path_buf(),
            armake2,
            steam,
            server,
        })
    }

    /// The fake tools as a [`Tools`] value.
    #[must_use]
    pub fn tools(&self) -> Tools {
        let common = self.steam.join("steamapps/common");
        Tools {
            armake2: self.armake2.clone(),
            dayz_server: self.server.clone(),
            binarize: BinarizeBridge {
                steam_root: self.steam.clone(),
                dayz_tools: common.join("DayZ Tools"),
                dayz: common.join("DayZ"),
                proton: common.join("Proton Hotfix/proton"),
                sandbox_base: self.root.join("sandboxes"),
                timeout: Duration::from_mins(1),
            },
        }
    }

    /// Environment variables that make [`Tools::locate`] find these fakes.
    #[must_use]
    pub fn env(&self) -> Vec<(&'static str, PathBuf)> {
        vec![
            ("ARMAKE2", self.armake2.clone()),
            ("STEAM_ROOT", self.steam.clone()),
            ("DAYZ_SERVER", self.server.clone()),
        ]
    }

    /// Replaces the fake `armake2` by a script with the given body (for failure tests).
    ///
    /// # Errors
    ///
    /// Any I/O error while writing the script.
    pub fn replace_armake2(&self, body: &str) -> std::io::Result<()> {
        write_script(&self.armake2, body)
    }
}

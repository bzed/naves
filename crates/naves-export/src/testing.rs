// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fake external tools for tests (feature `test-util`, Unix only): an `dayzmake` that lists what it
//! packs, a `proton` that "binarizes" by writing an `OPRW` file, a DayZ Server with a tiny
//! `dayzOffline.enoch` and a Steam folder around them.
//!
//! The CLI tests of the `naves` crate use [`FakeEnv::env`] to run the real binary against these fakes.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::tools::{BinarizeBridge, Tools};

const DAYZMAKE: &str = r#"case "$1" in
  --version) echo "v2.0.0" ;;
  img2paa)
    shift; [ "$1" = "-f" ] && shift
    # a PAA like the real tool writes (type, an OFFS tag pointing at the first mipmap, no palette, an LZO
    # mipmap with the tool's stray byte, the end marker), then the request for debugging
    { printf '\001\377GGATSFFO\100\000\000\000\120\000\000\000'
      head -c 60 /dev/zero; printf '\000\000'
      printf '\004\200\004\000\004\000\000\021\000\000\000'
      head -c 6 /dev/zero
      printf 'PAA:%s\n' "$2"; cat "$3"; } > "$4" ;;
  unpack)
    # the road pieces: a tree of synthetic models next to the script
    cp -r "$(dirname "$0")/../roads-tree"/. "$3"/ ;;
  keygen)
    shift; [ "$1" = "-f" ] && shift
    : > "$1.biprivatekey"; : > "$1.bikey" ;;
  build)
    shift; key=""
    while [ $# -gt 2 ]; do case "$1" in -k) key="$2"; shift 2;; *) shift;; esac; done
    { echo "key=$key"; (cd "$1" && find . -type f | sort); } > "$2"
    : > "$2.$(basename "$key" .biprivatekey).bisign" ;;
  *) echo "unexpected dayzmake call: $*" >&2; exit 2 ;;
esac"#;

/// The fake `proton`: creates the prefix, then writes an `OPRW` world for every world below `P:`.
const PROTON: &str = r#"if [ "$4" = "exit" ]; then mkdir -p "$STEAM_COMPAT_DATA_PATH/pfx/dosdevices"; exit 0; fi
sandbox="$(dirname "$STEAM_COMPAT_DATA_PATH")"
for d in "$(pwd -P)"/naves/worlds/*/; do
  w="$(basename "$d")"
  [ -f "$d$w.wrp" ] || { echo "no $w.wrp in $d" >&2; exit 7; }
  printf 'OPRW\040\000\000\000' > "$sandbox/out/$w.wrp"
done
{ echo "fake binarize log"
  if [ -d "$(pwd -P)/dz/structures/roads/Parts" ]; then echo "road models on P: yes"; echo "Road 2 dirs: 4"; else echo "road models on P: no"; echo "Road 2 dirs: 0"; fi
} > "$sandbox/binarize.log"
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
    /// The fake `dayzmake` script.
    pub dayzmake: PathBuf,
    /// The fake Steam root.
    pub steam: PathBuf,
    /// The fake DayZ Server install.
    pub server: PathBuf,
}

fn write_script(path: &Path, body: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Write next to the script and rename it into place: a script that is still open for writing cannot be
    // executed ("Text file busy"), and the tests run it immediately after the fixture is made.
    let temporary = path.with_extension("tmp");
    std::fs::write(
        &temporary,
        format!("#!/bin/sh\nPATH=\"$PATH:/usr/bin:/bin\"\n{body}\n"),
    )?;
    std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&temporary, path)
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
        let dayzmake = root.join("bin/dayzmake");
        write_script(&dayzmake, DAYZMAKE)?;
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
        std::fs::create_dir_all(server.join("addons"))?;
        std::fs::write(
            server.join("addons/structures_roads.pbo"),
            b"fake roads pbo",
        )?;
        write_road_tree(&root.join("roads-tree"))?;
        Ok(FakeEnv {
            root: root.to_path_buf(),
            dayzmake,
            steam,
            server,
        })
    }

    /// The fake tools as a [`Tools`] value.
    #[must_use]
    pub fn tools(&self) -> Tools {
        let common = self.steam.join("steamapps/common");
        Tools {
            dayzmake: self.dayzmake.clone(),
            dayz_server: self.server.clone(),
            cache_dir: self.root.join("cache"),
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
            ("DAYZMAKE", self.dayzmake.clone()),
            ("STEAM_ROOT", self.steam.clone()),
            ("DAYZ_SERVER", self.server.clone()),
        ]
    }

    /// Replaces the fake `dayzmake` by a script with the given body (for failure tests).
    ///
    /// # Errors
    ///
    /// Any I/O error while writing the script.
    pub fn replace_dayzmake(&self, body: &str) -> std::io::Result<()> {
        write_script(&self.dayzmake, body)
    }

    /// Replaces the fake `proton` by a script with the given body (for binarize log tests).
    ///
    /// # Errors
    ///
    /// Any I/O error while writing the script.
    pub fn replace_proton(&self, body: &str) -> std::io::Result<()> {
        write_script(
            &self.steam.join("steamapps/common/Proton Hotfix/proton"),
            body,
        )
    }
}

/// Writes the unpacked content of a road PBO with synthetic models of all pieces, junctions and two bridges
/// below `dir` (`Parts/` and `Bridges/`, in the mixed case of the game's files).
///
/// # Errors
///
/// Any I/O error while writing the files.
pub fn write_road_tree(dir: &Path) -> std::io::Result<()> {
    for (path, model) in naves_core::catalog::testing::synthetic_models() {
        let (folder, file) = if path.contains("\\bridges\\") {
            ("Bridges", path.rsplit('\\').next().unwrap_or_default())
        } else {
            ("Parts", path.rsplit('\\').next().unwrap_or_default())
        };
        let stem = file.strip_suffix(".p3d").unwrap_or(file);
        let stem = match stem.strip_prefix("bridge_") {
            // the game's bridge files are capitalised: Bridge_Stone_25
            Some(rest) => {
                let words: Vec<String> = rest
                    .split('_')
                    .map(|w| {
                        let mut c = w.chars();
                        c.next()
                            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                            .unwrap_or_default()
                    })
                    .collect();
                format!("Bridge_{}", words.join("_"))
            }
            None => stem.to_string(),
        };
        let points: Vec<(&str, [f32; 3])> = model
            .memory
            .iter()
            .filter_map(|(name, v)| v.first().map(|p| (name.as_str(), *p)))
            .collect();
        let target = dir.join(folder).join(format!("{stem}.p3d"));
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(
            target,
            naves_formats::odol::synthetic::model_bytes_with_geometry(&points, &model.geometry),
        )?;
    }
    Ok(())
}

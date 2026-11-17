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

const DAYZMAKE: &str = r#"echo "$*" >> "$(dirname "$0")/../dayzmake-calls.log"
trees="$(dirname "$0")/../pbo-trees"
case "$1" in
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
    cp -r "$trees/$(basename "$2" .pbo)"/. "$3"/ && rm -f "$3/.prefix" ;;
  inspect)
    # the header with the prefix, then the table of files, as the real tool prints them
    tree="$trees/$(basename "$2" .pbo)"
    [ -d "$tree" ] || { echo "cannot read $2" >&2; exit 3; }
    printf 'Header extensions:\n- product=dayz\n- prefix=%s\n\n# Files\n\nPath    Method  Original  Packed\n==========\n' "$(cat "$tree/.prefix")"
    (cd "$tree" && find . -type f ! -name .prefix | sort | sed 's|^\./||; s|/|\\|g; s|$|    0    0    1|') ;;
  cat)
    tree="$trees/$(basename "$2" .pbo)"
    cp "$tree/$(printf '%s' "$3" | tr '\\' '/')" "$4" ;;
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
  if [ -d "$(pwd -P)/dz/structures_sakhal/roads" ]; then echo "sakhal models on P: yes"; else echo "sakhal models on P: no"; fi
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
        // the same, for a replacement script to fall back on
        write_script(&root.join("bin/dayzmake-fake"), DAYZMAKE)?;
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
        for (pbo, content) in [
            ("addons/structures_roads.pbo", &b"fake roads pbo"[..]),
            ("addons/structures_industrial.pbo", b"fake industrial pbo"),
            ("sakhal/addons/structures_sakhal.pbo", b"fake sakhal pbo"),
        ] {
            let path = server.join(pbo);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, content)?;
        }
        write_pbo_trees(&root.join("pbo-trees"))?;
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

/// The archive a synthetic model's path belongs to, its prefix as the game spells it, and the path inside it.
fn archive_of(path: &str) -> Option<(&'static str, &'static str, &str)> {
    [
        (
            "structures_roads",
            "DZ\\structures\\roads",
            "dz\\structures\\roads\\",
        ),
        (
            "structures_sakhal",
            "DZ\\structures_sakhal",
            "dz\\structures_sakhal\\",
        ),
        (
            "structures_industrial",
            "DZ\\structures\\industrial",
            "dz\\structures\\industrial\\",
        ),
    ]
    .into_iter()
    .find_map(|(pbo, prefix, key)| path.strip_prefix(key).map(|rest| (pbo, prefix, rest)))
}

/// `bridge_stone_25` as `Bridge_Stone_25`, `parts` as `Parts`: the game's files are mixed case. The road
/// pieces keep their lower-case names.
fn game_case(name: &str, capitalise: bool) -> String {
    if !capitalise {
        return name.to_string();
    }
    name.split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join("_")
}

/// Writes the content of the three archives the catalog reads, with synthetic models of all pieces,
/// junctions, bridges, panels and the pipe, below `dir/<archive>/` (`structures_roads/Parts/`, `.../Bridges/`
/// and so on, in the mixed case of the game's files), each with its prefix in a file `.prefix`.
///
/// # Errors
///
/// Any I/O error while writing the files.
pub fn write_pbo_trees(dir: &Path) -> std::io::Result<()> {
    for (path, model) in naves_core::catalog::testing::synthetic_models() {
        let Some((pbo, prefix, rest)) = archive_of(&path) else {
            continue;
        };
        let tree = dir.join(pbo);
        std::fs::create_dir_all(&tree)?;
        std::fs::write(tree.join(".prefix"), prefix)?;
        let parts: Vec<&str> = rest.split('\\').collect();
        let mut target = tree.clone();
        for (i, part) in parts.iter().enumerate() {
            let is_file = i + 1 == parts.len();
            let piece = is_file && rest.starts_with("parts\\");
            let part = part.strip_suffix(".p3d").unwrap_or(part);
            // the Sakhal archive spells its folders in lower case
            let folder_case = is_file || pbo != "structures_sakhal";
            let name = game_case(part, !piece && folder_case);
            target = target.join(if is_file { format!("{name}.p3d") } else { name });
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let points: Vec<(&str, [f32; 3])> = model
            .memory
            .iter()
            .filter_map(|(name, v)| v.first().map(|p| (name.as_str(), *p)))
            .collect();
        let faces: Vec<Vec<u16>> = model
            .geometry_faces
            .iter()
            .map(|t| {
                t.iter()
                    .map(|i| u16::try_from(*i).unwrap_or(u16::MAX))
                    .collect()
            })
            .collect();
        std::fs::write(
            target,
            naves_formats::odol::synthetic::model_bytes_with_lods(
                &points,
                model.geometry.as_deref().unwrap_or_default(),
                &faces,
                &model.roadway,
            ),
        )?;
    }
    Ok(())
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The server mission: the installed `dayzOffline.enoch` with the player spawn moved onto the new map.
//!
//! The mission is assembled at build time from the user's server install (read-only source) into the
//! build folder and is never committed. The player spawn points become one position, and the positions
//! of Livonia's buildings and events are removed, because they do not exist on the new map.

use std::io::Write;
use std::path::{Path, PathBuf};

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};
use walkdir::WalkDir;

/// Name of the mission folder that is copied.
const SOURCE_MISSION: &str = "dayzOffline.enoch";

/// Errors assembling the mission.
#[derive(Debug, thiserror::Error)]
pub enum MissionError {
    /// The world identifier is not `^[a-z][a-z0-9_]*$`.
    #[error("invalid world identifier {0:?}")]
    BadWorld(String),
    /// The spawn position is not a finite number.
    #[error("the spawn position must be finite")]
    BadSpawn,
    /// The installed `dayzOffline.enoch` is missing.
    #[error("source mission not found: {}", .0.display())]
    SourceMissing(PathBuf),
    /// The target mission folder already exists.
    #[error("{} already exists", .0.display())]
    Exists(PathBuf),
    /// Reading or writing a file failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file or folder concerned.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// A mission XML file could not be processed.
    #[error("{file}: {message}")]
    Xml {
        /// The file concerned.
        file: String,
        /// What went wrong.
        message: String,
    },
    /// Walking the source folder failed.
    #[error("cannot read the source mission: {0}")]
    Walk(#[from] walkdir::Error),
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> MissionError + '_ {
    move |source| MissionError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn is_world_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// What happens to one XML file of the mission.
enum Edit {
    /// Replace the content of every `generator_posbubbles` by one position.
    SpawnPoint((f32, f32)),
    /// Remove every element with one of these names.
    Drop(&'static [&'static str]),
}

fn xml_edit(text: &str, edit: &Edit) -> Result<Vec<u8>, String> {
    let mut reader = Reader::from_str(text);
    let mut writer = Writer::new(Vec::new());
    let mut skip_depth = 0_usize;
    loop {
        let event = reader.read_event().map_err(|e| e.to_string())?;
        if matches!(event, Event::Eof) {
            break;
        }
        match (&event, edit) {
            // inside an element that is being dropped
            (Event::Start(_), _) if skip_depth > 0 => skip_depth += 1,
            (Event::End(_), _) if skip_depth > 0 => skip_depth -= 1,
            (_, _) if skip_depth > 0 => {}
            (Event::Start(e), Edit::Drop(names)) if has_name(e, names) => skip_depth = 1,
            (Event::Empty(e), Edit::Drop(names)) if has_name(e, names) => {}
            (Event::Start(e), Edit::SpawnPoint((x, z)))
                if has_name(e, &["generator_posbubbles"]) =>
            {
                writer
                    .write_event(event.borrow())
                    .map_err(|e| e.to_string())?;
                let pos = format!("\n            <pos x=\"{x}\" z=\"{z}\" />\n        ");
                writer
                    .get_mut()
                    .write_all(pos.as_bytes())
                    .map_err(|e| e.to_string())?;
                skip_depth = 1;
            }
            _ => writer
                .write_event(event.borrow())
                .map_err(|e| e.to_string())?,
        }
    }
    Ok(writer.into_inner())
}

fn has_name(e: &BytesStart<'_>, names: &[&str]) -> bool {
    let name: &str = e.name().into_inner();
    names.contains(&name)
}

/// Assembles `out_dir/dayzOffline.<world>` from `<server_mpmissions>/dayzOffline.enoch`.
///
/// `cfgplayerspawnpoints.xml` gets the single spawn position (`x`, `z`) in its fresh, hop and travel
/// sections; the building groups in `mapgrouppos.xml` and the positions in `cfgeventspawns.xml` are removed;
/// everything else is copied unchanged. The source is only read, and the copy is writable even if the
/// source is not. The folder is built next to its destination and renamed into place.
///
/// # Errors
///
/// [`MissionError::BadWorld`] / [`MissionError::BadSpawn`] for invalid arguments,
/// [`MissionError::SourceMissing`] if the source mission is not installed,
/// [`MissionError::Exists`] if the target exists, and I/O or XML errors naming the file.
pub fn write_mission(
    server_mpmissions: &Path,
    out_dir: &Path,
    world: &str,
    spawn: (f32, f32),
) -> Result<PathBuf, MissionError> {
    if !is_world_ident(world) {
        return Err(MissionError::BadWorld(world.to_string()));
    }
    if !(spawn.0.is_finite() && spawn.1.is_finite()) {
        return Err(MissionError::BadSpawn);
    }
    let source = server_mpmissions.join(SOURCE_MISSION);
    if !source.is_dir() {
        return Err(MissionError::SourceMissing(source));
    }
    let target = out_dir.join(format!("dayzOffline.{world}"));
    if target.exists() {
        return Err(MissionError::Exists(target));
    }
    std::fs::create_dir_all(out_dir).map_err(io_err(out_dir))?;
    let staging = tempfile::Builder::new()
        .prefix(".mission-")
        .tempdir_in(out_dir)
        .map_err(io_err(out_dir))?;

    let spawn_edit = Edit::SpawnPoint(spawn);
    let groups_edit = Edit::Drop(&["group"]);
    let events_edit = Edit::Drop(&["pos"]);
    for entry in WalkDir::new(&source).follow_links(true).sort_by_file_name() {
        let entry = entry?;
        let relative = entry.path().strip_prefix(&source).unwrap_or(entry.path());
        let dest = staging.path().join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest).map_err(io_err(&dest))?;
            continue;
        }
        let bytes = std::fs::read(entry.path()).map_err(io_err(entry.path()))?;
        let name = relative.to_str().unwrap_or_default();
        let edit = match name {
            "cfgplayerspawnpoints.xml" => Some(&spawn_edit),
            "mapgrouppos.xml" => Some(&groups_edit),
            "cfgeventspawns.xml" => Some(&events_edit),
            _ => None,
        };
        let bytes = match edit {
            Some(edit) => {
                let text = String::from_utf8_lossy(&bytes);
                xml_edit(&text, edit).map_err(|message| MissionError::Xml {
                    file: name.to_string(),
                    message,
                })?
            }
            None => bytes,
        };
        // Written as new files so the copy is writable even if the source is read-only.
        std::fs::write(&dest, bytes).map_err(io_err(&dest))?;
    }
    let staged = staging.keep();
    std::fs::rename(&staged, &target).map_err(io_err(&target))?;
    Ok(target)
}

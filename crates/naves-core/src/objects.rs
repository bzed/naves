// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Objects with a stable identity, and the user's edits to them.
//!
//! A generated object is identified by where it comes from (spec 5.2): the stretch of road and its place
//! along it, not its position in a list. An edit (`edits/overrides.toml`) is pinned to that identity, so that
//! regenerating the map keeps it. If the object it was made for is gone, or has become something else, the
//! edit is kept exactly as it is and becomes an object of its own; a warning points at it.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::catalog::BridgeModel;
use crate::roadplace::PlacedNetwork;

/// An edited object further than this from where it was generated is not the object the edit was for.
const SAME_OBJECT_M: f64 = 25.0;
/// The folder of the edits below a project.
const EDITS_DIR: &str = "edits";
/// The file of the overrides.
const OVERRIDES_FILE: &str = "overrides.toml";

/// Where a generated object comes from: `<preset>/<feature>/<slot>`, for example `road/123:1-5/4`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId {
    /// The thing it belongs to: a stretch id.
    pub feature: String,
    /// What made it (`road`).
    pub preset: String,
    /// Its place along the feature.
    pub slot: u32,
}

/// A text that is not an object id.
#[derive(Debug, thiserror::Error)]
#[error("not an object id: {0:?} (expected <preset>/<feature>/<slot>)")]
pub struct ObjectIdError(String);

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.preset, self.feature, self.slot)
    }
}

impl FromStr for ObjectId {
    type Err = ObjectIdError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bad = || ObjectIdError(text.to_string());
        let (preset, rest) = text.split_once('/').ok_or_else(bad)?;
        let (feature, slot) = rest.rsplit_once('/').ok_or_else(bad)?;
        if preset.is_empty() || feature.is_empty() {
            return Err(bad());
        }
        Ok(ObjectId {
            feature: feature.to_string(),
            preset: preset.to_string(),
            slot: slot.parse().map_err(|_| bad())?,
        })
    }
}

impl Serialize for ObjectId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ObjectId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// An object as the generator makes it.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedObject {
    /// Its identity.
    pub id: ObjectId,
    /// Model path.
    pub model: String,
    /// East, in map metres.
    pub x: f64,
    /// North, in map metres.
    pub y: f64,
    /// Rotation in radians clockwise from north.
    pub yaw: f64,
}

/// What the user did to an object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Override {
    /// Put somewhere else.
    Moved {
        /// East.
        x: f64,
        /// North.
        y: f64,
        /// Rotation.
        yaw: f64,
    },
    /// Taken away.
    Deleted,
    /// Another model in its place.
    Replaced {
        /// The new model path.
        model: String,
    },
}

/// An edit and the object it was made for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PinnedEdit {
    /// The object it belongs to.
    pub id: ObjectId,
    /// The model that object had when the edit was made.
    pub generated_model: String,
    /// Where that object was.
    pub generated_at: (f64, f64),
    /// How that object was turned, in radians clockwise from north. Files from before this field was
    /// written read as 0.
    #[serde(default)]
    pub generated_yaw: f64,
    /// What was done.
    pub edit: Override,
}

/// All edits of a project.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Overrides {
    /// The edits, in the order they were made.
    #[serde(default)]
    pub edits: Vec<PinnedEdit>,
}

/// Why the overrides cannot be read or written.
#[derive(Debug, thiserror::Error)]
pub enum OverridesError {
    /// A file operation failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file or folder.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The file is not valid.
    #[error("the overrides are damaged: {0}")]
    Parse(#[from] toml::de::Error),
    /// An edit holds a value that cannot be used.
    #[error("the overrides are not usable: the edit of {id} has {field} {problem}")]
    Invalid {
        /// The object the edit belongs to.
        id: ObjectId,
        /// The field of the edit.
        field: &'static str,
        /// What is wrong with it.
        problem: &'static str,
    },
    /// The overrides cannot be written as TOML.
    #[error("the overrides cannot be written: {0}")]
    Write(#[from] toml::ser::Error),
}

impl Overrides {
    /// Reads the overrides from TOML text.
    ///
    /// # Errors
    /// [`OverridesError::Parse`] if the text is not a valid list of edits, [`OverridesError::Invalid`] if an
    /// edit holds a number that is not finite (TOML allows `nan` and `inf`) or a model name that is empty or
    /// not ASCII.
    pub fn parse(text: &str) -> Result<Overrides, OverridesError> {
        let overrides: Overrides = toml::from_str(text)?;
        overrides.validate()?;
        Ok(overrides)
    }

    /// Checks that every edit can be used: finite numbers, and model names that are not empty and are ASCII.
    ///
    /// # Errors
    /// [`OverridesError::Invalid`] for the first edit that cannot be used.
    pub fn validate(&self) -> Result<(), OverridesError> {
        for edit in &self.edits {
            let bad = |field, problem| OverridesError::Invalid {
                id: edit.id.clone(),
                field,
                problem,
            };
            let finite = |field: &'static str, value: f64| {
                if value.is_finite() {
                    Ok(())
                } else {
                    Err(bad(field, "a number that is not finite"))
                }
            };
            let model = |field: &'static str, name: &str| {
                if name.is_empty() {
                    Err(bad(field, "an empty name"))
                } else if !name.is_ascii() {
                    Err(bad(field, "a name with characters other than ASCII"))
                } else {
                    Ok(())
                }
            };
            finite("generated_at", edit.generated_at.0)?;
            finite("generated_at", edit.generated_at.1)?;
            finite("generated_yaw", edit.generated_yaw)?;
            model("generated_model", &edit.generated_model)?;
            match &edit.edit {
                Override::Moved { x, y, yaw } => {
                    finite("x", *x)?;
                    finite("y", *y)?;
                    finite("yaw", *yaw)?;
                }
                Override::Deleted => {}
                Override::Replaced { model: name } => model("model", name)?,
            }
        }
        Ok(())
    }

    fn path(dir: &Path) -> PathBuf {
        dir.join(EDITS_DIR).join(OVERRIDES_FILE)
    }

    /// Reads `edits/overrides.toml` below `dir`; a missing file is an empty list.
    ///
    /// # Errors
    /// [`OverridesError`] if the file exists but cannot be read or is damaged.
    pub fn load(dir: &Path) -> Result<Overrides, OverridesError> {
        let path = Self::path(dir);
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Overrides::default()),
            Err(source) => Err(OverridesError::Io { path, source }),
        }
    }

    /// Writes `edits/overrides.toml` below `dir`, atomically.
    ///
    /// # Errors
    /// [`OverridesError`] if the file cannot be written or an edit cannot be used (see [`Overrides::validate`]).
    pub fn save(&self, dir: &Path) -> Result<(), OverridesError> {
        self.validate()?;
        let path = Self::path(dir);
        let folder = dir.join(EDITS_DIR);
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source| OverridesError::Io { path, source }
        };
        std::fs::create_dir_all(&folder).map_err(io(&folder))?;
        let text = toml::to_string_pretty(self)?;
        let temp = tempfile::NamedTempFile::new_in(&folder).map_err(io(&folder))?;
        std::fs::write(temp.path(), text).map_err(io(temp.path()))?;
        temp.persist(&path).map_err(|e| io(&path)(e.error))?;
        Ok(())
    }
}

/// An object as it ends up in the world.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedObject {
    /// The generated object it is, or `None` for an edit that outlived its object.
    pub id: Option<ObjectId>,
    /// Model path.
    pub model: String,
    /// East.
    pub x: f64,
    /// North.
    pub y: f64,
    /// Rotation.
    pub yaw: f64,
}

/// An edit that outlived its object, kept as an object of its own.
#[derive(Debug, Clone, PartialEq)]
pub struct OrphanedEdit {
    /// The object the edit was made for.
    pub id: ObjectId,
    /// Where the edit's object now is.
    pub at: (f64, f64),
}

/// Applies the edits to the generated objects.
///
/// An edit whose object is gone, has another model, or lies more than 25 m from where it was generated was
/// made for something else: it is kept exactly as it is and becomes an object of its own, and is reported.
/// A deletion with no object left has nothing to keep and is dropped silently.
#[must_use]
pub fn apply_overrides(
    generated: &[GeneratedObject],
    overrides: &Overrides,
) -> (Vec<PlacedObject>, Vec<OrphanedEdit>) {
    // a later edit of the same object replaces an earlier one
    let mut edits: BTreeMap<&ObjectId, &PinnedEdit> = BTreeMap::new();
    for edit in &overrides.edits {
        edits.insert(&edit.id, edit);
    }
    let mut placed = Vec::new();
    let mut used: Vec<&ObjectId> = Vec::new();
    for object in generated {
        let edit = edits.get(&object.id).filter(|e| {
            e.generated_model == object.model
                && (object.x - e.generated_at.0).hypot(object.y - e.generated_at.1) <= SAME_OBJECT_M
        });
        let mut object = PlacedObject {
            id: Some(object.id.clone()),
            model: object.model.clone(),
            x: object.x,
            y: object.y,
            yaw: object.yaw,
        };
        if let Some(edit) = edit {
            used.push(&edit.id);
            match &edit.edit {
                Override::Deleted => continue,
                Override::Moved { x, y, yaw } => (object.x, object.y, object.yaw) = (*x, *y, *yaw),
                Override::Replaced { model } => object.model.clone_from(model),
            }
        }
        placed.push(object);
    }
    let mut orphans = Vec::new();
    for (id, edit) in edits {
        if used.contains(&id) {
            continue;
        }
        let object = match &edit.edit {
            // nothing to keep
            Override::Deleted => continue,
            Override::Moved { x, y, yaw } => PlacedObject {
                id: None,
                model: edit.generated_model.clone(),
                x: *x,
                y: *y,
                yaw: *yaw,
            },
            Override::Replaced { model } => PlacedObject {
                id: None,
                model: model.clone(),
                x: edit.generated_at.0,
                y: edit.generated_at.1,
                yaw: edit.generated_yaw,
            },
        };
        orphans.push(OrphanedEdit {
            id: id.clone(),
            at: (object.x, object.y),
        });
        placed.push(object);
    }
    (placed, orphans)
}

/// The road objects of a placed network: one per piece, junction, bridge and end cap, identified by the
/// stretch and slot they sit on.
#[must_use]
pub fn road_objects(net: &PlacedNetwork) -> Vec<GeneratedObject> {
    net.roads
        .iter()
        .map(|road| GeneratedObject {
            id: ObjectId {
                feature: road.slot.stretch.clone(),
                preset: "road".to_string(),
                slot: road.slot.index,
            },
            model: road.model.clone(),
            x: road.origin.x,
            y: road.origin.y,
            yaw: road.origin.heading,
        })
        .collect()
}

fn narrow(v: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "world file values are 32 bit floats"
    )]
    let narrowed = v as f32;
    narrowed
}

/// The 8WVR transform of a piece turned by `yaw` and standing at `ground_height`: the rotation column by
/// column (the images of the model's x, y and z axes), then position x, height, y. The pieces are not pitched:
/// the engine drapes them on the terrain.
#[must_use]
pub fn piece_transform(object: &PlacedObject, ground_height: f32) -> [f32; 12] {
    let (sin, cos) = object.yaw.sin_cos();
    let (sin, cos) = (narrow(sin), narrow(cos));
    // `0.0 - sin` and not `-sin`: a yaw of zero must not write a negative zero into the world file
    [
        cos,
        0.0,
        0.0 - sin,
        0.0,
        1.0,
        0.0,
        sin,
        0.0,
        cos,
        narrow(object.x),
        ground_height,
        narrow(object.y),
    ]
}

/// The height of a bridge model's origin when its deck is at `deck_level`: bridges are rigid objects
/// whose height is the deck level less the height of the deck above the model's origin.
#[must_use]
pub fn bridge_object_height(deck_level: f64, model: &BridgeModel) -> f64 {
    deck_level - model.deck_height
}

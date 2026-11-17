// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The catalog of vanilla road pieces: straights, arcs, end caps, junctions and bridges, with the
//! geometry of their join points as the models state it, and the bridge parts without ports (the concrete
//! panels and the pipe). For every bridge it knows what hangs below its deck (see [`shape`]).
//!
//! The catalog is built from the memory points of the models (see `naves_formats::odol`), never
//! from the file names alone: the names only say which kind of piece a model is. The heading
//! change of an arc, its chord and the width of a road all come from the points.
//!
//! Coordinates are model coordinates: x east, z north, metres. A heading is in radians clockwise
//! from north (+z), like everywhere in Náves.

pub mod shape;

use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

use naves_formats::odol::OdolModel;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use shape::{
    Ceiling, MAX_COORDINATE_M, MAX_PART_LENGTH_M, PartShape, Pipe, SLICE_M, STAND_TOL_M, Slab,
    Solid, Underside,
};

/// The folder of the road pieces inside the game, as a model path prefix.
pub const PARTS_PREFIX: &str = "dz\\structures\\roads\\parts\\";
/// The folder of the bridges inside the game, as a model path prefix.
pub const BRIDGES_PREFIX: &str = "dz\\structures\\roads\\bridges\\";
/// The folder of the concrete panels inside the game.
pub const PANELS_PREFIX: &str = "dz\\structures\\roads\\panels\\";
/// The folder of the Sakhal bridges (`bridge_wood2`, `bridge_concrete3`) inside the game.
pub const SAKHAL_BRIDGES_PREFIX: &str = "dz\\structures_sakhal\\roads\\bridges\\";
/// The folder of the concrete pipe inside the game.
pub const INDUSTRIAL_MISC_PREFIX: &str = "dz\\structures\\industrial\\misc\\";
/// The panels the catalog uses (not the broken `_dam`, the sunken `_legacy` or the `_lc` variants).
pub const PANEL_MODELS: [&str; 2] = ["panel_concrete_1", "panel_concrete_2"];
/// The concrete pipe.
pub const PIPE_MODEL: &str = "misc_concretepipe";

/// A kind of road surface; one set of pieces each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Family {
    /// Motorway and trunk asphalt (12 m).
    Asf1,
    /// Asphalt road (9 m).
    Asf2,
    /// Narrow asphalt road (7 m).
    Asf3,
    /// Town street (10 m).
    City,
    /// Gravel road (6 m).
    Grav,
    /// Dirt track (5 m).
    Mud,
}

impl Family {
    /// All families, in the order of the placement rank (widest first).
    pub const ALL: [Family; 6] = [
        Family::Asf1,
        Family::Asf2,
        Family::Asf3,
        Family::City,
        Family::Grav,
        Family::Mud,
    ];

    /// The prefix of the model names of this family.
    #[must_use]
    pub fn prefix(self) -> &'static str {
        match self {
            Family::Asf1 => "asf1",
            Family::Asf2 => "asf2",
            Family::Asf3 => "asf3",
            Family::City => "city",
            Family::Grav => "grav",
            Family::Mud => "mud",
        }
    }

    /// The width of the road in metres (all pieces of a family agree).
    #[must_use]
    pub fn width_m(self) -> f64 {
        match self {
            Family::Asf1 => 12.0,
            Family::Asf2 => 9.0,
            Family::Asf3 => 7.0,
            Family::City => 10.0,
            Family::Grav => 6.0,
            Family::Mud => 5.0,
        }
    }

    /// The family whose model names start with `prefix`.
    #[must_use]
    pub fn from_prefix(prefix: &str) -> Option<Family> {
        Family::ALL.into_iter().find(|f| f.prefix() == prefix)
    }
}

/// A join point of a piece: where the road enters or leaves the model.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Port {
    /// East, in model coordinates.
    pub x: f64,
    /// North, in model coordinates.
    pub z: f64,
    /// Direction of travel here, in radians clockwise from north.
    pub heading: f64,
}

/// What a piece is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PieceKind {
    /// A straight piece (6.25, 12.5 or 25 m).
    Straight,
    /// A piece that turns, from half a degree to 60 degrees.
    Arc,
    /// A dead end (`_6konec`).
    EndCap,
}

/// One road piece.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoadPiece {
    /// Model path, lower case with backslashes.
    pub model: String,
    /// The surface.
    pub family: Family,
    /// Straight, arc or end cap.
    pub kind: PieceKind,
    /// Width of the road at the start.
    pub width: f64,
    /// Where the road enters the model.
    pub start: Port,
    /// Where the road leaves the model.
    pub end: Port,
}

/// Wraps an angle into (-π, π].
fn wrap(angle: f64) -> f64 {
    let r = angle.rem_euclid(TAU);
    if r > PI { r - TAU } else { r }
}

impl RoadPiece {
    /// The piece used the other way round: the ports swap and both headings turn by π, so that an
    /// arc turns the other way.
    #[must_use]
    pub fn reversed(&self) -> RoadPiece {
        let turn = |p: Port| Port {
            heading: wrap(p.heading + PI),
            ..p
        };
        RoadPiece {
            start: turn(self.end),
            end: turn(self.start),
            ..self.clone()
        }
    }

    /// How far the heading changes between start and end, in (-π, π]; positive turns right.
    #[must_use]
    pub fn heading_change(&self) -> f64 {
        wrap(self.end.heading - self.start.heading)
    }

    /// The distance between the ports.
    #[must_use]
    pub fn chord(&self) -> f64 {
        (self.end.x - self.start.x).hypot(self.end.z - self.start.z)
    }
}

/// A T or X junction: a through road and one or two side roads at right angles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Junction {
    /// Model path.
    pub model: String,
    /// The surface of the through road.
    pub through: Family,
    /// The surface of the side road (for an X, of both).
    pub side: Family,
    /// Where the through road enters.
    pub start: Port,
    /// Where the through road leaves.
    pub end: Port,
    /// Where the side roads start, heading away from the through road: one for a T, two for an X.
    pub side_ports: Vec<Port>,
}

/// Where the thickness of a bridge deck comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThicknessSource {
    /// Measured on the faces of the model's own Geometry LOD.
    Measured,
    /// The model's Geometry LOD has vertices but no faces that can be used: the lowest vertex within
    /// [`MAX_DECK_THICKNESS_M`] of the deck, taken over the whole length.
    Vertices,
    /// The model's Geometry LOD cannot be read: the thickest deck measured among the models of the same type
    /// (the word after `bridge_` in the name: stone, wood, metal).
    SameType,
    /// As before, and no model of the same type can be read either: the thickest deck of all.
    Thickest,
    /// No bridge model can be read: nothing is known, `deck_thickness` is 0 and means nothing.
    #[default]
    Unknown,
}

/// Geometry that hangs further below the deck than this is no deck (piers, abutments), in metres.
pub const MAX_DECK_THICKNESS_M: f64 = 4.0;

/// A bridge model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeModel {
    /// Model path.
    pub model: String,
    /// Distance between the ports.
    pub length: f64,
    /// Width of the deck.
    pub width: f64,
    /// Height of the deck above the model origin.
    pub deck_height: f64,
    /// How far the deck reaches down below its surface over its opening, in metres: [`BridgeModel::thickness`]
    /// over [`BridgeModel::opening`]. Where the model's faces cannot be used, the lowest vertex within
    /// [`MAX_DECK_THICKNESS_M`] of the deck, and where its Geometry LOD cannot be read, taken from the other
    /// models (see `thickness_source`).
    pub deck_thickness: f64,
    /// Where `deck_thickness` comes from.
    #[serde(default)]
    pub thickness_source: ThicknessSource,
    /// Where the road enters; the heading says which way the bridge lies.
    pub start: Port,
    /// Where the road leaves.
    pub end: Port,
    /// What hangs below the deck, per slice of [`SLICE_M`] from `start`. Where the faces cannot be used, the
    /// deck thickness in every slice.
    #[serde(default)]
    pub underside: Underside,
    /// The runs of slices deeper than [`MAX_DECK_THICKNESS_M`]; none where the faces cannot be used.
    #[serde(default)]
    pub solid: Vec<Solid>,
    /// Whether the model holds the ground back at `start` and at `end` (a solid wall across the deck there).
    /// Only geometry between the ports is seen: an abutment wholly behind a port does not retain.
    #[serde(default)]
    pub retains: (bool, bool),
    /// The deepest point of the Geometry LOD below the deck surface, also beyond the ports.
    #[serde(default)]
    pub reach_down: f64,
}

impl BridgeModel {
    /// See [`PartShape::thickness`]: how far the model reaches below its deck over `from..to`, in metres
    /// from `start`.
    #[must_use]
    pub fn thickness(&self, from: f64, to: f64) -> f64 {
        self.shape().thickness(from, to)
    }

    /// See [`PartShape::opening`].
    #[must_use]
    pub fn opening(&self) -> (f64, f64) {
        self.shape().opening()
    }

    /// The model's shape below its deck.
    #[must_use]
    pub fn shape(&self) -> PartShape {
        PartShape {
            length: self.length,
            underside: self.underside.clone(),
            solid: self.solid.clone(),
            retains: self.retains,
            reach_down: self.reach_down,
            deck_half_width: self.width / 2.0,
        }
    }

    /// Whether the model stands on posts that must reach the ground (`bridge_wood_25`): nothing deeper than
    /// [`MAX_DECK_THICKNESS_M`], and something stands (see [`Underside::stand`]).
    #[must_use]
    pub fn posts(&self) -> bool {
        shape::on_posts(&self.underside, &self.solid)
    }
}

/// What can go wrong building the catalog.
#[derive(Debug, Error)]
pub enum CatalogError {
    /// A model lacks a memory point that its kind needs.
    #[error("{model}: the memory point \"{point}\" is missing")]
    MissingPoint {
        /// The model path.
        model: String,
        /// The selection name.
        point: &'static str,
    },
    /// A family has no straight piece at all.
    #[error("the family {family:?} has no straight piece; expected the model {expected}")]
    NoStraight {
        /// The family.
        family: Family,
        /// The model that should be there.
        expected: String,
    },
}

/// All the pieces a fit can use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PieceCatalog {
    pieces: BTreeMap<Family, Vec<RoadPiece>>,
    t_junctions: Vec<Junction>,
    x_junctions: Vec<Junction>,
    bridges: Vec<BridgeModel>,
    #[serde(default)]
    panels: Vec<Slab>,
    #[serde(default)]
    pipe: Option<Pipe>,
    #[serde(default)]
    warnings: Vec<String>,
}

/// What a model name says.
enum Class {
    Piece(Family, PieceKind),
    T(Family, Family),
    X(Family, Family),
    Bridge,
    Panel,
    Pipe,
}

/// The file name without folder and extension.
fn stem(path: &str) -> &str {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    name.strip_suffix(".p3d").unwrap_or(name)
}

/// The type of a bridge model, the word after `bridge_` in its name without a number (stone, wood, metal,
/// concrete: `bridge_wood2` is wood).
fn bridge_type(model: &str) -> String {
    let name = stem(model).to_ascii_lowercase();
    let rest = name.strip_prefix("bridge_").unwrap_or(&name);
    let word = rest.split('_').next().unwrap_or(rest);
    word.trim_end_matches(|c: char| c.is_ascii_digit())
        .to_string()
}

/// Gives each bridge whose thickness could not be measured the thickest deck measured among the models of its
/// type, else the thickest of all; each readable model keeps its own. A model whose faces cannot be used is
/// level at that thickness over its whole length. Returns a warning for each model that borrows (or has no)
/// thickness.
fn fill_thickness(bridges: &mut [BridgeModel]) -> Vec<String> {
    let has_own =
        |s: ThicknessSource| matches!(s, ThicknessSource::Measured | ThicknessSource::Vertices);
    let measured: Vec<(String, f64)> = bridges
        .iter()
        .filter(|b| has_own(b.thickness_source))
        .map(|b| (bridge_type(&b.model), b.deck_thickness))
        .collect();
    let thickest = |of: &mut dyn Iterator<Item = f64>| of.max_by(f64::total_cmp);
    let overall = thickest(&mut measured.iter().map(|m| m.1));
    let mut warnings = Vec::new();
    for b in bridges.iter_mut().filter(|b| !has_own(b.thickness_source)) {
        let kind = bridge_type(&b.model);
        let same = thickest(&mut measured.iter().filter(|m| m.0 == kind).map(|m| m.1));
        let model = &b.model;
        // a Geometry LOD that reads but holds nothing within reach of the deck and no faces is not unreadable
        let why = if b.reach_down > 0.0 {
            "its Geometry LOD has no faces and no vertex within 4 m below the deck"
        } else {
            "its Geometry LOD cannot be read"
        };
        if let Some(value) = same {
            b.deck_thickness = value;
            b.thickness_source = ThicknessSource::SameType;
            warnings.push(format!(
                "bridge model {model}: {why}; the thickest deck of its type, {value:.2} m, is used for the clearance over a road below, and it is taken as open over its whole length"
            ));
        } else if let Some(value) = overall {
            b.deck_thickness = value;
            b.thickness_source = ThicknessSource::Thickest;
            warnings.push(format!(
                "bridge model {model}: {why}, and no model of its type can be measured; the thickest deck of all bridge models, {value:.2} m, is used for the clearance over a road below, and it is taken as open over its whole length"
            ));
        } else {
            warnings.push(format!(
                "bridge model {model}: no bridge model's Geometry LOD can be read; the clearance over a road below is not checked"
            ));
        }
        let level = PartShape::level(b.length, b.deck_thickness, b.width / 2.0);
        b.underside = level.underside;
        b.reach_down = b.reach_down.max(b.deck_thickness);
    }
    warnings
}

fn two_families(rest: &str) -> Option<(Family, Family)> {
    let (a, b) = rest.split_once('_')?;
    Some((Family::from_prefix(a)?, Family::from_prefix(b)?))
}

fn is_number(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit())
}

fn classify(stem: &str) -> Option<Class> {
    let s = stem.to_ascii_lowercase();
    if s.starts_with("bridge_") {
        return Some(Class::Bridge);
    }
    if PANEL_MODELS.contains(&s.as_str()) {
        return Some(Class::Panel);
    }
    if s == PIPE_MODEL {
        return Some(Class::Pipe);
    }
    if let Some(rest) = s.strip_prefix("kr_t_") {
        return two_families(rest).map(|(a, b)| Class::T(a, b));
    }
    if let Some(rest) = s.strip_prefix("kr_x_") {
        return two_families(rest).map(|(a, b)| Class::X(a, b));
    }
    let (prefix, rest) = s.split_once('_')?;
    let family = Family::from_prefix(prefix)?;
    let kind = match rest {
        "6" | "12" | "25" => PieceKind::Straight,
        "6konec" => PieceKind::EndCap,
        arc => {
            let (angle, radius) = arc.split_once(' ')?;
            if !(is_number(angle) && is_number(radius)) {
                return None;
            }
            PieceKind::Arc
        }
    };
    Some(Class::Piece(family, kind))
}

/// Whether the catalog uses the model with this file name (with or without folder and extension).
#[must_use]
pub fn is_catalog_model(name: &str) -> bool {
    classify(stem(name)).is_some()
}

fn point(m: &OdolModel, model: &str, name: &'static str) -> Result<[f32; 3], CatalogError> {
    m.memory
        .get(name)
        .and_then(|v| v.first())
        .copied()
        .ok_or_else(|| CatalogError::MissingPoint {
            model: model.to_string(),
            point: name,
        })
}

/// The join point between two points on the road's edges, and the width between them. The left
/// point has the smaller x when the road runs north.
fn gate(
    m: &OdolModel,
    model: &str,
    left: &'static str,
    right: &'static str,
) -> Result<(Port, f64), CatalogError> {
    let l = point(m, model, left)?;
    let r = point(m, model, right)?;
    let (dx, dz) = (
        f64::from(r[0]) - f64::from(l[0]),
        f64::from(r[2]) - f64::from(l[2]),
    );
    let port = Port {
        x: f64::midpoint(f64::from(l[0]), f64::from(r[0])),
        z: f64::midpoint(f64::from(l[2]), f64::from(r[2])),
        // the vector from left to right is the heading turned a quarter clockwise
        heading: (-dz).atan2(dx),
    };
    Ok((port, dx.hypot(dz)))
}

fn side_port(
    m: &OdolModel,
    model: &str,
    pair: (&'static str, &'static str),
    through_x: f64,
) -> Result<Port, CatalogError> {
    let a = point(m, model, pair.0)?;
    let b = point(m, model, pair.1)?;
    let x = f64::midpoint(f64::from(a[0]), f64::from(b[0]));
    Ok(Port {
        x,
        z: f64::midpoint(f64::from(a[2]), f64::from(b[2])),
        heading: if x >= through_x {
            FRAC_PI_2
        } else {
            -FRAC_PI_2
        },
    })
}

fn junction(
    m: &OdolModel,
    model: &str,
    through: Family,
    side: Family,
    sides: &[(&'static str, &'static str)],
) -> Result<Junction, CatalogError> {
    let (start, _) = gate(m, model, "lb", "pb")?;
    let (end, _) = gate(m, model, "le", "pe")?;
    let through_x = f64::midpoint(start.x, end.x);
    let side_ports = sides
        .iter()
        .map(|pair| side_port(m, model, *pair, through_x))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Junction {
        model: model.to_string(),
        through,
        side,
        start,
        end,
        side_ports,
    })
}

/// The bridge part of a model, `None` for a model without a memory LOD (no ports: the 50 m wooden bridges), or
/// a warning why it cannot be used (a missing port, coordinates that are not numbers or far out, ports at one
/// point or a part longer than [`MAX_PART_LENGTH_M`]). The second value is a warning about faces that cannot
/// be used (the model is then taken as open over its whole length).
fn bridge(m: &OdolModel, model: &str) -> Option<Result<(BridgeModel, Option<String>), String>> {
    if m.memory.is_empty() {
        return None;
    }
    Some(measured_bridge(m, model))
}

fn measured_bridge(m: &OdolModel, model: &str) -> Result<(BridgeModel, Option<String>), String> {
    let skip = |why: String| format!("bridge model {model}: {why}; it is not used");
    let (start, width) = gate(m, model, "lb", "pb").map_err(|e| skip(e.to_string()))?;
    let (end, _) = gate(m, model, "le", "pe").map_err(|e| skip(e.to_string()))?;
    let mut ports = Vec::new();
    for name in ["lb", "pb", "le", "pe"] {
        ports.push(point(m, model, name).map_err(|e| skip(e.to_string()))?);
    }
    let geometry = m.geometry.as_deref().unwrap_or_default();
    if !shape::sane(&ports) || !shape::sane(geometry) {
        return Err(skip(format!(
            "its ports or its Geometry LOD have coordinates that are not numbers or lie more than {MAX_COORDINATE_M} m out"
        )));
    }
    let deck_height = ports.iter().map(|p| f64::from(p[1])).sum::<f64>() / 4.0;
    let length = (end.x - start.x).hypot(end.z - start.z);
    if !(MIN_PART_M..=MAX_PART_LENGTH_M).contains(&length) {
        return Err(skip(format!(
            "it is {length:.2} m between its ports, not {MIN_PART_M} to {MAX_PART_LENGTH_M} m"
        )));
    }
    if width < MIN_PART_M {
        return Err(skip(format!("its deck is {width:.2} m wide")));
    }
    let span = shape::Span {
        x: start.x,
        z: start.z,
        dir_x: (end.x - start.x) / length,
        dir_z: (end.z - start.z) / length,
        length,
        deck: deck_height,
    };
    let faces_usable =
        !geometry.is_empty() && !m.geometry_faces.is_empty() && m.geometry_faces_error.is_none();
    let (part, source, deck_thickness, warning) = if faces_usable {
        let part = shape::measure(&span, width / 2.0, geometry, &m.geometry_faces);
        let (from, to) = part.opening();
        let mut thickness = part.thickness(from, to);
        if !part.solid.is_empty() {
            // a slice at the edge of the opening also holds the solid part beside it; within the opening
            // nothing is deeper than this, or it would be solid
            thickness = thickness.min(MAX_DECK_THICKNESS_M);
        }
        (part, ThicknessSource::Measured, thickness, None)
    } else {
        // what was measured before the faces were read: the lowest vertex within reach of the deck
        let lowest = geometry
            .iter()
            .map(|p| f64::from(p[1]))
            .filter(|y| *y >= deck_height - MAX_DECK_THICKNESS_M)
            .min_by(f64::total_cmp);
        let thickness = lowest.map_or(0.0, |low| (deck_height - low).max(0.0));
        let mut part = PartShape::level(length, thickness, width / 2.0);
        part.reach_down = geometry
            .iter()
            .map(|p| deck_height - f64::from(p[1]))
            .fold(thickness, f64::max);
        let why = match &m.geometry_faces_error {
            Some(e) => format!("the faces of its Geometry LOD cannot be used ({e})"),
            None if geometry.is_empty() => String::new(),
            None => "its Geometry LOD has no faces".to_string(),
        };
        let source = if lowest.is_some() {
            ThicknessSource::Vertices
        } else {
            ThicknessSource::Unknown
        };
        let warning = (!why.is_empty() && lowest.is_some()).then(|| {
            format!(
                "bridge model {model}: {why}; it is taken as open over its whole length, {thickness:.2} m deep, with no solid ends"
            )
        });
        (part, source, thickness, warning)
    };
    Ok((
        BridgeModel {
            model: model.to_string(),
            length,
            width,
            deck_height,
            deck_thickness,
            thickness_source: source,
            start,
            end,
            underside: part.underside,
            solid: part.solid,
            retains: part.retains,
            reach_down: part.reach_down,
        },
        warning,
    ))
}

/// A bridge part shorter or narrower than this, in metres, is not used.
const MIN_PART_M: f64 = 1.0;

/// The bridge models, panels and the pipe that the build looks for; one that is missing is a warning.
fn expected_parts() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = crate::roadplace::BRIDGE_MODELS
        .iter()
        .map(|(name, _)| *name)
        .collect();
    names.extend(PANEL_MODELS);
    names.push(PIPE_MODEL);
    names
}

impl PieceCatalog {
    /// Builds the catalog from the models of the road PBO, keyed by model path. Models that are not
    /// road pieces (crosswalks, runways, taxiways, centre lines, paths, other sets) are ignored. A bridge part
    /// that cannot be used is left out with a warning (see [`PieceCatalog::warnings`]), as is a missing one.
    ///
    /// # Errors
    /// [`CatalogError::MissingPoint`] for a piece or junction without its memory points, and
    /// [`CatalogError::NoStraight`] for a family without a straight.
    pub fn from_models(models: &BTreeMap<String, OdolModel>) -> Result<PieceCatalog, CatalogError> {
        let mut catalog = PieceCatalog {
            pieces: Family::ALL.into_iter().map(|f| (f, Vec::new())).collect(),
            t_junctions: Vec::new(),
            x_junctions: Vec::new(),
            bridges: Vec::new(),
            panels: Vec::new(),
            pipe: None,
            warnings: Vec::new(),
        };
        for (path, m) in models {
            let Some(class) = classify(stem(path)) else {
                continue;
            };
            match class {
                Class::Piece(family, kind) => {
                    let (start, width) = gate(m, path, "lb", "pb")?;
                    let (end, _) = gate(m, path, "le", "pe")?;
                    catalog.pieces.entry(family).or_default().push(RoadPiece {
                        model: path.clone(),
                        family,
                        kind,
                        width,
                        start,
                        end,
                    });
                }
                Class::T(through, side) => {
                    let j = junction(m, path, through, side, &[("ld", "lh")])?;
                    catalog.t_junctions.push(j);
                }
                Class::X(through, side) => {
                    let j = junction(m, path, through, side, &[("ld", "lh"), ("pd", "ph")])?;
                    catalog.x_junctions.push(j);
                }
                Class::Bridge => match bridge(m, path) {
                    None => {}
                    Some(Ok((b, warning))) => {
                        catalog.bridges.push(b);
                        catalog.warnings.extend(warning);
                    }
                    Some(Err(warning)) => catalog.warnings.push(warning),
                },
                Class::Panel => {
                    let geometry = m.geometry.as_deref().unwrap_or_default();
                    match shape::slab(path, geometry, &m.roadway) {
                        Ok(slab) => catalog.panels.push(slab),
                        Err(why) => catalog
                            .warnings
                            .push(format!("panel {why}; it is not used")),
                    }
                }
                Class::Pipe => {
                    let geometry = m.geometry.as_deref().unwrap_or_default();
                    match shape::pipe(path, geometry, &m.roadway) {
                        Ok(pipe) if catalog.pipe.is_none() => catalog.pipe = Some(pipe),
                        Ok(_) => {}
                        Err(why) => catalog.warnings.push(format!("pipe {why}; it is not used")),
                    }
                }
            }
        }
        for family in Family::ALL {
            let has_straight = catalog
                .pieces(family)
                .iter()
                .any(|p| p.kind == PieceKind::Straight);
            if !has_straight {
                return Err(CatalogError::NoStraight {
                    family,
                    expected: format!("{PARTS_PREFIX}{}_25.p3d", family.prefix()),
                });
            }
        }
        let borrowed = fill_thickness(&mut catalog.bridges);
        catalog.warnings.extend(borrowed);
        for name in expected_parts() {
            let present = catalog
                .bridges
                .iter()
                .map(|b| b.model.as_str())
                .chain(catalog.panels.iter().map(|p| p.model.as_str()))
                .chain(catalog.pipe.iter().map(|p| p.model.as_str()))
                .any(|m| stem(m).eq_ignore_ascii_case(name));
            if !present {
                catalog.warnings.push(format!(
                    "the bridge part {name} is not in the game's files or cannot be read; bridges are built without it"
                ));
            }
        }
        Ok(catalog)
    }

    /// The concrete panels, in the order of their model paths.
    #[must_use]
    pub fn panels(&self) -> &[Slab] {
        &self.panels
    }

    /// The concrete pipe, if the game has it.
    #[must_use]
    pub fn pipe(&self) -> Option<&Pipe> {
        self.pipe.as_ref()
    }

    /// What the build should say about the catalog: parts that are missing or cannot be read, models whose
    /// faces cannot be used or whose deck thickness is borrowed. In a fixed order.
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Adds a warning (a reader of the game's files that found an archive missing).
    pub fn warn(&mut self, warning: String) {
        self.warnings.push(warning);
    }

    /// The pieces of a family, in the order of their model paths.
    #[must_use]
    pub fn pieces(&self, family: Family) -> &[RoadPiece] {
        self.pieces.get(&family).map_or(&[], Vec::as_slice)
    }

    /// The radius of the tightest arc of a family, in metres: the chord over twice the sine of half the
    /// turn, the smallest over its arc pieces. `None` if the family has no arc.
    #[must_use]
    pub fn tightest_arc(&self, family: Family) -> Option<f64> {
        self.pieces(family)
            .iter()
            .filter(|p| p.kind == PieceKind::Arc)
            .filter_map(|p| {
                let half_turn = p.heading_change().abs() / 2.0;
                (half_turn.sin() > 1e-9).then(|| p.chord() / (2.0 * half_turn.sin()))
            })
            .min_by(f64::total_cmp)
    }

    /// The T junction of a through road with a side road, if the game has one.
    #[must_use]
    pub fn t_junction(&self, through: Family, side: Family) -> Option<&Junction> {
        self.t_junctions
            .iter()
            .find(|j| j.through == through && j.side == side)
    }

    /// The X junction of a through road with a crossing road, if the game has one.
    #[must_use]
    pub fn x_junction(&self, through: Family, cross: Family) -> Option<&Junction> {
        self.x_junctions
            .iter()
            .find(|j| j.through == through && j.side == cross)
    }

    /// The bridge models that have ports.
    #[must_use]
    pub fn bridges(&self) -> &[BridgeModel] {
        &self.bridges
    }
}

/// A catalog and its models built from true circular arcs, for tests of this and other crates
/// (feature `test-util`). It contains no game data.
#[cfg(feature = "test-util")]
pub mod testing {
    use std::collections::BTreeMap;

    use naves_formats::odol::OdolModel;

    use super::{
        BRIDGES_PREFIX, Family, INDUSTRIAL_MISC_PREFIX, PANELS_PREFIX, PARTS_PREFIX, PieceCatalog,
        SAKHAL_BRIDGES_PREFIX,
    };

    /// How far below its deck the plain synthetic bridges of [`synthetic_models_with_bridges`] reach.
    pub const SYNTHETIC_DECK_THICKNESS: f64 = 1.5;

    /// Heading change in degrees and radius in metres of the arcs every family has.
    const ARCS: [(f64, f64); 11] = [
        (0.5, 2000.0),
        (1.0, 1000.0),
        (7.5, 100.0),
        (10.0, 100.0),
        (10.0, 75.0),
        (10.0, 50.0),
        (10.0, 25.0),
        (15.0, 75.0),
        (22.5, 50.0),
        (30.0, 25.0),
        (60.0, 10.0),
    ];

    const T_JUNCTIONS: [(Family, Family); 11] = [
        (Family::Asf1, Family::Asf2),
        (Family::Asf1, Family::Asf3),
        (Family::Asf1, Family::City),
        (Family::Asf2, Family::Asf2),
        (Family::Asf2, Family::Asf3),
        (Family::Asf3, Family::Asf2),
        (Family::Asf3, Family::Asf3),
        (Family::Asf3, Family::Mud),
        (Family::City, Family::Asf3),
        (Family::City, Family::City),
        (Family::Mud, Family::Mud),
    ];

    const X_JUNCTIONS: [(Family, Family); 4] = [
        (Family::Asf1, Family::Asf3),
        (Family::Asf1, Family::City),
        (Family::Asf2, Family::Asf3),
        (Family::City, Family::City),
    ];

    /// Road width of a family in metres.
    #[must_use]
    pub fn width(family: Family) -> f64 {
        match family {
            Family::Asf1 => 12.0,
            Family::Asf2 => 9.0,
            Family::Asf3 => 7.0,
            Family::City => 10.0,
            Family::Grav => 6.0,
            Family::Mud => 5.0,
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "synthetic geometry: the models store f32"
    )]
    fn f32_from(v: f64) -> f32 {
        v as f32
    }

    fn vertex(x: f64, y: f64, z: f64) -> [f32; 3] {
        [f32_from(x), f32_from(y), f32_from(z)]
    }

    /// The left and right edge points of a road of `width` centred at `(x, z)` heading `heading`.
    fn edges(x: f64, y: f64, z: f64, heading: f64, width: f64) -> ([f32; 3], [f32; 3]) {
        let (rx, rz) = (heading.cos() * width / 2.0, -heading.sin() * width / 2.0);
        (vertex(x - rx, y, z - rz), vertex(x + rx, y, z + rz))
    }

    fn model(points: &[(&str, [f32; 3])]) -> OdolModel {
        OdolModel {
            version: 56,
            bbox_min: [0.0; 3],
            bbox_max: [0.0; 3],
            memory: points
                .iter()
                .map(|(k, p)| ((*k).to_string(), vec![*p]))
                .collect(),
            properties: BTreeMap::new(),
            geometry: None,
            geometry_faces: Vec::new(),
            geometry_faces_error: None,
            roadway: Vec::new(),
        }
    }

    /// A piece: starts at `(0, -chord/2)` heading north, turns right by `degrees`.
    fn piece(width: f64, length: f64, degrees: f64, y: f64) -> OdolModel {
        let turn = degrees.to_radians();
        let (sx, sz) = (0.0, -length / 2.0);
        let (ex, ez) = (
            sx + length * (turn / 2.0).sin(),
            sz + length * (turn / 2.0).cos(),
        );
        let (lb, pb) = edges(sx, y, sz, 0.0, width);
        let (le, pe) = edges(ex, y, ez, turn, width);
        model(&[("lb", lb), ("pb", pb), ("le", le), ("pe", pe)])
    }

    /// A junction: the through road runs north along x = 1 (the origin is not on its centre line,
    /// as in the game), the side roads leave 6.25 m to the west and, for an X, to the east.
    fn junction(through: Family, side: Family, both_sides: bool) -> OdolModel {
        let w = width(through);
        let (lb, pb) = edges(1.0, 0.0, -6.25, 0.0, w);
        let (le, pe) = edges(1.0, 0.0, 6.25, 0.0, w);
        let half = width(side) / 2.0;
        let mut points = vec![
            ("lb", lb),
            ("pb", pb),
            ("le", le),
            ("pe", pe),
            ("ld", vertex(1.0 - 6.25, 0.0, -half)),
            ("lh", vertex(1.0 - 6.25, 0.0, half)),
        ];
        if both_sides {
            points.push(("pd", vertex(1.0 + 6.25, 0.0, -half)));
            points.push(("ph", vertex(1.0 + 6.25, 0.0, half)));
        }
        model(&points)
    }

    /// A box from `lo` to `hi` (model coordinates) as 8 vertices and 12 triangles, appended to `vertices` and
    /// `faces`.
    fn push_box(
        vertices: &mut Vec<[f32; 3]>,
        faces: &mut Vec<[u32; 3]>,
        lo: [f64; 3],
        hi: [f64; 3],
    ) {
        let base = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
        for i in 0..8u8 {
            let pick = |bit: u8, l: f64, h: f64| if i & bit == 0 { l } else { h };
            vertices.push(vertex(
                pick(1, lo[0], hi[0]),
                pick(2, lo[1], hi[1]),
                pick(4, lo[2], hi[2]),
            ));
        }
        // the six sides, two triangles each
        for [a, b, c, d] in [
            [0, 1, 3, 2],
            [4, 6, 7, 5],
            [0, 4, 5, 1],
            [2, 3, 7, 6],
            [0, 2, 6, 4],
            [1, 5, 7, 3],
        ] {
            faces.push([base + a, base + b, base + c]);
            faces.push([base + a, base + c, base + d]);
        }
    }

    /// One block of a synthetic bridge in its own frame: from `a0` to `a1` along the span (metres from the first
    /// port), from `c0` to `c1` across the axis, from `depth` below the deck
    /// surface up to `top` below it.
    #[derive(Debug, Clone, Copy)]
    pub struct Block {
        /// Start along the span.
        pub a0: f64,
        /// End along the span.
        pub a1: f64,
        /// Start across.
        pub c0: f64,
        /// End across.
        pub c1: f64,
        /// Top, as a depth below the deck surface (0: the surface).
        pub top: f64,
        /// Bottom, as a depth below the deck surface.
        pub depth: f64,
    }

    /// A bridge `length` long between its ports, `width` wide, its deck `deck` above the origin, lying north
    /// (`along_x` false: the first port at z = -length/2) or east, with a Geometry LOD of `blocks` (with faces).
    #[must_use]
    pub fn bridge_of_blocks(
        length: f64,
        width: f64,
        deck: f64,
        along_x: bool,
        blocks: &[Block],
    ) -> OdolModel {
        let mut m = bridge_ports(length, width, deck, along_x);
        let (mut vertices, mut faces) = (Vec::new(), Vec::new());
        let h = length / 2.0;
        for b in blocks {
            let (y0, y1) = (deck - b.depth, deck - b.top);
            let (lo, hi) = if along_x {
                ([b.a0 - h, y0, b.c0], [b.a1 - h, y1, b.c1])
            } else {
                ([b.c0, y0, b.a0 - h], [b.c1, y1, b.a1 - h])
            };
            push_box(&mut vertices, &mut faces, lo, hi);
        }
        m.geometry = Some(vertices);
        m.geometry_faces = faces;
        m
    }

    /// A plain deck `thickness` thick over the whole length, like `bridge_wood2`.
    #[must_use]
    pub fn slab_bridge(length: f64, width: f64, deck: f64, thickness: f64) -> OdolModel {
        bridge_of_blocks(
            length,
            width,
            deck,
            false,
            &[Block {
                a0: 0.0,
                a1: length,
                c0: -(width / 2.0),
                c1: width / 2.0,
                top: 0.0,
                depth: thickness,
            }],
        )
    }

    /// A deck `slab` thick on two end blocks `block` long, `block_half` to either side and `block_depth` deep, like
    /// `bridge_metal_25`.
    #[must_use]
    pub fn bridge_with_end_blocks(
        length: f64,
        width: f64,
        deck: f64,
        slab: f64,
        (block, block_half, block_depth): (f64, f64, f64),
    ) -> OdolModel {
        let end = |a0: f64, a1: f64| Block {
            a0,
            a1,
            c0: -(block_half),
            c1: block_half,
            top: slab,
            depth: block_depth,
        };
        bridge_of_blocks(
            length,
            width,
            deck,
            false,
            &[
                Block {
                    a0: 0.0,
                    a1: length,
                    c0: -(width / 2.0),
                    c1: width / 2.0,
                    top: 0.0,
                    depth: slab,
                },
                end(0.0, block),
                end(length - block, length),
            ],
        )
    }

    /// A single arch, like `bridge_stone_25`: an abutment 2 m long at each port standing `spring` below the deck,
    /// and between them the intrados `crown` below the deck at mid-span, growing with the square of the distance
    /// from mid-span to 2 m short of `spring` at the abutments, in steps of half a metre.
    #[must_use]
    pub fn arch_bridge(length: f64, width: f64, deck: f64, crown: f64, spring: f64) -> OdolModel {
        let half = width / 2.0 + 0.5;
        let block = |a0: f64, a1: f64, depth: f64| Block {
            a0,
            a1,
            c0: -half,
            c1: half,
            top: 0.0,
            depth,
        };
        let abutment = 2.0;
        let mut blocks = vec![
            block(0.0, abutment, spring),
            block(length - abutment, length, spring),
        ];
        let span = length / 2.0 - abutment;
        let mut a = abutment;
        while a < length - abutment - 1e-9 {
            let a1 = (a + 0.5).min(length - abutment);
            let t = (f64::midpoint(a, a1) - length / 2.0) / span;
            blocks.push(block(a, a1, crown + (spring - 2.0 - crown) * t * t));
            a = a1;
        }
        bridge_of_blocks(length, width, deck, false, &blocks)
    }

    /// A deck on rows of posts, like `bridge_wood_25`: lying east, rails along the whole length just above the
    /// deck and no deck in the Geometry LOD, and at each of `rows` (metres from the first port) posts 0.4 m long
    /// reaching `post_depth` below the deck, outside the deck to either side.
    #[must_use]
    pub fn post_bridge(
        length: f64,
        width: f64,
        deck: f64,
        rows: &[f64],
        post_depth: f64,
    ) -> OdolModel {
        let half = width / 2.0;
        let mut blocks = vec![Block {
            a0: 0.0,
            a1: length,
            c0: -(half + 0.3),
            c1: half + 0.3,
            top: -1.0,
            depth: -0.9,
        }];
        for &row in rows {
            blocks.push(Block {
                a0: row,
                a1: row + 0.4,
                c0: -(half + 0.8),
                c1: half + 0.8,
                top: -1.0,
                depth: post_depth,
            });
        }
        bridge_of_blocks(length, width, deck, true, &blocks)
    }

    /// A concrete panel: a slab `along` × `across` (the short side along x), `thickness` thick and centred on
    /// the origin, with a Roadway LOD whose surface is 0.02 m above the slab.
    #[must_use]
    pub fn panel_model(along: f64, across: f64, thickness: f64) -> OdolModel {
        let mut m = model(&[]);
        let (mut vertices, mut faces) = (Vec::new(), Vec::new());
        let (a, c, t) = (along / 2.0, across / 2.0, thickness / 2.0);
        push_box(&mut vertices, &mut faces, [-a, -t, -c], [a, t, c]);
        let mut roadway = Vec::new();
        push_box(
            &mut roadway,
            &mut Vec::new(),
            [-a - 0.05, -t - 0.02, -c - 0.05],
            [a + 0.05, t + 0.02, c + 0.05],
        );
        m.geometry = Some(vertices);
        m.geometry_faces = faces;
        m.roadway = roadway;
        m
    }

    /// The concrete pipe: `diameter` high and wide, `length` long along z, its axis through the origin.
    #[must_use]
    pub fn pipe_model(length: f64, diameter: f64) -> OdolModel {
        let mut m = model(&[]);
        let (mut vertices, mut faces) = (Vec::new(), Vec::new());
        let (r, l) = (diameter / 2.0, length / 2.0);
        push_box(&mut vertices, &mut faces, [-r, -r, -l], [r, r, l]);
        m.geometry = Some(vertices);
        m.geometry_faces = faces;
        m
    }

    fn bridge_ports(length: f64, width: f64, deck: f64, along_x: bool) -> OdolModel {
        let h = length / 2.0;
        let w = width / 2.0;
        if along_x {
            // the road runs east, so the left edge is to the north
            model(&[
                ("lb", vertex(-h, deck, w)),
                ("pb", vertex(-h, deck, -w)),
                ("le", vertex(h, deck, w)),
                ("pe", vertex(h, deck, -w)),
            ])
        } else {
            model(&[
                ("lb", vertex(-w, deck, -h)),
                ("pb", vertex(w, deck, -h)),
                ("le", vertex(-w, deck, h)),
                ("pe", vertex(w, deck, h)),
            ])
        }
    }

    /// Models for all six families, with junctions and three bridges, keyed like the game's paths.
    #[must_use]
    pub fn synthetic_models() -> BTreeMap<String, OdolModel> {
        let mut models = BTreeMap::new();
        let mut add = |prefix: &str, name: String, m: OdolModel| {
            models.insert(format!("{prefix}{name}.p3d"), m);
        };
        for family in Family::ALL {
            let (p, w) = (family.prefix(), width(family));
            for (suffix, length) in [("6", 6.25), ("12", 12.5), ("25", 25.0), ("6konec", 6.25)] {
                add(
                    PARTS_PREFIX,
                    format!("{p}_{suffix}"),
                    piece(w, length, 0.0, 0.0),
                );
            }
            for (degrees, radius) in ARCS {
                let chord = 2.0 * radius * (degrees.to_radians() / 2.0).sin();
                add(
                    PARTS_PREFIX,
                    format!("{p}_{} {}", fmt_number(degrees.floor()), fmt_number(radius)),
                    piece(w, chord, degrees, 0.0),
                );
            }
        }
        for (through, side) in T_JUNCTIONS {
            let name = format!("kr_t_{}_{}", through.prefix(), side.prefix());
            add(PARTS_PREFIX, name, junction(through, side, false));
        }
        for (through, side) in X_JUNCTIONS {
            let name = format!("kr_x_{}_{}", through.prefix(), side.prefix());
            add(PARTS_PREFIX, name, junction(through, side, true));
        }
        add(
            BRIDGES_PREFIX,
            "bridge_metal_25".to_string(),
            bridge_with_end_blocks(25.0, 12.0, 8.05, 1.7, (3.4, 8.0, 20.5)),
        );
        add(
            BRIDGES_PREFIX,
            "bridge_stone_25".to_string(),
            arch_bridge(25.0, 9.0, 7.9, 3.2, 16.9),
        );
        add(
            BRIDGES_PREFIX,
            "bridge_wood_25".to_string(),
            post_bridge(
                31.6,
                5.0,
                0.05,
                &[3.0, 6.5, 10.5, 15.0, 19.25, 24.0, 27.25],
                2.6,
            ),
        );
        add(
            SAKHAL_BRIDGES_PREFIX,
            "bridge_wood2".to_string(),
            slab_bridge(19.7, 7.6, 4.04, 0.9),
        );
        let mut walls = vec![Block {
            a0: 0.0,
            a1: 21.1,
            c0: -3.8,
            c1: 3.8,
            top: 0.0,
            depth: 0.6,
        }];
        for at in [4.5, 10.5, 16.5] {
            walls.push(Block {
                a0: at,
                a1: at + 0.4,
                c0: -3.8,
                c1: 3.8,
                top: 0.6,
                depth: 4.8,
            });
        }
        add(
            SAKHAL_BRIDGES_PREFIX,
            "bridge_concrete3".to_string(),
            bridge_of_blocks(21.1, 7.6, 2.16, false, &walls),
        );
        for name in ["panel_concrete_1", "panel_concrete_2"] {
            add(PANELS_PREFIX, name.to_string(), panel_model(3.0, 6.0, 0.24));
        }
        add(
            INDUSTRIAL_MISC_PREFIX,
            "misc_concretepipe".to_string(),
            pipe_model(3.04, 2.46),
        );
        models
    }

    /// The synthetic models with the bridges replaced by `bridges`: for each a name (the table of
    /// `roadplace::BRIDGE_MODELS` says which roads it serves), its length, width and deck height, lying north,
    /// a plain deck [`SYNTHETIC_DECK_THICKNESS`] thick. The panels and the pipe stay.
    #[must_use]
    pub fn synthetic_models_with_bridges(
        bridges: &[(&str, f64, f64, f64)],
    ) -> BTreeMap<String, OdolModel> {
        let mut models = synthetic_models();
        models.retain(|path, _| {
            !path.starts_with(BRIDGES_PREFIX) && !path.starts_with(SAKHAL_BRIDGES_PREFIX)
        });
        for &(name, length, width, deck) in bridges {
            models.insert(
                format!("{BRIDGES_PREFIX}{name}.p3d"),
                slab_bridge(length, width, deck, SYNTHETIC_DECK_THICKNESS),
            );
        }
        models
    }

    /// `10.0` as `10`; the game's names drop the fraction of the angle (`_7 100` is 7.5°).
    fn fmt_number(v: f64) -> String {
        format!("{v}")
    }

    /// The catalog of [`synthetic_models`].
    ///
    /// # Panics
    /// Never, for the models of this module.
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "test-only constructor over models built here"
    )]
    pub fn synthetic_catalog() -> PieceCatalog {
        PieceCatalog::from_models(&synthetic_models()).expect("the synthetic models are complete")
    }
}

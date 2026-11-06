// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The catalog of vanilla road pieces: straights, arcs, end caps, junctions and bridges, with the
//! geometry of their join points as the models state it.
//!
//! The catalog is built from the memory points of the models (see `naves_formats::odol`), never
//! from the file names alone: the names only say which kind of piece a model is. The heading
//! change of an arc, its chord and the width of a road all come from the points.
//!
//! Coordinates are model coordinates: x east, z north, metres. A heading is in radians clockwise
//! from north (+z), like everywhere in Náves.

use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

use naves_formats::odol::OdolModel;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The folder of the road pieces inside the game, as a model path prefix.
pub const PARTS_PREFIX: &str = "dz\\structures\\roads\\parts\\";
/// The folder of the bridges inside the game, as a model path prefix.
pub const BRIDGES_PREFIX: &str = "dz\\structures\\roads\\bridges\\";

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
    /// Where the road enters; the heading says which way the bridge lies.
    pub start: Port,
    /// Where the road leaves.
    pub end: Port,
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
}

/// What a model name says.
enum Class {
    Piece(Family, PieceKind),
    T(Family, Family),
    X(Family, Family),
    Bridge,
}

/// The file name without folder and extension.
fn stem(path: &str) -> &str {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    name.strip_suffix(".p3d").unwrap_or(name)
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

fn bridge(m: &OdolModel, model: &str) -> Result<Option<BridgeModel>, CatalogError> {
    if m.memory.is_empty() {
        // no memory LOD, so no ports: the 50 m wooden bridges
        return Ok(None);
    }
    let (start, width) = gate(m, model, "lb", "pb")?;
    let (end, _) = gate(m, model, "le", "pe")?;
    let mut height = 0.0;
    for name in ["lb", "pb", "le", "pe"] {
        height += f64::from(point(m, model, name)?[1]);
    }
    Ok(Some(BridgeModel {
        model: model.to_string(),
        length: (end.x - start.x).hypot(end.z - start.z),
        width,
        deck_height: height / 4.0,
        start,
        end,
    }))
}

impl PieceCatalog {
    /// Builds the catalog from the models of the road PBO, keyed by model path. Models that are not
    /// road pieces (crosswalks, runways, taxiways, centre lines, paths, other sets) are ignored.
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
                Class::Bridge => catalog.bridges.extend(bridge(m, path)?),
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
        Ok(catalog)
    }

    /// The pieces of a family, in the order of their model paths.
    #[must_use]
    pub fn pieces(&self, family: Family) -> &[RoadPiece] {
        self.pieces.get(&family).map_or(&[], Vec::as_slice)
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

    use super::{BRIDGES_PREFIX, Family, PARTS_PREFIX, PieceCatalog};

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

    /// A bridge `length` long and `width` wide, lying north (`along_x` false) or east, with its
    /// deck `deck` above the origin.
    fn bridge(length: f64, width: f64, deck: f64, along_x: bool) -> OdolModel {
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

    /// Models for all six families, with junctions and two bridges, keyed like the game's paths.
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
            "bridge_stone_25".to_string(),
            bridge(25.0, 9.0, 7.9, false),
        );
        add(
            BRIDGES_PREFIX,
            "bridge_wood_25".to_string(),
            bridge(31.6, 5.0, 0.05, true),
        );
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

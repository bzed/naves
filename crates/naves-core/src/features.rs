// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Road and path features in map coordinates, made from OSM ways.
//!
//! A feature is one stretch of a way that lies on the map. A way that leaves the map is cut where it
//! crosses a line [`CLIP_INSET_M`] inside the edge, so that no road piece can reach past the map; the cut
//! ends get nodes with negative ids, which later steps read as "this road goes on outside, put no end cap
//! here".
//!
//! The classification is the table of the roads plan: what OSM calls a road becomes a family of vanilla
//! road pieces or a footpath, and `sac_scale` or `trail_visibility` never give a road.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::catalog::Family;
use crate::heightmap::MapGeo;
use crate::osm::OsmWays;

/// How far inside the map's edge ways are cut, in metres: half the longest straight piece (12.5 m)
/// and a little more.
pub const CLIP_INSET_M: f64 = 13.0;

/// Where a feature comes from (spec 5.2). Only `Osm` is produced so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    /// Read from OpenStreetMap.
    Osm,
    /// Drawn by the user.
    User,
    /// Proposed by an AI assistant.
    Ai,
}

/// What a feature is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum FeatureKind {
    /// A road that gets road pieces.
    Road {
        /// The set of pieces.
        family: Family,
        /// Whether a vehicle should be able to drive it. A track that OSM calls very bad is physically a
        /// road but only has to be walkable.
        drivable: bool,
        /// A bridge (`bridge=*` other than `no`).
        bridge: bool,
        /// A tunnel (`tunnel=*` other than `no`).
        tunnel: bool,
    },
    /// A footpath, painted on the ground and never given road pieces.
    Path {
        /// Width of the painted trace in metres.
        width_m: f64,
    },
}

/// One stretch of a road or path on the map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    /// Stable identity: `osm/way/<id>/<part>`; a way that leaves the map and comes back has several parts.
    pub id: String,
    /// The OSM way this comes from.
    pub way_id: i64,
    /// Where it comes from.
    pub source: Source,
    /// Road or path.
    pub kind: FeatureKind,
    /// Positions in map metres (x east, y north, origin south-west).
    pub points: Vec<(f64, f64)>,
    /// Node id of each point; ends cut at the map's edge have negative ids. Same length as `points`.
    pub nodes: Vec<i64>,
}

fn flagged(tags: &BTreeMap<String, String>, key: &str) -> bool {
    tags.get(key).is_some_and(|v| v != "no")
}

/// Road width of a driveway or parking aisle and of a footpath, and of a demanding mountain trail.
const DRIVEWAY_M: f64 = 3.0;
const FOOTPATH_M: f64 = 1.6;
const MOUNTAIN_TRAIL_M: f64 = 1.0;

/// What a way with these tags is, if it leaves a trace on the ground.
///
/// `motor_vehicle=no` and `access=*` do not count: they say what is allowed in the real world, and a
/// forestry road in a national park is still physically a road.
#[must_use]
pub fn classify(tags: &BTreeMap<String, String>) -> Option<FeatureKind> {
    let get = |key: &str| tags.get(key).map(String::as_str);
    let highway = get("highway")?;
    if matches!(
        highway,
        "construction" | "proposed" | "raceway" | "platform" | "abandoned" | "razed"
    ) || get("area") == Some("yes")
    {
        return None;
    }
    // no trace on the ground
    if matches!(
        get("sac_scale"),
        Some("alpine_hiking" | "demanding_alpine_hiking" | "difficult_alpine_hiking")
    ) || matches!(get("trail_visibility"), Some("bad" | "horrible" | "no"))
    {
        return None;
    }
    match get("sac_scale") {
        Some("hiking" | "mountain_hiking") => {
            return Some(FeatureKind::Path {
                width_m: FOOTPATH_M,
            });
        }
        Some("demanding_mountain_hiking") => {
            return Some(FeatureKind::Path {
                width_m: MOUNTAIN_TRAIL_M,
            });
        }
        _ => {}
    }
    let road = |family, drivable| {
        Some(FeatureKind::Road {
            family,
            drivable,
            bridge: flagged(tags, "bridge"),
            tunnel: flagged(tags, "tunnel"),
        })
    };
    match highway {
        "motorway" | "motorway_link" | "trunk" | "trunk_link" | "primary" | "primary_link" => {
            road(Family::Asf1, true)
        }
        "secondary" | "secondary_link" => road(Family::Asf2, true),
        "tertiary" | "tertiary_link" | "unclassified" => road(Family::Asf3, true),
        "residential" | "living_street" => road(Family::City, true),
        "service" => match get("service") {
            Some("driveway" | "parking_aisle") => Some(FeatureKind::Path {
                width_m: DRIVEWAY_M,
            }),
            _ => road(Family::City, true),
        },
        "track" => classify_track(tags, road),
        "path" | "footway" | "cycleway" | "bridleway" | "steps" | "pedestrian" => {
            Some(FeatureKind::Path {
                width_m: FOOTPATH_M,
            })
        }
        _ => None,
    }
}

fn classify_track(
    tags: &BTreeMap<String, String>,
    road: impl Fn(Family, bool) -> Option<FeatureKind>,
) -> Option<FeatureKind> {
    let get = |key: &str| tags.get(key).map(String::as_str);
    if matches!(
        get("smoothness"),
        Some("horrible" | "very_horrible" | "impassable")
    ) {
        return road(Family::Mud, false);
    }
    match get("tracktype") {
        Some("grade1" | "grade2") => road(Family::Grav, true),
        Some("grade3") => road(Family::Mud, true),
        Some("grade4" | "grade5") => road(Family::Mud, false),
        _ => match get("surface") {
            Some("asphalt" | "paved" | "concrete" | "gravel" | "fine_gravel" | "compacted") => {
                road(Family::Grav, true)
            }
            _ => road(Family::Mud, true),
        },
    }
}

/// The part of the segment `p` to `q` inside the square `[lo, hi]²` as parameters `(t0, t1)`
/// (Liang–Barsky), or `None` if it lies outside.
fn clip_segment(p: (f64, f64), q: (f64, f64), lo: f64, hi: f64) -> Option<(f64, f64)> {
    let (dx, dy) = (q.0 - p.0, q.1 - p.1);
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (direction, room) in [
        (-dx, p.0 - lo),
        (dx, hi - p.0),
        (-dy, p.1 - lo),
        (dy, hi - p.1),
    ] {
        if direction.abs() < 1e-12 {
            if room < 0.0 {
                return None;
            }
        } else {
            let t = room / direction;
            if direction < 0.0 {
                if t > t1 {
                    return None;
                }
                t0 = t0.max(t);
            } else {
                if t < t0 {
                    return None;
                }
                t1 = t1.min(t);
            }
        }
    }
    Some((t0, t1))
}

fn lerp(p: (f64, f64), q: (f64, f64), t: f64) -> (f64, f64) {
    (p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t)
}

/// A run of a polyline: its points and their node ids.
type Run = (Vec<(f64, f64)>, Vec<i64>);

/// Cut ends a way can have: an OSM way has at most 2000 nodes, so at most two cuts per segment.
const CUTS_PER_WAY: i64 = 4096;

/// The id of the `index`-th cut end of the way `way`: `-(way * 4096 + index + 1)`. It depends on nothing
/// but the way and the place on it, it is negative so that it cannot be an OSM node id, and no two cut ends
/// of any ways share it. `None` if the way id is not positive or the id does not fit.
fn cut_id(way: i64, index: i64) -> Option<i64> {
    if way <= 0 || !(0..CUTS_PER_WAY).contains(&index) {
        return None;
    }
    way.checked_mul(CUTS_PER_WAY)?
        .checked_add(index)?
        .checked_add(1)?
        .checked_neg()
}

/// The runs of a polyline that lie inside the square `[lo, hi]²`, as points and node ids. The cut ends take
/// their ids from [`cut_id`] in the order they are made along `way`; `None` if one cannot get an id.
fn clip_polyline(
    points: &[(f64, f64)],
    nodes: &[i64],
    lo: f64,
    hi: f64,
    way: i64,
) -> Option<Vec<Run>> {
    let mut runs: Vec<Run> = Vec::new();
    // the last run ends exactly at the end of the previous segment
    let mut open = false;
    let mut made = 0_i64;
    let mut cut = || {
        let id = cut_id(way, made);
        made = made.saturating_add(1);
        id
    };
    let segments = points
        .iter()
        .zip(points.iter().skip(1))
        .zip(nodes.iter().zip(nodes.iter().skip(1)));
    for ((&p, &q), (&a, &b)) in segments {
        let Some((t0, t1)) = clip_segment(p, q, lo, hi) else {
            open = false;
            continue;
        };
        if t1 - t0 < 1e-9 {
            // touches the square in a single point
            open = false;
            continue;
        }
        if !(open && t0 <= 0.0) {
            let (start, id) = if t0 <= 0.0 {
                (p, a)
            } else {
                (lerp(p, q, t0), cut()?)
            };
            runs.push((vec![start], vec![id]));
        }
        let (end, id) = if t1 >= 1.0 {
            (q, b)
        } else {
            (lerp(p, q, t1), cut()?)
        };
        if let Some((run_points, run_nodes)) = runs.last_mut() {
            run_points.push(end);
            run_nodes.push(id);
        }
        open = t1 >= 1.0;
    }
    Some(runs)
}

/// The roads and paths of `ways` that lie on the map, in map coordinates, in way id order.
///
/// `map_size_m` is the side of the square map. Ways are cut [`CLIP_INSET_M`] inside its edge; the cut ends
/// get negative node ids that depend only on the way and the place on it (`-(way * 4096 + n + 1)` for the
/// `n`-th cut of the way), so that adding or removing other ways does not change them. A way that cannot be
/// projected, or that needs a cut end and has no positive id or one that is too large, is left out.
#[must_use]
pub fn road_features(ways: &OsmWays, frame: &MapGeo, map_size_m: f64) -> Vec<Feature> {
    let (lo, hi) = (CLIP_INSET_M, map_size_m - CLIP_INSET_M);
    if hi <= lo {
        return Vec::new();
    }
    let mut features = Vec::new();
    // in way id order whatever the order of the input, so that the ids of the cut ends are reproducible
    let mut ordered: Vec<_> = ways.ways.iter().collect();
    ordered.sort_by_key(|w| w.id);
    for way in ordered {
        let Some(kind) = classify(&way.tags) else {
            continue;
        };
        let projected: Result<Vec<(f64, f64)>, _> = way
            .points
            .iter()
            .map(|&(lat, lon)| frame.to_map(lat, lon))
            .collect();
        // a position that cannot be projected, or is not a number, leaves the way out
        let Ok(points) = projected else { continue };
        if points.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
            continue;
        }
        let Some(runs) = clip_polyline(&points, &way.nodes, lo, hi, way.id) else {
            continue;
        };
        for (part, (points, nodes)) in runs.into_iter().enumerate() {
            features.push(Feature {
                id: format!("osm/way/{}/{part}", way.id),
                way_id: way.id,
                source: Source::Osm,
                kind,
                points,
                nodes,
            });
        }
    }
    features
}

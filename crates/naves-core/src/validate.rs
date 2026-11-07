// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The violations of the road build: what the generation could not do, or did and cannot vouch for.
//!
//! Each rule has a stable id (for the report and for MCP filters) and a message that says what is wrong and
//! where, in the words the editor shows.

use std::collections::BTreeMap;

use crate::checks::{DriveState, PieceCheck, SteepPathSection};
use crate::conform::Conformed;
use crate::heightmap::MapGeo;
use crate::objects::OrphanedEdit;
use crate::osm::LandPolygon;
use crate::profile::Profiles;
use crate::roadplace::{PlacedKind, PlacedNetwork};

/// The priority of water among the OSM areas.
const WATER_PRIORITY: u8 = 3;
/// Places where the earthworks are capped are reported once per square of this many metres.
const REGION_M: f64 = 50.0;

/// What a violation is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    /// A road or drivable track that vehicles cannot drive.
    RoadNotDrivable,
    /// A road piece in the air or buried.
    RoadMissesTerrain,
    /// A path that is too steep to walk.
    PathTooSteep,
    /// A profile that exceeds the grade limits of its road.
    RoadGrade,
    /// A road that does not meet what it has to meet.
    RoadGap,
    /// A road piece standing in water.
    RoadInWater,
    /// Two roads that want different heights at one place.
    RoadConflict,
    /// Earthworks that stop at the cap and leave a step.
    EarthworksCapped,
    /// An edit that outlived its object.
    OrphanedEdit,
    /// Roads that meet without a junction piece.
    JoinWithoutJunction,
    /// A tunnel that got no pieces.
    TunnelOmitted,
}

impl Rule {
    /// Every rule, in the order violations are sorted.
    pub const ALL: [Rule; 11] = [
        Rule::RoadNotDrivable,
        Rule::RoadMissesTerrain,
        Rule::PathTooSteep,
        Rule::RoadGrade,
        Rule::RoadGap,
        Rule::RoadInWater,
        Rule::RoadConflict,
        Rule::EarthworksCapped,
        Rule::OrphanedEdit,
        Rule::JoinWithoutJunction,
        Rule::TunnelOmitted,
    ];

    /// The stable id of the rule.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Rule::RoadNotDrivable => "road_not_drivable",
            Rule::RoadMissesTerrain => "road_misses_terrain",
            Rule::PathTooSteep => "path_too_steep",
            Rule::RoadGrade => "road_grade",
            Rule::RoadGap => "road_gap",
            Rule::RoadInWater => "road_in_water",
            Rule::RoadConflict => "road_conflict",
            Rule::EarthworksCapped => "earthworks_capped",
            Rule::OrphanedEdit => "orphaned_edit",
            Rule::JoinWithoutJunction => "join_without_junction",
            Rule::TunnelOmitted => "tunnel_omitted",
        }
    }
}

/// One thing that is wrong, and where.
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    /// What it is about.
    pub rule: Rule,
    /// Where, in map metres.
    pub at: (f64, f64),
    /// What is wrong, for people.
    pub message: String,
}

fn violation(rule: Rule, at: (f64, f64), message: String) -> Violation {
    Violation { rule, at, message }
}

fn length_of(check: &PieceCheck) -> f64 {
    (check.end.0 - check.start.0).hypot(check.end.1 - check.start.1)
}

/// Neighbouring pieces that cannot be driven make one violation, with the length and the steepest grade.
fn not_drivable(net: &PlacedNetwork, pieces: &[PieceCheck]) -> Vec<Violation> {
    let by_slot: BTreeMap<usize, &PieceCheck> = pieces.iter().map(|p| (p.slot, p)).collect();
    let mut out = Vec::new();
    for chain in &net.chains {
        let mut run: Vec<&PieceCheck> = Vec::new();
        let mut flush = |run: &mut Vec<&PieceCheck>| {
            if let Some(first) = run.first() {
                let length: f64 = run.iter().map(|p| length_of(p)).sum();
                let grade = run.iter().map(|p| p.grade).fold(0.0, f64::max);
                let fall = run.iter().map(|p| p.cross_fall).fold(0.0, f64::max);
                let mut message =
                    format!("Not drivable: {:.0} % over {length:.0} m", grade * 100.0);
                if fall > crate::checks::MAX_CROSS_FALL {
                    message = format!("{message} (and {:.0} % across)", fall * 100.0);
                }
                out.push(violation(Rule::RoadNotDrivable, first.start, message));
            }
            run.clear();
        };
        for slot in chain {
            match by_slot.get(slot) {
                Some(piece) if piece.state == DriveState::NotDrivable => run.push(piece),
                _ => flush(&mut run),
            }
        }
        flush(&mut run);
    }
    out
}

fn ring_contains(ring: &[(f64, f64)], point: (f64, f64)) -> bool {
    let mut inside = false;
    for pair in ring.windows(2) {
        let [(lat_a, lon_a), (lat_b, lon_b)] = pair else {
            continue;
        };
        if (*lat_a > point.0) != (*lat_b > point.0) {
            let at = lon_a + (point.0 - lat_a) / (lat_b - lat_a) * (lon_b - lon_a);
            if point.1 < at {
                inside = !inside;
            }
        }
    }
    inside
}

fn in_water(water: &[&LandPolygon], lat: f64, lon: f64) -> bool {
    water.iter().any(|polygon| {
        ring_contains(&polygon.outer, (lat, lon))
            && !polygon
                .holes
                .iter()
                .any(|hole| ring_contains(hole, (lat, lon)))
    })
}

/// The road pieces that stand in water (bridges may).
fn water_violations(net: &PlacedNetwork, water: &[LandPolygon], frame: &MapGeo) -> Vec<Violation> {
    let lakes: Vec<&LandPolygon> = water
        .iter()
        .filter(|p| p.priority == WATER_PRIORITY)
        .collect();
    if lakes.is_empty() {
        return Vec::new();
    }
    net.roads
        .iter()
        .filter(|road| road.kind != PlacedKind::Bridge)
        .filter_map(|road| {
            let at = (road.origin.x, road.origin.y);
            let (lat, lon) = frame.to_geo(at.0, at.1).ok()?;
            in_water(&lakes, lat, lon).then(|| {
                violation(
                    Rule::RoadInWater,
                    at,
                    format!("Road {} stands in water", road.slot.stretch),
                )
            })
        })
        .collect()
}

/// The places where the earthworks hit the cap, once per square of 50 m.
fn capped_violations(capped: &[(f64, f64)]) -> Vec<Violation> {
    let mut regions: BTreeMap<(i64, i64), (f64, f64)> = BTreeMap::new();
    for &(x, y) in capped {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a region number of a map position"
        )]
        let key = ((x / REGION_M).floor() as i64, (y / REGION_M).floor() as i64);
        regions.entry(key).or_insert((x, y));
    }
    regions
        .into_values()
        .map(|at| {
            violation(
                Rule::EarthworksCapped,
                at,
                "The earthworks stop at 60 m and leave a step: a steep slope meets the road here"
                    .to_string(),
            )
        })
        .collect()
}

/// All the violations of a generated road network, sorted by rule and then by position.
///
/// `water` are the OSM areas of the project; only those with the priority of water count. Bridges may stand
/// in water.
#[must_use]
#[expect(clippy::too_many_arguments, reason = "one input per kind of violation")]
pub fn check_roads(
    net: &PlacedNetwork,
    profiles: &Profiles,
    conformed: &Conformed,
    pieces: &[PieceCheck],
    steep_paths: &[SteepPathSection],
    orphans: &[OrphanedEdit],
    water: &[LandPolygon],
    frame: &MapGeo,
) -> Vec<Violation> {
    let mut out = not_drivable(net, pieces);
    out.extend(
        pieces
            .iter()
            .filter(|p| p.miss_m > crate::checks::MAX_MISS_M)
            .map(|p| {
                violation(
                    Rule::RoadMissesTerrain,
                    p.start,
                    format!("Road piece misses the terrain by {:.1} m", p.miss_m),
                )
            }),
    );
    out.extend(steep_paths.iter().map(|s| {
        violation(
            Rule::PathTooSteep,
            s.from,
            format!(
                "Too steep to walk: {:.0}° over {:.0} m",
                s.slope_deg, s.length_m
            ),
        )
    }));
    out.extend(profiles.excesses.iter().map(|e| {
        violation(
            Rule::RoadGrade,
            e.at,
            format!(
                "Grade {:.0} % exceeds the limit of {:.0} %",
                e.grade * 100.0,
                e.limit * 100.0
            ),
        )
    }));
    out.extend(net.gaps.iter().map(|g| {
        violation(
            Rule::RoadGap,
            g.at,
            format!(
                "Road {} misses what it has to meet by {:.1} m",
                g.stretch, g.gap_m
            ),
        )
    }));
    out.extend(water_violations(net, water, frame));
    out.extend(conformed.conflicts.iter().map(|c| {
        violation(
            Rule::RoadConflict,
            c.at,
            format!(
                "Two roads want heights {:.1} m apart here",
                c.height_difference_m
            ),
        )
    }));
    out.extend(capped_violations(&conformed.capped));
    out.extend(orphans.iter().map(|o| {
        violation(
            Rule::OrphanedEdit,
            o.at,
            format!(
                "The edit of {} no longer matches an object; it is kept where it was",
                o.id
            ),
        )
    }));
    out.extend(net.joins_without_junction.iter().map(|&at| {
        violation(
            Rule::JoinWithoutJunction,
            at,
            "Roads meet here without a junction piece (the game has none for this pair)"
                .to_string(),
        )
    }));
    out.extend(net.tunnels_omitted.iter().map(|(id, at)| {
        violation(
            Rule::TunnelOmitted,
            *at,
            format!("Tunnel {id} has no pieces: real tunnels need terrain holes"),
        )
    }));
    out.sort_by(|a, b| {
        a.rule
            .cmp(&b.rule)
            .then(a.at.0.total_cmp(&b.at.0))
            .then(a.at.1.total_cmp(&b.at.1))
    });
    out
}

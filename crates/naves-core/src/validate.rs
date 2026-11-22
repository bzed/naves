// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The violations of the road build: what the generation could not do, or did and cannot vouch for.
//!
//! Each rule has a stable id (for the report and for MCP filters) and a message that says what is wrong and
//! where, in the words the editor shows.

use std::collections::BTreeMap;

use crate::bridgekind::Fallback;
use crate::checks::{DriveState, PieceCheck, SteepPathSection};
use crate::conform::{Buried, Conformed, Holder, POST_FLOATS_M, PostFloat};
use crate::dips::{Dip, DipAction};
use crate::heightmap::MapGeo;
use crate::junctions::{HOLD_M, JunctionKind, Junctions};
use crate::limits::{CROSS_FALL_LIMIT, RAMP_JOINT_MIN_M, earthwork_allowance_m};
use crate::lot::{LOT_MAX_MOVE_M, LOT_STEEP, LotReport, Outcome, centre};
use crate::netclean::{ApproachKind, CleanReport};
use crate::objects::OrphanedEdit;
use crate::osm::LandPolygon;
use crate::profile::{CLEARANCE_TOLERANCE_M, GradeExcess, GradeExcessKind, Profiles};
use crate::roadnet::DroppedDuplicate;
use crate::roadplace::{OVERLAY_DROP_M, PlacedKind, PlacedNetwork};
use crate::smooth::SmoothReport;
use crate::surface::SurfaceClass;

/// A bridge's deck that ends this far (metres) above or below the road or junction it joins is a finding.
pub const END_STEP_TOL_M: f64 = 0.5;
/// How far beyond a junction's patch a junction of a bridge's roads may lie from the end of its deck, inside a
/// chain, for the strip that runs through the patch to be compared with the junction's plane: the run from the
/// deck to the next variable of the height solve is at most a piece (25 m) long. The 25 m is set by hand from the
/// longest piece of the catalog, not derived from it: a longer piece would put its junction out of reach and its break
/// would not be compared.
pub const CROSSING_REACH_M: f64 = 25.0;
/// A chain end this close (metres) to the end of a deck is where the deck ends.
const END_JOIN_M: f64 = 1.5;
/// The priority of water among the OSM areas.
pub(crate) const WATER_PRIORITY: u8 = 3;
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
    /// A road that meets the end of a bridge turned away from the deck's heading.
    RoadKink,
    /// A road piece standing in water.
    RoadInWater,
    /// Two roads that want different heights at one place.
    RoadConflict,
    /// Earthworks that stop at the cap and leave a step.
    EarthworksCapped,
    /// An edit that outlived its object.
    OrphanedEdit,
    /// Roads that all end at one point without a junction piece (where one road runs through, the lower
    /// roads end under it instead, see `roadplace::Overlay`).
    JoinWithoutJunction,
    /// A tunnel that got no pieces.
    TunnelOmitted,
    /// Two junctions too close together for the difference of their heights.
    JunctionTooSteep,
    /// A road planned further from the original ground than fits its terrain.
    RoadFarFromGround,
    /// A road standing high over water that got no bridge.
    RoadOverWater,
    /// A road that OpenStreetMap draws twice: the lower-ranked copy's overlapping part is not placed.
    RoadDuplicateDropped,
    /// A bridge over a road that does not clear it by [`crate::profile::BRIDGE_CLEARANCE_M`].
    BridgeClearance,
    /// A solid part of a bridge, or the end of its deck, on the road below it: the road does not pass through the
    /// opening. The message has the metres of the road's interval that are missing.
    BridgeBlocksRoad,
    /// An OpenStreetMap bridge laid as plain road because the install has no part for it: no concrete panels and
    /// no bridge model that fits (a bridge of at most [`crate::bridgekind::FILL_MAX_M`] is filled on purpose and
    /// is none).
    BridgeNotBuilt,
    /// Ground above the underside of a bridge's deck after the shaping: another road's core stopped the cut.
    BridgeBuried,
    /// Posts, piers or walls of a bridge that stand more than [`crate::conform::POST_FLOATS_M`] above the ground.
    BridgePostFloats,
    /// The end of a bridge's deck and the road or junction it joins differ in height by more than
    /// [`END_STEP_TOL_M`]: the deck is level and the roads on its two sides want different heights.
    BridgeEndStep,
    /// A roundabout too small to be built (a radius below [`crate::roadnet::ROUNDABOUT_KEEP_M`] or the tightest
    /// arc of its surface) that became a plain junction.
    RoadRoundaboutCollapsed,
    /// A part of a road that runs beside a road of higher rank, and was not placed (the clean-up of the graph).
    RoadSidepathDropped,
    /// A short dead end that was not placed (the clean-up of the graph).
    RoadStubDropped,
    /// Junctions so close together that they became one (the clean-up of the graph).
    RoadJunctionsMerged,
    /// Close junctions that were left as they are, with the reason (the clean-up of the graph).
    RoadJunctionsNotMerged,
    /// A junction that was moved off the end of a bridge, so that the bridge's end touches only the road that
    /// goes on.
    BridgeApproachMade,
    /// A junction on the end of a bridge that could not be moved off it: the bridge is too short, a road passes
    /// below, or there is no room.
    BridgeEndAtJunction,
    /// A curve tighter than the road pieces can build that was widened outward (the smoothing of the graph).
    RoadCurveWidened,
    /// A curve that cannot be built as a curve: widening did not reach the radius of the road pieces.
    RoadCurveTooTight,
    /// A lot whose ground is shaped to a smooth surface (the form, the largest move and the surface class).
    LotSmoothed,
    /// A lot that gets a surface but whose ground is not shaped, with the reason.
    LotSurfaceOnly,
    /// A lot that is left alone, with the reason.
    LotExcluded,
    /// A lot whose slope is above what a vehicle can take across a road.
    LotSteep,
    /// A lot whose surface would move the ground too far, so the change is scaled for the whole lot.
    LotMoveLimited,
    /// A road inside a lot that is not built, and an access road cut at a lot's edge.
    RoadInLotDropped,
}

impl Rule {
    /// Every rule, in the order violations are sorted.
    pub const ALL: [Rule; 37] = [
        Rule::RoadNotDrivable,
        Rule::RoadMissesTerrain,
        Rule::PathTooSteep,
        Rule::RoadGrade,
        Rule::RoadGap,
        Rule::RoadKink,
        Rule::RoadInWater,
        Rule::RoadConflict,
        Rule::EarthworksCapped,
        Rule::OrphanedEdit,
        Rule::JoinWithoutJunction,
        Rule::TunnelOmitted,
        Rule::JunctionTooSteep,
        Rule::RoadFarFromGround,
        Rule::RoadOverWater,
        Rule::RoadDuplicateDropped,
        Rule::BridgeClearance,
        Rule::BridgeBlocksRoad,
        Rule::BridgeNotBuilt,
        Rule::BridgeBuried,
        Rule::BridgePostFloats,
        Rule::BridgeEndStep,
        Rule::RoadRoundaboutCollapsed,
        Rule::RoadSidepathDropped,
        Rule::RoadStubDropped,
        Rule::RoadJunctionsMerged,
        Rule::RoadJunctionsNotMerged,
        Rule::BridgeApproachMade,
        Rule::BridgeEndAtJunction,
        Rule::RoadCurveWidened,
        Rule::RoadCurveTooTight,
        Rule::LotSmoothed,
        Rule::LotSurfaceOnly,
        Rule::LotExcluded,
        Rule::LotSteep,
        Rule::LotMoveLimited,
        Rule::RoadInLotDropped,
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
            Rule::RoadKink => "road_kink",
            Rule::RoadInWater => "road_in_water",
            Rule::RoadConflict => "road_conflict",
            Rule::EarthworksCapped => "earthworks_capped",
            Rule::OrphanedEdit => "orphaned_edit",
            Rule::JoinWithoutJunction => "join_without_junction",
            Rule::TunnelOmitted => "tunnel_omitted",
            Rule::JunctionTooSteep => "junction_too_steep",
            Rule::RoadFarFromGround => "road_far_from_ground",
            Rule::RoadOverWater => "road_over_water",
            Rule::RoadDuplicateDropped => "road_duplicate_dropped",
            Rule::BridgeClearance => "bridge_clearance",
            Rule::BridgeBlocksRoad => "bridge_blocks_road",
            Rule::BridgeNotBuilt => "bridge_not_built",
            Rule::BridgeBuried => "bridge_buried",
            Rule::BridgePostFloats => "bridge_post_floats",
            Rule::BridgeEndStep => "bridge_end_step",
            Rule::RoadRoundaboutCollapsed => "road_roundabout_collapsed",
            Rule::RoadSidepathDropped => "road_sidepath_dropped",
            Rule::RoadStubDropped => "road_stub_dropped",
            Rule::RoadJunctionsMerged => "road_junctions_merged",
            Rule::RoadJunctionsNotMerged => "road_junctions_not_merged",
            Rule::BridgeApproachMade => "bridge_approach_made",
            Rule::BridgeEndAtJunction => "bridge_end_at_junction",
            Rule::RoadCurveWidened => "road_curve_widened",
            Rule::RoadCurveTooTight => "road_curve_too_tight",
            Rule::LotSmoothed => "lot_smoothed",
            Rule::LotSurfaceOnly => "lot_surface_only",
            Rule::LotExcluded => "lot_excluded",
            Rule::LotSteep => "lot_steep",
            Rule::LotMoveLimited => "lot_move_limited",
            Rule::RoadInLotDropped => "road_in_lot_dropped",
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

/// The roads that miss what they have to meet (`road_gap`) and the ones that meet a bridge turned (`road_kink`).
fn meeting_violations(net: &PlacedNetwork) -> Vec<Violation> {
    let mut out = Vec::new();
    out.extend(net.gaps.iter().map(|g| {
        violation(
            Rule::RoadGap,
            g.at,
            if g.nothing_placed {
                format!(
                    "Road {} has no pieces: nothing could be placed on its {:.1} m",
                    g.stretch, g.gap_m
                )
            } else {
                format!(
                    "Road {} misses what it has to meet by {:.1} m",
                    g.stretch, g.gap_m
                )
            },
        )
    }));
    out.extend(net.kinks.iter().map(|k| {
        violation(
            Rule::RoadKink,
            k.at,
            format!(
                "Road {} meets its bridge {:.1} degrees off the deck's heading",
                k.stretch, k.heading_error_deg
            ),
        )
    }));
    out
}

/// A bridge that clears the road below by less than it should.
fn clearance_violations(profiles: &Profiles) -> Vec<Violation> {
    profiles
        .clearances
        .iter()
        .filter(|c| c.reached < c.wanted - CLEARANCE_TOLERANCE_M)
        .map(|c| {
            violation(
                Rule::BridgeClearance,
                c.at,
                format!(
                    "A bridge clears the road below by only {:.1} m, the minimum is {:.1} m",
                    c.reached, c.wanted
                ),
            )
        })
        .collect()
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
                if fall > CROSS_FALL_LIMIT {
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

/// A box `(min lat, max lat, min lon, max lon)`.
pub(crate) type Bounds = (f64, f64, f64, f64);

/// The box around the outer ring of a polygon: `(min lat, max lat, min lon, max lon)`.
pub(crate) fn outer_bounds(polygon: &LandPolygon) -> Bounds {
    polygon.outer.iter().fold(
        (
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ),
        |(lat0, lat1, lon0, lon1), &(lat, lon)| {
            (lat0.min(lat), lat1.max(lat), lon0.min(lon), lon1.max(lon))
        },
    )
}

/// Whether a position lies in one of the lakes, each given with the box around its outer ring: a position
/// outside the box is not in the lake, which spares the walk over the ring.
pub(crate) fn in_water(water: &[(Bounds, &LandPolygon)], lat: f64, lon: f64) -> bool {
    water
        .iter()
        .any(|(bounds, polygon)| in_area(bounds, polygon, lat, lon))
}

/// Whether a position lies in one area, given the box around its outer ring.
fn in_area(bounds: &Bounds, polygon: &LandPolygon, lat: f64, lon: f64) -> bool {
    let (lat0, lat1, lon0, lon1) = *bounds;
    (lat0..=lat1).contains(&lat)
        && (lon0..=lon1).contains(&lon)
        && ring_contains(&polygon.outer, (lat, lon))
        && !polygon
            .holes
            .iter()
            .any(|hole| ring_contains(hole, (lat, lon)))
}

/// A test "is this point of the map in water" with the rule of `road_in_water` (the areas of
/// `WATER_PRIORITY` (3), their holes left out), for the smoothing of the road lines. The areas are turned into map
/// metres once, so the test can be shared between threads, which a [`MapGeo`] cannot. A ring that cannot be
/// projected is left out.
pub fn water_test(
    water: &[LandPolygon],
    frame: &MapGeo,
) -> impl Fn((f64, f64)) -> bool + Sync + use<> {
    let to_map = |ring: &[(f64, f64)]| -> Option<Vec<(f64, f64)>> {
        ring.iter()
            .map(|&(lat, lon)| frame.to_map(lat, lon).ok())
            .collect()
    };
    // the "lat" of a position is its x and the "lon" its y: the ring walk does not care
    let areas: Vec<(Bounds, LandPolygon)> = water
        .iter()
        .filter(|p| p.priority == WATER_PRIORITY)
        .filter_map(|p| {
            let outer = to_map(&p.outer)?;
            let holes = p.holes.iter().filter_map(|h| to_map(h)).collect();
            let area = LandPolygon {
                class: p.class,
                priority: p.priority,
                outer,
                holes,
            };
            Some((outer_bounds(&area), area))
        })
        .collect();
    move |(x, y)| {
        areas
            .iter()
            .any(|(bounds, polygon)| in_area(bounds, polygon, x, y))
    }
}

/// The road pieces that stand in water (bridges may).
fn water_violations(net: &PlacedNetwork, water: &[LandPolygon], frame: &MapGeo) -> Vec<Violation> {
    let lakes: Vec<_> = water
        .iter()
        .filter(|p| p.priority == WATER_PRIORITY)
        .map(|p| (outer_bounds(p), p))
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

/// One violation for each link that is still over its ceiling after the height solve, at the middle of the two
/// junctions.
fn junction_violations(junctions: &Junctions) -> Vec<Violation> {
    junctions
        .unresolved
        .iter()
        .filter_map(|&i| junctions.links.get(i))
        .filter_map(|link| {
            let (a, b) = (junctions.points.get(link.a)?, junctions.points.get(link.b)?);
            Some(violation(
                Rule::JunctionTooSteep,
                (f64::midpoint(a.at.0, b.at.0), f64::midpoint(a.at.1, b.at.1)),
                format!(
                    "Junctions too close for their heights: {:.1} m over {:.0} m",
                    (a.height_at(link.a_at) - b.height_at(link.b_at)).abs(),
                    link.length
                ),
            ))
        })
        .collect()
}

/// A piece planned further from the original ground than [`earthwork_allowance_m`] allows for its side
/// slope, at its middle; a piece without ground data (NaN) is not reported.
fn far_from_ground(piece: &PieceCheck) -> Option<Violation> {
    let allowance = earthwork_allowance_m(piece.side_slope);
    if piece.off_ground_m.is_nan() || piece.off_ground_m.abs() <= allowance {
        return None;
    }
    let message = if piece.off_ground_m < 0.0 {
        format!(
            "Cut {:.1} m deep; {allowance:.0} m fit this ground",
            -piece.off_ground_m
        )
    } else {
        format!(
            "Embankment {:.1} m high; {allowance:.0} m fit this ground",
            piece.off_ground_m
        )
    };
    let at = (
        f64::midpoint(piece.start.0, piece.end.0),
        f64::midpoint(piece.start.1, piece.end.1),
    );
    Some(violation(Rule::RoadFarFromGround, at, message))
}

/// One violation for each part of a road left out because another road runs along it, in the middle of it.
fn duplicate_violations(duplicates: &[DroppedDuplicate]) -> Vec<Violation> {
    duplicates
        .iter()
        .map(|d| {
            violation(
                Rule::RoadDuplicateDropped,
                d.at,
                format!(
                    "Road {} runs along {} for {:.0} m: OpenStreetMap draws it twice, this part is not placed",
                    d.stretch,
                    d.kept,
                    d.to_m - d.from_m
                ),
            )
        })
        .collect()
}

/// One violation for each curve the smoothing widened (`road_curve_widened`) and one for each curve that could
/// not be built as a curve (`road_curve_too_tight`), at the tightest point of the curve.
fn smooth_violations(smooth: Option<&SmoothReport>) -> Vec<Violation> {
    let Some(smooth) = smooth else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for z in &smooth.zones {
        let road = z.stretches.join(", ");
        if z.blocked_by_water {
            out.push(violation(
                Rule::RoadCurveTooTight,
                z.at,
                format!(
                    "The curve of {road} was left as drawn: widening it would put the road in water (radius {:.1} m, {:.1} m wanted)",
                    z.radius_before_m, z.radius_wanted_m
                ),
            ));
        } else if z.plate_candidate {
            out.push(violation(
                Rule::RoadCurveTooTight,
                z.at,
                format!(
                    "The curve of {road} cannot be built as a curve: radius {:.1} m reached, {:.1} m wanted",
                    z.radius_after_m, z.radius_wanted_m
                ),
            ));
        } else if z.max_outward_m > 0.0 || z.max_inward_m > 0.0 {
            out.push(violation(
                Rule::RoadCurveWidened,
                z.at,
                format!(
                    "The curve of {road} was widened: radius {:.1} m -> {:.1} m, moved by up to {:.1} m",
                    z.radius_before_m,
                    z.radius_after_m,
                    z.max_outward_m.max(z.max_inward_m)
                ),
            ));
        }
    }
    out
}

/// One violation for each change the clean-up of the graph made, at the place of the change. Nothing is changed
/// silently.
fn clean_violations(clean: &CleanReport) -> Vec<Violation> {
    let mut out = Vec::new();
    out.extend(clean.roundabouts.iter().map(|r| {
        violation(
            Rule::RoadRoundaboutCollapsed,
            r.centre,
            format!(
                "A roundabout of {:.1} m radius is too small: it is a plain junction",
                r.radius
            ),
        )
    }));
    out.extend(clean.sidepaths.iter().map(|d| {
        violation(
            Rule::RoadSidepathDropped,
            d.at,
            format!(
                "Road {} runs beside {} for {:.0} m: this part is not placed",
                d.stretch,
                d.road,
                d.to_m - d.from_m
            ),
        )
    }));
    out.extend(clean.stubs.iter().map(|d| {
        violation(
            Rule::RoadStubDropped,
            d.at,
            format!(
                "Road {} is a dead end of {:.1} m: it is not placed",
                d.stretch, d.length_m
            ),
        )
    }));
    out.extend(clean.merged.iter().map(|m| {
        violation(
            Rule::RoadJunctionsMerged,
            m.at,
            format!(
                "The junctions {:?} are close to {}: they are one junction",
                m.merged, m.kept
            ),
        )
    }));
    out.extend(clean.not_merged.iter().map(|n| {
        violation(
            Rule::RoadJunctionsNotMerged,
            n.at,
            format!(
                "The junctions {:?} are close together but were left as they are: {}",
                n.nodes, n.reason
            ),
        )
    }));
    out.extend(clean.approaches.iter().map(|a| {
        let what = match a.kind {
            ApproachKind::SideRoadsMoved => {
                format!("{} road(s) were joined further along", a.moved.len())
            }
            ApproachKind::BridgeShortened => "the bridge was shortened".to_string(),
        };
        violation(
            Rule::BridgeApproachMade,
            a.at,
            format!(
                "A junction was moved off the end {} of bridge {}: {what}",
                a.end, a.bridge
            ),
        )
    }));
    out.extend(clean.no_approach.iter().map(|n| {
        violation(
            Rule::BridgeEndAtJunction,
            n.at,
            format!(
                "A junction sits on the end {} of bridge {} and could not be moved off it: {}",
                n.end, n.bridge, n.reason
            ),
        )
    }));
    out
}

/// An OpenStreetMap bridge that the install has no part for: one violation in its middle.
fn not_built_violations(net: &PlacedNetwork) -> Vec<Violation> {
    net.bridge_fits
        .iter()
        .filter(|f| f.fallback.contains(&Fallback::NoParts))
        .map(|f| {
            violation(
                Rule::BridgeNotBuilt,
                f.at,
                format!(
                    "Bridge {} ({:.0} m) is plain road: the install has no concrete panels and no bridge model that fits it",
                    f.stretch, f.length
                ),
            )
        })
        .collect()
}

/// Less than this of a road's interval missing is no finding (rounding of the model's measured geometry), in metres.
const BLOCKS_MIN_M: f64 = 0.05;

/// A bridge whose part stands on a road below it, or ends on it: one violation for each road, in the order of the
/// bridges and their roads, with the metres of the road's interval that are missing. A road that was not placed
/// under the bridge is none (see [`crate::bridgefit::BelowFit::placed`]).
fn blocks_violations(net: &PlacedNetwork) -> Vec<Violation> {
    net.bridge_fits
        .iter()
        .flat_map(|f| f.below.iter().map(move |b| (f, b)))
        .filter(|(_, b)| b.placed && b.missing > BLOCKS_MIN_M)
        .map(|(f, b)| {
            violation(
                Rule::BridgeBlocksRoad,
                b.at,
                format!(
                    "Bridge {} lies on the road {} below it: {:.1} m of the {:.1} m the road needs are under a solid part or past the end of the deck",
                    f.stretch,
                    b.lower,
                    b.missing,
                    b.interval.1 - b.interval.0
                ),
            )
        })
        .collect()
}

/// Bridges with ground left above the underside of their deck: one violation for each, where the ground is highest
/// above it, naming what holds it.
fn buried_violations(
    buried: &[Buried],
    net: &PlacedNetwork,
    profiles: &Profiles,
) -> Vec<Violation> {
    buried
        .iter()
        .map(|b| {
            let holder = match b.holder {
                Holder::Patch(at) => {
                    format!("a level patch at ({:.0}, {:.0})", at.0, at.1)
                }
                Holder::None => "no road (the ground was not cut there)".to_string(),
                Holder::Strip(i) => {
                    let mut ids: Vec<&str> = profiles
                        .strips
                        .get(i)
                        .into_iter()
                        .flat_map(|s| s.slots.iter())
                        .filter_map(|r| net.roads.get(*r))
                        .map(|r| r.slot.stretch.as_str())
                        .collect();
                    ids.dedup();
                    ids.truncate(3);
                    format!("the road {}", ids.join(", "))
                }
            };
            let reference = if b.guarded {
                "the deck"
            } else {
                "the underside of its deck"
            };
            violation(
                Rule::BridgeBuried,
                b.at,
                format!(
                    "The ground under bridge {} is {:.1} m above {reference} at {} {}, held by {holder}",
                    b.stretch,
                    b.above_m,
                    b.vertices,
                    if b.vertices == 1 { "place" } else { "places" }
                ),
            )
        })
        .collect()
}

/// The stretch ids of a road of the network, junction pieces' `a|b` ids split.
fn stretch_parts(net: &PlacedNetwork, slot: usize) -> impl Iterator<Item = &str> {
    net.roads
        .get(slot)
        .into_iter()
        .flat_map(|r| r.slot.stretch.split('|'))
}

/// Where a strip passes closest to `target` on its way out from joint `from`, heading `forward` (towards higher
/// joint indices) or back, up to `reach` metres of strip: the point and the strip's height there.
fn strip_near(
    strip: &crate::profile::RoadStrip,
    from: usize,
    forward: bool,
    target: (f64, f64),
    reach: f64,
) -> Option<((f64, f64), f64)> {
    let mut best: Option<(f64, (f64, f64), f64)> = None;
    let mut walked = 0.0;
    let mut here = *strip.joints.get(from)?;
    let mut index = from;
    while walked <= reach {
        let stepped = if forward {
            index.checked_add(1)
        } else {
            index.checked_sub(1)
        };
        let Some((next_index, next)) =
            stepped.and_then(|i| strip.joints.get(i).map(|joint| (i, *joint)))
        else {
            break;
        };
        index = next_index;
        let (dx, dy) = (next.0 - here.0, next.1 - here.1);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 {
            (((target.0 - here.0) * dx + (target.1 - here.1) * dy) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let point = (here.0 + dx * t, here.1 + dy * t);
        let height = here.2 + (next.2 - here.2) * t;
        let d = (point.0 - target.0).hypot(point.1 - target.1);
        if best.is_none_or(|(b, _, _)| d < b) {
            best = Some((d, point, height));
        }
        walked += len2.sqrt();
        here = next;
    }
    best.map(|(_, point, height)| (point, height))
}

/// Where a strip first enters the circle `(centre, radius)` on its way out from joint `from`, heading `forward` or
/// back, within [`RAMP_JOINT_MIN_M`] of strip: the point, the strip's height there and how far along the strip it
/// is from joint `from`. `None` where it does not (or if joint `from` is already inside).
fn strip_enter(
    strip: &crate::profile::RoadStrip,
    from: usize,
    forward: bool,
    (centre, radius): ((f64, f64), f64),
) -> Option<((f64, f64), f64, f64)> {
    let mut walked = 0.0;
    let mut here = *strip.joints.get(from)?;
    let mut index = from;
    while walked < RAMP_JOINT_MIN_M {
        let stepped = if forward {
            index.checked_add(1)
        } else {
            index.checked_sub(1)
        };
        let (next_index, next) =
            stepped.and_then(|i| strip.joints.get(i).map(|joint| (i, *joint)))?;
        index = next_index;
        let (dx, dy) = (next.0 - here.0, next.1 - here.1);
        let qa = dx * dx + dy * dy;
        if qa > 0.0 {
            let (fx, fy) = (here.0 - centre.0, here.1 - centre.1);
            let qb = 2.0 * (fx * dx + fy * dy);
            let discriminant = qb * qb - 4.0 * qa * (fx * fx + fy * fy - radius * radius);
            if discriminant > 0.0 {
                let t = (-qb - discriminant.sqrt()) / (2.0 * qa);
                // a tolerance, as a joint may lie on the circle's edge
                if (-1e-9..=1.0 + 1e-9).contains(&t) {
                    let height = here.2 + (next.2 - here.2) * t;
                    return Some((
                        (here.0 + dx * t, here.1 + dy * t),
                        height,
                        walked + t * qa.sqrt(),
                    ));
                }
            }
        }
        walked += qa.sqrt();
        here = next;
    }
    None
}

/// What a deck's end joins, as the step to it (its height less the deck's, see [`EndStep::step`]): the junction
/// it belongs to within its reach, or the end of another road within [`END_JOIN_M`]. The one with the largest
/// step, if any has a finite height.
fn end_neighbour(
    net: &PlacedNetwork,
    profiles: &Profiles,
    junctions: &Junctions,
    own: (usize, usize, bool, bool),
    ports: ((f64, f64), (f64, f64)),
    deck: f64,
    ids: &[&str],
) -> Option<(f64, String, bool)> {
    let (port, other_port) = ports;
    let (chain_end, forward) = (own.2, own.3);
    let mut found: Vec<(f64, String, bool)> = Vec::new();
    // the junctions that belong to the bridge, with their distance from this end and from the deck's other end
    let named: Vec<(&crate::junctions::Junction, f64, f64)> = junctions
        .points
        .iter()
        .filter(|j| ids.iter().any(|id| j.stretches.contains(*id)))
        .map(|j| {
            (
                j,
                (j.at.0 - port.0).hypot(j.at.1 - port.1),
                (j.at.0 - other_port.0).hypot(j.at.1 - other_port.1),
            )
        })
        .collect();
    // The nearest one within its patch and, for the end of a chain, the reach of the end rule: what the height
    // solve holds the deck to (a deck inside a chain is held only within the patch). Beyond the patch it must
    // also be nearer this end than the deck's other end.
    let junction = named
        .iter()
        .filter(|(j, d, other)| {
            *d <= j.patch_m || (chain_end && *d <= j.patch_m + HOLD_M && *d < *other)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((j, _, _)) = junction {
        let under = j.kind == JunctionKind::Overlay && ids.iter().any(|id| j.lower.contains(*id));
        let height = j.height_at(port) - if under { OVERLAY_DROP_M } else { 0.0 };
        found.push((
            height - deck,
            format!("junction at ({:.0}, {:.0})", j.at.0, j.at.1),
            false,
        ));
    }
    if !chain_end {
        // A deck inside a chain is not held to a junction beyond its patch, but the strip may run through the
        // junction's patch there: the height solve holds it to the junction's plane (a crossing row) and may give
        // that up where keeping it costs more than the break (the vertical-bend rows next to the level deck). Compare the strip with the junction's plane where it passes
        // closest to the junction, inside the patch, for every junction of the bridge's roads within
        // [`CROSSING_REACH_M`] of the patch and nearer this end than the deck's other end.
        for (j, d, _) in named.iter().filter(|(j, d, other)| {
            *d > j.patch_m && *d <= j.patch_m + CROSSING_REACH_M && *d < *other
        }) {
            // where the height solve ties the strip to the plane: where it enters the patch if that is within
            // `RAMP_JOINT_MIN_M` of the deck's end (no joint fits there), else where it passes nearest
            let reached = profiles.strips.get(own.0).and_then(|strip| {
                strip_enter(strip, own.1, forward, (j.at, j.patch_m))
                    .filter(|(_, _, walked)| *walked < RAMP_JOINT_MIN_M)
                    .map(|(point, height, _)| (point, height))
                    .or_else(|| strip_near(strip, own.1, forward, j.at, d + j.patch_m))
                    .filter(|(point, _)| (point.0 - j.at.0).hypot(point.1 - j.at.1) <= j.patch_m)
            });
            let Some((point, strip_height)) = reached else {
                continue;
            };
            let under =
                j.kind == JunctionKind::Overlay && ids.iter().any(|id| j.lower.contains(*id));
            let height = j.height_at(point) - if under { OVERLAY_DROP_M } else { 0.0 };
            found.push((
                height - strip_height,
                format!("junction at ({:.0}, {:.0})", j.at.0, j.at.1),
                true,
            ));
        }
    }
    // a road that ends at the deck's end (another road, or the other end of the bridge's own road)
    for (index, strip) in profiles.strips.iter().enumerate() {
        let last = strip.joints.len().saturating_sub(1);
        for joint in [0, last] {
            if index == own.0 && joint == own.1 {
                continue;
            }
            let Some(at) = strip.joints.get(joint) else {
                continue;
            };
            if (at.0 - port.0).hypot(at.1 - port.1) <= END_JOIN_M {
                let id = strip
                    .slots
                    .first()
                    .and_then(|s| stretch_parts(net, *s).next())
                    .unwrap_or("?");
                found.push((at.2 - deck, format!("road {id}"), false));
            }
        }
    }
    found
        .into_iter()
        .filter(|(step, _, _)| step.is_finite())
        .max_by(|a, b| a.0.abs().total_cmp(&b.0.abs()))
}

/// The deck of a bridge at one end and what it joins there (see [`bridge_end_steps`]).
#[derive(Debug, Clone, PartialEq)]
pub struct EndStep {
    /// The id of the bridge stretch.
    pub stretch: String,
    /// Which end of the deck: 0 the first, 1 the last.
    pub end: usize,
    /// The end of the deck, in map metres.
    pub at: (f64, f64),
    /// The height of what it joins less the deck's height, in metres: positive where the deck is below it. For a deck
    /// inside a chain whose junction lies beyond the junction's patch, it is the junction's plane less the strip's
    /// height where the strip passes closest to the junction (inside the patch).
    pub step: f64,
    /// What it joins, for people.
    pub what: String,
    /// Whether the step is the road beyond the deck against the plane of a junction it runs through (the deck's end is
    /// not at that junction), not the deck against what it joins.
    pub crossing: bool,
}

/// Every end of a bridge's deck that joins a junction of the bridge's roads or the end of another road, with the
/// step to it (not only the ones over [`END_STEP_TOL_M`]).
///
/// A deck is level and one variable of the height solve. An end of it is compared with the junction of the
/// bridge's roads that the solve holds the deck to (within the junction's patch, and for the end of a chain also
/// [`HOLD_M`] beyond it if it is nearer this end than the deck's other end): the junction's plane (an overlay's lower
/// road is [`OVERLAY_DROP_M`] below it); or with a road that ends within 1.5 m of it. What is not joined
/// to the bridge is not compared. A deck inside a chain whose junction lies beyond the patch is not held to it,
/// but the strip beyond the deck may run through the patch (the height solve holds it to the junction's plane
/// there, and gives that up where keeping it costs more than the break, the vertical-bend rows next to the level
/// deck): then the strip's height where it passes closest to
/// the junction (inside the patch) is compared with the junction's plane there, for every junction of the bridge's
/// roads within [`CROSSING_REACH_M`] of the patch and nearer this end than the deck's other end (a break with a
/// junction farther than that beyond its patch is not compared); the plane is not
/// read beyond its patch at the deck's port (a ramp between the deck and the patch is `road_grade`'s).
/// Heights that are not numbers give nothing.
#[must_use]
pub fn bridge_end_steps(
    net: &PlacedNetwork,
    profiles: &Profiles,
    junctions: &Junctions,
) -> Vec<EndStep> {
    let mut out = Vec::new();
    for (index, strip) in profiles.strips.iter().enumerate() {
        let is_deck = |i: usize| strip.bridge.get(i).copied().unwrap_or(false);
        let segments = strip.joints.len().saturating_sub(1);
        let mut i = 0;
        while i < segments {
            if !is_deck(i) {
                i += 1;
                continue;
            }
            let first = i;
            while i < segments && is_deck(i) {
                i += 1;
            }
            // the deck runs from joint `first` to joint `i`
            let ids: Vec<&str> = (first..i)
                .filter_map(|k| strip.slots.get(k))
                .flat_map(|s| stretch_parts(net, *s))
                .collect();
            let name = ids.first().copied().unwrap_or("?");
            let (Some(start), Some(finish)) = (strip.joints.get(first), strip.joints.get(i)) else {
                continue;
            };
            for (end, joint, port, other) in [(0, first, start, finish), (1, i, finish, start)] {
                let chain_end = joint == 0 || joint + 1 == strip.joints.len();
                let at = (port.0, port.1);
                let ports = (at, (other.0, other.1));
                let Some((step, what, crossing)) = end_neighbour(
                    net,
                    profiles,
                    junctions,
                    (index, joint, chain_end, end == 1),
                    ports,
                    port.2,
                    &ids,
                ) else {
                    continue;
                };
                if step.is_finite() {
                    out.push(EndStep {
                        stretch: name.to_string(),
                        end,
                        at,
                        step,
                        what,
                        crossing,
                    });
                }
            }
        }
    }
    out
}

/// The message of a [`GradeExcess`]: a grade over its ceiling, a bend sharper than the vertical radius allows, or
/// a break of the grade over what a vehicle takes in a ramp zone.
fn excess_message(e: &GradeExcess) -> String {
    match e.kind {
        GradeExcessKind::Grade => format!(
            "Grade {:.0} % exceeds the limit of {:.0} %",
            e.grade * 100.0,
            e.limit * 100.0
        ),
        // the change of grade per metre is 1 / radius
        GradeExcessKind::VerticalBend => format!(
            "Crest or dip too sharp: vertical radius {:.0} m, the minimum is {:.0} m",
            1.0 / e.grade,
            1.0 / e.limit
        ),
        GradeExcessKind::GradeBreak => format!(
            "The grade changes by {:.0} % at one point, the most a vehicle takes there is {:.0} %",
            e.grade * 100.0,
            e.limit * 100.0
        ),
    }
}

/// Bridges whose deck ends at another height than the road or junction it joins: one violation for each such end
/// (see [`bridge_end_steps`]), at the end of the deck, where the step is over [`END_STEP_TOL_M`].
fn end_step_violations(
    net: &PlacedNetwork,
    profiles: &Profiles,
    junctions: &Junctions,
) -> Vec<Violation> {
    bridge_end_steps(net, profiles, junctions)
        .into_iter()
        .filter(|e| e.step.abs() > END_STEP_TOL_M)
        .map(|e| {
            let side = if e.step > 0.0 { "below" } else { "above" };
            let message = if e.crossing {
                format!(
                    "The road beyond the deck of bridge {} is {:.1} m {side} the plane of the {} where it enters its patch: nothing in the project sets heights, look at the site",
                    e.stretch,
                    e.step.abs(),
                    e.what
                )
            } else {
                format!(
                    "The deck of bridge {} ends {:.1} m {side} the {} it joins: nothing in the project sets heights, look at the site",
                    e.stretch,
                    e.step.abs(),
                    e.what
                )
            };
            violation(Rule::BridgeEndStep, e.at, message)
        })
        .collect()
}

/// Bridges whose posts, piers or walls do not reach the ground: one violation for each, where the foot is
/// furthest from it.
fn post_violations(floating: &[PostFloat]) -> Vec<Violation> {
    floating
        .iter()
        .map(|f| {
            violation(
                Rule::BridgePostFloats,
                f.at,
                format!(
                    "The posts of bridge {} stand {:.1} m above the ground at the furthest ({:.1} m is allowed): the model does not reach it",
                    f.stretch,
                    f.by_m + POST_FLOATS_M,
                    POST_FLOATS_M
                ),
            )
        })
        .collect()
}

/// A road left standing over water: one violation for each dip that is reported, where it is.
fn dip_violations(dips: &[(Dip, DipAction)]) -> Vec<Violation> {
    dips.iter()
        .filter(|(_, action)| *action == DipAction::Report)
        .map(|(dip, _)| {
            violation(
                Rule::RoadOverWater,
                dip.at,
                format!(
                    "Road crosses water {:.0} m below it without a bridge; look at it",
                    dip.depth_m
                ),
            )
        })
        .collect()
}

/// Sorts violations the way they are reported: by rule, then by position (x, then y). Deterministic, which is what
/// `build/report.json` and the findings rely on.
pub fn sort_violations(violations: &mut [Violation]) {
    violations.sort_by(|a, b| {
        a.rule
            .cmp(&b.rule)
            .then(a.at.0.total_cmp(&b.at.0))
            .then(a.at.1.total_cmp(&b.at.1))
    });
}

/// The class of a lot in words, for the findings.
fn class_word(class: Option<SurfaceClass>) -> &'static str {
    match class {
        Some(SurfaceClass::Tarmac) => "paved",
        Some(_) => "grass",
        None => "no",
    }
}

/// The findings of the lot stage: what became of every lot (the form, surface class and largest move of a smoothed
/// one, the reason of a surface-only or an excluded one), the steep and the move-limited ones with their numbers
/// against [`LOT_STEEP`] and [`LOT_MAX_MOVE_M`], and every road that is not built or is cut. The position is the
/// lot's centre ([`LotReport`]'s lots) or the place of the road.
#[must_use]
pub fn lot_violations(report: &LotReport, clean: &CleanReport) -> Vec<Violation> {
    let mut out = Vec::new();
    for lot in &report.assessed {
        let at = centre(&lot.area);
        let id = lot.area.id.as_str();
        let (one, _) = lot.area.kind.words();
        let class = class_word(report.class_of(id));
        match lot.outcome {
            Outcome::Smoothed { form } => {
                out.push(violation(
                    Rule::LotSmoothed,
                    at,
                    format!(
                        "Lot {id} ({one}) is smoothed: {form}, {class}, the ground moves by up to {:.2} m",
                        lot.move_max_m,
                        form = form.id()
                    ),
                ));
                if lot.steep {
                    out.push(violation(
                        Rule::LotSteep,
                        at,
                        format!(
                            "Lot {id} ({one}) keeps a slope of {:.0} % (95th percentile), more than the {:.0} % a                              vehicle can take across a road",
                            lot.slope_p95 * 100.0,
                            LOT_STEEP * 100.0
                        ),
                    ));
                }
                if let Some(lambda) = report.move_limited(lot) {
                    out.push(violation(
                        Rule::LotMoveLimited,
                        at,
                        format!(
                            "Lot {id} ({one}) would move the ground by {:.2} m, more than the {LOT_MAX_MOVE_M:.1} m a                              lot may; the change is scaled by lambda {lambda:.2}",
                            LOT_MAX_MOVE_M / lambda
                        ),
                    ));
                }
            }
            Outcome::SurfaceOnly { reason } => out.push(violation(
                Rule::LotSurfaceOnly,
                at,
                format!("Lot {id} ({one}) gets a {class} surface only ({reason}), its ground is not shaped"),
            )),
            Outcome::Excluded { reason } => out.push(violation(
                Rule::LotExcluded,
                at,
                format!("Lot {id} ({one}) is left alone ({reason}): no surface, no shaping"),
            )),
        }
    }
    for dropped in &clean.lot_roads {
        out.push(violation(
            Rule::RoadInLotDropped,
            dropped.at,
            format!(
                "Road {} inside lot {} is not built ({:.0} m)",
                dropped.stretch, dropped.lot, dropped.length_m
            ),
        ));
    }
    for entry in &clean.lot_access {
        out.push(violation(
            Rule::RoadInLotDropped,
            entry.entry,
            format!(
                "The access road {} of lot {} is cut {:.0} m inside the lot",
                entry.stretch, entry.lot, entry.cut_m
            ),
        ));
    }
    out
}

/// All the violations of a generated road network, sorted by rule and then by position.
///
/// `water` are the OSM areas of the project; only those with the priority of water count. Bridges may stand
/// in water. `dips` are the dips of the final placement with what was decided about each; a dip that is
/// reported is a violation.
#[must_use]
#[expect(clippy::too_many_arguments, reason = "one input per kind of violation")]
pub fn check_roads(
    net: &PlacedNetwork,
    profiles: &Profiles,
    junctions: &Junctions,
    conformed: &Conformed,
    pieces: &[PieceCheck],
    steep_paths: &[SteepPathSection],
    orphans: &[OrphanedEdit],
    water: &[LandPolygon],
    frame: &MapGeo,
    dips: &[(Dip, DipAction)],
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
    out.extend(pieces.iter().filter_map(far_from_ground));
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
    out.extend(
        profiles
            .excesses
            .iter()
            .map(|e| violation(Rule::RoadGrade, e.at, excess_message(e))),
    );
    out.extend(meeting_violations(net));
    out.extend(water_violations(net, water, frame));
    out.extend(conformed.conflicts.iter().map(|c| {
        violation(
            Rule::RoadConflict,
            c.at,
            if c.bridge {
                format!(
                    "The cut under a bridge's deck and a road want heights {:.1} m apart here",
                    c.height_difference_m
                )
            } else {
                format!(
                    "Two roads want heights {:.1} m apart here",
                    c.height_difference_m
                )
            },
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
            "Roads end here without a junction piece (the game has none for this pair)".to_string(),
        )
    }));
    out.extend(net.tunnels_omitted.iter().map(|(id, at)| {
        violation(
            Rule::TunnelOmitted,
            *at,
            format!("Tunnel {id} has no pieces: real tunnels need terrain holes"),
        )
    }));
    out.extend(clearance_violations(profiles));
    out.extend(duplicate_violations(&net.duplicates));
    out.extend(clean_violations(&net.clean));
    out.extend(smooth_violations(net.smooth.as_ref()));
    out.extend(junction_violations(junctions));
    out.extend(dip_violations(dips));
    out.extend(not_built_violations(net));
    out.extend(blocks_violations(net));
    out.extend(buried_violations(&conformed.buried, net, profiles));
    out.extend(post_violations(&conformed.floating));
    out.extend(end_step_violations(net, profiles, junctions));
    sort_violations(&mut out);
    out
}

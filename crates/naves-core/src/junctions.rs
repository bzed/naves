// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Junctions, their ground fit, and the links between them.
//!
//! A junction is a place where roads meet: a junction piece, a crossing where roads meet without a junction
//! piece, or a chain end that lies on another road. Each starts at the median of the original ground around it
//! and a plane through that height that tilts along the through road with the slope of the ground
//! ([`Junction::gradient`]): the ground fit, which is what the height solve ([`crate::profile::solve_clearing`])
//! pulls the junction's height and tilt toward, and the height and tilt of a junction that holds no joint or lies
//! on a ring. A link is the road between two neighbouring junctions, from the joint that holds one to the joint
//! that holds the other; the solve reports the links it could not keep under their ceiling in
//! [`Junctions::unresolved`]. The design is `docs/superpowers/specs/2026-11-13-road-heights-design.md`,
//! section 4, and `docs/superpowers/specs/2026-11-16-height-solve-design.md`.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::Family;
use crate::heightmap::Heightmap;
use crate::limits::{CROSS_FALL_AIM, grade_ceiling};
use crate::profile::{ON_ROAD_M, RoadStrip, chain_holds, meeting_bounds, meeting_on, rank};
use crate::roadnet::{Ring, RoadGraph};
use crate::roadplace::{Overlay, PlacedKind, PlacedNetwork};

/// Radius of the circle the ground is sampled in.
pub const GROUND_RADIUS_M: f64 = 10.0;
/// The least distance, in metres, along the through road either side of a junction that its tilt is measured
/// over: about the length over which the road's own profile changes its grade (research 9.8).
const MIN_TILT_REACH_M: f64 = 25.0;
/// The shortest stretch of road, in metres, that a tilt is fitted to (a junction near a chain end).
const MIN_TILT_SPAN_M: f64 = 4.0;
/// Spacing, in metres, of the ground samples along the through road that the tilt is fitted to.
const TILT_STEP_M: f64 = 1.0;
/// The share of the grade ceiling of the through road that a junction's tilt may reach. The tilt is also
/// never more than [`CROSS_FALL_AIM`].
pub const JUNCTION_TILT_SHARE: f64 = 0.9;
/// Junctions this close along a road are one.
pub const MERGE_M: f64 = 3.0;

/// How far beyond a junction's patch a chain end or a junction piece may lie and still belong to the junction:
/// the side ports of a junction piece lie as far from its centre as the patch reaches, and the centre of the
/// piece may be off the crossing by a few metres.
pub const HOLD_M: f64 = 4.0;

/// How far from a crossing a road may pass and still be one of its arms.
pub(crate) const ARM_REACH_M: f64 = 13.0;
/// An overlay of the placement belongs to the crossing of the graph within this distance.
const OVERLAY_MATCH_M: f64 = 1.0;
/// A projection this close to the end of a road is the end.
const END_M: f64 = 3.0;
/// A junction piece whose centre is this close to a station is the one the station lies on.
const PIECE_STATION_M: f64 = 4.0;
/// A chain end this close to a crossing's junction belongs to it.
const NEAR_CROSSING_M: f64 = 12.0;
/// Chain ends on another road this close together share a junction.
const END_CLUSTER_M: f64 = 6.0;
/// The shortest link, in metres.
const MIN_LINK_M: f64 = 1.0;

/// What kind of meeting a junction is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JunctionKind {
    /// A placed junction piece.
    Piece,
    /// Roads meet at a crossing without a junction piece and none of them lies on top (all end there).
    Join,
    /// Roads meet at a crossing without a junction piece: the higher road runs on, the lower roads end under
    /// it, [`crate::roadplace::OVERLAY_DROP_M`] below its surface.
    Overlay,
    /// A chain ends on another road without a crossing.
    EndOnRoad,
}

/// A place where roads meet, with one height for all of them.
#[derive(Debug, Clone, PartialEq)]
pub struct Junction {
    /// Position in map metres.
    pub at: (f64, f64),
    /// What meets here.
    pub kind: JunctionKind,
    /// Median of the original ground.
    pub ground: f64,
    /// The height the roads get: `ground` as found, the solved height after [`crate::profile::solve_clearing`].
    pub height: f64,
    /// Rank of the most important road meeting here.
    pub rank: u8,
    /// Radius of the patch the roads are held to the junction's plane over: half the length of the junction piece, or half the width of the widest
    /// road meeting here.
    pub patch_m: f64,
    /// The ids of the stretches of the roads that belong to this junction (they end here, cross here in the graph,
    /// or are the road a chain end lies on), junction pieces' `a|b` ids split. A chain that has none of them is
    /// not held to this junction, whatever its distance. Empty: not recorded, every chain counts.
    pub stretches: BTreeSet<String>,
    /// For an overlay, the ids of the stretches of the roads that lie under the top road here: they are held
    /// [`crate::roadplace::OVERLAY_DROP_M`] below the junction's height. Empty for the other kinds.
    pub lower: BTreeSet<String>,
    /// Rise per metre east and north of the plane the junction's joints are held to: the slope of the ground
    /// along the road that runs through, `(0, 0)` for a join, a ring and a junction without one. `height` is
    /// the height at `at`.
    pub gradient: (f64, f64),
    /// The unit vector along the road that runs through the junction, which its plane tilts along; `None`
    /// for a level junction (a join, a junction on a ring, a junction without a through road).
    pub tilt_axis: Option<(f64, f64)>,
    /// The steepest tilt of the plane along `tilt_axis`, rise per metre: [`JUNCTION_TILT_SHARE`] of the grade
    /// ceiling of the through road, at most [`CROSS_FALL_AIM`]; 0 where there is no axis.
    pub tilt_cap: f64,
}

impl Junction {
    /// The height of the junction's plane at `point`.
    #[must_use]
    pub fn height_at(&self, point: (f64, f64)) -> f64 {
        self.height
            + self.gradient.0 * (point.0 - self.at.0)
            + self.gradient.1 * (point.1 - self.at.1)
    }
}

/// The road between two neighbouring junctions.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    /// Index into [`Junctions::points`], the lower of the two.
    pub a: usize,
    /// Index into [`Junctions::points`], the higher of the two.
    pub b: usize,
    /// Road length along the chain between the two joints that are held to the two junctions, at least 1 m.
    pub length: f64,
    /// `grade_ceiling` of the road's family.
    pub ceiling: f64,
    /// Where the joint held to junction `a` lies.
    pub a_at: (f64, f64),
    /// Where the joint held to junction `b` lies.
    pub b_at: (f64, f64),
}

/// The plane of a roundabout: one tilted surface that follows the slope, which the ring, the ground inside it
/// and the places where roads join it lie on.
#[derive(Debug, Clone, PartialEq)]
pub struct RingPlane {
    /// The centre of the ring in map metres.
    pub at: (f64, f64),
    /// The radius of the surface: the ring's centre line and half its width.
    pub radius: f64,
    /// The inner edge of the surface: the centre line less half the width (not below 0). The plane is
    /// fitted to, and the terrain is shaped to, the band between `inner_radius` and `radius`.
    pub inner_radius: f64,
    /// The height of the plane at `at`.
    pub height: f64,
    /// Rise per metre to the east and to the north, at most [`grade_ceiling`] of the ring's family in all.
    pub gradient: (f64, f64),
    /// Rank of the ring's road.
    pub rank: u8,
    /// The ids of the stretches of the ring.
    pub stretches: BTreeSet<String>,
}

impl RingPlane {
    /// The height of the plane at `point`.
    #[must_use]
    pub fn height_at(&self, point: (f64, f64)) -> f64 {
        self.height
            + self.gradient.0 * (point.0 - self.at.0)
            + self.gradient.1 * (point.1 - self.at.1)
    }
}

/// The plane of `ring`: least squares over the original ground inside its surface (a 1 m grid through the
/// centre, the samples inside the map), with the tilt cut down to the grade ceiling of its family where it is
/// steeper. A ring with fewer than three samples or none that vary is level at the median.
#[must_use]
pub fn ring_plane(base: &Heightmap, ring: &Ring, graph: &RoadGraph) -> RingPlane {
    let half = ring.family.width_m() / 2.0;
    let radius = ring.radius + half;
    let inner_radius = (ring.radius - half).max(0.0);
    let extent = f64::from(base.size.saturating_sub(1)) * f64::from(base.cell_m);
    let inside = |v: f64| (0.0..=extent).contains(&v);
    let (mut n, mut sx, mut sy, mut sz) = (0.0, 0.0, 0.0, 0.0);
    let (mut sxx, mut syy, mut sxy, mut sxz, mut syz) = (0.0, 0.0, 0.0, 0.0, 0.0);
    // the ground under the band only, about 1 m apart: work grows with the ring's length, not its area
    let mut rho = inner_radius;
    while rho <= radius {
        let steps = (std::f64::consts::TAU * rho).ceil().max(8.0);
        let mut index = 0.0;
        while index < steps {
            let angle = std::f64::consts::TAU * index / steps;
            let (dx, dy) = (rho * angle.sin(), rho * angle.cos());
            let (east, north) = (ring.centre.0 + dx, ring.centre.1 + dy);
            if inside(east) && inside(north) {
                let z = f64::from(base.sample(east, north).height);
                n += 1.0;
                sx += dx;
                sy += dy;
                sz += z;
                sxx += dx * dx;
                syy += dy * dy;
                sxy += dx * dy;
                sxz += dx * z;
                syz += dy * z;
            }
            index += 1.0;
        }
        rho += 1.0;
    }
    // the normal equations of z = a + gx dx + gy dy, by Cramer's rule
    let det = n * (sxx * syy - sxy * sxy) - sx * (sx * syy - sxy * sy) + sy * (sx * sxy - sxx * sy);
    let level = ground_median(base, ring.centre, radius.min(GROUND_RADIUS_M));
    let (height, mut gradient) = if n >= 3.0 && det.abs() > 1e-9 {
        let a = (sz * (sxx * syy - sxy * sxy) - sx * (sxz * syy - sxy * syz)
            + sy * (sxz * sxy - sxx * syz))
            / det;
        let gx = (n * (sxz * syy - sxy * syz) - sz * (sx * syy - sxy * sy)
            + sy * (sx * syz - sxz * sy))
            / det;
        let gy = (n * (sxx * syz - sxz * sxy) - sx * (sx * syz - sxz * sy)
            + sz * (sx * sxy - sxx * sy))
            / det;
        (a, (gx, gy))
    } else {
        (level, (0.0, 0.0))
    };
    let ceiling = grade_ceiling(ring.family);
    let steepness = gradient.0.hypot(gradient.1);
    if steepness > ceiling {
        gradient = (
            gradient.0 / steepness * ceiling,
            gradient.1 / steepness * ceiling,
        );
    }
    RingPlane {
        at: ring.centre,
        radius,
        inner_radius,
        height,
        gradient,
        rank: rank(ring.family),
        stretches: ring
            .stretches
            .iter()
            .filter_map(|i| graph.stretches.get(*i))
            .map(|s| s.id.clone())
            .collect(),
    }
}

/// All junctions of a network and the links between them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Junctions {
    /// The planes of the roundabouts of the network: the junctions on a ring take their height from it.
    pub rings: Vec<RingPlane>,
    /// The junctions, sorted by position.
    pub points: Vec<Junction>,
    /// The links, sorted by their ends and length.
    pub links: Vec<Link>,
    /// Links still over their ceiling after the height solve, by index into `links`. From
    /// [`crate::profile::solve_clearing`]: the links of the junctions that hold an end of a segment the solve could
    /// not keep under its ceiling.
    pub unresolved: Vec<usize>,
}

/// `n` as a float; counts here are far below 2^32.
fn count(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// The median of the original ground in a circle of `radius` metres around `at`: the samples on a 1 m grid
/// through `at` that lie inside the circle and inside the map. With no sample inside, the height at `at`,
/// clamped to the map.
#[must_use]
pub fn ground_median(base: &Heightmap, at: (f64, f64), radius: f64) -> f64 {
    let mut offsets = vec![0.0];
    let mut d = 1.0;
    while d <= radius {
        offsets.push(d);
        offsets.push(-d);
        d += 1.0;
    }
    let extent = f64::from(base.size.saturating_sub(1)) * f64::from(base.cell_m);
    let inside = |v: f64| (0.0..=extent).contains(&v);
    let mut heights = Vec::new();
    for dx in &offsets {
        for dy in &offsets {
            let (x, y) = (at.0 + dx, at.1 + dy);
            if dx.hypot(*dy) <= radius && inside(x) && inside(y) {
                heights.push(f64::from(base.sample(x, y).height));
            }
        }
    }
    heights.sort_by(f64::total_cmp);
    let n = heights.len();
    let middle = |i: usize| heights.get(i).copied();
    let median = if n == 0 {
        None
    } else if n % 2 == 1 {
        middle(n / 2)
    } else {
        middle(n / 2 - 1)
            .zip(middle(n / 2))
            .map(|(low, high)| f64::midpoint(low, high))
    };
    median.unwrap_or_else(|| f64::from(base.sample(at.0, at.1).height))
}

/// One chain as a polyline, for finding where roads meet.
struct Line {
    /// The joints of the chain, at height 0, for [`meeting_on`].
    strip: RoadStrip,
    /// Stretch ids of the chain's roads, junction pieces' `a|b` ids split.
    ids: BTreeSet<String>,
    family: Family,
    /// Station of each joint along the polyline.
    stations: Vec<f64>,
    length: f64,
    bounds: [f64; 4],
}

impl Line {
    fn first(&self) -> (f64, f64) {
        self.strip.joints.first().map_or((0.0, 0.0), |j| (j.0, j.1))
    }

    fn last(&self) -> (f64, f64) {
        self.strip.joints.last().map_or((0.0, 0.0), |j| (j.0, j.1))
    }

    /// The nearest point of the polyline to `p`: its distance and station.
    fn project(&self, p: (f64, f64)) -> (f64, f64) {
        let (distance, station, _) = self.project_segment(p);
        (distance, station)
    }

    /// The unit heading of the polyline at the point nearest `p`.
    fn heading_near(&self, p: (f64, f64)) -> Option<(f64, f64)> {
        self.project_segment(p).2
    }

    /// [`Line::project`] and the unit heading of the segment the nearest point lies on.
    fn project_segment(&self, p: (f64, f64)) -> (f64, f64, Option<(f64, f64)>) {
        let mut best = (f64::INFINITY, 0.0, None);
        for (pair, stations) in self.strip.joints.windows(2).zip(self.stations.windows(2)) {
            let ([from, to], [s0, s1]) = (pair, stations) else {
                continue;
            };
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            let length2 = dx * dx + dy * dy;
            let t = if length2 > 0.0 {
                (((p.0 - from.0) * dx + (p.1 - from.1) * dy) / length2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let distance = (p.0 - from.0 - t * dx).hypot(p.1 - from.1 - t * dy);
            if distance < best.0 {
                best = (distance, s0 + t * (s1 - s0), unit((dx, dy)));
            }
        }
        best
    }

    /// The unit heading, start to end, of the junction piece that carries `station`.
    fn piece_axis(&self, station: f64) -> Option<(f64, f64)> {
        for (k, stations) in self.stations.windows(2).enumerate() {
            let [s0, s1] = stations else { continue };
            let piece = self.strip.junction.get(k).copied().unwrap_or(false);
            if piece && (f64::midpoint(*s0, *s1) - station).abs() < PIECE_STATION_M {
                let (from, to) = (self.strip.joints.get(k)?, self.strip.joints.get(k + 1)?);
                return unit((to.0 - from.0, to.1 - from.1));
            }
        }
        None
    }

    /// The point of the polyline at `station`, clamped to its ends.
    fn point_at(&self, station: f64) -> Option<(f64, f64)> {
        let mut last = None;
        for (pair, stations) in self.strip.joints.windows(2).zip(self.stations.windows(2)) {
            let ([from, to], [s0, s1]) = (pair, stations) else {
                continue;
            };
            let t = if s1 - s0 > 1e-9 {
                ((station - s0) / (s1 - s0)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let point = (from.0 + t * (to.0 - from.0), from.1 + t * (to.1 - from.1));
            if station <= *s1 {
                return Some(point);
            }
            last = Some(point);
        }
        last.or_else(|| self.strip.joints.first().map(|j| (j.0, j.1)))
    }

    /// Half the length of the junction piece that carries `station`, or 0.
    fn piece_half(&self, station: f64) -> f64 {
        for (k, stations) in self.stations.windows(2).enumerate() {
            let [s0, s1] = stations else { continue };
            let piece = self.strip.junction.get(k).copied().unwrap_or(false);
            if piece && (f64::midpoint(*s0, *s1) - station).abs() < PIECE_STATION_M {
                return 0.5 * (s1 - s0);
            }
        }
        0.0
    }

    /// The end a station is at (`false` the start, `true` the end), if it is at one.
    fn end_at(&self, station: f64) -> Option<bool> {
        if station <= END_M {
            Some(false)
        } else if station >= self.length - END_M {
            Some(true)
        } else {
            None
        }
    }

    fn end_point(&self, end: bool) -> (f64, f64) {
        if end { self.last() } else { self.first() }
    }

    fn end_station(&self, end: bool) -> f64 {
        if end { self.length } else { 0.0 }
    }
}

/// The chains of `net` as polylines, in an order that does not depend on the order of the input.
fn lines_of(net: &PlacedNetwork) -> Vec<Line> {
    let mut lines = Vec::new();
    for chain in &net.chains {
        let roads: Vec<_> = chain.iter().filter_map(|i| net.roads.get(*i)).collect();
        let Some(first) = roads.first() else { continue };
        let mut joints = vec![(first.start.x, first.start.y, 0.0)];
        joints.extend(roads.iter().map(|r| (r.end.x, r.end.y, 0.0)));
        let mut stations = vec![0.0];
        for pair in joints.windows(2) {
            if let [from, to] = pair {
                let last = stations.last().copied().unwrap_or(0.0);
                stations.push(last + (to.0 - from.0).hypot(to.1 - from.1));
            }
        }
        let strip = RoadStrip {
            slots: chain.clone(),
            joints,
            bridge: roads.iter().map(|r| r.kind == PlacedKind::Bridge).collect(),
            junction: roads
                .iter()
                .map(|r| r.kind == PlacedKind::Junction)
                .collect(),
            side_slope: Vec::new(),
            half_width: first.width / 2.0,
            rank: rank(first.family),
        };
        let ids = roads
            .iter()
            .flat_map(|r| r.slot.stretch.split('|').map(str::to_string))
            .collect();
        lines.push(Line {
            bounds: meeting_bounds(&strip),
            strip,
            ids,
            family: first.family,
            length: stations.last().copied().unwrap_or(0.0),
            stations,
        });
    }
    lines.sort_by(|a, b| {
        let key = |l: &Line| (l.first(), l.last());
        let (ka, kb) = (key(a), key(b));
        ka.0.0
            .total_cmp(&kb.0.0)
            .then(ka.0.1.total_cmp(&kb.0.1))
            .then(ka.1.0.total_cmp(&kb.1.0))
            .then(ka.1.1.total_cmp(&kb.1.1))
            .then(a.length.total_cmp(&b.length))
            .then(a.ids.cmp(&b.ids))
    });
    lines
}

/// A road at a junction: where along its chain, and half the length of the junction piece it carries there.
#[derive(Debug, Clone, Copy)]
struct Arm {
    line: usize,
    station: f64,
    half: f64,
}

/// A junction before its height and the merging.
struct Raw {
    at: (f64, f64),
    kind: JunctionKind,
    arms: Vec<Arm>,
    /// Of an overlay: the stretches that lie under the top road.
    lower: BTreeSet<String>,
}

fn kind_order(kind: JunctionKind) -> u8 {
    match kind {
        JunctionKind::Piece => 0,
        JunctionKind::Overlay => 1,
        JunctionKind::Join => 2,
        JunctionKind::EndOnRoad => 3,
    }
}

fn by_position(a: &(f64, f64), b: &(f64, f64)) -> std::cmp::Ordering {
    a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))
}

/// `v` scaled to length 1, or none for a vector too short to have a direction.
fn unit(v: (f64, f64)) -> Option<(f64, f64)> {
    let length = v.0.hypot(v.1);
    (length.is_finite() && length > 1e-9).then(|| (v.0 / length, v.1 / length))
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// The junctions at the crossings of the graph where three or more roads meet, and the chain ends they
/// take (`claimed`).
fn crossing_junctions(
    graph: &RoadGraph,
    overlays: &[Overlay],
    lines: &[Line],
    claimed: &mut BTreeSet<(usize, bool)>,
) -> Vec<Raw> {
    let mut by_id: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, line) in lines.iter().enumerate() {
        for id in &line.ids {
            by_id.entry(id.as_str()).or_default().push(index);
        }
    }
    let mut out = Vec::new();
    for crossing in &graph.crossings {
        let stretches: Vec<_> = crossing
            .stretches
            .iter()
            .filter_map(|i| graph.stretches.get(*i))
            .filter(|s| !s.tunnel)
            .collect();
        if stretches.len() < 3 {
            continue;
        }
        let candidates: BTreeSet<usize> = stretches
            .iter()
            .filter_map(|s| by_id.get(s.id.as_str()))
            .flatten()
            .copied()
            .collect();
        let mut arms = Vec::new();
        let mut ends = Vec::new();
        for index in candidates {
            let Some(line) = lines.get(index) else {
                continue;
            };
            let (reach, station) = line.project(crossing.point);
            if reach > ARM_REACH_M {
                continue;
            }
            match line.end_at(station) {
                Some(end) => {
                    ends.push((index, end));
                    arms.push(Arm {
                        line: index,
                        station: line.end_station(end),
                        half: 0.0,
                    });
                }
                None => arms.push(Arm {
                    line: index,
                    station,
                    half: line.piece_half(station),
                }),
            }
        }
        let distinct: BTreeSet<usize> = arms.iter().map(|a| a.line).collect();
        if distinct.len() < 2 {
            continue;
        }
        claimed.extend(ends);
        let overlay = overlays
            .iter()
            .find(|o| distance(o.at, crossing.point) < OVERLAY_MATCH_M);
        let (kind, lower) = if arms.iter().any(|a| a.half > 0.0) {
            (JunctionKind::Piece, BTreeSet::new())
        } else if let Some(o) = overlay {
            (JunctionKind::Overlay, o.lower.iter().cloned().collect())
        } else {
            (JunctionKind::Join, BTreeSet::new())
        };
        out.push(Raw {
            at: crossing.point,
            kind,
            arms,
            lower,
        });
    }
    out
}

/// A chain end that lies on another road: the end, its position, and the other road.
struct OnRoad {
    line: usize,
    end: bool,
    at: (f64, f64),
    owner: usize,
}

/// Adds the junctions of chain ends that lie on another road without a crossing: to the crossing junction
/// within [`NEAR_CROSSING_M`] if there is one, else to a junction of their own.
fn end_junctions(lines: &[Line], claimed: &BTreeSet<(usize, bool)>, raw: &mut Vec<Raw>) {
    let mut found: Vec<OnRoad> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        for end in [false, true] {
            if claimed.contains(&(index, end)) {
                continue;
            }
            let at = line.end_point(end);
            let owner = lines
                .iter()
                .enumerate()
                .filter(|(other, l)| {
                    *other != index
                        && (l.bounds[0]..=l.bounds[2]).contains(&at.0)
                        && (l.bounds[1]..=l.bounds[3]).contains(&at.1)
                })
                .filter_map(|(other, l)| {
                    meeting_on(&l.strip, other, at, ON_ROAD_M, ON_ROAD_M)
                        .map(|(distance, _)| (distance, other))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            if let Some((_, owner)) = owner {
                found.push(OnRoad {
                    line: index,
                    end,
                    at,
                    owner,
                });
            }
        }
    }
    found.sort_by(|a, b| {
        by_position(&a.at, &b.at)
            .then(a.line.cmp(&b.line))
            .then(a.end.cmp(&b.end))
    });
    let crossings = raw.len();
    for item in found {
        let Some(line) = lines.get(item.line) else {
            continue;
        };
        let own = Arm {
            line: item.line,
            station: line.end_station(item.end),
            half: 0.0,
        };
        let near_crossing = raw
            .iter()
            .enumerate()
            .take(crossings)
            .map(|(i, r)| (i, distance(r.at, item.at)))
            .filter(|(_, d)| *d < NEAR_CROSSING_M)
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
            .map(|(i, _)| i);
        let own_junction = raw
            .iter()
            .enumerate()
            .skip(crossings)
            .find(|(_, r)| distance(r.at, item.at) < END_CLUSTER_M)
            .map(|(i, _)| i);
        if let Some(target) = near_crossing {
            if let Some(r) = raw.get_mut(target) {
                r.arms.push(own);
            }
            continue;
        }
        let Some(owner) = lines.get(item.owner) else {
            continue;
        };
        let (_, station) = owner.project(item.at);
        let on = Arm {
            line: item.owner,
            station: owner
                .end_at(station)
                .map_or(station, |end| owner.end_station(end)),
            half: 0.0,
        };
        match own_junction.and_then(|i| raw.get_mut(i)) {
            Some(r) => {
                r.arms.push(own);
                if !r.arms.iter().any(|a| a.line == item.owner) {
                    r.arms.push(on);
                }
            }
            None => raw.push(Raw {
                at: item.at,
                kind: JunctionKind::EndOnRoad,
                arms: vec![own, on],
                lower: BTreeSet::new(),
            }),
        }
    }
}

/// A link before merging: indices into the raw junctions.
struct RawLink {
    a: usize,
    b: usize,
    length: f64,
    ceiling: f64,
}

/// The links along every chain, between the junctions on it in the order of their stations. Junction pieces
/// are taken off the length at both ends; the free ends of a chain are not junctions and carry no link.
fn raw_links(lines: &[Line], raw: &[Raw]) -> Vec<RawLink> {
    let mut per_line: Vec<Vec<(f64, usize, f64)>> = vec![Vec::new(); lines.len()];
    for (junction, r) in raw.iter().enumerate() {
        for arm in &r.arms {
            if let Some(entries) = per_line.get_mut(arm.line) {
                entries.push((arm.station, junction, arm.half));
            }
        }
    }
    let mut out = Vec::new();
    for (line, entries) in lines.iter().zip(per_line.iter_mut()) {
        entries.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        entries.dedup_by_key(|e| e.1);
        for pair in entries.windows(2) {
            let [(sa, a, half_a), (sb, b, half_b)] = pair else {
                continue;
            };
            if a == b {
                continue;
            }
            out.push(RawLink {
                a: *a.min(b),
                b: *a.max(b),
                length: ((sb - half_b) - (sa + half_a)).max(MIN_LINK_M),
                ceiling: grade_ceiling(line.family),
            });
        }
    }
    out
}

fn find_root(parent: &[usize], mut i: usize) -> usize {
    while let Some(p) = parent.get(i).copied() {
        if p == i {
            break;
        }
        i = p;
    }
    i
}

/// The junctions of the network, each at the median of the original ground, with the links between them.
/// Junctions are sorted by position (x, then y) and links by their ends and length, so the result does not
/// depend on the order of the roads.
#[must_use]
pub fn find_junctions(net: &PlacedNetwork, graph: &RoadGraph, base: &Heightmap) -> Junctions {
    let lines = lines_of(net);
    let mut claimed = BTreeSet::new();
    let mut raw = crossing_junctions(graph, &net.overlays, &lines, &mut claimed);
    end_junctions(&lines, &claimed, &mut raw);
    raw.sort_by(|a, b| by_position(&a.at, &b.at).then(kind_order(a.kind).cmp(&kind_order(b.kind))));
    let rings: Vec<RingPlane> = net
        .rings
        .iter()
        .map(|ring| ring_plane(base, ring, graph))
        .collect();
    let mut junctions = merged(&lines, &raw, &raw_links(&lines, &raw), base, &rings);
    junctions.links = held_links(net, f64::from(base.cell_m), &junctions);
    junctions
}

/// The links of the junctions as the roads hold them: along every chain, the road between two consecutive
/// joints that are held (by [`chain_holds`], the rules the height solve uses) to different junctions. Its
/// length is the length along the chain between the two joints, at least 1 m, so that the grade of a link
/// is the grade between the places where the heights are actually held. Sorted by ends and
/// length, which does not depend on the order of the roads.
fn held_links(net: &PlacedNetwork, cell: f64, junctions: &Junctions) -> Vec<Link> {
    let mut out = Vec::new();
    for chain in &net.chains {
        let Some(family) = chain
            .first()
            .and_then(|i| net.roads.get(*i))
            .map(|r| r.family)
        else {
            continue;
        };
        let holds = chain_holds(net, chain, cell, junctions);
        let mut along = 0.0;
        let mut previous: Option<((f64, f64), usize, f64)> = None;
        let mut last: Option<(f64, f64)> = None;
        for (at, hold) in &holds {
            if let Some(from) = last {
                along += distance(from, *at);
            }
            last = Some(*at);
            let Some(hold) = hold else { continue };
            if let Some((from, junction, station)) = previous
                && junction != hold.junction
            {
                // a is the lower junction index, with the joint that is held to it
                let (a_at, b_at) = if junction < hold.junction {
                    (from, *at)
                } else {
                    (*at, from)
                };
                out.push(Link {
                    a: junction.min(hold.junction),
                    b: junction.max(hold.junction),
                    length: (along - station).max(MIN_LINK_M),
                    ceiling: grade_ceiling(family),
                    a_at,
                    b_at,
                });
            }
            previous = Some((*at, hold.junction, along));
        }
    }
    out.sort_by(|x, y| {
        x.a.cmp(&y.a)
            .then(x.b.cmp(&y.b))
            .then(x.length.total_cmp(&y.length))
            .then(x.ceiling.total_cmp(&y.ceiling))
    });
    out
}

/// The road that runs through a raw junction: its line, the station on it nearest the junction, and its unit
/// heading there. A junction piece's own axis at the piece, the top road of an overlay, the road a chain end
/// lies on. A join has none.
fn through_road<'a>(lines: &'a [Line], raw: &Raw) -> Option<(&'a Line, f64, (f64, f64))> {
    match raw.kind {
        JunctionKind::Piece => {
            let arm = raw
                .arms
                .iter()
                .filter(|a| a.half > 0.0)
                .max_by(|a, b| a.half.total_cmp(&b.half))?;
            let line = lines.get(arm.line)?;
            Some((line, arm.station, line.piece_axis(arm.station)?))
        }
        JunctionKind::Overlay => {
            let line = raw
                .arms
                .iter()
                .filter_map(|a| lines.get(a.line))
                .find(|l| l.ids.is_disjoint(&raw.lower))?;
            Some((line, line.project(raw.at).1, line.heading_near(raw.at)?))
        }
        JunctionKind::EndOnRoad => {
            let line = lines.get(raw.arms.get(1)?.line)?;
            Some((line, line.project(raw.at).1, line.heading_near(raw.at)?))
        }
        JunctionKind::Join => None,
    }
}

/// The ground at `point` of the original heightmap, if the point is inside the map and the height is a number.
fn ground_at(base: &Heightmap, point: (f64, f64)) -> Option<f64> {
    let extent = f64::from(base.size.saturating_sub(1)) * f64::from(base.cell_m);
    let inside = |v: f64| (0.0..=extent).contains(&v);
    let height = f64::from(base.sample(point.0, point.1).height);
    (inside(point.0) && inside(point.1) && height.is_finite()).then_some(height)
}

/// The slope of the original ground under the road `line`, `reach` metres either side of `station`: the
/// least-squares line through samples every [`TILT_STEP_M`] along the road (where the road goes on; a chain
/// end cuts it short). None where a sample is outside the map or not a number, or the road is too short there.
fn ground_slope_along(base: &Heightmap, line: &Line, station: f64, reach: f64) -> Option<f64> {
    let mut samples = Vec::new();
    let mut offset = -reach;
    while offset <= reach + 1e-9 {
        let along = station + offset;
        if (0.0..=line.length).contains(&along) {
            samples.push((offset, ground_at(base, line.point_at(along)?)?));
        }
        offset += TILT_STEP_M;
    }
    let n = count(samples.len());
    let mean_d = samples.iter().map(|s| s.0).sum::<f64>() / n.max(1.0);
    let mean_z = samples.iter().map(|s| s.1).sum::<f64>() / n.max(1.0);
    let spread: f64 = samples.iter().map(|s| (s.0 - mean_d).powi(2)).sum();
    let covariance: f64 = samples
        .iter()
        .map(|s| (s.0 - mean_d) * (s.1 - mean_z))
        .sum();
    let span = samples.last().map_or(0.0, |l| l.0) - samples.first().map_or(0.0, |f| f.0);
    (span >= MIN_TILT_SPAN_M && spread > 0.0).then(|| covariance / spread)
}

/// The tilt of a junction's plane: the slope of the original ground under the through road, fitted over `r`
/// metres either side of the junction (`r` is its patch radius, at least [`MIN_TILT_REACH_M`]), cut down to
/// [`JUNCTION_TILT_SHARE`] of the grade ceiling of the road's family and to [`CROSS_FALL_AIM`]. The plane has
/// no fall across the through road. Its fall along the through road is the fall across every road that meets
/// it there, and a road has no fall across of its own: on a steeper tilt the roads that meet would lie on the
/// through road's surface only along their centre lines.
fn junction_gradient(
    base: &Heightmap,
    radius: f64,
    through: (&Line, f64, (f64, f64)),
) -> (f64, f64) {
    let (line, station, t) = through;
    let Some(slope) = ground_slope_along(base, line, station, radius.max(MIN_TILT_REACH_M)) else {
        return (0.0, 0.0);
    };
    let limit = tilt_cap(line.family);
    let s = slope.clamp(-limit, limit);
    (s * t.0, s * t.1)
}

/// The steepest tilt of a junction whose through road is of `family`: [`JUNCTION_TILT_SHARE`] of its grade
/// ceiling, at most [`CROSS_FALL_AIM`].
fn tilt_cap(family: Family) -> f64 {
    (JUNCTION_TILT_SHARE * grade_ceiling(family)).min(CROSS_FALL_AIM)
}

/// The junction that the raw junctions `members` make: at the mean of their points, the first kind, the
/// highest rank and the largest patch.
fn one_junction(
    lines: &[Line],
    raw: &[Raw],
    members: &[usize],
    base: &Heightmap,
    rings: &[RingPlane],
) -> Junction {
    let width_of = |arms: &[Arm]| {
        arms.iter()
            .filter_map(|a| lines.get(a.line))
            .map(|l| l.strip.half_width)
            .fold(0.0, f64::max)
    };
    let (mut sx, mut sy) = (0.0, 0.0);
    let mut kind = JunctionKind::EndOnRoad;
    let (mut top_rank, mut patch) = (0, 0.0_f64);
    let mut stretches = BTreeSet::new();
    let mut lower = BTreeSet::new();
    for r in members.iter().filter_map(|m| raw.get(*m)) {
        sx += r.at.0;
        sy += r.at.1;
        if kind_order(r.kind) < kind_order(kind) {
            kind = r.kind;
        }
        top_rank = r
            .arms
            .iter()
            .filter_map(|a| lines.get(a.line))
            .map(|l| rank(l.family))
            .fold(top_rank, u8::max);
        let own_patch = if r.kind == JunctionKind::Piece {
            r.arms.iter().map(|a| a.half).fold(0.0, f64::max)
        } else {
            width_of(&r.arms)
        };
        patch = patch.max(own_patch);
        for line in r.arms.iter().filter_map(|a| lines.get(a.line)) {
            stretches.extend(line.ids.iter().cloned());
        }
        lower.extend(r.lower.iter().cloned());
    }
    let n = count(members.len());
    let at = (sx / n, sy / n);
    // a junction on a ring lies on the ring's plane, not on the ground
    let ring = rings
        .iter()
        .find(|ring| !ring.stretches.is_disjoint(&stretches));
    let ground = ring.map_or_else(
        || ground_median(base, at, GROUND_RADIUS_M),
        |ring| ring.height_at(at),
    );
    // the tilt follows the through road of the first member of the merged kind that has one; a junction on a
    // ring is held by the ring's plane
    let through = if ring.is_some() {
        None
    } else {
        members
            .iter()
            .filter_map(|m| raw.get(*m))
            .filter(|r| r.kind == kind)
            .find_map(|r| through_road(lines, r))
    };
    let (gradient, tilt_axis, cap) = through.map_or(((0.0, 0.0), None, 0.0), |through| {
        (
            junction_gradient(base, patch, through),
            Some(through.2),
            tilt_cap(through.0.family),
        )
    });
    Junction {
        at,
        kind,
        ground,
        height: ground,
        rank: top_rank,
        patch_m: patch,
        stretches,
        lower: if kind == JunctionKind::Overlay {
            lower
        } else {
            BTreeSet::new()
        },
        gradient,
        tilt_axis,
        tilt_cap: cap,
    }
}

/// The raw junctions with those joined by a link of [`MERGE_M`] or less made one, at the median of the
/// ground, and the links between what is left, sorted.
fn merged(
    lines: &[Line],
    raw: &[Raw],
    raw_links: &[RawLink],
    base: &Heightmap,
    rings: &[RingPlane],
) -> Junctions {
    // junctions joined by a link of MERGE_M or less are one
    let mut parent: Vec<usize> = (0..raw.len()).collect();
    for link in raw_links.iter().filter(|l| l.length <= MERGE_M) {
        let (ra, rb) = (find_root(&parent, link.a), find_root(&parent, link.b));
        if let Some(p) = parent.get_mut(ra.max(rb)) {
            *p = ra.min(rb);
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..raw.len() {
        groups.entry(find_root(&parent, i)).or_default().push(i);
    }
    let mut merged: Vec<Junction> = Vec::new();
    let mut group_of = vec![0; raw.len()];
    for (group, members) in groups.values().enumerate() {
        for member in members {
            if let Some(slot) = group_of.get_mut(*member) {
                *slot = group;
            }
        }
        merged.push(one_junction(lines, raw, members, base, rings));
    }

    // sorted by position, and the links with them
    let mut order: Vec<usize> = (0..merged.len()).collect();
    order.sort_by(|a, b| {
        let at = |i: &usize| merged.get(*i).map_or((0.0, 0.0), |m| m.at);
        by_position(&at(a), &at(b))
    });
    let mut new_index = vec![0; merged.len()];
    for (new, old) in order.iter().enumerate() {
        if let Some(slot) = new_index.get_mut(*old) {
            *slot = new;
        }
    }
    let points: Vec<Junction> = order
        .iter()
        .filter_map(|i| merged.get(*i).cloned())
        .collect();
    let target = |raw_index: usize| {
        group_of
            .get(raw_index)
            .and_then(|g| new_index.get(*g))
            .copied()
    };
    let mut out_links: Vec<Link> = raw_links
        .iter()
        .filter_map(|l| {
            let (a, b) = (target(l.a)?, target(l.b)?);
            let (low, high) = (a.min(b), a.max(b));
            (a != b).then(|| Link {
                a: low,
                b: high,
                length: l.length,
                ceiling: l.ceiling,
                a_at: points.get(low).map_or((0.0, 0.0), |p| p.at),
                b_at: points.get(high).map_or((0.0, 0.0), |p| p.at),
            })
        })
        .collect();
    out_links.sort_by(|x, y| {
        x.a.cmp(&y.a)
            .then(x.b.cmp(&y.b))
            .then(x.length.total_cmp(&y.length))
            .then(x.ceiling.total_cmp(&y.ceiling))
    });
    Junctions {
        rings: rings.to_vec(),
        points,
        links: out_links,
        unresolved: Vec::new(),
    }
}

impl Junctions {
    /// The junction whose patch holds `point`, the nearest if several.
    #[must_use]
    pub fn at(&self, point: (f64, f64)) -> Option<&Junction> {
        self.near(point, 0.0)
    }

    /// The nearest junction within `reach` metres beyond whose patch `point` lies; `at` is `near` with a
    /// reach of 0. Chain ends and junction pieces use [`HOLD_M`].
    #[must_use]
    pub fn near(&self, point: (f64, f64), reach: f64) -> Option<&Junction> {
        self.index_near(point, reach)
            .and_then(|i| self.points.get(i))
    }

    /// The index in [`Junctions::points`] of what [`Junctions::near`] finds.
    #[must_use]
    fn index_near(&self, point: (f64, f64), reach: f64) -> Option<usize> {
        self.points
            .iter()
            .enumerate()
            .map(|(i, j)| (distance(j.at, point), i, j))
            .filter(|(d, _, j)| *d <= j.patch_m + reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, i, _)| i)
    }
}

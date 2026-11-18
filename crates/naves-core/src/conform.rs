// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Shapes the terrain to fit the roads.
//!
//! The terrain is not smoothed towards a road. It is **held to a slope limit around it, at every grid
//! vertex**: what sticks out above the cutting slope is cut, what is below the embankment slope is filled.
//! Every vertex within half a road width and one and a half cells of a road's centre line takes the road's
//! height, so that every terrain triangle under the road has all three corners on it, whatever way the
//! engine splits a cell. Where the DEM is too coarse to show a valley the road crosses, the road gets a
//! bench.
//!
//! The ground under a bridge span is shaped for the bridge ([`conform_bridges`]): cut to a ceiling under the
//! deck (never to the foot of what stands), cut open to both sides, not cut within the end guard of a free end,
//! never raised beyond it, and never changed where another road's core holds it (see [`Buried`]).
//!
//! The earthworks reach only as far as they are connected to the road: a flood from the road through the
//! vertices that break the limits stops where the ground meets the slope again, so that a cliff beyond that
//! stays a cliff.

use std::collections::{BTreeMap, VecDeque};

use crate::bridgeground::BridgeSpan;
use crate::bridgekind::samples;
use crate::catalog::{Ceiling, SLICE_M};
use crate::heightmap::Heightmap;
use crate::profile::RoadStrip;

/// Rise over run of the gentlest cutting: 45°. A cutting on a steeper hillside may be steeper, up to
/// [`CUT_SLOPE_MAX`].
pub const CUT_SLOPE: f64 = 1.0;
/// Rise over run of the steepest cutting: 63°.
pub const CUT_SLOPE_MAX: f64 = 2.0;
/// Rise over run of the gentlest embankment: 34°. An embankment on a steeper hillside may be steeper, up to
/// [`FILL_SLOPE_MAX`].
pub const FILL_SLOPE: f64 = 0.667;
/// Rise over run of the steepest embankment: 45°. It stays below the walk limit, so the downhill side of a
/// road can always be left on foot.
pub const FILL_SLOPE_MAX: f64 = 1.0;
/// How far earthworks reach from a road's core, in metres.
pub const MAX_EARTHWORK_M: f64 = 60.0;
/// Side slope above which the uphill side of a road may be a face.
pub const FACE_SIDE_SLOPE: f64 = 0.35;
/// Rise over run of a face: steeper than any grid cell can show.
pub const FACE_SLOPE: f64 = 50.0;
/// Cores of different roads that differ by more than this are a conflict.
const CONFLICT_M: f64 = 0.3;
/// Passes of averaging over the changed vertices.
const ROUNDING_PASSES: usize = 4;
/// Impossible limits are reported once per square of this many metres.
const CONFLICT_REGION_M: f64 = 50.0;
/// The air kept between the ground and the underside of a bridge's deck, in metres.
pub const BRIDGE_AIR_M: f64 = 1.0;
/// A post, pier or wall foot that stands further than this above the ground does not reach it, in metres.
pub const POST_FLOATS_M: f64 = 0.5;
/// Two parts of a bridge whose ports are this close are joined there, in metres.
const PORT_JOIN_M: f64 = 1.0;
/// A vertex this close to a port's line is on it, in metres.
const PORT_LINE_M: f64 = 1e-6;
/// Ground less than this above the underside of a deck is not buried (rounding), in metres.
const BURIED_TOL_M: f64 = 0.1;

/// A circle of ground levelled to one plane, where roads meet without a junction piece or a roundabout lies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelPatch {
    /// Centre in map metres.
    pub at: (f64, f64),
    /// Radius in metres; every vertex within it takes the height of the plane.
    pub radius: f64,
    /// The height of the patch at its centre.
    pub height: f64,
    /// Rise per metre to the east and to the north; `(0.0, 0.0)` is a level patch. A vertex takes
    /// `height + gradient.0 * dx + gradient.1 * dy` for its offset `(dx, dy)` from the centre.
    pub gradient: (f64, f64),
    /// Vertices nearer to the centre than this are not claimed: 0 is a disc, more is a ring-shaped band (the
    /// ground inside a roundabout keeps its height).
    pub inner_radius: f64,
    /// Rank of the most important road meeting here; wins ties against a road of a lower rank.
    pub rank: u8,
}

/// Two roads that want different heights at the same place.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadConflict {
    /// Where.
    pub at: (f64, f64),
    /// By how much the heights differ.
    pub height_difference_m: f64,
    /// Whether one side is the cut under a bridge's deck (the other is a road).
    pub bridge: bool,
}

/// The shaped terrain and what it took.
#[derive(Debug, Clone, PartialEq)]
pub struct Conformed {
    /// The terrain with the earthworks.
    pub heightmap: Heightmap,
    /// The deepest cut.
    pub max_cut_m: f32,
    /// The highest fill.
    pub max_fill_m: f32,
    /// Places where two roads cannot both be met.
    pub conflicts: Vec<RoadConflict>,
    /// Places where the earthworks hit [`MAX_EARTHWORK_M`] and a step is left.
    pub capped: Vec<(f64, f64)>,
    /// Bridges with ground above the underside of their deck that the shaping could not cut (another road's core
    /// stops the cut), one for each bridge, in the order of their ids.
    pub buried: Vec<Buried>,
    /// Bridges whose posts, piers or walls stand more than [`POST_FLOATS_M`] above the shaped ground, one for each
    /// bridge, in the order of their ids.
    pub floating: Vec<PostFloat>,
}

/// Ground that another road's core keeps above the underside of a bridge's deck.
#[derive(Debug, Clone, PartialEq)]
pub struct Buried {
    /// The id of the bridge stretch.
    pub stretch: String,
    /// The vertex with the most ground above the reference, in map metres.
    pub at: (f64, f64),
    /// How far the ground there is above the reference, in metres: beyond the end guard the underside of the deck
    /// (the deck less what hangs below it there), within the guard of a free end its surface.
    pub above_m: f64,
    /// How many vertices within the deck's width are above the reference.
    pub vertices: usize,
    /// What holds the ground at the worst vertex.
    pub holder: Holder,
    /// Whether the worst vertex lies in the end guard of a free end, so that `above_m` is measured from the deck's
    /// surface and not from its underside.
    pub guarded: bool,
}

/// What holds the ground that another road keeps under a bridge's deck.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Holder {
    /// The core of a road: the index of its strip.
    Strip(usize),
    /// A level patch (where roads meet without a junction piece), at this place.
    Patch((f64, f64)),
    /// No road: the ground was not cut (within the end guard, or under a deck shorter than two guards).
    None,
}

/// Posts, piers or walls of a bridge that do not reach the ground.
#[derive(Debug, Clone, PartialEq)]
pub struct PostFloat {
    /// The id of the bridge stretch.
    pub stretch: String,
    /// The sample under the foot that is furthest from the ground, in map metres.
    pub at: (f64, f64),
    /// How far the foot is above the ground there, less [`POST_FLOATS_M`], in metres.
    pub by_m: f64,
}

/// Why the terrain cannot be shaped.
#[derive(Debug, thiserror::Error)]
pub enum ConformError {
    /// The heightmap does not hold `size * size` heights.
    #[error("the heightmap has {len} heights but a size of {size}")]
    BadHeightmap {
        /// Vertices per side.
        size: u32,
        /// Heights present.
        len: usize,
    },
}

fn narrow(v: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "heights and slopes fit f32"
    )]
    let narrowed = v as f32;
    narrowed
}

/// The vertex grid: indices, positions and neighbours.
#[derive(Clone, Copy)]
struct Grid {
    size: usize,
    cell: f64,
}

impl Grid {
    fn count(self) -> usize {
        self.size * self.size
    }

    fn coord(value: usize) -> f64 {
        u32::try_from(value).map_or(f64::MAX, f64::from)
    }

    /// The position of a vertex in map metres.
    fn position(self, vertex: usize) -> (f64, f64) {
        (
            Self::coord(vertex % self.size) * self.cell,
            Self::coord(vertex / self.size) * self.cell,
        )
    }

    /// The column or row nearest to a position, clamped to the grid.
    fn nearest(self, metres: f64) -> usize {
        let last = Self::coord(self.size.saturating_sub(1));
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a grid index from a clamped position"
        )]
        let index = (metres / self.cell).clamp(0.0, last) as usize;
        index
    }

    /// The neighbour of `vertex` at the offset (`dc`, `dr`), if it is on the grid.
    fn neighbour(self, vertex: usize, dc: i64, dr: i64) -> Option<usize> {
        let span = i64::try_from(self.size).ok()?;
        let col = i64::try_from(vertex % self.size).ok()? + dc;
        let row = i64::try_from(vertex / self.size).ok()? + dr;
        if col < 0 || row < 0 || col >= span || row >= span {
            return None;
        }
        usize::try_from(row * span + col).ok()
    }
}

/// A vertex that belongs to a road's core.
#[derive(Clone, Copy)]
struct Core {
    height: f32,
    /// The owner of the claim, as in [`Candidate::owner`].
    owner: usize,
    /// Rise over run of the cutting this vertex's hillside needs.
    cut: f32,
    /// Rise over run of the embankment this vertex's hillside needs.
    fill: f32,
}

/// Where a centre line passes a vertex.
#[derive(Clone, Copy)]
struct Hit {
    /// Distance from the vertex to the centre line.
    distance: f64,
    /// The height of the centre line at the closest point.
    height: f64,
    /// The segment of the strip that is closest.
    segment: usize,
}

/// The vertices within `reach` of the centre line of `strip`. A vertex has one hit for each leg of the strip
/// that passes it: the closest segment, and any segment more than one segment away from it (a road that
/// folds back on itself, or a loop) that is near too.
fn core_vertices(strip: &RoadStrip, grid: Grid, reach: f64) -> BTreeMap<usize, Vec<Hit>> {
    let mut found: BTreeMap<usize, Vec<Hit>> = BTreeMap::new();
    let last = grid.size.saturating_sub(1);
    for (segment, pair) in strip.joints.windows(2).enumerate() {
        let [from, to] = pair else { continue };
        // the ground under a bridge is left alone
        if strip.bridge.get(segment).copied().unwrap_or(false) {
            continue;
        }
        // beside a bridge the core ends flat at the abutment, without a rounded cap into the span
        let bridge_before = segment > 0 && strip.bridge.get(segment - 1).copied().unwrap_or(false);
        let bridge_after = strip.bridge.get(segment + 1).copied().unwrap_or(false);
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let length2 = dx * dx + dy * dy;
        let cols = (
            grid.nearest(from.0.min(to.0) - reach).saturating_sub(1),
            (grid.nearest(from.0.max(to.0) + reach) + 1).min(last),
        );
        let rows = (
            grid.nearest(from.1.min(to.1) - reach).saturating_sub(1),
            (grid.nearest(from.1.max(to.1) + reach) + 1).min(last),
        );
        for row in rows.0..=rows.1 {
            for col in cols.0..=cols.1 {
                let (px, py) = (Grid::coord(col) * grid.cell, Grid::coord(row) * grid.cell);
                let raw = if length2 > 0.0 {
                    ((px - from.0) * dx + (py - from.1) * dy) / length2
                } else {
                    0.0
                };
                if (raw < 0.0 && bridge_before) || (raw > 1.0 && bridge_after) {
                    continue;
                }
                let along = raw.clamp(0.0, 1.0);
                let distance = (px - from.0 - along * dx).hypot(py - from.1 - along * dy);
                if distance <= reach {
                    let hit = Hit {
                        distance,
                        height: from.2 + along * (to.2 - from.2),
                        segment,
                    };
                    let hits = found.entry(row * grid.size + col).or_default();
                    match hits
                        .iter_mut()
                        .find(|other| other.segment.abs_diff(segment) <= 1)
                    {
                        Some(other) if distance < other.distance => *other = hit,
                        Some(_) => {}
                        None => hits.push(hit),
                    }
                }
            }
        }
    }
    found
}

/// A vertex under a bridge's deck, or within one and a half cells of its width: what it is held to.
#[derive(Clone, Copy)]
struct Under {
    /// The ground here stays at or below this height: the lowest ceiling within half a cell along the span.
    ceiling: f32,
    /// The underside of the deck at the vertex's own place (the deck less what hangs there, no air, never the foot
    /// of what stands): a road that lies above it claims the vertex only within its own width.
    underside: f32,
    /// The lowest height that the cutting of another road may take the ground to under what stands (the foot of a
    /// post, pier or wall, deck less [`crate::catalog::PartShape::reach_down`]); minus infinity where nothing
    /// stands. The bridge's own cut never goes below the ceiling.
    floor: f32,
    /// The span that gave the ceiling.
    span: usize,
    /// Whether the vertex lies on the line of one of the span's ports, or in the end guard.
    at_port: bool,
    /// Within the end guard of a free end: under the deck but not cut for it.
    guarded: bool,
    /// Within the deck's width, not only in the band beside it.
    inside: bool,
    /// The deck's surface at the vertex's own place (the lowest where spans overlap): the ground inside the guard is
    /// cut down to it where no road holds it.
    surface: f32,
}

impl Under {
    /// What a vertex that lies in the bands of two spans is held to: the lower ceiling, but guarded, at a port and
    /// inside if either span says so (a guard is lost to nobody: only a free end guards, and a joint of two steps
    /// is none).
    fn merge(self, other: Under) -> Under {
        let kept = if self.ceiling <= other.ceiling {
            self
        } else {
            other
        };
        Under {
            guarded: self.guarded || other.guarded,
            at_port: self.at_port || other.at_port,
            inside: self.inside || other.inside,
            surface: self.surface.min(other.surface),
            ..kept
        }
    }
}

/// Calls `visit(vertex, along, across, half_width)` for every vertex between the ports of `span` and within
/// one and a half cells of the width of what is below it there.
fn band(span: &BridgeSpan, grid: Grid, mut visit: impl FnMut(usize, f64, f64, f64)) {
    let length = span.length();
    if !(length.is_finite() && length > 0.0)
        || span.axes().is_none()
        || span.shape.underside.depth.is_empty()
        || !(span.first.0.is_finite() && span.first.1.is_finite())
    {
        return;
    }
    let widest = span
        .shape
        .underside
        .half_width
        .iter()
        .fold(span.shape.deck_half_width, |a, b| a.max(*b));
    let reach = widest.max(0.0) + 1.5 * grid.cell;
    let last = grid.size.saturating_sub(1);
    let cols = (
        grid.nearest(span.first.0.min(span.second.0) - reach)
            .saturating_sub(1),
        (grid.nearest(span.first.0.max(span.second.0) + reach) + 1).min(last),
    );
    let rows = (
        grid.nearest(span.first.1.min(span.second.1) - reach)
            .saturating_sub(1),
        (grid.nearest(span.first.1.max(span.second.1) + reach) + 1).min(last),
    );
    for row in rows.0..=rows.1 {
        for col in cols.0..=cols.1 {
            let vertex = row * grid.size + col;
            let Some((along, across)) = span.locate(grid.position(vertex)) else {
                continue;
            };
            if !(0.0..=length).contains(&along) {
                continue;
            }
            let half = span.half_width_at(along);
            if across.abs() <= half + 1.5 * grid.cell {
                visit(vertex, along, across, half);
            }
        }
    }
}

/// Whether each end of each span is a free end of its bridge: no other span begins or ends there.
fn free_ends(spans: &[BridgeSpan]) -> Vec<(bool, bool)> {
    let meets = |i: usize, p: (f64, f64)| {
        spans.iter().enumerate().any(|(j, other)| {
            j != i
                && [other.first, other.second]
                    .iter()
                    .any(|q| (q.0 - p.0).hypot(q.1 - p.1) < PORT_JOIN_M)
        })
    };
    spans
        .iter()
        .enumerate()
        .map(|(i, span)| {
            (
                !meets(i, (span.first.0, span.first.1)),
                !meets(i, (span.second.0, span.second.1)),
            )
        })
        .collect()
}

/// For each span the strips of its own road: those of its chain up to the next junction ([`BridgeSpan::own_strips`])
/// and those with a deck joint at one of its ports. The road of a bridge is
/// not another road under its deck.
fn own_strips(spans: &[BridgeSpan], strips: &[RoadStrip]) -> Vec<Vec<usize>> {
    spans
        .iter()
        .map(|span| {
            strips
                .iter()
                .enumerate()
                .filter(|(i, s)| {
                    span.own_strips.contains(i)
                        || (s.bridge.iter().any(|b| *b)
                            && s.joints.iter().any(|j| {
                                [span.first, span.second]
                                    .iter()
                                    .any(|p| (p.0 - j.0).hypot(p.1 - j.1) < PORT_JOIN_M)
                            }))
                })
                .map(|(i, _)| i)
                .collect()
        })
        .collect()
}

/// The height the ground at `along`, `across` under `span` is cut to: the deck less what hangs there and the
/// air, whatever stands there (a solid part hides the ground, the cut never goes to its foot), and the foot
/// (deck less [`crate::catalog::PartShape::reach_down`]) if `across` is under something that stands: the lowest
/// another road's cutting may take the ground to. A point beside the deck has no foot. Where a port holds nothing
/// back (`ramps`) the ceiling rises to the deck's height at the port, [`FILL_SLOPE_MAX`] over run, never lower
/// than it is further in.
fn ceiling_at(
    span: &BridgeSpan,
    length: f64,
    ramps: (bool, bool),
    along: f64,
    across: f64,
) -> (f64, f64) {
    let half = span.half_width_at(along);
    let deck = span.deck_at(along);
    let depth = span.hang_at(along) + BRIDGE_AIR_M;
    let mut height = deck - depth;
    if ramps.0 {
        height = height.max(deck - depth.min(FILL_SLOPE_MAX * along));
    }
    if ramps.1 {
        height = height.max(deck - depth.min(FILL_SLOPE_MAX * (length - along)));
    }
    let floor = match span.shape.ceiling(along, across) {
        Ceiling::Foot(foot) if across.abs() <= half => deck - foot.max(0.0),
        _ => f64::NEG_INFINITY,
    };
    (height, floor)
}

/// The length along a deck of the end guard: the far corner of a terrain cell that straddles the port line is up to
/// `cell * (|ax| + |ay|)` from it (a cell for a deck along an axis, 1.41 cells at 45 degrees).
fn guard_length(span: &BridgeSpan, cell: f64) -> f64 {
    span.axes()
        .map_or(cell, |(along, _)| cell * (along.0.abs() + along.1.abs()))
}

/// What every vertex under a deck, or beside it within one and a half cells, and more than a cell from a free end of
/// the bridge, is held to: per vertex the lowest
/// ceiling that a half cell along the span (the ground between vertices is interpolated, and under cross walls
/// every few metres a vertex stands for a stretch of the span) gives.
fn bridge_ceilings(spans: &[BridgeSpan], grid: Grid) -> Vec<Option<Under>> {
    let mut found: Vec<Option<Under>> = vec![None; grid.count()];
    let free = free_ends(spans);
    for (number, span) in spans.iter().enumerate() {
        let length = span.length();
        let (first, second) = free.get(number).copied().unwrap_or((false, false));
        let ramps = (
            first && !span.shape.retains.0,
            second && !span.shape.retains.1,
        );
        band(span, grid, |vertex, along, across, half| {
            // within the guard of a free end the ground is not cut for the deck: a terrain cell that straddles the
            // port line would slope the road's last piece down to the cut (the terrain cannot show a ramp shorter
            // than a cell), and what lies there is the abutment. It is still under the deck: no level patch claims
            // it and only the bridge's own road may raise it (the abutment's fill)
            let guard = guard_length(span, grid.cell);
            let guarded = (first && along <= guard) || (second && along >= length - guard);
            let (lo, hi) = (
                (along - grid.cell / 2.0).max(0.0),
                (along + grid.cell / 2.0).min(length),
            );
            let mut lowest = f64::INFINITY;
            let mut floor = f64::NEG_INFINITY;
            let mut at = lo;
            if !guarded {
                loop {
                    let (height, foot) = ceiling_at(span, length, ramps, at, across);
                    lowest = lowest.min(height);
                    floor = floor.max(foot);
                    if at >= hi {
                        break;
                    }
                    at = (at + SLICE_M).min(hi);
                }
            }
            if !guarded && !lowest.is_finite() {
                return;
            }
            let under = Under {
                ceiling: narrow(lowest),
                underside: narrow(span.deck_at(along) - span.hang_at(along)),
                floor: narrow(floor),
                span: number,
                guarded,
                at_port: guarded || along <= PORT_LINE_M || along >= length - PORT_LINE_M,
                inside: across.abs() <= half,
                surface: narrow(span.deck_at(along)),
            };
            if let Some(slot) = found.get_mut(vertex) {
                *slot = Some(slot.map_or(under, |old| old.merge(under)));
            }
        });
    }
    found
}

/// The fields the earthworks are held to.
struct Fields {
    /// Distance to the nearest core in metres.
    dist: Vec<f32>,
    /// Index of the nearest core vertex.
    nearest: Vec<u32>,
    upper: Vec<f32>,
    lower: Vec<f32>,
    /// Rise over run of the cutting of the nearest core vertex.
    cut: Vec<f32>,
    /// Rise over run of the embankment of the nearest core vertex.
    fill: Vec<f32>,
}

impl Fields {
    /// Lets `vertex` take what is better of its own values and those of `from`, `step` metres away.
    fn relax(&mut self, vertex: usize, from: usize, step: f32) {
        let (Some(&dist), Some(&upper), Some(&lower), Some(&nearest), Some(&cut), Some(&fill)) = (
            self.dist.get(from),
            self.upper.get(from),
            self.lower.get(from),
            self.nearest.get(from),
            self.cut.get(from),
            self.fill.get(from),
        ) else {
            return;
        };
        if dist + step < self.dist.get(vertex).copied().unwrap_or(f32::INFINITY)
            && let (Some(d), Some(n), Some(c), Some(f)) = (
                self.dist.get_mut(vertex),
                self.nearest.get_mut(vertex),
                self.cut.get_mut(vertex),
                self.fill.get_mut(vertex),
            )
        {
            *d = dist + step;
            *n = nearest;
            *c = cut;
            *f = fill;
        }
        if let Some(value) = self.upper.get_mut(vertex) {
            *value = value.min(upper + cut * step);
        }
        if let Some(value) = self.lower.get_mut(vertex) {
            *value = value.max(lower - fill * step);
        }
    }
}

/// Offsets that have been visited before a vertex in a forward sweep, and after it in a backward one.
const FORWARD: [(i64, i64); 4] = [(-1, 0), (-1, -1), (0, -1), (1, -1)];
const BACKWARD: [(i64, i64); 4] = [(1, 0), (1, 1), (0, 1), (-1, 1)];

/// Two sweeps of an eight-neighbour chamfer over the grid.
fn sweep(cores: &[Option<Core>], under: &[Option<Under>], base: &Heightmap, grid: Grid) -> Fields {
    let count = grid.count();
    let mut fields = Fields {
        dist: vec![f32::INFINITY; count],
        nearest: vec![u32::MAX; count],
        upper: vec![f32::INFINITY; count],
        lower: vec![f32::NEG_INFINITY; count],
        cut: vec![0.0; count],
        fill: vec![0.0; count],
    };
    for (index, core) in cores.iter().enumerate() {
        if let (Some(core), Some(d), Some(u), Some(l), Some(n), Some(c), Some(f)) = (
            core,
            fields.dist.get_mut(index),
            fields.upper.get_mut(index),
            fields.lower.get_mut(index),
            fields.nearest.get_mut(index),
            fields.cut.get_mut(index),
            fields.fill.get_mut(index),
        ) {
            *d = 0.0;
            *u = core.height;
            *l = core.height;
            *n = u32::try_from(index).unwrap_or(u32::MAX);
            *c = core.cut;
            *f = core.fill;
        }
    }
    // the ground under a deck is held to its ceiling, and the cutting spreads from it like from a core
    for (index, held) in under.iter().enumerate() {
        let (Some(held), None) = (held, cores.get(index).copied().flatten()) else {
            continue;
        };
        let (cut, fill) = hillside_slopes(base, grid, index);
        if let (Some(d), Some(u), Some(n), Some(c), Some(f)) = (
            fields.dist.get_mut(index),
            fields.upper.get_mut(index),
            fields.nearest.get_mut(index),
            fields.cut.get_mut(index),
            fields.fill.get_mut(index),
        ) {
            *d = 0.0;
            *u = held.ceiling;
            *n = u32::MAX;
            *c = cut;
            *f = fill;
        }
    }
    let straight = narrow(grid.cell);
    let diagonal = narrow(grid.cell * std::f64::consts::SQRT_2);
    for backward in [false, true] {
        let offsets = if backward { &BACKWARD } else { &FORWARD };
        let order: Vec<usize> = if backward {
            (0..count).rev().collect()
        } else {
            (0..count).collect()
        };
        for vertex in order {
            for &(dc, dr) in offsets {
                if let Some(from) = grid.neighbour(vertex, dc, dr) {
                    let step = if dc != 0 && dr != 0 {
                        diagonal
                    } else {
                        straight
                    };
                    fields.relax(vertex, from, step);
                }
            }
        }
    }
    // under a deck the ground is never raised, and what stands there holds it at its foot whatever another
    // road's cutting wants
    for (index, held) in under.iter().enumerate() {
        let (Some(held), None) = (held, cores.get(index).copied().flatten()) else {
            continue;
        };
        if let Some(lower) = fields.lower.get_mut(index) {
            *lower = f32::NEG_INFINITY;
        }
        if let Some(upper) = fields.upper.get_mut(index) {
            *upper = if held.guarded {
                // the abutment is not cut for the deck, but ground standing in the deck is cut down to its
                // surface (never below it: the approach's fill is at the deck's height and is a core)
                if held.inside {
                    if let Some(lower) = fields.lower.get_mut(index) {
                        *lower = base
                            .h
                            .get(index)
                            .copied()
                            .unwrap_or(f32::NEG_INFINITY)
                            .min(held.surface);
                    }
                    held.surface
                } else {
                    f32::INFINITY
                }
            } else {
                // the cut aims at the ceiling; another road's cutting may take the ground lower, but not below the
                // foot of what stands
                upper.min(held.ceiling).max(held.floor)
            };
        }
    }
    fields
}

/// A core that wants a vertex: one leg of a strip, or a level patch.
#[derive(Clone, Copy)]
struct Candidate {
    /// Whether it is a level patch.
    patch: bool,
    /// The strip's index, or the number of strips plus the patch's index.
    owner: usize,
    rank: u8,
    height: f32,
    /// Distance from the vertex to the centre line, or to the point of a patch.
    distance: f64,
    /// Distance from the vertex to the edge of the road (to the centre line less half the width), or of the
    /// patch (to its point less its radius); not positive within the width.
    edge: f64,
    cut: f32,
    fill: f32,
}

/// The cutting and embankment slopes the hillside under `vertex` needs.
fn hillside_slopes(base: &Heightmap, grid: Grid, vertex: usize) -> (f32, f32) {
    let (x, y) = grid.position(vertex);
    let sample = base.sample(x, y);
    let natural = f64::from(sample.dz_dx.hypot(sample.dz_dy));
    (
        narrow((1.5 * natural).clamp(CUT_SLOPE, CUT_SLOPE_MAX)),
        narrow((1.5 * natural).clamp(FILL_SLOPE, FILL_SLOPE_MAX)),
    )
}

/// Everything that wants each vertex: the legs of the strips, and the patches that are not under a bridge span.
fn candidates(
    strips: &[RoadStrip],
    patches: &[LevelPatch],
    grid: Grid,
    base: &Heightmap,
    under: &[Option<Under>],
    own: &[Vec<usize>],
) -> BTreeMap<usize, Vec<Candidate>> {
    let mut found: BTreeMap<usize, Vec<Candidate>> = BTreeMap::new();
    for (index, strip) in strips.iter().enumerate() {
        let reach = strip.half_width + 1.5 * grid.cell;
        for (vertex, hits) in core_vertices(strip, grid, reach) {
            // the road of a bridge does not claim the ground under its deck beyond the port line: the ceiling
            // with the end ramp holds there
            if let Some(held) = under.get(vertex).copied().flatten()
                && !held.at_port
                && own.get(held.span).is_some_and(|o| o.contains(&index))
            {
                continue;
            }
            let (cut, fill) = hillside_slopes(base, grid, vertex);
            let held = under.get(vertex).copied().flatten();
            let own_road =
                held.is_some_and(|h| own.get(h.span).is_some_and(|o| o.contains(&index)));
            for hit in hits {
                // beyond the end guard a road that lies above the underside of the deck claims a vertex under it only
                // within its own width, not with its ring: its ring would hold ground in the deck. In the guard the
                // ring stays: the pieces of a road at the deck's end need the ground around them. A road below the underside (a road
                // that passes under the deck) keeps its ring, its pieces need the ground around them.
                if let Some(held) = held
                    && !own_road
                    && !held.guarded
                    && held.inside
                    && narrow(hit.height) > held.underside
                    && hit.distance - strip.half_width > 0.0
                {
                    continue;
                }
                let steep =
                    strip.side_slope.get(hit.segment).copied().unwrap_or(0.0) > FACE_SIDE_SLOPE;
                found.entry(vertex).or_default().push(Candidate {
                    patch: false,
                    owner: index,
                    rank: strip.rank,
                    height: narrow(hit.height),
                    distance: hit.distance,
                    edge: hit.distance - strip.half_width,
                    cut: if steep { narrow(FACE_SLOPE) } else { cut },
                    fill,
                });
            }
        }
    }
    let last = grid.size.saturating_sub(1);
    for (number, patch) in patches.iter().enumerate() {
        // a patch that is not a circle on the map claims nothing
        if !(patch.at.0.is_finite()
            && patch.at.1.is_finite()
            && patch.radius.is_finite()
            && patch.radius > 0.0
            && patch.height.is_finite()
            && patch.gradient.0.is_finite()
            && patch.gradient.1.is_finite()
            && patch.inner_radius.is_finite())
        {
            continue;
        }
        let cols = (
            grid.nearest(patch.at.0 - patch.radius).saturating_sub(1),
            (grid.nearest(patch.at.0 + patch.radius) + 1).min(last),
        );
        let rows = (
            grid.nearest(patch.at.1 - patch.radius).saturating_sub(1),
            (grid.nearest(patch.at.1 + patch.radius) + 1).min(last),
        );
        for row in rows.0..=rows.1 {
            for col in cols.0..=cols.1 {
                let vertex = row * grid.size + col;
                let (px, py) = grid.position(vertex);
                let distance = (px - patch.at.0).hypot(py - patch.at.1);
                if distance > patch.radius
                    || distance < patch.inner_radius
                    || under.get(vertex).is_some_and(Option::is_some)
                {
                    continue;
                }
                let (cut, fill) = hillside_slopes(base, grid, vertex);
                let plane = patch.height
                    + patch.gradient.0 * (px - patch.at.0)
                    + patch.gradient.1 * (py - patch.at.1);
                found.entry(vertex).or_default().push(Candidate {
                    patch: true,
                    owner: strips.len() + number,
                    rank: patch.rank,
                    height: narrow(plane),
                    distance,
                    edge: distance - patch.radius,
                    cut,
                    fill,
                });
            }
        }
    }
    found
}

/// Rounds a distance to a millimetre, so that two that are equal but for rounding tie.
fn tie_distance(metres: f64) -> f64 {
    (metres * 1000.0).round()
}

/// Puts the candidates for one vertex in the order of their claim, the winner first, and says whether the
/// vertex is contested (two of them want heights more than [`CONFLICT_M`] apart).
///
/// A vertex inside a road's own width belongs to that road: a patch takes only what is outside every road's
/// width. Where the heights are compatible the higher rank wins. Where they are contested the candidate whose
/// edge is nearer wins, and an exact tie goes to the higher rank; a candidate that has the vertex inside its
/// own width (edge not positive) is always nearer than one that has it only in its ring. Two inside their width
/// remain a clash, which the caller reports.
fn order_wanting(wanting: &mut Vec<Candidate>) -> bool {
    if wanting.iter().any(|c| !c.patch && c.edge <= 0.0) {
        wanting.retain(|c| !c.patch);
    }
    let contested = wanting.first().is_some_and(|first| {
        wanting
            .iter()
            .any(|c| f64::from((c.height - first.height).abs()) > CONFLICT_M)
    });
    let by_rank = |a: &Candidate, b: &Candidate| {
        b.rank
            .cmp(&a.rank)
            .then(a.owner.cmp(&b.owner))
            .then(a.distance.total_cmp(&b.distance))
    };
    if contested {
        wanting.sort_by(|a, b| {
            tie_distance(a.edge)
                .total_cmp(&tie_distance(b.edge))
                .then_with(|| by_rank(a, b))
        });
    } else {
        wanting.sort_by(by_rank);
    }
    contested
}

/// Gives every vertex near a road or a patch the height of what claims it. Where two want different heights
/// the vertex goes to the one whose edge is nearer (so to the one that has it inside its own width), an exact
/// tie to the higher rank; the height difference becomes a face between them. It is a clash only where the
/// vertex is within the width of both.
/// Returns the cores and, per pair of owners, the worst clash and where it is.
#[expect(
    clippy::type_complexity,
    reason = "a map from a pair of owners to (difference, place)"
)]
fn claim_cores(
    strips: &[RoadStrip],
    patches: &[LevelPatch],
    grid: Grid,
    base: &Heightmap,
    under: &[Option<Under>],
    own: &[Vec<usize>],
) -> (
    Vec<Option<Core>>,
    BTreeMap<(usize, usize), (f64, (f64, f64))>,
) {
    let mut cores: Vec<Option<Core>> = vec![None; grid.count()];
    let mut clashes: BTreeMap<(usize, usize), (f64, (f64, f64))> = BTreeMap::new();
    for (vertex, mut wanting) in candidates(strips, patches, grid, base, under, own) {
        let contested = order_wanting(&mut wanting);
        let Some(&winner) = wanting.first() else {
            continue;
        };
        for other in &wanting {
            let difference = f64::from((winner.height - other.height).abs());
            if difference > CONFLICT_M && winner.edge <= 0.0 && other.edge <= 0.0 {
                let key = (winner.owner.min(other.owner), winner.owner.max(other.owner));
                let worst = clashes
                    .entry(key)
                    .or_insert((difference, grid.position(vertex)));
                if difference > worst.0 {
                    *worst = (difference, grid.position(vertex));
                }
            }
        }
        if let Some(slot) = cores.get_mut(vertex) {
            *slot = Some(Core {
                height: winner.height,
                owner: winner.owner,
                // between two roads at different heights the step is a face, as steep as the grid allows
                cut: if contested {
                    narrow(FACE_SLOPE)
                } else {
                    winner.cut
                },
                fill: if contested {
                    narrow(FACE_SLOPE)
                } else {
                    winner.fill
                },
            });
        }
    }
    (cores, clashes)
}

/// The vertices connected to a core, or to ground under a deck, through ground that breaks the limits, and the
/// places where that reaches [`MAX_EARTHWORK_M`]. The ground under a deck is in it where it is above its
/// ceiling.
fn flood(
    base: &Heightmap,
    cores: &[Option<Core>],
    fields: &Fields,
    under: &[Option<Under>],
    grid: Grid,
) -> (Vec<bool>, Vec<(f64, f64)>) {
    let mut flooded = vec![false; grid.count()];
    let held = |index: usize| under.get(index).is_some_and(Option::is_some);
    let mut seen: Vec<bool> = cores
        .iter()
        .enumerate()
        .map(|(index, core)| core.is_some() || held(index))
        .collect();
    let mut queue: VecDeque<usize> = VecDeque::new();
    for (index, core) in cores.iter().enumerate() {
        if core.is_some() {
            queue.push_back(index);
        } else if held(index) {
            let ground = base.h.get(index).copied().unwrap_or(0.0);
            let upper = fields.upper.get(index).copied().unwrap_or(f32::INFINITY);
            if let Some(flag) = flooded.get_mut(index) {
                *flag = ground > upper;
            }
            queue.push_back(index);
        }
    }
    let mut capped = Vec::new();
    while let Some(vertex) = queue.pop_front() {
        for dr in -1..=1_i64 {
            for dc in -1..=1_i64 {
                let Some(next) = grid
                    .neighbour(vertex, dc, dr)
                    .filter(|_| dc != 0 || dr != 0)
                else {
                    continue;
                };
                if seen.get(next).copied().unwrap_or(true) {
                    continue;
                }
                if let Some(flag) = seen.get_mut(next) {
                    *flag = true;
                }
                let ground = base.h.get(next).copied().unwrap_or(0.0);
                let upper = fields.upper.get(next).copied().unwrap_or(f32::INFINITY);
                let lower = fields.lower.get(next).copied().unwrap_or(f32::NEG_INFINITY);
                if ground <= upper && ground >= lower {
                    continue;
                }
                if f64::from(fields.dist.get(next).copied().unwrap_or(0.0)) > MAX_EARTHWORK_M {
                    capped.push(grid.position(next));
                    continue;
                }
                if let Some(flag) = flooded.get_mut(next) {
                    *flag = true;
                }
                queue.push_back(next);
            }
        }
    }
    (flooded, capped)
}

/// Shapes `base` to fit `strips`; `base` itself is not changed.
///
/// # Errors
/// [`ConformError::BadHeightmap`] if the heightmap is not `size * size`.
pub fn conform(
    base: &Heightmap,
    strips: &[RoadStrip],
    patches: &[LevelPatch],
) -> Result<Conformed, ConformError> {
    conform_bridges(base, strips, patches, &[])
}

/// Shapes `base` to fit `strips` and to the decks of `bridges`; `base` itself is not changed.
///
/// # Errors
/// [`ConformError::BadHeightmap`] if the heightmap is not `size * size`.
pub fn conform_bridges(
    base: &Heightmap,
    strips: &[RoadStrip],
    patches: &[LevelPatch],
    bridges: &[BridgeSpan],
) -> Result<Conformed, ConformError> {
    let size = usize::try_from(base.size).unwrap_or(0);
    if base.h.len() != size * size {
        return Err(ConformError::BadHeightmap {
            size: base.size,
            len: base.h.len(),
        });
    }
    if size == 0 {
        return Ok(Conformed {
            heightmap: base.clone(),
            max_cut_m: 0.0,
            max_fill_m: 0.0,
            conflicts: Vec::new(),
            capped: Vec::new(),
            buried: Vec::new(),
            floating: Vec::new(),
        });
    }
    let grid = Grid {
        size,
        cell: f64::from(base.cell_m),
    };
    let under = bridge_ceilings(bridges, grid);
    let own = own_strips(bridges, strips);
    let (cores, clashes) = claim_cores(strips, patches, grid, base, &under, &own);
    if cores.iter().all(Option::is_none) && under.iter().all(Option::is_none) {
        return Ok(Conformed {
            heightmap: base.clone(),
            max_cut_m: 0.0,
            max_fill_m: 0.0,
            conflicts: Vec::new(),
            capped: Vec::new(),
            buried: Vec::new(),
            floating: Vec::new(),
        });
    }
    let fields = sweep(&cores, &under, base, grid);
    let (flooded, capped) = flood(base, &cores, &fields, &under, grid);
    let mut heights = base.h.clone();
    for (vertex, core) in cores.iter().enumerate() {
        if let (Some(core), Some(height)) = (core, heights.get_mut(vertex)) {
            *height = core.height;
        }
    }
    let impossible = clamp_flooded(base, &cores, &fields, &flooded, grid, &mut heights);
    round(base, &fields, &flooded, grid, &mut heights);
    // the ground under a deck is never raised above the original ground, whatever the rounding did
    for (vertex, held) in under.iter().enumerate() {
        if held.is_some()
            && cores.get(vertex).is_some_and(Option::is_none)
            && let (Some(height), Some(ground)) = (heights.get_mut(vertex), base.h.get(vertex))
        {
            *height = height.min(*ground);
        }
    }
    let mut max_cut = 0.0_f32;
    let mut max_fill = 0.0_f32;
    for (new, old) in heights.iter().zip(&base.h) {
        max_cut = max_cut.max(old - new);
        max_fill = max_fill.max(new - old);
    }
    let mut conflicts: Vec<RoadConflict> = clashes
        .into_values()
        .map(|(difference, at)| RoadConflict {
            at,
            height_difference_m: difference,
            bridge: false,
        })
        .collect();
    conflicts.extend(impossible);
    let buried = buried_under(bridges, patches, strips.len(), &cores, &heights, grid);
    let shaped = Heightmap {
        size: base.size,
        cell_m: base.cell_m,
        h: heights,
    };
    let floating = floating_posts(bridges, &shaped);
    Ok(Conformed {
        heightmap: shaped,
        max_cut_m: max_cut,
        max_fill_m: max_fill,
        conflicts,
        capped,
        buried,
        floating,
    })
}

/// One vertex of ground that is in a bridge's deck.
struct InDeck {
    span: usize,
    vertex: usize,
    /// How far the ground is above the reference, in metres.
    above: f64,
    /// Whether the reference is the deck's surface (the vertex is in the end guard).
    guarded: bool,
}

/// The ground that is in a deck: the vertices within the width of a part, where `ground` is more than
/// [`BURIED_TOL_M`] above the deck's underside (the deck less what hangs below it there: no air, never the foot of
/// what stands), or, within the end guard of a free end (so in the whole of a deck shorter than two guards), above
/// the deck's surface. Whoever holds the ground there. The one measure of the finding [`Buried`] and of
/// [`spans_above_underside`].
fn in_decks(spans: &[BridgeSpan], ground: &dyn Fn(usize) -> f64, grid: Grid) -> Vec<InDeck> {
    let free = free_ends(spans);
    let mut found = Vec::new();
    for (number, span) in spans.iter().enumerate() {
        let length = span.length();
        let (first, second) = free.get(number).copied().unwrap_or((false, false));
        let guard = guard_length(span, grid.cell);
        band(span, grid, |vertex, along, across, half| {
            if across.abs() > half {
                return;
            }
            let guarded = (first && along <= guard) || (second && along >= length - guard);
            let deck = span.deck_at(along);
            let reference = if guarded {
                deck
            } else {
                deck - span.hang_at(along)
            };
            let above = ground(vertex) - reference;
            if above.is_finite() && above > BURIED_TOL_M {
                found.push(InDeck {
                    span: number,
                    vertex,
                    above,
                    guarded,
                });
            }
        });
    }
    found
}

/// The bridges that have ground in their deck after the shaping (see [`in_decks`]), one for each stretch, in the
/// order of the ids, with what holds the ground at the worst vertex: the core of a road or a level patch (the cut
/// stops at the core of a road that is not the bridge's own, and the bridge's own road may hold ground in the end
/// guard), or nothing where the ground was not cut (the end guard, a short deck).
fn buried_under(
    spans: &[BridgeSpan],
    patches: &[LevelPatch],
    strips: usize,
    cores: &[Option<Core>],
    heights: &[f32],
    grid: Grid,
) -> Vec<Buried> {
    let ground = |vertex: usize| f64::from(heights.get(vertex).copied().unwrap_or(f32::NAN));
    let mut found: BTreeMap<&str, Buried> = BTreeMap::new();
    for hit in in_decks(spans, &ground, grid) {
        let Some(span) = spans.get(hit.span) else {
            continue;
        };
        let holder = match cores.get(hit.vertex).copied().flatten() {
            None => Holder::None,
            Some(core) if core.owner < strips => Holder::Strip(core.owner),
            Some(core) => Holder::Patch(
                patches
                    .get(core.owner - strips)
                    .map_or(grid.position(hit.vertex), |p| p.at),
            ),
        };
        let entry = found.entry(span.stretch.as_str()).or_insert(Buried {
            stretch: span.stretch.clone(),
            at: grid.position(hit.vertex),
            above_m: hit.above,
            vertices: 0,
            holder,
            guarded: hit.guarded,
        });
        entry.vertices += 1;
        if hit.above > entry.above_m {
            entry.at = grid.position(hit.vertex);
            entry.above_m = hit.above;
            entry.holder = holder;
            entry.guarded = hit.guarded;
        }
    }
    found.into_values().collect()
}

/// The bridges with a post, pier or wall foot more than [`POST_FLOATS_M`] above the shaped ground, one for each
/// stretch, in the order of the ids. The feet are sampled as the choice of a model on posts samples them (at the
/// corners and every [`crate::bridgekind::GROUND_STEP_M`] along and across each standing rectangle, see
/// [`crate::bridgekind`]), against the deck's height as placed.
fn floating_posts(spans: &[BridgeSpan], shaped: &Heightmap) -> Vec<PostFloat> {
    let mut found: BTreeMap<&str, PostFloat> = BTreeMap::new();
    for span in spans {
        for (a0, a1, c0, c1) in span.shape.stands() {
            for along in samples(a0, a1) {
                for across in samples(c0, c1) {
                    let Some(at) = span.point(along, across) else {
                        continue;
                    };
                    let ground = f64::from(shaped.sample(at.0, at.1).height);
                    let by = span.deck_at(along) - ground - span.shape.reach_down - POST_FLOATS_M;
                    if !(by > 0.0 && by.is_finite()) {
                        continue;
                    }
                    let entry = found.entry(span.stretch.as_str()).or_insert(PostFloat {
                        stretch: span.stretch.clone(),
                        at,
                        by_m: by,
                    });
                    if by > entry.by_m {
                        entry.at = at;
                        entry.by_m = by;
                    }
                }
            }
        }
    }
    found.into_values().collect()
}

/// The flooded vertices take the base height clamped into the limits. Between two roads that cannot both be
/// met (the limits do not overlap) a vertex follows the nearest road, and the clash is reported once per
/// region.
fn clamp_flooded(
    base: &Heightmap,
    cores: &[Option<Core>],
    fields: &Fields,
    flooded: &[bool],
    grid: Grid,
    heights: &mut [f32],
) -> Vec<RoadConflict> {
    let mut impossible: BTreeMap<(i64, i64), RoadConflict> = BTreeMap::new();
    for vertex in 0..grid.count() {
        if !flooded.get(vertex).copied().unwrap_or(false) {
            continue;
        }
        let ground = base.h.get(vertex).copied().unwrap_or(0.0);
        let upper = fields.upper.get(vertex).copied().unwrap_or(f32::INFINITY);
        let lower = fields
            .lower
            .get(vertex)
            .copied()
            .unwrap_or(f32::NEG_INFINITY);
        let value = if lower <= upper {
            ground.clamp(lower, upper)
        } else {
            let at = grid.position(vertex);
            let region = |metres: f64| {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "a region number of a map position"
                )]
                let number = (metres / CONFLICT_REGION_M).floor() as i64;
                number
            };
            let difference = f64::from(lower - upper);
            // the nearest source of the limits is the cut under a deck, not a road
            let bridge = fields.nearest.get(vertex) == Some(&u32::MAX);
            let entry = impossible
                .entry((region(at.0), region(at.1)))
                .or_insert(RoadConflict {
                    at,
                    height_difference_m: difference,
                    bridge,
                });
            if difference > entry.height_difference_m {
                *entry = RoadConflict {
                    at,
                    height_difference_m: difference,
                    bridge: bridge || entry.bridge,
                };
            }
            let nearest = fields
                .nearest
                .get(vertex)
                .and_then(|&i| cores.get(usize::try_from(i).unwrap_or(usize::MAX)))
                .and_then(|core| *core);
            nearest.map_or(ground, |core| {
                let distance = fields.dist.get(vertex).copied().unwrap_or(0.0);
                let fill = fields
                    .fill
                    .get(vertex)
                    .copied()
                    .unwrap_or(narrow(FILL_SLOPE));
                let cut = fields.cut.get(vertex).copied().unwrap_or(narrow(CUT_SLOPE));
                core.height + (ground - core.height).clamp(-fill * distance, cut * distance)
            })
        };
        if let Some(height) = heights.get_mut(vertex) {
            *height = value;
        }
    }
    impossible.into_values().collect()
}

/// Four passes of averaging over the changed vertices, clamped back into the limits after each pass.
fn round(base: &Heightmap, fields: &Fields, flooded: &[bool], grid: Grid, heights: &mut [f32]) {
    let changed: Vec<usize> = (0..grid.count())
        .filter(|&vertex| {
            flooded.get(vertex).copied().unwrap_or(false)
                && heights.get(vertex) != base.h.get(vertex)
        })
        .collect();
    for _ in 0..ROUNDING_PASSES {
        let previous = heights.to_vec();
        for &vertex in &changed {
            let around: Vec<f32> = [(-1, 0), (1, 0), (0, -1), (0, 1)]
                .iter()
                .filter_map(|&(dc, dr)| grid.neighbour(vertex, dc, dr))
                .filter_map(|next| previous.get(next).copied())
                .collect();
            if around.is_empty() {
                continue;
            }
            let mean = around.iter().sum::<f32>() / narrow(Grid::coord(around.len()));
            let own = previous.get(vertex).copied().unwrap_or(0.0);
            let averaged = 0.5 * own + 0.5 * mean;
            let upper = fields.upper.get(vertex).copied().unwrap_or(f32::INFINITY);
            let lower = fields
                .lower
                .get(vertex)
                .copied()
                .unwrap_or(f32::NEG_INFINITY);
            let value = if lower <= upper {
                averaged.clamp(lower, upper)
            } else {
                averaged
            };
            if let Some(height) = heights.get_mut(vertex) {
                *height = value;
            }
        }
    }
}

/// A bridge with ground above the underside of its deck.
#[derive(Debug, Clone, PartialEq)]
pub struct AboveUnderside {
    /// The id of the bridge stretch.
    pub stretch: String,
    /// The vertex with the most ground above the underside, in map metres.
    pub at: (f64, f64),
    /// How far the ground there is above the underside, in metres.
    pub above_m: f64,
    /// How many vertices within the deck's width are above the underside.
    pub vertices: usize,
}

/// The bridges of `bridges` that have ground in their deck on `heightmap`, in the order of their ids: the measure of
/// the finding [`Buried`] (see [`conform_bridges`]) without what holds the ground: above the underside of the deck
/// (what hangs below it, no air kept, never the foot of what stands) beyond the end guard of a free end, above the
/// deck's surface within it.
#[must_use]
pub fn spans_above_underside(heightmap: &Heightmap, bridges: &[BridgeSpan]) -> Vec<AboveUnderside> {
    let size = usize::try_from(heightmap.size).unwrap_or(0);
    if size == 0 || heightmap.h.len() != size * size {
        return Vec::new();
    }
    let grid = Grid {
        size,
        cell: f64::from(heightmap.cell_m),
    };
    let ground = |vertex: usize| f64::from(heightmap.h.get(vertex).copied().unwrap_or(f32::NAN));
    let mut found: BTreeMap<&str, AboveUnderside> = BTreeMap::new();
    for hit in in_decks(bridges, &ground, grid) {
        let Some(span) = bridges.get(hit.span) else {
            continue;
        };
        let at = grid.position(hit.vertex);
        let entry = found
            .entry(span.stretch.as_str())
            .or_insert(AboveUnderside {
                stretch: span.stretch.clone(),
                at,
                above_m: hit.above,
                vertices: 0,
            });
        entry.vertices += 1;
        if hit.above > entry.above_m {
            entry.at = at;
            entry.above_m = hit.above;
        }
    }
    found.into_values().collect()
}

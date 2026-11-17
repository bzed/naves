// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What hangs below the deck of a bridge part, measured on the faces of its Geometry LOD, and the sizes of the
//! parts without ports (the concrete panels and the pipe).
//!
//! A part is cut into slices of [`SLICE_M`] along its span, the axis from its first port to its second. Per
//! slice the triangles of the Geometry LOD are clipped to the slice (a solid block has faces across slices
//! that hold no vertex), and the slice keeps the lowest point of what is left, as a depth below the deck
//! surface, and how far across the axis it reaches. Geometry beyond the ports is not sliced. Zero-area
//! triangles clip like any other.
//!
//! What is left of a triangle in a slice either **stands** (it reaches to within [`STAND_TOL_M`] of the part's
//! deepest point, [`PartShape::reach_down`]: an abutment, a pier, a post, a wall foot) or **hangs** (the deck,
//! the intrados of an arch, the truss floor). Per slice the underside keeps the deepest hanging point and the
//! across intervals of what stands. A part that reaches nowhere deeper than its shallowest slice (a plain slab)
//! has nothing that stands.
//!
//! Slices are half open, `[i, i + 1)`, for a face with an along extent: a face that only touches a slice at a
//! whole metre does not count for it. A face with no along extent (a vertical face across the span) on a whole
//! metre between two slices is left out; the faces of its solid that have an along extent carry it.
//!
//! A triangle that reaches the foot anywhere never hangs, also where it is shallow: a hanging member that shares
//! a triangle with a foot is not in `hang` there.
//!
//! Limitation: "stands" is measured against one foot depth for the whole part, the deepest point. A model whose
//! piers stand at different depths (none in the catalog today; `bridge_concrete1/2` would) would need a foot
//! per pier.

use serde::{Deserialize, Serialize};

use super::MAX_DECK_THICKNESS_M;

/// The length of a slice of an [`Underside`], in metres.
pub const SLICE_M: f64 = 1.0;
/// A part longer than this between its ports is not measured: it is left out of the catalog with a warning.
/// This bounds the number of slices (200).
pub const MAX_PART_LENGTH_M: f64 = 200.0;
/// A model coordinate further than this from the origin, in metres, is no part of a road: the part is left out
/// with a warning.
pub const MAX_COORDINATE_M: f64 = 1000.0;
/// Deep geometry with gaps across of at most this, in metres, still closes the deck's width (a wall).
const WALL_GAP_M: f64 = 0.5;
/// Geometry that reaches to within this of the part's deepest point stands on the ground, in metres.
pub const STAND_TOL_M: f64 = 0.3;
/// Along extents shorter than this, in metres, are no extent (a face across the span, a touch at a boundary).
const EDGE_M: f64 = 1e-3;

/// The most slices a part has.
const MAX_SLICES: u32 = 200;

/// What is below a deck, per slice of [`SLICE_M`] from the first port; the last slice ends at the second
/// port and may be shorter (or longer, for a panel row past [`MAX_PART_LENGTH_M`]: it covers the rest).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Underside {
    /// The depth of the lowest geometry in the slice below the deck surface, over the whole width, in metres; 0
    /// where the slice has no geometry or only geometry above the deck.
    pub depth: Vec<f64>,
    /// How far the geometry in the slice reaches from the axis between the ports, to either side, in metres; 0
    /// where the slice has no geometry.
    pub half_width: Vec<f64>,
    /// The depth of the lowest geometry in the slice that hangs (does not stand), in metres; 0 where nothing
    /// hangs below the deck surface.
    #[serde(default)]
    pub hang: Vec<f64>,
    /// The across intervals (signed, metres from the axis, sorted and merged) of the feet of what stands in the
    /// slice; empty where nothing stands. `across` is positive to the left of the way from the first port to the
    /// second.
    #[serde(default)]
    pub stand: Vec<Vec<(f64, f64)>>,
}

/// What a ground point under a part must keep from the deck surface (see [`PartShape::ceiling`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ceiling {
    /// Geometry hangs this deep below the deck surface here: the ground stays below it (and the air beneath).
    Hang(f64),
    /// Geometry stands here with its foot this deep below the deck surface: the ground meets the foot and is
    /// not cut below it.
    Foot(f64),
}

/// A run of slices deeper than [`MAX_DECK_THICKNESS_M`]: an abutment, an arch flank, a pier or a wall. It
/// merges what stands and what hangs: for a road below or the ground use [`PartShape::passable`] and
/// [`PartShape::ceiling`], not its depth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Solid {
    /// Where its geometry deeper than [`MAX_DECK_THICKNESS_M`] starts along the span, in metres from the first
    /// port.
    pub from: f64,
    /// Where that geometry ends (at most the second port).
    pub to: f64,
    /// How far its deep geometry reaches from the axis, to either side: the largest of its slices.
    pub half_width: f64,
    /// Its deepest point below the deck surface.
    pub depth: f64,
    /// Whether in one of its slices the deep geometry closes the deck's whole width (gaps of at most 0.5 m): a
    /// wall across the road, not two pylons beside it.
    pub wall: bool,
}

/// The shape of a bridge part below its deck, as a later step lays it: its length, its underside and what of
/// it is solid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartShape {
    /// The length between the ports, in metres.
    pub length: f64,
    /// What hangs below the deck, per slice.
    pub underside: Underside,
    /// The solid parts, in order along the span.
    pub solid: Vec<Solid>,
    /// Whether the part holds the ground back at its first and at its second port: a solid part that reaches
    /// the port and is a wall across the deck's width. Only geometry between the ports is seen: an abutment
    /// wholly behind a port does not retain.
    pub retains: (bool, bool),
    /// The deepest point of the Geometry LOD below the deck surface, also beyond the ports, in metres: the foot
    /// of what stands.
    pub reach_down: f64,
    /// Half the width of the deck (its ports), in metres.
    #[serde(default)]
    pub deck_half_width: f64,
}

/// A concrete panel: a slab with a roadway and nothing below it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slab {
    /// Model path.
    pub model: String,
    /// The short horizontal side, laid along the road, in metres.
    pub along: f64,
    /// The long horizontal side, laid across the road.
    pub across: f64,
    /// Whether the short side runs along the model's x axis (else along its z axis).
    pub along_x: bool,
    /// The height of its surface above the model origin.
    pub top: f64,
    /// Its thickness, the vertical extent of its Geometry LOD.
    pub thickness: f64,
}

/// The concrete pipe, for a culvert.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pipe {
    /// Model path.
    pub model: String,
    /// Its length along its axis, the longest horizontal extent, in metres.
    pub length: f64,
    /// Its vertical extent.
    pub diameter: f64,
    /// Whether its axis runs along the model's x axis (else along its z axis).
    pub axis_x: bool,
    /// The height of its lowest point above the model origin (negative: below it).
    pub bottom: f64,
}

/// `v` as an index in `0..=max`; NaN and negatives count as 0.
fn clamp_index(v: f64, max: usize) -> usize {
    if v.is_nan() || v <= 0.0 {
        return 0;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "positive, capped at u32::MAX and clamped to max below"
    )]
    let i = v.min(f64::from(u32::MAX)) as usize;
    i.min(max)
}

/// A last slice shorter than this, in metres, is no slice: ports that a float rounding lies beyond a whole
/// length (the game's 25 m bridges are 25.000002 m) do not give a sliver.
const SLIVER_M: f64 = 1e-3;

/// The number of slices of a part `length` long: `ceil(length / SLICE_M)` less a sliver, at most
/// [`MAX_SLICES`]; the last slice ends at `length`.
fn slice_count(length: f64) -> usize {
    if !length.is_finite() {
        return 0;
    }
    (0..MAX_SLICES)
        .take_while(|i| f64::from(*i) * SLICE_M < length - SLIVER_M)
        .count()
}

/// The metres along the span where slice `i` starts.
fn slice_start(i: usize) -> f64 {
    f64::from(u32::try_from(i).unwrap_or(MAX_SLICES)) * SLICE_M
}

/// The slices `from..to` touches over a positive length, in an underside of `n` slices; a single point touches
/// the slice that holds it. Inputs that are not numbers count as the part's ends.
fn touched(n: usize, length: f64, from: f64, to: f64) -> std::ops::RangeInclusive<usize> {
    let last = n.saturating_sub(1);
    let a = if from.is_finite() { from } else { 0.0 };
    let b = if to.is_finite() { to } else { length };
    let (a, b) = (a.min(b).max(0.0), a.max(b).min(length.max(0.0)));
    let first = clamp_index((a / SLICE_M).floor(), last);
    let end = if b > a {
        // a slice that the interval only reaches with its end point is not touched
        clamp_index((b / SLICE_M).ceil() - 1.0, last)
    } else {
        first
    };
    first..=end.max(first)
}

/// Slice `i`'s extent along the span in an underside of `n` slices of a part `length` long (the last one ends at
/// `length`).
fn slice_bounds(i: usize, n: usize, length: f64) -> (f64, f64) {
    let end = if i + 1 >= n {
        length
    } else {
        slice_start(i + 1).min(length)
    };
    (slice_start(i), end)
}

/// Whether one of `solid` overlaps `a..b` over a positive length, or, for a point (`b <= a`), holds it inside.
fn in_solid(solid: &[Solid], a: f64, b: f64) -> bool {
    if b > a + EDGE_M {
        solid.iter().any(|s| s.to.min(b) - s.from.max(a) > EDGE_M)
    } else {
        solid.iter().any(|s| s.from < a && s.to > a)
    }
}

/// See [`PartShape::thickness`].
fn thickness_over(underside: &Underside, solid: &[Solid], length: f64, from: f64, to: f64) -> f64 {
    let n = underside.depth.len();
    if n == 0 {
        return 0.0;
    }
    let a = if from.is_finite() { from } else { 0.0 };
    let b = if to.is_finite() { to } else { length };
    let (a, b) = (a.min(b).max(0.0), a.max(b).min(length.max(0.0)));
    touched(n, length, a, b)
        .map(|i| {
            let depth = underside.depth.get(i).copied().unwrap_or_default();
            let (s0, s1) = slice_bounds(i, n, length);
            let (lo, hi) = (a.max(s0), b.min(s1));
            // posts or pylons outside any solid part count with their depth: the slice has no exact extent
            // for them
            let loose_stand =
                !in_solid(solid, s0, s1) && underside.stand.get(i).is_some_and(|f| !f.is_empty());
            if loose_stand || in_solid(solid, lo, hi.max(lo)) {
                depth
            } else {
                underside.hang.get(i).copied().unwrap_or(depth)
            }
        })
        .fold(0.0, f64::max)
}

/// The longest stretch of the span outside `solid`, exact to the solid parts' ends; the first of equal ones.
/// `(0, 0)` if there is none.
fn opening_of(solid: &[Solid], length: f64) -> (f64, f64) {
    let length = length.max(0.0);
    let mut parts: Vec<(f64, f64)> = solid
        .iter()
        .map(|s| (s.from.clamp(0.0, length), s.to.clamp(0.0, length)))
        .collect();
    parts.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut best = (0.0, 0.0);
    let mut at = 0.0;
    for (from, to) in parts.iter().copied().chain([(length, length)]) {
        if from - at > best.1 - best.0 + EDGE_M {
            best = (at, from);
        }
        at = f64::max(at, to);
    }
    best
}

impl PartShape {
    /// How far the part reaches below its deck surface over `from..to` (metres from the first port): per slice
    /// the interval touches over a positive length, [`Underside::depth`] where the interval meets a solid part
    /// in that slice or where something stands in a slice outside every solid part (posts), else
    /// [`Underside::hang`]; the largest of them. So an interval in the opening gets the
    /// deck's (or the arch's) depth and one that touches a solid part gets the solid part's.
    #[must_use]
    pub fn thickness(&self, from: f64, to: f64) -> f64 {
        thickness_over(&self.underside, &self.solid, self.length, from, to)
    }

    /// The longest stretch of the span between the solid parts, as `(from, to)` in metres from the first
    /// port, exact to the ends of the solid parts.
    #[must_use]
    pub fn opening(&self) -> (f64, f64) {
        opening_of(&self.solid, self.length)
    }

    /// Whether something stands in slice `i`, anywhere across: a road below crosses the whole model, also beside
    /// the deck (the posts of `bridge_wood_25` stand outside its deck).
    fn stands_in(&self, i: usize) -> bool {
        self.underside.stand.get(i).is_some_and(|s| !s.is_empty())
    }

    /// The stretches along the span (metres from the first port) where a road `depth_below_deck` below the deck
    /// surface clears `clearance` under the part: slices whose [`Underside::depth`] (over the whole width) is at
    /// most `depth_below_deck - clearance` and where nothing stands (anywhere across: a road below crosses the
    /// whole model); neighbouring slices merged. A run bounded by a solid part that ends inside the next slice reaches to that part's end, if what
    /// hangs there clears. Standing geometry never passes, however deep the road. Inputs that are not numbers
    /// pass nothing.
    #[must_use]
    pub fn passable(&self, depth_below_deck: f64, clearance: f64) -> Vec<(f64, f64)> {
        let limit = depth_below_deck - clearance;
        let n = self.underside.depth.len();
        if !limit.is_finite() || n == 0 {
            return Vec::new();
        }
        let hang = |i: usize| {
            self.underside
                .hang
                .get(i)
                .or_else(|| self.underside.depth.get(i))
                .copied()
                .unwrap_or_default()
        };
        let ok = |i: usize| {
            self.underside.depth.get(i).is_some_and(|d| *d <= limit) && !self.stands_in(i)
        };
        let mut out = Vec::new();
        let mut i = 0;
        while i < n {
            if !ok(i) {
                i += 1;
                continue;
            }
            let start = i;
            while i < n && ok(i) {
                i += 1;
            }
            let (mut from, _) = slice_bounds(start, n, self.length);
            let (_, mut to) = slice_bounds(i - 1, n, self.length);
            if let Some(before) = start.checked_sub(1) {
                let (s0, s1) = slice_bounds(before, n, self.length);
                let end = self
                    .solid
                    .iter()
                    .map(|s| s.to)
                    .filter(|t| *t > s0 && *t < s1)
                    .fold(f64::NEG_INFINITY, f64::max);
                if end.is_finite() && hang(before) <= limit {
                    from = end;
                }
            }
            if i < n {
                let (s0, s1) = slice_bounds(i, n, self.length);
                let begin = self
                    .solid
                    .iter()
                    .map(|s| s.from)
                    .filter(|f| *f > s0 && *f < s1)
                    .fold(f64::INFINITY, f64::min);
                if begin.is_finite() && hang(i) <= limit {
                    to = begin;
                }
            }
            out.push((from, to));
        }
        out
    }

    /// What the ground at `along` (metres from the first port) and `across` (signed, metres from the axis) must
    /// keep from the deck surface: [`Ceiling::Foot`] at [`PartShape::reach_down`] where `across` lies under
    /// something that stands in that slice, else [`Ceiling::Hang`] with the slice's [`Underside::hang`]. A point
    /// outside the part takes the nearest slice (a number that is not one: the first); a part without slices
    /// gives `Hang(0.0)`. It does not look at the part's across extent: callers ask only within the slice's
    /// [`Underside::half_width`], and in a slice where a solid part ends tell its open side by [`Solid::from`]
    /// and [`Solid::to`].
    #[must_use]
    pub fn ceiling(&self, along: f64, across: f64) -> Ceiling {
        let n = self.underside.depth.len();
        if n == 0 {
            return Ceiling::Hang(0.0);
        }
        let i = clamp_index((along / SLICE_M).floor(), n - 1);
        let stands = self
            .underside
            .stand
            .get(i)
            .is_some_and(|s| s.iter().any(|(a, b)| *a <= across && across <= *b));
        if stands {
            Ceiling::Foot(self.reach_down)
        } else {
            Ceiling::Hang(
                self.underside
                    .hang
                    .get(i)
                    .or_else(|| self.underside.depth.get(i))
                    .copied()
                    .unwrap_or_default(),
            )
        }
    }

    /// Every foot of what stands, as `(along from, along to, across from, across to)` in metres: the slice's
    /// extent along the span and the foot's across interval. In order along the span, then across.
    pub fn stands(&self) -> impl Iterator<Item = (f64, f64, f64, f64)> + '_ {
        let n = self.underside.depth.len();
        self.underside
            .stand
            .iter()
            .enumerate()
            .flat_map(move |(i, feet)| {
                let (s0, s1) = slice_bounds(i, n, self.length);
                feet.iter().map(move |(a, b)| (s0, s1, *a, *b))
            })
    }

    /// A row of `count` panels laid one after the other, each `slab.along · scale` long: a plain slab
    /// `slab.thickness · scale` deep everywhere, nothing solid, holding nothing back. A scale that is not a
    /// positive number counts as 1.
    #[must_use]
    pub fn panels(slab: &Slab, count: usize, scale: f64) -> PartShape {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let count = f64::from(u32::try_from(count).unwrap_or(u32::MAX));
        let length = count * slab.along * scale;
        let depth = (slab.thickness * scale).max(0.0);
        PartShape::level(length, depth, slab.across / 2.0 * scale)
    }

    /// A part with the same `depth` and `half_width` in every slice, nothing solid and nothing that stands
    /// (everything hangs): a plain slab, a panel row, or a model whose faces cannot be read.
    #[must_use]
    pub fn level(length: f64, depth: f64, half_width: f64) -> PartShape {
        let length = if length.is_finite() {
            length.max(0.0)
        } else {
            0.0
        };
        let n = slice_count(length);
        let depth = if depth.is_finite() { depth } else { 0.0 };
        let half_width = if half_width.is_finite() {
            half_width
        } else {
            0.0
        };
        PartShape {
            length,
            underside: Underside {
                depth: vec![depth; n],
                half_width: vec![half_width; n],
                hang: vec![depth; n],
                stand: vec![Vec::new(); n],
            },
            solid: Vec::new(),
            retains: (false, false),
            reach_down: depth,
            deck_half_width: half_width,
        }
    }
}

/// The frame of a part: its first port, the unit vector to its second, its length and its deck height, all in
/// model coordinates (x east, z north, y up).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Span {
    pub x: f64,
    pub z: f64,
    pub dir_x: f64,
    pub dir_z: f64,
    pub length: f64,
    pub deck: f64,
}

impl Span {
    /// `(along, across, depth)` of a model point: metres from the first port along the span, from the axis
    /// across it, and below the deck surface.
    fn local(&self, p: [f32; 3]) -> [f64; 3] {
        let (dx, dz) = (f64::from(p[0]) - self.x, f64::from(p[2]) - self.z);
        [
            dx * self.dir_x + dz * self.dir_z,
            dz * self.dir_x - dx * self.dir_z,
            self.deck - f64::from(p[1]),
        ]
    }
}

/// Whether every coordinate is a number within [`MAX_COORDINATE_M`] of the origin.
pub(crate) fn sane(points: &[[f32; 3]]) -> bool {
    points
        .iter()
        .flatten()
        .all(|c| c.is_finite() && f64::from(*c).abs() <= MAX_COORDINATE_M)
}

/// The part of `polygon` where `axis` is at least (`keep_above`) or at most `limit` (one step of
/// Sutherland–Hodgman). A point, a segment or an empty list come out as such.
fn clip(polygon: &[[f64; 3]], axis: usize, limit: f64, keep_above: bool) -> Vec<[f64; 3]> {
    let inside = |p: &[f64; 3]| {
        let v = p.get(axis).copied().unwrap_or(f64::NAN);
        if keep_above { v >= limit } else { v <= limit }
    };
    let mut out = Vec::new();
    let Some(&last) = polygon.last() else {
        return out;
    };
    let mut prev = last;
    for &cur in polygon {
        let (pi, ci) = (inside(&prev), inside(&cur));
        if pi != ci {
            let (a, b) = (
                prev.get(axis).copied().unwrap_or_default(),
                cur.get(axis).copied().unwrap_or_default(),
            );
            let t = if (b - a).abs() > f64::EPSILON {
                ((limit - a) / (b - a)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            out.push([
                prev[0] + (cur[0] - prev[0]) * t,
                prev[1] + (cur[1] - prev[1]) * t,
                prev[2] + (cur[2] - prev[2]) * t,
            ]);
        }
        if ci {
            out.push(cur);
        }
        prev = cur;
    }
    out
}

/// Per slice, what the clipped triangles leave.
#[derive(Clone, Default)]
struct Slice {
    /// The deepest point, `None` without geometry.
    depth: Option<f64>,
    half_width: f64,
    /// The across extent of each piece of geometry deeper than [`MAX_DECK_THICKNESS_M`].
    deep: Vec<(f64, f64)>,
    deep_half_width: f64,
    /// The along extent of that deep geometry, `(min, max)`.
    deep_along: Option<(f64, f64)>,
    /// The deepest point of the clipped pieces whose triangle does not reach the foot (they may hang); 0
    /// without any.
    loose: f64,
    /// The deepest point of all clipped pieces, at least 0: what hangs when nothing in the part can stand.
    any: f64,
    /// The across intervals of the feet of the pieces that reach the foot, merged as they come.
    feet: Vec<(f64, f64)>,
}

/// Feet kept per slice before they are merged.
const FEET_BEFORE_MERGE: usize = 32;

/// `spans` sorted and merged where they overlap or touch.
fn merged(mut spans: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    spans.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (a, b) in spans {
        match out.last_mut() {
            Some(last) if a <= last.1 + EDGE_M => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// The `(min, max)` of axis `axis` over `points`; `None` for no points.
fn range_of(points: &[[f64; 3]], axis: usize) -> Option<(f64, f64)> {
    let lo = points
        .iter()
        .filter_map(|p| p.get(axis))
        .copied()
        .fold(f64::INFINITY, f64::min);
    let hi = points
        .iter()
        .filter_map(|p| p.get(axis))
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    (lo <= hi).then_some((lo, hi))
}

/// Whether the intervals `spans` cover `-half..half` with gaps of at most [`WALL_GAP_M`].
fn closes(spans: &[(f64, f64)], half: f64) -> bool {
    let mut sorted = spans.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut reach = -half;
    for (a, b) in sorted {
        if a > reach + WALL_GAP_M {
            break;
        }
        reach = reach.max(b);
    }
    reach + WALL_GAP_M >= half
}

/// The runs of slices deeper than [`MAX_DECK_THICKNESS_M`], as [`Solid`]s.
fn solids(
    slices: &[Slice],
    underside: &Underside,
    length: f64,
    deck_half_width: f64,
) -> Vec<Solid> {
    let n = slices.len();
    let mut solid: Vec<Solid> = Vec::new();
    let mut run: Option<usize> = None;
    let deep = |i: usize| {
        underside
            .depth
            .get(i)
            .is_some_and(|d| *d > MAX_DECK_THICKNESS_M)
    };
    for i in 0..=n {
        match (i < n && deep(i), run) {
            (true, None) => run = Some(i),
            (false, Some(start)) => {
                let part = slices.get(start..i).unwrap_or_default();
                let (s_from, _) = slice_bounds(start, n, length);
                let (_, s_to) = slice_bounds(i - 1, n, length);
                let first = part
                    .first()
                    .and_then(|s| s.deep_along)
                    .map_or(s_from, |r| r.0);
                let last = part.last().and_then(|s| s.deep_along).map_or(s_to, |r| r.1);
                solid.push(Solid {
                    from: first.clamp(0.0, length),
                    to: last.clamp(0.0, length),
                    half_width: part.iter().map(|s| s.deep_half_width).fold(0.0, f64::max),
                    depth: part.iter().filter_map(|s| s.depth).fold(0.0, f64::max),
                    wall: part.iter().any(|s| closes(&s.deep, deck_half_width)),
                });
                run = None;
            }
            _ => {}
        }
    }
    solid
}

/// The slices of a part `length` long that a triangle with along extent `lo..hi` adds to: half open, so a
/// slice the face only touches at its start does not get it; a face across the span (no along extent) on a
/// whole metre between two slices gets none.
fn slices_of(lo: f64, hi: f64, n: usize, length: f64) -> Option<std::ops::RangeInclusive<usize>> {
    let last = n.checked_sub(1)?;
    if hi < 0.0 || lo > length {
        return None;
    }
    if hi - lo > EDGE_M {
        return Some(
            clamp_index((lo / SLICE_M).floor(), last)
                ..=clamp_index((hi / SLICE_M).ceil() - 1.0, last),
        );
    }
    let at = f64::midpoint(lo, hi);
    let k = (at / SLICE_M).round();
    if (at - k * SLICE_M).abs() <= EDGE_M && k > 0.0 && k * SLICE_M < length - EDGE_M {
        return None;
    }
    let i = clamp_index((at / SLICE_M).floor(), last);
    Some(i..=i)
}

/// Adds the part of `triangle` (local coordinates) in each slice to `slices`. `foot` is the depth from which
/// geometry stands.
fn add_face(slices: &mut [Slice], length: f64, triangle: &[[f64; 3]; 3], foot: f64) {
    let n = slices.len();
    let along = range_of(triangle, 0);
    let Some((lo, hi)) = along else {
        return;
    };
    let Some(range) = slices_of(lo, hi, n, length) else {
        return;
    };
    // a face that reaches the foot anywhere is part of what stands: its pieces that do not reach the foot in a
    // slice (the slanted half of a block's side) do not hang
    let reaches = range_of(triangle, 2).is_some_and(|(_, d)| d >= foot);
    for i in range {
        let (s0, s1) = slice_bounds(i, n, length);
        if hi - lo > EDGE_M && hi.min(s1) - lo.max(s0) <= EDGE_M {
            continue;
        }
        let piece = clip(&clip(triangle, 0, s0, true), 0, s1, false);
        let (Some(slice), Some((_, deepest))) = (slices.get_mut(i), range_of(&piece, 2)) else {
            continue;
        };
        slice.depth = Some(slice.depth.map_or(deepest, |d| d.max(deepest)));
        let across = piece.iter().map(|p| p[1].abs()).fold(0.0, f64::max);
        slice.half_width = slice.half_width.max(across);
        let foot_span = (deepest >= foot)
            .then(|| range_of(&clip(&piece, 2, foot, true), 1))
            .flatten();
        // per slice only two maxima and the merged feet are kept, not every piece: a model of many long
        // faces does not ask for memory per face and slice
        slice.any = slice.any.max(deepest);
        if !reaches {
            slice.loose = slice.loose.max(deepest);
        }
        if let Some(span) = foot_span {
            slice.feet.push(span);
            if slice.feet.len() > FEET_BEFORE_MERGE {
                slice.feet = merged(std::mem::take(&mut slice.feet));
            }
        }
        if deepest > MAX_DECK_THICKNESS_M {
            let deep = clip(&piece, 2, MAX_DECK_THICKNESS_M, true);
            if let Some((low, high)) = range_of(&deep, 1) {
                slice.deep.push((low, high));
                slice.deep_half_width = slice.deep_half_width.max(low.abs()).max(high.abs());
            }
            if let Some((a0, a1)) = range_of(&deep, 0) {
                slice.deep_along = Some(
                    slice
                        .deep_along
                        .map_or((a0, a1), |(x, y)| (x.min(a0), y.max(a1))),
                );
            }
        }
    }
}

/// The underside of `slices` for a part whose deepest point is `reach_down`.
fn underside_of(slices: &[Slice], reach_down: f64) -> Underside {
    let depth: Vec<f64> = slices
        .iter()
        .map(|s| s.depth.map_or(0.0, |d| d.max(0.0)))
        .collect();
    // a part that reaches nowhere deeper than its shallowest slice (a plain slab) has nothing that stands
    let shallowest = depth.iter().copied().fold(f64::INFINITY, f64::min);
    let can_stand = reach_down > shallowest + STAND_TOL_M;
    Underside {
        hang: slices
            .iter()
            .map(|s| if can_stand { s.loose } else { s.any })
            .collect(),
        stand: slices
            .iter()
            .map(|s| {
                if can_stand {
                    merged(s.feet.clone())
                } else {
                    Vec::new()
                }
            })
            .collect(),
        depth,
        half_width: slices.iter().map(|s| s.half_width).collect(),
    }
}

/// Measures a part on the `vertices` and `faces` of its Geometry LOD, with a deck `deck_half_width` to either
/// side of its axis. The caller has checked the span and that the vertices are [`sane`]. A face that names a
/// vertex that does not exist is skipped.
pub(crate) fn measure(
    span: &Span,
    deck_half_width: f64,
    vertices: &[[f32; 3]],
    faces: &[[u32; 3]],
) -> PartShape {
    let length = span.length;
    let count = slice_count(length);
    let local: Vec<[f64; 3]> = vertices.iter().map(|p| span.local(*p)).collect();
    let reach_down = local.iter().map(|p| p[2]).fold(0.0, f64::max);
    let mut slices = vec![Slice::default(); count];
    for face in faces {
        let corner = |i: usize| {
            face.get(i)
                .and_then(|v| usize::try_from(*v).ok())
                .and_then(|v| local.get(v))
                .copied()
        };
        if let (Some(p0), Some(p1), Some(p2)) = (corner(0), corner(1), corner(2)) {
            add_face(&mut slices, length, &[p0, p1, p2], reach_down - STAND_TOL_M);
        }
    }
    let underside = underside_of(&slices, reach_down);
    let solid = solids(&slices, &underside, length, deck_half_width);
    let holds = |s: &Solid| s.wall && s.half_width + WALL_GAP_M >= deck_half_width;
    let first_end = slice_bounds(0, count, length).1;
    let last_start = slice_bounds(count.saturating_sub(1), count, length).0;
    let retains = (
        solid
            .first()
            .is_some_and(|s| s.from < first_end && holds(s)),
        solid.last().is_some_and(|s| s.to > last_start && holds(s)),
    );
    PartShape {
        length,
        underside,
        solid,
        retains,
        reach_down,
        deck_half_width,
    }
}

/// The extent of `points` per axis: `(min, max)` of x, y and z. `None` for no points.
fn extent(points: &[[f32; 3]]) -> Option<[(f64, f64); 3]> {
    let mut out = [(f64::INFINITY, f64::NEG_INFINITY); 3];
    for p in points {
        for (range, v) in out.iter_mut().zip(p) {
            let v = f64::from(*v);
            *range = (range.0.min(v), range.1.max(v));
        }
    }
    (!points.is_empty()).then_some(out)
}

/// A panel sized from its Geometry LOD (the slab: its horizontal sides and thickness) and its Roadway LOD (the
/// surface, else the slab's top); with no Geometry LOD, the Roadway LOD stands in for the slab. `Err` says why
/// the model cannot be a panel.
pub(crate) fn slab(
    model: &str,
    geometry: &[[f32; 3]],
    roadway: &[[f32; 3]],
) -> Result<Slab, String> {
    if !sane(geometry) || !sane(roadway) {
        return Err(format!(
            "{model}: its Geometry or Roadway LOD has coordinates that are not numbers or lie more than {MAX_COORDINATE_M} m out"
        ));
    }
    let body = if geometry.is_empty() {
        roadway
    } else {
        geometry
    };
    let Some([x, y, z]) = extent(body) else {
        return Err(format!(
            "{model}: it has neither a Geometry nor a Roadway LOD to size it from"
        ));
    };
    let (dx, dz) = (x.1 - x.0, z.1 - z.0);
    let along = dx.min(dz);
    if along < 0.1 {
        return Err(format!(
            "{model}: its slab is {along:.2} m long, too short to lay"
        ));
    }
    let top = extent(roadway).map_or(y.1, |r| r[1].1);
    Ok(Slab {
        model: model.to_string(),
        along,
        across: dx.max(dz),
        along_x: dx <= dz,
        top,
        thickness: y.1 - y.0,
    })
}

/// The pipe sized from its Geometry LOD (else its Roadway LOD). `Err` says why the model cannot be the pipe.
pub(crate) fn pipe(
    model: &str,
    geometry: &[[f32; 3]],
    roadway: &[[f32; 3]],
) -> Result<Pipe, String> {
    if !sane(geometry) || !sane(roadway) {
        return Err(format!(
            "{model}: its Geometry or Roadway LOD has coordinates that are not numbers or lie more than {MAX_COORDINATE_M} m out"
        ));
    }
    let body = if geometry.is_empty() {
        roadway
    } else {
        geometry
    };
    let Some([x, y, z]) = extent(body) else {
        return Err(format!(
            "{model}: it has neither a Geometry nor a Roadway LOD to size it from"
        ));
    };
    let (dx, dz) = (x.1 - x.0, z.1 - z.0);
    let diameter = y.1 - y.0;
    if diameter < 0.1 || dx.max(dz) < 0.1 {
        return Err(format!("{model}: it is too small to be a pipe"));
    }
    Ok(Pipe {
        model: model.to_string(),
        length: dx.max(dz),
        diameter,
        axis_x: dx >= dz,
        bottom: y.0,
    })
}

/// Whether a part with no solid part stands on posts: nothing deeper than [`MAX_DECK_THICKNESS_M`], and
/// something stands.
pub(crate) fn on_posts(underside: &Underside, solid: &[Solid]) -> bool {
    solid.is_empty() && underside.stand.iter().any(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping_keeps_the_part_on_the_kept_side() {
        let triangle = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]];
        let left = clip(&triangle, 0, 1.0, false);
        assert!(left.iter().all(|p| p[0] <= 1.0 + 1e-12));
        assert_eq!(left.len(), 4);
        // a zero-area triangle (a segment) clips to a segment, not to nothing and not to a panic
        let flat = [[0.0, 0.0, 1.0], [2.0, 0.0, 3.0], [2.0, 0.0, 3.0]];
        let cut = clip(&flat, 0, 1.0, true);
        assert_ne!(cut.len(), 0);
        assert!(cut.iter().all(|p| p[0] >= 1.0 - 1e-12));
        assert_eq!(clip(&[], 0, 0.0, true).len(), 0);
    }

    #[test]
    fn spans_close_a_width_only_without_wide_gaps() {
        assert!(closes(&[(-6.0, 0.2), (0.5, 6.0)], 6.0));
        assert!(!closes(&[(-8.1, -6.7), (6.7, 8.1)], 6.0));
        assert!(!closes(&[], 1.0));
        assert!(closes(&[(-8.0, 8.0)], 6.0));
    }

    #[test]
    fn the_touched_slices_leave_out_a_slice_met_only_at_its_end() {
        assert_eq!(touched(25, 25.0, 4.0, 21.0), 4..=20);
        assert_eq!(touched(25, 25.0, 3.5, 3.5), 3..=3);
        assert_eq!(touched(25, 25.0, f64::NAN, f64::NAN), 0..=24);
        assert_eq!(touched(25, 25.0, 30.0, -5.0), 0..=24);
        assert_eq!(touched(0, 0.0, 0.0, 1.0), 0..=0);
    }

    #[test]
    fn an_empty_part_answers_every_query_without_slices() {
        let empty = PartShape::level(0.0, 1.0, 3.0);
        assert_eq!(empty.ceiling(1.0, 0.0), Ceiling::Hang(0.0));
        assert_eq!(empty.passable(10.0, 4.5), Vec::new());
        assert_eq!(empty.stands().count(), 0);
        assert!((empty.thickness(0.0, 1.0)).abs() < 1e-12);
        assert_eq!(empty.opening(), (0.0, 0.0));
        let slab = PartShape::level(5.0, 1.0, 3.0);
        assert_eq!(slab.passable(f64::NAN, 4.5), Vec::new());
        assert_eq!(slab.passable(5.5, 4.5), vec![(0.0, 5.0)]);
        assert_eq!(slab.ceiling(-3.0, 9.0), Ceiling::Hang(1.0));
        let open = PartShape::level(f64::NAN, f64::NAN, f64::INFINITY);
        assert_eq!(open.underside.depth.len(), 0);
    }

    #[test]
    fn spans_are_merged_where_they_touch() {
        assert_eq!(
            merged(vec![(2.0, 3.0), (-1.0, 0.5), (0.5, 1.0)]),
            vec![(-1.0, 1.0), (2.0, 3.0)]
        );
        assert_eq!(merged(Vec::new()), Vec::new());
    }

    #[test]
    fn a_face_across_the_span_on_a_whole_metre_belongs_to_no_slice() {
        assert_eq!(slices_of(3.0, 3.0, 25, 25.0), None);
        assert_eq!(slices_of(0.0, 0.0, 25, 25.0), Some(0..=0));
        assert_eq!(slices_of(25.0, 25.0, 25, 25.0), Some(24..=24));
        assert_eq!(slices_of(3.5, 3.5, 25, 25.0), Some(3..=3));
        assert_eq!(slices_of(0.0, 3.0, 25, 25.0), Some(0..=2));
        assert_eq!(slices_of(-5.0, -1.0, 25, 25.0), None);
        assert_eq!(slices_of(0.0, 1.0, 0, 0.0), None);
    }

    #[test]
    fn slices_are_capped() {
        assert_eq!(slice_count(25.0), 25);
        assert_eq!(slice_count(25.000_002), 25);
        assert_eq!(slice_count(25.01), 26);
        assert_eq!(slice_count(31.6), 32);
        assert_eq!(slice_count(1e30), 200);
        assert_eq!(slice_count(f64::NAN), 0);
        assert_eq!(slice_count(-1.0), 0);
    }
}

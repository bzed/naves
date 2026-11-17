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
/// A slice whose deck hangs less than this share of the deepest point below a part with no solid part stands
/// between posts (see [`super::BridgeModel::posts`]).
const POST_SHARE: f64 = 0.5;

/// The most slices a part has.
const MAX_SLICES: u32 = 200;

/// What hangs below a deck, per slice of [`SLICE_M`] from the first port; the last slice ends at the second
/// port and may be shorter.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Underside {
    /// The depth of the lowest geometry in the slice below the deck surface, in metres; 0 where the slice has
    /// no geometry or only geometry above the deck.
    pub depth: Vec<f64>,
    /// How far the geometry in the slice reaches from the axis between the ports, to either side, in metres; 0
    /// where the slice has no geometry.
    pub half_width: Vec<f64>,
}

/// A run of slices deeper than [`MAX_DECK_THICKNESS_M`]: an abutment, an arch flank, a pier or a wall.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Solid {
    /// Where it starts along the span, in metres from the first port (a slice boundary).
    pub from: f64,
    /// Where it ends (a slice boundary, or the second port).
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
    /// the port and is a wall across the deck's width.
    pub retains: (bool, bool),
    /// The deepest point of the Geometry LOD below the deck surface, also beyond the ports, in metres.
    pub reach_down: f64,
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

/// The largest depth of `underside` over the slices `from..to` touches.
fn thickness_over(underside: &Underside, length: f64, from: f64, to: f64) -> f64 {
    let n = underside.depth.len();
    if n == 0 {
        return 0.0;
    }
    underside
        .depth
        .get(touched(n, length, from, to))
        .unwrap_or_default()
        .iter()
        .copied()
        .fold(0.0, f64::max)
}

/// The longest run of slices outside `solid`, as metres along the span; the first of equal runs. `(0, 0)` if
/// every slice is solid or there are none.
fn opening_of(underside: &Underside, solid: &[Solid], length: f64) -> (f64, f64) {
    let n = underside.depth.len();
    let is_solid = |i: usize| {
        let (a, b) = (slice_start(i), slice_start(i + 1));
        solid.iter().any(|s| s.from < b && s.to > a)
    };
    let mut best: Option<(usize, usize)> = None;
    let mut run: Option<usize> = None;
    for i in 0..=n {
        let open = i < n && !is_solid(i);
        match (open, run) {
            (true, None) => run = Some(i),
            (false, Some(start)) => {
                if best.is_none_or(|(a, b)| i - start > b - a) {
                    best = Some((start, i));
                }
                run = None;
            }
            _ => {}
        }
    }
    best.map_or((0.0, 0.0), |(a, b)| {
        let to = if b == n { length } else { slice_start(b) };
        (slice_start(a), to.min(length.max(0.0)))
    })
}

impl PartShape {
    /// How far the part reaches below its deck surface over `from..to` (metres from the first port): the
    /// largest [`Underside::depth`] of the slices the interval touches over a positive length. Solid slices
    /// count with their real depth, so an interval in the opening gets the deck's (or the arch's) depth there
    /// and one that touches a solid part gets the solid part's.
    #[must_use]
    pub fn thickness(&self, from: f64, to: f64) -> f64 {
        thickness_over(&self.underside, self.length, from, to)
    }

    /// The longest run of slices that are not solid, as `(from, to)` in metres from the first port.
    #[must_use]
    pub fn opening(&self) -> (f64, f64) {
        opening_of(&self.underside, &self.solid, self.length)
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

    /// A part with the same `depth` and `half_width` in every slice and nothing solid: a plain slab, or a
    /// model whose faces cannot be read.
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
            },
            solid: Vec::new(),
            retains: (false, false),
            reach_down: depth,
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
                solid.push(Solid {
                    from: slice_start(start),
                    to: if i == n {
                        length
                    } else {
                        slice_start(i).min(length)
                    },
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
    let n = slice_count(length);
    let last = n.saturating_sub(1);
    let local: Vec<[f64; 3]> = vertices.iter().map(|p| span.local(*p)).collect();
    let reach_down = local.iter().map(|p| p[2]).fold(0.0, f64::max);
    let mut slices = vec![Slice::default(); n];
    for face in faces {
        let corner = |i: usize| {
            face.get(i)
                .and_then(|v| usize::try_from(*v).ok())
                .and_then(|v| local.get(v))
                .copied()
        };
        let (Some(a), Some(b), Some(c)) = (corner(0), corner(1), corner(2)) else {
            continue;
        };
        let triangle = [a, b, c];
        let lo = a[0].min(b[0]).min(c[0]);
        let hi = a[0].max(b[0]).max(c[0]);
        if n == 0 || hi < 0.0 || lo > length {
            continue;
        }
        // a slice is closed at both ends: a face on a boundary belongs to both slices
        let first = clamp_index((lo / SLICE_M).ceil() - 1.0, last);
        let end = clamp_index((hi / SLICE_M).floor(), last);
        for i in first..=end {
            let s1 = if i == last {
                length
            } else {
                slice_start(i + 1).min(length)
            };
            let s0 = slice_start(i);
            let piece = clip(&clip(&triangle, 0, s0, true), 0, s1, false);
            let Some(slice) = slices.get_mut(i) else {
                continue;
            };
            if piece.is_empty() {
                continue;
            }
            let deepest = piece.iter().map(|p| p[2]).fold(f64::NEG_INFINITY, f64::max);
            slice.depth = Some(slice.depth.map_or(deepest, |d| d.max(deepest)));
            let across = piece.iter().map(|p| p[1].abs()).fold(0.0, f64::max);
            slice.half_width = slice.half_width.max(across);
            if deepest > MAX_DECK_THICKNESS_M {
                let deep = clip(&piece, 2, MAX_DECK_THICKNESS_M, true);
                let low = deep.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
                let high = deep.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max);
                if low <= high {
                    slice.deep.push((low, high));
                    slice.deep_half_width = slice.deep_half_width.max(low.abs()).max(high.abs());
                }
            }
        }
    }
    let underside = Underside {
        depth: slices
            .iter()
            .map(|s| s.depth.map_or(0.0, |d| d.max(0.0)))
            .collect(),
        half_width: slices.iter().map(|s| s.half_width).collect(),
    };
    let solid = solids(&slices, &underside, length, deck_half_width);
    let holds = |s: &Solid| s.wall && s.half_width + WALL_GAP_M >= deck_half_width;
    let retains = (
        solid.first().is_some_and(|s| s.from <= 0.0 && holds(s)),
        solid.last().is_some_and(|s| s.to >= length && holds(s)),
    );
    PartShape {
        length,
        underside,
        solid,
        retains,
        reach_down,
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

/// Whether a part with no solid part stands on posts: some slice's deck hangs less than half of the deepest
/// point (the posts reach further down than the deck between them).
pub(crate) fn on_posts(underside: &Underside, solid: &[Solid], reach_down: f64) -> bool {
    solid.is_empty()
        && reach_down > 0.0
        && underside.depth.iter().any(|d| *d < POST_SHARE * reach_down)
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

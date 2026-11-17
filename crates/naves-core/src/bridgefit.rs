// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A bridge over a road: what the road below needs along the span, and where a part has to lie so that the road
//! passes through its opening.
//!
//! The road below crosses the bridge's line at an angle. Along the span it occupies
//!
//! - its **footprint**, `width / sin(angle)`, plus [`PASS_MARGIN_M`] on each side,
//! - the **skew** of its centre line across the part: the line runs over the whole width of what hangs under the
//!   deck, `max(half_width) · cot(angle)` further to each side,
//! - where the part does not hold the ground back (panels, a plain deck): the run of the cutting's walls under
//!   the deck, `(BRIDGE_CLEARANCE_M + thickness) / CUT_SLOPE_MAX` each side. Where it does (the solid ends of a
//!   truss or an arch) the cutting ends at the solid part, and the skew is the whole of it.
//!
//! The centre line needs [`BRIDGE_CLEARANCE_M`] under the deck and the footprint [`PASS_CLEARANCE_M`]; the
//! height solve raises the deck until the road has [`BRIDGE_CLEARANCE_M`] under the deepest hanging point over
//! the footprint, so the road is that deep below the deck, and [`PartShape::passable`] says where that
//! clears. Nothing that stands (a wall, a post, a pier) ever passes a road.
//!
//! [`fit_over`] finds where a part lies best: the nearest start to the one asked for from which the road's
//! interval lies in the part's opening; where none does, the one that leaves the least of the interval under a
//! solid part or outside the deck. The angle's sine is bounded below by [`SIN_MIN`], so a road nearly along the
//! bridge asks for a large but finite interval (and finds no fit).
//!
//! All lengths are metres on the map.

use crate::catalog::{Family, PartShape};
use crate::conform::CUT_SLOPE_MAX;
use crate::profile::BRIDGE_CLEARANCE_M;
use crate::roadplace::{PlacedKind, PlacedRoad};

/// The margin on each side of a road's footprint under a bridge, in metres.
pub const PASS_MARGIN_M: f64 = 1.0;
/// The clearance a road below needs over its whole footprint, in metres (at its centre line it needs
/// [`BRIDGE_CLEARANCE_M`]).
pub const PASS_CLEARANCE_M: f64 = 3.0;
/// The sine of the angle between a bridge and the road below is not taken smaller than this: a road nearly along
/// the bridge gets a long, finite interval.
pub const SIN_MIN: f64 = 0.01;
/// Lengths closer than this are equal.
const EPS_M: f64 = 1e-6;
/// The starts tried between the ends of a range are at least this far apart, in metres...
const SCAN_STEP_M: f64 = 0.25;
/// ... and at most this many.
const SCAN_MAX: f64 = 64.0;
/// The depth that passes everything that does not stand (for finding where a part could carry a road).
const ANY_DEPTH_M: f64 = 1.0e6;

/// A road below a bridge, where its line crosses the bridge's line (the input of the fit, see
/// [`crate::bridgekind::BridgeSite::below`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Below {
    /// Where its line crosses the bridge's, in metres from the bridge's start along its line.
    pub station: f64,
    /// The angle between the two lines there, in radians from 0 (along each other) to a right angle.
    pub angle: f64,
    /// The road below's surface (its width is the road piece's).
    pub family: Family,
    /// Where the lines cross, in map metres.
    pub at: (f64, f64),
    /// The road below's stretch.
    pub lower: String,
}

/// What a bridge got for one road below, for the report and the findings.
#[derive(Debug, Clone, PartialEq)]
pub struct BelowFit {
    /// The road below's stretch.
    pub lower: String,
    /// Where its line crosses the bridge's, in map metres.
    pub at: (f64, f64),
    /// Where along the bridge from its start, in metres.
    pub station: f64,
    /// What the road needs, along the bridge from its start: from, to.
    pub interval: (f64, f64),
    /// How much of that is under a solid part or outside the deck, in metres; 0 where the road passes.
    pub missing: f64,
    /// Whether a placed piece of the road below runs under the placed deck (see [`mark_placed`]); true until the
    /// placement is known. A road below that the placement laid away from the deck (its line has a bend tighter
    /// than a piece can follow, say) does not run under it: nothing is missing there that anyone drives.
    pub placed: bool,
}

/// Half the lengths along the span a road below needs about its crossing, in metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reach {
    /// The skew of the centre line across the part: the centre line lies in `station ± centre`.
    pub centre: f64,
    /// The footprint with its margins and the skew: the road's surface lies in `station ± footprint`.
    pub footprint: f64,
    /// What the cutting's walls add on each side where the part holds nothing back; 0 where it does.
    pub walls: f64,
}

impl Reach {
    /// The whole half interval, with the walls.
    #[must_use]
    pub fn total(&self) -> f64 {
        self.footprint + self.walls
    }
}

/// A crossing as the fit needs it: the station along the bridge and the road's [`Reach`] there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Need {
    /// Metres from the bridge's start.
    pub station: f64,
    /// What the road needs about it.
    pub reach: Reach,
}

/// The sine and cotangent of the angle between a bridge and the road below, the sine at least [`SIN_MIN`]; an
/// angle that is not a number counts as along the bridge.
fn sin_cot(angle: f64) -> (f64, f64) {
    let a = if angle.is_finite() {
        angle.abs() % std::f64::consts::PI
    } else {
        0.0
    };
    // the angle between two lines is taken acute
    let a = if a > std::f64::consts::FRAC_PI_2 {
        std::f64::consts::PI - a
    } else {
        a
    };
    let sin = a.sin().clamp(SIN_MIN, 1.0);
    (sin, a.cos().max(0.0) / sin)
}

/// What a road `width` wide below a bridge needs along `part` where it crosses at `angle` (see the module
/// documentation). Finite for every input: the sine is at least [`SIN_MIN`], a width that is not a number counts
/// as 0.
#[must_use]
pub fn below_interval(width: f64, angle: f64, part: &PartShape) -> Reach {
    let (sin, cot) = sin_cot(angle);
    let width = if width.is_finite() && width > 0.0 {
        width
    } else {
        0.0
    };
    let across = part
        .underside
        .half_width
        .iter()
        .copied()
        .filter(|h| h.is_finite())
        .fold(part.deck_half_width.max(0.0), f64::max);
    let centre = across * cot;
    let footprint = centre + width / (2.0 * sin) + PASS_MARGIN_M;
    let retains = part.retains.0 || part.retains.1;
    let walls = if retains {
        0.0
    } else {
        (BRIDGE_CLEARANCE_M + part.thickness(0.0, part.length).max(0.0)) / CUT_SLOPE_MAX
    };
    Reach {
        centre,
        footprint,
        walls,
    }
}

/// `count` copies of a part end to end.
#[derive(Debug, Clone, Copy)]
pub struct Chain<'a> {
    /// The part's shape.
    pub shape: &'a PartShape,
    /// How many end to end (at least 1).
    pub count: usize,
}

fn times(count: usize) -> f64 {
    u32::try_from(count).map_or(f64::from(u32::MAX), f64::from)
}

impl Chain<'_> {
    /// The length of the chain.
    #[must_use]
    pub fn length(&self) -> f64 {
        times(self.count.max(1)) * self.shape.length
    }

    /// The stretches of the chain where a road `depth` below the deck clears `clearance` (see
    /// [`PartShape::passable`]), the copies' runs joined where they meet.
    #[must_use]
    pub fn passable(&self, depth: f64, clearance: f64) -> Vec<(f64, f64)> {
        let one = self.shape.passable(depth, clearance);
        let mut out: Vec<(f64, f64)> = Vec::new();
        for k in 0..self.count.max(1) {
            let off = times(k) * self.shape.length;
            for (a, b) in &one {
                let (a, b) = (a + off, b + off);
                match out.last_mut() {
                    Some(last) if a <= last.1 + EPS_M => last.1 = last.1.max(b),
                    _ => out.push((a, b)),
                }
            }
        }
        out
    }

    /// How far the chain reaches below its deck over `from..to` (see [`PartShape::thickness`]); `None` where the
    /// interval does not meet the chain.
    #[must_use]
    pub fn thickness(&self, from: f64, to: f64) -> Option<f64> {
        let (a, b) = (from.min(to), from.max(to));
        let mut out: Option<f64> = None;
        for k in 0..self.count.max(1) {
            let off = times(k) * self.shape.length;
            let (lo, hi) = ((a - off).max(0.0), (b - off).min(self.shape.length));
            if hi + EPS_M < lo || !lo.is_finite() || !hi.is_finite() {
                continue;
            }
            let t = self.shape.thickness(lo, hi.max(lo));
            out = Some(out.map_or(t, |o| o.max(t)));
        }
        out
    }
}

/// The metres of `from..to` that no interval of `runs` (sorted, merged) covers.
fn uncovered(runs: &[(f64, f64)], from: f64, to: f64) -> f64 {
    if to <= from {
        return 0.0;
    }
    let covered: f64 = runs
        .iter()
        .map(|(a, b)| (b.min(to) - a.max(from)).max(0.0))
        .sum();
    let missing = (to - from - covered).max(0.0);
    if missing < EPS_M { 0.0 } else { missing }
}

/// The metres by which the deck's underside may hang deeper over a road's footprint than at its centre line: the
/// centre line needs [`BRIDGE_CLEARANCE_M`] and the footprint [`PASS_CLEARANCE_M`].
const FOOTPRINT_SLACK_M: f64 = BRIDGE_CLEARANCE_M - PASS_CLEARANCE_M;
/// The centre line of a road below is taken as the metre either side of the crossing when the thickness over it is
/// measured.
const CENTRE_M: f64 = 1.0;

/// The thickness the height solve keeps [`BRIDGE_CLEARANCE_M`] above a road below, with `at` the crossing in the
/// chain's own metres: the larger of what hangs over `at ± 1 m` (4.5 m under the deepest point at the centre line)
/// and what hangs over the footprint `at ± reach.footprint` less 1.5 m (3 m over the whole footprint). `None`
/// where neither interval meets the chain.
#[must_use]
pub fn clearance_thickness(chain: &Chain, at: f64, reach: &Reach) -> Option<f64> {
    let centre = chain.thickness(at - CENTRE_M, at + CENTRE_M);
    let footprint = chain.thickness(at - reach.footprint, at + reach.footprint);
    match (centre, footprint) {
        (None, None) => None,
        (c, f) => Some(
            c.unwrap_or(0.0)
                .max(f.map_or(0.0, |t| t - FOOTPRINT_SLACK_M)),
        ),
    }
}

/// How many metres of what `need` asks of `chain` lying with its first end `start` metres from the bridge's
/// start (negative where it reaches before it) are not available: under a solid part, or outside the deck. The
/// road's depth below the deck is [`BRIDGE_CLEARANCE_M`] plus [`clearance_thickness`], as the height solve makes
/// it.
#[must_use]
pub fn missing_at(chain: &Chain, start: f64, need: &Need) -> f64 {
    let s = need.station - start;
    let band = (s - need.reach.centre, s + need.reach.centre);
    let whole = (s - need.reach.total(), s + need.reach.total());
    let hanging = clearance_thickness(chain, s, &need.reach).unwrap_or_default();
    // (the epsilon keeps `depth - clearance >= hanging` true against rounding)
    let depth = BRIDGE_CLEARANCE_M + hanging + EPS_M;
    let at_centre = uncovered(&chain.passable(depth, BRIDGE_CLEARANCE_M), band.0, band.1);
    let at_edges = uncovered(&chain.passable(depth, PASS_CLEARANCE_M), whole.0, whole.1);
    at_centre.max(at_edges)
}

/// The sum of [`missing_at`] over `needs`.
#[must_use]
pub fn missing_total(chain: &Chain, start: f64, needs: &[Need]) -> f64 {
    needs.iter().map(|n| missing_at(chain, start, n)).sum()
}

/// What [`fit_over`] found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fit {
    /// The part lies at `start` and every road below passes through its opening (`start` is the start asked for
    /// when nothing had to move).
    Fits {
        /// Metres from the bridge's start to the part's first end.
        start: f64,
    },
    /// No start in the range lets every road pass: the part lies at `start`, which leaves the least of the
    /// intervals, `missing` metres of them, under a solid part or outside the deck.
    Blocked {
        /// Metres from the bridge's start to the part's first end.
        start: f64,
        /// The metres that are missing.
        missing: f64,
    },
}

impl Fit {
    /// The start the part lies at.
    #[must_use]
    pub fn start(&self) -> f64 {
        match self {
            Fit::Fits { start } | Fit::Blocked { start, .. } => *start,
        }
    }

    /// The metres missing: 0 for a fit.
    #[must_use]
    pub fn missing(&self) -> f64 {
        match self {
            Fit::Fits { .. } => 0.0,
            Fit::Blocked { missing, .. } => *missing,
        }
    }
}

/// Where `chain` lies best: among the starts in `range` (`valid` says which of them may be used) the nearest to
/// `wanted` from which every road of `needs` passes, else the one that leaves the least missing (then the
/// nearest to `wanted`, then the lower). The candidates are `wanted`, the ends of the range, and every start at
/// which an end of a road's interval meets an end of a run where a road can pass. With no valid start at all
/// the part stays at `wanted` and its `missing` is counted there.
#[must_use]
pub fn fit_over(
    chain: &Chain,
    range: (f64, f64),
    wanted: f64,
    needs: &[Need],
    valid: &dyn Fn(f64) -> bool,
) -> Fit {
    if !(range.0.is_finite() && range.1.is_finite()) {
        // no start to try
        return Fit::Blocked {
            start: wanted,
            missing: missing_total(chain, wanted, needs),
        };
    }
    let (lo, hi) = (range.0.min(range.1), range.0.max(range.1));
    let mut candidates = vec![wanted, lo, hi];
    // where `valid` cuts the range (the turn limit, the posts) no end of an interval says where: a grid of starts
    // finds the nearest that is valid, to within its step
    let step = SCAN_STEP_M.max((hi - lo) / SCAN_MAX);
    if step.is_finite() && step > 0.0 {
        let mut at = lo + step;
        while at < hi - EPS_M {
            candidates.push(at);
            at += step;
        }
    }
    // the runs where something can pass whatever its depth: not under what stands
    let runs = chain.passable(ANY_DEPTH_M, 0.0);
    for need in needs {
        let (from, to) = (
            need.station - need.reach.total(),
            need.station + need.reach.total(),
        );
        for (a, b) in &runs {
            // from - start >= a, to - start <= b
            candidates.push(to - b);
            candidates.push(from - a);
        }
    }
    let mut best: Option<(f64, f64, f64)> = None;
    for c in candidates {
        if !c.is_finite() {
            continue;
        }
        let start = c.clamp(lo, hi);
        if (start - wanted).abs() > EPS_M && !valid(start) {
            continue;
        }
        let missing = missing_total(chain, start, needs);
        let key = (missing, (start - wanted).abs(), start);
        let better = best.is_none_or(|b| {
            if (key.0 - b.0).abs() > EPS_M {
                return key.0 < b.0;
            }
            if (key.1 - b.1).abs() > EPS_M {
                return key.1 < b.1;
            }
            key.2 < b.2
        });
        if better {
            best = Some(key);
        }
    }
    // the ends of a range that is a range are candidates, so there is a best
    let (missing, _, start) = best.unwrap_or((f64::INFINITY, 0.0, wanted));
    if missing <= EPS_M {
        Fit::Fits { start }
    } else {
        Fit::Blocked { start, missing }
    }
}

/// What the bridge got for each road below, `chain` lying `start` from the bridge's start.
#[must_use]
pub fn below_fits(chain: &Chain, start: f64, below: &[Below], needs: &[Need]) -> Vec<BelowFit> {
    below
        .iter()
        .zip(needs)
        .map(|(b, need)| BelowFit {
            lower: b.lower.clone(),
            at: b.at,
            station: b.station,
            interval: (
                b.station - need.reach.total(),
                b.station + need.reach.total(),
            ),
            missing: missing_at(chain, start, need),
            placed: true,
        })
        .collect()
}

/// Whether the placed road `slot` stretch (a junction piece has two ids joined by `|`) is the stretch `id` or a part
/// split from it (the dip pass names the parts `<id>/after<k>`).
fn is_part_of(slot: &str, id: &str) -> bool {
    slot.split('|')
        .any(|p| p == id || p.strip_prefix(id).is_some_and(|rest| rest.starts_with('/')))
}

/// Sets [`BelowFit::placed`] of every road below of `fits`: whether a placed piece of that road (the stretch or a
/// part split from it) has its centre line inside the outline of the placed deck of the bridge, widened by the
/// road's half width (a step of a grid is measured by its chord and width, so a grid that follows a bend is
/// followed). A bridge whose deck is not in `roads` leaves them all placed.
pub fn mark_placed(fits: &mut [crate::bridgekind::BridgeFit], roads: &[PlacedRoad]) {
    for fit in fits {
        let deck: Vec<&PlacedRoad> = roads
            .iter()
            .filter(|r| r.kind == PlacedKind::Bridge && is_part_of(&r.slot.stretch, &fit.stretch))
            .collect();
        if deck.is_empty() {
            continue;
        }
        for below in &mut fit.below {
            below.placed = roads
                .iter()
                .filter(|r| is_part_of(&r.slot.stretch, &below.lower))
                .any(|piece| {
                    deck.iter().any(|step| {
                        segment_distance(
                            (piece.start.x, piece.start.y),
                            (piece.end.x, piece.end.y),
                            (step.start.x, step.start.y),
                            (step.end.x, step.end.y),
                        ) <= f64::midpoint(piece.width, step.width)
                    })
                });
        }
    }
}

/// The distance between the segments `a`-`b` and `c`-`d` (0 where they cross).
fn segment_distance(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> f64 {
    let side = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| {
        (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
    };
    let crossing = side(a, b, c) * side(a, b, d) < 0.0 && side(c, d, a) * side(c, d, b) < 0.0;
    if crossing {
        return 0.0;
    }
    [
        distance_to_segment(a, c, d),
        distance_to_segment(b, c, d),
        distance_to_segment(c, a, b),
        distance_to_segment(d, a, b),
    ]
    .into_iter()
    .fold(f64::INFINITY, f64::min)
}

/// The distance from `p` to the segment `a`-`b`.
fn distance_to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length2 = dx * dx + dy * dy;
    let t = if length2 > 1e-12 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The sites of the in-game road check (`tools/ingame-check/`): places on a built map that show how the
//! roads look, picked from the build's own data (`build/report.json` and the placed road pieces).
//!
//! The list is plain data (`sites.json`), so that a later run photographs exactly the sites of an earlier
//! one. Positions are map metres (x east, z north); the yaw is degrees clockwise from north.

use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

use naves_core::conform::FACE_SIDE_SLOPE;
use naves_core::profile::RoadStrip;
use naves_core::roadplace::{PlacedKind, PlacedRoad};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Two picked sites are at least this far apart (fixed sites are exempt).
pub const MIN_APART_M: f64 = 100.0;

/// Chains of arcs that turn at least this far count as a hairpin.
const HAIRPIN_DEG: f64 = 150.0;

/// A heading change below this is a straight piece.
const STRAIGHT_RAD: f64 = 0.5 * PI / 180.0;

/// Pieces whose ends are farther apart than this do not meet.
const MEET_M: f64 = 3.0;

/// Heading changes (degrees) that the "slight angle" sites aim for: the interior angle is 180 minus this.
const SLIGHT_TARGETS_DEG: [f64; 5] = [2.0, 5.0, 9.0, 15.0, 25.0];

/// An OSM bridge is this close to a bridge deck (metres): the bridge is not one over a dip.
const OSM_BRIDGE_NEAR_M: f64 = 20.0;

/// A painted path is at least this long, and this far from roads and from steep findings (metres).
const PATH_MIN_LENGTH_M: f64 = 150.0;
const PATH_APART_M: f64 = 60.0;
const STEEP_APART_M: f64 = 80.0;

/// What the site picking knows about the terrain, the height profiles of the roads and the paths.
pub struct Terrain<'a> {
    /// The height profile of each chain (`Profiles::strips` of the placement).
    pub strips: &'a [RoadStrip],
    /// The height of the original ground at a map position.
    pub ground: &'a dyn Fn(f64, f64) -> f64,
    /// The painted paths, as map positions.
    pub paths: &'a [Vec<(f64, f64)>],
    /// The bridges that OpenStreetMap has (the others span dips), as map positions.
    pub osm_bridges: &'a [Vec<(f64, f64)>],
}

impl Terrain<'_> {
    /// No terrain: only the sites that the pieces and the report give.
    #[must_use]
    pub fn none() -> Terrain<'static> {
        Terrain {
            strips: &[],
            ground: &|_, _| 0.0,
            paths: &[],
            osm_bridges: &[],
        }
    }
}

/// One place to photograph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Site {
    /// Number in the list, from 1; the shots are named after it.
    pub id: u32,
    /// What the site shows: `slight-angle`, `tight-arc`, `hairpin`, `junction`, `end-cap`, `bridge-<material>`,
    /// `bridge-scaled`, `bridge-over-dip`, `high-embankment`, `deep-cutting`, `rock-face`, `painted-path`,
    /// `steep-path`, `finding-<rule>`, or the category of a fixed site.
    pub category: String,
    /// East, map metres.
    pub x: f64,
    /// North, map metres.
    pub z: f64,
    /// Looking direction, degrees clockwise from north.
    pub yaw_deg: f64,
    /// Measurements that make the site worth a look.
    pub numbers: BTreeMap<String, f64>,
    /// What to look at.
    pub note: String,
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn wrap(a: f64) -> f64 {
    let r = a.rem_euclid(TAU);
    if r > PI { r - TAU } else { r }
}

fn deg(a: f64) -> f64 {
    a.to_degrees().rem_euclid(360.0)
}

/// The direction of the piece from its start to its end.
fn chord_yaw(p: &PlacedRoad) -> f64 {
    let (dx, dy) = (p.end.x - p.start.x, p.end.y - p.start.y);
    if dx.hypot(dy) < 1e-6 {
        deg(p.start.heading)
    } else {
        deg(dx.atan2(dy))
    }
}

fn middle(p: &PlacedRoad) -> (f64, f64) {
    (
        f64::midpoint(p.start.x, p.end.x),
        f64::midpoint(p.start.y, p.end.y),
    )
}

fn turn(p: &PlacedRoad) -> f64 {
    wrap(p.end.heading - p.start.heading)
}

fn is_straight(p: &PlacedRoad) -> bool {
    p.kind == PlacedKind::Piece && turn(p).abs() < STRAIGHT_RAD
}

fn is_arc(p: &PlacedRoad) -> bool {
    p.kind == PlacedKind::Piece && !is_straight(p)
}

fn meets(a: &PlacedRoad, b: &PlacedRoad) -> bool {
    (a.end.x - b.start.x).hypot(a.end.y - b.start.y) < MEET_M
}

/// The radius of an arc piece from its chord and its turn.
fn radius(p: &PlacedRoad) -> f64 {
    let chord = (p.end.x - p.start.x).hypot(p.end.y - p.start.y);
    let s = (turn(p).abs() / 2.0).sin();
    if s > 1e-9 {
        chord / (2.0 * s)
    } else {
        f64::INFINITY
    }
}

/// The direction of the road nearest to `(x, z)`, degrees clockwise from north; 0 without roads.
#[must_use]
pub fn yaw_near(objects: &[PlacedRoad], x: f64, z: f64) -> f64 {
    objects
        .iter()
        .filter(|o| o.kind == PlacedKind::Piece || o.kind == PlacedKind::Bridge)
        .map(|o| {
            let (mx, mz) = middle(o);
            ((mx - x).hypot(mz - z), o)
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map_or(0.0, |(_, o)| chord_yaw(o))
}

/// Collects sites and refuses those too close to one already picked.
struct Picker<'a> {
    fixed: &'a [Site],
    picked: Vec<Site>,
}

impl Picker<'_> {
    fn try_add(
        &mut self,
        category: &str,
        at: (f64, f64),
        yaw: f64,
        numbers: &[(&str, f64)],
        note: &str,
    ) -> bool {
        let (x, z) = (round1(at.0), round1(at.1));
        if !(x.is_finite() && z.is_finite() && yaw.is_finite()) || x < 0.0 || z < 0.0 {
            return false;
        }
        let near = |s: &Site| (s.x - x).hypot(s.z - z) < MIN_APART_M;
        if self.fixed.iter().any(near) || self.picked.iter().any(near) {
            return false;
        }
        self.picked.push(Site {
            id: 0,
            category: category.to_string(),
            x,
            z,
            yaw_deg: round1(yaw),
            numbers: numbers
                .iter()
                .map(|(k, v)| ((*k).to_string(), round1(*v)))
                .collect(),
            note: note.to_string(),
        });
        true
    }
}

/// One end of a straight piece: where it is, which way it points out of the piece, and whose it is.
struct Port {
    x: f64,
    y: f64,
    out_yaw: f64,
    piece: usize,
}

/// The grid cell of a position, for finding ports that meet.
#[expect(
    clippy::cast_possible_truncation,
    reason = "positions are clamped to a range that fits an i64 cell index"
)]
fn cell(v: f64) -> i64 {
    (v / MEET_M).floor().clamp(-1e12, 1e12) as i64
}

/// Straight pieces that meet at a slight angle: not the neighbours of one chain, which join exactly, but the
/// ends of different stretches that were placed to meet.
fn slight_angles(objects: &[PlacedRoad], picker: &mut Picker<'_>) {
    let mut grid: BTreeMap<(i64, i64), Vec<Port>> = BTreeMap::new();
    for (i, o) in objects.iter().enumerate().filter(|(_, o)| is_straight(o)) {
        let yaw = chord_yaw(o);
        for (x, y, out_yaw) in [
            (o.start.x, o.start.y, (yaw + 180.0).rem_euclid(360.0)),
            (o.end.x, o.end.y, yaw),
        ] {
            grid.entry((cell(x), cell(y))).or_default().push(Port {
                x,
                y,
                out_yaw,
                piece: i,
            });
        }
    }
    // (heading change in degrees, where, yaw looking at the meeting point, gap)
    let mut found: Vec<(f64, (f64, f64), f64, f64)> = Vec::new();
    for ((cx, cy), ports) in &grid {
        for a in ports {
            for (dx, dy) in [
                (0, 0),
                (1, 0),
                (-1, 0),
                (0, 1),
                (0, -1),
                (1, 1),
                (1, -1),
                (-1, 1),
                (-1, -1),
            ] {
                let Some(others) = grid.get(&(cx + dx, cy + dy)) else {
                    continue;
                };
                for b in others.iter().filter(|b| b.piece > a.piece) {
                    let gap = (a.x - b.x).hypot(a.y - b.y);
                    if gap >= MEET_M {
                        continue;
                    }
                    let change = wrap((a.out_yaw - b.out_yaw - 180.0).to_radians())
                        .abs()
                        .to_degrees();
                    if (1.0..=30.0).contains(&change) {
                        found.push((
                            change,
                            (f64::midpoint(a.x, b.x), f64::midpoint(a.y, b.y)),
                            a.out_yaw,
                            gap,
                        ));
                    }
                }
            }
        }
    }
    for target in SLIGHT_TARGETS_DEG {
        let mut by_distance: Vec<&(f64, (f64, f64), f64, f64)> = found.iter().collect();
        by_distance.sort_by(|p, q| {
            (p.0 - target)
                .abs()
                .total_cmp(&(q.0 - target).abs())
                .then(p.1.0.total_cmp(&q.1.0))
                .then(p.1.1.total_cmp(&q.1.1))
        });
        for (change, at, yaw, gap) in by_distance {
            let numbers = [
                ("heading_change_deg", *change),
                ("interior_angle_deg", 180.0 - change),
                ("gap_m", *gap),
            ];
            if picker.try_add(
                "slight-angle",
                *at,
                *yaw,
                &numbers,
                "two straight pieces meet at an angle",
            ) {
                break;
            }
        }
    }
}

/// A run of arcs that turn the same way and meet.
struct ArcRun<'a> {
    pieces: Vec<&'a PlacedRoad>,
}

impl ArcRun<'_> {
    fn total_turn_deg(&self) -> f64 {
        self.pieces.iter().map(|p| turn(p).abs().to_degrees()).sum()
    }

    fn min_radius(&self) -> f64 {
        self.pieces
            .iter()
            .map(|p| radius(p))
            .fold(f64::INFINITY, f64::min)
    }

    fn middle_piece(&self) -> Option<&PlacedRoad> {
        self.pieces.get(self.pieces.len() / 2).copied()
    }
}

fn arc_runs(objects: &[PlacedRoad]) -> Vec<ArcRun<'_>> {
    let mut runs: Vec<ArcRun<'_>> = Vec::new();
    let mut last: Option<&PlacedRoad> = None;
    for o in objects {
        if !is_arc(o) {
            last = None;
            continue;
        }
        let continues = last.is_some_and(|l| meets(l, o) && turn(l).signum() == turn(o).signum());
        match runs.last_mut() {
            Some(run) if continues => run.pieces.push(o),
            _ => runs.push(ArcRun { pieces: vec![o] }),
        }
        last = Some(o);
    }
    runs
}

fn arcs(objects: &[PlacedRoad], picker: &mut Picker<'_>) {
    let runs = arc_runs(objects);
    let site = |run: &ArcRun<'_>, picker: &mut Picker<'_>, category: &str, note: &str| {
        let Some(mid) = run.middle_piece() else {
            return false;
        };
        let numbers = [
            ("turn_deg", run.total_turn_deg()),
            ("min_radius_m", run.min_radius()),
            (
                "pieces",
                f64::from(u32::try_from(run.pieces.len()).unwrap_or(u32::MAX)),
            ),
        ];
        picker.try_add(category, middle(mid), chord_yaw(mid), &numbers, note)
    };
    let mut hairpins: Vec<&ArcRun<'_>> = runs
        .iter()
        .filter(|r| r.total_turn_deg() >= HAIRPIN_DEG)
        .collect();
    hairpins.sort_by(|a, b| b.total_turn_deg().total_cmp(&a.total_turn_deg()));
    let mut added = 0;
    for run in hairpins {
        if added == 2 {
            break;
        }
        if site(run, picker, "hairpin", "arcs that turn back on themselves") {
            added += 1;
        }
    }
    let mut tight: Vec<&ArcRun<'_>> = runs.iter().collect();
    tight.sort_by(|a, b| {
        a.min_radius().total_cmp(&b.min_radius()).then_with(|| {
            let key = |r: &ArcRun<'_>| r.middle_piece().map_or(0.0, |p| middle(p).0);
            key(a).total_cmp(&key(b))
        })
    });
    let mut added = 0;
    for run in tight {
        if added == 3 {
            break;
        }
        if site(run, picker, "tight-arc", "the tightest arc pieces") {
            added += 1;
        }
    }
}

/// The first object of each model among `kind`, up to `limit` models.
fn per_model(objects: &[PlacedRoad], kind: PlacedKind, limit: usize) -> Vec<&PlacedRoad> {
    let mut seen: BTreeMap<&str, &PlacedRoad> = BTreeMap::new();
    for o in objects.iter().filter(|o| o.kind == kind) {
        seen.entry(o.model.as_str()).or_insert(o);
    }
    seen.into_values().take(limit).collect()
}

fn material(model: &str) -> &'static str {
    let m = model.to_ascii_lowercase();
    if m.contains("metal") {
        "metal"
    } else if m.contains("stone") {
        "stone"
    } else if m.contains("wood") {
        "wood"
    } else {
        "other"
    }
}

fn objects_sites(objects: &[PlacedRoad], picker: &mut Picker<'_>) {
    for (kind, category, note, limit) in [
        (PlacedKind::Junction, "junction", "a junction piece", 3),
        (PlacedKind::EndCap, "end-cap", "a dead end", 2),
    ] {
        for o in per_model(objects, kind, limit) {
            let numbers = [("width_m", o.width)];
            picker.try_add(
                category,
                (o.origin.x, o.origin.y),
                deg(o.origin.heading),
                &numbers,
                note,
            );
        }
    }
    // one bridge per material and one per scaled model
    let mut by_material: BTreeMap<String, &PlacedRoad> = BTreeMap::new();
    let mut scaled: Option<&PlacedRoad> = None;
    for o in objects.iter().filter(|o| o.kind == PlacedKind::Bridge) {
        by_material
            .entry(format!("bridge-{}", material(&o.model)))
            .or_insert(o);
        if o.scale > 1.0 + 1e-6 && scaled.is_none() {
            scaled = Some(o);
        }
    }
    let scaled = scaled.map(|o| ("bridge-scaled".to_string(), o));
    for (category, o) in by_material.into_iter().chain(scaled) {
        let numbers = [("scale", o.scale), ("width_m", o.width)];
        picker.try_add(
            &category,
            middle(o),
            chord_yaw(o),
            &numbers,
            "a bridge model",
        );
    }
}

/// The first measure of a finding's message: a number followed by `m`, `%` or `°`.
fn measure(message: &str) -> Option<f64> {
    let tokens: Vec<&str> = message.split_whitespace().collect();
    for (i, t) in tokens.iter().enumerate() {
        let t = t.trim_matches(|c| c == ',' || c == '(' || c == ')');
        if let Some(v) = t.strip_suffix('°').and_then(|n| n.parse::<f64>().ok()) {
            return Some(v);
        }
        let unit = tokens
            .get(i + 1)
            .map(|u| u.trim_matches(|c| c == ',' || c == '(' || c == ')'));
        if matches!(unit, Some("m" | "%"))
            && let Ok(v) = t.parse::<f64>()
        {
            return Some(v);
        }
    }
    None
}

/// A finding of the report: rule, measure, position and message.
struct Finding {
    measure: Option<f64>,
    x: f64,
    z: f64,
    message: String,
}

fn findings(report: &Value) -> BTreeMap<String, Vec<Finding>> {
    let mut by_rule: BTreeMap<String, Vec<Finding>> = BTreeMap::new();
    let Some(list) = report.get("violations").and_then(Value::as_array) else {
        return by_rule;
    };
    for v in list {
        let (Some(rule), Some(x), Some(z)) = (
            v.get("rule").and_then(Value::as_str),
            v.get("x").and_then(Value::as_f64),
            v.get("y").and_then(Value::as_f64),
        ) else {
            continue;
        };
        if !(x.is_finite() && z.is_finite()) {
            continue;
        }
        let message = v
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        by_rule.entry(rule.to_string()).or_default().push(Finding {
            measure: measure(&message).filter(|m| m.is_finite()),
            x,
            z,
            message,
        });
    }
    by_rule
}

fn finding_sites(report: &Value, objects: &[PlacedRoad], picker: &mut Picker<'_>) {
    for (rule, mut list) in findings(report) {
        // the worst first; for a vertical radius a smaller one is worse
        let sign = if rule == "road_grade" { -1.0 } else { 1.0 };
        list.sort_by(|a, b| {
            let key = |f: &Finding| sign * f.measure.unwrap_or(0.0);
            key(b)
                .total_cmp(&key(a))
                .then(a.x.total_cmp(&b.x))
                .then(a.z.total_cmp(&b.z))
        });
        let mut added = 0;
        for f in &list {
            if added == 2 {
                break;
            }
            let numbers: Vec<(&str, f64)> = f.measure.map(|m| ("measure", m)).into_iter().collect();
            let category = format!("finding-{rule}");
            if picker.try_add(
                &category,
                (f.x, f.z),
                yaw_near(objects, f.x, f.z),
                &numbers,
                &f.message,
            ) {
                added += 1;
            }
        }
    }
}

/// Picks the sites of the check: slight angles between straight pieces, the tightest arcs and hairpins, a
/// junction, a dead end and a bridge of each material and a scaled one from `objects`, the worst of each
/// terrain fault (embankment, cutting, rock face, bridge over a dip, steep and gentle painted path) from
/// `terrain`, the worst two
/// findings of each rule from `report` (`build/report.json`, untrusted), then `fixed` sites that are always
/// kept. Picked sites are at least [`MIN_APART_M`] from each other and from the fixed ones. Same input, same
/// list; the ids run from 1.
#[must_use]
pub fn pick_sites(
    report: &Value,
    objects: &[PlacedRoad],
    terrain: &Terrain<'_>,
    fixed: &[Site],
) -> Vec<Site> {
    let mut picker = Picker {
        fixed,
        picked: Vec::new(),
    };
    slight_angles(objects, &mut picker);
    arcs(objects, &mut picker);
    terrain_sites(terrain, &mut picker);
    path_sites(terrain, report, objects, &mut picker);
    objects_sites(objects, &mut picker);
    finding_sites(report, objects, &mut picker);
    let mut sites = picker.picked;
    sites.extend(fixed.iter().cloned());
    for (i, s) in sites.iter_mut().enumerate() {
        s.id = u32::try_from(i + 1).unwrap_or(u32::MAX);
    }
    sites
}

/// One segment of a height profile: its middle, its direction and the heights of the road and the ground.
struct Segment {
    at: (f64, f64),
    yaw: f64,
    road_z: f64,
    ground_z: f64,
    bridge: bool,
    side_slope: f64,
}

fn segments(terrain: &Terrain<'_>) -> Vec<Segment> {
    let mut out = Vec::new();
    for strip in terrain.strips {
        for (i, w) in strip.joints.windows(2).enumerate() {
            let [a, b] = w else { continue };
            let at = (f64::midpoint(a.0, b.0), f64::midpoint(a.1, b.1));
            let seg = Segment {
                at,
                yaw: deg((b.0 - a.0).atan2(b.1 - a.1)),
                road_z: f64::midpoint(a.2, b.2),
                ground_z: (terrain.ground)(at.0, at.1),
                bridge: strip.bridge.get(i).copied().unwrap_or(false),
                side_slope: strip.side_slope.get(i).copied().unwrap_or(0.0),
            };
            if [seg.at.0, seg.at.1, seg.yaw, seg.road_z, seg.ground_z]
                .iter()
                .all(|v| v.is_finite())
            {
                out.push(seg);
            }
        }
    }
    out
}

fn near_line(lines: &[Vec<(f64, f64)>], x: f64, z: f64, within: f64) -> bool {
    lines
        .iter()
        .flatten()
        .any(|p| (p.0 - x).hypot(p.1 - z) < within)
}

/// The direction of the path nearest to `(x, z)`, degrees clockwise from north.
fn path_yaw_near(paths: &[Vec<(f64, f64)>], at_x: f64, at_z: f64) -> f64 {
    let mut best: Option<(f64, f64)> = None;
    for line in paths {
        for (i, p) in line.iter().enumerate() {
            let d = (p.0 - at_x).hypot(p.1 - at_z);
            let (from, to) = match (line.get(i.wrapping_sub(1)), line.get(i + 1)) {
                (Some(before), Some(after)) => (*before, *after),
                (None, Some(after)) => (*p, *after),
                (Some(before), None) => (*before, *p),
                (None, None) => continue,
            };
            if best.is_none_or(|(nearest, _)| d < nearest) {
                best = Some((d, deg((to.0 - from.0).atan2(to.1 - from.1))));
            }
        }
    }
    best.map_or(0.0, |(_, yaw)| yaw)
}

fn terrain_sites(terrain: &Terrain<'_>, picker: &mut Picker<'_>) {
    let segs = segments(terrain);
    // the first of `order` that is far enough from the sites picked so far
    let pick = |picker: &mut Picker<'_>,
                category: &str,
                mut order: Vec<(&Segment, f64)>,
                key: &str,
                note: &str| {
        order.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then(a.0.at.0.total_cmp(&b.0.at.0))
                .then(a.0.at.1.total_cmp(&b.0.at.1))
        });
        for (s, v) in order {
            if picker.try_add(category, s.at, s.yaw, &[(key, v)], note) {
                break;
            }
        }
    };
    let open: Vec<&Segment> = segs.iter().filter(|s| !s.bridge).collect();
    pick(
        picker,
        "high-embankment",
        open.iter()
            .map(|s| (*s, s.road_z - s.ground_z))
            .filter(|c| c.1 > 0.0)
            .collect(),
        "road_above_ground_m",
        "the road on a fill: the ground beside it is lower",
    );
    pick(
        picker,
        "deep-cutting",
        open.iter()
            .map(|s| (*s, s.ground_z - s.road_z))
            .filter(|c| c.1 > 0.0)
            .collect(),
        "road_below_ground_m",
        "the road in a cut: the ground beside it is higher",
    );
    pick(
        picker,
        "rock-face",
        open.iter()
            .map(|s| (*s, s.side_slope))
            .filter(|c| c.1 > FACE_SIDE_SLOPE)
            .collect(),
        "side_slope",
        "a steep side slope beside the road: the earthworks leave a face",
    );
    // a bridge that OpenStreetMap does not have spans a dip; with none of those, any bridge will do
    let decks: Vec<&Segment> = segs.iter().filter(|s| s.bridge).collect();
    let dips: Vec<&Segment> = decks
        .iter()
        .copied()
        .filter(|s| !near_line(terrain.osm_bridges, s.at.0, s.at.1, OSM_BRIDGE_NEAR_M))
        .collect();
    let spans = if dips.is_empty() { decks } else { dips };
    pick(
        picker,
        "bridge-over-dip",
        spans.iter().map(|s| (*s, s.road_z - s.ground_z)).collect(),
        "deck_above_ground_m",
        "a bridge over a dip",
    );
}

/// The steepest painted path of the report, and a long gentle one away from roads and from steep findings.
fn path_sites(
    terrain: &Terrain<'_>,
    report: &Value,
    objects: &[PlacedRoad],
    picker: &mut Picker<'_>,
) {
    let steep: Vec<(f64, f64, f64)> = findings(report)
        .remove("path_too_steep")
        .unwrap_or_default()
        .into_iter()
        .map(|f| (f.measure.unwrap_or(0.0), f.x, f.z))
        .collect();
    let mut steepest = steep.clone();
    steepest.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then(a.1.total_cmp(&b.1))
            .then(a.2.total_cmp(&b.2))
    });
    for (m, x, z) in steepest {
        let yaw = path_yaw_near(terrain.paths, x, z);
        if picker.try_add(
            "steep-path",
            (x, z),
            yaw,
            &[("slope_deg", m)],
            "a painted path that is too steep to walk",
        ) {
            break;
        }
    }
    let mut long: Vec<(f64, (f64, f64), f64)> = Vec::new();
    for line in terrain.paths {
        let length: f64 = line
            .windows(2)
            .map(|w| match w {
                [a, b] => (b.0 - a.0).hypot(b.1 - a.1),
                _ => 0.0,
            })
            .sum();
        let Some(&mid) = line.get(line.len() / 2) else {
            continue;
        };
        let far_from_roads = objects.iter().all(|o| {
            let m = middle(o);
            (m.0 - mid.0).hypot(m.1 - mid.1) >= PATH_APART_M
        });
        let far_from_steep = steep
            .iter()
            .all(|s| (s.1 - mid.0).hypot(s.2 - mid.1) >= STEEP_APART_M);
        if length >= PATH_MIN_LENGTH_M && far_from_roads && far_from_steep && length.is_finite() {
            long.push((
                length,
                mid,
                path_yaw_near(std::slice::from_ref(line), mid.0, mid.1),
            ));
        }
    }
    long.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then(a.1.0.total_cmp(&b.1.0))
            .then(a.1.1.total_cmp(&b.1.1))
    });
    for (length, mid, yaw) in long {
        if picker.try_add(
            "painted-path",
            mid,
            yaw,
            &[("length_m", length)],
            "a painted path on gentle ground",
        ) {
            break;
        }
    }
}

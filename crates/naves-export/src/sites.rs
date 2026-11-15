// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The sites of the in-game road check (`tools/ingame-check/`): places on a built map that show how the
//! roads look, picked from the build's own data (`build/report.json` and the placed road pieces).
//!
//! The list is plain data (`sites.json`), so that a later run photographs exactly the sites of an earlier
//! one. Positions are map metres (x east, z north); the yaw is degrees clockwise from north.

use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

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

/// One place to photograph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Site {
    /// Number in the list, from 1; the shots are named after it.
    pub id: u32,
    /// What the site shows: `slight-angle`, `tight-arc`, `hairpin`, `junction`, `end-cap`, `bridge-<material>`,
    /// `bridge-scaled`, `finding-<rule>`, or the category of a fixed site.
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
/// junction, a dead end and a bridge of each material and a scaled one from `objects`, the worst two
/// findings of each rule from `report` (`build/report.json`, untrusted), then `fixed` sites that are always
/// kept. Picked sites are at least [`MIN_APART_M`] from each other and from the fixed ones. Same input, same
/// list; the ids run from 1.
#[must_use]
pub fn pick_sites(report: &Value, objects: &[PlacedRoad], fixed: &[Site]) -> Vec<Site> {
    let mut picker = Picker {
        fixed,
        picked: Vec::new(),
    };
    slight_angles(objects, &mut picker);
    arcs(objects, &mut picker);
    objects_sites(objects, &mut picker);
    finding_sites(report, objects, &mut picker);
    let mut sites = picker.picked;
    sites.extend(fixed.iter().cloned());
    for (i, s) in sites.iter_mut().enumerate() {
        s.id = u32::try_from(i + 1).unwrap_or(u32::MAX);
    }
    sites
}

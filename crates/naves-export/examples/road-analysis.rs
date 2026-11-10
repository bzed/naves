// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The measurement of `docs/research/2026-11-10-road-alignment.md`: it breaks the road violations of the
//! Berchtesgaden example down by cause. It needs the example's data (`examples/berchtesgaden/README.md`) and
//! the DayZ tools.
//!
//! `cargo run --release -p naves-export --example road-analysis [terrain_size]`

#![expect(
    clippy::indexing_slicing,
    clippy::print_stdout,
    clippy::expect_used,
    clippy::manual_midpoint,
    clippy::cast_precision_loss,
    clippy::many_single_char_names,
    clippy::cast_possible_truncation,
    clippy::unwrap_used,
    clippy::similar_names,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::type_complexity,
    reason = "a measuring tool for developers; failing loudly is the point"
)]

use std::collections::BTreeMap;
use std::path::Path;

use naves_core::checks::{DriveState, PieceCheck, check_roads_on_terrain};
use naves_core::conform::conform;
use naves_core::dem::Dem;
use naves_core::features::road_features;
use naves_core::heightmap::{Heightmap, MapGeo, build_heightmap};
use naves_core::limits::{CROSS_FALL_LIMIT, drive_grade, grade_aim};
use naves_core::osm::read_ways;
use naves_core::profile::{RoadStrip, solve_profiles};
use naves_core::project::Project;
use naves_core::roadnet::build_graph;
use naves_core::roadplace::{PlacedKind, place_network};
use naves_export::catalog_read::road_catalog;
use naves_export::tools::Tools;

fn len(p: &PieceCheck) -> f64 {
    (p.end.0 - p.start.0).hypot(p.end.1 - p.start.1)
}

fn pct(values: &mut [f64], q: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let i = ((values.len() - 1) as f64 * q).round() as usize;
    values[i]
}

/// Distance from a point to a segment and the parameter of the closest point.
fn seg_dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ((p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy), t)
}

#[derive(Default, Clone, Copy)]
struct Stat {
    n: usize,
    m: f64,
}

struct Seg {
    strip: usize,
    index: usize,
    along: f64,
    a: (f64, f64, f64),
    b: (f64, f64, f64),
    bridge: bool,
}

fn main() {
    let size: Option<u32> = std::env::args().nth(1).and_then(|s| s.parse().ok());
    let dir = Path::new("examples/berchtesgaden");
    let mut project = Project::load(dir).expect("project");
    if let Some(size) = size {
        project.grid.terrain_size = size;
        project.grid.land_size = size / 8;
    }
    let tools = Tools::locate().expect("tools");
    let dem_files: Vec<_> = project.sources.dem.iter().map(|p| dir.join(p)).collect();
    let dem = Dem::open(&dem_files).expect("dem");
    let base = build_heightmap(&project, &dem).expect("heightmap");
    let frame = MapGeo::new(&project).expect("frame");
    let window = frame.window(100.0).expect("window");
    let files: Vec<_> = project.sources.osm.iter().map(|p| dir.join(p)).collect();
    let ways = read_ways(&files, window).expect("ways");
    let features = road_features(&ways, &frame, project.map_size_m());
    let catalog = road_catalog(&tools, &tools.cache_dir).expect("catalog");
    let graph = build_graph(&features);
    let net = place_network(&graph, &catalog).expect("place");
    let profiles = solve_profiles(&net, &base);
    let conformed = conform(&base, &profiles.strips).expect("conform");
    let pieces = check_roads_on_terrain(&net, &conformed.heightmap);
    let raw = check_roads_on_terrain(&net, &base);
    let raw_by: BTreeMap<usize, &PieceCheck> = raw.iter().map(|p| (p.slot, p)).collect();
    let cell = f64::from(base.cell_m);

    println!(
        "== map: cell {cell} m, {} pieces checked, {} chains, {} strips",
        pieces.len(),
        net.chains.len(),
        profiles.strips.len()
    );
    println!(
        "gaps {}, joins without junction {}, tunnels omitted {}, grade excesses {}, conflicts {}, capped {}, max cut {:.1} m, max fill {:.1} m",
        net.gaps.len(),
        net.joins_without_junction.len(),
        net.tunnels_omitted.len(),
        profiles.excesses.len(),
        conformed.conflicts.len(),
        conformed.capped.len(),
        conformed.max_cut_m,
        conformed.max_fill_m
    );

    // ---- horizontal alignment
    println!("\n== horizontal: joins between consecutive pieces of a chain");
    let (mut gaps, mut kinks) = (Vec::new(), Vec::new());
    for chain in &net.chains {
        for w in chain.windows(2) {
            let (a, b) = (&net.roads[w[0]], &net.roads[w[1]]);
            gaps.push((a.end.x - b.start.x).hypot(a.end.y - b.start.y));
            let mut d = (a.end.heading - b.start.heading).to_degrees().abs() % 360.0;
            if d > 180.0 {
                d = 360.0 - d;
            }
            kinks.push(d);
        }
    }
    println!(
        "joins {}: gap p50 {:.3} p99 {:.3} max {:.3} m; >0.1 m: {}, >0.5 m: {}",
        gaps.len(),
        pct(&mut gaps, 0.5),
        pct(&mut gaps, 0.99),
        pct(&mut gaps, 1.0),
        gaps.iter().filter(|g| **g > 0.1).count(),
        gaps.iter().filter(|g| **g > 0.5).count()
    );
    println!(
        "heading kink p50 {:.2} p99 {:.2} max {:.1} deg; >1 deg: {}, >5 deg: {}",
        pct(&mut kinks, 0.5),
        pct(&mut kinks, 0.99),
        pct(&mut kinks, 1.0),
        kinks.iter().filter(|g| **g > 1.0).count(),
        kinks.iter().filter(|g| **g > 5.0).count()
    );
    let mut gap_sizes: Vec<f64> = net.gaps.iter().map(|g| g.gap_m).collect();
    println!(
        "stretch end gaps (RoadGap): {} p50 {:.1} p90 {:.1} max {:.1} m",
        gap_sizes.len(),
        pct(&mut gap_sizes, 0.5),
        pct(&mut gap_sizes, 0.9),
        pct(&mut gap_sizes, 1.0)
    );
    // tight arcs: model names carry "<angle> <radius>"
    let mut tight: BTreeMap<(String, String), (usize, f64)> = BTreeMap::new();
    for r in net.roads.iter().filter(|r| r.kind == PlacedKind::Piece) {
        let name = r
            .model
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or("")
            .trim_end_matches(".p3d");
        let radius = name
            .rsplit(' ')
            .next()
            .filter(|_| name.contains(' '))
            .unwrap_or("straight");
        let e = tight
            .entry((format!("{:?}", r.family), radius.to_string()))
            .or_insert((0, 0.0));
        e.0 += 1;
        e.1 += (r.end.x - r.start.x).hypot(r.end.y - r.start.y);
    }
    println!("pieces by family and radius (count, km):");
    for ((fam, radius), (n, m)) in &tight {
        println!("  {fam:5} r={radius:8} {n:6} {:.2} km", m / 1000.0);
    }

    // ---- states
    println!("\n== drive states (drivable pieces), km");
    let drivable: Vec<&PieceCheck> = pieces
        .iter()
        .filter(|p| net.roads[p.slot].drivable)
        .collect();
    let total: f64 = drivable.iter().map(|p| len(p)).sum();
    let mut by_state: BTreeMap<String, f64> = BTreeMap::new();
    for p in &drivable {
        *by_state.entry(format!("{:?}", p.state)).or_default() += len(p);
    }
    println!("total {:.1} km: {by_state:.1?}", total / 1000.0);
    let hard = |p: &PieceCheck| drive_grade(p.family);
    let (mut only_grade, mut only_fall, mut both) = (0.0, 0.0, 0.0);
    for p in drivable
        .iter()
        .filter(|p| p.state == DriveState::NotDrivable)
    {
        match (p.grade > hard(p), p.cross_fall > CROSS_FALL_LIMIT) {
            (true, true) => both += len(p),
            (true, false) => only_grade += len(p),
            _ => only_fall += len(p),
        }
    }
    println!(
        "not drivable because: grade only {:.2} km, cross fall only {:.2} km, both {:.2} km",
        only_grade / 1000.0,
        only_fall / 1000.0,
        both / 1000.0
    );
    for limit in [0.05, 0.08, 0.10, 0.15, 0.20, 0.30, 0.50] {
        let km: f64 = drivable
            .iter()
            .filter(|p| p.cross_fall > limit)
            .map(|p| len(p))
            .sum();
        println!(
            "  cross fall > {:>2.0} %: {:.2} km",
            limit * 100.0,
            km / 1000.0
        );
    }
    for limit in [0.08, 0.12, 0.15, 0.20, 0.25, 0.30, 0.35, 0.50] {
        let km: f64 = drivable
            .iter()
            .filter(|p| p.grade > limit)
            .map(|p| len(p))
            .sum();
        let raw_km: f64 = drivable
            .iter()
            .filter(|p| raw_by.get(&p.slot).is_some_and(|r| r.grade > limit))
            .map(|p| len(p))
            .sum();
        println!(
            "  grade > {:>2.0} %: {:.2} km (on the unshaped terrain: {:.2} km)",
            limit * 100.0,
            km / 1000.0,
            raw_km / 1000.0
        );
    }
    for limit in [0.15, 0.3, 0.5, 1.0, 2.0, 5.0] {
        let n = pieces.iter().filter(|p| p.miss_m > limit).count();
        let raw_n = raw.iter().filter(|p| p.miss_m > limit).count();
        println!("  miss > {limit:.2} m: {n} pieces (unshaped terrain: {raw_n})");
    }
    println!(
        "by family: km, km steep (over aim), km over hard grade, km cross fall > 5 %, p50/p95 raw terrain side slope"
    );
    let mut fams: BTreeMap<String, Vec<&PieceCheck>> = BTreeMap::new();
    for p in &drivable {
        fams.entry(format!("{:?}", p.family)).or_default().push(p);
    }
    for (fam, list) in &fams {
        let km = |f: &dyn Fn(&PieceCheck) -> bool| {
            list.iter().filter(|p| f(p)).map(|p| len(p)).sum::<f64>() / 1000.0
        };
        let mut side: Vec<f64> = list
            .iter()
            .filter_map(|p| raw_by.get(&p.slot).map(|r| r.cross_fall))
            .collect();
        println!(
            "  {fam:5} {:6.1} {:6.2} {:6.2} {:6.2}   {:.2}/{:.2}",
            km(&|_| true),
            km(&|p| p.grade > grade_aim(p.family)),
            km(&|p| p.grade > hard(p)),
            km(&|p| p.cross_fall > 0.05),
            pct(&mut side, 0.5),
            pct(&mut side, 0.95)
        );
    }

    // ---- causes: geometry around each piece
    let mut segs: Vec<Seg> = Vec::new();
    let mut slot_seg: BTreeMap<usize, usize> = BTreeMap::new();
    for (si, strip) in profiles.strips.iter().enumerate() {
        let mut along = 0.0;
        for (i, w) in strip.joints.windows(2).enumerate() {
            if let Some(slot) = strip.slots.get(i) {
                slot_seg.insert(*slot, segs.len());
            }
            segs.push(Seg {
                strip: si,
                index: i,
                along,
                a: w[0],
                b: w[1],
                bridge: strip.bridge.get(i).copied().unwrap_or(false),
            });
            along += (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        }
    }
    let strip_len: Vec<f64> = profiles
        .strips
        .iter()
        .map(|st| {
            st.joints
                .windows(2)
                .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
                .sum()
        })
        .collect();
    let mut relation: BTreeMap<&'static str, Stat> = BTreeMap::new();
    let bucket = |v: f64| (v / 64.0).floor() as i64;
    let mut grid: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
    for (i, s) in segs.iter().enumerate() {
        grid.entry((bucket((s.a.0 + s.b.0) / 2.0), bucket((s.a.1 + s.b.1) / 2.0)))
            .or_default()
            .push(i);
    }
    let strips: &[RoadStrip] = &profiles.strips;
    let hm: &Heightmap = &conformed.heightmap;

    let mut causes: BTreeMap<&'static str, Stat> = BTreeMap::new();
    let mut fall_causes: BTreeMap<&'static str, Stat> = BTreeMap::new();
    let mut examples: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    let mut plan_vs_measured = Vec::new();
    let mut centre_shift = Vec::new();
    let (mut self_pairs, mut self_dh) = (0usize, Vec::new());
    let (mut all_m, mut near_other_m, mut near_other_diff_m, mut near_self_m) =
        (0.0, 0.0, 0.0, 0.0);
    let (mut leg_pairs, mut leg_short, mut leg_need) = (0usize, 0usize, Vec::new());
    for p in &pieces {
        let Some(&own) = slot_seg.get(&p.slot) else {
            continue;
        };
        let s = &segs[own];
        let length = len(p);
        let planned = (s.b.2 - s.a.2).abs() / length.max(0.5);
        plan_vs_measured.push(p.grade - planned);
        let mid = ((s.a.0 + s.b.0) / 2.0, (s.a.1 + s.b.1) / 2.0);
        let mid_h = (s.a.2 + s.b.2) / 2.0;
        centre_shift.push(mid_h - f64::from(base.sample(mid.0, mid.1).height));
        // nearest other road / other part of the own road, edge to edge
        let (mut other, mut own_far): (Option<(f64, f64)>, Option<(f64, f64)>) = (None, None);
        let (mut other_end, mut other_cos) = (f64::MAX, 0.0_f64);
        for gx in bucket(mid.0) - 1..=bucket(mid.0) + 1 {
            for gy in bucket(mid.1) - 1..=bucket(mid.1) + 1 {
                for &j in grid.get(&(gx, gy)).map_or(&[][..], Vec::as_slice) {
                    let o = &segs[j];
                    if j == own || o.bridge {
                        continue;
                    }
                    // sample the own piece at 5 points
                    for k in 0..=4 {
                        let t = f64::from(k) / 4.0;
                        let q = (s.a.0 + t * (s.b.0 - s.a.0), s.a.1 + t * (s.b.1 - s.a.1));
                        let qh = s.a.2 + t * (s.b.2 - s.a.2);
                        let (d, u) = seg_dist(q, (o.a.0, o.a.1), (o.b.0, o.b.1));
                        let oh = o.a.2 + u * (o.b.2 - o.a.2);
                        let clear = d - strips[s.strip].half_width - strips[o.strip].half_width;
                        let dh = (qh - oh).abs();
                        if o.strip == s.strip {
                            let run = (o.along + u * (o.b.0 - o.a.0).hypot(o.b.1 - o.a.1)
                                - (s.along + t * length))
                                .abs();
                            if run <= 1.4 * d + 3.0 || o.index.abs_diff(s.index) <= 1 {
                                continue;
                            }
                            if own_far.is_none_or(|(c, _)| clear < c) {
                                own_far = Some((clear, dh));
                            }
                        } else if other.is_none_or(|(c, _)| clear < c) {
                            other = Some((clear, dh));
                            let o_at = o.along + u * (o.b.0 - o.a.0).hypot(o.b.1 - o.a.1);
                            let s_at = s.along + t * length;
                            let ends = |at: f64, strip: usize| at.min(strip_len[strip] - at);
                            other_end = ends(o_at, o.strip).min(ends(s_at, s.strip));
                            // angle between the two roads
                            let (ax, ay) = (s.b.0 - s.a.0, s.b.1 - s.a.1);
                            let (bx, by) = (o.b.0 - o.a.0, o.b.1 - o.a.1);
                            other_cos = ((ax * bx + ay * by)
                                / (ax.hypot(ay) * bx.hypot(by)).max(1e-9))
                            .abs();
                        }
                    }
                }
            }
        }
        let margin = 3.0 * cell;
        if net.roads[p.slot].drivable {
            all_m += length;
            if other.is_some_and(|(c, _)| c < margin) {
                near_other_m += length;
            }
            if other.is_some_and(|(c, dh)| c < margin && dh > 0.1) {
                near_other_diff_m += length;
            }
            if own_far.is_some_and(|(c, _)| c < margin) {
                near_self_m += length;
            }
        }
        if let Some((c, dh)) = own_far {
            // what the ground between two legs can take: nothing within 3 cells, then the fill slope
            let room = (c - margin).max(0.0) * 0.667 + 0.1;
            if c < 60.0 {
                leg_pairs += 1;
                if dh > room {
                    leg_short += 1;
                    leg_need.push(dh - room);
                }
            }
        }
        if let Some((_, dh)) = own_far.filter(|(c, _)| *c < margin) {
            self_pairs += 1;
            self_dh.push(dh);
        }
        // how far the terrain under the piece's core is from its own profile
        let hw = p.width / 2.0;
        let mut core_off = 0.0_f64;
        for k in 0..=8 {
            let t = f64::from(k) / 8.0;
            let h = s.a.2 + t * (s.b.2 - s.a.2);
            let c = (s.a.0 + t * (s.b.0 - s.a.0), s.a.1 + t * (s.b.1 - s.a.1));
            let (nx, ny) = ((s.b.1 - s.a.1) / length, -(s.b.0 - s.a.0) / length);
            for side in [-1.0, 0.0, 1.0] {
                let q = (c.0 + nx * hw * side, c.1 + ny * hw * side);
                core_off = core_off.max((f64::from(hm.sample(q.0, q.1).height) - h).abs());
            }
        }
        let bad_fall = p.cross_fall > 0.05;
        let bad_miss = p.miss_m > 0.15;
        if !(bad_fall || bad_miss) {
            continue;
        }
        let junction_near = (s.index.saturating_sub(1)..=s.index + 1)
            .any(|i| strips[s.strip].junction.get(i).copied().unwrap_or(false));
        let near_other = other.filter(|(c, _)| *c < margin);
        let near_self = own_far.filter(|(c, _)| *c < margin);
        let model = net.roads[p.slot].model.rsplit('\\').next().unwrap_or("");
        let tight_arc = model.ends_with(" 10.p3d") || model.ends_with(" 25.p3d");
        let cause = match (near_other, near_self) {
            (Some((c, dh)), _) if dh > 0.1 && c < 0.0 => "other road overlaps, different height",
            (Some((_, dh)), _) if dh > 0.1 => "other road within 3 cells, different height",
            (_, Some((_, dh))) if dh > 0.1 => "own road doubling back (hairpin), different height",
            (Some((c, _)), _) if c < 0.0 => "other road overlaps, same height there",
            (Some(_), _) => "other road within 3 cells, same height there",
            (_, Some(_)) => "own road doubling back, same height",
            _ if junction_near => "alone, at a junction piece",
            _ if tight_arc => "alone, arc of radius 10 or 25 m",
            _ if core_off > 0.05 => "alone, terrain not on the profile",
            _ => "alone, terrain on the profile",
        };
        if near_other.is_some() && bad_fall {
            let kind = match (other_end < 30.0, other_cos > 0.87) {
                (true, _) => "within 30 m of an end of one of the two roads (a meeting)",
                (false, true) => "far from any end, roads parallel within 30 deg",
                (false, false) => "far from any end, roads at an angle (crossing or passing)",
            };
            let e = relation.entry(kind).or_default();
            e.n += 1;
            e.m += length;
        }
        for (flag, map) in [(true, &mut causes), (bad_fall, &mut fall_causes)] {
            if flag {
                let e = map.entry(cause).or_default();
                e.n += 1;
                e.m += length;
            }
        }
        let list = examples.entry(cause).or_default();
        if list.len() < 6 {
            list.push(format!(
                "    {:?} {:?} ({:.0},{:.0})-({:.0},{:.0}) len {:.1} planned grade {:.3} measured {:.3} fall {:.3} miss {:.2} core_off {:.2} other {:?} self {:?} model {}",
                p.family, net.roads[p.slot].kind, p.start.0, p.start.1, p.end.0, p.end.1, length, planned, p.grade, p.cross_fall, p.miss_m, core_off,
                other.map(|(c, d)| (format!("{c:.1}"), format!("{d:.2}"))), own_far.map(|(c, d)| (format!("{c:.1}"), format!("{d:.2}"))),
                net.roads[p.slot].model.rsplit('\\').next().unwrap_or("")
            ));
        }
    }
    println!("\n== causes of pieces with cross fall > 5 % or miss > 0.15 m (count, km)");
    for (cause, s) in &causes {
        let f = fall_causes.get(cause).copied().unwrap_or_default();
        println!(
            "  {cause:55} {:5} {:6.2} km   (cross fall: {:5} {:6.2} km)",
            s.n,
            s.m / 1000.0,
            f.n,
            f.m / 1000.0
        );
    }
    for (cause, list) in &examples {
        println!("  examples: {cause}");
        for l in list {
            println!("{l}");
        }
    }
    println!("cross fall > 5 % next to another road, by how the two roads relate:");
    for (k, v) in &relation {
        println!("  {k:62} {:5} {:6.2} km", v.n, v.m / 1000.0);
    }
    println!(
        "pieces within 3 cells of another part of their own road: {self_pairs}; height difference there p50 {:.2} p90 {:.2} max {:.2} m",
        pct(&mut self_dh, 0.5),
        pct(&mut self_dh, 0.9),
        pct(&mut self_dh, 1.0)
    );

    println!(
        "drivable road with another road within 3 cells of its edge: {:.1} of {:.1} km ({:.1} km at a different height); with another part of itself: {:.1} km",
        near_other_m / 1000.0,
        all_m / 1000.0,
        near_other_diff_m / 1000.0,
        near_self_m / 1000.0
    );
    println!(
        "pieces with another part of their own road within 60 m: {leg_pairs}; of these with more height difference than flat margins plus a 34 deg slope allow: {leg_short} (short by p50 {:.1} p90 {:.1} m)",
        pct(&mut leg_need, 0.5),
        pct(&mut leg_need, 0.9)
    );

    // ---- junctions: level along their length
    let (mut jn, mut before_over, mut jcurv) = (0usize, 0usize, Vec::new());
    for strip in strips {
        let Some(fam) = strip.slots.first().map(|s| net.roads[*s].family) else {
            continue;
        };
        let g: Vec<(f64, f64)> = strip
            .joints
            .windows(2)
            .map(|w| {
                let l = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1).max(0.5);
                ((w[1].2 - w[0].2) / l, l)
            })
            .collect();
        for i in 0..g.len() {
            if !strip.junction.get(i).copied().unwrap_or(false) {
                continue;
            }
            jn += 1;
            for n in [i.checked_sub(1), Some(i + 1)].into_iter().flatten() {
                let Some(&(grade, l)) = g.get(n) else {
                    continue;
                };
                if strip.junction.get(n).copied().unwrap_or(false)
                    || strip.bridge.get(n).copied().unwrap_or(false)
                {
                    continue;
                }
                if grade.abs() > grade_aim(fam) + 1e-6 {
                    before_over += 1;
                }
                jcurv.push((grade - g[i].0).abs() / ((l + g[i].1) / 2.0) * 100.0);
            }
        }
    }
    println!(
        "\n== junctions: {jn} junction pieces; neighbouring pieces planned over the family's aim: {before_over}; grade change into the junction p50 {:.2} p90 {:.2} max {:.2} %/m",
        pct(&mut jcurv, 0.5),
        pct(&mut jcurv, 0.9),
        pct(&mut jcurv, 1.0)
    );

    // ---- is the real road steeper than the aim? terrain grade over 200 m of road
    println!("\n== terrain under the road over 200 m windows against the family's aim");
    let mut long: BTreeMap<String, (f64, f64, f64, f64)> = BTreeMap::new();
    for strip in strips {
        let Some(fam) = strip.slots.first().map(|s| net.roads[*s].family) else {
            continue;
        };
        let mut along = vec![0.0];
        for w in strip.joints.windows(2) {
            along.push(along.last().unwrap() + (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1));
        }
        let ground: Vec<f64> = strip
            .joints
            .iter()
            .map(|j| f64::from(base.sample(j.0, j.1).height))
            .collect();
        let e = long.entry(format!("{fam:?}")).or_default();
        let mut j = 0;
        for i in 0..along.len().saturating_sub(1) {
            while j + 1 < along.len() && along[j] - along[i] < 200.0 {
                j += 1;
            }
            let run = along[j] - along[i];
            if run < 150.0 {
                break;
            }
            let l = along[i + 1] - along[i];
            let grade = (ground[j] - ground[i]).abs() / run;
            e.0 += l;
            if grade > grade_aim(fam) {
                e.1 += l;
            }
            if grade > 1.25 * grade_aim(fam) {
                e.2 += l;
            }
            if (strip.joints[i].2 - ground[i]).abs() > 5.0 {
                e.3 += l;
            }
        }
    }
    for (fam, (all, over, far_over, off)) in &long {
        println!(
            "  {fam:5} {:6.1} km in windows; terrain over the aim {:5.1} km, over 1.25 x aim {:5.1} km; profile more than 5 m off the terrain {:5.1} km",
            all / 1000.0,
            over / 1000.0,
            far_over / 1000.0,
            off / 1000.0
        );
    }

    // ---- why the profile leaves the terrain
    let (mut rough_all, mut off_n, mut off_limit, mut off_rough, mut off_both) =
        (Vec::new(), 0usize, 0usize, 0usize, 0usize);
    for strip in strips {
        let Some(fam) = strip.slots.first().map(|s| net.roads[*s].family) else {
            continue;
        };
        let mut along = vec![0.0];
        for w in strip.joints.windows(2) {
            along.push(along.last().unwrap() + (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1));
        }
        let ground: Vec<f64> = strip
            .joints
            .iter()
            .map(|j| f64::from(base.sample(j.0, j.1).height))
            .collect();
        for i in 0..ground.len() {
            // the ground against a straight line fitted through the ground 50 m either side
            let near: Vec<usize> = (0..ground.len())
                .filter(|&k| (along[k] - along[i]).abs() <= 50.0)
                .collect();
            if near.len() < 4 {
                continue;
            }
            let n = near.len() as f64;
            let (mx, my) = (
                near.iter().map(|&k| along[k]).sum::<f64>() / n,
                near.iter().map(|&k| ground[k]).sum::<f64>() / n,
            );
            let sxx: f64 = near.iter().map(|&k| (along[k] - mx).powi(2)).sum();
            let sxy: f64 = near
                .iter()
                .map(|&k| (along[k] - mx) * (ground[k] - my))
                .sum();
            let slope = if sxx > 0.0 { sxy / sxx } else { 0.0 };
            let rough = (ground[i] - (my + slope * (along[i] - mx))).abs();
            rough_all.push(rough);
            if (strip.joints[i].2 - ground[i]).abs() > 5.0 {
                off_n += 1;
                let grade_at = |k: usize| {
                    strip.joints.get(k + 1).map_or(0.0, |b| {
                        ((b.2 - strip.joints[k].2) / (along[k + 1] - along[k]).max(0.5)).abs()
                    })
                };
                let at_limit = grade_at(i) >= 0.95 * grade_aim(fam)
                    || (i > 0 && grade_at(i - 1) >= 0.95 * grade_aim(fam))
                    || slope.abs() > grade_aim(fam);
                match (at_limit, rough > 2.5) {
                    (true, true) => off_both += 1,
                    (true, false) => off_limit += 1,
                    (false, true) => off_rough += 1,
                    _ => {}
                }
            }
        }
    }
    println!("\n== why the profile leaves the terrain");
    println!(
        "ground under the joints against a straight line through the ground 50 m either side: p50 {:.2} p90 {:.2} p99 {:.2} m",
        pct(&mut rough_all.clone(), 0.5),
        pct(&mut rough_all.clone(), 0.9),
        pct(&mut rough_all, 0.99)
    );
    println!(
        "joints more than 5 m off the ground: {off_n}; at the grade limit {off_limit}, ground rough (> 2.5 m off the line) {off_rough}, both {off_both}, neither {}",
        off_n - off_limit - off_rough - off_both
    );

    // ---- profile
    println!("\n== profile");
    println!(
        "measured grade minus planned grade: p50 {:.3} p95 {:.3} p99 {:.3} max {:.3}",
        pct(&mut plan_vs_measured, 0.5),
        pct(&mut plan_vs_measured, 0.95),
        pct(&mut plan_vs_measured, 0.99),
        pct(&mut plan_vs_measured, 1.0)
    );
    let mut up: Vec<f64> = centre_shift.iter().map(|v| v.abs()).collect();
    println!(
        "profile above/below the unshaped terrain at piece centres: |p50| {:.2} |p90| {:.2} |p99| {:.2} max {:.2} m; fill p99 {:.2}, cut p99 {:.2}",
        pct(&mut up, 0.5),
        pct(&mut up, 0.9),
        pct(&mut up, 0.99),
        pct(&mut up, 1.0),
        pct(&mut centre_shift.clone(), 0.99),
        -pct(&mut centre_shift.clone(), 0.01)
    );
    // vertical curvature: change of grade per metre between neighbouring segments
    let mut curv: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut planned_over: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    for strip in strips {
        let fam = strip.slots.first().map(|s| net.roads[*s].family);
        let Some(fam) = fam else { continue };
        let g: Vec<(f64, f64)> = strip
            .joints
            .windows(2)
            .map(|w| {
                let l = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1).max(0.5);
                ((w[1].2 - w[0].2) / l, l)
            })
            .collect();
        let e = planned_over.entry(format!("{fam:?}")).or_default();
        for (grade, l) in &g {
            e.1 += l;
            if grade.abs() > grade_aim(fam) + 1e-6 {
                e.0 += l;
            }
        }
        for w in g.windows(2) {
            let run = (w[0].1 + w[1].1) / 2.0;
            curv.entry(format!("{fam:?}"))
                .or_default()
                .push((w[1].0 - w[0].0).abs() / run * 100.0);
        }
    }
    println!(
        "planned profile over the family's aim: {:?}",
        planned_over
            .iter()
            .map(|(k, v)| (
                k.clone(),
                format!("{:.2} of {:.1} km", v.0 / 1000.0, v.1 / 1000.0)
            ))
            .collect::<Vec<_>>()
    );
    println!("vertical curvature of the planned profile, % grade change per metre (K = 1/that):");
    for (fam, list) in &mut curv {
        let n1 = list.iter().filter(|v| **v > 0.5).count();
        let n2 = list.iter().filter(|v| **v > 1.0).count();
        println!(
            "  {fam:5} joints {:6} p50 {:.3} p95 {:.3} p99 {:.3} max {:.3};  > 0.5 %/m: {n1}, > 1 %/m: {n2}",
            list.len(),
            pct(list, 0.5),
            pct(list, 0.95),
            pct(list, 0.99),
            pct(list, 1.0)
        );
    }

    // ---- earthworks against how steep the hillside is
    println!(
        "\n== earthworks by raw side slope of the hillside under the piece (count; cross fall > 5 %; miss > 0.15; grade over hard)"
    );
    let bands = [0.0, 0.1, 0.2, 0.3, 0.45, 0.667, 1.0, 9.0];
    for w in bands.windows(2) {
        let list: Vec<&&PieceCheck> = drivable
            .iter()
            .filter(|p| {
                raw_by
                    .get(&p.slot)
                    .is_some_and(|r| r.cross_fall >= w[0] && r.cross_fall < w[1])
            })
            .collect();
        println!(
            "  side slope {:>3.0}-{:<3.0} %: {:6} pieces {:6.1} km; {:5} ; {:5} ; {:5}",
            w[0] * 100.0,
            w[1] * 100.0,
            list.len(),
            list.iter().map(|p| len(p)).sum::<f64>() / 1000.0,
            list.iter().filter(|p| p.cross_fall > 0.05).count(),
            list.iter().filter(|p| p.miss_m > 0.15).count(),
            list.iter().filter(|p| p.grade > hard(p)).count()
        );
    }
    let changed = conformed
        .heightmap
        .h
        .iter()
        .zip(&base.h)
        .filter(|(a, b)| (**a - **b).abs() > 0.01)
        .count();
    let big = conformed
        .heightmap
        .h
        .iter()
        .zip(&base.h)
        .filter(|(a, b)| (**a - **b).abs() > 5.0)
        .count();
    println!(
        "vertices changed: {changed} of {} ({:.1} %), by more than 5 m: {big}",
        base.h.len(),
        100.0 * changed as f64 / base.h.len() as f64
    );
    let mut capped_slope: Vec<f64> = conformed
        .capped
        .iter()
        .map(|c| f64::from(base.sample(c.0, c.1).slope_deg()))
        .collect();
    println!(
        "capped earthworks: {} vertices; base slope there p10 {:.0} p50 {:.0} p90 {:.0} deg",
        capped_slope.len(),
        pct(&mut capped_slope, 0.1),
        pct(&mut capped_slope, 0.5),
        pct(&mut capped_slope, 0.9)
    );
    let mut conf: Vec<f64> = conformed
        .conflicts
        .iter()
        .map(|c| c.height_difference_m)
        .collect();
    println!(
        "conflicts: {} p50 {:.1} p90 {:.1} max {:.1} m",
        conf.len(),
        pct(&mut conf, 0.5),
        pct(&mut conf, 0.9),
        pct(&mut conf, 1.0)
    );
    let mut ex: Vec<f64> = profiles
        .excesses
        .iter()
        .filter(|e| e.limit > 0.079)
        .map(|e| e.grade.abs())
        .collect();
    println!(
        "grade excesses (grade, not change): {} p50 {:.2} p90 {:.2} max {:.2}",
        ex.len(),
        pct(&mut ex, 0.5),
        pct(&mut ex, 0.9),
        pct(&mut ex, 1.0)
    );
}
